//! Host cancellation controls used by the original tool scheduler.
use sailry_protocol::TurnId;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct Controls;

impl Controls {
    pub fn tool_token(&self, _: TurnId) -> Option<CancellationToken> {
        None
    }
}
