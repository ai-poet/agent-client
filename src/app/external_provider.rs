//! Built-in-only provider policy for legacy external CLI state.
//!
//! The daemon still rejects external providers as a final safety boundary. The
//! app intercepts the same actions earlier, keeps old transcripts readable, and
//! routes new work to the built-in agent without exposing internal bail text.

use super::*;

const EXTERNAL_CLI_ERRORS: &[&str] = &[
    "external agent CLIs are not supported",
    "external CLI sessions cannot be restored by this build",
    "external CLI session forks are not supported",
    "external provider plan usage is not supported",
];

/// The one localized message for every external-CLI refusal, including when a
/// higher-level operation has wrapped the daemon's original error.
pub(super) fn external_cli_error_key(message: &str) -> Option<&'static str> {
    EXTERNAL_CLI_ERRORS
        .iter()
        .any(|error| message.contains(error))
        .then_some("errors.external_cli_unavailable")
}

pub(super) fn localized_external_cli_error(message: &str) -> Option<String> {
    external_cli_error_key(message).map(|_| tr!("errors.external_cli_unavailable"))
}

impl Waku {
    /// A remembered built-in choice wins; otherwise the catalog's own default.
    /// A remembered id remains useful while the catalog is still loading.
    fn preferred_builtin_model(&self) -> Option<String> {
        if self.state.last_provider == ProviderKind::Native
            && let Some(model) = self
                .state
                .last_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
        {
            return Some(model.to_owned());
        }
        self.provider_probe(ProviderKind::Native)
            .and_then(ProviderProbe::preferred_model)
            .map(|model| model.id.clone())
    }

    /// Move an unstarted legacy draft to the built-in agent. Started sessions
    /// are deliberately untouched: their provider is part of their history.
    /// Returns false for native sessions, started history, and missing tasks.
    pub(super) fn redirect_unstarted_session_to_builtin(
        &mut self,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_none_or(|session| session.provider.is_builtin() || session.has_started())
        {
            return false;
        }

        let model = self.preferred_builtin_model();
        let (reasoning_effort, mut service_tier, context_window) = model
            .as_deref()
            .map(|model| self.state.model_traits_for(ProviderKind::Native, model))
            .unwrap_or_default();
        if let Some(model) = model.as_deref() {
            let declared = self
                .probe_model(ProviderKind::Native, model)
                .and_then(|row| row.default_service_tier.clone());
            service_tier = Some(super::composer::native_wire_format(
                declared.as_deref().or(service_tier.as_deref()),
                Some(model),
            ));
        }
        if let Some(session) = self.state.session_mut(session_id) {
            session.provider = ProviderKind::Native;
            session.model = model.clone();
            session.reasoning_effort = reasoning_effort.clone();
            session.service_tier = service_tier.clone();
            session.context_window = context_window.clone();
            session.provider_cursor = None;
            session.agent_preset = None;
        }
        self.state.last_provider = ProviderKind::Native;
        self.state.last_model = model;
        self.state.last_reasoning_effort = reasoning_effort;
        self.state.last_service_tier = service_tier;
        self.state.last_context_window = context_window;
        self.save();
        cx.notify();
        true
    }

    pub(super) fn show_external_cli_notice(&mut self) {
        self.show_toast(tr!("errors.external_cli_unavailable"));
    }

    /// Open the built-in model list without toggling it shut when a repeated
    /// legacy action asks for it again.
    pub(super) fn open_native_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model_picker_tab = ModelPickerTab::Provider(ProviderKind::Native);
        let handle = self.menus.borrow().get(MODEL_PICKER_MENU_ID).cloned();
        let Some(handle) = handle else {
            return;
        };
        if handle.is_open() {
            self.sync_model_picker_vendor();
            cx.notify();
            return;
        }
        window.defer(cx, move |window, cx| {
            crate::ui::menu::toggle_popover(&handle, MenuAlign::AboveLeft, window, cx);
        });
    }

    pub(super) fn show_external_cli_notice_and_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_external_cli_notice();
        self.open_native_model_picker(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::external_cli_error_key;

    #[test]
    fn maps_every_external_cli_refusal_to_the_friendly_message() {
        for error in [
            "external agent CLIs are not supported",
            "external CLI sessions cannot be restored by this build",
            "external CLI session forks are not supported",
            "external provider plan usage is not supported",
        ] {
            assert_eq!(
                external_cli_error_key(error),
                Some("errors.external_cli_unavailable")
            );
            assert_eq!(
                external_cli_error_key(&format!("Could not fork the task: {error}")),
                Some("errors.external_cli_unavailable")
            );
        }
    }

    #[test]
    fn leaves_unrelated_errors_alone() {
        assert_eq!(external_cli_error_key("the daemon is disconnected"), None);
        assert_eq!(external_cli_error_key("external CLI"), None);
    }
}
