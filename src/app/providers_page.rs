//! Settings → Providers: one card per agent CLI.
//!
//! Fork addition, replacing upstream's provider list. Upstream shows a
//! detection row per provider and nothing else; this fork adds installation,
//! cloud routing, and custom endpoints, and the first cut stacked those as
//! three unrelated blocks under the rows — the same CLI appeared twice, with
//! different affordances in each place. Here everything about one CLI lives
//! in that CLI's card: detection (with "installed but not runnable" as its
//! own state), an inline install when it is missing, the binary override,
//! which route it is on, and the custom endpoint form.
//!
//! Nothing here does I/O on a frame. Detection comes from the background
//! pass in `cli_setup`, the stored endpoints from a cache, and every save,
//! test, and install runs on the background executor and notifies back.

use std::time::Duration;

use sub2api::custom_api::ProbeVerdict;

use crate::ui::ActivationExt as _;

use super::settings::{abbreviate_home_path, detection_checked_label};
use super::*;

/// Latency past which a reachable endpoint is reported as slow.
const SLOW_ENDPOINT: Duration = Duration::from_millis(800);

/// The connectivity test's progress and outcome.
pub(super) struct EndpointTest {
    pub running: bool,
    pub result: Option<sub2api::custom_api::ProbeResult>,
    pub(super) generation: u64,
}

/// A candidate speed test's progress and results.
pub(super) struct SpeedTest {
    pub running: bool,
    /// One entry per candidate, in the order they were listed.
    pub results: Vec<sub2api::speedtest::CandidateResult>,
    pub(super) generation: u64,
}

/// Which route a CLI is on, resolved from memory: the cached endpoints and
/// the cloud account's credentials. No file is read on a frame.
pub(super) fn cloud_config(waku: &Waku) -> Option<sub2api::GatewayConfig> {
    let origin = waku.cloud_account.gateway_origin.origin();
    waku.cloud_account.credentials.as_ref().map(|credentials| {
        sub2api::gateway_config_with_origin(
            credentials,
            waku.cloud_account.routing_enabled,
            origin.as_deref(),
        )
    })
}

pub(super) fn url_error_label(error: &sub2api::custom_api::UrlError) -> String {
    use sub2api::custom_api::UrlError;
    let reason = match error {
        UrlError::Empty => tr!("cli_setup.custom_url_empty"),
        UrlError::Whitespace => tr!("cli_setup.custom_url_whitespace"),
        UrlError::Scheme(scheme) => tr!("cli_setup.custom_url_scheme", scheme = scheme),
        UrlError::NoHost => tr!("cli_setup.custom_url_no_host"),
    };
    tr!("cli_setup.custom_invalid_url", reason = reason)
}

/// Colour for a measured latency. Always rendered beside the number itself,
/// never as the only signal.
pub(super) fn latency_color(theme: Theme, ms: u128) -> gpui::Hsla {
    use sub2api::speedtest::LatencyTier;
    match sub2api::speedtest::latency_tier(ms) {
        LatencyTier::Fast | LatencyTier::Ok => theme.success,
        LatencyTier::Slow => theme.warning,
        LatencyTier::VerySlow => theme.danger,
    }
}

fn env_source_label(source: &sub2api::env_conflicts::ConflictSource) -> String {
    use sub2api::env_conflicts::ConflictSource;
    match source {
        ConflictSource::Process => tr!("cli_setup.env_source_process"),
        ConflictSource::WindowsUser => tr!("cli_setup.env_source_user"),
        ConflictSource::WindowsMachine => tr!("cli_setup.env_source_machine"),
        ConflictSource::ShellFile { path, line } => tr!(
            "cli_setup.env_source_file",
            path = path.display().to_string(),
            line = line
        ),
    }
}

/// A card action button. Every one is keyboard-operable: focusable, with
/// a visible focus ring, and Enter/Space activate it like a click.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_button(
    theme: Theme,
    id: SharedString,
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
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_default()
        .text_size(sp(11.5))
        .opacity(if disabled { 0.55 } else { 1.0 });
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
    let button = button.child(label);
    if disabled {
        button
    } else {
        button.on_activation(cx, activate)
    }
}

