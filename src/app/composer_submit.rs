//! The composer's send, queue and stop control.
//!
//! Fork file. The block was moved out of `render_composer` (composer.rs) so
//! the fork can change it without touching the upstream function. Upstream
//! showed Stop and Queue side by side during a turn; after Zeron's composer
//! (crates/ui/src/composer.rs; MIT, Copyright (c) 2026 Wing) it is one button
//! that is Send when idle, Queue while a turn runs and the composer holds a
//! draft, Stop while it runs with nothing to send (and whenever Escape has
//! armed the stop), and a spinner while the task prepares. One element id
//! across all of them keeps keyboard focus on the button as it changes.
//! It is a focus stop with a visible ring and answers Enter and Space, as
//! AGENTS.md asks of every clickable control.
//! A focusable element takes focus on mouse-down, so activating one hands
//! focus back to the text field: clicking Send must not leave the next
//! keystroke with nowhere to go.

use super::composer::ComposerSubmitAction;
use super::*;
use crate::ui::ActivationExt as _;

impl Waku {
    pub(super) fn render_submit_controls(
        &self,
        action: ComposerSubmitAction,
        escape_stop_armed: bool,
        can_send: bool,
        no_providers: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        match action {
            ComposerSubmitAction::Preparing => div()
                .id("send-or-stop")
                .w(px(26.0))
                .h(px(26.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .bg(theme.overlay_strong)
                .child(motion::spin(icon(
                    "icons/loader-circle.svg",
                    15.0,
                    theme.text_secondary,
                )))
                .tooltip(Tooltip::text(tr!("composer.preparing_task"))),
            // A draft during a running turn queues behind it; Escape still
            // stops the turn, and once Escape is armed the button says so.
            ComposerSubmitAction::Stop if can_send && !escape_stop_armed => {
                submit_button("send-or-stop", theme)
                    .bg(theme.inverse)
                    .hover(|element| element.opacity(0.9))
                    .active(|element| element.opacity(0.8))
                    .child(icon("icons/arrow-up.svg", 16.0, theme.on_inverse))
                    .tooltip(Tooltip::text(tr!("composer.queue_followup")))
                    .on_activation(cx, |this, window, cx| {
                        this.submit_composer_draft(cx);
                        this.refocus_composer(window, cx);
                    })
            }
            ComposerSubmitAction::Stop => submit_button("send-or-stop", theme)
                .bg(theme.overlay_strong)
                .hover(|element| element.bg(theme.danger_soft))
                .active(|element| element.opacity(0.8))
                .when(escape_stop_armed, |element| {
                    element.child(
                        div()
                            .text_size(sp(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Esc"),
                    )
                })
                .when(!escape_stop_armed, |element| {
                    element.child(icon("icons/stop.svg", 18.0, theme.text))
                })
                .tooltip(Tooltip::text(tr!("composer.stop")))
                .on_activation(cx, |this, window, cx| {
                    this.cancel_turn(cx);
                    this.refocus_composer(window, cx);
                }),
            ComposerSubmitAction::Send => submit_button("send-or-stop", theme)
                .bg(if can_send {
                    theme.inverse
                } else {
                    theme.overlay_strong
                })
                .when(can_send, |element| {
                    element
                        .hover(|element| element.opacity(0.9))
                        .active(|element| element.opacity(0.8))
                })
                .child(icon(
                    "icons/arrow-up.svg",
                    16.0,
                    if can_send {
                        theme.on_inverse
                    } else {
                        theme.text_ghost
                    },
                ))
                // Says why the button is dead, for the case the draft is
                // ready and the machine is not.
                .when(no_providers, |element| {
                    element.tooltip(Tooltip::text(tr!("composer.no_providers")))
                })
                .on_activation(cx, |this, window, cx| {
                    this.submit_composer_draft(cx);
                    this.refocus_composer(window, cx);
                }),
        }
    }

    /// Send, or queue behind a running turn, whatever the composer holds.
    fn submit_composer_draft(&mut self, cx: &mut Context<Self>) {
        let prompt = self.composer.read(cx).content(cx).to_owned();
        if let Some(submission) = self.submission_with_attachments(&prompt, cx) {
            self.composer.update(cx, |input, cx| input.clear(cx));
            self.submit_composer_submission(submission, cx);
        }
    }

    fn refocus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
    }
}

/// A round 26px control that is a focus stop with a visible ring.
fn submit_button(id: &'static str, theme: Theme) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .w(px(26.0))
        .h(px(26.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|style| style.border_color(theme.accent))
}
