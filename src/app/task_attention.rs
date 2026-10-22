//! Which tasks wait on the person, and for what.
//!
//! Fork addition, after Claude Desktop's sidebar, which tells the sessions
//! that are working apart from the ones that need you. Waku already tagged a
//! waiting task 「等待确认」, but only while the task itself was waiting: a
//! team member's request reaching an idle captain left no mark at all, and a
//! folded group said nothing about it. Here the runtime's pending requests
//! are the one source — a permission, a finished plan, a question, a
//! teammate's request — so the mark goes the moment the request is answered,
//! the turn ends or is stopped, and never outlives what it is about.
//!
//! The kinds are told apart by text and icon, never by colour alone.

use super::permission_card::is_plan_approval;
use super::sidebar::SidebarRow;
use super::transcript::{mix, mix_uuid};
use super::*;

/// What a task waits on the person for, most pressing first.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum AttentionKind {
    /// A tool call to allow or deny.
    Approval,
    /// A finished plan to approve.
    Plan,
    /// A question to answer.
    Question,
    /// A team member's request, raised through its captain.
    Teammate,
}

/// The most pressing request a task holds, and how many it holds in all.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct Attention {
    pub(super) kind: AttentionKind,
    pub(super) count: usize,
}

impl Attention {
    /// What the row's tag and tooltip say.
    pub(super) fn label(self) -> String {
        match self.kind {
            AttentionKind::Approval => tr!("needs_you.approval"),
            AttentionKind::Plan => tr!("needs_you.plan"),
            AttentionKind::Question => tr!("needs_you.question"),
            AttentionKind::Teammate => tr!("needs_you.teammate"),
        }
    }
}

/// One pending permission, reduced to what classifying it reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct PendingRequest {
    pub(super) teammate: bool,
    pub(super) plan: bool,
}

/// The attention a task's pending requests call for, or `None` when it
/// holds none.
pub(super) fn classify(
    permissions: impl IntoIterator<Item = PendingRequest>,
    question: bool,
    computer: bool,
) -> Option<Attention> {
    let mut kind: Option<AttentionKind> = None;
    let mut count = 0;
    let mut note = |found: AttentionKind| {
        count += 1;
        kind = Some(kind.map_or(found, |kind| kind.min(found)));
    };
    for request in permissions {
        note(if request.teammate {
            AttentionKind::Teammate
        } else if request.plan {
            AttentionKind::Plan
        } else {
            AttentionKind::Approval
        });
    }
    if question {
        note(AttentionKind::Question);
    }
    if computer {
        note(AttentionKind::Approval);
    }
    kind.map(|kind| Attention { kind, count })
}

/// The task after `current` in `order` that needs the person, wrapping
/// around; the first such task when `current` is not one of them. `None`
/// when no task needs the person, or only the current one does.
pub(super) fn next_needs_you(order: &[Uuid], needs: &HashSet<Uuid>, current: Option<Uuid>) -> Option<Uuid> {
    let start = current
        .and_then(|current| order.iter().position(|id| *id == current))
        .map_or(0, |index| index + 1);
    (0..order.len())
        .map(|offset| order[(start + offset) % order.len()])
        .find(|id| needs.contains(id) && Some(*id) != current)
}

impl Waku {
    /// What the task waits on the person for, from its runtime alone: a
    /// task with no runtime holds no request, whatever its stored status.
    pub(super) fn task_attention(&self, session_id: Uuid) -> Option<Attention> {
        let runtime = self.runtimes.get(&session_id)?;
        classify(
            runtime.pending_permissions.iter().map(|permission| PendingRequest {
                teammate: agent_teams::requests::is_member_request(&permission.request_id),
                plan: is_plan_approval(permission),
            }),
            runtime.pending_user_input.is_some(),
            runtime.pending_computer_approval.is_some(),
        )
    }

    /// Every task that needs the person now.
    pub(super) fn needs_you_tasks(&self) -> HashSet<Uuid> {
        self.runtimes
            .keys()
            .copied()
            .filter(|session_id| self.task_attention(*session_id).is_some())
            .collect()
    }

