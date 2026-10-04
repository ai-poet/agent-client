//! Settings → Agent → Teams: AgentTeams for the built-in agent.
//!
//! Fork addition, in place of the former Settings → Workflow page. Teams
//! are started from a conversation (`/agent-teams <goal>`), not configured
//! here; this section holds the defaults every team starts from and the
//! named team templates (profiles) a captain can be asked to use.
//!
//! The settings live in `agent-teams.json` beside the engine's own settings
//! file — the engine rewrites `settings.json` from its typed struct and
//! would drop a key it does not know. Loaded and saved on the background
//! executor; a running session keeps the settings it started with.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use agent_teams::config::TeamsConfig;
use agent_teams::profiles::TeamProfileConfig;

use super::providers_page::card_button;
use super::*;

use crate::ui::text_field::TextField;

/// Presets for the member cap.
const MAX_MEMBER_PRESETS: [u32; 5] = [2, 4, 8, 12, 16];

#[derive(Default)]
pub(super) struct AgentTeamsPageState {
    pub config: Option<TeamsConfig>,
    pub loading: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub saved_at: Option<Instant>,
    /// The execution-prompt editor, while open.
    pub prompt_input: Option<Entity<TextInput>>,
    /// The profile editor, while open.
    pub profile_editor: Option<ProfileEditor>,
}

pub(super) struct ProfileEditor {
    pub name: Entity<TextInput>,
    pub body: Entity<TextInput>,
    /// The name being edited; `None` for a new profile.
    pub original: Option<String>,
    pub error: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    sub2api::global_config::native::config_dir()
        .map(|dir| dir.join(agent_teams::config::CONFIG_FILE))
}

fn load_config() -> Result<TeamsConfig, String> {
    let path = config_path().ok_or_else(|| "no home directory".to_owned())?;
    agent_teams::config::load(&path).map_err(|error| format!("{error:#}"))
}

fn save_config(config: &TeamsConfig) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "no home directory".to_owned())?;
    agent_teams::config::save(&path, config).map_err(|error| format!("{error:#}"))
}

/// A profile as the editor shows it: pretty JSON, keys as the file has them.
fn profile_text(profile: &TeamProfileConfig) -> String {
    serde_json::to_string_pretty(profile).unwrap_or_default()
}

/// What a new profile starts as.
fn profile_skeleton() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "description": "An engineer implements, a reviewer checks the result.",
        "protocol": "Plan the smallest useful task graph; every change is reviewed before it is integrated.",
        "taskPlanning": "captain",
        "members": [
            { "name": "engineer", "role": "implements the change" },
            { "name": "reviewer", "role": "reviews the change" }
        ]
    }))
    .unwrap_or_default()
}

