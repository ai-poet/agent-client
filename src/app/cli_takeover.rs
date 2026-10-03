//! Which CLIs the cloud account configures: the per-CLI switches on Settings
//! → Cloud Account, and the question asked at sign-in when a CLI already
//! runs on the user's own account.
//!
//! Fork addition. The choice and the detection live in
//! `sub2api::cli_takeover`; routing honours it through
//! `GatewayConfig::own_login_clis` (filled in by
//! [`super::providers_page::cloud_config`]), so a CLI kept on its own
//! sign-in is simply never written — and, if it was, is restored from the
//! takeover backup by the next reconcile.

use gpui::{KeyBinding, actions};
use sub2api::cli_takeover::{CliMode, OwnSetup, OwnSetupReason, TAKEOVER_CLIS};

use crate::ui::ActivationExt as _;

use super::cloud_account::section_title;
use super::*;

actions!(
    waku_cli_takeover,
    [AcceptCliTakeoverPrompt, DismissCliTakeoverPrompt]
);

const PROMPT_CONTEXT: &str = "CliTakeoverPrompt";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", AcceptCliTakeoverPrompt, Some(PROMPT_CONTEXT)),
        KeyBinding::new("escape", DismissCliTakeoverPrompt, Some(PROMPT_CONTEXT)),
    ]);
}

/// The sign-in question: one row per CLI found with a setup of its own.
pub(super) struct CliTakeoverPrompt {
    rows: Vec<PromptRow>,
    confirm_focus: FocusHandle,
    /// Where focus was before the prompt took it; restored on close.
    previous_focus: Option<FocusHandle>,
    /// The first frame after opening moves focus into the prompt.
    focus_pending: bool,
}

struct PromptRow {
    setup: OwnSetup,
    /// Hand this CLI to the account. On by default: the user just signed in
    /// to use it, and the prompt is where they say otherwise.
    use_cloud: bool,
}

/// The provider a takeover CLI id names.
fn takeover_provider(cli: &str) -> Option<ProviderKind> {
    ProviderKind::ALL
        .into_iter()
        .find(|provider| provider.id() == cli)
}

fn cli_name(cli: &str) -> String {
    takeover_provider(cli)
        .map(|provider| provider.display_name().to_owned())
        .unwrap_or_else(|| cli.to_owned())
}

/// What was found, as the prompt says it.
fn setup_detail(setup: &OwnSetup) -> String {
    match (&setup.reason, setup.account.as_deref()) {
        (OwnSetupReason::SignedIn, Some(account)) => {
            tr!("cloud.cli_takeover.found_signed_in_as", account = account)
        }
        (OwnSetupReason::SignedIn, None) => tr!("cloud.cli_takeover.found_signed_in"),
        (OwnSetupReason::ApiKey, _) => tr!("cloud.cli_takeover.found_api_key"),
        (OwnSetupReason::CustomProvider, _) => tr!("cloud.cli_takeover.found_custom_provider"),
    }
}

