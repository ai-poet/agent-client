// Fork addition (Waku): process-group ownership for background shell commands.
//
// A background shell is not the whole workload: `npm run dev`, `cargo test` and
// watchers all start children of their own. Giving the shell a new process group
// at spawn lets both stop paths — the model's `monitor cancel` and the desktop's
// background-work button — terminate that whole workload instead of its first
// process only.

/// Put a background command in a process group of its own on Unix. Other
/// platforms already stop workloads by their process tree and need no setup.
pub(crate) fn isolate_process_group(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(not(unix))]
    let _ = command;
}

/// Stop the workload rooted at `pid`, with a direct-PID fallback on Unix for a
/// task registered before process groups were introduced.
pub fn terminate_process_tree(pid: u32) -> Result<(), String> {
    terminate(pid)
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<(), String> {
    use nix::sys::signal::{kill, killpg, Signal};
    use nix::unistd::Pid;

    let raw = i32::try_from(pid).map_err(|_| format!("process id {pid} is out of range"))?;
    let pid = Pid::from_raw(raw);
    let group_error = match killpg(pid, Signal::SIGTERM) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    kill(pid, Signal::SIGTERM).map_err(|error| {
        format!("could not stop process group {pid} ({group_error}) or process {pid} ({error})")
    })
}

#[cfg(windows)]
fn terminate(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    // The System32 copy, so cleanup does not depend on PATH or on a file named
    // taskkill beside the agent's workspace.
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    let status = Command::new(root.join("System32").join("taskkill.exe"))
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .status()
        .map_err(|error| format!("could not run taskkill: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("taskkill exited with {status}"))
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate(_pid: u32) -> Result<(), String> {
    Err("stopping a background process is not supported on this platform".to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    struct KillOnDrop(Option<u32>);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            if let Some(pid) = self.0 {
                let _ = terminate_process_tree(pid);
            }
        }
    }

    fn process_is_live(pid: i32) -> bool {
        let Ok(output) = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
        else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        !state.is_empty() && !state.starts_with('Z')
    }

    #[tokio::test]
    async fn stopping_a_background_shell_reaches_its_children() {
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg("sleep 60 & echo $!; wait")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        isolate_process_group(&mut command);
        let mut child = command.spawn().expect("start background shell");
        let parent = child.id().expect("shell pid") as i32;
        let mut cleanup = KillOnDrop(Some(parent as u32));

        let stdout = child.stdout.take().expect("shell stdout");
        let mut first_line = String::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            BufReader::new(stdout).read_line(&mut first_line),
        )
        .await
        .expect("child pid should arrive promptly")
        .expect("read child pid");
        let descendant: i32 = first_line.trim().parse().expect("numeric child pid");
        assert_eq!(
            nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(descendant))),
            Ok(nix::unistd::Pid::from_raw(parent)),
            "the workload shares the shell's isolated process group"
        );
        assert!(process_is_live(descendant));

        terminate_process_tree(parent as u32).expect("stop the whole workload");
        cleanup.0 = None;
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("the shell exits after its group is stopped")
            .expect("wait for shell");

        let mut stopped = false;
        for _ in 0..50 {
            if !process_is_live(descendant) {
                stopped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(stopped, "the descendant did not outlive the stop request");
    }
}
