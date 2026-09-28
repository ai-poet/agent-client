//! A sub-agent's record, as a driver streams it into the panel that shows it.
//!
//! Shared by every driver whose provider reports what its sub-agents do —
//! the built-in agent (`native.rs`) and Claude Code (`claude_subagent.rs`).
//! Each sub-agent is one piece of background work of kind `Subagent`; its
//! record travels as `BackgroundWorkEvent::Transcript` entries keyed by the
//! same key, which the desktop keeps beside the item and renders the way it
//! renders the main transcript.

use std::collections::HashMap;

use serde_json::Value;

use crate::model::{
    ActivityItem, ActivityKind, BackgroundWorkEvent, BackgroundWorkKey, SubagentTranscriptBody,
    SubagentTranscriptEntry,
};

/// What a tool call looked like when it started, so its completion is sent
/// as the same row.
#[derive(Clone, Debug)]
pub(super) struct ChildCall {
    pub kind: ActivityKind,
    pub title: String,
    pub input: Value,
}

/// The stream of one sub-agent's record.
pub(super) struct SubagentFeed {
    key: BackgroundWorkKey,
    seq: u32,
    /// The text entry new text goes into, until something else is recorded.
    open_text: Option<String>,
    calls: HashMap<String, ChildCall>,
}

impl SubagentFeed {
    pub(super) fn new(key: BackgroundWorkKey) -> Self {
        Self {
            key,
            seq: 0,
            open_text: None,
            calls: HashMap::new(),
        }
    }

    /// Text the sub-agent wrote: onto the text entry already open, or a new
    /// one after anything else was recorded.
    pub(super) fn text(&mut self, text: &str) -> Option<BackgroundWorkEvent> {
        if text.is_empty() {
            return None;
        }
        let (id, append) = match &self.open_text {
            Some(id) => (id.clone(), true),
            None => {
                self.seq += 1;
                let id = format!("text-{}", self.seq);
                self.open_text = Some(id.clone());
                (id, false)
            }
        };
        Some(self.entry(
            id,
            SubagentTranscriptBody::Text {
                text: text.to_owned(),
                append,
            },
        ))
    }

    /// The next text starts an entry of its own — a new message, even when
    /// nothing came between.
    pub(super) fn close_text(&mut self) {
        self.open_text = None;
    }

    /// One tool call, whole, in its current state. Keyed by the provider's
    /// call id, so its completion replaces its start.
    pub(super) fn activity(&mut self, activity: ActivityItem) -> BackgroundWorkEvent {
        self.open_text = None;
        let id = activity
            .source_id
            .clone()
            .unwrap_or_else(|| activity.id.to_string());
        self.entry(id, SubagentTranscriptBody::Activity { activity })
    }

    pub(super) fn remember(&mut self, id: &str, call: ChildCall) {
        self.calls.insert(id.to_owned(), call);
    }

    pub(super) fn take(&mut self, id: &str) -> Option<ChildCall> {
        self.calls.remove(id)
    }

    fn entry(&self, id: String, body: SubagentTranscriptBody) -> BackgroundWorkEvent {
        BackgroundWorkEvent::Transcript {
            key: self.key.clone(),
            entry: SubagentTranscriptEntry { id, body },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BackgroundWorkKind;

    fn text_of(event: &BackgroundWorkEvent) -> (&str, &str, bool) {
        let BackgroundWorkEvent::Transcript { entry, .. } = event else {
            panic!("expected a transcript entry");
        };
        let SubagentTranscriptBody::Text { text, append } = &entry.body else {
            panic!("expected text");
        };
        (entry.id.as_str(), text.as_str(), *append)
    }

    /// Text runs into one entry until a tool call comes between; the call is
    /// keyed by its own id so its completion lands on the same row.
    #[test]
    fn text_runs_until_a_tool_call_comes_between() {
        let mut feed = SubagentFeed::new(BackgroundWorkKey::new(BackgroundWorkKind::Subagent, "p"));
        assert!(feed.text("").is_none());
        let first = feed.text("Looking ").unwrap();
        assert_eq!(text_of(&first), ("text-1", "Looking ", false));
        let more = feed.text("around.").unwrap();
        assert_eq!(text_of(&more), ("text-1", "around.", true));

        let call = ActivityItem::new(Some("toolu_9".into()), ActivityKind::Search, "auth", None, false);
        let BackgroundWorkEvent::Transcript { entry, .. } = feed.activity(call) else {
            panic!("expected a transcript entry");
        };
        assert_eq!(entry.id, "toolu_9");

        let after = feed.text("Found it.").unwrap();
        assert_eq!(text_of(&after), ("text-2", "Found it.", false));
        feed.close_text();
        assert_eq!(text_of(&feed.text("Next").unwrap()).0, "text-3");
    }
}
