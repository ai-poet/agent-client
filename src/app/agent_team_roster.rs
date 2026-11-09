//! Structured role and model editing for one AgentTeams profile.
//!
//! The surrounding profile editor still owns the protocol, task DAG and review
//! policy as JSON. This editor changes only the roster, preserving every other
//! field and each member's advanced prompt/fallback settings.

use std::rc::Rc;

use super::*;

/// One profile's roster while it is edited. Saving validates the whole profile
/// before replacing it, including references from fixed tasks to member names.
pub(super) struct RosterEditor {
    pub profile: String,
    members: Vec<RosterMemberEditor>,
    error: Option<String>,
}

struct RosterMemberEditor {
    name: Entity<TextInput>,
    role: Entity<TextInput>,
    /// Everything not exposed by the structured editor is retained from here.
    template: ProfileMemberConfig,
    provider: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

fn roster_member_editor(
    member: ProfileMemberConfig,
    window: &mut Window,
    cx: &mut Context<Waku>,
) -> RosterMemberEditor {
    let name = cx.new(|cx| {
        let mut input =
            TextInput::new(window, cx).placeholder(tr!("team.settings.role_name_placeholder"));
        input.set_content(member.name.clone(), cx);
        input
    });
    let role = cx.new(|cx| {
        let mut input = TextInput::new(window, cx)
            .placeholder(tr!("team.settings.role_description_placeholder"));
        input.set_content(member.role.clone().unwrap_or_default(), cx);
        input
    });
    for input in [&name, &role] {
        cx.subscribe(input, |_: &mut Waku, _, _: &InputEvent, cx| cx.notify())
            .detach();
    }
    RosterMemberEditor {
        name,
        role,
        provider: member.provider.clone(),
        model: member.model.clone(),
        reasoning_effort: member.reasoning_effort.clone(),
        template: member,
    }
}

impl Waku {
    pub(super) fn open_agent_team_roster_editor(
        &mut self,
        profile_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map(TeamsConfig::effective_profiles)
            .and_then(|profiles| profiles.get(&profile_name).cloned())
        else {
            return;
        };
        let members = profile
            .members
            .into_iter()
            .map(|member| roster_member_editor(member, window, cx))
            .collect();
        self.agent_page.teams.profile_editor = None;
        self.agent_page.teams.roster_editor = Some(RosterEditor {
            profile: profile_name,
            members,
            error: None,
        });
        cx.notify();
    }

    fn add_agent_team_roster_member(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let max_members = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map_or(8, |config| config.max_members() as usize);
        let Some(editor) = self.agent_page.teams.roster_editor.as_mut() else {
            return;
        };
        if editor.members.len() >= max_members {
            editor.error = Some(tr!("team.settings.roster_limit", max = max_members));
        } else {
            editor.error = None;
            editor.members.push(roster_member_editor(
                ProfileMemberConfig::default(),
                window,
                cx,
            ));
        }
        cx.notify();
    }

    fn remove_agent_team_roster_member(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.agent_page.teams.roster_editor.as_mut() {
            if index < editor.members.len() {
                editor.members.remove(index);
                editor.error = None;
            }
        }
        cx.notify();
    }

    fn set_agent_team_roster_route(
        &mut self,
        index: usize,
        selected: Option<ProviderModel>,
        cx: &mut Context<Self>,
    ) {
        let Some(member) = self
            .agent_page
            .teams
            .roster_editor
            .as_mut()
            .and_then(|editor| editor.members.get_mut(index))
        else {
            return;
        };
        match selected {
            Some(model) => {
                // A picker id carries its own platform. Keeping a legacy
                // separate provider here would describe the same route twice.
                member.provider = None;
                member.reasoning_effort = model.default_reasoning_effort.clone();
                member.model = Some(model.id.clone());
            }
            None => {
                member.provider = None;
                member.model = None;
                member.reasoning_effort = None;
            }
        }
        if let Some(editor) = self.agent_page.teams.roster_editor.as_mut() {
            editor.error = None;
        }
        cx.notify();
    }

    fn set_agent_team_roster_effort(
        &mut self,
        index: usize,
        effort: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(member) = self
            .agent_page
            .teams
            .roster_editor
            .as_mut()
            .and_then(|editor| editor.members.get_mut(index))
        {
            member.reasoning_effort = Some(effort);
        }
        cx.notify();
    }