impl Waku {
    pub(super) fn ensure_agent_teams_loaded(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.agent_page.teams;
        if state.loading || state.config.is_some() {
            return;
        }
        state.loading = true;
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async { load_config() })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.teams;
                state.loading = false;
                match loaded {
                    Ok(config) => state.config = Some(config),
                    Err(error) => {
                        state.config = Some(TeamsConfig::default());
                        state.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Apply a change and write the file.
    fn update_agent_teams(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut TeamsConfig)) {
        let Some(config) = self.agent_page.teams.config.as_mut() else {
            return;
        };
        edit(config);
        let snapshot = config.clone();
        self.agent_page.teams.saving = true;
        self.agent_page.teams.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { save_config(&snapshot) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.agent_page.teams;
                state.saving = false;
                match result {
                    Ok(()) => state.saved_at = Some(Instant::now()),
                    Err(error) => state.error = Some(error),
                }
                // `/agent-teams` and its profile aliases come and go with
                // these settings.
                this.composer_sources_stale = true;
                cx.notify();
            });
        })
        .detach();
    }

    /// The profiles, materialized from the built-in templates the first
    /// time anything about them is changed.
    fn edit_agent_team_profiles(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut BTreeMap<String, TeamProfileConfig>),
    ) {
        self.update_agent_teams(cx, move |config| {
            let profiles = config
                .profiles
                .get_or_insert_with(agent_teams::profiles::builtin_profiles);
            edit(profiles);
        });
    }

    fn open_agent_teams_prompt_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_page.teams.prompt_input.is_some() {
            return;
        }
        let current = self
            .agent_page
            .teams
            .config
            .as_ref()
            .and_then(|config| config.execution_prompt.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(window, cx)
                .multi_line()
                .placeholder(tr!("team.settings.execution_prompt_placeholder"));
            input.set_content(current, cx);
            input
        });
        cx.subscribe(&input, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
            .detach();
        self.agent_page.teams.prompt_input = Some(input);
        cx.notify();
    }

    fn commit_agent_teams_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.agent_page.teams.prompt_input.take() else {
            return;
        };
        let text = input.read(cx).content().trim().to_owned();
        self.update_agent_teams(cx, move |config| {
            config.execution_prompt = (!text.is_empty()).then_some(text);
        });
    }

    fn open_agent_team_profile_editor(
        &mut self,
        original: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profiles = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map(TeamsConfig::effective_profiles)
            .unwrap_or_default();
        let body_text = original
            .as_ref()
            .and_then(|name| profiles.get(name))
            .map(profile_text)
            .unwrap_or_else(profile_skeleton);
        let name_text = original.clone().unwrap_or_default();
        let name = cx.new(|cx| {
            let mut input = TextInput::new(window, cx)
                .placeholder(tr!("team.settings.profile_name_placeholder"));
            input.set_content(name_text, cx);
            input
        });
        let body = cx.new(|cx| {
            let mut input = TextInput::new(window, cx).multi_line();
            input.set_content(body_text, cx);
            input
        });
        for input in [&name, &body] {
            cx.subscribe(input, |_: &mut Self, _, _: &InputEvent, cx| cx.notify())
                .detach();
        }
        self.agent_page.teams.profile_editor = Some(ProfileEditor {
            name,
            body,
            original,
            error: None,
        });
        cx.notify();
    }

    fn commit_agent_team_profile(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_page.teams.profile_editor.as_ref() else {
            return;
        };
        let name = editor.name.read(cx).content().trim().to_owned();
        let body = editor.body.read(cx).content().to_owned();
        let original = editor.original.clone();
        let max_members = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map_or(8, |config| config.max_members() as usize);
        let checked = (|| -> Result<TeamProfileConfig, String> {
            if name.is_empty() {
                return Err(tr!("team.settings.profile_name_required"));
            }
            let value: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
                tr!(
                    "team.settings.profile_invalid_json",
                    error = error.to_string()
                )
            })?;
            agent_teams::profiles::normalize_team_profile_value(&name, &value, max_members)?;
            serde_json::from_value(value).map_err(|error| error.to_string())
        })();
        let taken = original.as_deref() != Some(name.as_str())
            && self
                .agent_page
                .teams
                .config
                .as_ref()
                .is_some_and(|config| config.effective_profiles().contains_key(&name));
        match checked {
            Ok(_) if taken => {
                if let Some(editor) = self.agent_page.teams.profile_editor.as_mut() {
                    editor.error = Some(tr!("team.settings.profile_name_taken", name = name));
                }
                cx.notify();
            }
            Ok(profile) => {
                self.agent_page.teams.profile_editor = None;
                self.edit_agent_team_profiles(cx, move |profiles| {
                    if let Some(original) = &original {
                        profiles.remove(original);
                    }
                    profiles.insert(name, profile);
                });
            }
            Err(error) => {
                if let Some(editor) = self.agent_page.teams.profile_editor.as_mut() {
                    editor.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    fn remove_agent_team_profile(&mut self, name: String, cx: &mut Context<Self>) {
        self.edit_agent_team_profiles(cx, move |profiles| {
            profiles.remove(&name);
        });
    }

    /// Put back the built-in templates the user removed, keeping their own.
    fn restore_agent_team_templates(&mut self, cx: &mut Context<Self>) {
        self.edit_agent_team_profiles(cx, |profiles| {
            for (name, profile) in agent_teams::profiles::builtin_profiles() {
                profiles.entry(name).or_insert(profile);
            }
        });
    }

    // ---- render -----------------------------------------------------------

    pub(super) fn render_agent_teams_section(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let state = &self.agent_page.teams;
        let Some(config) = state.config.as_ref() else {
            return super::agent_page::section_card(theme)
                .child(super::agent_page::section_title(
                    theme,
                    tr!("team.settings.title"),
                ))
                .child(super::agent_page::section_description(
                    theme,
                    tr!("common.loading"),
                ));
        };
        let saving = state.saving;
        let enabled = config.enabled;
        let mut card = super::agent_page::section_card(theme)
            .child(super::agent_page::section_title(
                theme,
                tr!("team.settings.title"),
            ))
            .child(super::agent_page::section_description(
                theme,
                tr!("team.settings.description"),
            ))
            .child(super::agent_page::setting_row(
                theme,
                tr!("team.settings.enabled"),
                tr!("team.settings.enabled_description"),
                toggle_switch(
                    "agent-teams-enabled",
                    enabled,
                    saving,
                    theme,
                    cx,
                    move |this, _, cx| {
                        this.update_agent_teams(cx, move |config| config.enabled = !enabled)
                    },
                )
                .into_any_element(),
            ));
        if !enabled {
            return card.when_some(state.error.clone(), |card, error| {
                card.child(error_line(theme, error))
            });
        }

        let slash = config.slash_command;
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("team.settings.slash_command"),
            tr!("team.settings.slash_command_description"),
            toggle_switch(
                "agent-teams-slash",
                slash,
                saving,
                theme,
                cx,
                move |this, _, cx| {
                    this.update_agent_teams(cx, move |config| config.slash_command = !slash)
                },
            )
            .into_any_element(),
        ));

        let current_cap = config.max_members();
        let mut caps = div().flex().flex_wrap().gap(px(6.0));
        for cap in MAX_MEMBER_PRESETS {
            caps = caps.child(super::agent_page::preset_button(
                theme,
                SharedString::from(format!("agent-teams-cap-{cap}")),
                cap.to_string(),
                cap == current_cap,
                cx,
                move |this, _, cx| {
                    this.update_agent_teams(cx, move |config| config.max_members = cap)
                },
            ));
        }
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("team.settings.max_members"),
            tr!("team.settings.max_members_description"),
            caps.into_any_element(),
        ));

        let delegate = config.member_max_depth() > 0;
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("team.settings.member_subagents"),
            tr!("team.settings.member_subagents_description"),
            toggle_switch(
                "agent-teams-depth",
                delegate,
                saving,
                theme,
                cx,
                move |this, _, cx| {
                    this.update_agent_teams(cx, move |config| {
                        config.member_max_depth = if delegate { 0 } else { 1 }
                    })
                },
            )
            .into_any_element(),
        ));

        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("team.settings.member_model"),
            tr!("team.settings.member_model_description"),
            self.render_agent_teams_model_picker(config, theme, cx),
        ));
        card = card.child(super::agent_page::setting_row(
            theme,
            tr!("team.settings.execution_prompt"),
            tr!("team.settings.execution_prompt_description"),
            self.render_agent_teams_prompt(config, theme, cx),
        ));
        card = card.child(self.render_agent_team_profiles(config, theme, cx));
        card.when_some(state.error.clone(), |card, error| {
            card.child(error_line(theme, error))
        })
    }

    fn render_agent_teams_model_picker(
        &self,
        config: &TeamsConfig,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let models: Vec<ProviderModel> = self
            .provider_probe(ProviderKind::Native)
            .map(|probe| probe.models.clone())
            .unwrap_or_default();
        let current = config.member_model.clone();
        let current_label = current
            .as_ref()
            .map(|id| {
                models
                    .iter()
                    .find(|model| &model.id == id)
                    .map(|model| model.name.clone())
                    .unwrap_or_else(|| id.clone())
            })
            .unwrap_or_else(|| tr!("team.settings.member_model_inherit"));
        let effort = config.member_reasoning_effort.clone();
        let efforts: Vec<(String, String)> = current
            .as_ref()
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .map(|model| {
                model
                    .reasoning_efforts
                    .iter()
                    .map(|option| (option.id.clone(), option.label.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let weak = cx.entity().downgrade();
        let model_handle = self.menu_handle("agent-teams-model", cx);
        let model_trigger = picker_trigger(theme, "agent-teams-model-trigger", current_label);
        let model_menu = {
            let models = models.clone();
            let current = current.clone();
            let weak = weak.clone();
            dropdown_menu(
                model_trigger,
                "agent-teams-model-menu",
                &model_handle,
                MenuAlign::BelowLeft,
                move |_| {
                    let mut items = Vec::new();
                    let inherit = weak.clone();
                    items.push(
                        MenuItem::new(tr!("team.settings.member_model_inherit"), move |_, cx| {
                            let _ = inherit.update(cx, |this, cx| {
                                this.update_agent_teams(cx, |config| {
                                    config.member_model = None;
                                    config.member_reasoning_effort = None;
                                })
                            });
                        })
                        .selected(current.is_none()),
                    );
                    items.push(MenuItem::Separator);
                    for model in &models {
                        let id = model.id.clone();
                        let default_effort = model.default_reasoning_effort.clone();
                        let pick = weak.clone();
                        items.push(
                            MenuItem::new(model.name.clone(), move |_, cx| {
                                let id = id.clone();
                                let default_effort = default_effort.clone();
                                let _ = pick.update(cx, |this, cx| {
                                    this.update_agent_teams(cx, move |config| {
                                        config.member_model = Some(id);
                                        config.member_reasoning_effort = default_effort;
                                    })
                                });
                            })
                            .selected(current.as_deref() == Some(model.id.as_str())),
                        );
                    }
                    items
                },
            )
        };

        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(model_menu);
        if !efforts.is_empty() {
            let label = effort
                .as_ref()
                .and_then(|id| efforts.iter().find(|(option, _)| option == id))
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| tr!("team.settings.member_effort_default"));
            let effort_handle = self.menu_handle("agent-teams-effort", cx);
            let effort_trigger = picker_trigger(theme, "agent-teams-effort-trigger", label);
            row = row.child(dropdown_menu(
                effort_trigger,
                "agent-teams-effort-menu",
                &effort_handle,
                MenuAlign::BelowLeft,
                move |_| {
                    efforts
                        .iter()
                        .map(|(id, label)| {
                            let id = id.clone();
                            let pick = weak.clone();
                            let selected = effort.as_deref() == Some(id.as_str());
                            MenuItem::new(label.clone(), move |_, cx| {
                                let id = id.clone();
                                let _ = pick.update(cx, |this, cx| {
                                    this.update_agent_teams(cx, move |config| {
                                        config.member_reasoning_effort = Some(id)
                                    })
                                });
                            })
                            .selected(selected)
                        })
                        .collect()
                },
            ));
        }
        row.into_any_element()
    }

    fn render_agent_teams_prompt(
        &self,
        config: &TeamsConfig,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(input) = self.agent_page.teams.prompt_input.clone() {
            return div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .min_h(px(80.0))
                        .child(TextField::new("agent-teams-prompt-input", input)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(card_button(
                            theme,
                            SharedString::from("agent-teams-prompt-save"),
                            tr!("common.save"),
                            true,
                            false,
                            cx,
                            |this, _, cx| this.commit_agent_teams_prompt(cx),
                        ))
                        .child(card_button(
                            theme,
                            SharedString::from("agent-teams-prompt-cancel"),
                            tr!("common.cancel"),
                            false,
                            false,
                            cx,
                            |this, _, cx| {
                                this.agent_page.teams.prompt_input = None;
                                cx.notify();
                            },
                        )),
                )
                .into_any_element();
        }
        let preview = config
            .execution_prompt
            .clone()
            .filter(|text| !text.trim().is_empty());
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .line_height(sp(18.0))
                    .text_color(if preview.is_some() {
                        theme.text
                    } else {
                        theme.text_tertiary
                    })
                    .child(preview.unwrap_or_else(|| tr!("team.settings.execution_prompt_empty"))),
            )
            .child(div().child(card_button(
                theme,
                SharedString::from("agent-teams-prompt-edit"),
                tr!("common.edit"),
                false,
                false,
                cx,
                |this, window, cx| this.open_agent_teams_prompt_editor(window, cx),
            )))
            .into_any_element()
    }

    fn render_agent_team_profiles(
        &self,
        config: &TeamsConfig,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let saving = self.agent_page.teams.saving;
        let profiles = config.effective_profiles();
        let mut list = div().mt(px(6.0)).flex().flex_col();
        if profiles.is_empty() {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.profiles_empty")),
            );
        }
        for (name, profile) in &profiles {
            let planning = match agent_teams::profiles::resolve_profile_task_planning(Some(profile))
            {
                agent_teams::types::TaskPlanning::Captain => tr!("team.settings.planning_captain"),
                agent_teams::types::TaskPlanning::Seed => {
                    tr!("team.settings.planning_seed", count = profile.tasks.len())
                }
            };
            let summary = profile
                .description
                .clone()
                .or_else(|| profile.protocol.clone())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_default();
            let roster = profile
                .members
                .iter()
                .map(|member| member.name.clone())
                .collect::<Vec<_>>()
                .join(" · ");
            let edit_name = name.clone();
            let remove_name = name.clone();
            let command = agent_teams::command::profile_command_name(name)
                .map(|alias| format!("/{alias}"))
                .unwrap_or_default();
            list = list.child(
                div()
                    .py(px(9.0))
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .text_size(sp(12.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme.text)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(sp(11.5))
                                            .text_color(theme.text_ghost)
                                            .child(planning),
                                    ),
                            )
                            .when(!summary.is_empty(), |element| {
                                element.child(
                                    div()
                                        .text_size(sp(12.0))
                                        .line_height(sp(17.0))
                                        .text_color(theme.text_secondary)
                                        .child(summary),
                                )
                            })
                            .child(
                                div()
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child(roster),
                            )
                            .when(!command.is_empty(), |element| {
                                element.child(
                                    div()
                                        .font_family(crate::md::render::MONO_FAMILY)
                                        .text_size(sp(11.5))
                                        .text_color(theme.text_tertiary)
                                        .child(command),
                                )
                            }),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-teams-profile-edit-{name}")),
                        tr!("common.edit"),
                        false,
                        saving,
                        cx,
                        move |this, window, cx| {
                            this.open_agent_team_profile_editor(Some(edit_name.clone()), window, cx)
                        },
                    ))
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-teams-profile-remove-{name}")),
                        tr!("common.remove"),
                        false,
                        saving,
                        cx,
                        move |this, _, cx| this.remove_agent_team_profile(remove_name.clone(), cx),
                    )),
            );
        }

        let mut block = div()
            .mt(px(16.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("team.settings.profiles")),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.profiles_description")),
            )
            .child(list);

        if let Some(editor) = &self.agent_page.teams.profile_editor {
            block = block.child(
                div()
                    .mt(px(12.0))
                    .p(px(12.0))
                    .rounded(px(9.0))
                    .bg(theme.overlay)
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(TextField::new(
                        "agent-teams-profile-name",
                        editor.name.clone(),
                    ))
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!("team.settings.profile_body_hint")),
                    )
                    .child(
                        div()
                            .min_h(px(220.0))
                            .font_family(crate::md::render::MONO_FAMILY)
                            .child(TextField::new(
                                "agent-teams-profile-body",
                                editor.body.clone(),
                            )),
                    )
                    .when_some(editor.error.clone(), |element, error| {
                        element.child(error_line(theme, error))
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-teams-profile-save"),
                                tr!("common.save"),
                                true,
                                saving,
                                cx,
                                |this, _, cx| this.commit_agent_team_profile(cx),
                            ))
                            .child(card_button(
                                theme,
                                SharedString::from("agent-teams-profile-cancel"),
                                tr!("common.cancel"),
                                false,
                                false,
                                cx,
                                |this, _, cx| {
                                    this.agent_page.teams.profile_editor = None;
                                    cx.notify();
                                },
                            )),
                    ),
            );
        } else {
            block = block.child(
                div()
                    .mt(px(12.0))
                    .flex()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(card_button(
                        theme,
                        SharedString::from("agent-teams-profile-new"),
                        tr!("team.settings.profile_new"),
                        false,
                        saving,
                        cx,
                        |this, window, cx| this.open_agent_team_profile_editor(None, window, cx),
                    ))
                    .child(card_button(
                        theme,
                        SharedString::from("agent-teams-profile-restore"),
                        tr!("team.settings.profile_restore"),
                        false,
                        saving,
                        cx,
                        |this, _, cx| this.restore_agent_team_templates(cx),
                    )),
            );
        }
        block
    }
}

