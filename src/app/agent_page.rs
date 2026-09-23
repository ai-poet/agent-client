//! Settings → Agent: the built-in agent's behaviour, tools, MCP servers and
//! permission rules.
//!
//! Fork addition. Every CLI provider keeps its configuration in its own
//! files, and Waku can only reach in through `global_config` to write
//! routing. The built-in agent's configuration is the engine's
//! `settings.json`, which this fork owns end to end — so this is the first
//! provider whose MCP roster, tool set and standing approvals are editable
//! in the app rather than in a text editor.
//!
//! The logic is `sub2api::agent_settings`, GPUI-free and unit-tested; this
//! file is the view. Every change saves immediately on the background
//! executor, the way the other settings pages do, and the last failure stays
//! on screen until the next successful save.
//!
//! Text fields are created in click handlers, which have a `Window`; render
//! does not, so a field exists only once its section has been opened.

use std::time::Instant;

use sub2api::agent_settings::{
    AgentSettings, BUILTIN_TOOLS, ESSENTIAL_TOOLS, McpServer, PermissionRule,
};

use super::providers_page::card_button;
use super::settings::abbreviate_home_path;
use super::*;

use crate::ui::text_field::TextField;

/// Presets for the compaction threshold, as fractions of the window.
const COMPACT_PRESETS: [(f32, &str); 3] = [(0.6, "60%"), (0.8, "80%"), (0.9, "90%")];

/// How long "Saved" stays on the status line.
const SAVED_FLASH: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Default)]
pub(super) struct AgentPageState {
    /// Loaded off-thread on first open; `None` until then.
    pub settings: Option<AgentSettings>,
    pub loading: bool,
    pub saving: bool,
    /// Last save failure, kept until the next successful save.
    pub error: Option<String>,
    pub saved_at: Option<Instant>,
    /// The system-prompt editor, while open.
    pub prompt_input: Option<Entity<TextInput>>,
    /// The add-server form, while open.
    pub mcp_form: Option<McpForm>,
    /// The add-rule form, while open.
    pub rule_form: Option<RuleForm>,
    /// Which of the built-in agent's three endpoints the detail pane shows.
    /// Messages leads because it is the route Claude models take, which is
    /// what a signed-in session uses by default.
    pub selected_endpoint: NativeEndpoint,
}

/// One of the built-in agent's three routes.
///
/// It speaks three APIs and reaches each separately, so each has its own
/// address, key and saved profiles — unlike a CLI, which has one endpoint.
/// The ids are the ones `sub2api::custom_api` stores them under.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum NativeEndpoint {
    #[default]
    Messages,
    Responses,
    Chat,
}

impl NativeEndpoint {
    pub(super) const ALL: [Self; 3] = [Self::Messages, Self::Responses, Self::Chat];

    /// The storage id, which is also what the routing writer files it under.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Messages => "native_messages",
            Self::Responses => "native_responses",
            Self::Chat => "native_chat",
        }
    }

    /// The path the adapter appends to whatever address is entered.
    ///
    /// Worth showing: pointing a route at a third-party service means that
    /// service has to implement *this* path, and a server that answers
    /// `/v1/chat/completions` very often does not answer `/v1/responses`.
    /// Verified against the adapters — `registry::responses_endpoint`,
    /// `providers/openai.rs`, and the Messages client in `api/src/lib.rs`.
    pub(super) fn request_path(self) -> &'static str {
        match self {
            Self::Messages => "/v1/messages",
            Self::Responses => "/v1/responses",
            Self::Chat => "/v1/chat/completions",
        }
    }

    /// The API it speaks, which is the only thing that distinguishes them.
    pub(super) fn label(self) -> String {
        match self {
            Self::Messages => tr!("cli_setup.native_route_messages"),
            Self::Responses => tr!("cli_setup.native_route_responses"),
            Self::Chat => tr!("cli_setup.native_route_chat"),
        }
    }
}

pub(super) struct McpForm {
    pub name: Entity<TextInput>,
    /// The command line for stdio servers, or the URL for the rest.
    pub target: Entity<TextInput>,
    pub stdio: bool,
    pub error: Option<String>,
}