impl Waku {
    /// At sign-in, before the first reconcile: find the CLIs that already
    /// run on the user's own account and were never asked about, record
    /// them as kept (so nothing is overwritten even if the app quits with
    /// the question open), and ask.
    pub(super) fn prompt_cli_takeover_on_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(paths) = sub2api::global_config::Paths::resolve() else {
            return;
        };
        let prefs = &self.cloud_account.cli_takeover;
        let found: Vec<OwnSetup> = sub2api::cli_takeover::detect_own_setup(&paths)
            .into_iter()
            .filter(|setup| prefs.mode(setup.cli).is_none())
            .collect();
        if found.is_empty() {
            return;
        }
        for setup in &found {
            self.cloud_account
                .cli_takeover
                .set(setup.cli, CliMode::Own);
        }
        self.save_cli_takeover();
        self.cloud_account.takeover_prompt = Some(CliTakeoverPrompt {
            rows: found
                .into_iter()
                .map(|setup| PromptRow {
                    setup,
                    use_cloud: true,
                })
                .collect(),
            confirm_focus: cx.focus_handle(),
            previous_focus: None,
            focus_pending: true,
        });
        cx.notify();
    }

    /// Turn the account's takeover of one CLI on or off.
    pub(super) fn set_cli_takeover(&mut self, cli: &str, mode: CliMode, cx: &mut Context<Self>) {
        self.cloud_account.cli_takeover.set(cli, mode);
        self.save_cli_takeover();
        self.apply_cli_takeover_change();
        let name = cli_name(cli);
        self.show_toast(match mode {
            CliMode::Own => tr!("cloud.cli_takeover.now_own", cli = name),
            CliMode::Managed => tr!("cloud.cli_takeover.now_managed", cli = name),
        });
        cx.notify();
    }

    fn save_cli_takeover(&mut self) {
        if let Err(error) = sub2api::cli_takeover::save(&self.cloud_account.cli_takeover) {
            self.show_toast(format!("{error:#}"));
        }
    }

    /// Rewrite the CLIs' files for the new choice, and re-read their model
    /// lists: a CLI back on its own sign-in lists its own provider's models.
    fn apply_cli_takeover_change(&mut self) {
        self.apply_cloud_routing();
        self.refresh_provider_detection(None);
    }

    fn toggle_cli_takeover_prompt_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self
            .cloud_account
            .takeover_prompt
            .as_mut()
            .and_then(|prompt| prompt.rows.get_mut(index))
        {
            row.use_cloud = !row.use_cloud;
            cx.notify();
        }
    }

    fn accept_cli_takeover_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut prompt) = self.cloud_account.takeover_prompt.take() else {
            return;
        };
        self.restore_focus_after_takeover_prompt(prompt.previous_focus.take(), window, cx);
        let handed_over: Vec<&'static str> = prompt
            .rows
            .iter()
            .filter(|row| row.use_cloud)
            .map(|row| row.setup.cli)
            .collect();
        if handed_over.is_empty() {
            self.show_toast(tr!("cloud.cli_takeover.kept_all"));
        } else {
            for cli in &handed_over {
                self.cloud_account.cli_takeover.set(cli, CliMode::Managed);
            }
            self.save_cli_takeover();
            self.apply_cli_takeover_change();
            if handed_over.len() < prompt.rows.len() {
                self.show_toast(tr!("cloud.cli_takeover.kept_some"));
            }
        }
        cx.notify();
    }

    /// Closing without an answer keeps every CLI on its own sign-in — the
    /// choice already recorded when the prompt opened.
    fn dismiss_cli_takeover_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut prompt) = self.cloud_account.takeover_prompt.take() else {
            return;
        };
        self.restore_focus_after_takeover_prompt(prompt.previous_focus.take(), window, cx);
        self.show_toast(tr!("cloud.cli_takeover.kept_all"));
        cx.notify();
    }

    fn restore_focus_after_takeover_prompt(
        &self,
        previous: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match previous {
            Some(handle) => window.focus(&handle, cx),
            None => {
                let composer = self.composer_focus(cx);
                window.focus(&composer, cx);
            }
        }
    }

    pub(super) fn render_cli_takeover_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let theme = Theme::current(cx);
        let prompt = self.cloud_account.takeover_prompt.as_mut()?;
        if prompt.focus_pending {
            prompt.focus_pending = false;
            prompt.previous_focus = window.focused(cx);
            window.focus(&prompt.confirm_focus, cx);
        }
        let confirm_focus = prompt.confirm_focus.clone();
        let rows: Vec<(usize, String, String, bool)> = prompt
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                (
                    index,
                    cli_name(row.setup.cli),
                    setup_detail(&row.setup),
                    row.use_cloud,
                )
            })
            .collect();

        let mut list = div().mt(px(4.0)).flex().flex_col().gap(px(6.0));
        for (index, name, detail, use_cloud) in rows {
            list = list.child(
                div()
                    .w_full()
                    .px(px(12.0))
                    .py(px(10.0))
                    .rounded(px(10.0))
                    .bg(theme.inset)
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(sp(12.8))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_ghost)
                                    .truncate()
                                    .child(detail),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .child(if use_cloud {
                                tr!("cloud.cli_takeover.use_cloud")
                            } else {
                                tr!("cloud.cli_takeover.keep_own")
                            }),
                    )
                    .child(crate::ui::toggle_switch(
                        ("cli-takeover-prompt-row", index),
                        use_cloud,
                        false,
                        theme,
                        cx,
                        move |this, _, cx| this.toggle_cli_takeover_prompt_row(index, cx),
                    )),
            );
        }

        let keep_all = div()
            .id("cli-takeover-prompt-keep")
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(30.0))
            .px(px(14.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
            .child(tr!("cloud.cli_takeover.keep_all"))
            .on_activation(cx, |this, window, cx| this.dismiss_cli_takeover_prompt(window, cx));

        let confirm = div()
            .id("cli-takeover-prompt-confirm")
            .track_focus(&confirm_focus)
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .h(px(30.0))
            .px(px(14.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .font_weight(FontWeight::MEDIUM)
            .bg(theme.inverse)
            .text_color(theme.on_inverse)
            .hover(|style| style.opacity(0.9))
            .child(tr!("cloud.cli_takeover.confirm"))
            .on_activation(cx, |this, window, cx| this.accept_cli_takeover_prompt(window, cx));

        let card = div()
            .id("cli-takeover-prompt-card")
            .key_context(PROMPT_CONTEXT)
            .on_action(cx.listener(|this, _: &AcceptCliTakeoverPrompt, window, cx| {
                this.accept_cli_takeover_prompt(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DismissCliTakeoverPrompt, window, cx| {
                this.dismiss_cli_takeover_prompt(window, cx);
            }))
            .tab_group()
            .tab_stop(false)
            .w_full()
            .max_w(px(460.0))
            .rounded(px(16.0))
            .bg(theme.composer)
            .shadow_xl()
            .p(px(18.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(sp(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("cloud.cli_takeover.prompt_title")),
            )
            .child(
                div()
                    .text_size(sp(12.5))
                    .line_height(sp(18.0))
                    .text_color(theme.text_secondary)
                    .child(tr!("cloud.cli_takeover.prompt_detail")),
            )
            .child(list)
            .child(
                div()
                    .text_size(sp(11.5))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("cloud.cli_takeover.prompt_footnote")),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(keep_all)
                    .child(confirm),
            );

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("cli-takeover-prompt-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .child(card);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    /// Settings → Cloud Account: one switch per CLI the account would
    /// configure. Shown signed out too, so the choice can be made first.
    pub(super) fn render_cli_takeover_settings(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let prefs = &self.cloud_account.cli_takeover;
        let mut rows = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.cli_takeover.title"),
            &tr!("cloud.cli_takeover.detail"),
        ));
        for cli in TAKEOVER_CLIS {
            let managed = !prefs.keeps_own(cli);
            rows = rows.child(
                div()
                    .w_full()
                    .px(px(16.0))
                    .py(px(11.0))
                    .rounded(px(11.0))
                    .bg(theme.raised)
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(sp(12.8))
                                    .text_color(theme.text)
                                    .child(cli_name(cli)),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_ghost)
                                    .child(if managed {
                                        tr!("cloud.cli_takeover.managed_detail")
                                    } else {
                                        tr!("cloud.cli_takeover.own_detail")
                                    }),
                            ),
                    )
                    .child(crate::ui::toggle_switch(
                        SharedString::from(format!("cli-takeover-{cli}")),
                        managed,
                        false,
                        theme,
                        cx,
                        move |this, _, cx| {
                            this.set_cli_takeover(
                                cli,
                                if managed { CliMode::Own } else { CliMode::Managed },
                                cx,
                            );
                        },
                    )),
            );
        }
        if self.cloud_account.credentials.is_some() && !self.cloud_account.routing_enabled {
            rows = rows.child(
                div()
                    .text_size(sp(11.5))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("cloud.cli_takeover.routing_off_note")),
            );
        }
        rows
    }
}