    fn commit_agent_team_roster(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.agent_page.teams.roster_editor.as_ref() else {
            return;
        };
        let profile_name = editor.profile.clone();
        let Some(mut profile) = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map(TeamsConfig::effective_profiles)
            .and_then(|profiles| profiles.get(&profile_name).cloned())
        else {
            return;
        };
        profile.members = editor
            .members
            .iter()
            .map(|member| {
                let mut template = member.template.clone();
                template.name = member.name.read(cx).content().trim().to_owned();
                let role = member.role.read(cx).content().trim().to_owned();
                template.role = (!role.is_empty()).then_some(role);
                template.provider = member.provider.clone();
                template.model = member.model.clone();
                template.reasoning_effort = member.reasoning_effort.clone();
                template
            })
            .collect();
        let max_members = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map_or(8, |config| config.max_members() as usize);
        let checked = serde_json::to_value(&profile)
            .map_err(|error| error.to_string())
            .and_then(|value| {
                agent_teams::profiles::normalize_team_profile_value(
                    &profile_name,
                    &value,
                    max_members,
                )
                .map(|_| ())
            });
        match checked {
            Ok(()) => {
                self.agent_page.teams.roster_editor = None;
                self.edit_agent_team_profiles(cx, move |profiles| {
                    profiles.insert(profile_name, profile);
                });
            }
            Err(error) => {
                if let Some(editor) = self.agent_page.teams.roster_editor.as_mut() {
                    editor.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    pub(super) fn render_agent_team_roster_editor(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(editor) = self.agent_page.teams.roster_editor.as_ref() else {
            return div();
        };
        let models = Rc::new(
            self.provider_probe(ProviderKind::Native)
                .map(|probe| probe.models.clone())
                .unwrap_or_default(),
        );
        let saving = self.agent_page.teams.saving;
        let max_members = self
            .agent_page
            .teams
            .config
            .as_ref()
            .map_or(8, |config| config.max_members() as usize);
        let mut block = div()
            .mt(px(12.0))
            .p(px(12.0))
            .rounded(px(9.0))
            .bg(theme.overlay)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!(
                        "team.settings.roster_title",
                        name = editor.profile.clone()
                    )),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.roster_description")),
            );
        for (index, member) in editor.members.iter().enumerate() {
            block = block
                .child(self.render_agent_team_roster_member(index, member, &models, theme, cx));
        }
        if editor.members.is_empty() {
            block = block.child(
                div()
                    .py(px(8.0))
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.roster_empty")),
            );
        }
        if let Some(error) = editor.error.clone() {
            block = block.child(
                div()
                    .text_size(sp(12.0))
                    .line_height(sp(17.0))
                    .text_color(theme.warning)
                    .child(error),
            );
        }
        block.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .child(card_button(
                    theme,
                    SharedString::from("agent-teams-roster-add"),
                    tr!("team.settings.role_add"),
                    false,
                    saving || editor.members.len() >= max_members,
                    cx,
                    |this, window, cx| this.add_agent_team_roster_member(window, cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from("agent-teams-roster-save"),
                    tr!("common.save"),
                    true,
                    saving,
                    cx,
                    |this, _, cx| this.commit_agent_team_roster(cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from("agent-teams-roster-cancel"),
                    tr!("common.cancel"),
                    false,
                    saving,
                    cx,
                    |this, _, cx| {
                        this.agent_page.teams.roster_editor = None;
                        cx.notify();
                    },
                )),
        )
    }

    fn render_agent_team_roster_member(
        &self,
        index: usize,
        member: &RosterMemberEditor,
        models: &Rc<Vec<ProviderModel>>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let current = profile_member_picker_id(&ProfileMemberConfig {
            provider: member.provider.clone(),
            model: member.model.clone(),
            ..ProfileMemberConfig::default()
        });
        let current_label = current
            .as_ref()
            .map(|id| {
                models
                    .iter()
                    .find(|model| &model.id == id)
                    .map(|model| model.name.clone())
                    .unwrap_or_else(|| id.clone())
            })
            .unwrap_or_else(|| tr!("team.settings.role_model_inherit"));
        let model_id = SharedString::from(format!(
            "agent-teams-roster-{}-{index}-model",
            self.agent_page
                .teams
                .roster_editor
                .as_ref()
                .map(|editor| editor.profile.as_str())
                .unwrap_or_default()
        ));
        let weak = cx.entity().downgrade();
        let model_handle = self.menu_handle(model_id.clone(), cx);
        let model_trigger = picker_trigger(theme, model_id.clone(), current_label);
        let menu_models = Rc::clone(models);
        let selected_model = current.clone();
        let model_menu = dropdown_menu(
            model_trigger,
            model_id,
            &model_handle,
            MenuAlign::BelowLeft,
            move |_| {
                let mut items = Vec::new();
                let inherit = weak.clone();
                items.push(
                    MenuItem::new(tr!("team.settings.role_model_inherit"), move |_, cx| {
                        let _ = inherit.update(cx, |this, cx| {
                            this.set_agent_team_roster_route(index, None, cx)
                        });
                    })
                    .selected(selected_model.is_none()),
                );
                items.push(MenuItem::Separator);
                for model in menu_models.iter() {
                    let picked = model.clone();
                    let pick = weak.clone();
                    items.push(
                        MenuItem::new(model.name.clone(), move |_, cx| {
                            let picked = picked.clone();
                            let _ = pick.update(cx, |this, cx| {
                                this.set_agent_team_roster_route(index, Some(picked), cx)
                            });
                        })
                        .selected(selected_model.as_deref() == Some(model.id.as_str())),
                    );
                }
                items
            },
        );

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
        let effort = member.reasoning_effort.clone();
        let mut route_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .child(model_menu);
        if !efforts.is_empty() {
            let label = effort
                .as_ref()
                .filter(|id| id.as_str() != "default")
                .and_then(|id| efforts.iter().find(|(option, _)| option == id))
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| tr!("team.settings.member_effort_default"));
            let effort_id = SharedString::from(format!(
                "agent-teams-roster-{}-{index}-effort",
                self.agent_page
                    .teams
                    .roster_editor
                    .as_ref()
                    .map(|editor| editor.profile.as_str())
                    .unwrap_or_default()
            ));
            let effort_handle = self.menu_handle(effort_id.clone(), cx);
            let effort_trigger = picker_trigger(theme, effort_id.clone(), label);
            let weak = cx.entity().downgrade();
            let selected_effort = effort.clone();
            route_row = route_row.child(dropdown_menu(
                effort_trigger,
                effort_id,
                &effort_handle,
                MenuAlign::BelowLeft,
                move |_| {
                    let default = weak.clone();
                    let mut items = vec![
                        MenuItem::new(tr!("team.settings.member_effort_default"), move |_, cx| {
                            let _ = default.update(cx, |this, cx| {
                                this.set_agent_team_roster_effort(index, "default".to_owned(), cx)
                            });
                        })
                        .selected(
                            selected_effort.is_none()
                                || selected_effort.as_deref() == Some("default"),
                        ),
                    ];
                    for (id, label) in &efforts {
                        let id = id.clone();
                        let pick = weak.clone();
                        let selected = selected_effort.as_deref() == Some(id.as_str());
                        items.push(
                            MenuItem::new(label.clone(), move |_, cx| {
                                let id = id.clone();
                                let _ = pick.update(cx, |this, cx| {
                                    this.set_agent_team_roster_effort(index, id, cx)
                                });
                            })
                            .selected(selected),
                        );
                    }
                    items
                },
            ));
        }

        div()
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.raised)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(sp(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text_secondary)
                            .child(tr!("team.settings.role_position", index = index + 1)),
                    )
                    .child(card_button(
                        theme,
                        SharedString::from(format!("agent-teams-roster-remove-{index}")),
                        tr!("common.remove"),
                        false,
                        false,
                        cx,
                        move |this, _, cx| this.remove_agent_team_roster_member(index, cx),
                    )),
            )
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.role_name")),
            )
            .child(TextField::new(
                SharedString::from(format!("agent-teams-roster-name-{index}")),
                member.name.clone(),
            ))
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.role_description")),
            )
            .child(TextField::new(
                SharedString::from(format!("agent-teams-roster-role-{index}")),
                member.role.clone(),
            ))
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("team.settings.role_model")),
            )
            .child(route_row)
    }
}