/// How a connectivity probe went, in one line.
///
/// Shared with the model-providers page so the two surfaces cannot drift
/// into describing the same failure differently — an unauthorized key and an
/// unreachable host are different problems, and the wording is the only
/// thing that says which.
pub(super) fn probe_status_line(theme: Theme, test: &EndpointTest) -> Div {
    if test.running {
        return status_line(
            theme,
            "icons/loader-circle.svg",
            theme.text_ghost,
            tr!("cli_setup.custom_testing"),
        );
    }
    let Some(result) = &test.result else {
        return div();
    };
    match result.verdict {
        ProbeVerdict::Ok if result.latency_ms < SLOW_ENDPOINT.as_millis() => status_line(
            theme,
            "icons/check.svg",
            theme.success,
            tr!("cli_setup.custom_connect_ok", ms = result.latency_ms),
        ),
        ProbeVerdict::Ok => status_line(
            theme,
            "icons/check.svg",
            theme.warning,
            tr!("cli_setup.custom_connect_slow", ms = result.latency_ms),
        ),
        ProbeVerdict::Unauthorized => status_line(
            theme,
            "icons/alert.svg",
            theme.warning,
            tr!(
                "cli_setup.custom_test_unauthorized",
                status = result.status.unwrap_or_default()
            ),
        ),
        ProbeVerdict::HttpError => status_line(
            theme,
            "icons/alert.svg",
            theme.warning,
            tr!(
                "cli_setup.custom_test_http",
                status = result.status.unwrap_or_default(),
                detail = result.detail.clone()
            ),
        ),
        ProbeVerdict::Unreachable => status_line(
            theme,
            "icons/x.svg",
            theme.danger,
            tr!("cli_setup.custom_connect_failed", error = result.detail.clone()),
        ),
    }
}

/// A status line inside a card: icon, tinted text.
fn status_line(theme: Theme, icon_path: &'static str, color: gpui::Hsla, text: String) -> Div {
    let _ = theme;
    div()
        .flex()
        .items_start()
        .gap(px(6.0))
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(color)
        .child(div().flex_none().pt(px(2.0)).child(icon(icon_path, 12.0, color)))
        .child(div().min_w_0().flex_1().child(text))
}

impl Waku {
    /// Open the CLI's live configuration file — the artifact routing writes,
    /// and the thing users check to trust it.
    fn open_provider_config_file(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some(path) = sub2api::global_config::config_file_for(provider_id) else {
            return;
        };
        if !path.exists() {
            self.show_toast(tr!("cli_setup.custom_file_missing"));
            return;
        }
        cx.open_url(&path.display().to_string());
    }

    /// Install one CLI from its card: tick just that one and run the batch,
    /// which installs Node first when it has to.
    fn install_provider_cli(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        {
            let mut selected = self.cli_setup.selected.borrow_mut();
            selected.clear();
            selected.insert(provider_id.to_owned());
        }
        self.run_selected_cli_installs(cx);
    }

    /// Point the provider at the binary an install left outside the search
    /// directories, through the same override the expanded row edits.
    fn use_installed_path(
        &mut self,
        kind: ProviderKind,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.state
            .provider_binary_overrides
            .insert(kind, path.display().to_string());
        self.save();
        self.refresh_provider_detection(Some(kind));
        self.refresh_composer_sources(cx);
        cx.notify();
    }

    /// Windows: the Environment Variables dialog; elsewhere the unset line
    /// is copied, since editing shell profiles from here is not on offer.
    fn open_environment_variables(&mut self, cx: &mut Context<Self>) {
        if cfg!(target_os = "windows") {
            cx.background_executor()
                .spawn(async {
                    let mut command = std::process::Command::new("rundll32");
                    command.args(["sysdm.cpl,EditEnvironmentVariables"]);
                    #[cfg(target_os = "windows")]
                    {
                        use std::os::windows::process::CommandExt as _;
                        command.creation_flags(0x0800_0000);
                    }
                    let _ = command.spawn();
                })
                .detach();
        }
    }

    // ── Rendering ──────────────────────────────────────────────────────

    pub(super) fn render_providers_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        self.ensure_cli_environment_fresh(cx);
        let snapshot = self.cli_setup.snapshot.clone();
        let checking = self.provider_detection_remaining > 0 || self.cli_setup.snapshot_pending();
        let checked_label = self
            .provider_detection_checked_at
            .filter(|_| !checking)
            .map(|checked_at| detection_checked_label(checked_at.elapsed()));

