//! The sign-in window: what the app is waiting for while the user signs in
//! in the browser, and where the one-time code goes when the browser cannot
//! find its way back.
//!
//! Fork addition. The attempt itself — loopback wait, code redemption,
//! storing the session — lives in `cloud_account.rs`; the protocol in
//! `sub2api::auth`.

use super::*;

impl Waku {
    pub(super) fn render_cloud_sign_in_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let attempt = self
            .cloud_account
            .sign_in
            .as_ref()
            .filter(|attempt| attempt.dialog_open)?;
        let theme = Theme::current(cx);
        let exchanging = attempt.exchanging;
        let status = if exchanging {
            tr!("cloud.sign_in_dialog.finishing")
        } else if attempt.listening {
            tr!("cloud.sign_in_dialog.waiting")
        } else {
            tr!("cloud.sign_in_dialog.paste_only")
        };
        let link_copied = attempt.link_copied;
        let error = attempt.error.clone();
        let pasted = !self
            .cloud_sign_in_input
            .read(cx)
            .content()
            .trim()
            .is_empty();

        let browser_actions = div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .child(self.sign_in_secondary_button(
                "cloud-sign-in-reopen",
                tr!("cloud.sign_in_dialog.reopen"),
                theme,
                cx,
                |this, cx| {
                    if let Some(attempt) = this.cloud_account.sign_in.as_ref() {
                        cx.open_url(&attempt.login_url);
                    }
                },
            ))
            .child(self.sign_in_secondary_button(
                "cloud-sign-in-copy-link",
                if link_copied {
                    tr!("cloud.sign_in_dialog.link_copied")
                } else {
                    tr!("cloud.sign_in_dialog.copy_link")
                },
                theme,
                cx,
                |this, cx| {
                    if let Some(attempt) = this.cloud_account.sign_in.as_mut() {
                        cx.write_to_clipboard(ClipboardItem::new_string(attempt.login_url.clone()));
                        attempt.link_copied = true;
                        cx.notify();
                    }
                },
            ));

        let divider = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(div().flex_1().h(px(1.0)).bg(theme.border))
            .child(
                div()
                    .flex_none()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("cloud.sign_in_dialog.fallback_title")),
            )
            .child(div().flex_1().h(px(1.0)).bg(theme.border));

        let submit_enabled = pasted && !exchanging;
        let submit = div()
            .id("cloud-sign-in-submit")
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .flex_none()
            .h(px(28.0))
            .px(px(14.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .bg(theme.inverse)
            .text_color(theme.on_inverse)
            .text_size(sp(12.5))
            .font_weight(FontWeight::MEDIUM)
            .opacity(if submit_enabled { 1.0 } else { 0.45 })
            .child(if exchanging {
                tr!("cloud.sign_in_dialog.signing_in")
            } else {
                tr!("cloud.sign_in_dialog.submit")
            })
            .when(submit_enabled, |button| {
                button
                    .hover(|style| style.opacity(0.9))
                    .on_click(cx.listener(|this, _, _, cx| this.submit_pasted_sign_in(cx)))
            });

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .line_height(sp(19.0))
                    .text_color(theme.text_secondary)
                    .child(status),
            )
            .child(browser_actions)
            .child(divider)
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(18.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("cloud.sign_in_dialog.paste_hint")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div().flex_1().min_w_0().child(TextField::new(
                            "cloud-sign-in-code",
                            self.cloud_sign_in_input.clone(),
                        )),
                    )
                    .child(submit),
            );
        if let Some(error) = error {
            body = body.child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(18.0))
                    .text_color(theme.danger)
                    .child(error),
            );
        }
        body = body.child(
            div().flex().justify_end().child(self.sign_in_secondary_button(
                "cloud-sign-in-cancel",
                tr!("cloud.sign_in_dialog.cancel"),
                theme,
                cx,
                |this, cx| this.cancel_cloud_sign_in(cx),
            )),
        );

        let card = div()
            .id("cloud-sign-in-card")
            .w_full()
            .max_w(px(420.0))
            .rounded(px(18.0))
            .bg(theme.composer)
            .shadow_xl()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(48.0))
                    .px(px(16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(sp(14.0))
                            .text_color(theme.text)
                            .child(tr!(
                                "cloud.sign_in_dialog.title",
                                name = sub2api::brand::DISPLAY_NAME
                            )),
                    )
                    .child(
                        div()
                            .id("cloud-sign-in-close")
                            .tab_index(0)
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(7.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_default()
                            .hover(|style| style.bg(theme.overlay))
                            .child(icon("icons/x.svg", 14.0, theme.text_secondary))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_cloud_sign_in_dialog(cx);
                            })),
                    ),
            )
            .child(div().px(px(16.0)).pb(px(16.0)).child(body));

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        // Clicking outside only puts the window away; the attempt keeps
        // waiting for the browser, and the footer chip brings it back.
        let layer = div()
            .id("cloud-sign-in-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_cloud_sign_in_dialog(cx)),
            )
            .child(card);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    fn sign_in_secondary_button(
        &self,
        id: &'static str,
        label: String,
        theme: Theme,
        cx: &mut Context<Self>,
        activate: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(28.0))
            .px(px(12.0))
            .rounded_full()
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.0))
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| activate(this, cx)))
    }
}
