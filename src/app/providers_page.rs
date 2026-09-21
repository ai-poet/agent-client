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
    /// Field for adding an alternate domain. Built on first use: creating a
    /// `TextInput` needs a `Window`, which render does not have.
    pub candidate_input: Option<Entity<TextInput>>,
    /// Field for naming a profile, shown while `renaming` or `naming_new`.
    pub profile_name_input: Option<Entity<TextInput>>,
    /// The name field is committing a rename of the active profile.
    pub renaming: bool,
    /// The alternate-domain block is open. It also opens on its own once a
    /// profile has more than one candidate, so a single-URL form is
    /// unchanged from before profiles existed.
    pub candidates_open: bool,
    /// The last speed test's progress and results.
    pub speed: Option<SpeedTest>,
}

/// The connectivity test's progress and outcome.
pub(super) struct EndpointTest {
    pub running: bool,
    pub result: Option<sub2api::custom_api::ProbeResult>,
    generation: u64,
}

/// A candidate speed test's progress and results.
pub(super) struct SpeedTest {
    pub running: bool,
    /// One entry per candidate, in the order they were listed.
    pub results: Vec<sub2api::speedtest::CandidateResult>,
    generation: u64,
}

#[derive(Default)]
pub(super) struct ProvidersPageState {
    pub forms: HashMap<&'static str, EndpointFormState>,
    test_generation: u64,
    speed_generation: u64,
}

