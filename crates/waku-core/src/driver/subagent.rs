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
    /// Fork: what the open text entry holds, and how much of it requests
    /// that finished wrote — a retry that threw its answer away takes the
    /// entry back to that.
    open_body: String,
    settled_len: usize,
    /// Fork: requests finished so far; a request's retries share one line.
    request: u32,
    calls: HashMap<String, ChildCall>,
}

impl SubagentFeed {
    pub(super) fn new(key: BackgroundWorkKey) -> Self {
        Self {
            key,
            seq: 0,
            open_text: None,
            open_body: String::new(),
            settled_len: 0,
            request: 0,
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
                self.open_body.clear();
                self.settled_len = 0;
                (id, false)
            }
        };
        self.open_body.push_str(text);
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
        self.open_body.clear();
        self.settled_len = 0;
    }

    /// Fork: one of the sub-agent's requests finished; the text it wrote
    /// stays whatever a later retry does.
    pub(super) fn settle(&mut self) {
        self.settled_len = self.open_body.len();
        self.request += 1;
    }

    /// Fork: one of the sub-agent's requests is being sent again. With
    /// `discarded`, the text written since the last finished request is
    /// taken back (an emptied entry is not shown). `line` says so in the
    /// record — one line per request, updated by each of its retries; the
    /// answer that follows starts an entry of its own after it.
    pub(super) fn retry(&mut self, discarded: bool, line: String) -> Vec<BackgroundWorkEvent> {
        let mut events = Vec::new();
        if discarded && let Some(id) = self.open_text.clone() {
            let mut keep = self.settled_len.min(self.open_body.len());
            while !self.open_body.is_char_boundary(keep) {
                keep -= 1;
            }
            if keep < self.open_body.len() {
                self.open_body.truncate(keep);
                events.push(self.entry(
                    id,
                    SubagentTranscriptBody::Text {
                        text: self.open_body.clone(),
                        append: false,
                    },
                ));
            }
        }
        self.close_text();
        events.push(self.entry(
            format!("retry-{}", self.request),
            SubagentTranscriptBody::Text {
                text: line,
                append: false,
            },
        ));
        events
    }

    /// One tool call, whole, in its current state. Keyed by the provider's
    /// call id, so its completion replaces its start.
    pub(super) fn activity(&mut self, activity: ActivityItem) -> BackgroundWorkEvent {
        self.close_text();
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

    #[test]
    fn a_retry_takes_back_unsettled_text_and_keeps_one_line_per_request() {
        let mut feed = SubagentFeed::new(BackgroundWorkKey::new(BackgroundWorkKind::Subagent, "p"));
        feed.text("Half an ans").unwrap();
        let first = feed.retry(true, "*retry 1*".into());
        assert_eq!(first.len(), 2);
        assert_eq!(text_of(&first[0]), ("text-1", "", false), "the half answer is emptied");
        assert_eq!(text_of(&first[1]), ("retry-0", "*retry 1*", false));
        // The second attempt's text goes after the line, and its own retry
        // updates the same line.
        assert_eq!(text_of(&feed.text("Half ag").unwrap()).0, "text-2");
        let second = feed.retry(true, "*retry 2*".into());
        assert_eq!(text_of(&second[0]), ("text-2", "", false));
        assert_eq!(text_of(&second[1]), ("retry-0", "*retry 2*", false));
        // A retry that threw nothing away only updates its line.
        assert_eq!(feed.retry(false, "*retry 3*".into()).len(), 1);
    }

    #[test]
    fn text_settled_before_a_retry_stays() {
        let mut feed = SubagentFeed::new(BackgroundWorkKey::new(BackgroundWorkKind::Subagent, "p"));
        feed.text("Done with step one. ").unwrap();
        feed.settle();
        feed.text("Now st").unwrap();
        let events = feed.retry(true, "*retry*".into());
        assert_eq!(text_of(&events[0]), ("text-1", "Done with step one. ", false));
        assert_eq!(text_of(&events[1]).0, "retry-1", "the line belongs to the second request");
    }
}
