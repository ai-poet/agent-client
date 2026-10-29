//! Queued follow-ups can be put in a different order.
//!
//! Fork addition, after Zeron's queue (crates/ui/src/queue.rs; MIT,
//! Copyright (c) 2026 Wing). Upstream sends queued messages strictly in the
//! order they were written. A focused row now moves with Alt-↑ / Alt-↓, and
//! its menu can move it up, down, or to the front so it goes next. The new
//! order is saved like any other change to the queue.

use super::*;

/// Where `index` lands after moving by `delta` in a list of `len`, or `None`
/// when it cannot move that way.
fn moved_index(index: usize, delta: isize, len: usize) -> Option<usize> {
    let target = index.checked_add_signed(delta)?;
    (target < len && target != index).then_some(target)
}

impl Waku {
    /// Move a queued message by `delta` places, or to the front with
    /// `isize::MIN`.
    pub(super) fn move_queued_message(
        &mut self,
        session_id: Uuid,
        message_id: Uuid,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.state.session_mut(session_id) else {
            return;
        };
        let queue = &mut session.queued_messages;
        let Some(index) = queue.iter().position(|message| message.id == message_id) else {
            return;
        };
        let target = if delta == isize::MIN {
            (index > 0).then_some(0)
        } else {
            moved_index(index, delta, queue.len())
        };
        let Some(target) = target else {
            return;
        };
        let message = queue.remove(index);
        queue.insert(target, message);
        self.save();
        cx.notify();
    }

    /// A queued row's reordering keys.
    pub(super) fn queue_row_keys(
        &self,
        row: Stateful<Div>,
        session_id: Uuid,
        message_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        row.on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            let modifiers = event.keystroke.modifiers;
            if !modifiers.alt || modifiers.control || modifiers.platform || modifiers.shift {
                return;
            }
            let delta = match event.keystroke.key.as_str() {
                "up" => -1,
                "down" => 1,
                _ => return,
            };
            this.move_queued_message(session_id, message_id, delta, cx);
            cx.stop_propagation();
        }))
    }
}

/// Menu entries for a queued row: send it next, or move it one place.
pub(super) fn queue_menu_items(
    waku: &WeakEntity<Waku>,
    session_id: Uuid,
    message_id: Uuid,
    cx: &App,
) -> Vec<MenuItem> {
    let Some((index, len)) = waku.upgrade().and_then(|waku| {
        let session = waku
            .read(cx)
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)?;
        let index = session
            .queued_messages
            .iter()
            .position(|message| message.id == message_id)?;
        Some((index, session.queued_messages.len()))
    }) else {
        return Vec::new();
    };
    let item = |label: String, icon: &'static str, delta: isize| {
        let waku = waku.clone();
        MenuItem::new(label, move |_, cx| {
            let _ = waku.update(cx, |this, cx| {
                this.move_queued_message(session_id, message_id, delta, cx);
            });
        })
        .icon(icon)
    };
    let mut items = Vec::new();
    if index > 1 {
        items.push(item(
            tr!("composer.send_next"),
            "icons/corner-down-right.svg",
            isize::MIN,
        ));
    }
    if index > 0 {
        items.push(item(
            tr!("composer.queue_move_up"),
            "icons/arrow-up.svg",
            -1,
        ));
    }
    if index + 1 < len {
        items.push(item(
            tr!("composer.queue_move_down"),
            "icons/arrow-down.svg",
            1,
        ));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_moves_within_the_queue_only() {
        assert_eq!(moved_index(0, -1, 3), None);
        assert_eq!(moved_index(0, 1, 3), Some(1));
        assert_eq!(moved_index(2, 1, 3), None);
        assert_eq!(moved_index(2, -1, 3), Some(1));
        assert_eq!(moved_index(1, 0, 3), None);
    }
}
