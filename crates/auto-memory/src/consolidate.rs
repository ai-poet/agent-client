// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! Consolidation: after a stretch of a session (and when it ends), the
//! model reads what was said and files the new facts worth keeping. This is
//! the pure part — capturing the conversation, the prompt, and turning the
//! model's untrusted JSON into memories; the bridge runs the model call.

use std::collections::VecDeque;

use serde_json::Value;

use crate::name::normalize_name;
use crate::store::MemoryStore;
use crate::types::{MemoryDraft, MemoryScope, MemoryType, ScopeDir};

const USER_MAX_CHARS: usize = 2000;
const ASSISTANT_MAX_CHARS: usize = 500;
const MAX_LINES: usize = 40;
const MAX_BUFFER_CHARS: usize = 24_000;
const DESCRIPTION_MAX: usize = 160;
const BODY_MAX: usize = 2000;
const TITLE_MAX: usize = 80;

/// Who said a captured line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    User,
    Assistant,
}

/// Text worth capturing from one message: trimmed and capped (a person's
/// words matter more than the agent's narration). `None` for nothing.
pub fn capture_text(speaker: Speaker, text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let cap = match speaker {
        Speaker::User => USER_MAX_CHARS,
        Speaker::Assistant => ASSISTANT_MAX_CHARS,
    };
    Some(cut(text, cap))
}

fn cut(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// The recent conversation, oldest dropped first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureBuffer {
    lines: VecDeque<String>,
    chars: usize,
    user_lines: usize,
}

impl CaptureBuffer {
    pub fn push(&mut self, speaker: Speaker, text: &str) {
        let Some(text) = capture_text(speaker, text) else {
            return;
        };
        let line = match speaker {
            Speaker::User => {
                self.user_lines += 1;
                format!("USER: {text}")
            }
            Speaker::Assistant => format!("ASSISTANT: {text}"),
        };
        self.chars += line.chars().count();
        self.lines.push_back(line);
        while self.lines.len() > 1
            && (self.lines.len() > MAX_LINES || self.chars > MAX_BUFFER_CHARS)
            && let Some(dropped) = self.lines.pop_front()
        {
            self.chars -= dropped.chars().count();
        }
    }

