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

use std::collections::HashMap;
use std::time::Duration;

use crate::ui::ActivationExt as _;

use super::settings::{abbreviate_home_path, detection_checked_label};
use super::*;

/// Latency past which a reachable endpoint is reported as slow.
const SLOW_ENDPOINT: Duration = Duration::from_millis(800);

/// View state for one CLI's endpoint form.
#[derive(Default)]
pub(super) struct EndpointFormState {
    /// The key field is shown as a text box rather than a masked stub.
    pub key_revealed: bool,
    /// A validation failure on the last save attempt, shown in the card.
    pub error: Option<String>,
    /// What `reconcile` warned about on the last save, kept until the next
    /// one — a toast would vanish before it could be read.
    pub last_warning: Option<String>,
    pub test: Option<EndpointTest>,
    /// A save is in flight; the buttons wait.
    pub saving: bool,
}

/// The connectivity test's progress and outcome.
pub(super) struct EndpointTest {
    pub running: bool,
    pub result: Option<sub2api::custom_api::ProbeResult>,
    generation: u64,
}

#[derive(Default)]
pub(super) struct ProvidersPageState {
    pub forms: HashMap<&'static str, EndpointFormState>,
    test_generation: u64,
}

/// Which route a CLI is on, resolved from memory: the cached endpoints and
/// the cloud account's credentials. No file is read on a frame.
fn cloud_config(waku: &Waku) -> Option<sub2api::GatewayConfig> {
    waku.cloud_account
        .credentials
        .as_ref()
        .map(|credentials| sub2api::gateway_config_from(credentials, waku.cloud_account.routing_enabled))
}

