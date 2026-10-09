//! What a sub-agent got done, for when it does not finish.
//!
//! A child's conversation lives only for its run. When the run failed or was
//! stopped, the parent used to read one line back — `Sub-agent error: …` —
//! and nothing of the files the child had read or changed, the commands it
//! had run, or the step that went wrong. Asked to carry on, the parent could
//! only send a new child to do the whole task again, failing step included.
//!
//! [`Progress`] is kept from the events the child's record is already built
//! from, so it costs no second copy of its conversation and is up to date
//! the moment the run ends — including when the run is abandoned because the
//! parent's turn was stopped. [`Progress::render`] turns it into a bounded
//! digest that is appended to whatever the parent reads back.

use serde_json::Value;

use crate::events::SubagentEvent;

/// How much of a call's arguments a step shows.
const STEP_INPUT_CHARS: usize = 160;
/// How much of a failed call's output a step shows.
const FAILURE_OUTPUT_CHARS: usize = 280;
/// How much of the child's last text the digest keeps.
const LAST_TEXT_CHARS: usize = 1_200;
/// The budget for the step list; older steps give way first.
const STEPS_CHARS: usize = 4_000;
/// At most this many changed files are named.
const CHANGED_FILES: usize = 20;

/// Tools whose successful call changed the file named in their input.
const FILE_WRITING_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "BatchEdit"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum StepOutcome {
    Running,
    Ok,
    Failed(String),
}

#[derive(Clone, Debug)]
struct Step {
    id: String,
    name: String,
    input: String,
    /// The file a writing tool names, kept whole for "files it changed".
    file: Option<String>,
    outcome: StepOutcome,
}

/// One child's record, as the parent will need it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Progress {
    steps: Vec<Step>,
    /// Text since its last tool call.
    current_text: String,
    /// The last stretch of text that had any.
    last_text: String,
}

impl Progress {
    pub(crate) fn record(&mut self, event: &SubagentEvent) {
        match event {
            SubagentEvent::Text(delta) => {
                self.current_text.push_str(delta);
                // Only the tail is ever shown; a long report must not grow
                // this without bound.
                if self.current_text.len() > LAST_TEXT_CHARS * 8 {
                    self.current_text = tail(&self.current_text, LAST_TEXT_CHARS * 2);
                }
            }
            SubagentEvent::ToolStarted { id, name, input } => {
                self.close_text();
                self.steps.push(Step {
                    id: id.clone(),
                    name: name.clone(),
                    input: describe_input(input),
                    file: FILE_WRITING_TOOLS
                        .contains(&name.as_str())
                        .then(|| file_of(input))
                        .flatten(),
                    outcome: StepOutcome::Running,
                });
            }
            SubagentEvent::ToolFinished {
                id, output, failed, ..
            } => {
                if let Some(step) = self.steps.iter_mut().rev().find(|step| step.id == *id) {
                    step.outcome = if *failed {
                        StepOutcome::Failed(clip(&output_text(output), FAILURE_OUTPUT_CHARS))
                    } else {
                        StepOutcome::Ok
                    };
                }
            }
            SubagentEvent::Started { .. } | SubagentEvent::Finished { .. } => {}
        }
    }

    fn close_text(&mut self) {
        let text = std::mem::take(&mut self.current_text);
        if !text.trim().is_empty() {
            self.last_text = text;
        }
    }

