//! The line a sub-agent call leaves in the transcript.
//!
//! A sub-agent does its work in a context of its own; its steps are not this
//! conversation's. So its call is one summary line — "SubAgent", the kind of
//! agent, the task — that never expands in place, and opens the sub-agent's
//! live record in the right panel (`subagent_panel.rs`), the way ZCode opens
//! a child session beside the conversation. While the sub-agent runs, the
//! label shimmers like any running step; a failed or stopped one says so in
//! words.

use super::*;
use crate::model::SubagentCall;

/// Hues an agent kind is drawn in, picked by name so a kind keeps its colour
/// from row to row and session to session.
const AGENT_TYPE_HUES: [f32; 8] = [45.0, 0.0, 25.0, 140.0, 185.0, 215.0, 270.0, 325.0];

impl Waku {
    pub(super) fn render_subagent_row(
        &self,
        activity: &ActivityItem,
        call: &SubagentCall,
        live_turn: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = activity.id;
        let session_id = self.state.selected_session;
        let work = session_id
            .zip(activity.source_id.as_deref())
            .and_then(|(session_id, source_id)| {
                self.background_work_for_activity(session_id, source_id)
            });
        // Its own entry says when it is done: the call itself is answered
        // only once every call it was batched with is, so a sub-agent that
        // finished first would otherwise shimmer on until the slowest did.
        let running = match work {
            Some(work) => work.status.is_live(),
            None => live_turn && super::components::activity_is_running(activity),
        };
        let failed = activity.failed
            || work.is_some_and(|work| work.status == BackgroundWorkStatus::Failed);
        let stopped = !failed
            && (activity.stopped
                || work.is_some_and(|work| work.status == BackgroundWorkStatus::Stopped));
        let can_open = session_id.is_some() && (work.is_some() || activity.complete || activity.stopped);

        let label = tr!("subagent.label");
        let label = if running {
            motion::shimmer(label, theme.text_tertiary, theme.text)
                .weight(FontWeight::MEDIUM)
                .into_any_element()
        } else {
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_secondary)
                .child(label)
                .into_any_element()
        };
        let hover_group = SharedString::from(format!("subagent-row-{id}"));
        let focus = self.transcript_control_focus(format!("subagent-row-{id}"), cx);
        let description = call.description.clone();
        let open_activity = activity.clone();
        let key_activity = activity.clone();

