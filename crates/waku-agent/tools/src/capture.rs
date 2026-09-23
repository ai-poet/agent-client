//! Running a child process and collecting what it printed.
//!
//! Fork addition (Waku). Both Windows execution paths — the Bash tool's
//! `cmd /C` fallback and the PowerShell tool — read stdout to its end and only
//! then stderr, line by line as UTF-8. Either half of that hangs a command:
//!
//! - a child that fills the stderr pipe while nobody reads it blocks, stdout
//!   never closes, and the call waits out its whole timeout. `cargo build`,
//!   `npm install` and `git clone` all write their progress to stderr;
//! - `next_line` gives up at the first line that is not UTF-8, which on a
//!   Chinese-language Windows is the first line any console program prints in
//!   the OEM code page (GBK) — and the unread pipe then fills the same way.
//!
//! Here both streams are drained concurrently as bytes and decoded afterwards,
//! line by line, so one GBK line cannot garble the UTF-8 around it. A timed-out
//! command takes its whole process tree with it: killing only the direct child
//! left `npm`, `node` or `cargo` running under a dead `cmd.exe`.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// What a finished — or abandoned — command left behind.
pub(crate) struct Captured {
    pub stdout: String,
    pub stderr: String,
    /// `None` when the process ended without a code (killed, or never waited).
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Run `command` to completion or until `timeout`, whichever is first.
pub(crate) async fn run_captured(mut command: Command, timeout: Duration) -> std::io::Result<Captured> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut command);
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let collect = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        tokio::join!(
            async {
                if let Some(pipe) = stdout.as_mut() {
                    let _ = pipe.read_to_end(&mut out).await;
                }
            },
            async {
                if let Some(pipe) = stderr.as_mut() {
                    let _ = pipe.read_to_end(&mut err).await;
                }
            },
        );
        let status = child.wait().await;
        (out, err, status)
    };

    match tokio::time::timeout(timeout, collect).await {
        Ok((out, err, status)) => Ok(Captured {
            stdout: decode_output(&out),
            stderr: decode_output(&err),
            exit_code: status.ok().and_then(|status| status.code()),
            timed_out: false,
        }),
        Err(_) => {
            kill_tree(&mut child).await;
            Ok(Captured {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: true,
            })
        }
    }
}

/// Kill `child` and, on Windows, everything it started.
pub(crate) async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        // The System32 copy, so cleanup does not depend on what is on PATH.
        let root = std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
        let mut taskkill = Command::new(root.join("System32").join("taskkill.exe"));
        taskkill
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut taskkill);
        let _ = taskkill.status().await;
    }
    let _ = child.kill().await;
}

/// Keep a console child from opening a window of its own. A process that has
/// a console already shares it with its children; one that has none — a GUI
/// host — would otherwise flash a window for every command.
pub(crate) fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Decode a command's output, one line at a time.
///
/// Lines that are UTF-8 are kept as they are. A line that is not is read in
/// the console's code page on Windows — what `cmd.exe`, `where`, `ipconfig`
/// and friends write when their output is a pipe — and lossily elsewhere.
/// Per line, because Git Bash's own tools write UTF-8 and the native programs
/// it runs do not, in the same output.
pub(crate) fn decode_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.replace("\r\n", "\n");
    }
    let mut decoded = String::with_capacity(bytes.len());
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if index > 0 {
            decoded.push('\n');
        }
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        match std::str::from_utf8(line) {
            Ok(text) => decoded.push_str(text),
            Err(_) => decoded.push_str(&decode_legacy_line(line)),
        }
    }
    decoded
}

