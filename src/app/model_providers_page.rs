//! Settings → Model providers: the endpoints, described once.
//!
//! Fork addition. The Providers page is about *CLIs* — is one installed, can
//! it run, which binary. This page is about the *endpoints* those CLIs (and
//! the built-in agent) are pointed at: a list on the left, one form on the
//! right, the same shape ZCode uses and for the same reason — an endpoint is
//! a thing you own, not a property of each program that happens to call it.
//!
//! The store is [`sub2api::providers`]; a slot binds to an entry rather than
//! holding its own copy, so a relay serving three CLIs is edited in one
//! place. What the CLI page still owns is which entry each slot is bound to.
//!
//! Nothing here does I/O on a frame: the registry comes from the same cache
//! the Providers page reads, and every save, test and speed test runs on the
//! background executor.

use sub2api::providers::{ApiFormat, ModelEntry, ProviderEntry};

use crate::ui::ActivationExt as _;

use super::providers_page::{card_button, probe_status_line};
use super::*;

/// Which row of the left column is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProviderSelection {
    /// The managed gateway. One built-in row, not an entry in the registry:
    /// it is configured by signing in, not by typing an address.
    Cloud,
    Custom(String),
}

/// Fields of the detail pane.
///
/// One set, not one per entry: the pane shows exactly one provider, and
/// keeping a set per entry would mean rebuilding them for providers the user
/// may never open. They are re-seeded whenever the selection moves.
#[derive(Default)]
pub(super) struct ProviderDetailInputs {
    pub name: Option<Entity<TextInput>>,
    pub url: Option<Entity<TextInput>>,
    pub key: Option<Entity<TextInput>>,
    /// The "add a model" field.
    pub model: Option<Entity<TextInput>>,
    /// The "add an alternate domain" field.
    pub candidate: Option<Entity<TextInput>>,
    /// Which entry the fields currently hold, so a re-render never re-seeds
    /// over something half-typed.
    pub loaded: Option<String>,
}

#[derive(Default)]
pub(super) struct ModelProvidersPageState {
    pub selected: Option<ProviderSelection>,
    pub inputs: ProviderDetailInputs,
    /// A validation failure on the last save, shown in the pane. A toast
    /// would vanish before it could be read.
    pub error: Option<String>,
    /// What `reconcile` warned about on the last save.
    pub warning: Option<String>,
    pub saving: bool,
    pub test: Option<super::providers_page::EndpointTest>,
    pub speed: Option<super::providers_page::SpeedTest>,
    test_generation: u64,
    speed_generation: u64,
}

/// How long each candidate gets in a speed test. Short on purpose: this
/// measures reachability from here, not how fast the model answers.
const SPEEDTEST_TIMEOUT_SECS: u32 = 10;

/// The label for a wire format, which is what the user picks by.
fn format_label(format: ApiFormat) -> String {
    match format {
        ApiFormat::Anthropic => tr!("model_providers.format_anthropic"),
        ApiFormat::OpenAiResponses => tr!("model_providers.format_responses"),
        ApiFormat::OpenAiChat => tr!("model_providers.format_chat"),
    }
}

/// Format and path together, because the path is what a third-party server
/// either implements or does not.
fn format_label_with_path(format: ApiFormat) -> String {
    format!("{} ({})", format_label(format), format.request_path())
}

/// A context window as a badge: `1M`, `200K`, `64K`.
fn context_badge(tokens: u32) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// What to call an entry the user never named.
fn entry_label(entry: &ProviderEntry) -> String {
    let name = entry.name.trim();
    if name.is_empty() {
        tr!("model_providers.unnamed")
    } else {
        name.to_owned()
    }
}

fn field_label(theme: Theme, text: String) -> Div {
    div()
        .text_size(sp(11.5))
        .text_color(theme.text_tertiary)
        .child(text)
}

impl Waku {
    /// The registry as last read. Same cache the Providers page uses, so the
    /// two pages never disagree about what is stored.
    fn provider_registry(&self) -> sub2api::providers::ProviderRegistry {
        self.custom_api_snapshot().registry.clone()
    }

    /// The entry the pane is showing, if a custom one is selected.
    fn selected_provider(&self) -> Option<ProviderEntry> {
        let ProviderSelection::Custom(id) = self.model_providers.selected.as_ref()? else {
            return None;
        };
        self.provider_registry().get(id).cloned()
    }

    // --- the page -----------------------------------------------------------

    pub(super) fn render_model_providers_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let registry = self.provider_registry();

        // The gateway when nothing else is open, including when the open
        // entry has just been deleted. Never *auto*-selecting a custom entry
        // matters: its pane is only usable once `open_provider` has seeded
        // the fields, and that needs a Window, which render does not have.
        let selected = match self.model_providers.selected.clone() {
            Some(ProviderSelection::Custom(id)) if registry.get(&id).is_some() => {
                ProviderSelection::Custom(id)
            }
            _ => ProviderSelection::Cloud,
        };
        let detail: AnyElement = match &selected {
            ProviderSelection::Cloud => self.render_cloud_route_detail(theme, cx).into_any_element(),
            ProviderSelection::Custom(id) => match registry.get(id) {
                Some(entry) => self
                    .render_provider_detail(entry, theme, cx)
                    .into_any_element(),
                None => div().into_any_element(),
            },
        };