pub(super) struct RuleForm {
    pub tool: Entity<TextInput>,
    pub allow: bool,
    pub error: Option<String>,
}

impl Waku {
    /// Read the engine's settings once, off-thread. Re-entrant: a page that is
    /// already loading or loaded does nothing.
    pub(super) fn ensure_agent_settings_loaded(&mut self, cx: &mut Context<Self>) {
        if self.agent_page.loading || self.agent_page.settings.is_some() {
            return;
        }
        self.agent_page.loading = true;
        cx.spawn(async move |this, cx| {
            let settings = cx
                .background_executor()
                .spawn(async { sub2api::agent_settings::load() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_page.loading = false;
                this.agent_page.settings = Some(settings);
                cx.notify();
            });
        })
        .detach();
    }

    /// Drop the loaded copy and read the file again — for a user who edited
    /// it by hand while the page was open.
    fn reload_agent_settings(&mut self, cx: &mut Context<Self>) {
        self.agent_page.settings = None;
        self.agent_page.error = None;
        self.ensure_agent_settings_loaded(cx);
    }

    /// Apply a change to the loaded settings and write them.
    fn update_agent_settings(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut AgentSettings),
    ) {
        let Some(settings) = self.agent_page.settings.as_mut() else {
            return;
        };
        edit(settings);
        let snapshot = settings.clone();
        self.agent_page.saving = true;
        self.agent_page.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { sub2api::agent_settings::save(&snapshot) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.agent_page.saving = false;
                match result {
                    Ok(()) => this.agent_page.saved_at = Some(Instant::now()),
                    Err(error) => this.agent_page.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- system prompt -------------------------------------------------

    fn open_agent_prompt_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_page.prompt_input.is_some() {
            return;
        }
        let current = self
            .agent_page
            .settings
            .as_ref()
            .and_then(|settings| settings.append_system_prompt.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(window, cx)
                .multi_line()
                .placeholder(tr!("agent.system_prompt_placeholder"));
            input.set_content(current, cx);
            input
        });
        cx.subscribe(&input, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
            .detach();
        self.agent_page.prompt_input = Some(input);
        cx.notify();
    }

    fn commit_agent_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.agent_page.prompt_input.take() else {
            return;
        };
        let text = input.read(cx).content().trim().to_owned();
        self.update_agent_settings(cx, move |settings| {
            settings.append_system_prompt = (!text.is_empty()).then_some(text);
        });
    }

    // ---- MCP servers ---------------------------------------------------

    fn open_agent_mcp_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_page.mcp_form.is_some() {
            return;
        }
        let name = cx.new(|cx| TextInput::new(window, cx).placeholder(tr!("agent.mcp_name")));
        let target =
            cx.new(|cx| TextInput::new(window, cx).placeholder(tr!("agent.mcp_command")));
        for input in [&name, &target] {
            cx.subscribe(input, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
                .detach();
        }
        self.agent_page.mcp_form = Some(McpForm {
            name,
            target,
            stdio: true,
            error: None,
        });
        cx.notify();
    }

    fn commit_agent_mcp_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.agent_page.mcp_form.as_ref() else {
            return;
        };
        let name = form.name.read(cx).content().trim().to_owned();
        let target = form.target.read(cx).content().trim().to_owned();
        let server = if form.stdio {
            // The first word is the executable; the rest are its arguments,
            // the way a shell would read the line.
            let mut words = target.split_whitespace().map(str::to_owned);
            McpServer {
                name: name.clone(),
                transport: "stdio".into(),
                command: words.next(),
                args: words.collect(),
                env: Default::default(),
                url: None,
            }
        } else {
            McpServer {
                name: name.clone(),
                transport: "http".into(),
                command: None,
                args: Vec::new(),
                env: Default::default(),
                url: Some(target),
            }
        };
        let duplicate = self
            .agent_page
            .settings
            .as_ref()
            .is_some_and(|settings| settings.mcp_servers.iter().any(|existing| existing.name == name));
        if !server.is_usable() || duplicate {
            if let Some(form) = self.agent_page.mcp_form.as_mut() {
                form.error = Some(if duplicate {
                    tr!("agent.mcp_duplicate", name = name)
                } else {
                    tr!("agent.mcp_invalid")
                });
            }
            cx.notify();
            return;
        }
        self.agent_page.mcp_form = None;
        self.update_agent_settings(cx, move |settings| settings.mcp_servers.push(server));
    }

    fn remove_agent_mcp_server(&mut self, name: String, cx: &mut Context<Self>) {
        self.update_agent_settings(cx, move |settings| {
            settings.mcp_servers.retain(|server| server.name != name);
        });
    }

    // ---- permission rules ----------------------------------------------

    fn open_agent_rule_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_page.rule_form.is_some() {
            return;
        }
        let tool = cx.new(|cx| TextInput::new(window, cx).placeholder(tr!("agent.rule_tool")));
        cx.subscribe(&tool, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
            .detach();
        self.agent_page.rule_form = Some(RuleForm {
            tool,
            allow: true,
            error: None,
        });
        cx.notify();
    }

    fn commit_agent_rule_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.agent_page.rule_form.as_ref() else {
            return;
        };
        let tool = form.tool.read(cx).content().trim().to_owned();
        let allow = form.allow;
        if tool.is_empty() {
            if let Some(form) = self.agent_page.rule_form.as_mut() {
                form.error = Some(tr!("agent.rule_invalid"));
            }
            cx.notify();
            return;
        }
        self.agent_page.rule_form = None;
        self.update_agent_settings(cx, move |settings| {
            settings.permission_rules.push(PermissionRule {
                tool_name: Some(tool),
                path_pattern: None,
                action: if allow { "Allow" } else { "Deny" }.to_owned(),
            });
        });
    }

    fn remove_agent_rule(&mut self, index: usize, cx: &mut Context<Self>) {
        self.update_agent_settings(cx, move |settings| {
            if index < settings.permission_rules.len() {
                settings.permission_rules.remove(index);
            }
        });
    }

    // ---- render ----------------------------------------------------------

    pub(super) fn render_agent_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(settings) = self.agent_page.settings.as_ref() else {
            return div()
                .mt(px(15.0))
                .w_full()
                .px(px(20.0))
                .py(px(16.0))
                .rounded(px(13.0))
                .bg(theme.raised)
                .text_size(sp(12.5))
                .text_color(theme.text_secondary)
                .child(tr!("common.loading"))
                .into_any_element();
        };

        div()
            .mt(px(15.0))
            .w_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(self.render_agent_header(theme, cx))
            .child(self.render_agent_endpoints(theme, cx))
            .child(self.render_agent_behaviour(settings, theme, cx))
            .child(self.render_agent_tools(settings, theme, cx))
            .child(self.render_agent_mcp(settings, theme, cx))
            .child(self.render_agent_rules(settings, theme, cx))
            .into_any_element()
    }

    /// The three APIs the built-in agent speaks, and where each one goes.
    ///
    /// A list of the three beside one form, rather than three stacked forms:
    /// they are alternatives, only one is being edited at a time, and
    /// stacking them made the page scroll past what the user came for.
    /// Each carries its own address, key and saved profiles — a route left
    /// blank falls back to the managed gateway, so filling one in is not a
    /// commitment about the other two.
    fn render_agent_endpoints(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let selected = self.agent_page.selected_endpoint;
        let stored = self.custom_api_snapshot();

        let mut list = div().flex().flex_col().gap(px(2.0)).w(px(188.0));
        for endpoint in NativeEndpoint::ALL {
            let active = endpoint == selected;
            // The dot says "pointed somewhere of your own" — the gateway
            // needs no mention, it is the default.
            let custom = stored.get(endpoint.id()).is_some();
            list = list.child(
                div()
                    .id(SharedString::from(format!("agent-endpoint-{}", endpoint.id())))
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .cursor_default()
                    .when(active, |row| row.bg(theme.surface))
                    .child(
                        div()
                            .w(px(5.0))
                            .h(px(5.0))
                            .rounded_full()
                            .bg(if custom { theme.accent } else { theme.border_strong }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(12.5))
                            .text_color(if active { theme.text } else { theme.text_secondary })
                            .child(SharedString::from(endpoint.label())),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent_page.selected_endpoint = endpoint;
                        cx.notify();
                    })),
            );
        }

        section_card(theme)
            .child(section_title(theme, tr!("agent.endpoints_title")))
            .child(section_description(theme, tr!("agent.endpoints_description")))
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .gap(px(16.0))
                    .items_start()
                    .child(list)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            // Which of the described endpoints serves this
                            // API — and nothing else. What an endpoint *is*
                            // (address, key, models) belongs to the
                            // model-providers page, because these three are
                            // not the only things that can use one; and the
                            // route in effect is not a second fact here,
                            // since for the built-in agent the binding is
                            // what decides it.
                            .child(self.render_route_binding(selected.id(), theme, cx))
                            .child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(tr!(
                                        "agent.endpoint_path_hint",
                                        path = selected.request_path()
                                    )),
                            ),
                    ),
            )
    }

    fn render_agent_header(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let file = sub2api::agent_settings::settings_path()
            .map(|path| abbreviate_home_path(&path, self.home_directory.as_deref()))
            .unwrap_or_default();
        let status = if let Some(error) = &self.agent_page.error {
            Some((tr!("agent.save_failed", error = error.clone()), theme.warning))
        } else if self.agent_page.saving {
            Some((tr!("common.saving"), theme.text_tertiary))
        } else if self
            .agent_page
            .saved_at
            .is_some_and(|at| at.elapsed() < SAVED_FLASH)
        {
            Some((tr!("agent.saved"), theme.success))
        } else {
            None
        };

        // The built-in agent is the only provider with no card on the
        // Providers page — everything about it lives here — so its
        // enable switch does too. Off, it stops offering models to new
        // sessions; ones already locked to it keep working.
        let disabled = self
            .state
            .disabled_providers
            .contains(&crate::model::ProviderKind::Native);
        let enable = crate::ui::toggle_switch(
            SharedString::from("agent-enabled"),
            !disabled,
            false,
            theme,
            cx,
            move |this, _, cx| {
                this.set_provider_enabled(crate::model::ProviderKind::Native, disabled, cx)
            },
        );

        section_card(theme)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(section_title(theme, tr!("agent.title_hint"))),
                    )
                    .child(enable),
            )
            .child(section_description(theme, tr!("agent.description")))
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(crate::md::render::MONO_FAMILY)
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(SharedString::from(file)),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from("agent-reload"),
                        tr!("agent.reload"),
                        false,
                        self.agent_page.saving,
                        cx,
                        |this, _, cx| this.reload_agent_settings(cx),
                    )),
            )
            .when_some(status, |element, (text, color)| {
                element.child(
                    div()
                        .mt(px(8.0))
                        .text_size(sp(12.5))
                        .text_color(color)
                        .child(text),
                )
            })
    }

    fn render_agent_behaviour(
        &self,
        settings: &AgentSettings,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let prompt_editor = self.agent_page.prompt_input.clone();
        let prompt_preview = settings
            .append_system_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty());

        let mut prompt_block = div().flex().flex_col().gap(px(8.0));
        if let Some(input) = prompt_editor {
            prompt_block = prompt_block
                .child(
                    div()
                        .min_h(px(96.0))
                        .child(TextField::new("agent-prompt-input", input)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(card_button(
                            theme,
                            SharedString::from("agent-prompt-save"),
                            tr!("common.save"),
                            true,
                            false,
                            cx,
                            |this, _, cx| this.commit_agent_prompt(cx),
                        ))
                        .child(card_button(
                            theme,
                            SharedString::from("agent-prompt-cancel"),
                            tr!("common.cancel"),
                            false,
                            false,
                            cx,
                            |this, _, cx| {
                                this.agent_page.prompt_input = None;
                                cx.notify();
                            },
                        )),
                );
        } else {
            prompt_block = prompt_block
                .child(
                    div()
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(if prompt_preview.is_some() {
                            theme.text
                        } else {
                            theme.text_tertiary
                        })
                        .child(prompt_preview.unwrap_or_else(|| tr!("agent.system_prompt_empty"))),
                )
                .child(
                    div().child(card_button(
                        theme,
                        SharedString::from("agent-prompt-edit"),
                        tr!("common.edit"),
                        false,
                        false,
                        cx,
                        |this, window, cx| this.open_agent_prompt_editor(window, cx),
                    )),
                );
        }

        let auto_compact = settings.auto_compact.unwrap_or(true);
        let threshold = settings.compact_threshold;
        let mut threshold_presets = div().flex().flex_wrap().gap(px(6.0));
        for (value, label) in COMPACT_PRESETS {
            let selected = threshold.is_some_and(|current| (current - value).abs() < 0.01)
                || (threshold.is_none() && (value - 0.8).abs() < 0.01);
            threshold_presets = threshold_presets.child(preset_button(
                theme,
                SharedString::from(format!("agent-compact-{label}")),
                label.to_owned(),
                selected,
                cx,
                move |this, _, cx| {
                    this.update_agent_settings(cx, move |settings| {
                        settings.compact_threshold = Some(value)
                    })
                },
            ));
        }

        section_card(theme)
            .child(section_title(theme, tr!("agent.behaviour")))
            .child(setting_row(
                theme,
                tr!("agent.system_prompt"),
                tr!("agent.system_prompt_description"),
                prompt_block.into_any_element(),
            ))
            .child(setting_row(
                theme,
                tr!("agent.auto_compact"),
                tr!("agent.auto_compact_description"),
                toggle_switch(
                    "agent-auto-compact",
                    auto_compact,
                    self.agent_page.saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        this.update_agent_settings(cx, move |settings| {
                            settings.auto_compact = Some(!auto_compact)
                        })
                    },
                )
                .into_any_element(),
            ))
            .when(auto_compact, |element| {
                element.child(setting_row(
                    theme,
                    tr!("agent.compact_threshold"),
                    tr!("agent.compact_threshold_description"),
                    threshold_presets.into_any_element(),
                ))
            })
    }

    fn render_agent_tools(
        &self,
        settings: &AgentSettings,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().mt(px(6.0)).flex().flex_col();
        for tool in BUILTIN_TOOLS {
            let essential = ESSENTIAL_TOOLS.contains(&tool);
            let enabled = !settings.disallowed_tools.iter().any(|name| name == tool);
            let name = tool.to_owned();
            list = list.child(
                div()
                    .h(px(34.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(crate::md::render::MONO_FAMILY)
                            .text_size(sp(12.5))
                            .text_color(if enabled {
                                theme.text
                            } else {
                                theme.text_tertiary
                            })
                            .child(tool),
                    )
                    .when(essential, |element| {
                        element.child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_ghost)
                                .child(tr!("agent.tool_locked")),
                        )
                    })
                    .child(toggle_switch(
                        SharedString::from(format!("agent-tool-{tool}")),
                        enabled,
                        essential || self.agent_page.saving,
                        theme,
                        cx,
                        move |this, _, cx| {
                            let name = name.clone();
                            this.update_agent_settings(cx, move |settings| {
                                if enabled {
                                    settings.disallowed_tools.push(name);
                                } else {
                                    settings.disallowed_tools.retain(|item| item != &name);
                                }
                            })
                        },
                    )),
            );
        }

        section_card(theme)
            .child(section_title(theme, tr!("agent.tools")))
            .child(section_description(theme, tr!("agent.tools_description")))
            .child(list)
    }

    fn render_agent_mcp(
        &self,
        settings: &AgentSettings,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().mt(px(6.0)).flex().flex_col();
        if settings.mcp_servers.is_empty() {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("agent.mcp_empty")),
            );
        }
        for server in &settings.mcp_servers {
            let name = server.name.clone();
            list = list.child(
                div()
                    .py(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(server.name.clone()),
                            )
                            .child(
                                div()
                                    .font_family(crate::md::render::MONO_FAMILY)
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(server.summary()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(server.transport.clone()),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-mcp-remove-{}", server.name)),
                        tr!("common.remove"),
                        false,
                        self.agent_page.saving,
                        cx,
                        move |this, _, cx| this.remove_agent_mcp_server(name.clone(), cx),
                    )),
            );
        }

        let trust = settings.trust_project_mcp_servers;
        let mut card = section_card(theme)
            .child(section_title(theme, tr!("agent.mcp")))
            .child(section_description(theme, tr!("agent.mcp_description")))
            .child(list)
            .child(setting_row(
                theme,
                tr!("agent.mcp_trust_project"),
                tr!("agent.mcp_trust_project_description"),
                toggle_switch(
                    "agent-mcp-trust",
                    trust,
                    self.agent_page.saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        this.update_agent_settings(cx, move |settings| {
                            settings.trust_project_mcp_servers = !trust
                        })
                    },
                )
                .into_any_element(),
            ));

        if let Some(form) = &self.agent_page.mcp_form {
            let stdio = form.stdio;
            card = card.child(
                div()
                    .mt(px(12.0))
                    .p(px(12.0))
                    .rounded(px(9.0))
                    .bg(theme.overlay)
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(preset_button(
                                theme,
                                "agent-mcp-stdio",
                                tr!("agent.mcp_transport_stdio"),
                                stdio,
                                cx,
                                |this, _, cx| {
                                    if let Some(form) = this.agent_page.mcp_form.as_mut() {
                                        form.stdio = true;
                                        form.target.update(cx, |input, cx| {
                                            input.set_placeholder(tr!("agent.mcp_command"), cx)
                                        });
                                    }
                                    cx.notify();
                                },
                            ))
                            .child(preset_button(
                                theme,
                                "agent-mcp-http",
                                tr!("agent.mcp_transport_http"),
                                !stdio,
                                cx,
                                |this, _, cx| {
                                    if let Some(form) = this.agent_page.mcp_form.as_mut() {
                                        form.stdio = false;
                                        form.target.update(cx, |input, cx| {
                                            input.set_placeholder(tr!("agent.mcp_url"), cx)
                                        });
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(TextField::new("agent-mcp-name", form.name.clone()))
                    .child(TextField::new("agent-mcp-target", form.target.clone()))
                    .when_some(form.error.clone(), |element, error| {
                        element.child(
                            div()
                                .text_size(sp(12.0))
                                .text_color(theme.warning)
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-mcp-add-confirm"),
                                tr!("common.add"),
                                true,
                                self.agent_page.saving,
                                cx,
                                |this, _, cx| this.commit_agent_mcp_form(cx),
                            ))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-mcp-add-cancel"),
                                tr!("common.cancel"),
                                false,
                                false,
                                cx,
                                |this, _, cx| {
                                    this.agent_page.mcp_form = None;
                                    cx.notify();
                                },
                            )),
                    ),
            );
        } else {
            card = card.child(div().mt(px(12.0)).child(card_button(
                theme,
                SharedString::from("agent-mcp-add"),
                tr!("agent.mcp_add"),
                false,
                self.agent_page.saving,
                cx,
                |this, window, cx| this.open_agent_mcp_form(window, cx),
            )));
        }
        card
    }

    fn render_agent_rules(
        &self,
        settings: &AgentSettings,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut list = div().mt(px(6.0)).flex().flex_col();
        if settings.permission_rules.is_empty() {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("agent.rules_empty")),
            );
        }
        for (index, rule) in settings.permission_rules.iter().enumerate() {
            let allow = rule.is_allow();
            list = list.child(
                div()
                    .h(px(34.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(crate::md::render::MONO_FAMILY)
                            .text_size(sp(12.5))
                            .text_color(theme.text)
                            .child(rule.subject()),
                    )
                    .child(
                        div()
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(5.0))
                            .text_size(sp(11.5))
                            .text_color(if allow { theme.success } else { theme.warning })
                            .border_1()
                            .border_color(if allow { theme.success } else { theme.warning })
                            .child(if allow {
                                tr!("agent.rule_allow")
                            } else {
                                tr!("agent.rule_deny")
                            }),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-rule-remove-{index}")),
                        tr!("common.remove"),
                        false,
                        self.agent_page.saving,
                        cx,
                        move |this, _, cx| this.remove_agent_rule(index, cx),
                    )),
            );
        }

        let mut card = section_card(theme)
            .child(section_title(theme, tr!("agent.rules")))
            .child(section_description(theme, tr!("agent.rules_description")))
            .child(list);

        if let Some(form) = &self.agent_page.rule_form {
            let allow = form.allow;
            card = card.child(
                div()
                    .mt(px(12.0))
                    .p(px(12.0))
                    .rounded(px(9.0))
                    .bg(theme.overlay)
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(TextField::new("agent-rule-tool", form.tool.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(preset_button(
                                theme,
                                "agent-rule-allow",
                                tr!("agent.rule_allow"),
                                allow,
                                cx,
                                |this, _, cx| {
                                    if let Some(form) = this.agent_page.rule_form.as_mut() {
                                        form.allow = true;
                                    }
                                    cx.notify();
                                },
                            ))
                            .child(preset_button(
                                theme,
                                "agent-rule-deny",
                                tr!("agent.rule_deny"),
                                !allow,
                                cx,
                                |this, _, cx| {
                                    if let Some(form) = this.agent_page.rule_form.as_mut() {
                                        form.allow = false;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .when_some(form.error.clone(), |element, error| {
                        element.child(
                            div()
                                .text_size(sp(12.0))
                                .text_color(theme.warning)
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-rule-add-confirm"),
                                tr!("common.add"),
                                true,
                                self.agent_page.saving,
                                cx,
                                |this, _, cx| this.commit_agent_rule_form(cx),
                            ))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-rule-add-cancel"),
                                tr!("common.cancel"),
                                false,
                                false,
                                cx,
                                |this, _, cx| {
                                    this.agent_page.rule_form = None;
                                    cx.notify();
                                },
                            )),
                    ),
            );
        } else {
            card = card.child(div().mt(px(12.0)).child(card_button(
                theme,
                SharedString::from("agent-rule-add"),
                tr!("agent.rule_add"),
                false,
                self.agent_page.saving,
                cx,
                |this, window, cx| this.open_agent_rule_form(window, cx),
            )));
        }
        card
    }
}

// ---- small layout helpers, local to this page ------------------------------

fn section_card(theme: Theme) -> Div {
    div()
        .w_full()
        .px(px(20.0))
        .py(px(16.0))
        .rounded(px(13.0))
        .bg(theme.raised)
        .flex()
        .flex_col()
}

fn section_title(theme: Theme, title: String) -> Div {
    div()
        .text_size(sp(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.text)
        .child(title)
}

fn section_description(theme: Theme, text: String) -> Div {
    div()
        .mt(px(4.0))
        .text_size(sp(12.5))
        .line_height(sp(18.0))
        .text_color(theme.text_secondary)
        .child(text)
}

/// A labelled setting: name and explanation on the left, the control on the
/// right — or below, for controls that need the width.
fn setting_row(theme: Theme, label: String, description: String, control: AnyElement) -> Div {
    div()
        .mt(px(14.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(sp(12.0))
                        .line_height(sp(17.0))
                        .text_color(theme.text_tertiary)
                        .child(description),
                ),
        )
        .child(control)
}

/// A small selectable chip, for a choice among a handful of presets.
fn preset_button(
    theme: Theme,
    id: impl Into<SharedString>,
    label: String,
    selected: bool,
    cx: &mut Context<Waku>,
    activate: impl Fn(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
) -> Stateful<Div> {
    div()
        .id(id.into())
        .tab_index(0)
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(if selected {
            theme.accent
        } else {
            theme.border_strong
        })
        .bg(if selected { theme.accent.opacity(0.12) } else { theme.raised })
        .flex()
        .items_center()
        .cursor_default()
        .text_size(sp(12.0))
        .text_color(if selected { theme.text } else { theme.text_secondary })
        .focus_visible(|style| style.border_color(theme.accent))
        .hover(|element| element.bg(theme.overlay))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| activate(this, window, cx)))
}