    /// Folded into the sidebar's row fingerprint, so a request arriving or
    /// being answered re-sorts the rows and redraws a folded group's badge.
    /// Order-independent: the runtimes are a map.
    pub(super) fn mix_task_attention_fingerprint(&self, fingerprint: u64) -> u64 {
        let combined = self.runtimes.keys().fold(0u64, |combined, session_id| {
            match self.task_attention(*session_id) {
                Some(attention) => combined.wrapping_add(mix(
                    mix_uuid(attention.kind as u64, *session_id),
                    attention.count as u64,
                )),
                None => combined,
            }
        });
        mix(fingerprint, combined)
    }

    /// The Dock shows how many tasks need the person (macOS). Only a change
    /// reaches AppKit, deferred out of the frame.
    pub(super) fn note_task_attention(&mut self, cx: &mut Context<Self>) {
        let count = self
            .runtimes
            .keys()
            .filter(|session_id| self.task_attention(**session_id).is_some())
            .count();
        if self.task_marks.dock_badge == count {
            return;
        }
        self.task_marks.dock_badge = count;
        cx.defer(move |_| crate::platform::set_dock_badge(count));
    }

    /// ⇧⌘A: the next task that needs the person, in the sidebar's order.
    pub(super) fn jump_to_needs_you_action(
        &mut self,
        _: &super::shortcuts::JumpToNeedsYou,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_needs_you(window, cx);
    }

    pub(super) fn jump_to_needs_you(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let needs = self.needs_you_tasks();
        if needs.is_empty() {
            self.show_toast(tr!("needs_you.none"));
            cx.notify();
            return;
        }
        // The sidebar's order first, then tasks it does not list right now.
        let mut order: Vec<Uuid> = self
            .sidebar_row_cache
            .borrow()
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(session_id) => Some(*session_id),
                _ => None,
            })
            .collect();
        for session in &self.state.sessions {
            if needs.contains(&session.id) && !order.contains(&session.id) {
                order.push(session.id);
            }
        }
        let current = self.state.selected_session;
        let Some(target) = next_needs_you(&order, &needs, current) else {
            // The task on screen is the only one; it already shows its card.
            self.show_toast(tr!("needs_you.only_this"));
            cx.notify();
            return;
        };
        self.settings_page = None;
        self.select_session(target, cx);
        // The card above the composer answers to its digit keys from there.
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(teammate: bool, plan: bool) -> PendingRequest {
        PendingRequest { teammate, plan }
    }

    #[test]
    fn nothing_pending_needs_nobody() {
        assert_eq!(classify([], false, false), None);
    }

    #[test]
    fn the_most_pressing_kind_names_the_task_and_every_request_counts() {
        let attention = classify([request(true, false), request(false, true)], true, false).unwrap();
        assert_eq!(attention.kind, AttentionKind::Plan);
        assert_eq!(attention.count, 3);

        let attention = classify([request(false, false), request(false, true)], false, false).unwrap();
        assert_eq!(attention.kind, AttentionKind::Approval);
        assert_eq!(attention.count, 2);
    }

    #[test]
    fn a_teammate_request_alone_is_a_teammate_request() {
        let attention = classify([request(true, false)], false, false).unwrap();
        assert_eq!(attention, Attention { kind: AttentionKind::Teammate, count: 1 });
    }

    #[test]
    fn a_question_or_a_desktop_approval_counts() {
        assert_eq!(classify([], true, false).unwrap().kind, AttentionKind::Question);
        assert_eq!(classify([], false, true).unwrap().kind, AttentionKind::Approval);
    }

    #[test]
    fn the_jump_goes_to_the_next_waiting_task_and_wraps() {
        let ids: Vec<Uuid> = (1..=4).map(Uuid::from_u128).collect();
        let needs: HashSet<Uuid> = [ids[0], ids[2]].into_iter().collect();
        assert_eq!(next_needs_you(&ids, &needs, Some(ids[1])), Some(ids[2]));
        assert_eq!(next_needs_you(&ids, &needs, Some(ids[2])), Some(ids[0]));
        assert_eq!(next_needs_you(&ids, &needs, None), Some(ids[0]));
        let only: HashSet<Uuid> = [ids[3]].into_iter().collect();
        assert_eq!(next_needs_you(&ids, &only, Some(ids[3])), None);
        assert_eq!(next_needs_you(&ids, &HashSet::new(), None), None);
    }
}
