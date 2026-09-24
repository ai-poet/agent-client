//! The failure of the latest turn, shown above the composer.
//!
//! Fork addition. A failed turn used to say so in a toast gone after five
//! seconds, or in an assistant message the provider never wrote. The failure
//! now stays on the turn (`AgentTurn::error`) and this banner shows it until
//! the person retries, dismisses it, or starts another turn.

use super::*;

/// Past this the message is clipped to two lines until its details open.
const COLLAPSED_CHARS: usize = 160;

impl Waku {
    pub(super) fn render_error_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.selected_session()?;
        if session.is_busy() || self.submission_preparations.contains(&session.id) {
            return None;
        }
        let turn = session
            .turns
            .last()
            .filter(|turn| turn.status == TurnStatus::Failed)?;
        let error = turn.error.as_ref().filter(|error| !error.dismissed)?;
        let theme = Theme::current(cx);
        let session_id = session.id;
        let turn_id = turn.id;
        let message = error.message.trim().to_owned();
        let long = message.lines().count() > 2 || message.chars().count() > COLLAPSED_CHARS;
        let details_open = self.error_details_open == Some(turn_id);
        let copy_text = message.clone();

        let button = |id: &str, label: String, primary: bool| {
            div()
                .id(SharedString::from(format!("error-banner-{id}-{turn_id}")))
                .h(px(26.0))
                .px(px(10.0))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .cursor_default()
                .text_size(sp(12.5))
                .font_weight(FontWeight::MEDIUM)
                .when(primary, |button| {
                    button
                        .bg(theme.inverse)
                        .text_color(theme.on_inverse)
                        .hover(|button| button.opacity(0.9))
                })
                .when(!primary, |button| {
                    button
                        .border_1()
                        .border_color(theme.border_strong)
                        .text_color(theme.text_secondary)
                        .hover(|button| button.bg(theme.overlay).text_color(theme.text))
                })
                .active(|button| button.opacity(0.8))
                .child(label)
        };

        let mut actions = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .mt(px(10.0))
            .child(
                button("retry", tr!("error_banner.retry"), true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.retry_failed_turn(session_id, cx);
                    },
                )),
            )
            .child(
                button("copy", tr!("common.copy"), false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        this.show_toast(tr!("common.copied"));
                        cx.notify();
                    },
                )),
            );
        if long {
            let label = if details_open {
                tr!("error_banner.hide_details")
            } else {
                tr!("error_banner.show_details")
            };
            actions = actions.child(button("details", label, false).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.error_details_open = (!details_open).then_some(turn_id);
                    cx.notify();
                },
            )));
        }

        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon("icons/circle-x.svg", 14.0, theme.danger))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("error_banner.title")),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "error-banner-dismiss-{turn_id}"
                    )))
                    .size(px(22.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_default()
                    .hover(|button| button.bg(theme.overlay_strong))
                    .child(icon("icons/x.svg", 12.0, theme.text_tertiary))
                    .tooltip(Tooltip::text(tr!("error_banner.dismiss")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismiss_turn_error(session_id, turn_id, cx);
                    })),
            );
        let body = div()
            .id(SharedString::from(format!(
                "error-banner-message-{turn_id}"
            )))
            .mt(px(6.0))
            .text_size(sp(12.5))
            .line_height(sp(17.0))
            .text_color(theme.text_secondary)
            .whitespace_normal()
            .when(details_open, |body| {
                body.max_h(px(180.0))
                    .overflow_y_scroll()
                    .font_family(crate::md::render::MONO_FAMILY)
            })
            .when(!details_open, |body| body.line_clamp(2))
            .child(message);

        Some(
            div().px(px(20.0)).pb(px(8.0)).child(
                div()
                    .w_full()
                    .max_w(px(CONTENT_MAX_WIDTH))
                    .mx_auto()
                    .p(px(12.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(theme.danger.opacity(0.35))
                    .bg(theme.raised)
                    .shadow_md()
                    .child(header)
                    .child(body)
                    .child(actions),
            ),
        )
    }

    /// Put the banner away for this turn. The choice is kept on the turn, so
    /// it survives a restart.
    pub(super) fn dismiss_turn_error(
        &mut self,
        session_id: Uuid,
        turn_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let dismissed = self
            .state
            .session_mut(session_id)
            .and_then(|session| session.turns.iter_mut().find(|turn| turn.id == turn_id))
            .and_then(|turn| turn.error.as_mut())
            .map(|error| error.dismissed = true)
            .is_some();
        if dismissed {
            self.state.mark_session_dirty(session_id);
        }
        if self.error_details_open == Some(turn_id) {
            self.error_details_open = None;
        }
        cx.notify();
    }
}
