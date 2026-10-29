//! The question card answers the keyboard.
//!
//! Fork addition, after Zeron's question panel (crates/ui/src/composer.rs;
//! MIT, Copyright (c) 2026 Wing), on the model of the permission card
//! (permission_card.rs):
//!
//! - The card takes focus the first time a question shows, unless the person
//!   is writing in the composer, where a stray digit would answer for them.
//! - A bare digit picks that option and Enter on the card moves on, but only
//!   while the free-text answer is not focused — a digit typed there is part
//!   of the answer.
//! - Escape steps back to the previous question while the free-text answer is
//!   empty. On the first question it does what it did before, so it never
//!   swallows the way to stop the turn.
//! - Picking an option of a single-choice question moves to the next question
//!   after [`AUTO_ADVANCE`], time enough to see the pick land. The last
//!   question is never sent on its own: that stays a deliberate click.

use std::cell::{Cell, RefCell};

use gpui::{KeyBinding, actions};

use super::*;

actions!(waku_question_panel, [QuestionEscape]);

const CARD_CONTEXT: &str = "UserInputCard";

/// How long a single-choice pick shows before the next question.
const AUTO_ADVANCE: Duration = Duration::from_millis(220);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        QuestionEscape,
        Some(CARD_CONTEXT),
    )]);
}

#[derive(Default)]
pub(super) struct QuestionPanelState {
    focus: RefCell<Option<FocusHandle>>,
    focused_request: RefCell<Option<String>>,
    advance_generation: Cell<u64>,
}

/// The option a bare digit picks.
fn keyed_option(key: &str, options: usize) -> Option<usize> {
    let digit = key
        .parse::<usize>()
        .ok()
        .filter(|digit| (1..=9).contains(digit))?;
    (digit <= options).then_some(digit - 1)
}

impl Waku {
    fn question_focus(&self, cx: &mut App) -> FocusHandle {
        self.question_panel
            .focus
            .borrow_mut()
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// From `render_permission`, before the card renders.
    pub(super) fn focus_new_question(&self, request_id: &str, window: &mut Window, cx: &mut App) {
        if self.question_panel.focused_request.borrow().as_deref() == Some(request_id) {
            return;
        }
        *self.question_panel.focused_request.borrow_mut() = Some(request_id.to_owned());
        if self.composer.read(cx).content(cx).trim().is_empty() {
            let focus = self.question_focus(cx);
            window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        }
    }

    /// The card's root, made a focus stop that answers keys.
    pub(super) fn question_card_keys(
        &self,
        card: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = self.question_focus(cx);
        card.key_context(CARD_CONTEXT)
            .track_focus(&focus)
            .on_action(cx.listener(|this, _: &QuestionEscape, window, cx| {
                this.question_escape(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.question_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
    }

    fn free_answer(&self, window: &Window, cx: &App) -> (bool, bool) {
        let input = self.user_input_answer.read(cx);
        (
            input.focus_handle(cx).is_focused(window),
            input.content().trim().is_empty(),
        )
    }

    /// Returns whether the key answered.
    fn question_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.modifiers.modified() {
            return false;
        }
        let (answer_focused, _) = self.free_answer(window, cx);
        if answer_focused {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let Some(question) = self
            .selected_runtime()
            .and_then(|runtime| runtime.pending_user_input.as_ref())
            .and_then(|pending| pending.current_question())
        else {
            return false;
        };
        if let Some(index) = keyed_option(key, question.options.len()) {
            let label = question.options[index].label.clone();
            self.select_user_input_option(label, cx);
            return true;
        }
        let card_focused = self
            .question_panel
            .focus
            .borrow()
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        if key == "enter" && card_focused {
            self.advance_user_input(cx);
            return true;
        }
        false
    }

    fn question_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (_, answer_empty) = self.free_answer(window, cx);
        let can_go_back = self
            .selected_runtime()
            .and_then(|runtime| runtime.pending_user_input.as_ref())
            .is_some_and(|pending| pending.question_index > 0);
        if answer_empty && can_go_back {
            self.previous_user_input(cx);
            let focus = self.question_focus(cx);
            window.focus(&focus, cx);
        } else {
            cx.propagate();
        }
    }

    /// From `select_user_input_option`, once the pick is recorded.
    pub(super) fn after_user_input_selection(&mut self, cx: &mut Context<Self>) {
        let generation = self.question_panel.advance_generation.get().wrapping_add(1);
        self.question_panel.advance_generation.set(generation);
        let Some(pending) = self
            .selected_runtime()
            .and_then(|runtime| runtime.pending_user_input.as_ref())
        else {
            return;
        };
        let Some(question) = pending.current_question() else {
            return;
        };
        let is_last = pending.question_index + 1 >= pending.questions.len();
        let picked = pending
            .selections
            .get(&question.id)
            .is_some_and(|selected| !selected.is_empty());
        if question.multi_select || is_last || !picked {
            return;
        }
        let request_id = pending.request_id.clone();
        let question_index = pending.question_index;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AUTO_ADVANCE).await;
            let _ = this.update(cx, |this, cx| {
                let still_here = this.question_panel.advance_generation.get() == generation
                    && this
                        .selected_runtime()
                        .and_then(|runtime| runtime.pending_user_input.as_ref())
                        .is_some_and(|pending| {
                            pending.request_id == request_id
                                && pending.question_index == question_index
                        });
                if still_here {
                    this.advance_user_input(cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digit_picks_an_existing_option_only() {
        assert_eq!(keyed_option("1", 3), Some(0));
        assert_eq!(keyed_option("3", 3), Some(2));
        assert_eq!(keyed_option("4", 3), None);
        assert_eq!(keyed_option("0", 3), None);
        assert_eq!(keyed_option("a", 3), None);
        assert_eq!(keyed_option("10", 12), None);
    }
}
