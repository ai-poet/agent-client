//! Keyboard shortcuts for the composer's mode chips.
//!
//! Fork addition. The access mode, plan mode and reasoning effort each sat
//! behind a click; these step through them from the keyboard, anywhere in
//! the window, and say where they landed.

use gpui::{KeyBinding, actions};

use super::*;

actions!(
    waku_shortcuts,
    [CycleAccessMode, TogglePlanMode, CycleReasoningEffort]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-shift-m", CycleAccessMode, Some("Waku")),
        KeyBinding::new("secondary-shift-p", TogglePlanMode, Some("Waku")),
        KeyBinding::new("secondary-t", CycleReasoningEffort, Some("Waku")),
    ]);
}

/// The option after `current` in `options`, wrapping around; the first when
/// `current` is not among them.
pub(super) fn next_in_cycle<T: PartialEq + Clone>(options: &[T], current: Option<&T>) -> Option<T> {
    let next = current
        .and_then(|current| options.iter().position(|option| option == current))
        .map_or(0, |index| (index + 1) % options.len().max(1));
    options.get(next).cloned()
}

impl Waku {
    pub(super) fn cycle_access_mode_action(
        &mut self,
        _: &CycleAccessMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self
            .selected_session()
            .map(|session| session.runtime_mode)
            .filter(|mode| *mode != RuntimeMode::Plan)
        else {
            return;
        };
        let Some(next) = next_in_cycle(&RuntimeMode::ACCESS_OPTIONS, Some(&current)) else {
            return;
        };
        self.set_runtime_mode(next, cx);
        self.show_toast(tr!("shortcuts.access_now", mode = next.label()));
        cx.notify();
    }

    pub(super) fn toggle_plan_mode_action(
        &mut self,
        _: &TogglePlanMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let mode = session.interaction_mode;
        // The same gate as the chip: back to Build always, into Plan only
        // where the agent can plan.
        let supports_plan = session.provider != ProviderKind::Fx
            && (session.provider != ProviderKind::DeepSeek
                || self.agent_preset_for_session(session).as_deref() != Some("minimal"));
        let next = if mode == InteractionMode::Plan {
            InteractionMode::Build
        } else if supports_plan {
            InteractionMode::Plan
        } else {
            self.show_toast(tr!("mode.plan_not_supported"));
            cx.notify();
            return;
        };
        self.set_interaction_mode(next, cx);
        self.show_toast(tr!("shortcuts.mode_now", mode = next.label()));
        cx.notify();
    }

    pub(super) fn cycle_reasoning_effort_action(
        &mut self,
        _: &CycleReasoningEffort,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let Some(model) = self.model_metadata_for_session(session) else {
            return;
        };
        let options = model
            .reasoning_efforts
            .iter()
            .map(|option| (option.id.clone(), option.label.clone()))
            .collect::<Vec<_>>();
        let current = session
            .reasoning_effort
            .clone()
            .or_else(|| model.default_reasoning_effort.clone());
        let ids = options.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let Some(next) = next_in_cycle(&ids, current.as_ref()) else {
            return;
        };
        let label = options
            .iter()
            .find(|(id, _)| *id == next)
            .map_or_else(|| next.clone(), |(_, label)| label.clone());
        self.set_reasoning_effort(next, cx);
        self.show_toast(tr!("shortcuts.effort_now", effort = label));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_option_wraps_around() {
        let options = ["low", "medium", "high"];
        assert_eq!(next_in_cycle(&options, Some(&"low")), Some("medium"));
        assert_eq!(next_in_cycle(&options, Some(&"high")), Some("low"));
        assert_eq!(next_in_cycle(&options, Some(&"max")), Some("low"));
        assert_eq!(next_in_cycle(&options, None), Some("low"));
        assert_eq!(next_in_cycle::<&str>(&[], None), None);
    }
}
