//! The first-run checklist: sign in, install a CLI, send a message.
//!
//! Fork addition. Before this the welcome screen carried two unrelated
//! nudges (a sign-in card, a "no CLI found" hint) that vanished the moment
//! a project was open and never remembered being dismissed. This is one
//! card that says what is left, ticks steps off as the app observes them,
//! and — once a project is open — shrinks to a strip above the composer
//! until the last step is done or the user closes it. What "done" means
//! for each step, and whether the card shows, is decided by
//! `sub2api::onboarding`; this file only renders and persists.

use crate::ui::ActivationExt as _;

use super::*;

use sub2api::onboarding::{OnboardingState, Step};

#[derive(Default)]
pub(super) struct OnboardingViewState {
    /// Loaded once at startup, off-thread. Nothing renders until it is
    /// here — better a card that appears a frame late than one that flashes
    /// for a user who dismissed it last week.
    pub persisted: Option<OnboardingState>,
    /// Completion has been handed to the background writer; the frame that
    /// noticed must not schedule it twice.
    completion_scheduled: std::cell::Cell<bool>,
}

impl Waku {
    /// Read the stored state on a background thread. Called at startup.
    pub(super) fn load_onboarding_state(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = cx
                .background_executor()
                .spawn(async { sub2api::onboarding::load() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.onboarding.persisted = Some(state);
                cx.notify();
            });
        })
        .detach();
    }

    /// The three steps, judged from what the app already holds in memory.
    fn onboarding_steps(&self) -> [(Step, bool); 3] {
        let signed_in = self.cloud_account.credentials.is_some();
        let cli_installed = self.probes.iter().any(|probe| probe.installed);
        let message_sent = self.state.sessions.iter().any(|session| {
            session
                .messages
                .iter()
                .any(|message| message.role == MessageRole::User)
        });
        sub2api::onboarding::steps(signed_in, cli_installed, message_sent)
    }

    /// The steps when the checklist should be on screen, else `None`.
    /// Also the seam where completion is noticed and recorded.
    fn visible_onboarding_steps(&self, cx: &mut Context<Self>) -> Option<[(Step, bool); 3]> {
        let persisted = self.onboarding.persisted.as_ref()?;
        let steps = self.onboarding_steps();
        if sub2api::onboarding::all_done(&steps)
            && persisted.completed_at.is_none()
            && !self.onboarding.completion_scheduled.get()
        {
            self.onboarding.completion_scheduled.set(true);
            self.record_onboarding_completion(cx);
        }
        sub2api::onboarding::visible(persisted, &steps).then_some(steps)
    }

    fn record_onboarding_completion(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let state = this
                .update(cx, |this, _| {
                    let state = this.onboarding.persisted.get_or_insert_with(Default::default);
                    sub2api::onboarding::mark_completed(state, unix_time());
                    state.clone()
                })
                .ok();
            if let Some(state) = state {
                cx.background_executor()
                    .spawn(async move {
                        let _ = sub2api::onboarding::save(&state);
                    })
                    .await;
            }
        })
        .detach();
    }

    /// "Later": hide the checklist and remember that.
    pub(super) fn dismiss_onboarding(&mut self, cx: &mut Context<Self>) {
        let state = self
            .onboarding
            .persisted
            .get_or_insert_with(Default::default);
        state.dismissed = true;
        let state = state.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = sub2api::onboarding::save(&state);
            })
            .detach();
        cx.notify();
    }

    /// The two agents this product is built around, in one click.
    fn install_default_clis(&mut self, cx: &mut Context<Self>) {
        {
            let mut selected = self.cli_setup.selected.borrow_mut();
            selected.clear();
            selected.insert("claude".to_owned());
            selected.insert("codex".to_owned());
        }
        self.run_selected_cli_installs(cx);
    }

    fn onboarding_step_label(step: Step) -> String {
        match step {
            Step::SignIn => tr!("onboarding.step_sign_in", name = sub2api::brand::DISPLAY_NAME),
            Step::InstallCli => tr!("onboarding.step_install_cli"),
            Step::FirstMessage => tr!("onboarding.step_first_message"),
        }
    }

    /// The buttons for a step that is still to do.
    fn onboarding_step_actions(
        &self,
        step: Step,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let installing = self.cli_setup.running.is_some();
        match step {
            Step::SignIn => {
                let pending = self.cloud_account.pending;
                vec![onboarding_button(
                    theme,
                    "onboarding-sign-in",
                    if pending {
                        tr!("cloud.onboarding_waiting")
                    } else {
                        tr!("cloud.sign_in")
                    },
                    true,
                    pending,
                    cx,
                    |this, _, cx| this.start_cloud_sign_in(cx),
                )]
            }
            Step::InstallCli => vec![
                onboarding_button(
                    theme,
                    "onboarding-install-defaults",
                    if installing {
                        self.cli_setup
                            .node_stage
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| tr!("cli_setup.installing"))
                    } else {
                        tr!("onboarding.install_defaults")
                    },
                    true,
                    installing,
                    cx,
                    |this, _, cx| this.install_default_clis(cx),
                ),
                onboarding_button(
                    theme,
                    "onboarding-open-providers",
                    tr!("onboarding.open_providers"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.open_settings_page(SettingsPage::Providers, cx),
                ),
            ],
            Step::FirstMessage => {
                if self.selected_project().is_some() {
                    vec![onboarding_button(
                        theme,
                        "onboarding-focus-composer",
                        tr!("onboarding.write_first_message"),
                        true,
                        false,
                        cx,
                        |this, window, cx| {
                            let focus = this.composer_focus(cx);
                            window.focus(&focus, cx);
                        },
                    )]
                } else {
                    vec![onboarding_button(
                        theme,
                        "onboarding-open-project",
                        tr!("onboarding.open_project_folder"),
                        true,
                        false,
                        cx,
                        |this, _, cx| this.add_project(cx),
                    )]
                }
            }
        }
    }

    /// The welcome-screen card: every step, with the next one's actions.
    pub(super) fn render_onboarding_checklist(&self, cx: &mut Context<Self>) -> Option<Div> {
        let steps = self.visible_onboarding_steps(cx)?;
        let theme = Theme::current(cx);
        let next = sub2api::onboarding::next_step(&steps);
        let done_count = steps.iter().filter(|(_, done)| *done).count();

        let mut card = div()
            .mt(px(28.0))
            .max_w(px(460.0))
            .w_full()
            .px(px(20.0))
            .py(px(16.0))
            .rounded(px(14.0))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(sp(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("onboarding.checklist_title")),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_ghost)
                            .child(tr!(
                                "onboarding.progress",
                                done = done_count,
                                total = steps.len()
                            )),
                    ),
            );

        for (index, (step, done)) in steps.iter().enumerate() {
            let is_next = next == Some(*step);
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_none()
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        .bg(theme.overlay)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if *done {
                            icon("icons/check.svg", 12.0, theme.success).into_any_element()
                        } else {
                            div()
                                .text_size(sp(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if is_next { theme.text } else { theme.text_ghost })
                                .child(format!("{}", index + 1))
                                .into_any_element()
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(sp(13.0))
                        .text_color(if *done {
                            theme.text_tertiary
                        } else if is_next {
                            theme.text
                        } else {
                            theme.text_secondary
                        })
                        .child(Self::onboarding_step_label(*step)),
                );
            if is_next {
                let actions = self.onboarding_step_actions(*step, theme, cx);
                row = row.child(div().flex().flex_none().gap(px(6.0)).children(actions));
            }
            card = card.child(row);
        }

        if let Some(error) = self.cloud_account.error.clone() {
            card = card.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .child(error),
            );
        }

        card = card.child(
            div().flex().justify_end().child(onboarding_button(
                theme,
                "onboarding-later",
                tr!("onboarding.later"),
                false,
                false,
                cx,
                |this, _, cx| this.dismiss_onboarding(cx),
            )),
        );
        Some(card)
    }

    /// The compact form above the composer once a project is open: progress,
    /// the next step, its action, and a close.
    pub(super) fn render_onboarding_strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        let steps = self.visible_onboarding_steps(cx)?;
        let next = sub2api::onboarding::next_step(&steps)?;
        let theme = Theme::current(cx);
        let done_count = steps.iter().filter(|(_, done)| *done).count();
        let actions = self.onboarding_step_actions(next, theme, cx);

        Some(
            div()
                .mx(px(16.0))
                .mb(px(6.0))
                .px(px(12.0))
                .py(px(7.0))
                .rounded(px(10.0))
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(icon("icons/sparkle.svg", 12.0, theme.accent))
                .child(
                    div()
                        .flex_none()
                        .text_size(sp(12.0))
                        .text_color(theme.text_ghost)
                        .child(tr!(
                            "onboarding.progress",
                            done = done_count,
                            total = steps.len()
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(Self::onboarding_step_label(next)),
                )
                .children(actions)
                .child(
                    div()
                        .id("onboarding-strip-close")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(theme.accent))
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .hover(|style| style.bg(theme.overlay))
                        .tooltip(Tooltip::text(tr_cow!("onboarding.later")))
                        .child(icon("icons/x.svg", 12.0, theme.text_tertiary))
                        .on_activation(cx, |this, _, cx| this.dismiss_onboarding(cx)),
                ),
        )
    }
}

/// A checklist button: pill-shaped like the welcome screen's, and keyboard
/// operable like everything else.
fn onboarding_button(
    theme: Theme,
    id: &'static str,
    label: String,
    primary: bool,
    disabled: bool,
    cx: &mut Context<Waku>,
    activate: impl Fn(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .h(px(28.0))
        .px(px(12.0))
        .rounded_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_default()
        .text_size(sp(12.0))
        .opacity(if disabled { 0.55 } else { 1.0 });
    let button = if primary {
        button
            .bg(theme.inverse)
            .text_color(theme.on_inverse)
            .font_weight(FontWeight::MEDIUM)
            .hover(|style| style.opacity(0.9))
    } else {
        button
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
    };
    let button = button.child(label);
    if disabled {
        button
    } else {
        button.on_activation(cx, activate)
    }
}