#[cfg(windows)]
fn decode_legacy_line(line: &[u8]) -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            source: *const u8,
            source_len: i32,
            destination: *mut u16,
            destination_len: i32,
        ) -> i32;
        fn GetConsoleOutputCP() -> u32;
    }
    const CP_OEMCP: u32 = 1;
    let Ok(length) = i32::try_from(line.len()) else {
        return String::from_utf8_lossy(line).into_owned();
    };
    if length == 0 {
        return String::new();
    }
    // SAFETY: the pointers and lengths describe `line` and `wide`, which
    // outlive both calls; a zero return is handled as failure.
    unsafe {
        let console = GetConsoleOutputCP();
        let code_page = if console == 0 { CP_OEMCP } else { console };
        let needed = MultiByteToWideChar(code_page, 0, line.as_ptr(), length, std::ptr::null_mut(), 0);
        if needed <= 0 {
            return String::from_utf8_lossy(line).into_owned();
        }
        let mut wide = vec![0u16; needed as usize];
        let written = MultiByteToWideChar(code_page, 0, line.as_ptr(), length, wide.as_mut_ptr(), needed);
        if written <= 0 {
            return String::from_utf8_lossy(line).into_owned();
        }
        String::from_utf16_lossy(&wide[..written as usize])
    }
}

#[cfg(not(windows))]
fn decode_legacy_line(line: &[u8]) -> String {
    String::from_utf8_lossy(line).into_owned()
}

/// Read `pipe` to its end, handing each decoded line to `line` as it arrives.
///
/// For background commands, whose output is shown while they run. Lines are
/// split on bytes and decoded one at a time, so a line in the console code
/// page is decoded rather than ending the read.
pub(crate) async fn for_each_line<R>(mut pipe: R, mut line: impl FnMut(String))
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut pending: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        pending.extend_from_slice(&buffer[..read]);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let rest = pending.split_off(end + 1);
            let complete = std::mem::replace(&mut pending, rest);
            let bytes = &complete[..complete.len() - 1];
            line(decode_output(bytes.strip_suffix(b"\r").unwrap_or(bytes)));
        }
    }
    if !pending.is_empty() {
        line(decode_output(&pending));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_output_is_kept_and_crlf_normalized() {
        assert_eq!(decode_output("编译完成\r\nok\r\n".as_bytes()), "编译完成\nok\n");
    }

    /// One undecodable line must not take the rest of the output with it.
    #[test]
    fn a_line_that_is_not_utf8_leaves_its_neighbours_intact() {
        let mut bytes = b"first\n".to_vec();
        bytes.extend_from_slice(&[0xCF, 0xB5, 0xCD, 0xB3]); // "系统" in GBK
        bytes.extend_from_slice("\n最后\n".as_bytes());
        let decoded = decode_output(&bytes);
        let lines: Vec<&str> = decoded.lines().collect();
        assert_eq!(lines.len(), 3, "{decoded:?}");
        assert_eq!(lines[0], "first");
        assert_eq!(lines[2], "最后");
    }

    #[tokio::test]
    async fn lines_are_delivered_one_at_a_time_including_the_unterminated_last() {
        let input: &[u8] = b"one\ntwo\r\nthree";
        let mut seen = Vec::new();
        for_each_line(input, |line| seen.push(line)).await;
        assert_eq!(seen, ["one", "two", "three"]);
    }

    /// Both pipes are drained at once: a child that writes more to stderr
    /// than a pipe holds before touching stdout would deadlock a sequential
    /// reader until the timeout.
    #[tokio::test]
    async fn a_full_stderr_pipe_does_not_stall_the_command() {
        let script = "for i in $(seq 1 20000); do echo 'line of stderr output' >&2; done; echo done";
        let mut command = if cfg!(windows) {
            match claurst_core::shell::windows_bash() {
                Some(bash) => {
                    let mut command = Command::new(bash);
                    command.args(["-c", script]);
                    command
                }
                None => return,
            }
        } else {
            let mut command = Command::new("bash");
            command.args(["-c", script]);
            command
        };
        command.current_dir(std::env::temp_dir());
        let captured = run_captured(command, Duration::from_secs(60)).await.expect("spawn");
        assert!(!captured.timed_out);
        assert_eq!(captured.stdout.trim(), "done");
        assert!(captured.stderr.len() > 100_000);
    }
}