    fn latest_text(&self) -> &str {
        if self.current_text.trim().is_empty() {
            &self.last_text
        } else {
            &self.current_text
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.latest_text().trim().is_empty()
    }

    /// The digest the parent reads, or `None` when the child had done
    /// nothing worth reporting.
    pub(crate) fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut out = String::from(
            "What the sub-agent had done before it stopped (from its own record; its \
             conversation is gone):",
        );

        let mut changed: Vec<&str> = Vec::new();
        for step in &self.steps {
            if step.outcome == StepOutcome::Ok
                && let Some(file) = step.file.as_deref()
                && !changed.contains(&file)
            {
                changed.push(file);
            }
        }
        if !changed.is_empty() {
            out.push_str("\nFiles it changed: ");
            let shown = changed.len().min(CHANGED_FILES);
            out.push_str(&changed[..shown].join(", "));
            if changed.len() > shown {
                out.push_str(&format!(" and {} more", changed.len() - shown));
            }
        }

        if !self.steps.is_empty() {
            // Newest first until the budget runs out: the steps nearest the
            // stop are the ones to continue from.
            let mut lines = Vec::new();
            let mut used = 0;
            for step in self.steps.iter().rev() {
                let line = step_line(step);
                if used + line.len() > STEPS_CHARS && !lines.is_empty() {
                    break;
                }
                used += line.len();
                lines.push(line);
            }
            let omitted = self.steps.len() - lines.len();
            out.push_str(&format!("\nTool calls ({}):", self.steps.len()));
            if omitted > 0 {
                out.push_str(&format!("\n- … {omitted} earlier call(s) not shown"));
            }
            for line in lines.into_iter().rev() {
                out.push('\n');
                out.push_str(&line);
            }
        }

        let text = self.latest_text().trim();
        if !text.is_empty() {
            out.push_str("\nThe last thing it wrote:\n");
            out.push_str(&tail(text, LAST_TEXT_CHARS));
        }

        out.push_str(
            "\nDo not start this task over: check what it changed, then continue from where it \
             stopped — yourself, or with a new sub-agent briefed on the progress above.",
        );
        Some(out)
    }
}

/// `text` followed by the digest of `progress`, when there is one.
pub(crate) fn with_progress(text: String, progress: Option<&Progress>) -> String {
    match progress.and_then(Progress::render) {
        Some(digest) => format!("{text}\n\n{digest}"),
        None => text,
    }
}

fn step_line(step: &Step) -> String {
    let call = if step.input.is_empty() {
        step.name.clone()
    } else {
        format!("{} {}", step.name, step.input)
    };
    match &step.outcome {
        StepOutcome::Ok => format!("- {call} → ok"),
        StepOutcome::Running => format!("- {call} → still running when it stopped"),
        StepOutcome::Failed(output) if output.is_empty() => format!("- {call} → FAILED"),
        StepOutcome::Failed(output) => format!("- {call} → FAILED: {output}"),
    }
}

/// The argument that says what a call was about, else the arguments.
fn describe_input(input: &Value) -> String {
    const KEYS: [&str; 8] = [
        "command",
        "file_path",
        "notebook_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ];
    if let Some(object) = input.as_object() {
        for key in KEYS {
            if let Some(value) = object
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return format!("{key}={}", clip(value, STEP_INPUT_CHARS));
            }
        }
        if object.is_empty() {
            return String::new();
        }
    }
    if input.is_null() {
        return String::new();
    }
    clip(&input.to_string(), STEP_INPUT_CHARS)
}

