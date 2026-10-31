//! Fork: Fusion pairing state and the built-in model picker's pair editor.
//!
//! The lead stays in `AgentSession::{model, reasoning_effort, service_tier}`.
//! Only the sidekick needs a second selection. The execution half lives in
//! `waku-agent-bridge::fusion`; this file owns presentation and persistence.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FusionPickerRole {
    Lead,
    Sidekick,
}

#[derive(Clone)]
enum FusionAction {
    Role(FusionPickerRole),
    CycleEffort,
    Apply,
    Back,
    Disable,
}

impl Waku {
    pub(super) fn fusion_tab_selected(&self) -> bool {
        self.model_picker_tab == ModelPickerTab::Provider(ProviderKind::Native)
            && self
                .selected_session()
                .is_some_and(|session| session.can_choose_model(ProviderKind::Native))
    }

    pub(super) fn fusion_summary(&self, session: &AgentSession) -> Option<String> {
        let pair = session.fusion.as_ref()?;
        let lead = self.model_display_name(ProviderKind::Native, self.model_for_session(session));
        let sidekick =
            self.model_display_name(ProviderKind::Native, Some(pair.sidekick_model.as_str()));
        Some(format!("Fusion · {lead} ◆ {sidekick}"))
    }

    fn recommended_sidekick(&self) -> Option<crate::model::FusionPair> {
        let lead = self
            .selected_session()
            .and_then(|session| self.model_for_session(session))
            .map(str::to_owned);
        let models = self.provider_probe(ProviderKind::Native)?.models.clone();
        let preferred = ["luna", "haiku", "flash", "mini", "deepseek", "glm"];
        let id = preferred
            .iter()
            .find_map(|needle| {
                models
                    .iter()
                    .find(|model| {
                        Some(model.id.as_str()) != lead.as_deref()
                            && model.id.to_ascii_lowercase().contains(needle)
                    })
                    .map(|model| model.id.clone())
            })
            .or_else(|| {
                models
                    .iter()
                    .find(|model| Some(model.id.as_str()) != lead.as_deref())
                    .map(|model| model.id.clone())
            })?;
        Some(self.sidekick_pair_for(id))
    }

    fn sidekick_pair_for(&self, model: String) -> crate::model::FusionPair {
        let (effort, mut service_tier, _) =
            self.state.model_traits_for(ProviderKind::Native, &model);
        let declared = sub2api::providers::parse_agent_model_id(&model)
            .and_then(|_| self.probe_model(ProviderKind::Native, &model))
            .and_then(|row| row.default_service_tier.clone());
        service_tier = Some(super::composer::native_wire_format(
            declared.as_deref().or(service_tier.as_deref()),
            Some(model.as_str()),
        ));
        crate::model::FusionPair {
            sidekick_model: model,
            sidekick_reasoning_effort: effort,
            sidekick_service_tier: service_tier,
        }
    }

    pub(super) fn open_fusion_picker(&mut self, cx: &mut Context<Self>) {
        self.model_picker_tab = ModelPickerTab::Provider(ProviderKind::Native);
        self.fusion_picker_open = true;
        self.fusion_picker_role = if self
            .selected_session()
            .is_some_and(|session| session.provider.is_builtin() && session.model.is_some())
        {
            FusionPickerRole::Sidekick
        } else {
            FusionPickerRole::Lead
        };
        if self.fusion_draft.is_none() {
            self.fusion_draft = self
                .selected_session()
                .and_then(|session| session.fusion.clone())
                .or_else(|| self.recommended_sidekick());
        }
        self.model_picker_highlight = None;
        self.refresh_native_catalog(false, cx);
        self.sync_model_picker_vendor();
        cx.notify();
    }

    pub(super) fn choose_fusion_model(
        &mut self,
        kind: ProviderKind,
        model: String,
        cx: &mut Context<Self>,
    ) {
        if !kind.is_builtin() {
            self.fusion_picker_open = false;
            self.choose_model(kind, model, cx);
            return;
        }
        match self.fusion_picker_role {
            FusionPickerRole::Lead => {
                self.choose_model(kind, model, cx);
                if self.fusion_draft.is_none() {
                    self.fusion_draft = self.recommended_sidekick();
                }
            }
            FusionPickerRole::Sidekick => {
                self.fusion_draft = Some(self.sidekick_pair_for(model));
                self.ensure_native_model_route_for_selection(cx);
            }
        }
        self.model_picker_highlight = None;
        cx.notify();
    }

