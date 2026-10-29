//! Each task remembers where its transcript was scrolled.
//!
//! Fork addition, after Zeron's per-chat scroll memory
//! (crates/ui/src/transcript.rs; MIT, Copyright (c) 2026 Wing). Upstream
//! resets the transcript to its tail whenever the selection moves to another
//! task, so reading back through one task, glancing at another and returning
//! lost the place. The place is kept by row identity — the row at the top of
//! the viewport and how far into it — not by row number or pixel offset, both
//! of which shift as rows fold and grow. A row that no longer exists falls
//! back to the nearest earlier message, then to the tail. A task left at its
//! tail returns to its tail, which is what the reset already does.

use super::*;

/// How many tasks keep a remembered place.
const CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
enum SavedScroll {
    Tail,
    Row {
        kind: TranscriptRowKind,
        offset: f32,
    },
}

#[derive(Default)]
pub(super) struct TranscriptMemory {
    /// Most recently left last.
    entries: Vec<(Uuid, SavedScroll)>,
}

impl TranscriptMemory {
    fn insert(&mut self, session_id: Uuid, saved: SavedScroll) {
        self.entries.retain(|(id, _)| *id != session_id);
        self.entries.push((session_id, saved));
        if self.entries.len() > CAPACITY {
            self.entries.remove(0);
        }
    }

    fn get(&self, session_id: Uuid) -> Option<SavedScroll> {
        self.entries
            .iter()
            .find(|(id, _)| *id == session_id)
            .map(|(_, saved)| *saved)
    }
}

impl Waku {
    /// Before the selection leaves the current task.
    pub(super) fn remember_transcript_scroll(&self) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let saved = if self.transcript_is_scrolled.get() {
            let top = self.active_transcript_rows().logical_scroll_top();
            self.transcript_row_kinds
                .borrow()
                .get(top.item_ix)
                .map_or(SavedScroll::Tail, |kind| SavedScroll::Row {
                    kind: *kind,
                    offset: f32::from(top.offset_in_item),
                })
        } else {
            SavedScroll::Tail
        };
        self.fork_motion
            .transcript_memory
            .borrow_mut()
            .insert(session_id, saved);
    }

    /// After the transcript was reset for the newly selected task.
    pub(super) fn restore_transcript_scroll(&self) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        // A turn just sent pins its prompt; that wins over an old place.
        if self.transcript_anchor.get().is_some() {
            return;
        }
        let saved = self.fork_motion.transcript_memory.borrow().get(session_id);
        let Some(SavedScroll::Row { kind, offset }) = saved else {
            return;
        };
        let Some(item_ix) = restore_index(&self.transcript_row_kinds.borrow(), kind) else {
            return;
        };
        self.active_transcript_rows().scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(offset),
        });
        self.transcript_is_scrolled.set(true);
        self.transcript_anchor_following.set(false);
        self.transcript_tail_recheck.set(false);
    }
}

/// Where `kind` is now, or the nearest earlier message when it is gone.
fn restore_index(kinds: &[TranscriptRowKind], kind: TranscriptRowKind) -> Option<usize> {
    if let Some(index) = kinds.iter().position(|candidate| *candidate == kind) {
        return Some(index);
    }
    let TranscriptRowKind::Message(message) = kind else {
        return None;
    };
    kinds
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| match candidate {
            TranscriptRowKind::Message(earlier) if *earlier <= message => Some((*earlier, index)),
            _ => None,
        })
        .max_by_key(|(earlier, _)| *earlier)
        .map(|(_, index)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_found_where_it_is_now() {
        let kinds = [
            TranscriptRowKind::Message(0),
            TranscriptRowKind::TurnBlock(0),
            TranscriptRowKind::Message(1),
        ];
        assert_eq!(
            restore_index(&kinds, TranscriptRowKind::TurnBlock(0)),
            Some(1)
        );
    }

    #[test]
    fn a_folded_away_message_falls_back_to_the_nearest_earlier_one() {
        let kinds = [
            TranscriptRowKind::Message(0),
            TranscriptRowKind::Message(4),
            TranscriptRowKind::Message(9),
        ];
        assert_eq!(
            restore_index(&kinds, TranscriptRowKind::Message(7)),
            Some(1)
        );
        assert_eq!(restore_index(&kinds, TranscriptRowKind::TurnBlock(3)), None);
    }

    #[test]
    fn the_memory_keeps_the_most_recent_tasks() {
        let mut memory = TranscriptMemory::default();
        for n in 0..(CAPACITY as u128 + 10) {
            memory.insert(Uuid::from_u128(n), SavedScroll::Tail);
        }
        assert_eq!(memory.entries.len(), CAPACITY);
        assert_eq!(memory.get(Uuid::from_u128(0)), None);
        assert_eq!(
            memory.get(Uuid::from_u128(CAPACITY as u128 + 9)),
            Some(SavedScroll::Tail)
        );

        memory.insert(Uuid::from_u128(20), SavedScroll::Tail);
        assert_eq!(
            memory.entries.last().map(|(id, _)| *id),
            Some(Uuid::from_u128(20))
        );
    }
}