fn error_line(theme: Theme, error: String) -> Div {
    div()
        .mt(px(10.0))
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(theme.warning)
        .child(error)
}

/// A dropdown trigger: the current choice and a chevron.
fn picker_trigger(theme: Theme, id: &'static str, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.raised)
        .flex()
        .items_center()
        .gap(px(6.0))
        .cursor_default()
        .text_size(sp(12.0))
        .text_color(theme.text)
        .focus_visible(|style| style.border_color(theme.accent))
        .hover(|element| element.bg(theme.overlay))
        .child(label)
        .child(icon("icons/chevron-down.svg", 12.0, theme.text_tertiary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_profile_starts_valid() {
        let value: serde_json::Value = serde_json::from_str(&profile_skeleton()).unwrap();
        assert!(agent_teams::profiles::normalize_team_profile_value("new", &value, 8).is_ok());
    }

    #[test]
    fn built_in_templates_round_trip_through_the_editor() {
        for (name, profile) in agent_teams::profiles::builtin_profiles() {
            let value: serde_json::Value = serde_json::from_str(&profile_text(&profile)).unwrap();
            agent_teams::profiles::normalize_team_profile_value(&name, &value, 8).unwrap();
            let back: TeamProfileConfig = serde_json::from_value(value).unwrap();
            assert_eq!(back, profile, "{name}");
        }
    }
}
