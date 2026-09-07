//! The objective gate for a workflow stage.
//!
//! An agent's own report of success is not a verdict. When a stage carries
//! a check command — the project's test suite, a linter, a build — it runs
//! in the stage's worktree after the agent's turn, and only its exit status
//! decides whether the stage passed. Same rule Codeg's task engine follows
//! for merges: read git and the shell, not the transcript.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use sub2api::cli_detect::run_with_timeout;

/// How much of the command's output is kept for the failure report.
pub const OUTPUT_TAIL_CHARS: usize = 1500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOutcome {
    pub passed: bool,
    pub timed_out: bool,
    /// The end of the combined output — where a test runner prints its
    /// verdict.
    pub output_tail: String,
}

/// The command as the platform's shell would run it, in `cwd`.
pub fn shell_command(command: &str, cwd: &Path) -> Command {
    #[cfg(windows)]
    let mut process = {
        let mut process = Command::new("cmd");
        process.args(["/C", command]);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    process.current_dir(cwd);
    process
}

/// Run the check to completion or `timeout`. Blocks; callers run it off
/// the UI thread.
pub fn run_check(command: &str, cwd: &Path, timeout: Duration) -> CheckOutcome {
    let run = run_with_timeout(shell_command(command, cwd), timeout);
    CheckOutcome {
        passed: run.success && !run.timed_out,
        timed_out: run.timed_out,
        output_tail: tail(&run.output, OUTPUT_TAIL_CHARS),
    }
}

/// The last `chars` characters of `text`, trimmed.
pub fn tail(text: &str, chars: usize) -> String {
    let total = text.chars().count();
    let skip = total.saturating_sub(chars);
    text.chars()
        .skip(skip)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn passing_command_passes_and_keeps_output() {
        let outcome = run_check("echo workflow-check-ok", &cwd(), Duration::from_secs(30));
        assert!(outcome.passed, "{outcome:?}");
        assert!(!outcome.timed_out);
        assert!(outcome.output_tail.contains("workflow-check-ok"));
    }

    #[test]
    fn failing_command_fails() {
        let outcome = run_check("exit 3", &cwd(), Duration::from_secs(30));
        assert!(!outcome.passed);
        assert!(!outcome.timed_out);
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("  ab  ", 10), "ab");
        assert_eq!(tail("", 5), "");
    }
}
