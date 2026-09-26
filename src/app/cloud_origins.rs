//! Settings → Cloud Account: which of the service's domains the CLIs are
//! pointed at, with a latency test to choose one.
//!
//! Fork addition. The session stays on the domain it signed in on; only the
//! endpoint handed to the CLIs moves (`sub2api::gateway_origin`).

use super::cloud_account::section_title;
use super::providers_page::card_button;
use super::*;

/// A gateway-domain measurement. Which run it belongs to is tracked by
/// `origin_generation` on the state, so a superseded result is discarded.
#[derive(Default)]
pub(super) struct OriginTest {
    pub running: bool,
    pub results: Vec<sub2api::speedtest::CandidateResult>,
}

impl Waku {
    /// Build (once) the field for adding a service domain.
    fn ensure_origin_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self.cloud_account.origin_input.clone() {
            return input;
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .select_all_on_focus_click()
                .placeholder(tr!("cloud.origin_placeholder"))
        });
        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.add_gateway_origin(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.cloud_account.origin_input = Some(input.clone());
        input
    }

    /// Reveal the domain field and put the cursor in it.
    pub(super) fn open_gateway_origin_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.ensure_origin_input(window, cx);
        let focus = input.read(cx).focus();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Save the origin choice and re-point every routed CLI at it.
    fn persist_gateway_origin(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = sub2api::gateway_origin::save(&self.cloud_account.gateway_origin) {
            self.show_toast(format!("{error:#}"));
        }
        self.apply_cloud_routing();
        // The model list is served by the domain that was just swapped.
        self.refresh_provider_detection(None);
        self.refresh_native_catalog(false, cx);
        cx.notify();
    }

    pub(super) fn add_gateway_origin(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.cloud_account.origin_input.clone() else {
            return;
        };
        let raw = input.read(cx).content().trim().to_owned();
        if raw.is_empty() {
            return;
        }
        match self.cloud_account.gateway_origin.add(&raw) {
            Ok(true) => {
                input.update(cx, |input, cx| input.clear(cx));
                self.cloud_account.error = None;
                // Adding a domain does not route to it; measuring comes
                // first, then a deliberate choice.
                if let Err(error) =
                    sub2api::gateway_origin::save(&self.cloud_account.gateway_origin)
                {
                    self.show_toast(format!("{error:#}"));
                }
                cx.notify();
            }
            Ok(false) => {
                self.cloud_account.error = Some(tr!("cloud.origin_duplicate"));
                cx.notify();
            }
            Err(error) => {
                self.cloud_account.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    pub(super) fn remove_gateway_origin(&mut self, origin: String, cx: &mut Context<Self>) {
        let routed = self.cloud_account.gateway_origin.origin();
        self.cloud_account.gateway_origin.remove(&origin);
        self.cloud_account.origin_test = None;
        if routed.as_deref() == Some(origin.as_str()) {
            self.persist_gateway_origin(cx);
        } else if let Err(error) =
            sub2api::gateway_origin::save(&self.cloud_account.gateway_origin)
        {
            self.show_toast(format!("{error:#}"));
        }
        cx.notify();
    }

    pub(super) fn select_gateway_origin(&mut self, origin: String, cx: &mut Context<Self>) {
        if self.cloud_account.gateway_origin.origin().as_deref() == Some(origin.as_str()) {
            return;
        }
        if let Err(error) = self.cloud_account.gateway_origin.select(&origin) {
            self.show_toast(format!("{error:#}"));
            return;
        }
        self.persist_gateway_origin(cx);
    }

    pub(super) fn set_gateway_origin_auto_select(&mut self, on: bool, cx: &mut Context<Self>) {
        self.cloud_account.gateway_origin.auto_select = on;
        if let Err(error) = sub2api::gateway_origin::save(&self.cloud_account.gateway_origin) {
            self.show_toast(format!("{error:#}"));
        }
        cx.notify();
    }

    /// Check the chosen domain once per run.
    ///
    /// A domain that stopped serving this account would break every CLI with
    /// no visible cause, so the fallback to the build's primary happens
    /// whether or not automatic selection is on.
    pub(super) fn verify_gateway_origin(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.origin_checked {
            return;
        }
        let nothing_to_check = self.cloud_account.gateway_origin.effective_candidates().len() < 2;
        let no_key = self
            .cloud_account
            .credentials
            .as_ref()
            .is_none_or(|credentials| {
                sub2api::gateway_origin::probe_target(credentials).is_none()
            });
        if nothing_to_check || no_key {
            return;
        }
        self.cloud_account.origin_checked = true;
        self.run_gateway_origin_test(true, cx);
    }

    /// Measure every service domain, and act on the result.
    pub(super) fn run_gateway_origin_test(&mut self, automatic: bool, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let config = self.cloud_account.gateway_origin.clone();
        if sub2api::gateway_origin::probe_target(&credentials).is_none() {
            if !automatic {
                self.show_toast(tr!("cloud.origin_no_key"));
            }
            return;
        }
        self.cloud_account.origin_generation += 1;
        let generation = self.cloud_account.origin_generation;
        self.cloud_account.origin_test = Some(OriginTest {
            running: true,
            results: Vec::new(),
        });
        cx.notify();

        let routed = config.origin();
        let auto_select = config.auto_select;
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    sub2api::gateway_origin::test_origins(
                        &config,
                        &credentials,
                        sub2api::speedtest::DEFAULT_TIMEOUT_SECS,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cloud_account.origin_generation != generation {
                    return;
                }
                this.cloud_account.origin_test = Some(OriginTest {
                    running: false,
                    results: results.clone(),
                });
                cx.notify();

                let fastest = sub2api::speedtest::fastest_ok(&results)
                    .map(|index| results[index].url.clone());
                let routed_ok = routed.as_ref().is_some_and(|current| {
                    results
                        .iter()
                        .any(|candidate| candidate.url == *current && candidate.is_ok())
                });
                // A domain that no longer answers is left in place only when
                // there is nothing better to move to.
                if !routed_ok
                    && let Some(fallback) = fastest.clone()
                    && routed.as_deref() != Some(fallback.as_str())
                {
                    this.select_gateway_origin(fallback.clone(), cx);
                    this.show_toast(tr!("cloud.origin_fallback", url = fallback));
                    return;
                }
                if !auto_select {
                    if !automatic && fastest.is_none() {
                        this.show_toast(tr!("cloud.origin_no_ok"));
                    }
                    return;
                }
                match fastest {
                    Some(best) if routed.as_deref() != Some(best.as_str()) => {
                        this.select_gateway_origin(best.clone(), cx);
                        this.show_toast(tr!("cloud.origin_auto_selected", url = best));
                    }
                    Some(_) => {}
                    None if !automatic => this.show_toast(tr!("cloud.origin_no_ok")),
                    None => {}
                }
            });
        })
        .detach();
    }

    /// The service domains this build knows, their measured latency, and
    /// which one the CLIs are pointed at.
    pub(super) fn render_gateway_origins(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let candidates = self.cloud_account.gateway_origin.effective_candidates();
        let extra = self.cloud_account.origin_input.is_some();
        // One domain and nothing added: there is no choice to present.
        if candidates.len() < 2 && !extra {
            return div().child(card_button(
                theme,
                "cloud-origin-open".into(),
                tr!("cloud.origin_title"),
                false,
                false,
                cx,
                |this, window, cx| this.open_gateway_origin_input(window, cx),
            ));
        }

        let routed = self.cloud_account.gateway_origin.origin();
        let auto_select = self.cloud_account.gateway_origin.auto_select;
        let test = self.cloud_account.origin_test.as_ref();
        let testing = test.is_some_and(|test| test.running);
        let results = test.map(|test| test.results.clone()).unwrap_or_default();
        let outcome = |url: &str| results.iter().find(|candidate| candidate.url == url).cloned();

        let mut ordered = candidates.clone();
        if !results.is_empty() {
            ordered.sort_by_key(|url| match outcome(url) {
                Some(candidate) => match candidate.latency_ms() {
                    Some(ms) => (0u8, ms),
                    None => (1, 0),
                },
                None => (2, 0),
            });
        }

        let mut rows = div().flex().flex_col().gap(px(6.0)).child(section_title(
            theme,
            &tr!("cloud.origin_title"),
            &tr!("cloud.origin_detail"),
        ));

        for (index, url) in ordered.iter().enumerate() {
            let in_use = routed.as_deref() == Some(url.as_str());
            let candidate = outcome(url);
            let (status_text, status_color) = match candidate.as_ref() {
                Some(result) if result.invalid.is_some() => {
                    (tr!("cli_setup.speed_invalid_url"), theme.danger)
                }
                Some(result) => match result.result.as_ref() {
                    Some(probe) => match probe.verdict {
                        sub2api::custom_api::ProbeVerdict::Ok => (
                            tr!("cli_setup.candidate_latency", ms = probe.latency_ms),
                            theme.success,
                        ),
                        sub2api::custom_api::ProbeVerdict::Unauthorized => {
                            (tr!("cli_setup.candidate_unauthorized"), theme.warning)
                        }
                        sub2api::custom_api::ProbeVerdict::HttpError => (
                            tr!(
                                "cli_setup.candidate_http",
                                status = probe.status.unwrap_or_default()
                            ),
                            theme.warning,
                        ),
                        sub2api::custom_api::ProbeVerdict::Unreachable => {
                            (tr!("cli_setup.candidate_unreachable"), theme.danger)
                        }
                    },
                    None => (String::new(), theme.text_ghost),
                },
                None if testing => (tr!("cli_setup.custom_testing"), theme.text_ghost),
                None => (String::new(), theme.text_ghost),
            };

            let mut row = div()
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .rounded(px(11.0))
                .bg(theme.raised)
                .border_1()
                .border_color(if in_use { theme.accent } else { theme.raised })
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(crate::md::render::MONO_FAMILY)
                        .text_size(sp(12.0))
                        .text_color(theme.text)
                        .child(url.clone()),
                );
            if !status_text.is_empty() {
                row = row.child(
                    div()
                        .flex_none()
                        .text_size(sp(11.5))
                        .text_color(status_color)
                        .child(status_text),
                );
            }
            if in_use {
                row = row.child(
                    div()
                        .flex_none()
                        .text_size(sp(12.0))
                        .text_color(theme.accent)
                        .child(tr!("cli_setup.candidate_active")),
                );
            } else {
                let pick = url.clone();
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("cloud-origin-use-{index}")),
                    tr!("cli_setup.candidate_use"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.select_gateway_origin(pick.clone(), cx),
                ));
            }
            // The domains the build ships with are not the user's to remove.
            if self.cloud_account.gateway_origin.is_removable(url) {
                let drop = url.clone();
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("cloud-origin-remove-{index}")),
                    tr!("cli_setup.candidate_remove"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| this.remove_gateway_origin(drop.clone(), cx),
                ));
            }
            rows = rows.child(row);
        }

        if let Some(input) = self.cloud_account.origin_input.clone() {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(TextField::new("cloud-origin-input", input).flex_1())
                    .child(card_button(
                        theme,
                        "cloud-origin-add".into(),
                        tr!("cli_setup.candidate_add"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.add_gateway_origin(cx),
                    )),
            );
        } else {
            rows = rows.child(div().flex().child(card_button(
                theme,
                "cloud-origin-add-open".into(),
                tr!("cli_setup.candidate_add"),
                false,
                false,
                cx,
                |this, window, cx| this.open_gateway_origin_input(window, cx),
            )));
        }

        rows.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .child(card_button(
                    theme,
                    "cloud-origin-test".into(),
                    if testing {
                        tr!("cli_setup.speed_testing")
                    } else {
                        tr!("cli_setup.speed_test_all")
                    },
                    false,
                    testing,
                    cx,
                    |this, _, cx| this.run_gateway_origin_test(false, cx),
                ))
                .child(toggle_switch(
                    "cloud-origin-auto",
                    auto_select,
                    false,
                    theme,
                    cx,
                    move |this, _, cx| this.set_gateway_origin_auto_select(!auto_select, cx),
                ))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("cli_setup.speed_auto_select")),
                ),
        )
    }
}
