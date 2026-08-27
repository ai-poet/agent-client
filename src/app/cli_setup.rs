//! Settings → Providers: what to install when an agent CLI is missing.
//!
//! Fork addition. Upstream tells the user a CLI was "not detected" and stops
//! there; this section says what to run. The plan itself — which CLIs are
//! missing, whether Node is new enough, and the exact command per platform —
//! comes from the `sub2api::cli_install` module, which is unit-tested without
//! GPUI.
//!
//! The section renders nothing when the machine is already set up, so a
//! correctly configured install sees no extra chrome.

use std::time::{Duration, Instant};

use super::*;

/// View state for the setup section.
#[derive(Default)]
pub(super) struct CliSetupState {
    /// Provider id whose install is currently running; `"node"` for the
    /// runtime itself.
    pub running: Option<String>,
    /// Tail of the last failed install, kept on screen so the user can read
    /// npm's own reason rather than a generic failure.
    pub last_error: Option<String>,
    /// Stage label for the Node install, written by the background installer
    /// and read by a UI poll loop while it runs.
    pub node_stage: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// CLI ids ticked for installation. Interior mutability: the default tick
    /// is seeded during render, which only has `&self`.
    pub selected: std::cell::RefCell<std::collections::HashSet<String>>,
    /// The default tick happens once; after that the user's choices stand,
    /// even across detection refreshes.
    selection_seeded: std::cell::Cell<bool>,
    /// Cached detection result. Interior mutability because detection is
    /// filled in during render, which only has `&self`.
    cache: std::cell::RefCell<Option<CliSetupSnapshot>>,
    cached_at: std::cell::Cell<Option<Instant>>,
    /// Which CLI's custom-endpoint form is expanded, if any.
    pub custom_expanded: Option<&'static str>,
    /// Cached stored custom endpoints; render reads this instead of the
    /// file. Refilled lazily, replaced on save.
    custom_cache: std::cell::RefCell<Option<sub2api::custom_api::CustomApiConfig>>,
}

impl CliSetupState {
    /// Throw the cache away; the next render re-detects.
    pub fn invalidate(&self) {
        self.cached_at.set(None);
    }
}

/// What detection found, at one point in time.
#[derive(Clone)]
struct CliSetupSnapshot {
    node_version: Option<String>,
    npm_version: Option<String>,
    plan: Vec<sub2api::cli_install::SetupStep>,
}

/// How long a detection result stays good.
///
/// Detection spawns `node --version`, and render runs on every notify — typing
/// in any field on the page would otherwise start a process per keystroke.
const DETECTION_TTL: Duration = Duration::from_secs(5);

/// The five custom-endpoint CLIs, as the user knows them. OpenCode and Pi
/// are not in the install list, so this cannot lean on the descriptors.
fn custom_provider_display(provider_id: &str) -> &'static str {
    match provider_id {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "grok" => "Grok Build",
        "opencode" => "OpenCode",
        "pi" => "Pi",
        _ => "?",
    }
}

fn custom_provider_icon(provider_id: &str) -> &'static str {
    match provider_id {
        "claude" => "icons/provider-claude.svg",
        "codex" => "icons/provider-openai.svg",
        "grok" => "icons/provider-grok.svg",
        "opencode" => "icons/provider-opencode.svg",
        "pi" => "icons/provider-pi.svg",
        _ => "icons/bot.svg",
    }
}

/// `sk-abcdef...xyz` — the Electron client's masking, enough to recognise a
/// key without exposing it.
fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 12 {
        return "***".to_owned();
    }
    format!("{}...{}", &trimmed[..8], &trimmed[trimmed.len() - 4..])
}