        div()
            .id(hover_group.clone())
            .group(hover_group.clone())
            .w_full()
            .min_w_0()
            .h(px(26.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(sp(12.5))
            .line_height(sp(16.0))
            .when(can_open, |row| {
                row.track_focus(&focus)
                    .tab_index(0)
                    .cursor_default()
                    .focus_visible(|row| row.text_color(theme.text))
            })
            .child(icon("icons/bot.svg", 14.0, theme.text_tertiary))
            .child(div().flex_none().child(label))
            .when_some(call.agent_type.clone(), |row, agent_type| {
                row.child(
                    div()
                        .flex_none()
                        .max_w(px(140.0))
                        .truncate()
                        .font_family(md::render::MONO_FAMILY)
                        .text_color(agent_type_color(&agent_type, theme))
                        .child(agent_type),
                )
            })
            .child(div().flex_none().text_color(theme.text_ghost).child("·"))
            .child(
                div()
                    .id(SharedString::from(format!("subagent-row-task-{id}")))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text_tertiary)
                    .when(can_open, |target| {
                        target.group_hover(hover_group.clone(), |style| {
                            style.text_color(theme.text_secondary)
                        })
                    })
                    .tooltip(Tooltip::text(description.clone()))
                    .child(SharedString::from(single_line_label(&description))),
            )
            .when(failed, |row| {
                row.child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.danger)
                        .child(tr!("activity.status_failed")),
                )
            })
            .when(stopped, |row| {
                row.child(
                    div()
                        .flex_none()
                        .text_color(theme.text_ghost)
                        .child(tr!("activity.status_stopped")),
                )
            })
            .when(can_open, |row| {
                row.child(
                    div()
                        .flex_none()
                        .invisible()
                        .group_hover(hover_group.clone(), |style| style.visible())
                        .child(icon("icons/chevron-right.svg", 12.0, theme.text_tertiary)),
                )
                .tooltip(Tooltip::text(tr!("subagent.open")))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if can_open && let Some(session_id) = session_id {
                    this.open_subagent_activity(session_id, &open_activity, cx);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if can_open
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    && let Some(session_id) = session_id
                {
                    this.open_subagent_activity(session_id, &key_activity, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }

    /// Open the record of the sub-agent `activity` started. A call whose
    /// sub-agent left no entry — a session restored after a restart, whose
    /// records were not kept — opens one made from the call itself: its
    /// task, its prompt and its answer.
    pub(super) fn open_subagent_activity(
        &mut self,
        session_id: Uuid,
        activity: &ActivityItem,
        cx: &mut Context<Self>,
    ) {
        let existing = activity
            .source_id
            .as_deref()
            .and_then(|source_id| self.background_work_for_activity(session_id, source_id))
            .map(|work| work.key.clone());
        if let Some(key) = existing {
            self.open_background_work_surface(session_id, key, cx);
            return;
        }
        let Some(item) = settled_item_from_call(activity) else {
            return;
        };
        let key = item.key.clone();
        self.handle_background_work_event(session_id, BackgroundWorkEvent::Upsert(item));
        self.open_background_work_surface(session_id, key, cx);
    }
}

/// The entry a finished sub-agent call stands for, when nothing else left
/// one.
fn settled_item_from_call(activity: &ActivityItem) -> Option<BackgroundWorkItem> {
    let call = activity.subagent.as_ref()?;
    let provider_id = activity
        .source_id
        .clone()
        .unwrap_or_else(|| activity.id.to_string());
    let status = if activity.failed {
        BackgroundWorkStatus::Failed
    } else if activity.stopped || !activity.complete {
        BackgroundWorkStatus::Stopped
    } else {
        BackgroundWorkStatus::Completed
    };
    let mut item = BackgroundWorkItem::new(
        BackgroundWorkKind::Subagent,
        provider_id.clone(),
        call.description.clone(),
        status,
    );
    item.origin_activity_id = Some(provider_id);
    item.role = call.agent_type.clone();
    item.command = activity
        .arguments
        .as_deref()
        .and_then(|arguments| serde_json::from_str::<serde_json::Value>(arguments).ok())
        .and_then(|arguments| {
            arguments
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    item.output = activity.output.clone();
    // No clock survives for it; zero reads as "no duration" rather than as
    // the time since 1970.
    item.duration_ms = Some(0);
    Some(item)
}

/// The colour an agent kind's name is drawn in: one of eight hues, by an
/// FNV-1a hash of the lowercased name, at a lightness that reads on the
/// current theme. The name is always there as text, so colour only helps
/// tell kinds apart at a glance; it never carries meaning alone.
pub(super) fn agent_type_color(agent_type: &str, theme: &Theme) -> Hsla {
    let hue = AGENT_TYPE_HUES[agent_type_hue_index(agent_type)];
    let lightness = if theme.is_dark { 0.72 } else { 0.40 };
    gpui::hsla(hue / 360.0, 0.62, lightness, 1.0)
}

fn agent_type_hue_index(agent_type: &str) -> usize {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in agent_type.trim().to_lowercase().bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash as usize % AGENT_TYPE_HUES.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ActivityKind;

    /// A kind keeps its colour whatever its case, and the index is always a
    /// hue that exists.
    #[test]
    fn an_agent_kind_keeps_its_colour() {
        assert_eq!(agent_type_hue_index("Explore"), agent_type_hue_index("explore "));
        for name in ["general-purpose", "Explore", "Plan", "", "代码审查"] {
            assert!(agent_type_hue_index(name) < AGENT_TYPE_HUES.len());
        }
    }

    /// With nothing else left of it, a finished call still opens onto its
    /// task, its prompt and its answer.
    #[test]
    fn a_finished_call_stands_for_its_own_entry() {
        let activity = ActivityItem::new(Some("toolu_1".into()), ActivityKind::Tool, "Find it", None, true)
            .with_arguments(Some(r#"{"description":"Find it","prompt":"Look for it"}"#.into()))
            .with_output(Some("It is in auth.rs".into()))
            .with_subagent(Some(SubagentCall {
                agent_type: Some("Explore".into()),
                description: "Find it".into(),
            }));
        let item = settled_item_from_call(&activity).unwrap();
        assert_eq!(item.key, BackgroundWorkKey::new(BackgroundWorkKind::Subagent, "toolu_1"));
        assert_eq!(item.status, BackgroundWorkStatus::Completed);
        assert_eq!(item.command.as_deref(), Some("Look for it"));
        assert_eq!(item.output.as_deref(), Some("It is in auth.rs"));
        assert_eq!(item.role.as_deref(), Some("Explore"));

        let plain = ActivityItem::new(None, ActivityKind::Tool, "Bash", None, true);
        assert!(settled_item_from_call(&plain).is_none());
    }
}