        div()
            .size_full()
            .min_h_0()
            .flex()
            .child(self.render_provider_list_column(&registry, &selected, theme, cx))
            .child(
                div()
                    .id("model-providers-detail-scroll")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_provider_list_column(
        &self,
        registry: &sub2api::providers::ProviderRegistry,
        selected: &ProviderSelection,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let signed_in = self.cloud_account.credentials.is_some();
        let mut list = div()
            .id("model-providers-list-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(8.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(2.0));

        // The gateway is always listed, signed in or not: its row is where
        // you learn it exists and how to turn it on.
        list = list.child(self.render_provider_row(
            SharedString::from("model-provider-cloud"),
            tr!("model_providers.cloud_route"),
            if signed_in && self.cloud_account.routing_enabled {
                Some(theme.success)
            } else {
                None
            },
            Some(tr!("model_providers.cloud_caption")),
            *selected == ProviderSelection::Cloud,
            theme,
            cx,
            move |this, _, cx| {
                this.model_providers.selected = Some(ProviderSelection::Cloud);
                cx.notify();
            },
        ));

        if !registry.is_empty() {
            list = list.child(
                div()
                    .mt(px(12.0))
                    .mb(px(4.0))
                    .px(px(10.0))
                    .text_size(sp(10.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("model_providers.custom_group")),
            );
        }
        for entry in &registry.providers {
            let id = entry.id.clone();
            let is_selected = matches!(selected, ProviderSelection::Custom(open) if open == &id);
            let dot = if !entry.enabled {
                None
            } else if entry.is_usable() {
                Some(theme.success)
            } else {
                Some(theme.warning)
            };
            let click_id = id.clone();
            list = list.child(self.render_provider_row(
                SharedString::from(format!("model-provider-{id}")),
                entry_label(entry),
                dot,
                Some(format_label(entry.format)),
                is_selected,
                theme,
                cx,
                move |this, window, cx| {
                    this.open_provider(click_id.clone(), window, cx);
                },
            ));
        }

        div()
            .w(px(240.0))
            .flex_none()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(list)
            .child(
                div()
                    .flex_none()
                    .px(px(12.0))
                    .py(px(10.0))
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(card_button(
                        theme,
                        SharedString::from("model-provider-add"),
                        tr!("model_providers.add"),
                        false,
                        false,
                        cx,
                        |this, window, cx| this.add_provider(window, cx),
                    )),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_provider_row(
        &self,
        id: SharedString,
        label: String,
        dot: Option<gpui::Hsla>,
        caption: Option<String>,
        selected: bool,
        theme: Theme,
        cx: &mut Context<Self>,
        activate: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .cursor_default()
            .when(selected, |row| row.bg(theme.surface))
            .hover(|style| style.bg(theme.overlay))
            .child(
                // No light at all when the entry is switched off: a grey
                // light would read as a state it is in, rather than as off.
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .flex_none()
                    .rounded_full()
                    .bg(dot.unwrap_or_else(gpui::transparent_black)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(sp(12.5))
                            .text_color(if selected { theme.text } else { theme.text_secondary })
                            .child(label),
                    )
                    .children(caption.map(|caption| {
                        div()
                            .truncate()
                            .text_size(sp(10.5))
                            .text_color(theme.text_tertiary)
                            .child(caption)
                    })),
            )
            .on_activation(cx, activate)
    }

    /// The gateway's row: what it is, and where to configure it.
    ///
    /// Read-only on purpose — signing in, choosing a group and picking an
    /// origin all live on the Cloud Account page, and a second set of
    /// controls here would be a second place for them to disagree.
    fn render_cloud_route_detail(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let signed_in = self.cloud_account.credentials.is_some();
        let origin = self
            .cloud_account
            .gateway_origin
            .origin()
            .unwrap_or_else(|| sub2api::brand::MANAGED_SERVICE_URL.to_owned());

        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(detail_field(
                theme,
                tr!("model_providers.cloud_origin"),
                origin,
            ))
            .child(detail_field(
                theme,
                tr!("model_providers.cloud_status"),
                if !signed_in {
                    tr!("model_providers.cloud_signed_out")
                } else if self.cloud_account.routing_enabled {
                    tr!("model_providers.cloud_on")
                } else {
                    tr!("model_providers.cloud_off")
                },
            ));
        body = body.child(
            div().pt(px(4.0)).child(card_button(
                theme,
                SharedString::from("model-provider-open-cloud"),
                tr!("model_providers.cloud_open"),
                false,
                false,
                cx,
                |this, _, cx| {
                    this.settings_page = Some(SettingsPage::CloudAccount);
                    cx.notify();
                },
            )),
        );

        detail_pane(theme)
            .child(detail_heading(
                theme,
                tr!("model_providers.cloud_route"),
                tr!("model_providers.cloud_detail"),
            ))
            .child(body)
    }

    // --- the detail pane ----------------------------------------------------

    fn render_provider_detail(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = entry.id.clone();
        let state = &self.model_providers;
        let ready = state.inputs.loaded.as_deref() == Some(id.as_str());

        let mut pane = detail_pane(theme).child(self.render_provider_header(entry, theme, cx));

        if !ready {
            // The fields are built and seeded when a row is opened, which
            // needs a Window. One frame at most.
            return pane;
        }

        let key_revealed = state
            .inputs
            .key
            .as_ref()
            .is_some_and(|input| !input.read(cx).is_masked());

        let mut form = div().flex().flex_col().gap(px(12.0));
        form = form.child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(field_label(theme, tr!("model_providers.format")))
                .child(self.render_format_picker(entry, theme, cx)),
        );
        if let Some(url) = state.inputs.url.clone() {
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(field_label(theme, tr!("model_providers.base_url")))
                    .child(TextField::new(SharedString::from("model-provider-url"), url))
                    .child(
                        div()
                            .text_size(sp(10.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!(
                                "model_providers.path_hint",
                                path = entry.format.request_path()
                            )),
                    ),
            );
        }
        if let Some(key) = state.inputs.key.clone() {
            let has_key = !key.read(cx).content().is_empty();
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(field_label(theme, tr!("model_providers.api_key")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                TextField::new(SharedString::from("model-provider-key"), key)
                                    .flex_1(),
                            )
                            .when(has_key, |row| {
                                row.child(card_button(
                                    theme,
                                    SharedString::from("model-provider-key-toggle"),
                                    if key_revealed {
                                        tr!("cli_setup.custom_hide")
                                    } else {
                                        tr!("cli_setup.custom_reveal")
                                    },
                                    false,
                                    false,
                                    cx,
                                    |this, _, cx| this.toggle_provider_key_mask(cx),
                                ))
                            }),
                    ),
            );
        }

        pane = pane
            .child(form)
            .child(self.render_provider_candidates(entry, theme, cx))
            .child(self.render_provider_models(entry, theme, cx));

        if let Some(error) = &self.model_providers.error {
            pane = pane.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }
        if let Some(warning) = &self.model_providers.warning {
            pane = pane.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.warning)
                    .child(warning.clone()),
            );
        }
        if let Some(test) = &self.model_providers.test {
            pane = pane.child(probe_status_line(theme, test));
        }

        pane.child(self.render_provider_actions(entry, theme, cx))
    }

    fn render_provider_header(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = entry.id.clone();
        let enabled = entry.enabled;
        let name_field: AnyElement = match self.model_providers.inputs.name.clone() {
            Some(name) if self.model_providers.inputs.loaded.as_deref() == Some(id.as_str()) => {
                TextField::new(SharedString::from("model-provider-name"), name)
                    .flex_1()
                    .into_any_element()
            }
            _ => div()
                .flex_1()
                .text_size(sp(15.0))
                .font_weight(FontWeight::MEDIUM)
                .child(entry_label(entry))
                .into_any_element(),
        };
        let toggle_id = id.clone();
        let delete_id = id.clone();

        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(name_field)
            .child(toggle_switch(
                SharedString::from("model-provider-enabled"),
                enabled,
                self.model_providers.saving,
                theme,
                cx,
                move |this, _, cx| this.toggle_provider_enabled(toggle_id.clone(), cx),
            ))
            .child(card_button(
                theme,
                SharedString::from("model-provider-delete"),
                tr!("model_providers.delete"),
                false,
                self.model_providers.saving,
                cx,
                move |this, _, cx| this.confirm_delete_provider(delete_id.clone(), cx),
            ))
    }

    fn render_format_picker(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id.clone();
        let current = entry.format;
        let trigger = div()
            .id(SharedString::from("model-provider-format"))
            .tab_index(0)
            .focus_visible(|style| style.border_color(theme.accent))
            .h(px(30.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .cursor_default()
            .text_size(sp(12.0))
            .text_color(theme.text)
            .child(format_label_with_path(current))
            .child(icon("icons/chevron-down.svg", 12.0, theme.text_secondary));

        let handle = self.menu_handle("model-provider-format-menu", cx);
        let weak = cx.entity().downgrade();
        dropdown_menu(
            trigger,
            SharedString::from("model-provider-format-menu"),
            &handle,
            MenuAlign::BelowLeft,
            move |_| {
                ApiFormat::ALL
                    .into_iter()
                    .map(|format| {
                        let weak = weak.clone();
                        let id = id.clone();
                        MenuItem::new(
                            SharedString::from(format_label_with_path(format)),
                            move |_, cx| {
                                let id = id.clone();
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_provider_format(id, format, cx);
                                });
                            },
                        )
                        .selected(format == current)
                    })
                    .collect()
            },
        )
    }

    /// The models the user says this endpoint serves.
    ///
    /// A list rather than a comma-separated box, because each line now
    /// carries more than a name: nothing lists the models behind somebody
    /// else's address, so what the picker knows about them is whatever is
    /// typed here.
    fn render_provider_models(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = entry.id.clone();
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(field_label(theme, tr!("model_providers.models")));

        if entry.models.is_empty() {
            section = section.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("model_providers.models_empty")),
            );
        }
        for model in &entry.models {
            let model_id = model.id.clone();
            let entry_id = id.clone();
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .bg(theme.inset)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(12.0))
                            .text_color(theme.text)
                            .child(model.display_name().to_owned()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(5.0))
                            .bg(theme.overlay)
                            .text_size(sp(10.0))
                            .text_color(theme.text_tertiary)
                            .child(context_badge(model.context_window_or_default())),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("model-remove-{entry_id}-{model_id}")),
                        tr!("model_providers.model_remove"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| {
                            this.remove_provider_model(entry_id.clone(), model_id.clone(), cx)
                        },
                    )),
            );
        }
        if let Some(input) = self.model_providers.inputs.model.clone() {
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        TextField::new(SharedString::from("model-provider-model-add"), input)
                            .flex_1(),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from("model-provider-model-add-button"),
                        tr!("model_providers.model_add"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.add_provider_model(cx),
                    ))
                    .child(card_button(
                        theme,
                        SharedString::from("model-provider-model-fetch"),
                        tr!("cli_setup.fetch_models"),
                        false,
                        self.model_providers.saving,
                        cx,
                        |this, _, cx| this.fetch_provider_models(cx),
                    )),
            );
        }
        section
    }

    fn render_provider_actions(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let saving = self.model_providers.saving;
        let testing = self
            .model_providers
            .test
            .as_ref()
            .is_some_and(|test| test.running);
        let id = entry.id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pt(px(4.0))
            .child(card_button(
                theme,
                SharedString::from("model-provider-save"),
                tr!("cli_setup.custom_save"),
                true,
                saving,
                cx,
                |this, _, cx| this.save_provider_form(cx),
            ))
            .child(card_button(
                theme,
                SharedString::from("model-provider-discard"),
                tr!("cli_setup.custom_cancel"),
                false,
                saving,
                cx,
                move |this, window, cx| this.open_provider(id.clone(), window, cx),
            ))
            .child(card_button(
                theme,
                SharedString::from("model-provider-test"),
                tr!("cli_setup.custom_test"),
                false,
                saving || testing,
                cx,
                |this, _, cx| this.test_provider_form(cx),
            ))
    }
}