fn file_of(input: &Value) -> Option<String> {
    ["file_path", "notebook_path", "path"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

fn output_text(output: &Value) -> String {
    match output {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// One line, at most `max` characters: whitespace runs fold to one space.
fn clip(text: &str, max: usize) -> String {
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.chars().count() <= max {
        return folded;
    }
    let mut clipped: String = folded.chars().take(max).collect();
    clipped.push('…');
    clipped
}

/// The last `max` characters of `text`, marked when cut.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let mut cut = String::from("…");
    cut.extend(text.chars().skip(count - max));
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn started(id: &str, name: &str, input: Value) -> SubagentEvent {
        SubagentEvent::ToolStarted {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    fn finished(id: &str, failed: bool, output: &str) -> SubagentEvent {
        SubagentEvent::ToolFinished {
            id: id.into(),
            name: String::new(),
            output: Value::String(output.into()),
            failed,
            image_source: None,
        }
    }

    #[test]
    fn a_child_that_did_nothing_has_no_digest() {
        let progress = Progress::default();
        assert!(progress.render().is_none());
        assert_eq!(
            with_progress("Sub-agent error: 502".into(), Some(&progress)),
            "Sub-agent error: 502"
        );
        assert_eq!(with_progress("x".into(), None), "x");
    }

    #[test]
    fn the_digest_names_each_call_its_outcome_and_what_changed() {
        let mut progress = Progress::default();
        progress.record(&SubagentEvent::Text("Reading the config first.".into()));
        progress.record(&started("1", "Read", json!({"file_path": "src/a.rs"})));
        progress.record(&finished("1", false, "fn main() {}"));
        progress.record(&started(
            "2",
            "Edit",
            json!({"file_path": "src/a.rs", "old_string": "a"}),
        ));
        progress.record(&finished("2", false, "ok"));
        progress.record(&started(
            "3",
            "Bash",
            json!({"command": "cargo   build\n--release"}),
        ));
        progress.record(&finished(
            "3",
            true,
            "Command exited with code 101\nerror[E0425]: x",
        ));
        progress.record(&started("4", "Bash", json!({"command": "cargo test"})));
        progress.record(&SubagentEvent::Text("Now running the tests".into()));

        let digest = progress.render().expect("digest");
        assert!(digest.contains("Files it changed: src/a.rs"), "{digest}");
        assert!(
            digest.contains("- Read file_path=src/a.rs → ok"),
            "{digest}"
        );
        assert!(
            digest.contains(
                "- Bash command=cargo build --release → FAILED: Command exited with code 101 error[E0425]: x"
            ),
            "{digest}"
        );
        assert!(
            digest.contains("- Bash command=cargo test → still running when it stopped"),
            "{digest}"
        );
        assert!(digest.contains("Now running the tests"), "{digest}");
        assert!(digest.contains("Do not start this task over"), "{digest}");
        // The order is the order it worked in.
        let read = digest.find("- Read").unwrap();
        let test = digest.find("command=cargo test").unwrap();
        assert!(read < test);
    }

    #[test]
    fn a_failed_write_did_not_change_its_file() {
        let mut progress = Progress::default();
        progress.record(&started(
            "1",
            "Write",
            json!({"file_path": "a.txt", "content": "x"}),
        ));
        progress.record(&finished("1", true, "permission denied"));
        let digest = progress.render().unwrap();
        assert!(!digest.contains("Files it changed"), "{digest}");
    }

    #[test]
    fn a_long_run_keeps_the_newest_calls_within_budget() {
        let mut progress = Progress::default();
        for index in 0..400 {
            let id = index.to_string();
            progress.record(&started(
                &id,
                "Grep",
                json!({"pattern": format!("needle-{index}")}),
            ));
            progress.record(&finished(&id, false, "match"));
        }
        let digest = progress.render().unwrap();
        assert!(digest.contains("Tool calls (400):"), "{digest}");
        assert!(digest.contains("earlier call(s) not shown"), "{digest}");
        assert!(digest.contains("needle-399"), "{digest}");
        assert!(!digest.contains("needle-0 "), "{digest}");
        assert!(digest.chars().count() < STEPS_CHARS + 1_000);
    }

    #[test]
    fn only_the_tail_of_its_text_is_kept() {
        let mut progress = Progress::default();
        let long = "字".repeat(LAST_TEXT_CHARS * 20);
        progress.record(&SubagentEvent::Text(long));
        progress.record(&SubagentEvent::Text("END".into()));
        let digest = progress.render().unwrap();
        assert!(digest.contains("…"), "a cut tail is marked");
        assert!(digest.contains("END"));
        assert!(progress.current_text.len() <= LAST_TEXT_CHARS * 8 + "END".len());
    }

    #[test]
    fn inputs_read_by_their_telling_argument() {
        assert_eq!(
            describe_input(&json!({"command": "ls -la"})),
            "command=ls -la"
        );
        assert_eq!(
            describe_input(&json!({"url": "https://x"})),
            "url=https://x"
        );
        assert_eq!(describe_input(&json!({})), "");
        assert_eq!(describe_input(&Value::Null), "");
        assert_eq!(describe_input(&json!({"a": 1})), r#"{"a":1}"#);
        let long = "x".repeat(STEP_INPUT_CHARS * 2);
        assert_eq!(
            describe_input(&json!({ "command": long })).chars().count(),
            "command=".len() + STEP_INPUT_CHARS + 1
        );
    }
}
