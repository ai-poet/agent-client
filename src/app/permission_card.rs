//! The card above the composer that asks the person to decide on a request.
//!
//! Fork addition. Requests queue rather than replace one another, and the
//! card counts them ("1/2"). It answers the keyboard: a digit picks that
//! answer, Enter the first that allows, and Escape does nothing — it used to
//! fall through and stop the whole turn. "Deny and explain" sends the refusal
//! and then the explanation as a steering message, so the agent learns why
//! whichever provider runs it. A plan to approve reads as rendered markdown,
//! capped until opened in full, with the same way to send it back with notes.

use gpui::{KeyBinding, actions};

use super::*;

actions!(waku_permission_card, [SwallowPermissionEscape]);

const CARD_CONTEXT: &str = "PermissionCard";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        SwallowPermissionEscape,
        Some(CARD_CONTEXT),
    )]);
}

/// Whether a request is a finished plan asking to be approved. Every driver
/// that raises one titles it with the same string.
pub(super) fn is_plan_approval(permission: &PendingPermission) -> bool {
    permission.title == tr!("plan.ready_title")
}

/// The answer a key picks: a digit its option, Enter the first that allows.
pub(super) fn keyed_option(permission: &PendingPermission, key: &str) -> Option<usize> {
    if key == "enter" {
        return permission.options.iter().position(|option| option.allow);
    }
    let digit = key
        .parse::<usize>()
        .ok()
        .filter(|digit| (1..=9).contains(digit))?;
    (digit <= permission.options.len()).then_some(digit - 1)
}

