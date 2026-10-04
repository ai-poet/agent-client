//! Fork addition: carrying an effort-only change to a running ACP agent.
//!
//! The options handler used to re-apply only when the model changed (Grok
//! also on an effort change), so picking another level for a running Kimi
//! session looked accepted and changed nothing until the model did. This
//! decides what an options update has to send, per agent, and sends the
//! effort on its own where the agent takes it as a standalone setting.

use agent_client_protocol::schema::v1::{SessionId, SetSessionConfigOptionRequest};
use agent_client_protocol::{Agent, ConnectionTo};

use super::reasoning_effort_config_id;
use crate::model::ProviderKind;

/// What an options update has to send to the running agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Reapply {
    Nothing,
    /// Select the model again, carrying the effort with it.
    Model,
    /// Send only the effort setting.
    Effort,
}

/// Decide what an options update sends.
///
/// * A new model always goes through the model setter, which carries the
///   effort for every agent.
/// * Grok's effort rides `set_model`'s `_meta`, so it has no other way in.
/// * Cursor's thought level is one of the per-model options the agent only
///   returns after the model is selected, so it is re-selected too.
/// * Kimi takes its effort as the standalone `thinking` config option — the
///   same request `apply_model` ends with — so that alone is sent. A cleared
///   effort has nothing to send.
/// * Every other agent keeps the behaviour it had: effort changes alone are
///   not forwarded.
pub(super) fn reapply(
    provider: ProviderKind,
    model_changed: bool,
    effort_changed: bool,
    effort_present: bool,
) -> Reapply {
    if model_changed {
        return Reapply::Model;
    }
    if !effort_changed {
        return Reapply::Nothing;
    }
    match provider {
        ProviderKind::Grok | ProviderKind::Cursor => Reapply::Model,
        ProviderKind::Kimi if effort_present => Reapply::Effort,
        _ => Reapply::Nothing,
    }
}

/// Send only the effort setting. Returns whether the agent accepted it, so a
/// refused change is tried again with the next update.
pub(super) async fn apply_effort(
    connection: &ConnectionTo<Agent>,
    provider: ProviderKind,
    session_id: &SessionId,
    effort: &str,
) -> bool {
    connection
        .send_request(SetSessionConfigOptionRequest::new(
            session_id.clone(),
            reasoning_effort_config_id(provider),
            effort,
        ))
        .block_task()
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_model_is_always_selected_again() {
        for provider in [
            ProviderKind::Grok,
            ProviderKind::Kimi,
            ProviderKind::Cursor,
            ProviderKind::OpenCode,
        ] {
            assert_eq!(reapply(provider, true, false, true), Reapply::Model);
            assert_eq!(reapply(provider, true, true, false), Reapply::Model);
        }
    }

    #[test]
    fn nothing_changed_sends_nothing() {
        for provider in [ProviderKind::Grok, ProviderKind::Kimi, ProviderKind::Cursor] {
            assert_eq!(reapply(provider, false, false, true), Reapply::Nothing);
        }
    }

    #[test]
    fn an_effort_only_change_reaches_each_agent_its_own_way() {
        assert_eq!(reapply(ProviderKind::Grok, false, true, true), Reapply::Model);
        assert_eq!(
            reapply(ProviderKind::Cursor, false, true, true),
            Reapply::Model
        );
        assert_eq!(reapply(ProviderKind::Kimi, false, true, true), Reapply::Effort);
        assert_eq!(
            reapply(ProviderKind::OpenCode, false, true, true),
            Reapply::Nothing
        );
    }

    #[test]
    fn a_cleared_kimi_effort_has_nothing_to_send() {
        assert_eq!(
            reapply(ProviderKind::Kimi, false, true, false),
            Reapply::Nothing
        );
    }
}