fn url_error_label(error: &sub2api::custom_api::UrlError) -> String {
    use sub2api::custom_api::UrlError;
    let reason = match error {
        UrlError::Empty => tr!("cli_setup.custom_url_empty"),
        UrlError::Whitespace => tr!("cli_setup.custom_url_whitespace"),
        UrlError::Scheme(scheme) => tr!("cli_setup.custom_url_scheme", scheme = scheme),
        UrlError::NoHost => tr!("cli_setup.custom_url_no_host"),
    };
    tr!("cli_setup.custom_invalid_url", reason = reason)
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
fn card_button(
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

/// A small label above a form field.
fn field_label(theme: Theme, text: String) -> Div {
    div()
        .text_size(sp(11.5))
        .text_color(theme.text_tertiary)
        .child(text)
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
    /// Wire the endpoint fields: Enter saves, and edits re-render so the
    /// Save/Discard buttons track the unsaved state. Called from the
    /// constructor, before `Waku` exists — hence a static fn.
    pub(super) fn subscribe_custom_api_inputs(
        inputs: &[(
            &'static str,
            Entity<TextInput>,
            Entity<TextInput>,
            Option<Entity<TextInput>>,
        )],
        cx: &mut Context<Self>,
    ) {
        for (provider_id, url_input, key_input, models_input) in inputs {
            let provider_id: &'static str = provider_id;
            let fields = [Some(url_input), Some(key_input), models_input.as_ref()];
            for input in fields.into_iter().flatten() {
                cx.subscribe(
                    input,
                    move |this: &mut Self, _, event: &InputEvent, cx| match event {
                        InputEvent::Submit(_) => this.save_endpoint_form(provider_id, cx),
                        InputEvent::Edited => cx.notify(),
                        _ => {}
                    },
                )
                .detach();
            }
        }
    }

    fn endpoint_inputs(
        &self,
        provider_id: &str,
    ) -> Option<(&Entity<TextInput>, &Entity<TextInput>, Option<&Entity<TextInput>>)> {
        self.custom_api_inputs
            .iter()
            .find(|(id, ..)| *id == provider_id)
            .map(|(_, url, key, models)| (url, key, models.as_ref()))
    }

    /// What the form holds right now, trimmed.
    fn endpoint_draft(&self, provider_id: &str, cx: &App) -> Option<(String, String, Vec<String>)> {
        let (url, key, models) = self.endpoint_inputs(provider_id)?;
        let models = models
            .map(|input| {
                input
                    .read(cx)
                    .content()
                    .split([',', '\u{3001}', ' ', '\n'])
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Some((
            url.read(cx).content().trim().to_owned(),
            key.read(cx).content().trim().to_owned(),
            models,
        ))
    }

    /// The form differs from what is stored — the Save/Discard gate.
    fn endpoint_form_dirty(&self, provider_id: &str, cx: &App) -> bool {
        let Some((url, key, models)) = self.endpoint_draft(provider_id, cx) else {
            return false;
        };
        let stored = self.custom_api_snapshot();
        match stored.get(provider_id) {
            Some(entry) => {
                url != entry.base_url.trim() || key != entry.api_key.trim() || models != entry.models
            }
            None => !url.is_empty() || !key.is_empty() || !models.is_empty(),
        }
    }

    /// Put the stored values back into the fields.
    fn discard_endpoint_form(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let stored = self.custom_api_snapshot();
        let entry = stored.get(provider_id).cloned().unwrap_or_default();
        if let Some((url, key, models)) = self.endpoint_inputs(provider_id) {
            let (url, key, models) = (url.clone(), key.clone(), models.cloned());
            url.update(cx, |input, cx| input.set_content(entry.base_url.clone(), cx));
            key.update(cx, |input, cx| input.set_content(entry.api_key.clone(), cx));
            if let Some(models) = models {
                models.update(cx, |input, cx| input.set_content(entry.models.join(", "), cx));
            }
        }
        let form = self.cli_setup.page.forms.entry(provider_id).or_default();
        form.error = None;
        cx.notify();
    }

    /// Validate the form and write it: the endpoint file, then the CLI's own
    /// global configuration — both off the UI thread. Both fields filled
    /// saves; both empty clears; one of each is refused, since a URL without
    /// a key would route with no credentials.
    pub(super) fn save_endpoint_form(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some((raw_url, api_key, models)) = self.endpoint_draft(provider_id, cx) else {
            return;
        };
        if self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .is_some_and(|form| form.saving)
        {
            return;
        }
        let clearing = raw_url.is_empty() && api_key.is_empty();
        let endpoint = if clearing {
            None
        } else {
            if raw_url.is_empty() != api_key.is_empty() {
                self.cli_setup.page.forms.entry(provider_id).or_default().error =
                    Some(tr!("cli_setup.custom_need_both"));
                cx.notify();
                return;
            }
            let base_url = match sub2api::custom_api::normalize_base_url(&raw_url) {
                Ok(url) => url,
                Err(error) => {
                    self.cli_setup.page.forms.entry(provider_id).or_default().error =
                        Some(url_error_label(&error));
                    cx.notify();
                    return;
                }
            };
            Some(sub2api::custom_api::CustomEndpoint {
                base_url,
                api_key,
                models,
            })
        };
        // Show the normalized URL so what is saved is what is seen.
        if let Some(endpoint) = &endpoint
            && let Some((url_input, ..)) = self.endpoint_inputs(provider_id)
        {
            let url_input = url_input.clone();
            let normalized = endpoint.base_url.clone();
            url_input.update(cx, |input, cx| input.set_content(normalized, cx));
        }
        let cloud = cloud_config(self);
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.error = None;
            form.last_warning = None;
            form.saving = true;
            form.test = None;
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let mut config = sub2api::custom_api::load();
                    config.set(provider_id, endpoint);
                    sub2api::custom_api::save(&config)?;
                    let desired = sub2api::global_config::desired_routes(cloud.as_ref(), &config);
                    let warnings = sub2api::global_config::reconcile(&desired)?;
                    anyhow::Ok((config, warnings))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                form.saving = false;
                match outcome {
                    Ok((config, warnings)) => {
                        *this.cli_setup.custom_cache.borrow_mut() = Some(config);
                        if !warnings.is_empty() {
                            form.last_warning = Some(warnings.join("\n"));
                        }
                        if clearing {
                            form.key_revealed = false;
                        }
                        this.show_toast(if clearing {
                            tr!("cli_setup.custom_cleared")
                        } else {
                            tr!("cli_setup.custom_saved")
                        });
                    }
                    Err(error) => form.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ask, then empty the fields and save — which restores the CLI's
    /// original configuration.
    fn confirm_clear_endpoint(
        &mut self,
        kind: ProviderKind,
        provider_id: &'static str,
        cx: &mut Context<Self>,
    ) {
        let stored = self.custom_api_snapshot();
        let detail = stored
            .get(provider_id)
            .map(|entry| entry.base_url.clone())
            .filter(|url| !url.is_empty());
        let mut detail_text = tr!("cli_setup.custom_clear_confirm_detail");
        if let Some(url) = detail {
            detail_text = format!("{url}\n{detail_text}");
        }
        self.request_confirm(
            tr!("cli_setup.custom_clear_confirm", name = kind.display_name()),
            Some(detail_text),
            tr!("cli_setup.custom_clear"),
            true,
            cx,
            move |this, _, cx| {
                if let Some((url, key, models)) = this.endpoint_inputs(provider_id) {
                    let (url, key, models) = (url.clone(), key.clone(), models.cloned());
                    url.update(cx, |input, cx| input.clear(cx));
                    key.update(cx, |input, cx| input.clear(cx));
                    if let Some(models) = models {
                        models.update(cx, |input, cx| input.clear(cx));
                    }
                }
                this.save_endpoint_form(provider_id, cx);
            },
        );
    }

    /// Probe the typed endpoint with the typed key, the way the CLI would.
    fn test_endpoint_form(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some((raw_url, api_key, _)) = self.endpoint_draft(provider_id, cx) else {
            return;
        };
        let base_url = match sub2api::custom_api::normalize_base_url(&raw_url) {
            Ok(url) => url,
            Err(error) => {
                self.cli_setup.page.forms.entry(provider_id).or_default().error =
                    Some(url_error_label(&error));
                cx.notify();
                return;
            }
        };
        self.cli_setup.page.test_generation += 1;
        let generation = self.cli_setup.page.test_generation;
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.error = None;
            form.test = Some(EndpointTest {
                running: true,
                result: None,
                generation,
            });
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    sub2api::custom_api::probe_endpoint(provider_id, &base_url, &api_key)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                if form
                    .test
                    .as_ref()
                    .is_some_and(|test| test.generation == generation)
                {
                    form.test = Some(EndpointTest {
                        running: false,
                        result: Some(result),
                        generation,
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

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
        let mut card = div()
            .w_full()
            .px(px(20.0))
            .py(px(12.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(status_line(
                theme,
                "icons/alert.svg",
                theme.warning,
                tr!("cli_setup.env_conflicts_title"),
            ))
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
        let probe = self.provider_probe(kind);
        let daemon_installed = probe.is_some_and(|probe| probe.installed);
        // The fork's own pass runs the binary; the daemon only finds it.
        let detection = snapshot.and_then(|snapshot| snapshot.detection(provider_id));
        let not_runnable = detection.and_then(|detection| match &detection.probe {
            Probe::FoundButFailed { diagnostic, .. } => Some(diagnostic.clone()),
            _ => None,
        });
        let installed = daemon_installed || detection.is_some_and(|detection| detection.is_installed());
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
            card = card.child(self.render_provider_expanded_settings(kind, theme, cx));
            if sub2api::custom_api::CUSTOM_API_PROVIDERS.contains(&provider_id) {
                card = card
                    .child(self.render_route_section(kind, provider_id, theme, cx))
                    .child(self.render_endpoint_form(kind, provider_id, theme, cx));
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
    fn render_route_section(
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

    fn render_endpoint_form(
        &self,
        kind: ProviderKind,
        provider_id: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        use sub2api::custom_api::ProbeVerdict;

        let Some((url_input, key_input, models_input)) = self.endpoint_inputs(provider_id) else {
            return div();
        };
        let (url_input, key_input, models_input) =
            (url_input.clone(), key_input.clone(), models_input.cloned());
        let stored = self.custom_api_snapshot();
        let entry = stored.get(provider_id).cloned();
        let dirty = self.endpoint_form_dirty(provider_id, cx);
        let form = self.cli_setup.page.forms.get(provider_id);
        let saving = form.is_some_and(|form| form.saving);
        let key_revealed = form.is_some_and(|form| form.key_revealed);
        let key_content = key_input.read(cx).content().trim().to_owned();
        let testing = form
            .and_then(|form| form.test.as_ref())
            .is_some_and(|test| test.running);

        let hint = if provider_id == "claude" {
            tr!("cli_setup.custom_hint_anthropic")
        } else {
            tr!("cli_setup.custom_hint_openai")
        };

        // The key: a masked stub with a reveal button until the user asks,
        // or the field itself while there is nothing to hide.
        let key_row: Div = if key_revealed || key_content.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    TextField::new(
                        SharedString::from(format!("custom-api-key-{provider_id}")),
                        key_input.clone(),
                    )
                    .flex_1(),
                )
                .when(key_revealed && !key_content.is_empty(), |row| {
                    row.child(card_button(
                        theme,
                        SharedString::from(format!("custom-api-hide-{provider_id}")),
                        tr!("cli_setup.custom_hide"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| {
                            this.cli_setup.page.forms.entry(provider_id).or_default().key_revealed =
                                false;
                            cx.notify();
                        },
                    ))
                })
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .h(px(28.0))
                        .px(px(8.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme.border_strong)
                        .bg(theme.inset)
                        .flex()
                        .items_center()
                        .font_family(crate::md::render::MONO_FAMILY)
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(super::cli_setup::mask_api_key(&key_content)),
                )
                .child(card_button(
                    theme,
                    SharedString::from(format!("custom-api-reveal-{provider_id}")),
                    tr!("cli_setup.custom_reveal"),
                    false,
                    false,
                    cx,
                    move |this, window, cx| {
                        this.cli_setup.page.forms.entry(provider_id).or_default().key_revealed =
                            true;
                        if let Some((_, key, _)) = this.endpoint_inputs(provider_id) {
                            let focus = key.read(cx).focus();
                            window.focus(&focus, cx);
                        }
                        cx.notify();
                    },
                ))
        };

        let mut section = div()
            .mt(px(10.0))
            .pl(px(42.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("cli_setup.custom_title")),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("cli_setup.custom_detail")),
            )
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.warning)
                    .child(hint),
            )
            .child(field_label(theme, tr!("cli_setup.custom_url_label")))
            .child(
                TextField::new(
                    SharedString::from(format!("custom-api-url-{provider_id}")),
                    url_input,
                )
                .w_full()
                .max_w(px(520.0)),
            )
            .child(field_label(theme, tr!("cli_setup.custom_key_label")))
            .child(key_row.w_full().max_w(px(520.0)));

        if let Some(models_input) = models_input {
            section = section
                .child(field_label(theme, tr!("cli_setup.custom_models_label")))
                .child(
                    TextField::new(
                        SharedString::from(format!("custom-api-models-{provider_id}")),
                        models_input,
                    )
                    .w_full()
                    .max_w(px(520.0)),
                );
        }

        if let Some(error) = form.and_then(|form| form.error.clone()) {
            section = section.child(status_line(theme, "icons/x.svg", theme.danger, error));
        }
        if let Some(warning) = form.and_then(|form| form.last_warning.clone()) {
            section = section.child(status_line(theme, "icons/alert.svg", theme.warning, warning));
        }
        if let Some(test) = form.and_then(|form| form.test.as_ref()) {
            let line = if test.running {
                status_line(
                    theme,
                    "icons/loader-circle.svg",
                    theme.text_ghost,
                    tr!("cli_setup.custom_testing"),
                )
            } else if let Some(result) = &test.result {
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
            } else {
                div()
            };
            section = section.child(line);
        }

        let mut actions = div().mt(px(2.0)).flex().flex_wrap().items_center().gap(px(6.0));
        if dirty {
            actions = actions
                .child(card_button(
                    theme,
                    SharedString::from(format!("custom-api-save-{provider_id}")),
                    tr!("cli_setup.custom_save"),
                    true,
                    saving,
                    cx,
                    move |this, _, cx| this.save_endpoint_form(provider_id, cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from(format!("custom-api-cancel-{provider_id}")),
                    tr!("cli_setup.custom_cancel"),
                    false,
                    saving,
                    cx,
                    move |this, _, cx| this.discard_endpoint_form(provider_id, cx),
                ))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!("cli_setup.custom_unsaved")),
                );
        }
        actions = actions
            .child(card_button(
                theme,
                SharedString::from(format!("custom-api-test-{provider_id}")),
                if testing {
                    tr!("cli_setup.custom_testing")
                } else {
                    tr!("cli_setup.custom_test")
                },
                false,
                testing,
                cx,
                move |this, _, cx| this.test_endpoint_form(provider_id, cx),
            ))
            .child(card_button(
                theme,
                SharedString::from(format!("custom-api-open-{provider_id}")),
                tr!("cli_setup.custom_open_file"),
                false,
                false,
                cx,
                move |this, _, cx| this.open_provider_config_file(provider_id, cx),
            ));
        if entry.is_some_and(|entry| entry.is_usable()) {
            actions = actions.child(card_button(
                theme,
                SharedString::from(format!("custom-api-clear-{provider_id}")),
                tr!("cli_setup.custom_clear"),
                false,
                saving,
                cx,
                move |this, _, cx| this.confirm_clear_endpoint(kind, provider_id, cx),
            ));
        }
        section.child(actions)
    }
}
