//! Virtualized message-list state, adapted from Longbridge GPUI Kit's
//! `MessageScrollerState` (Apache-2.0). Presentation and message data remain
//! with the caller.

use std::ops::Range;

use gpui::{Context, FollowMode, ListAlignment, ListOffset, ListState, Pixels, px};

const LIST_OVERDRAW: Pixels = px(400.0);

/// Entity-owned list bookkeeping with tail following and measured-row repair.
pub struct MessageScrollerState {
    list_state: ListState,
}

impl MessageScrollerState {
    pub fn new(item_count: usize, cx: &mut Context<Self>) -> Self {
        let list_state = ListState::new(item_count, ListAlignment::Top, LIST_OVERDRAW);
        list_state.set_follow_mode(FollowMode::Tail);

        let weak_state = cx.weak_entity();
        list_state.set_scroll_handler(move |_, _, cx| {
            let weak_state = weak_state.clone();
            cx.defer(move |cx| {
                let _ = weak_state.update(cx, |_, cx| cx.notify());
            });
        });

        Self { list_state }
    }

    pub fn list_state(&self) -> &ListState {
        &self.list_state
    }

    pub fn item_count(&self) -> usize {
        self.list_state.item_count()
    }

    pub fn is_scrolled_up(&self) -> bool {
        self.list_state.max_offset_for_scrollbar().y > px(0.0)
            && !self.list_state.is_following_tail()
            && !self.list_state.is_scrolled_to_end().unwrap_or(false)
    }

    pub fn is_following_tail(&self) -> bool {
        self.list_state.is_following_tail()
    }

    pub fn logical_scroll_top(&self) -> ListOffset {
        self.list_state.logical_scroll_top()
    }

    pub fn reset(&mut self, item_count: usize, cx: &mut Context<Self>) {
        self.list_state.reset(item_count);
        self.list_state.set_follow_mode(FollowMode::Tail);
        cx.notify();
    }

    pub fn splice(
        &mut self,
        old_range: Range<usize>,
        count: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.valid_range(&old_range) {
            return false;
        }

        let neighbor = old_range.start.checked_sub(1);
        let touches_start = old_range.start == 0;
        self.list_state.splice(old_range, count);

        if let Some(last) = self.list_state.item_count().checked_sub(1) {
            self.list_state.remeasure_items(last..last + 1);
            if let Some(neighbor) = neighbor.filter(|neighbor| *neighbor != last) {
                self.list_state.remeasure_items(neighbor..neighbor + 1);
            }
        }
        if touches_start {
            for index in [0, count] {
                if index < self.list_state.item_count() {
                    self.list_state.remeasure_items(index..index + 1);
                }
            }
        }

        cx.notify();
        true
    }

    pub fn append(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        let item_count = self.list_state.item_count();
        self.splice(item_count..item_count, count, cx)
    }

    pub fn prepend(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        self.splice(0..0, count, cx)
    }

    pub fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.list_state.remeasure();
        cx.notify();
    }

    pub fn remeasure_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> bool {
        if !self.valid_range(&range) {
            return false;
        }
        self.list_state.remeasure_items(range);
        cx.notify();
        true
    }

    pub fn scroll_to_item(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if index >= self.list_state.item_count() {
            return false;
        }
        self.list_state.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.0),
        });
        cx.notify();
        true
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.list_state.scroll_to_end();
        cx.notify();
    }

    fn valid_range(&self, range: &Range<usize>) -> bool {
        range.start <= range.end && range.end <= self.list_state.item_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    #[gpui::test]
    fn splice_append_prepend_and_remeasure_preserve_state(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|cx| MessageScrollerState::new(3, cx));
        cx.update(|cx| {
            assert_eq!(state.read(cx).item_count(), 3);
            assert!(state.read(cx).is_following_tail());
            state.update(cx, |state, cx| {
                assert!(!state.scroll_to_item(3, cx));
                assert!(state.append(2, cx));
                assert!(state.prepend(1, cx));
                assert_eq!(state.item_count(), 6);
                assert!(!state.splice(5..7, 0, cx));
                assert!(state.remeasure_items(0..6, cx));
                assert!(!state.remeasure_items(6..7, cx));
                assert!(state.scroll_to_item(2, cx));
                assert!(state.is_following_tail() == false);
                state.scroll_to_end(cx);
                assert!(state.is_following_tail());
                state.reset(2, cx);
                assert_eq!(state.item_count(), 2);
            });
        });
    }

    #[test]
    fn prepend_moves_the_logical_anchor_with_the_old_row() {
        let rows = ListState::new(5, ListAlignment::Top, LIST_OVERDRAW);
        rows.scroll_to(ListOffset {
            item_ix: 2,
            offset_in_item: px(12.0),
        });
        rows.splice(0..0, 2);
        assert_eq!(rows.item_count(), 7);
        let anchor = rows.logical_scroll_top();
        assert_eq!(anchor.item_ix, 4);
        assert_eq!(anchor.offset_in_item, px(12.0));
        rows.splice(7..7, 1);
        assert_eq!(rows.logical_scroll_top().item_ix, 4);
    }
}