impl Waku {
    pub(super) fn render_permission_card(
        &self,
        permission: &PendingPermission,
        queued: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let plan = is_plan_approval(permission);
        let request_id = permission.request_id.clone();
        self.focus_new_permission(&request_id, window, cx);

        let mut buttons = div().flex().items_center().flex_wrap().gap(px(8.0));
        for (index, option) in permission.options.iter().enumerate() {
            let request_id = request_id.clone();
            let option_id = option.id.clone();
            let allow = option.allow;
            buttons = buttons.child(
                div()
                    .id(SharedString::from(format!(
                        "permission-{}-{}",
                        permission.request_id, option.id
                    )))
                    .h(px(28.0))
                    .pl(px(6.0))
                    .pr(px(12.0))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_default()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(allow, |element| {
                        element
                            .bg(theme.inverse)
                            .text_color(theme.on_inverse)
                            .hover(|element| element.opacity(0.9))
                    })
                    .when(!allow, |element| {
                        element
                            .border_1()
                            .border_color(theme.border_strong)
                            .text_color(theme.text_secondary)
                            .hover(|element| element.bg(theme.overlay).text_color(theme.text))
                    })
                    .active(|element| element.opacity(0.8))
                    .when(index < 9, |element| {
                        element.child(
                            div()
                                .size(px(16.0))
                                .rounded(px(4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(sp(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .bg(if allow {
                                    theme.on_inverse.opacity(0.16)
                                } else {
                                    theme.overlay_strong
                                })
                                .child(format!("{}", index + 1)),
                        )
                    })
                    .child(SharedString::from(option.label.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.respond_permission(request_id.clone(), option_id.clone(), cx);
                    })),
            );
        }

        let body = if plan {
            self.render_plan_body(permission, &theme, cx)
        } else {
            div()
                .id("permission-detail")
                .max_h(px(92.0))
                .overflow_y_scroll()
                .p(px(8.0))
                .rounded(px(7.0))
                .bg(theme.inset)
                .font_family(crate::md::render::MONO_FAMILY)
                .text_size(sp(12.5))
                .line_height(sp(16.0))
                .text_color(theme.text_secondary)
                .whitespace_normal()
                .child(SharedString::from(permission.detail.clone()))
                .into_any_element()
        };

        let feedback_request = request_id.clone();
        let feedback = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.0))
                    .px(px(8.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.inset)
                    .flex()
                    .items_center()
                    .text_size(sp(12.5))
                    .child(self.permission_feedback.clone()),
            )
            .child(
                div()
                    .id("permission-deny-with-feedback")
                    .h(px(28.0))
                    .px(px(10.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border_strong)
                    .flex()
                    .items_center()
                    .flex_none()
                    .cursor_default()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_secondary)
                    .hover(|element| element.bg(theme.overlay).text_color(theme.text))
                    .child(if plan {
                        tr!("plan.keep_planning_with_notes")
                    } else {
                        tr!("permission.deny_with_feedback")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.deny_permission_with_feedback(feedback_request.clone(), cx);
                    })),
            );

        let key_request = request_id.clone();
        div().px(px(20.0)).pb(px(8.0)).child(
            div()
                .id("permission-card")
                .key_context(CARD_CONTEXT)
                .track_focus(&self.permission_focus)
                .on_action(cx.listener(|_, _: &SwallowPermissionEscape, _, cx| {
                    cx.stop_propagation();
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    // Only the card itself: a digit typed into the note is
                    // part of the note.
                    if event.keystroke.modifiers.modified()
                        || !this.permission_focus.is_focused(window)
                    {
                        return;
                    }
                    let focus = this.composer_focus(cx);
                    if this.respond_permission_by_key(&key_request, &event.keystroke.key, cx) {
                        cx.stop_propagation();
                        if this
                            .selected_runtime()
                            .is_none_or(|runtime| runtime.pending_permissions.is_empty())
                        {
                            window.focus(&focus, cx);
                        }
                    }
                }))
                .w_full()
                .max_w(px(CONTENT_MAX_WIDTH))
                .mx_auto()
                .p(px(12.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.raised)
                .shadow_md()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon(
                            if plan {
                                "icons/list.svg"
                            } else {
                                "icons/alert.svg"
                            },
                            13.0,
                            if plan { theme.accent } else { theme.warning },
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(sp(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(SharedString::from(permission.title.clone())),
                        )
                        .when(queued > 1, |header| {
                            header.child(
                                div()
                                    .flex_none()
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_tertiary)
                                    .child(tr!("permission.counter", index = 1, total = queued)),
                            )
                        }),
                )
                .child(body)
                .child(buttons)
                .child(feedback),
        )
    }

    /// The plan as rendered markdown, capped until the person opens it in
    /// full.
    fn render_plan_body(
        &self,
        permission: &PendingPermission,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.plan_card_expanded.borrow().as_deref() == Some(&*permission.request_id);
        let palette = MarkdownPalette::from_theme(theme);
        let ctx = self.markdown_ctx(
            format!("plan-{}", permission.request_id),
            &palette,
            self.scaled_markdown_metrics(MarkdownMetrics::COMPACT),
            false,
        );
        let mut view = self.plan_markdown.borrow_mut();
        view.set_text(&permission.detail, false);
        let markdown = md::render::markdown(&view, &ctx);
        let request_id = permission.request_id.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .id("permission-plan")
                    .max_h(px(if expanded { 480.0 } else { 240.0 }))
                    .overflow_y_scroll()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .bg(theme.inset)
                    .children(markdown),
            )
            .child(
                div()
                    .id("permission-plan-expand")
                    .self_start()
                    .h(px(22.0))
                    .px(px(6.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .hover(|element| element.bg(theme.overlay).text_color(theme.text_secondary))
                    .child(if expanded {
                        tr!("plan.collapse")
                    } else {
                        tr!("plan.expand_full")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut open = this.plan_card_expanded.borrow_mut();
                        *open = (!expanded).then(|| request_id.clone());
                        drop(open);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Take the keyboard for a request the first time it shows — unless the
    /// person is in the middle of writing, where a stray digit would answer
    /// for them.
    fn focus_new_permission(&self, request_id: &str, window: &mut Window, cx: &mut App) {
        if self.permission_focused_request.borrow().as_deref() == Some(request_id) {
            return;
        }
        *self.permission_focused_request.borrow_mut() = Some(request_id.to_owned());
        if self.composer.read(cx).content(cx).trim().is_empty() {
            window.focus(&self.permission_focus, cx);
        }
    }

    /// Answer the request by a key. Returns whether the key answered.
    fn respond_permission_by_key(
        &mut self,
        request_id: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(option_id) = self
            .selected_runtime()
            .and_then(|runtime| {
                runtime
                    .pending_permissions
                    .iter()
                    .find(|permission| permission.request_id == request_id)
            })
            .and_then(|permission| {
                let index = keyed_option(permission, key)?;
                permission
                    .options
                    .get(index)
                    .map(|option| option.id.clone())
            })
        else {
            return false;
        };
        self.respond_permission(request_id.to_owned(), option_id, cx);
        true
    }

    /// Refuse the request, then tell the agent why: the note goes into the
    /// running turn as a steering message (or queues, where the provider
    /// cannot be steered).
    pub(super) fn deny_permission_with_feedback(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(option_id) = self
            .selected_runtime()
            .and_then(|runtime| {
                runtime
                    .pending_permissions
                    .iter()
                    .find(|permission| permission.request_id == request_id)
            })
            .and_then(|permission| permission.options.iter().find(|option| !option.allow))
            .map(|option| option.id.clone())
        else {
            return;
        };
        let note = self
            .permission_feedback
            .read(cx)
            .content()
            .trim()
            .to_owned();
        self.permission_feedback
            .update(cx, |input, cx| input.clear(cx));
        self.respond_permission(request_id, option_id, cx);
        if !note.is_empty() {
            self.steer_composer_submission(ComposerSubmission::plain(note), cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PermissionOption;

    fn permission() -> PendingPermission {
        PendingPermission {
            request_id: "r1".into(),
            title: "Run a command".into(),
            detail: "cargo test".into(),
            options: vec![
                PermissionOption {
                    id: "deny".into(),
                    label: "Deny".into(),
                    allow: false,
                },
                PermissionOption {
                    id: "allow".into(),
                    label: "Allow once".into(),
                    allow: true,
                },
            ],
        }
    }

    #[test]
    fn a_digit_picks_its_answer_and_enter_the_first_that_allows() {
        let permission = permission();
        assert_eq!(keyed_option(&permission, "1"), Some(0));
        assert_eq!(keyed_option(&permission, "2"), Some(1));
        assert_eq!(keyed_option(&permission, "3"), None);
        assert_eq!(keyed_option(&permission, "0"), None);
        assert_eq!(keyed_option(&permission, "enter"), Some(1));
        assert_eq!(keyed_option(&permission, "a"), None);
    }
}