/// Which route a CLI is on, resolved from memory: the cached endpoints and
/// the cloud account's credentials. No file is read on a frame.
fn cloud_config(waku: &Waku) -> Option<sub2api::GatewayConfig> {
    let origin = waku.cloud_account.gateway_origin.origin();
    waku.cloud_account.credentials.as_ref().map(|credentials| {
        sub2api::gateway_config_with_origin(
            credentials,
            waku.cloud_account.routing_enabled,
            origin.as_deref(),
        )
    })
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

/// What to call a profile. Entries carried over from before profiles
/// existed have no name of their own.
fn profile_label(profile: &sub2api::custom_api::EndpointProfile) -> String {
    let name = profile.name.trim();
    if name.is_empty() {
        tr!("cli_setup.profile_default_name")
    } else {
        name.to_owned()
    }
}

/// Colour for a measured latency. Always rendered beside the number itself,
/// never as the only signal.
fn latency_color(theme: Theme, ms: u128) -> gpui::Hsla {
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

    /// Put the active profile's stored values into the three fields.
    fn refill_endpoint_fields(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
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
    }

    /// Put the stored values back into the fields.
    fn discard_endpoint_form(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        self.refill_endpoint_fields(provider_id, cx);
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
                let mut clear_the_key = false;
                let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                form.saving = false;
                match outcome {
                    Ok((config, warnings)) => {
                        *this.cli_setup.custom_cache.borrow_mut() = Some(config);
                        if !warnings.is_empty() {
                            form.last_warning = Some(warnings.join("\n"));
                        }
                        if clearing {
                            clear_the_key = true;
                        }
                        // The Chat route's model list is the picker's Chat
                        // section. Nothing else would notice until the next
                        // catalog or probe landed.
                        if provider_id == "native_chat" {
                            this.sync_native_models();
                        }
                        this.show_toast(if clearing {
                            tr!("cli_setup.custom_cleared")
                        } else {
                            tr!("cli_setup.custom_saved")
                        });
                    }
                    Err(error) => form.error = Some(format!("{error:#}")),
                }
                // After the `form` borrow, which the mask helper also needs.
                if clear_the_key {
                    this.remask_endpoint_key(provider_id, cx);
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

    // ── Endpoint profiles and alternate domains ────────────────────────

    /// One CLI's stored profiles, from the render-safe cache.
    fn endpoint_profiles(&self, provider_id: &str) -> sub2api::custom_api::ProviderProfiles {
        self.custom_api_snapshot()
            .profiles(provider_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Apply a change to one CLI's profiles and re-route it: load, mutate,
    /// save, reconcile — all off the UI thread — then refresh the cache and,
    /// when the active profile may have changed, the form fields.
    /// Put a key back behind its mask.
    ///
    /// Kept beside the flag it mirrors: `key_revealed` decides what the
    /// button says, the input decides what is drawn, and the two going out
    /// of step is exactly how a secret ends up on screen under a control
    /// that claims it is hidden.
    fn remask_endpoint_key(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        self.cli_setup.page.forms.entry(provider_id).or_default().key_revealed = false;
        let Some((_, key, _)) = self.endpoint_inputs(provider_id) else {
            return;
        };
        key.clone().update(cx, |input, cx| input.set_masked(true, cx));
    }

    fn commit_profiles(
        &mut self,
        provider_id: &'static str,
        refill: bool,
        toast: Option<String>,
        mutate: impl FnOnce(&mut sub2api::custom_api::ProviderProfiles) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        // Every profile operation lands a different key in the field, and
        // none of them should leave it on screen. The form flag alone would
        // only relabel the button — the input is what actually hides it.
        self.remask_endpoint_key(provider_id, cx);
        if self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .is_some_and(|form| form.saving)
        {
            return;
        }
        let cloud = cloud_config(self);
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.error = None;
            form.last_warning = None;
            form.saving = true;
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let mut config = sub2api::custom_api::load();
                    if let Some(slot) = config.profiles_mut(provider_id) {
                        mutate(slot);
                    }
                    sub2api::custom_api::save(&config)?;
                    let desired = sub2api::global_config::desired_routes(cloud.as_ref(), &config);
                    let warnings = sub2api::global_config::reconcile(&desired)?;
                    anyhow::Ok((config, warnings))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                {
                    let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                    form.saving = false;
                    match &outcome {
                        Ok((_, warnings)) if !warnings.is_empty() => {
                            form.last_warning = Some(warnings.join("\n"));
                        }
                        Ok(_) => {}
                        Err(error) => form.error = Some(format!("{error:#}")),
                    }
                }
                if let Ok((config, _)) = outcome {
                    *this.cli_setup.custom_cache.borrow_mut() = Some(config);
                    if refill {
                        this.refill_endpoint_fields(provider_id, cx);
                    }
                    if let Some(toast) = toast {
                        this.show_toast(toast);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Route this CLI through another saved profile. A CLI reads its config
    /// at process start, so the toast says the switch applies to new
    /// sessions rather than letting it read as "nothing happened".
    fn switch_endpoint_profile(
        &mut self,
        provider_id: &'static str,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        if self
            .endpoint_profiles(provider_id)
            .active
            .as_deref()
            .is_some_and(|active| active == profile_id)
        {
            return;
        }
        let name = self
            .endpoint_profiles(provider_id)
            .find(&profile_id)
            .map(profile_label)
            .unwrap_or_default();
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            // A different profile's key must not stay revealed, and the
            // latencies belong to the profile that was measured.
            form.key_revealed = false;
            form.renaming = false;
            form.speed = None;
        }
        self.commit_profiles(
            provider_id,
            true,
            Some(tr!("cli_setup.profile_switched", name = name)),
            move |slot| {
                slot.set_active(&profile_id);
            },
            cx,
        );
    }

    /// Add an empty profile and switch to it, so the fields the user is
    /// about to fill belong to the new entry.
    fn add_endpoint_profile(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let name = tr!(
            "cli_setup.profile_new_name",
            n = self.endpoint_profiles(provider_id).profiles.len() + 1
        );
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.key_revealed = false;
            form.renaming = false;
            form.speed = None;
        }
        self.commit_profiles(provider_id, true, None, move |slot| {
            slot.add(&name);
        }, cx);
    }

    /// Copy the active profile — the usual way to try a different domain or
    /// key without losing the working one.
    fn duplicate_endpoint_profile(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let profiles = self.endpoint_profiles(provider_id);
        let Some(active) = profiles.active_profile() else {
            return;
        };
        let id = active.id.clone();
        let name = format!("{}{}", profile_label(active), tr!("cli_setup.profile_copy_suffix"));
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.key_revealed = false;
            form.renaming = false;
            form.speed = None;
        }
        self.commit_profiles(provider_id, true, None, move |slot| {
            slot.duplicate(&id, &name);
        }, cx);
    }

    /// Ask, then delete the active profile. The next one takes over; the
    /// last one leaving restores the CLI's own configuration.
    fn confirm_delete_endpoint_profile(
        &mut self,
        provider_id: &'static str,
        cx: &mut Context<Self>,
    ) {
        let profiles = self.endpoint_profiles(provider_id);
        let Some(active) = profiles.active_profile() else {
            return;
        };
        let id = active.id.clone();
        let name = profile_label(active);
        let last = profiles.profiles.len() == 1;
        let mut detail = active.endpoint.base_url.clone();
        if last {
            let note = tr!("cli_setup.custom_clear_confirm_detail");
            detail = if detail.is_empty() {
                note
            } else {
                format!("{detail}\n{note}")
            };
        }
        self.request_confirm(
            tr!("cli_setup.profile_delete_confirm", name = name),
            (!detail.is_empty()).then_some(detail),
            tr!("cli_setup.profile_delete"),
            true,
            cx,
            move |this, _, cx| {
                {
                    let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                    form.key_revealed = false;
                    form.renaming = false;
                    form.speed = None;
                }
                this.commit_profiles(provider_id, true, None, move |slot| {
                    slot.remove(&id);
                }, cx);
            },
        );
    }

    /// Build (once) the field used for renaming a profile.
    fn ensure_profile_name_input(
        &mut self,
        provider_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .and_then(|form| form.profile_name_input.clone())
        {
            return input;
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .select_all_on_focus_click()
                .placeholder(tr!("cli_setup.profile_name_placeholder"))
        });
        cx.subscribe(
            &input,
            move |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.commit_profile_name(provider_id, cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.cli_setup
            .page
            .forms
            .entry(provider_id)
            .or_default()
            .profile_name_input = Some(input.clone());
        input
    }

    /// Swap the profile picker for a name field, seeded with the current
    /// name and focused.
    fn begin_rename_profile(
        &mut self,
        provider_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profiles = self.endpoint_profiles(provider_id);
        let Some(active) = profiles.active_profile() else {
            return;
        };
        let name = profile_label(active);
        let input = self.ensure_profile_name_input(provider_id, window, cx);
        input.update(cx, |input, cx| input.set_content(name, cx));
        let focus = input.read(cx).focus();
        window.focus(&focus, cx);
        self.cli_setup.page.forms.entry(provider_id).or_default().renaming = true;
        cx.notify();
    }

    /// Save the typed name, or leave the profile alone when it is blank.
    fn commit_profile_name(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let name = self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .and_then(|form| form.profile_name_input.as_ref())
            .map(|input| input.read(cx).content().trim().to_owned())
            .unwrap_or_default();
        let profiles = self.endpoint_profiles(provider_id);
        let Some(id) = profiles.active.clone() else {
            return;
        };
        self.cli_setup.page.forms.entry(provider_id).or_default().renaming = false;
        if name.is_empty() {
            cx.notify();
            return;
        }
        self.commit_profiles(provider_id, false, None, move |slot| {
            slot.rename(&id, &name);
        }, cx);
    }

    /// Build (once) the field used for adding an alternate domain.
    fn ensure_candidate_input(
        &mut self,
        provider_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .and_then(|form| form.candidate_input.clone())
        {
            return input;
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .select_all_on_focus_click()
                .placeholder(tr!("cli_setup.candidate_placeholder"))
        });
        cx.subscribe(
            &input,
            move |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.add_candidate_url(provider_id, cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.cli_setup
            .page
            .forms
            .entry(provider_id)
            .or_default()
            .candidate_input = Some(input.clone());
        input
    }

    /// Open the alternate-domain block and put the cursor in its field.
    fn open_candidates(
        &mut self,
        provider_id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.ensure_candidate_input(provider_id, window, cx);
        let focus = input.read(cx).focus();
        window.focus(&focus, cx);
        self.cli_setup
            .page
            .forms
            .entry(provider_id)
            .or_default()
            .candidates_open = true;
        cx.notify();
    }

    /// Add the typed origin to the active profile's candidates.
    fn add_candidate_url(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let Some(input) = self
            .cli_setup
            .page
            .forms
            .get(provider_id)
            .and_then(|form| form.candidate_input.clone())
        else {
            return;
        };
        let raw = input.read(cx).content().trim().to_owned();
        if raw.is_empty() {
            return;
        }
        let url = match sub2api::custom_api::normalize_base_url(&raw) {
            Ok(url) => url,
            Err(error) => {
                self.cli_setup.page.forms.entry(provider_id).or_default().error =
                    Some(url_error_label(&error));
                cx.notify();
                return;
            }
        };
        let profiles = self.endpoint_profiles(provider_id);
        if profiles
            .active_profile()
            .is_some_and(|profile| profile.candidate_urls.iter().any(|known| *known == url))
        {
            self.cli_setup.page.forms.entry(provider_id).or_default().error =
                Some(tr!("cli_setup.candidate_duplicate"));
            cx.notify();
            return;
        }
        input.update(cx, |input, cx| input.clear(cx));
        // A profile whose URL was never set adopts the first domain added,
        // so the list and the routed endpoint cannot disagree.
        let adopt = profiles
            .active_profile()
            .is_none_or(|profile| profile.endpoint.base_url.trim().is_empty());
        if adopt {
            self.select_candidate_url(provider_id, url, cx);
            return;
        }
        self.commit_profiles(provider_id, false, None, move |slot| {
            if let Some(profile) = slot.active_profile_mut() {
                profile.add_candidate(&url);
            }
        }, cx);
    }

    /// Drop an alternate domain. Removing the one in use promotes the next.
    fn remove_candidate_url(
        &mut self,
        provider_id: &'static str,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let in_use = self
            .endpoint_profiles(provider_id)
            .active_profile()
            .is_some_and(|profile| profile.endpoint.base_url == url);
        self.cli_setup.page.forms.entry(provider_id).or_default().speed = None;
        self.commit_profiles(provider_id, in_use, None, move |slot| {
            if let Some(profile) = slot.active_profile_mut() {
                profile.remove_candidate(&url);
            }
        }, cx);
    }

    /// Route the active profile through one of its candidates.
    fn select_candidate_url(
        &mut self,
        provider_id: &'static str,
        url: String,
        cx: &mut Context<Self>,
    ) {
        self.commit_profiles(provider_id, true, None, move |slot| {
            if let Some(profile) = slot.active_profile_mut() {
                profile.select_url(&url);
            }
        }, cx);
    }

    fn set_profile_auto_select(
        &mut self,
        provider_id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        self.commit_profiles(provider_id, false, None, move |slot| {
            if let Some(profile) = slot.active_profile_mut() {
                profile.auto_select = on;
            }
        }, cx);
    }

    /// Measure every candidate at once and, when the profile asks for it,
    /// route through the fastest that answered.
    fn run_candidate_speed_test(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
        let profiles = self.endpoint_profiles(provider_id);
        let (mut urls, stored_key, auto_select, current) = match profiles.active_profile() {
            Some(profile) => (
                profile.candidate_urls.clone(),
                profile.endpoint.api_key.clone(),
                profile.auto_select,
                profile.endpoint.base_url.clone(),
            ),
            None => (Vec::new(), String::new(), false, String::new()),
        };
        // Whatever is typed counts too, so a domain can be measured before
        // it is saved.
        let draft = self.endpoint_draft(provider_id, cx);
        let api_key = draft
            .as_ref()
            .map(|(_, key, _)| key.clone())
            .filter(|key| !key.is_empty())
            .unwrap_or(stored_key);
        if let Some((raw_url, ..)) = &draft
            && let Ok(url) = sub2api::custom_api::normalize_base_url(raw_url)
            && !urls.contains(&url)
        {
            urls.push(url);
        }
        if urls.is_empty() {
            return;
        }

        self.cli_setup.page.speed_generation += 1;
        let generation = self.cli_setup.page.speed_generation;
        {
            let form = self.cli_setup.page.forms.entry(provider_id).or_default();
            form.error = None;
            form.candidates_open = true;
            form.speed = Some(SpeedTest {
                running: true,
                results: Vec::new(),
                generation,
            });
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    sub2api::speedtest::test_candidates(
                        provider_id,
                        &urls,
                        &api_key,
                        sub2api::speedtest::DEFAULT_TIMEOUT_SECS,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                {
                    let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                    if !form
                        .speed
                        .as_ref()
                        .is_some_and(|speed| speed.generation == generation)
                    {
                        return;
                    }
                    form.speed = Some(SpeedTest {
                        running: false,
                        results: results.clone(),
                        generation,
                    });
                }
                cx.notify();
                if !auto_select {
                    return;
                }
                let Some(best) = sub2api::speedtest::fastest_ok(&results) else {
                    this.show_toast(tr!("cli_setup.speed_no_ok"));
                    return;
                };
                let winner = results[best].url.clone();
                if winner == current {
                    return;
                }
                let ms = results[best].latency_ms().unwrap_or_default();
                this.select_candidate_url(provider_id, winner.clone(), cx);
                this.show_toast(tr!("cli_setup.speed_auto_selected", url = winner, ms = ms));
            });
        })
        .detach();
    }

    /// Ask the endpoint which models it serves and put them in the model
    /// field, for the CLIs whose configuration has to list them.
    fn fetch_models_from_endpoint(&mut self, provider_id: &'static str, cx: &mut Context<Self>) {
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
                {
                    let form = this.cli_setup.page.forms.entry(provider_id).or_default();
                    if !form
                        .test
                        .as_ref()
                        .is_some_and(|test| test.generation == generation)
                    {
                        return;
                    }
                    form.test = Some(EndpointTest {
                        running: false,
                        result: Some(result.clone()),
                        generation,
                    });
                }
                let models = sub2api::speedtest::model_ids_from_body(&result.body);
                if models.is_empty() {
                    this.show_toast(tr!("cli_setup.fetch_models_none"));
                } else {
                    if let Some((.., Some(input))) = this
                        .custom_api_inputs
                        .iter()
                        .find(|(id, ..)| *id == provider_id)
                        .map(|(id, url, key, models)| (id, url, key, models.clone()))
                    {
                        let joined = models.join(", ");
                        input.update(cx, |input, cx| input.set_content(joined, cx));
                    }
                    this.show_toast(tr!("cli_setup.fetch_models_done", count = models.len()));
                }
                cx.notify();
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

    /// Which saved endpoint configuration routes this CLI, and the actions
    /// that manage the set. A CLI with one profile still reads as a single
    /// form; the picker only starts to matter once there are several.
    fn render_profile_row(
        &self,
        provider_id: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let profiles = self.endpoint_profiles(provider_id);
        let form = self.cli_setup.page.forms.get(provider_id);
        let saving = form.is_some_and(|form| form.saving);
        let renaming = form.is_some_and(|form| form.renaming);

        if renaming
            && let Some(input) = form.and_then(|form| form.profile_name_input.clone())
        {
            return div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .w_full()
                .max_w(px(520.0))
                .child(
                    TextField::new(
                        SharedString::from(format!("profile-name-{provider_id}")),
                        input,
                    )
                    .flex_1(),
                )
                .child(card_button(
                    theme,
                    SharedString::from(format!("profile-name-save-{provider_id}")),
                    tr!("cli_setup.custom_save"),
                    true,
                    saving,
                    cx,
                    move |this, _, cx| this.commit_profile_name(provider_id, cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from(format!("profile-name-cancel-{provider_id}")),
                    tr!("cli_setup.custom_cancel"),
                    false,
                    false,
                    cx,
                    move |this, _, cx| {
                        this.cli_setup.page.forms.entry(provider_id).or_default().renaming =
                            false;
                        cx.notify();
                    },
                ));
        }

        let has_active = profiles.active_profile().is_some();
        let several = profiles.profiles.len() > 1;
        let current = profiles
            .active_profile()
            .map(profile_label)
            .unwrap_or_else(|| tr!("cli_setup.profile_none"));
        let entries: Vec<(String, String)> = profiles
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile_label(profile)))
            .collect();
        let active_id = profiles.active.clone();

        let trigger = div()
            .id(SharedString::from(format!("profile-trigger-{provider_id}")))
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(27.0))
            .px(px(10.0))
            .min_w(px(150.0))
            .max_w(px(260.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.0))
            .cursor_default()
            .hover(|style| style.bg(theme.overlay))
            .text_size(sp(12.0))
            .text_color(theme.text)
            .child(div().min_w_0().truncate().child(current))
            .child(icon("icons/chevron-down.svg", 12.0, theme.text_secondary));

        let handle = self.menu_handle(format!("profile-menu-{provider_id}"), cx);
        let weak = cx.entity().downgrade();
        let picker = dropdown_menu(
            trigger,
            SharedString::from(format!("profile-menu-{provider_id}")),
            &handle,
            MenuAlign::BelowLeft,
            move |_| {
                entries
                    .iter()
                    .map(|(id, label)| {
                        let selected = active_id.as_deref() == Some(id.as_str());
                        let entry_weak = weak.clone();
                        let id = id.clone();
                        MenuItem::new(label.clone(), move |_, cx| {
                            let id = id.clone();
                            let _ = entry_weak.update(cx, |this, cx| {
                                this.switch_endpoint_profile(provider_id, id, cx);
                            });
                        })
                        .selected(selected)
                    })
                    .collect()
            },
        );

        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .child(field_label(theme, tr!("cli_setup.profile_label")))
            .child(picker)
            .child(card_button(
                theme,
                SharedString::from(format!("profile-new-{provider_id}")),
                tr!("cli_setup.profile_new"),
                false,
                saving,
                cx,
                move |this, _, cx| this.add_endpoint_profile(provider_id, cx),
            ));
        if has_active {
            row = row
                .child(card_button(
                    theme,
                    SharedString::from(format!("profile-duplicate-{provider_id}")),
                    tr!("cli_setup.profile_duplicate"),
                    false,
                    saving,
                    cx,
                    move |this, _, cx| this.duplicate_endpoint_profile(provider_id, cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from(format!("profile-rename-{provider_id}")),
                    tr!("cli_setup.profile_rename"),
                    false,
                    saving,
                    cx,
                    move |this, window, cx| this.begin_rename_profile(provider_id, window, cx),
                ));
        }
        // Deleting the last profile is the same act as clearing the
        // endpoint, which the Clear button below already offers.
        if several {
            row = row.child(card_button(
                theme,
                SharedString::from(format!("profile-delete-{provider_id}")),
                tr!("cli_setup.profile_delete"),
                false,
                saving,
                cx,
                move |this, _, cx| this.confirm_delete_endpoint_profile(provider_id, cx),
            ));
        }
        row
    }

    /// Alternate origins for the active profile, their measured latencies,
    /// and the controls that manage and re-measure them.
    fn render_candidates_block(
        &self,
        provider_id: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let profiles = self.endpoint_profiles(provider_id);
        let form = self.cli_setup.page.forms.get(provider_id);
        let saving = form.is_some_and(|form| form.saving);
        let (candidates, active_url, auto_select) = match profiles.active_profile() {
            Some(profile) => (
                profile.candidate_urls.clone(),
                profile.endpoint.base_url.clone(),
                profile.auto_select,
            ),
            None => (Vec::new(), String::new(), false),
        };
        let speed = form.and_then(|form| form.speed.as_ref());
        let testing = speed.is_some_and(|speed| speed.running);
        let open = form.is_some_and(|form| form.candidates_open) || candidates.len() > 1;

        if !open {
            return div().child(card_button(
                theme,
                SharedString::from(format!("candidates-open-{provider_id}")),
                tr!("cli_setup.candidates_open"),
                false,
                false,
                cx,
                move |this, window, cx| this.open_candidates(provider_id, window, cx),
            ));
        }

        let results = speed.map(|speed| speed.results.clone()).unwrap_or_default();
        let outcome = |url: &str| {
            results
                .iter()
                .find(|candidate| candidate.url == url)
                .cloned()
        };
        // After a measurement the fastest belongs at the top; before one,
        // the list keeps the order the user built.
        let mut ordered: Vec<String> = candidates.clone();
        if !results.is_empty() {
            ordered.sort_by_key(|url| match outcome(url) {
                Some(candidate) => match candidate.latency_ms() {
                    Some(ms) => (0u8, ms),
                    None => (1, 0),
                },
                None => (2, 0),
            });
        }

        let mut block = div()
            .mt(px(4.0))
            .w_full()
            .max_w(px(520.0))
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(field_label(theme, tr!("cli_setup.candidates_title")))
            .child(
                div()
                    .text_size(sp(11.5))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(tr!("cli_setup.candidates_detail")),
            );

        for (index, url) in ordered.iter().enumerate() {
            let candidate = outcome(url);
            let in_use = *url == active_url;
            let (status_text, status_color) = match &candidate {
                Some(result) if result.invalid.is_some() => {
                    (tr!("cli_setup.speed_invalid_url"), theme.danger)
                }
                Some(result) => match result.result.as_ref() {
                    Some(probe) => match probe.verdict {
                        sub2api::custom_api::ProbeVerdict::Ok => (
                            tr!("cli_setup.candidate_latency", ms = probe.latency_ms),
                            latency_color(theme, probe.latency_ms),
                        ),
                        sub2api::custom_api::ProbeVerdict::Unauthorized => (
                            tr!("cli_setup.candidate_unauthorized"),
                            theme.warning,
                        ),
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
                        .text_color(if in_use {
                            theme.text
                        } else {
                            theme.text_secondary
                        })
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
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_size(sp(11.5))
                        .text_color(theme.success)
                        .child(icon("icons/check.svg", 11.0, theme.success))
                        .child(tr!("cli_setup.candidate_active")),
                );
            } else {
                let pick = url.clone();
                row = row.child(card_button(
                    theme,
                    SharedString::from(format!("candidate-use-{provider_id}-{index}")),
                    tr!("cli_setup.candidate_use"),
                    false,
                    saving,
                    cx,
                    move |this, _, cx| this.select_candidate_url(provider_id, pick.clone(), cx),
                ));
            }
            let drop = url.clone();
            row = row.child(card_button(
                theme,
                SharedString::from(format!("candidate-remove-{provider_id}-{index}")),
                tr!("cli_setup.candidate_remove"),
                false,
                saving,
                cx,
                move |this, _, cx| this.remove_candidate_url(provider_id, drop.clone(), cx),
            ));
            block = block.child(row);
        }

        if let Some(input) = form.and_then(|form| form.candidate_input.clone()) {
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        TextField::new(
                            SharedString::from(format!("candidate-input-{provider_id}")),
                            input,
                        )
                        .flex_1(),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("candidate-add-{provider_id}")),
                        tr!("cli_setup.candidate_add"),
                        false,
                        saving,
                        cx,
                        move |this, _, cx| this.add_candidate_url(provider_id, cx),
                    )),
            );
        } else {
            block = block.child(div().flex().child(card_button(
                theme,
                SharedString::from(format!("candidate-add-open-{provider_id}")),
                tr!("cli_setup.candidate_add"),
                false,
                false,
                cx,
                move |this, window, cx| this.open_candidates(provider_id, window, cx),
            )));
        }

        block.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .child(card_button(
                    theme,
                    SharedString::from(format!("candidate-test-{provider_id}")),
                    if testing {
                        tr!("cli_setup.speed_testing")
                    } else {
                        tr!("cli_setup.speed_test_all")
                    },
                    false,
                    testing || saving,
                    cx,
                    move |this, _, cx| this.run_candidate_speed_test(provider_id, cx),
                ))
                .child(toggle_switch(
                    SharedString::from(format!("candidate-auto-{provider_id}")),
                    auto_select,
                    saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        this.set_profile_auto_select(provider_id, !auto_select, cx);
                    },
                ))
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("cli_setup.speed_auto_select")),
                ),
        )
    }

    fn render_endpoint_form(
        &self,
        kind: ProviderKind,
        provider_id: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        self.render_endpoint_form_titled(
            kind,
            provider_id,
            tr!("cli_setup.custom_title"),
            Some(tr!("cli_setup.custom_detail")),
            theme,
            cx,
        )
    }

    /// [`Self::render_endpoint_form`] under a caller-chosen heading, for the
    /// built-in agent — where one card holds three of these and the shared
    /// "custom endpoint" wording belongs to the group rather than to each.
    pub(super) fn render_endpoint_form_titled(
        &self,
        kind: ProviderKind,
        provider_id: &'static str,
        title: String,
        detail: Option<String>,
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

        let hint = if sub2api::custom_api::uses_anthropic_shape(provider_id) {
            tr!("cli_setup.custom_hint_anthropic")
        } else {
            tr!("cli_setup.custom_hint_openai")
        };

        // The key stays masked until asked for, entry included — a stub
        // would have hidden a stored key but left a new one in cleartext
        // while it was being typed, which is when a screen is most likely
        // being shared.
        let key_row: Div = div()
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
            .when(!key_content.is_empty(), |row| {
                row.child(card_button(
                    theme,
                    SharedString::from(format!("custom-api-reveal-{provider_id}")),
                    if key_revealed {
                        tr!("cli_setup.custom_hide")
                    } else {
                        tr!("cli_setup.custom_reveal")
                    },
                    false,
                    false,
                    cx,
                    move |this, _, cx| {
                        let form =
                            this.cli_setup.page.forms.entry(provider_id).or_default();
                        form.key_revealed = !form.key_revealed;
                        let revealed = form.key_revealed;
                        if let Some((_, key, _)) = this.endpoint_inputs(provider_id) {
                            key.update(cx, |input, cx| input.set_masked(!revealed, cx));
                        }
                        cx.notify();
                    },
                ))
            });

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
                    .child(title),
            )
            .children(detail.map(|detail| {
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(16.0))
                    .text_color(theme.text_ghost)
                    .child(detail)
            }))
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.warning)
                    .child(hint),
            )
            .child(self.render_profile_row(provider_id, theme, cx))
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

        section = section.child(self.render_candidates_block(provider_id, theme, cx));

        if let Some(models_input) = models_input {
            section = section
                .child(field_label(theme, tr!("cli_setup.custom_models_label")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .w_full()
                        .max_w(px(520.0))
                        .child(
                            TextField::new(
                                SharedString::from(format!("custom-api-models-{provider_id}")),
                                models_input,
                            )
                            .flex_1(),
                        )
                        .child(card_button(
                            theme,
                            SharedString::from(format!("custom-api-fetch-models-{provider_id}")),
                            tr!("cli_setup.fetch_models"),
                            false,
                            testing,
                            cx,
                            move |this, _, cx| this.fetch_models_from_endpoint(provider_id, cx),
                        )),
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