    fn ensure_native_model_route_for_selection(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let session_id = session.id;
        // `native_route_needed` also considers the draft sidekick below, so a
        // route can be prepared before the pair is applied.
        self.ensure_native_model_route(session_id, cx);
    }

    pub(super) fn apply_fusion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pair) = self.fusion_draft.clone() else {
            self.show_toast(tr!("fusion.choose_sidekick"));
            return false;
        };
        let Some((session_id, changed)) = self.selected_session().and_then(|session| {
            (session.provider.is_builtin() && session.model.is_some())
                .then_some((session.id, session.fusion.as_ref() != Some(&pair)))
        }) else {
            self.show_toast(tr!("fusion.choose_lead"));
            return false;
        };
        if let Some(session) = self.selected_session_mut() {
            session.fusion = Some(pair);
            if changed {
                self.reset_session_runtime(session_id);
            }
            self.save();
            self.ensure_native_model_route(session_id, cx);
            self.fusion_picker_open = false;
            self.show_success_toast(tr!("fusion.enabled"));
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn clear_fusion(&mut self, cx: &mut Context<Self>) {
        let Some((session_id, was_enabled)) = self
            .selected_session()
            .map(|session| (session.id, session.fusion.is_some()))
        else {
            return;
        };
        if let Some(session) = self.selected_session_mut() {
            session.fusion = None;
            if was_enabled {
                self.reset_session_runtime(session_id);
                self.save();
            }
        }
        self.fusion_picker_open = false;
        cx.notify();
    }

    fn cycle_fusion_lead_effort(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let Some(model) = self.model_metadata_for_session(session) else {
            return;
        };
        if model.reasoning_efforts.is_empty() {
            return;
        }
        let current = session.reasoning_effort.as_deref();
        let next = model
            .reasoning_efforts
            .iter()
            .position(|option| Some(option.id.as_str()) == current)
            .map(|index| (index + 1) % model.reasoning_efforts.len())
            .unwrap_or(0);
        self.set_reasoning_effort(model.reasoning_efforts[next].id.clone(), cx);
    }

    fn execute_fusion_action(
        &mut self,
        action: FusionAction,
        menu: &ContextMenuHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            FusionAction::Role(role) => {
                self.fusion_picker_role = role;
                self.model_picker_highlight = None;
            }
            FusionAction::CycleEffort => self.cycle_fusion_lead_effort(cx),
            FusionAction::Apply => {
                if self.apply_fusion(cx) {
                    menu.close(window, cx);
                }
            }
            FusionAction::Back => self.fusion_picker_open = false,
            FusionAction::Disable => self.clear_fusion(cx),
        }
        cx.notify();
    }

    fn fusion_action_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        selected: bool,
        action: FusionAction,
        menu: &ContextMenuHandle,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let focus = self.transcript_control_focus(format!("fusion-{id}"), cx);
        let weak = cx.entity().downgrade();
        let click_action = action.clone();
        let click_menu = menu.clone();
        let key_action = action;
        let key_menu = menu.clone();
        let key_weak = weak.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(true)
            .h(px(32.0))
            .px(px(9.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(if selected { theme.accent } else { theme.border })
            .bg(if selected {
                theme.overlay_strong
            } else {
                theme.surface
            })
            .hover(|style| style.bg(theme.overlay))
            .focus_visible(|style| style.border_color(theme.accent))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .child(
                div()
                    .truncate()
                    .text_size(sp(12.0))
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(theme.text)
                    .child(label.into()),
            )
            .on_click(move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.execute_fusion_action(click_action.clone(), &click_menu, window, cx);
                });
            })
            .on_key_down(move |event, window, cx| {
                if !matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    return;
                }
                let _ = key_weak.update(cx, |this, cx| {
                    this.execute_fusion_action(key_action.clone(), &key_menu, window, cx);
                });
                cx.stop_propagation();
            })
    }

    /// The Fusion family row at the top of the built-in agent's model list.
    pub(super) fn render_fusion_entry(
        &self,
        highlighted: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let selected = self.fusion_picker_open
            || self
                .selected_session()
                .is_some_and(|session| session.fusion.is_some());
        let weak = cx.entity().downgrade();
        div()
            .id("model-row-native-fusion")
            .h(px(58.0))
            .px(px(12.0))
            .rounded(px(9.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_default()
            .border_1()
            .border_color(if highlighted {
                theme.accent
            } else {
                gpui::transparent_black()
            })
            .bg(if selected {
                theme.overlay_strong
            } else {
                theme.surface
            })
            .hover(|style| style.bg(theme.overlay))
            .child(icon("icons/sparkle.svg", 16.0, theme.accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Fusion"),
                    )
                    .child(
                        div()
                            .mt(px(3.0))
                            .truncate()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .child(tr!("fusion.description")),
                    ),
            )
            .child(icon("icons/chevron-right.svg", 13.0, theme.text_tertiary))
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.open_fusion_picker(cx));
            })
    }

    /// Pair controls above the same model list. Choosing a role changes what
    /// the rows below mean, so model search and keyboard navigation stay the
    /// existing picker's rather than a second implementation.
    pub(super) fn render_fusion_pair_header(
        &self,
        menu: &ContextMenuHandle,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let session = self.selected_session();
        let lead = session
            .filter(|session| session.provider.is_builtin())
            .and_then(|session| self.model_for_session(session))
            .map(|model| self.model_display_name(ProviderKind::Native, Some(model)))
            .unwrap_or_else(|| tr!("fusion.choose_lead"));
        let sidekick = self
            .fusion_draft
            .as_ref()
            .map(|pair| {
                self.model_display_name(ProviderKind::Native, Some(pair.sidekick_model.as_str()))
            })
            .unwrap_or_else(|| tr!("fusion.choose_sidekick"));
        let effort = session
            .and_then(|session| session.reasoning_effort.clone())
            .unwrap_or_else(|| tr!("models.standard"));
        let active = session.is_some_and(|session| session.fusion.is_some());

        div()
            .mx(px(9.0))
            .mb(px(7.0))
            .p(px(9.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.canvas)
            .child(
                div()
                    .mb(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Fusion"),
                    )
                    .child(self.fusion_action_button(
                        "fusion-back",
                        tr!("fusion.back"),
                        false,
                        FusionAction::Back,
                        menu,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .child(div().flex_1().min_w_0().child(self.fusion_action_button(
                        "fusion-lead",
                        format!("{} · {lead}", tr!("fusion.lead")),
                        self.fusion_picker_role == FusionPickerRole::Lead,
                        FusionAction::Role(FusionPickerRole::Lead),
                        menu,
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().child(self.fusion_action_button(
                        "fusion-effort",
                        format!("{} · {effort}", tr!("fusion.effort")),
                        false,
                        FusionAction::CycleEffort,
                        menu,
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().child(self.fusion_action_button(
                        "fusion-sidekick",
                        format!("{} · {sidekick}", tr!("fusion.sidekick")),
                        self.fusion_picker_role == FusionPickerRole::Sidekick,
                        FusionAction::Role(FusionPickerRole::Sidekick),
                        menu,
                        cx,
                    ))),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!("fusion.pair_hint")),
                    )
                    .when(active, |row| {
                        row.child(self.fusion_action_button(
                            "fusion-disable",
                            tr!("fusion.disable"),
                            false,
                            FusionAction::Disable,
                            menu,
                            cx,
                        ))
                    })
                    .child(self.fusion_action_button(
                        "fusion-apply",
                        tr!("fusion.apply"),
                        true,
                        FusionAction::Apply,
                        menu,
                        cx,
                    )),
            )
    }

    /// `/fusion` opens the pair editor; `/model` returns to a single-model
    /// picker. Both commands leave the draft text untouched beyond the command.
    pub(super) fn execute_fusion_command(&mut self, prompt: &str, cx: &mut Context<Self>) -> bool {
        let command = prompt.trim();
        if command != "/fusion" && command != "/model" {
            return false;
        }
        if !self.selected_session().is_some_and(|session| {
            session.provider.is_builtin() && session.can_choose_model(session.provider)
        }) {
            return false;
        }
        self.composer.update(cx, |input, cx| input.clear(cx));
        if command == "/fusion" {
            self.open_fusion_picker(cx);
        } else {
            self.clear_fusion(cx);
        }
        cx.defer(|cx| cx.dispatch_action(&ToggleModelPicker));
        true
    }
}