        let refresh = div()
            .id("refresh-providers")
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(28.0))
            .px(px(11.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .gap(px(6.0))
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .opacity(if checking { 0.6 } else { 1.0 })
            .hover(|element| element.bg(theme.overlay))
            .child(icon("icons/rotate-cw.svg", 11.0, theme.text_tertiary))
            .child(if checking {
                tr!("common.checking")
            } else {
                tr!("common.refresh")
            })
            .on_activation(cx, |this, _, cx| {
                this.refresh_provider_detection(None);
                this.refresh_cli_environment(cx);
                cx.notify();
            });

        let header = div()
            .flex()
            .items_start()
            .gap(px(20.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(sp(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("providers.coding_agents")),
                    )
                    .child(
                        div()
                            .mt(px(5.0))
                            .text_size(sp(12.5))
                            .line_height(sp(18.0))
                            .text_color(theme.text_secondary)
                            .child(tr!("providers.description")),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(6.0))
                    .child(refresh)
                    .when_some(checked_label, |element, label| {
                        element.child(
                            div()
                                .text_size(sp(12.5))
                                .text_color(theme.text_ghost)
                                .child(SharedString::from(label)),
                        )
                    }),
            );

        let mut page = div()
            .mt(px(15.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .w_full()
                    .px(px(20.0))
                    .py(px(14.0))
                    .rounded(px(13.0))
                    .bg(theme.raised)
                    .child(header),
            )
            .child(self.render_runtime_card(snapshot.as_deref(), theme, cx));

        if let Some(snapshot) = snapshot.as_deref()
            && !snapshot.conflicts.is_empty()
        {
            page = page.child(self.render_env_conflicts_card(&snapshot.conflicts, theme, cx));
        }

        for kind in ProviderKind::ALL {
            // The built-in agent has no card here: it has no binary to find,
            // no version to report and no installer to run, and everything
            // that *is* configurable about it — endpoints, behaviour, tools,
            // MCP servers, permission rules — lives on the Agent page. A
            // card carrying only a name would just be one more place to look.
            if kind.is_builtin() {
                continue;
            }
            page = page.child(self.render_provider_card(kind, snapshot.as_deref(), theme, cx));
        }

        if let Some(error) = self.cli_setup.last_error.clone() {
            page = page.child(
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

        page.into_any_element()
    }

    /// Node and npm: the one prerequisite every npm install shares, so it
    /// is stated once at the top rather than on every card.
    fn render_runtime_card(
        &self,
        snapshot: Option<&sub2api::cli_detect::EnvironmentSnapshot>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::cli_detect::Probe;

        let running = self.cli_setup.running.as_deref() == Some("node");
        let busy = self.cli_setup.running.is_some();
        let installable = sub2api::node_install::install_supported();

        let (status_icon, status_color, status_text) = match snapshot.map(|snapshot| &snapshot.node) {
            None => (
                "icons/loader-circle.svg",
                theme.text_ghost,
                tr!("cli_setup.detecting"),
            ),
            Some(Probe::Found { version, .. }) if sub2api::cli_install::node_is_supported(version) => {
                let npm = snapshot
                    .and_then(|snapshot| snapshot.npm.version())
                    .map(|npm| format!("  \u{00b7}  npm {npm}"))
                    .unwrap_or_default();
                (
                    "icons/check.svg",
                    theme.success,
                    format!("{}{npm}", version.trim()),
                )
            }
            Some(Probe::Found { version, .. }) => (
                "icons/alert.svg",
                theme.warning,
                format!(
                    "{}  \u{00b7}  {}",
                    tr!("cli_setup.node_found", version = version.trim()),
                    tr!(
                        "cli_setup.node_requirement",
                        major = sub2api::cli_install::REQUIRED_NODE_MAJOR
                    )
                ),
            ),
            Some(Probe::FoundButFailed { diagnostic, .. }) => (
                "icons/alert.svg",
                theme.warning,
                tr!("cli_setup.node_not_runnable", detail = diagnostic),
            ),
            Some(Probe::NotFound) if !installable => (
                "icons/alert.svg",
                theme.warning,
                tr!("cli_setup.node_manual"),
            ),
            Some(Probe::NotFound) => (
                "icons/alert.svg",
                theme.warning,
                tr!("cli_setup.node_missing"),
            ),
        };
        let needs_install = installable
            && snapshot.is_some_and(|snapshot| {
                !snapshot
                    .node
                    .version()
                    .is_some_and(sub2api::cli_install::node_is_supported)
            });
        let stage = if running {
            self.cli_setup.node_stage.lock().unwrap().clone()
        } else {
            None
        };

        div()
            .w_full()
            .px(px(20.0))
            .py(px(12.0))
            .rounded(px(13.0))
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
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(format!(
                                "{}  \u{00b7}  Node.js",
                                tr!("cli_setup.env_title")
                            )),
                    )
                    .child(status_line(
                        theme,
                        status_icon,
                        status_color,
                        stage.unwrap_or(status_text),
                    )),
            )
            .when(needs_install, |card| {
                card.child(card_button(
                    theme,
                    "run-toolchain-install".into(),
                    if running {
                        tr!("cli_setup.installing")
                    } else {
                        tr!("cli_setup.install")
                    },
                    true,
                    busy,
                    cx,
                    |this, _, cx| this.run_node_install(cx),
                ))
            })
    }

    fn render_env_conflicts_card(
        &self,
        conflicts: &[sub2api::env_conflicts::EnvConflict],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::env_fix::FixPlan;

        let busy = self.cli_setup.env_fix_busy;
        let removable: Vec<sub2api::env_conflicts::EnvConflict> = conflicts
            .iter()
            .filter(|conflict| {
                matches!(
                    sub2api::env_fix::plan(conflict),
                    FixPlan::RemoveUserVar | FixPlan::CommentOutLine { .. }
                )
            })
            .cloned()
            .collect();
        let has_backup = self.cli_setup.env_backup_latest.is_some();

        let mut header = div()
            .flex()
            .items_start()
            .gap(px(8.0))
            .child(div().flex_1().min_w_0().child(status_line(
                theme,
                "icons/alert.svg",
                theme.warning,
                tr!("cli_setup.env_conflicts_title"),
            )));
        if removable.len() > 1 {
            let all = removable.clone();
            header = header.child(card_button(
                theme,
                "env-conflict-remove-all".into(),
                tr!("cli_setup.env_conflict_remove_all"),
                true,
                busy,
                cx,
                move |this, _, cx| this.confirm_remove_env_conflicts(all.clone(), cx),
            ));
        }
        if has_backup {
            header = header.child(card_button(
                theme,
                "env-conflict-restore".into(),
                tr!("cli_setup.env_conflict_restore"),
                false,
                busy,
                cx,
                |this, _, cx| this.restore_last_env_backup(cx),
            ));
        }

        let mut card = div()
            .w_full()
            .px(px(20.0))
            .py(px(12.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(header)
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.text_secondary)
                    .child(tr!("cli_setup.env_conflicts_detail")),
            );
        for (index, conflict) in conflicts.iter().enumerate() {
            let name = conflict.name.clone();
            let unset_line = if cfg!(target_os = "windows") {
                format!("[Environment]::SetEnvironmentVariable('{name}', $null, 'User')")
            } else {
                format!("unset {name}")
            };
            let plan = sub2api::env_fix::plan(conflict);
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(crate::md::render::MONO_FAMILY)
                        .text_size(sp(12.0))
                        .text_color(theme.text)
                        .child(format!("{}={}", conflict.name, conflict.value_masked)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(sp(11.5))
                        .text_color(theme.text_tertiary)
                        .child(format!(
                            "{}  \u{00b7}  {}",
                            conflict.provider_id,
                            env_source_label(&conflict.source)
                        )),
                );
            match &plan {
                // Removable from here: the Windows per-user block, or a
                // line in a shell profile.
                FixPlan::RemoveUserVar | FixPlan::CommentOutLine { .. } => {
                    let one = vec![conflict.clone()];
                    row = row.child(card_button(
                        theme,
                        SharedString::from(format!("env-conflict-remove-{index}")),
                        tr!("cli_setup.env_conflict_remove"),
                        false,
                        busy,
                        cx,
                        move |this, _, cx| {
                            this.confirm_remove_env_conflicts(one.clone(), cx)
                        },
                    ));
                }
                // Machine-wide: an elevated shell is required, so the
                // command is handed over instead of being run.
                FixPlan::RemoveMachineVar { elevated_command } => {
                    let command = elevated_command.clone();
                    row = row.child(card_button(
                        theme,
                        SharedString::from(format!("env-conflict-admin-{index}")),
                        tr!("cli_setup.env_conflict_copy_admin"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                            this.show_toast(tr!("cli_setup.env_conflict_needs_admin"));
                        },
                    ));
                }
                FixPlan::SkipProcess => {}
            }
            if cfg!(target_os = "windows")
                && !matches!(
                    conflict.source,
                    sub2api::env_conflicts::ConflictSource::Process
                )
            {
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("env-conflict-open-{index}")),
                    tr!("cli_setup.env_conflict_open_settings"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.open_environment_variables(cx),
                ));
            }
            row = row.child(card_button(
                theme,
                SharedString::from(format!("env-conflict-copy-{index}")),
                tr!("cli_setup.env_conflict_copy_unset"),
                false,
                false,
                cx,
                move |this, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(unset_line.clone()));
                    this.show_toast(tr!("cli_setup.env_conflict_copied"));
                },
            ));
            card = card.child(row);
            // Nothing here can change a variable the app already inherited;
            // say so rather than offering a button that would not work.
            if matches!(plan, FixPlan::SkipProcess) {
                card = card.child(
                    div()
                        .text_size(sp(11.5))
                        .line_height(sp(16.0))
                        .text_color(theme.text_ghost)
                        .child(tr!("cli_setup.env_conflict_process_note")),
                );
            }
        }
        if let Some(report) = self.cli_setup.env_fix_report.clone() {
            card = card.child(status_line(
                theme,
                "icons/alert.svg",
                theme.warning,
                report,
            ));
        }
        card
    }

    fn render_provider_card(
        &self,
        kind: ProviderKind,
        snapshot: Option<&sub2api::cli_detect::EnvironmentSnapshot>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::cli_detect::Probe;

        let provider_id: &'static str = kind.id();
        // A built-in provider has no binary, so every detection question below
        // is meaningless for it: it is always present, never installable, and
        // has no path or version to show. Answering them this way is what
        // collapses its card to the parts that do apply - routing and models.
        // Only CLIs reach this: the built-in agent is skipped by the caller,
        // because it has no binary to detect, version or install.
        debug_assert!(!kind.is_builtin());
        let probe = self.provider_probe(kind);
        let daemon_installed = probe.is_some_and(|probe| probe.installed);
        // The fork's own pass runs the binary; the daemon only finds it.
        let detection = snapshot.and_then(|snapshot| snapshot.detection(provider_id));
        let not_runnable = detection.and_then(|detection| match &detection.probe {
            Probe::FoundButFailed { diagnostic, .. } => Some(diagnostic.clone()),
            _ => None,
        });
        let installed =
            daemon_installed || detection.is_some_and(|detection| detection.is_installed());
        let disabled = self.state.disabled_providers.contains(&kind);
        let descriptor = sub2api::cli_install::descriptor(provider_id);
        let installable = descriptor.is_some() && !installed;
        let running = self.cli_setup.running.as_deref() == Some(provider_id);
        let busy = self.cli_setup.running.is_some();

        let version = self
            .provider_versions
            .get(&kind)
            .and_then(|version| version.clone())
            .or_else(|| {
                detection
                    .and_then(|detection| detection.probe.version())
                    .map(|version| version.trim_start_matches('v').to_owned())
            });
        let binary_path = probe
            .filter(|probe| probe.installed)
            .and_then(|probe| probe.path.as_deref())
            .or_else(|| detection.and_then(|detection| detection.path()))
            .map(|path| abbreviate_home_path(path, self.home_directory.as_deref()));
        let model_count = probe.map(|probe| probe.models.len()).unwrap_or(0);

        let (dot_color, status_text, status_color) = if let Some(diagnostic) = &not_runnable {
            (
                theme.warning,
                format!("{}: {diagnostic}", tr!("providers.status_not_runnable")),
                theme.warning,
            )
        } else if !installed {
            (
                theme.text_ghost,
                tr!("providers.not_detected_as", command = kind.command()),
                theme.text_tertiary,
            )
        } else if disabled {
            (
                theme.warning,
                tr!("providers.disabled_for_new_tasks"),
                theme.text_tertiary,
            )
        } else {
            let mut parts = vec![tr!("providers.status_installed")];
            if let Some(path) = binary_path {
                parts.push(path);
            }
            if model_count > 0 {
                parts.push(if model_count == 1 {
                    tr!("providers.model_count_one", count = model_count)
                } else {
                    tr!("providers.model_count_many", count = model_count)
                });
            }
            (theme.success, parts.join("  \u{00b7}  "), theme.text_tertiary)
        };

        let expanded = self.expanded_provider_settings == Some(kind);
        let expand_button = icon_button(
            SharedString::from(format!("provider-expand-{provider_id}")),
            if expanded {
                "icons/chevron-down.svg"
            } else {
                "icons/chevron-right.svg"
            },
            theme,
        )
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.toggle_provider_expanded(kind, window, cx);
        }));

        let mut header = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .relative()
                    .w(px(30.0))
                    .h(px(30.0))
                    .flex_none()
                    .rounded(px(7.0))
                    .bg(theme.overlay)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(
                        provider_icon(kind),
                        16.0,
                        provider_color(&theme, kind).opacity(if installed { 1.0 } else { 0.5 }),
                    ))
                    .child(
                        div()
                            .absolute()
                            .bottom(px(-2.0))
                            .right(px(-2.0))
                            .w(px(10.0))
                            .h(px(10.0))
                            .rounded_full()
                            .border_2()
                            .border_color(theme.raised)
                            .bg(dot_color),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if installed {
                                        theme.text
                                    } else {
                                        theme.text_secondary
                                    })
                                    .child(kind.display_name()),
                            )
                            .when_some(version, |element, version| {
                                element.child(
                                    div()
                                        .font_family(crate::md::render::MONO_FAMILY)
                                        .text_size(sp(12.5))
                                        .text_color(theme.text_tertiary)
                                        .child(SharedString::from(format!("v{version}"))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt(px(3.0))
                            .text_size(sp(12.5))
                            .line_height(sp(16.0))
                            .text_color(status_color)
                            .child(if running {
                                self.cli_setup
                                    .node_stage
                                    .lock()
                                    .unwrap()
                                    .clone()
                                    .unwrap_or_else(|| tr!("cli_setup.installing"))
                            } else {
                                status_text
                            }),
                    ),
            );

        if installable && let Some(descriptor) = descriptor {
            let command = sub2api::cli_install::install_candidates(descriptor.package)
                .into_iter()
                .next()
                .unwrap_or_default();
            header = header
                .child(card_button(
                    theme,
                    SharedString::from(format!("cli-copy-{provider_id}")),
                    tr!("cli_setup.copy"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                        this.show_toast(tr!("cli_setup.copied"));
                    },
                ))
                .child(card_button(
                    theme,
                    SharedString::from(format!("cli-install-{provider_id}")),
                    if running {
                        tr!("cli_setup.installing")
                    } else {
                        tr!("cli_setup.install")
                    },
                    true,
                    busy,
                    cx,
                    move |this, _, cx| this.install_provider_cli(provider_id, cx),
                ));
        }

        header = header.child(expand_button);
        if installed {
            let toggle = toggle_switch(
                SharedString::from(format!("provider-enabled-{provider_id}")),
                !disabled,
                false,
                theme,
                cx,
                move |this, _, cx| this.set_provider_enabled(kind, disabled, cx),
            );
            header = header.child(toggle);
        }

        let mut card = div()
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(header);

        // How the last install from this card ended, verified against the
        // binary rather than npm's exit status.
        if let Some(verdict) = self.cli_setup.install_results.get(provider_id) {
            card = card.child(self.render_install_verdict(kind, verdict, theme, cx));
        }

        if expanded {
            // The binary-path editor is the whole of the expanded settings for
            // a CLI, and this page only renders CLIs now — the built-in
            // agent's settings all live on the Agent page.
            card = card.child(self.render_provider_expanded_settings(kind, theme, cx));
            if sub2api::custom_api::CUSTOM_API_PROVIDERS.contains(&provider_id) {
                // Which endpoint, not what the endpoint is: the address, key
                // and models are described once on the model-providers page
                // and chosen here. A form per CLI is how the same relay came
                // to be typed in three times.
                card = card
                    .child(self.render_route_section(kind, provider_id, theme, cx))
                    .child(
                        div()
                            .mt(px(10.0))
                            .pl(px(42.0))
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(self.render_route_binding(provider_id, theme, cx))
                            // The live file routing actually writes — what a
                            // user opens to decide whether to trust any of
                            // this. It is about the CLI, not the endpoint, so
                            // it stayed here when the form left.
                            .child(
                                div().child(card_button(
                                    theme,
                                    SharedString::from(format!("open-config-{provider_id}")),
                                    tr!("cli_setup.custom_open_file"),
                                    false,
                                    false,
                                    cx,
                                    move |this, _, cx| {
                                        this.open_provider_config_file(provider_id, cx)
                                    },
                                )),
                            ),
                    );
            }
        }
        card
    }

    fn render_install_verdict(
        &self,
        kind: ProviderKind,
        verdict: &sub2api::cli_install::InstallVerdict,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::cli_install::{InstallHint, InstallVerdict};

        let provider_id = kind.id();
        match verdict {
            InstallVerdict::Installed { version, .. } => status_line(
                theme,
                "icons/check.svg",
                theme.success,
                tr!("providers.install_result_ok", version = version),
            ),
            InstallVerdict::InstalledNotOnPath { bin_dir, path, .. } => {
                let path = path.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div().flex_1().min_w_0().child(status_line(
                            theme,
                            "icons/alert.svg",
                            theme.warning,
                            tr!(
                                "cli_setup.installed_not_on_path",
                                name = kind.display_name(),
                                dir = bin_dir.display().to_string()
                            ),
                        )),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("use-installed-path-{provider_id}")),
                        tr!("cli_setup.use_this_path"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| this.use_installed_path(kind, path.clone(), cx),
                    ))
            }
            InstallVerdict::InstalledNotRunnable { .. } => status_line(
                theme,
                "icons/alert.svg",
                theme.warning,
                super::cli_setup::install_verdict_detail(verdict),
            ),
            InstallVerdict::Failed { output, hint } => {
                let mut block = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(status_line(
                        theme,
                        "icons/x.svg",
                        theme.danger,
                        match hint {
                            Some(InstallHint::Permission) => tr!("cli_setup.hint_permission"),
                            Some(InstallHint::Network) => tr!("cli_setup.hint_network"),
                            None => output.lines().last().unwrap_or_default().to_owned(),
                        },
                    ))
                    .child(
                        div()
                            .font_family(crate::md::render::MONO_FAMILY)
                            .text_size(sp(11.5))
                            .line_height(sp(16.0))
                            .text_color(theme.text_secondary)
                            .child(output.clone()),
                    );
                if *hint == Some(InstallHint::Permission) {
                    let commands = sub2api::cli_install::permission_fix_commands().join("\n");
                    block = block.child(div().flex().child(card_button(
                        theme,
                        SharedString::from(format!("copy-fix-{provider_id}")),
                        tr!("cli_setup.copy_fix"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(commands.clone()));
                            this.show_toast(tr!("cli_setup.copied"));
                        },
                    )));
                }
                block
            }
        }
    }

    /// Which configuration the CLI runs with, and where to change it.
    pub(super) fn render_route_section(
        &self,
        kind: ProviderKind,
        provider_id: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::global_config::RouteKind;

        let stored = self.custom_api_snapshot();
        let cloud = cloud_config(self);
        let route = sub2api::global_config::active_route_kind(provider_id, cloud.as_ref(), &stored);
        let custom_configured = stored.endpoint_for(provider_id).is_some();
        let (label, color) = match route {
            RouteKind::Cloud => (tr!("providers.route_cloud"), theme.accent),
            RouteKind::Custom => (tr!("providers.route_custom"), theme.success),
            // "The CLI's own configuration" is the wrong sentence for a
            // provider that is not a CLI: what it falls back to is the
            // engine's own settings file.
            RouteKind::CliOwn if kind.is_builtin() => {
                (tr!("providers.route_engine_default"), theme.text_tertiary)
            }
            RouteKind::CliOwn => (tr!("providers.route_cli_own"), theme.text_tertiary),
        };
        let _ = kind;

        div()
            .mt(px(10.0))
            .pl(px(42.0))
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("providers.route_title")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded_full()
                            .flex_none()
                            .bg(color),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(12.5))
                            .text_color(theme.text_secondary)
                            .child(label),
                    )
                    .when(matches!(route, RouteKind::Cloud) || cloud.is_some(), |row| {
                        row.child(card_button(
                            theme,
                            SharedString::from(format!("route-manage-{provider_id}")),
                            tr!("providers.route_manage"),
                            false,
                            false,
                            cx,
                            |this, _, cx| this.open_settings_page(SettingsPage::CloudAccount, cx),
                        ))
                    }),
            )
            .when(
                matches!(route, RouteKind::Cloud) && custom_configured,
                |section| {
                    section.child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.warning)
                            .child(tr!("cli_setup.custom_overridden")),
                    )
                },
            )
    }
}