/// A small card action button; `primary` fills it for the main call to
/// action (Save), the rest stay outlined.
fn custom_card_button(
    theme: Theme,
    id: SharedString,
    label: String,
    primary: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .tab_index(0)
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_default()
        .text_size(sp(11.5));
    let button = if primary {
        button
            .bg(theme.inverse)
            .text_color(theme.on_inverse)
            .font_weight(FontWeight::MEDIUM)
    } else {
        button
            .border_1()
            .border_color(theme.border_strong)
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
    };
    button.child(label).on_click(on_click)
}

/// Human label for an installer stage.
fn stage_label(reported: sub2api::node_install::NodeStage) -> String {
    match reported {
        sub2api::node_install::NodeStage::ResolvingDownload => tr!("cli_setup.stage_resolving"),
        sub2api::node_install::NodeStage::Downloading => tr!("cli_setup.stage_downloading"),
        sub2api::node_install::NodeStage::Installing { method } => {
            tr!("cli_setup.stage_installing", method = method)
        }
        sub2api::node_install::NodeStage::Verifying => tr!("cli_setup.stage_verifying"),
    }
}


impl Waku {
    /// Install Node unattended, reporting each stage as it starts.
    pub(super) fn run_node_install(&mut self, cx: &mut Context<Self>) {
        if self.cli_setup.running.is_some() {
            return;
        }
        self.cli_setup.running = Some("node".to_owned());
        self.cli_setup.last_error = None;
        *self.cli_setup.node_stage.lock().unwrap() = Some(tr!("cli_setup.stage_resolving"));
        cx.notify();

        // The installer reports stages from the background thread; this poll
        // loop moves them onto the screen. 300ms is imperceptible next to a
        // download measured in tens of seconds.
        let poll_id = "node";
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let still_running = this.update(cx, |this, cx| {
                    let running = this.cli_setup.running.as_deref() == Some(poll_id);
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(still_running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let stage = self.cli_setup.node_stage.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    sub2api::node_install::install_node(|reported| {
                        *stage.lock().unwrap() = Some(stage_label(reported));
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                *this.cli_setup.node_stage.lock().unwrap() = None;
                this.cli_setup.invalidate();
                if outcome.success {
                    this.show_toast(tr!("cli_setup.installed"));
                } else {
                    this.cli_setup.last_error = Some(outcome.output);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Toggle one CLI's tick.
    pub(super) fn toggle_cli_selection(&mut self, id: String, cx: &mut Context<Self>) {
        let mut selected = self.cli_setup.selected.borrow_mut();
        if !selected.remove(&id) {
            selected.insert(id);
        }
        drop(selected);
        cx.notify();
    }

    /// Install every ticked CLI, resolving prerequisites first.
    ///
    /// Component-installer semantics: the user picks agents; Node is a
    /// dependency of every npm install, so a missing Node is installed as part
    /// of the batch rather than being a separate chore. Items run
    /// sequentially — npm's global prefix is not safe for concurrent writes —
    /// with the active row marked as it goes.
    pub(super) fn run_selected_cli_installs(&mut self, cx: &mut Context<Self>) {
        if self.cli_setup.running.is_some() {
            return;
        }
        // Descriptor order, not hash order, so the run is deterministic.
        let queue: Vec<(String, Vec<String>)> = {
            let selected = self.cli_setup.selected.borrow();
            sub2api::cli_install::DESCRIPTORS
                .iter()
                .filter(|descriptor| selected.contains(descriptor.id))
                .map(|descriptor| {
                    (
                        descriptor.id.to_owned(),
                        sub2api::cli_install::install_candidates(descriptor.package),
                    )
                })
                .collect()
        };
        if queue.is_empty() {
            return;
        }
        let needs_node = !sub2api::node_install::detect_node()
            .as_deref()
            .is_some_and(sub2api::cli_install::node_is_supported);

        self.cli_setup.last_error = None;
        self.cli_setup.running = Some(if needs_node {
            "node".to_owned()
        } else {
            queue[0].0.clone()
        });
        cx.notify();

        // One poll loop for the whole batch keeps the stage text and the
        // per-row "Installing…" marker moving.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(300))
                    .await;
                let still_running = this.update(cx, |this, cx| {
                    let running = this.cli_setup.running.is_some();
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(still_running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let stage = self.cli_setup.node_stage.clone();
        cx.spawn(async move |this, cx| {
            let mut failures: Vec<String> = Vec::new();
            let mut installed = 0usize;

            if needs_node {
                let stage = stage.clone();
                let outcome = cx
                    .background_executor()
                    .spawn(async move {
                        sub2api::node_install::install_node(|reported| {
                            *stage.lock().unwrap() = Some(stage_label(reported));
                        })
                    })
                    .await;
                if !outcome.success {
                    // Without Node every npm item would fail identically, so
                    // stop here with the one error that matters.
                    let _ = this.update(cx, |this, cx| {
                        this.cli_setup.running = None;
                        *this.cli_setup.node_stage.lock().unwrap() = None;
                        this.cli_setup.invalidate();
                        this.cli_setup.last_error = Some(outcome.output);
                        cx.notify();
                    });
                    return;
                }
                let _ = this.update(cx, |this, _| {
                    *this.cli_setup.node_stage.lock().unwrap() = None;
                });
            }

            for (id, commands) in queue {
                let _ = this.update(cx, |this, cx| {
                    this.cli_setup.running = Some(id.clone());
                    cx.notify();
                });
                let outcome = cx
                    .background_executor()
                    .spawn(async move { sub2api::cli_install::run_candidates(&commands) })
                    .await;
                if outcome.success {
                    installed += 1;
                    let _ = this.update(cx, |this, _| {
                        this.cli_setup.selected.borrow_mut().remove(&id);
                    });
                } else {
                    failures.push(format!("{id}:\n{}", outcome.output));
                }
            }

            let _ = this.update(cx, |this, cx| {
                this.cli_setup.running = None;
                this.cli_setup.invalidate();
                // A fresh install must show up everywhere without a manual
                // refresh: the Providers rows above and the model picker.
                if installed > 0 {
                    this.refresh_provider_detection(None);
                }
                if failures.is_empty() {
                    this.show_toast(tr!("cli_setup.installed"));
                } else {
                    if installed > 0 {
                        this.show_toast(tr!("cli_setup.installed"));
                    }
                    this.cli_setup.last_error = Some(failures.join("\n\n"));
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Environment status (always visible) plus what is missing.
    /// A compact pointer shown on the welcome screen when provider detection
    /// has settled and found no usable agent CLI at all. The composer — and
    /// with it the "No providers" empty state — is only rendered once a
    /// project is open, so without this row a brand-new machine gets no hint
    /// that anything needs installing.
    pub(super) fn render_missing_cli_hint(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.model_picker_has_no_providers() {
            return None;
        }
        let theme = Theme::current(cx);
        Some(
            div()
                .mt(px(14.0))
                .px(px(12.0))
                .py(px(7.0))
                .rounded(px(10.0))
                .bg(theme.raised)
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(icon("icons/alert.svg", 12.0, theme.warning))
                .child(
                    div()
                        .text_size(sp(12.0))
                        .text_color(theme.text_secondary)
                        .child(tr!("cli_setup.welcome_missing")),
                )
                .child(
                    div()
                        .id("welcome-open-providers")
                        .tab_index(0)
                        .px(px(8.0))
                        .h(px(24.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(theme.border_strong)
                        .flex()
                        .items_center()
                        .cursor_default()
                        .text_size(sp(12.0))
                        .text_color(theme.text)
                        .hover(|style| style.bg(theme.overlay))
                        .child(tr!("models.open_provider_settings"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_settings_action(&OpenSettings, window, cx);
                            this.open_settings_page(SettingsPage::Providers, cx);
                        })),
                ),
        )
    }

    pub(super) fn render_cli_setup_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let snapshot = self.cli_setup_snapshot();
        let node_version = snapshot.node_version;
        let npm_version = snapshot.npm_version;
        let plan = snapshot.plan;

        let node_satisfied = node_version
            .as_deref()
            .is_some_and(sub2api::cli_install::node_is_supported);

        let mut section = div()
            .mt(px(18.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(sp(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("cli_setup.env_title")),
            );

        // The environment row never disappears: a healthy machine shows its
        // versions in green rather than showing nothing, matching the old
        // client's runtime check.
        if node_satisfied {
            let node_label = node_version.clone().unwrap_or_default();
            let npm_label = npm_version
                .clone()
                .map(|npm| format!("  \u{00b7}  npm {npm}"))
                .unwrap_or_default();
            section = section.child(
                div()
                    .w_full()
                    .px(px(20.0))
                    .py(px(12.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child("Node.js"),
                    )
                    .child(
                        div()
                            .text_size(sp(12.5))
                            .text_color(theme.success)
                            .child(format!("\u{2713} {}{npm_label}", node_label.trim())),
                    ),
            );
        }

        // The default tick: the two agents this product is built around.
        // Seeded once; after that the user's choices stand.
        if !self.cli_setup.selection_seeded.get() {
            let mut selected = self.cli_setup.selected.borrow_mut();
            for step in &plan {
                if let sub2api::cli_install::SetupStep::InstallCli { id, .. } = step
                    && matches!(*id, "claude" | "codex")
                {
                    selected.insert((*id).to_owned());
                }
            }
            self.cli_setup.selection_seeded.set(true);
        }

        let busy = self.cli_setup.running.is_some();
        let mut selectable = 0usize;
        let mut ticked = 0usize;

        for step in plan {
            match step {
                sub2api::cli_install::SetupStep::InstallNode { required_major } => {
                    let running = self.cli_setup.running.as_deref() == Some("node");
                    // Linux stays report-only: distro package managers own
                    // Node there, same as the old client.
                    let action = sub2api::node_install::install_supported();
                    // While the installer runs, the row narrates its progress.
                    let detail = if running {
                        self.cli_setup
                            .node_stage
                            .lock()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| tr!("cli_setup.installing"))
                    } else {
                        match node_version.as_deref() {
                            Some(version) => {
                                tr!("cli_setup.node_found", version = version.trim())
                            }
                            // Without an unattended installer the row must
                            // say what to do, not just that Node is absent.
                            None if !action => tr!("cli_setup.node_manual"),
                            None => tr!("cli_setup.node_missing"),
                        }
                    };
                    section = section.child(node_row(
                        theme,
                        tr!("cli_setup.node_requirement", major = required_major),
                        detail,
                        action,
                        running,
                        busy,
                        cx,
                    ));
                }
                sub2api::cli_install::SetupStep::InstallCli {
                    id,
                    display_name,
                    commands,
                } => {
                    // Head the missing-agent list once, before its first row.
                    if selectable == 0 {
                        section = section.child(
                            div()
                                .mt(px(6.0))
                                .text_size(sp(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.text)
                                .child(tr!("cli_setup.title")),
                        );
                    }
                    selectable += 1;
                    let checked = self.cli_setup.selected.borrow().contains(id);
                    if checked {
                        ticked += 1;
                    }
                    let running = self.cli_setup.running.as_deref() == Some(id);
                    section = section.child(cli_row(
                        theme,
                        id,
                        display_name,
                        commands,
                        checked,
                        running,
                        busy,
                        cx,
                    ));
                }
            }
        }

        // One action for all ticked agents, like an installer's component
        // page — five separate install buttons were five decisions too many.
        if selectable > 0 {
            let disabled = busy || ticked == 0;
            section = section.child(
                div().flex().justify_end().child(
                    div()
                        .id("install-selected-clis")
                        .tab_index(0)
                        .h(px(29.0))
                        .px(px(12.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(theme.border_strong)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .opacity(if disabled { 0.55 } else { 1.0 })
                        .child(if busy {
                            tr!("cli_setup.installing")
                        } else {
                            tr!("cli_setup.install_selected", count = ticked)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if disabled {
                                return;
                            }
                            this.run_selected_cli_installs(cx);
                        })),
                ),
            );
        }

        if let Some(error) = self.cli_setup.last_error.clone() {
            section = section.child(
                div()
                    .w_full()
                    .px(px(20.0))
                    .py(px(14.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_secondary)
                            .child(error),
                    ),
            );
        }

        section = section.child(self.render_custom_api_section(theme, cx));

        section.into_any_element()
    }

    /// Per-CLI custom endpoints — the cc-switch-style card list.
    ///
    /// Saving writes the endpoint straight into that CLI's own global
    /// configuration (with the original backed up); clearing restores it.
    /// The card shows what is in effect at a glance: endpoint, masked key,
    /// and whether cloud routing currently outranks the entry.
    fn render_custom_api_section(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let stored = self.custom_api_snapshot();
        // Which CLIs the signed-in cloud account routes right now; those
        // cards flag that the custom entry is overridden.
        let cloud = self
            .cloud_account
            .credentials
            .as_ref()
            .filter(|_| self.cloud_account.routing_enabled)
            .map(|credentials| sub2api::gateway_config_from(credentials, true));
        let cloud_desired = sub2api::global_config::desired_routes(
            cloud.as_ref(),
            &sub2api::custom_api::CustomApiConfig::default(),
        );

        let mut section = div()
            .mt(px(10.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(sp(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("cli_setup.custom_title")),
                    )
                    .child(
                        div()
                            .mt(px(3.0))
                            .text_size(sp(12.0))
                            .text_color(theme.text_ghost)
                            .child(tr!("cli_setup.custom_detail")),
                    ),
            );

        for (provider_id, url_input, key_input, models_input) in &self.custom_api_inputs {
            let provider_id: &'static str = provider_id;
            let cloud_covers = match provider_id {
                "claude" => cloud_desired.claude.is_some(),
                "codex" => cloud_desired.codex.is_some(),
                "grok" => cloud_desired.grok.is_some(),
                _ => false,
            };
            let entry = stored.get(provider_id).cloned();
            let configured = entry.as_ref().is_some_and(|entry| entry.is_usable());
            let expanded = self.cli_setup.custom_expanded == Some(provider_id);

            // The status machine, cc-switch style: one label slot, three
            // states, color always paired with text.
            let (status_label, status_color) = if cloud_covers && configured {
                (tr!("cli_setup.custom_overridden"), theme.warning)
            } else if cloud_covers {
                (tr!("cli_setup.custom_cloud_active"), theme.accent)
            } else if configured {
                (tr!("cli_setup.custom_active"), theme.success)
            } else {
                (tr!("cli_setup.custom_inactive"), theme.text_ghost)
            };

            let mut card = div()
                .w_full()
                .px(px(16.0))
                .py(px(12.0))
                .rounded(px(13.0))
                .bg(theme.raised)
                .flex()
                .flex_col()
                .gap(px(8.0));

            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .child(icon(custom_provider_icon(provider_id), 18.0, theme.text))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(custom_provider_display(provider_id)),
                    )
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(status_color)
                            .child(status_label),
                    ),
            );

            // What is configured, at a glance: endpoint and masked key.
            if let Some(entry) = entry.as_ref().filter(|entry| !expanded && entry.is_usable()) {
                card = card.child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .truncate()
                        .child(format!(
                            "{}  \u{00b7}  {}",
                            entry.base_url,
                            mask_api_key(&entry.api_key)
                        )),
                );
            }

            if expanded {
                // The endpoint hint follows the CLI's protocol family —
                // cc-switch's amber helper line.
                let hint = if provider_id == "claude" {
                    tr!("cli_setup.custom_hint_anthropic")
                } else {
                    tr!("cli_setup.custom_hint_openai")
                };
                let mut form = div()
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.warning)
                            .child(hint),
                    )
                    .child(
                        TextField::new(
                            SharedString::from(format!("custom-api-url-{provider_id}")),
                            url_input.clone(),
                        )
                        .w_full(),
                    )
                    .child(
                        TextField::new(
                            SharedString::from(format!("custom-api-key-{provider_id}")),
                            key_input.clone(),
                        )
                        .w_full(),
                    );
                if let Some(models_input) = models_input {
                    form = form.child(
                        TextField::new(
                            SharedString::from(format!("custom-api-models-{provider_id}")),
                            models_input.clone(),
                        )
                        .w_full(),
                    );
                }
                form = form.child(
                    div().flex().justify_end().child(custom_card_button(
                        theme,
                        SharedString::from(format!("custom-api-save-{provider_id}")),
                        tr!("cli_setup.custom_save"),
                        true,
                        cx.listener(move |this, _, _, cx| {
                            this.save_custom_api_endpoint(provider_id, cx);
                        }),
                    )),
                );
                card = card.child(form);
            }

            // Action row. Testing is confirmation-free — it only probes
            // reachability, so there is nothing to warn about.
            let mut actions = div().flex().items_center().gap(px(6.0)).child(
                custom_card_button(
                    theme,
                    SharedString::from(format!("custom-api-edit-{provider_id}")),
                    if expanded {
                        tr!("cli_setup.custom_collapse")
                    } else {
                        tr!("cli_setup.custom_edit")
                    },
                    false,
                    cx.listener(move |this, _, _, cx| {
                        this.cli_setup.custom_expanded =
                            (this.cli_setup.custom_expanded != Some(provider_id))
                                .then_some(provider_id);
                        cx.notify();
                    }),
                ),
            );
            actions = actions.child(custom_card_button(
                theme,
                SharedString::from(format!("custom-api-test-{provider_id}")),
                tr!("cli_setup.custom_test"),
                false,
                cx.listener(move |this, _, _, cx| {
                    this.test_custom_endpoint(provider_id, cx);
                }),
            ));
            actions = actions.child(custom_card_button(
                theme,
                SharedString::from(format!("custom-api-open-{provider_id}")),
                tr!("cli_setup.custom_open_file"),
                false,
                cx.listener(move |this, _, _, cx| {
                    this.open_custom_config_file(provider_id, cx);
                }),
            ));
            if entry.is_some() {
                actions = actions.child(custom_card_button(
                    theme,
                    SharedString::from(format!("custom-api-clear-{provider_id}")),
                    tr!("cli_setup.custom_clear"),
                    false,
                    cx.listener(move |this, _, _, cx| {
                        this.clear_custom_api_endpoint(provider_id, cx);
                    }),
                ));
            }
            card = card.child(actions);
            section = section.child(card);
        }
        section
    }

    /// The stored custom endpoints, cached — reading a file on every render
    /// frame would not do. Invalidated on save.
    fn custom_api_snapshot(&self) -> sub2api::custom_api::CustomApiConfig {
        if let Some(config) = self.cli_setup.custom_cache.borrow().clone() {
            return config;
        }
        let config = sub2api::custom_api::load();
        *self.cli_setup.custom_cache.borrow_mut() = Some(config.clone());
        config
    }

    /// Persist one CLI's custom endpoint from its fields and rewrite that
    /// CLI's global configuration. Both filled saves; both empty clears; a
    /// half-filled pair is rejected — a URL without a key would silently
    /// route with no credentials.
    fn save_custom_api_endpoint(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some((_, url_input, key_input, models_input)) = self
            .custom_api_inputs
            .iter()
            .find(|(id, ..)| *id == provider_id)
        else {
            return;
        };
        let base_url = url_input.read(cx).content().trim().to_owned();
        let api_key = key_input.read(cx).content().trim().to_owned();
        if base_url.is_empty() != api_key.is_empty() {
            self.show_toast(tr!("cli_setup.custom_need_both"));
            return;
        }
        let models: Vec<String> = models_input
            .as_ref()
            .map(|input| {
                input
                    .read(cx)
                    .content()
                    .split([',', '\u{3001}', ' '])
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let mut config = sub2api::custom_api::load();
        let endpoint = (!base_url.is_empty()).then(|| sub2api::custom_api::CustomEndpoint {
            base_url,
            api_key,
            models,
        });
        let clearing = endpoint.is_none();
        config.set(provider_id, endpoint);
        if let Err(error) = sub2api::custom_api::save(&config) {
            self.show_toast(format!("{error:#}"));
            return;
        }
        *self.cli_setup.custom_cache.borrow_mut() = Some(config);
        // Rewrite the CLI's global configuration right away — this is what
        // the user opens to verify the save took.
        self.apply_cloud_routing();
        if !clearing {
            self.cli_setup.custom_expanded = None;
        }
        self.show_toast(if clearing {
            tr!("cli_setup.custom_cleared")
        } else {
            tr!("cli_setup.custom_saved")
        });
        cx.notify();
    }

    /// Clear one CLI's endpoint: empty the fields and save, which restores
    /// the CLI's original configuration.
    fn clear_custom_api_endpoint(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        if let Some((_, url_input, key_input, models_input)) = self
            .custom_api_inputs
            .iter()
            .find(|(id, ..)| *id == provider_id)
        {
            url_input.update(cx, |input, cx| input.clear(cx));
            key_input.update(cx, |input, cx| input.clear(cx));
            if let Some(models_input) = models_input {
                models_input.update(cx, |input, cx| input.clear(cx));
            }
        }
        self.save_custom_api_endpoint(provider_id, cx);
    }

    /// Probe the endpoint currently typed into the card: any HTTP answer
    /// proves reachability (auth is a different question). Three-tier toast,
    /// no confirmation — the probe sends no billable request.
    fn test_custom_endpoint(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some((_, url_input, ..)) = self
            .custom_api_inputs
            .iter()
            .find(|(id, ..)| *id == provider_id)
        else {
            return;
        };
        let mut url = url_input.read(cx).content().trim().to_owned();
        if url.is_empty() {
            self.show_toast(tr!("cli_setup.custom_need_both"));
            return;
        }
        if !url.contains("://") {
            url = format!("https://{url}");
        }
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    sub2api::http::Request::new()
                        .timeout_seconds(8)
                        .send(&url)
                        .map(|response| response.status)
                })
                .await;
            let ms = started.elapsed().as_millis();
            let _ = this.update(cx, |this, _| match outcome {
                Ok(_) if ms < 500 => {
                    this.show_toast(tr!("cli_setup.custom_connect_ok", ms = ms))
                }
                Ok(_) => this.show_toast(tr!("cli_setup.custom_connect_slow", ms = ms)),
                Err(error) => this.show_toast(tr!(
                    "cli_setup.custom_connect_failed",
                    error = format!("{error:#}")
                )),
            });
        })
        .detach();
    }

    /// Open the CLI's live configuration file — the artifact routing writes,
    /// and the thing users check to trust it.
    fn open_custom_config_file(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some(path) = sub2api::global_config::config_file_for(provider_id) else {
            return;
        };
        if !path.exists() {
            self.show_toast(tr!("cli_setup.custom_file_missing"));
            return;
        }
        cx.open_url(&path.display().to_string());
    }

    /// Current detection, recomputed at most once per [`DETECTION_TTL`].
    fn cli_setup_snapshot(&self) -> CliSetupSnapshot {
        let fresh = self
            .cli_setup
            .cached_at
            .get()
            .is_some_and(|at| at.elapsed() < DETECTION_TTL);
        if fresh && let Some(snapshot) = self.cli_setup.cache.borrow().clone() {
            return snapshot;
        }
        // detect_node probes the managed runtime and the MSI location too, so
        // a Node installed a moment ago turns the row green without a restart.
        let node_version = sub2api::node_install::detect_node();
        let npm_version = sub2api::node_install::detect_npm();
        let detections = sub2api::cli_install::detect_all();
        let plan = sub2api::cli_install::setup_plan(node_version.as_deref(), &detections);
        let snapshot = CliSetupSnapshot {
            node_version,
            npm_version,
            plan,
        };
        *self.cli_setup.cache.borrow_mut() = Some(snapshot.clone());
        self.cli_setup.cached_at.set(Some(Instant::now()));
        snapshot
    }
}

/// A prerequisite row (Node, Git): title, status, and its own Install button.
fn node_row(
    theme: Theme,
    title: String,
    detail: String,
    installable: bool,
    running: bool,
    busy: bool,
    cx: &mut Context<Waku>,
) -> Div {
    let row = div()
        .w_full()
        .px(px(20.0))
        .py(px(14.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_secondary)
                        .truncate()
                        .child(detail),
                ),
        );

    if !installable {
        // No unattended route on this platform; the row states the
        // requirement and leaves the choice of package manager to the user.
        return row;
    }
    row.child(
        div()
            .id("run-toolchain-install")
            .tab_index(0)
            .h(px(28.0))
            .px(px(11.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .opacity(if busy { 0.55 } else { 1.0 })
            .child(if running {
                tr!("cli_setup.installing")
            } else {
                tr!("cli_setup.install")
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if busy {
                    return;
                }
                this.run_node_install(cx);
            })),
    )
}

/// A selectable agent row: checkbox, name, command preview, and a copy button.
#[allow(clippy::too_many_arguments)]
fn cli_row(
    theme: Theme,
    id: &'static str,
    display_name: &'static str,
    commands: Vec<String>,
    checked: bool,
    running: bool,
    busy: bool,
    cx: &mut Context<Waku>,
) -> impl IntoElement {
    let command_preview = commands.first().cloned().unwrap_or_default();
    let copy_command = command_preview.clone();

    // The whole row toggles; a 15px box alone is a needlessly hard target.
    let checkbox = div()
        .flex_none()
        .size(px(15.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(if checked {
            theme.accent
        } else {
            theme.border_strong
        })
        .when(checked, |element| element.bg(theme.accent))
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |element| {
            element.child(
                div()
                    .text_size(sp(10.0))
                    .text_color(theme.raised)
                    .child("\u{2713}"),
            )
        });

    div()
        .id(SharedString::from(format!("cli-select-{id}")))
        .tab_index(0)
        .w_full()
        .px(px(20.0))
        .py(px(12.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .cursor_default()
        .opacity(if busy && !running { 0.7 } else { 1.0 })
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(checkbox)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(display_name),
                )
                .child(
                    div()
                        .mt(px(3.0))
                        .text_size(sp(12.0))
                        .text_color(theme.text_ghost)
                        .truncate()
                        .child(if running {
                            tr!("cli_setup.installing")
                        } else {
                            // Status first, exact command after — the command
                            // stays one copy-click away regardless.
                            format!("{} \u{00b7} {command_preview}", tr!("cli_setup.not_installed"))
                        }),
                ),
        )
        // Copy stays: when an install fails for reasons the app cannot fix —
        // a proxy, a root-owned prefix — the user needs the exact command.
        .child(
            div()
                .id(SharedString::from(format!("cli-copy-{id}")))
                .tab_index(0)
                .h(px(26.0))
                .px(px(9.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .child(tr!("cli_setup.copy"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_command.clone()));
                    this.show_toast(tr!("cli_setup.copied"));
                })),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if busy {
                return;
            }
            this.toggle_cli_selection(id.to_owned(), cx);
        }))
}
