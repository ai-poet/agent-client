//! Small process and download helpers for the optional Computer Use driver.
//! Agent CLI descriptor, npm installation, and environment rewriting live here
//! no longer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::{brand, http};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOutcome {
    pub success: bool,
    pub output: String,
}

/// Pull the first `x.y.z` out of arbitrary version output.
pub fn parse_semantic_version(output: &str) -> Option<String> {
    let mut digits = String::new();
    let mut dots = 0;
    for character in output.chars() {
        match character {
            '0'..='9' => digits.push(character),
            '.' if !digits.is_empty() && dots < 2 && !digits.ends_with('.') => {
                digits.push('.');
                dots += 1;
            }
            _ if dots == 2 && !digits.ends_with('.') => break,
            _ => {
                digits.clear();
                dots = 0;
            }
        }
    }
    let trimmed = digits.trim_end_matches('.');
    (trimmed.split('.').count() == 3).then(|| trimmed.to_owned())
}

/// Run a program directly and capture a useful tail of its output.
pub fn run_program(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> InstallOutcome {
    let mut process = std::process::Command::new(program.as_ref());
    process.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        process.creation_flags(CREATE_NO_WINDOW);
    }
    match process.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let errors = String::from_utf8_lossy(&output.stderr);
            if !errors.trim().is_empty() {
                if !text.trim().is_empty() {
                    text.push('\n');
                }
                text.push_str(&errors);
            }
            InstallOutcome {
                success: output.status.success(),
                output: last_meaningful_lines(text.trim(), 6),
            }
        }
        Err(error) => InstallOutcome {
            success: false,
            output: format!(
                "could not start {}: {error}",
                program.as_ref().to_string_lossy()
            ),
        },
    }
}

pub(crate) fn last_meaningful_lines(text: &str, limit: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

/// A fresh staging directory under the app's own toolchain directory.
pub(crate) fn staging_dir() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let root = brand::data_dir()
        .map(|data| data.join("toolchains").join(".staging"))
        .unwrap_or_else(|| std::env::temp_dir().join("toolchain-install"));
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    Ok(dir)
}

/// Download a non-empty file, retrying when a successful curl exit did not
/// leave the archive on disk.
pub(crate) fn download(url: &str, destination: &Path) -> Result<()> {
    let mut last_error = String::new();
    for _ in 0..3 {
        let outcome = run_program(
            http::CURL_PATH,
            &[
                "-fsSL",
                "--connect-timeout",
                "20",
                "--retry",
                "2",
                "--retry-delay",
                "1",
                "-o",
                &destination.to_string_lossy(),
                url,
            ],
        );
        if !outcome.success {
            return Err(anyhow!("download failed: {}", outcome.output));
        }
        match std::fs::metadata(destination) {
            Ok(metadata) if metadata.len() > 0 => return Ok(()),
            Ok(_) => return Err(anyhow!("{url} answered with an empty file")),
            Err(error) => {
                last_error = format!(
                    "{url} did not leave {} on disk: {error}",
                    destination.display()
                );
                std::thread::sleep(std::time::Duration::from_secs(1));
                if let Some(parent) = destination.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
        }
    }
    Err(anyhow!(last_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_versions_are_extracted_from_command_output() {
        assert_eq!(
            parse_semantic_version("cua-driver v0.23.2"),
            Some("0.23.2".into())
        );
        assert_eq!(parse_semantic_version("not a version"), None);
    }

    #[test]
    fn output_keeps_the_non_empty_tail() {
        assert_eq!(
            last_meaningful_lines("one\n\n two\nthree", 2),
            " two\nthree"
        );
    }
}
