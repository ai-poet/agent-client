//! A sub-agent's record, kept beside its background-work entry.
//!
//! Drivers stream it as `BackgroundWorkEvent::Transcript` entries (see
//! `waku-core/src/driver/subagent.rs`): text the sub-agent wrote, and its
//! tool calls as whole `ActivityItem`s, each keyed by an id so a later entry
//! replaces — or, for text, extends — an earlier one. This holds them in
//! order, bounded, with the parsed markdown for each text entry, so the panel
//! (`subagent_panel.rs`) only renders what is already here.
//!
//! Parsing happens on the output refresh tick, not per delta and not while
//! rendering: a sub-agent streams a delta per token, and the panel repaints
//! at most ten times a second.

use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use super::*;
use crate::model::{SubagentTranscriptBody, SubagentTranscriptEntry};

/// Entries one record keeps. A sub-agent that ran longer shows its latest
/// steps and says how many earlier ones are not shown; the parent's
/// transcript and the sub-agent's final report are unaffected.
pub(super) const MAX_SUBAGENT_ROWS: usize = 400;

pub(super) enum SubagentRow {
    Text {
        id: String,
        text: String,
        view: MarkdownView,
    },
    Activity {
        id: String,
        activity: ActivityItem,
    },
}

impl SubagentRow {
    fn id(&self) -> &str {
        match self {
            Self::Text { id, .. } | Self::Activity { id, .. } => id,
        }
    }
}

pub(super) struct SubagentTranscript {
    rows: VecDeque<SubagentRow>,
    /// Entries dropped off the front to stay within [`MAX_SUBAGENT_ROWS`].
    trimmed: usize,
    /// Text entries whose markdown has not caught up with their text.
    stale_text: HashSet<String>,
    pub(super) prompt_expanded: bool,
    pub(super) scroll: ScrollHandle,
    pub(super) scrollbar: Rc<ScrollbarState>,
}

impl Default for SubagentTranscript {
    fn default() -> Self {
        Self {
            rows: VecDeque::new(),
            trimmed: 0,
            stale_text: HashSet::new(),
            prompt_expanded: false,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }
}

impl SubagentTranscript {
    pub(super) fn apply(&mut self, entry: SubagentTranscriptEntry) {
        // A record grows at its end; the entry being updated is almost
        // always among the last few.
        let index = self.rows.iter().rposition(|row| row.id() == entry.id);
        match entry.body {
            SubagentTranscriptBody::Text { text, append } => {
                match index.map(|index| &mut self.rows[index]) {
                    Some(SubagentRow::Text { text: current, .. }) => {
                        if append {
                            current.push_str(&text);
                        } else {
                            *current = text;
                        }
                    }
                    Some(row) => {
                        *row = SubagentRow::Text {
                            id: entry.id.clone(),
                            text,
                            view: MarkdownView::new(),
                        };
                    }
                    None => self.rows.push_back(SubagentRow::Text {
                        id: entry.id.clone(),
                        text,
                        view: MarkdownView::new(),
                    }),
                }
                self.stale_text.insert(entry.id);
            }
            SubagentTranscriptBody::Activity { mut activity } => {
                match index.map(|index| &mut self.rows[index]) {
                    Some(SubagentRow::Activity {
                        activity: current, ..
                    }) => {
                        // The row's identity is the first one it was shown
                        // with: its expanded state is keyed by it.
                        activity.id = current.id;
                        *current = activity;
                    }
                    Some(row) => {
                        *row = SubagentRow::Activity {
                            id: entry.id,
                            activity,
                        };
                    }
                    None => self.rows.push_back(SubagentRow::Activity {
                        id: entry.id,
                        activity,
                    }),
                }
            }
        }
        while self.rows.len() > MAX_SUBAGENT_ROWS {
            if let Some(SubagentRow::Text { id, .. }) = self.rows.pop_front() {
                self.stale_text.remove(&id);
            }
            self.trimmed += 1;
        }
    }

    /// Bring the markdown of changed text entries up to date. `true` when
    /// anything changed.
    pub(super) fn refresh(&mut self) -> bool {
        if self.stale_text.is_empty() {
            return false;
        }
        let stale = std::mem::take(&mut self.stale_text);
        for row in &mut self.rows {
            if let SubagentRow::Text { id, text, view } = row
                && stale.contains(id.as_str())
            {
                view.set_text(text, false);
            }
        }
        true
    }

    pub(super) fn rows(&self) -> impl Iterator<Item = &SubagentRow> {
        self.rows.iter()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn trimmed(&self) -> usize {
        self.trimmed
    }

    /// Whether any text entry is waiting for [`Self::refresh`].
    pub(super) fn is_stale(&self) -> bool {
        !self.stale_text.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActivityKind;

    fn text(id: &str, text: &str, append: bool) -> SubagentTranscriptEntry {
        SubagentTranscriptEntry {
            id: id.into(),
            body: SubagentTranscriptBody::Text {
                text: text.into(),
                append,
            },
        }
    }

    fn call(id: &str, complete: bool) -> SubagentTranscriptEntry {
        SubagentTranscriptEntry {
            id: id.into(),
            body: SubagentTranscriptBody::Activity {
                activity: ActivityItem::new(
                    Some(id.into()),
                    ActivityKind::Search,
                    "auth",
                    None,
                    complete,
                ),
            },
        }
    }

    fn texts(record: &SubagentTranscript) -> Vec<String> {
        record
            .rows()
            .filter_map(|row| match row {
                SubagentRow::Text { text, .. } => Some(text.clone()),
                SubagentRow::Activity { .. } => None,
            })
            .collect()
    }

    #[test]
    fn text_extends_or_replaces_its_entry() {
        let mut record = SubagentTranscript::default();
        record.apply(text("t1", "Looking ", false));
        record.apply(text("t1", "around.", true));
        assert_eq!(texts(&record), ["Looking around."]);
        record.apply(text("t1", "Rewritten.", false));
        assert_eq!(texts(&record), ["Rewritten."]);
        assert!(record.is_stale());
        assert!(record.refresh());
        assert!(!record.is_stale());
        assert!(!record.refresh());
    }

    /// A call's completion replaces its start in place and keeps the row's
    /// identity, so a row the user opened stays open.
    #[test]
    fn a_completion_replaces_its_call_and_keeps_its_identity() {
        let mut record = SubagentTranscript::default();
        record.apply(call("c1", false));
        record.apply(text("t1", "Found it.", false));
        let first = match record.rows().next() {
            Some(SubagentRow::Activity { activity, .. }) => activity.id,
            _ => panic!("expected the call first"),
        };
        record.apply(call("c1", true));
        let rows: Vec<_> = record.rows().collect();
        assert_eq!(rows.len(), 2);
        let SubagentRow::Activity { activity, .. } = rows[0] else {
            panic!("the call stays where it was");
        };
        assert_eq!(activity.id, first);
        assert!(activity.complete);
    }

    #[test]
    fn a_long_record_keeps_its_latest_steps() {
        let mut record = SubagentTranscript::default();
        for index in 0..MAX_SUBAGENT_ROWS + 5 {
            record.apply(call(&format!("c{index}"), true));
        }
        assert_eq!(record.rows().count(), MAX_SUBAGENT_ROWS);
        assert_eq!(record.trimmed(), 5);
        let Some(SubagentRow::Activity { id, .. }) = record.rows().next() else {
            panic!("expected a call");
        };
        assert_eq!(id, "c5");
    }
}