    /// Whether anything a person said is in the buffer.
    pub fn has_user_text(&self) -> bool {
        self.user_lines > 0
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Hand the transcript over and start again.
    pub fn take(&mut self) -> String {
        let transcript = self.lines.drain(..).collect::<Vec<_>>().join("\n\n");
        self.chars = 0;
        self.user_lines = 0;
        transcript
    }
}

/// The model's instructions (verbatim from the reference).
pub fn build_consolidation_prompt(existing_names: &[String], transcript: &str, max: usize) -> String {
    let known = if existing_names.is_empty() {
        "(none)".to_owned()
    } else {
        existing_names.join(", ")
    };
    format!(
        "You are the memory-consolidation step of a coding agent. Below is a transcript
summary of a session that just ended. Extract NEW facts worth persisting across
sessions for this user, following these rules:

- Persist: who the user is (role, expertise, durable preferences); corrections or
  confirmations about how to work; ongoing goals/constraints with absolute dates;
  external resources worth returning to.
- Do NOT persist: one-off task details, anything recoverable from the codebase or
  AGENTS.md, session-specific context.
- Existing memories (do not duplicate them): {known}
- Output AT MOST {max} items. If nothing is worth persisting, output [].

Return ONLY a JSON array, each element exactly:
{{\"name\":\"kebab-case-id\",\"title\":\"short human heading\",\"description\":\"one line <=160 chars\",\"type\":\"user|feedback|project|reference\",\"body\":\"the fact in markdown; for feedback include **Why:** and **How to apply:** lines\",\"scope\":\"project\"}}
Use scope \"user\" only for user-global preferences; default \"project\".

<transcript>
{transcript}
</transcript>"
    )
}

/// The JSON array in the model's answer (first `[` to last `]`).
pub fn parse_candidates(raw: &str) -> Option<Vec<Value>> {
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    if end <= start {
        return None;
    }
    match serde_json::from_str::<Value>(&raw[start..=end]).ok()? {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

/// One checked candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub draft: MemoryDraft,
    pub scope: MemoryScope,
}

/// Check one item of the model's answer; `None` drops it.
pub fn sanitize_candidate(raw: &Value, user_scope_enabled: bool) -> Option<Candidate> {
    let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::trim);
    let name = normalize_name(text("name")?).ok()?;
    let description = cut(text("description")?, DESCRIPTION_MAX);
    let body = cut(text("body")?, BODY_MAX);
    if description.is_empty() || body.is_empty() {
        return None;
    }
    let kind = match raw.get("type") {
        None | Some(Value::Null) => MemoryType::Reference,
        Some(value) => MemoryType::parse(value.as_str()?)?,
    };
    let scope = if text("scope") == Some("user") && user_scope_enabled {
        MemoryScope::User
    } else {
        MemoryScope::Project
    };
    Some(Candidate {
        draft: MemoryDraft {
            name,
            title: text("title")
                .filter(|title| !title.is_empty())
                .map(|title| cut(title, TITLE_MAX)),
            description,
            kind,
            body,
            pinned: None,
        },
        scope,
    })
}

/// File the candidates: at most `max`, never over an existing name in any
/// scope (updating from an unattended pass is riskier than skipping).
pub fn apply_candidates(
    store: &MemoryStore,
    user: Option<&ScopeDir>,
    project: &ScopeDir,
    candidates: &[Value],
    max: usize,
) -> usize {
    let scopes = user
        .into_iter()
        .cloned()
        .chain(std::iter::once(project.clone()))
        .collect::<Vec<_>>();
    let mut written = 0;
    for raw in candidates {
        if written >= max {
            break;
        }
        let Some(candidate) = sanitize_candidate(raw, user.is_some()) else {
            continue;
        };
        if matches!(store.find_in(&candidate.draft.name, &scopes), Ok(Some(_))) {
            continue;
        }
        let target = match candidate.scope {
            MemoryScope::User => user.unwrap_or(project),
            MemoryScope::Project => project,
        };
        match store.write(target, candidate.draft) {
            Ok(_) => written += 1,
            Err(error) => tracing::warn!("consolidated memory not written: {error:#}"),
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn captures_are_capped_by_speaker() {
        let long = "x".repeat(3000);
        assert_eq!(capture_text(Speaker::User, &long).unwrap().chars().count(), 2001);
        assert_eq!(capture_text(Speaker::Assistant, &long).unwrap().chars().count(), 501);
        assert_eq!(capture_text(Speaker::User, "  "), None);
    }

    #[test]
    fn the_buffer_drops_the_oldest_lines() {
        let mut buffer = CaptureBuffer::default();
        for i in 0..60 {
            buffer.push(Speaker::Assistant, &format!("reply {i}"));
        }
        assert!(!buffer.has_user_text());
        buffer.push(Speaker::User, "remember pnpm");
        assert!(buffer.has_user_text());
        let transcript = buffer.take();
        assert_eq!(transcript.split("\n\n").count(), 40);
        assert!(transcript.ends_with("USER: remember pnpm"));
        assert!(buffer.is_empty() && !buffer.has_user_text());
    }

    #[test]
    fn parses_the_array_out_of_chatter() {
        let raw = "Sure! Here you go:\n```json\n[{\"name\":\"a\"}]\n```";
        assert_eq!(parse_candidates(raw).unwrap().len(), 1);
        assert!(parse_candidates("no json").is_none());
        assert!(parse_candidates("[not json]").is_none());
    }

    #[test]
    fn sanitizes_untrusted_items() {
        let ok = json!({"name":"Use PNPM","title":"PNPM","description":"d","type":"feedback","body":"b","scope":"user"});
        let candidate = sanitize_candidate(&ok, true).unwrap();
        assert_eq!(candidate.draft.name, "use-pnpm");
        assert_eq!(candidate.scope, MemoryScope::User);
        assert_eq!(sanitize_candidate(&ok, false).unwrap().scope, MemoryScope::Project);
        for bad in [
            json!({"name":"中文","description":"d","body":"b"}),
            json!({"name":"a","description":"","body":"b"}),
            json!({"name":"a","description":"d","body":"b","type":"other"}),
            json!("string"),
        ] {
            assert!(sanitize_candidate(&bad, true).is_none(), "{bad}");
        }
        let long = json!({"name":"a","description":"d".repeat(400),"body":"b"});
        assert_eq!(sanitize_candidate(&long, true).unwrap().draft.description.chars().count(), 161);
    }

    #[test]
    fn applying_skips_existing_names_and_stops_at_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(tmp.path());
        let user = store.user_dir();
        let project = store.project_dir(std::path::Path::new("/w"));
        let item = |name: &str| json!({"name": name, "description": "d", "body": "b"});
        assert_eq!(
            apply_candidates(&store, Some(&user), &project, &[item("a"), item("b")], 5),
            2
        );
        assert_eq!(
            apply_candidates(&store, Some(&user), &project, &[item("a"), item("c"), item("d"), item("e")], 2),
            2
        );
        assert_eq!(store.list(&project).len(), 4);
    }

    #[test]
    fn the_prompt_lists_existing_names() {
        let prompt = build_consolidation_prompt(&["a".into(), "b".into()], "USER: hi", 5);
        assert!(prompt.contains("Existing memories (do not duplicate them): a, b"));
        assert!(prompt.contains("AT MOST 5 items"));
        assert!(prompt.contains("<transcript>\nUSER: hi\n</transcript>"));
        assert!(build_consolidation_prompt(&[], "", 1).contains("(none)"));
    }
}