/// The pane's outer padding and column, shared by both kinds of detail.
fn detail_pane(theme: Theme) -> Div {
    let _ = theme;
    div()
        .px(px(20.0))
        .py(px(18.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
}

fn detail_heading(theme: Theme, title: String, detail: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(sp(15.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text)
                .child(title),
        )
        .child(
            div()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .child(detail),
        )
}

fn detail_field(theme: Theme, label: String, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .child(field_label(theme, label))
        .child(
            div()
                .text_size(sp(12.5))
                .text_color(theme.text)
                .child(value),
        )
}

// --- actions ---------------------------------------------------------------

impl Waku {
    /// Build the detail pane's fields, once.
    ///
    /// Creating a `TextInput` needs a `Window`, which render does not have,
    /// so this runs from the handler that opens a row.
    fn ensure_provider_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_providers.inputs.url.is_some() {
            return;
        }
        let name = cx.new(|cx| {
            TextInput::new(window, cx)
                .select_all_on_focus_click()
                .placeholder(tr!("model_providers.name_placeholder"))
        });
        let url = cx.new(|cx| {
            TextInput::new(window, cx).placeholder(tr!("cli_setup.custom_url_placeholder"))
        });
        // Masked from the start, entry included: a key is most likely to be
        // typed while a screen is being shared.
        let key = cx.new(|cx| {
            TextInput::new(window, cx)
                .masked(true)
                .placeholder(tr!("cli_setup.custom_key_placeholder"))
        });
        let model = cx.new(|cx| {
            TextInput::new(window, cx).placeholder(tr!("model_providers.model_placeholder"))
        });
        let candidate = cx.new(|cx| {
            TextInput::new(window, cx).placeholder(tr!("cli_setup.candidate_placeholder"))
        });
        for input in [&name, &url, &key] {
            cx.subscribe(
                input,
                |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Submit(_) => this.save_provider_form(cx),
                    InputEvent::Edited => cx.notify(),
                    _ => {}
                },
            )
            .detach();
        }
        cx.subscribe(
            &model,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.add_provider_model(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        cx.subscribe(
            &candidate,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.add_provider_candidate(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.model_providers.inputs.name = Some(name);
        self.model_providers.inputs.url = Some(url);
        self.model_providers.inputs.key = Some(key);
        self.model_providers.inputs.model = Some(model);
        self.model_providers.inputs.candidate = Some(candidate);
    }

    /// Open a provider in the detail pane, seeding the fields from it.
    ///
    /// This is also Discard: re-seeding from what is stored is exactly what
    /// throwing an edit away means.
    pub(super) fn open_provider(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.provider_registry().get(&id).cloned() else {
            return;
        };
        self.ensure_provider_inputs(window, cx);
        let inputs = (
            self.model_providers.inputs.name.clone(),
            self.model_providers.inputs.url.clone(),
            self.model_providers.inputs.key.clone(),
            self.model_providers.inputs.model.clone(),
        );
        if let Some(name) = inputs.0 {
            name.update(cx, |input, cx| input.set_content(entry.name.clone(), cx));
        }
        if let Some(url) = inputs.1 {
            url.update(cx, |input, cx| input.set_content(entry.base_url.clone(), cx));
        }
        if let Some(key) = inputs.2 {
            // Back behind the mask on every open: revealing was a decision
            // about one key, not a mode.
            key.update(cx, |input, cx| {
                input.set_masked(true, cx);
                input.set_content(entry.api_key.clone(), cx);
            });
        }
        if let Some(model) = inputs.3 {
            model.update(cx, |input, cx| input.clear(cx));
        }
        if let Some(candidate) = self.model_providers.inputs.candidate.clone() {
            candidate.update(cx, |input, cx| input.clear(cx));
        }
        self.model_providers.selected = Some(ProviderSelection::Custom(id.clone()));
        self.model_providers.inputs.loaded = Some(id);
        self.model_providers.error = None;
        self.model_providers.warning = None;
        self.model_providers.test = None;
        // Measurements belong to the endpoint that was measured.
        self.model_providers.speed = None;
        cx.notify();
    }

    fn toggle_provider_key_mask(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.model_providers.inputs.key.clone() else {
            return;
        };
        let masked = key.read(cx).is_masked();
        key.update(cx, |input, cx| input.set_masked(!masked, cx));
        cx.notify();
    }

    /// Add an empty provider and open it.
    fn add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = ProviderEntry::new("", ApiFormat::OpenAiChat);
        let id = entry.id.clone();
        // Into the cache first, so the pane can show it on this frame rather
        // than after the write lands. The write reloads from disk and
        // replaces the cache with the real thing either way.
        let mut config = self.custom_api_snapshot();
        config.registry.providers.push(entry.clone());
        *self.cli_setup.custom_cache.borrow_mut() = Some(config);
        self.open_provider(id, window, cx);
        self.commit_registry(
            None,
            move |registry| {
                registry.add(entry);
            },
            cx,
        );
    }

    fn toggle_provider_enabled(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(entry) = self.provider_registry().get(&id).cloned() else {
            return;
        };
        let enabled = !entry.enabled;
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.enabled = enabled;
                }
            },
            cx,
        );
    }

    fn set_provider_format(&mut self, id: String, format: ApiFormat, cx: &mut Context<Self>) {
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.format = format;
                }
            },
            cx,
        );
    }

    /// Deleting an entry unbinds every slot pointing at it, which is why it
    /// asks first: a CLI can stop being routed by a click three pages away.
    fn confirm_delete_provider(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(entry) = self.provider_registry().get(&id).cloned() else {
            return;
        };
        let bound = self.slots_bound_to(&id);
        let mut detail = entry.base_url.clone();
        if !bound.is_empty() {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(&tr!(
                "model_providers.delete_confirm_bound",
                slots = bound.join(", ")
            ));
        }
        self.request_confirm(
            tr!("model_providers.delete_confirm", name = entry_label(&entry)),
            (!detail.is_empty()).then_some(detail),
            tr!("model_providers.delete"),
            true,
            cx,
            move |this, _, cx| {
                let id = id.clone();
                this.model_providers.selected = None;
                this.model_providers.inputs.loaded = None;
                this.commit_registry(
                    Some(tr!("model_providers.deleted")),
                    move |registry| {
                        registry.remove(&id);
                    },
                    cx,
                );
            },
        );
    }

    /// Which CLI slots route through an entry, by the name the rest of the
    /// app shows.
    fn slots_bound_to(&self, id: &str) -> Vec<String> {
        let stored = self.custom_api_snapshot();
        sub2api::custom_api::CUSTOM_API_PROVIDERS
            .into_iter()
            .filter(|slot| {
                stored
                    .bound_provider(slot)
                    .is_some_and(|entry| entry.id == id)
            })
            .map(slot_display_name)
            .collect()
    }

    fn add_provider_model(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.model_providers.inputs.loaded.clone() else {
            return;
        };
        let Some(input) = self.model_providers.inputs.model.clone() else {
            return;
        };
        let typed = input.read(cx).content().trim().to_owned();
        if typed.is_empty() {
            return;
        }
        input.update(cx, |input, cx| input.clear(cx));
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    // One line may carry several, the way the comma-separated
                    // box it replaces did.
                    for model in typed.split([',', '\u{3001}', ' ', '\n']) {
                        let model = model.trim();
                        if !model.is_empty() {
                            entry.add_model(ModelEntry::new(model));
                        }
                    }
                }
            },
            cx,
        );
    }

    fn remove_provider_model(&mut self, id: String, model_id: String, cx: &mut Context<Self>) {
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.remove_model(&model_id);
                }
            },
            cx,
        );
    }

    /// Validate the pane and write it.
    ///
    /// Both fields filled saves; both empty leaves the entry described but
    /// unusable; one of each is refused, since an address with no key would
    /// route with no credentials.
    fn save_provider_form(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.model_providers.inputs.loaded.clone() else {
            return;
        };
        let (Some(name_input), Some(url_input), Some(key_input)) = (
            self.model_providers.inputs.name.clone(),
            self.model_providers.inputs.url.clone(),
            self.model_providers.inputs.key.clone(),
        ) else {
            return;
        };
        let name = name_input.read(cx).content().trim().to_owned();
        let raw_url = url_input.read(cx).content().trim().to_owned();
        let api_key = key_input.read(cx).content().trim().to_owned();
        if raw_url.is_empty() != api_key.is_empty() {
            self.model_providers.error = Some(tr!("cli_setup.custom_need_both"));
            cx.notify();
            return;
        }
        let base_url = if raw_url.is_empty() {
            String::new()
        } else {
            match sub2api::custom_api::normalize_base_url(&raw_url) {
                Ok(url) => url,
                Err(error) => {
                    self.model_providers.error =
                        Some(super::providers_page::url_error_label(&error));
                    cx.notify();
                    return;
                }
            }
        };
        // Show the normalized URL, so what is saved is what is seen.
        let normalized = base_url.clone();
        url_input.update(cx, |input, cx| input.set_content(normalized, cx));
        self.commit_registry(
            Some(tr!("cli_setup.custom_saved")),
            move |registry| {
                let Some(entry) = registry.get_mut(&id) else {
                    return;
                };
                entry.name = name;
                entry.api_key = api_key;
                if base_url.is_empty() {
                    entry.base_url.clear();
                } else {
                    // Lists it as a candidate too, so a speed test can reach
                    // the address that was just typed.
                    entry.select_url(&base_url);
                }
            },
            cx,
        );
    }

    /// Probe what is typed, in the wire shape this entry's format speaks.
    fn test_provider_form(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_provider() else {
            return;
        };
        let (Some(url_input), Some(key_input)) = (
            self.model_providers.inputs.url.clone(),
            self.model_providers.inputs.key.clone(),
        ) else {
            return;
        };
        let raw_url = url_input.read(cx).content().trim().to_owned();
        let api_key = key_input.read(cx).content().trim().to_owned();
        let base_url = match sub2api::custom_api::normalize_base_url(&raw_url) {
            Ok(url) => url,
            Err(error) => {
                self.model_providers.error = Some(super::providers_page::url_error_label(&error));
                cx.notify();
                return;
            }
        };
        self.model_providers.test_generation += 1;
        let generation = self.model_providers.test_generation;
        self.model_providers.error = None;
        self.model_providers.test = Some(super::providers_page::EndpointTest {
            running: true,
            result: None,
            generation,
        });
        cx.notify();

        // Shaped by the endpoint's own format, not by which CLI happens to
        // use it: an Anthropic server wants `x-api-key` and a version
        // header, and a bearer token gets a 401 that would read as a bad key.
        let format = entry.format;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    sub2api::custom_api::probe_endpoint_for_format(format, &base_url, &api_key)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // A test the user has already moved past must not overwrite
                // the one they are waiting on.
                if this
                    .model_providers
                    .test
                    .as_ref()
                    .is_none_or(|test| test.generation != generation)
                {
                    return;
                }
                this.model_providers.test = Some(super::providers_page::EndpointTest {
                    running: false,
                    result: Some(result),
                    generation,
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Read the registry, change it, write it, and re-route every CLI.
    ///
    /// Always a read-modify-write of the file rather than a write of what the
    /// cache held: another surface may have written since this frame's
    /// snapshot, and the point of one registry is that they do not overwrite
    /// each other.
    fn commit_registry(
        &mut self,
        toast: Option<String>,
        mutate: impl FnOnce(&mut sub2api::providers::ProviderRegistry) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.model_providers.saving {
            return;
        }
        self.model_providers.saving = true;
        self.model_providers.error = None;
        self.model_providers.warning = None;
        let cloud = super::providers_page::cloud_config(self);
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let mut config = sub2api::custom_api::load();
                    mutate(&mut config.registry);
                    config.registry.normalize();
                    sub2api::custom_api::save(&config)?;
                    let desired = sub2api::global_config::desired_routes(cloud.as_ref(), &config);
                    let warnings = sub2api::global_config::reconcile(&desired)?;
                    anyhow::Ok((config, warnings))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.model_providers.saving = false;
                match outcome {
                    Ok((config, warnings)) => {
                        *this.cli_setup.custom_cache.borrow_mut() = Some(config);
                        if !warnings.is_empty() {
                            this.model_providers.warning = Some(warnings.join("\n"));
                        }
                        // The Chat route's models are the picker's Chat
                        // section; nothing else would notice until the next
                        // catalog fetch.
                        this.sync_native_models();
                        if let Some(toast) = toast {
                            this.show_toast(toast);
                        }
                    }
                    Err(error) => this.model_providers.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// What to call a slot in a sentence. The ids are internal; these are the
/// names the rest of the app already shows for the same things.
fn slot_display_name(slot: &str) -> String {
    use crate::model::ProviderKind;
    match slot {
        "native_messages" => tr!("cli_setup.native_route_messages"),
        "native_responses" => tr!("cli_setup.native_route_responses"),
        "native_chat" => tr!("cli_setup.native_route_chat"),
        "claude" => ProviderKind::Claude.display_name().to_owned(),
        "codex" => ProviderKind::Codex.display_name().to_owned(),
        "grok" => ProviderKind::Grok.display_name().to_owned(),
        "opencode" => ProviderKind::OpenCode.display_name().to_owned(),
        "pi" => ProviderKind::Pi.display_name().to_owned(),
        other => other.to_owned(),
    }
}

// --- alternate domains -----------------------------------------------------

impl Waku {
    /// The other origins that serve the same endpoint, and how fast each was.
    ///
    /// A relay often publishes several domains for one service; which is
    /// quickest is a property of where you are sitting, not of the service,
    /// so it has to be measured here rather than chosen once by whoever
    /// wrote the address down.
    fn render_provider_candidates(
        &self,
        entry: &ProviderEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = entry.id.clone();
        let running = self
            .model_providers
            .speed
            .as_ref()
            .is_some_and(|speed| speed.running);
        let measured = self
            .model_providers
            .speed
            .as_ref()
            .filter(|speed| !speed.running)
            .map(|speed| speed.results.clone())
            .unwrap_or_default();

        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(field_label(theme, tr!("model_providers.candidates")))
                    .child(card_button(
                        theme,
                        SharedString::from("model-provider-speedtest"),
                        if running {
                            tr!("cli_setup.speed_testing")
                        } else {
                            tr!("cli_setup.speed_test_all")
                        },
                        false,
                        running || self.model_providers.saving,
                        cx,
                        |this, _, cx| this.run_provider_speed_test(cx),
                    )),
            );

        for url in &entry.candidate_urls {
            let in_use = &entry.base_url == url;
            let latency = measured
                .iter()
                .find(|result| &result.url == url)
                .and_then(|result| result.latency_ms());
            let select_id = id.clone();
            let select_url = url.clone();
            let remove_id = id.clone();
            let remove_url = url.clone();
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .bg(if in_use { theme.surface } else { theme.inset })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(12.0))
                            .text_color(if in_use { theme.text } else { theme.text_secondary })
                            .child(url.clone()),
                    )
                    .children(latency.map(|ms| {
                        div()
                            .flex_none()
                            .text_size(sp(11.0))
                            .text_color(super::providers_page::latency_color(theme, ms))
                            .child(tr!("cli_setup.candidate_latency", ms = ms))
                    }))
                    .when(!in_use, |row| {
                        row.child(card_button(
                            theme,
                            SharedString::from(format!("candidate-use-{remove_id}-{remove_url}")),
                            tr!("cli_setup.candidate_use"),
                            false,
                            false,
                            cx,
                            move |this, _, cx| {
                                let url = select_url.clone();
                                this.select_provider_url(select_id.clone(), url, cx);
                            },
                        ))
                    })
                    .when(entry.candidate_urls.len() > 1, |row| {
                        row.child(card_button(
                            theme,
                            SharedString::from(format!("candidate-drop-{remove_id}-{remove_url}")),
                            tr!("model_providers.model_remove"),
                            false,
                            false,
                            cx,
                            move |this, _, cx| {
                                let url = remove_url.clone();
                                this.remove_provider_url(remove_id.clone(), url, cx);
                            },
                        ))
                    }),
            );
        }

        if let Some(input) = self.model_providers.inputs.candidate.clone() {
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        TextField::new(SharedString::from("model-provider-candidate"), input)
                            .flex_1(),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from("model-provider-candidate-add"),
                        tr!("model_providers.model_add"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.add_provider_candidate(cx),
                    )),
            );
        }

        let auto = entry.auto_select;
        let auto_id = id.clone();
        section.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(toggle_switch(
                    SharedString::from("model-provider-auto-select"),
                    auto,
                    self.model_providers.saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        let id = auto_id.clone();
                        this.commit_registry(
                            None,
                            move |registry| {
                                if let Some(entry) = registry.get_mut(&id) {
                                    entry.auto_select = !auto;
                                }
                            },
                            cx,
                        );
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

    fn select_provider_url(&mut self, id: String, url: String, cx: &mut Context<Self>) {
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.select_url(&url);
                }
            },
            cx,
        );
    }

    fn remove_provider_url(&mut self, id: String, url: String, cx: &mut Context<Self>) {
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.remove_candidate(&url);
                }
            },
            cx,
        );
    }

    fn add_provider_candidate(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.model_providers.inputs.loaded.clone() else {
            return;
        };
        let Some(input) = self.model_providers.inputs.candidate.clone() else {
            return;
        };
        let typed = input.read(cx).content().trim().to_owned();
        if typed.is_empty() {
            return;
        }
        let url = match sub2api::custom_api::normalize_base_url(&typed) {
            Ok(url) => url,
            Err(error) => {
                self.model_providers.error = Some(super::providers_page::url_error_label(&error));
                cx.notify();
                return;
            }
        };
        input.update(cx, |input, cx| input.clear(cx));
        self.commit_registry(
            None,
            move |registry| {
                if let Some(entry) = registry.get_mut(&id) {
                    entry.add_candidate(&url);
                    // The first one entered is also the one in use: an entry
                    // with candidates but no address routes nothing.
                    if entry.base_url.trim().is_empty() {
                        entry.select_url(&url);
                    }
                }
            },
            cx,
        );
    }

    /// Measure every candidate and, when asked to, move onto the fastest.
    fn run_provider_speed_test(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_provider() else {
            return;
        };
        let urls = entry.candidate_urls.clone();
        if urls.is_empty() {
            return;
        }
        let api_key = self
            .model_providers
            .inputs
            .key
            .as_ref()
            .map(|input| input.read(cx).content().trim().to_owned())
            .unwrap_or_default();
        self.model_providers.speed_generation += 1;
        let generation = self.model_providers.speed_generation;
        self.model_providers.error = None;
        self.model_providers.speed = Some(super::providers_page::SpeedTest {
            running: true,
            results: Vec::new(),
            generation,
        });
        cx.notify();

        let format = entry.format;
        let auto_select = entry.auto_select;
        let id = entry.id.clone();
        cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    sub2api::speedtest::test_candidates_for_format(
                        format,
                        &urls,
                        &api_key,
                        SPEEDTEST_TIMEOUT_SECS,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .model_providers
                    .speed
                    .as_ref()
                    .is_none_or(|speed| speed.generation != generation)
                {
                    return;
                }
                let fastest = sub2api::speedtest::fastest_ok(&results)
                    .and_then(|index| results.get(index))
                    .map(|result| result.url.clone());
                this.model_providers.speed = Some(super::providers_page::SpeedTest {
                    running: false,
                    results,
                    generation,
                });
                // Only when asked: silently repointing an endpoint the user
                // typed would be a routing change they did not make.
                if let (true, Some(url)) = (auto_select, fastest) {
                    this.select_provider_url(id, url, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ask the endpoint what it serves, and list what it answers.
    ///
    /// Only an offer: what comes back is added to the list, and anything
    /// already there keeps the context window and tiers it was given.
    fn fetch_provider_models(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.selected_provider() else {
            return;
        };
        let (Some(url_input), Some(key_input)) = (
            self.model_providers.inputs.url.clone(),
            self.model_providers.inputs.key.clone(),
        ) else {
            return;
        };
        let raw_url = url_input.read(cx).content().trim().to_owned();
        let api_key = key_input.read(cx).content().trim().to_owned();
        let base_url = match sub2api::custom_api::normalize_base_url(&raw_url) {
            Ok(url) => url,
            Err(error) => {
                self.model_providers.error = Some(super::providers_page::url_error_label(&error));
                cx.notify();
                return;
            }
        };
        self.model_providers.test_generation += 1;
        let generation = self.model_providers.test_generation;
        self.model_providers.error = None;
        self.model_providers.test = Some(super::providers_page::EndpointTest {
            running: true,
            result: None,
            generation,
        });
        cx.notify();

        let format = entry.format;
        let id = entry.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    sub2api::custom_api::probe_endpoint_for_format(format, &base_url, &api_key)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .model_providers
                    .test
                    .as_ref()
                    .is_none_or(|test| test.generation != generation)
                {
                    return;
                }
                let models = sub2api::speedtest::model_ids_from_body(&result.body);
                this.model_providers.test = Some(super::providers_page::EndpointTest {
                    running: false,
                    result: Some(result),
                    generation,
                });
                if models.is_empty() {
                    this.show_toast(tr!("cli_setup.fetch_models_none"));
                    cx.notify();
                    return;
                }
                let found = models.len();
                this.commit_registry(
                    Some(tr!("cli_setup.fetch_models_done", count = found)),
                    move |registry| {
                        if let Some(entry) = registry.get_mut(&id) {
                            for model in models {
                                entry.add_model(ModelEntry::new(&model));
                            }
                        }
                    },
                    cx,
                );
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window is a number read at a glance, so it is abbreviated — but
    /// never past the point of being wrong about which model is bigger.
    #[test]
    fn the_context_badge_reads_at_a_glance() {
        assert_eq!(context_badge(1_000_000), "1M");
        assert_eq!(context_badge(2_000_000), "2M");
        assert_eq!(context_badge(200_000), "200K");
        assert_eq!(context_badge(64_000), "64K");
        assert_eq!(context_badge(512), "512");
        // Not a round million: shown in thousands rather than as a "1M" that
        // would claim the same size as the real thing.
        assert_eq!(context_badge(1_048_576), "1048K");
    }

    /// Choosing a format is choosing a path, so the menu has to say which
    /// one — that is the fact a third-party server either satisfies or not.
    #[test]
    fn every_format_option_names_its_path() {
        for format in ApiFormat::ALL {
            let label = format_label_with_path(format);
            assert!(label.contains(format.request_path()), "{label}");
        }
    }

    /// The delete confirmation lists what is using an endpoint. Every slot
    /// needs a name for that sentence, or it would print a raw id.
    #[test]
    fn every_slot_has_a_name_to_show() {
        for slot in sub2api::custom_api::CUSTOM_API_PROVIDERS {
            assert_ne!(slot_display_name(slot), slot, "{slot}");
        }
    }
}
