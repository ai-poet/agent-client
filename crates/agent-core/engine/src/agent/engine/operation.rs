use super::{Effect, Session};
use crate::agent::model::Message;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Continuing,
    Yielded,
    Cancelled,
}

pub struct Transition {
    pub effects: Vec<Effect>,
    pub status: RunStatus,
}

pub enum Outcome {
    NotApplicable,
    Applied(Transition),
}

pub fn not_applicable() -> anyhow::Result<Outcome> {
    Ok(Outcome::NotApplicable)
}

pub fn applied(effects: impl IntoIterator<Item = Effect>) -> anyhow::Result<Outcome> {
    Ok(Outcome::Applied(Transition {
        effects: effects.into_iter().collect(),
        status: RunStatus::Continuing,
    }))
}

pub fn yielded() -> anyhow::Result<Outcome> {
    yielded_with([])
}

pub fn yielded_with(effects: impl IntoIterator<Item = Effect>) -> anyhow::Result<Outcome> {
    Ok(Outcome::Applied(Transition {
        effects: effects.into_iter().collect(),
        status: RunStatus::Yielded,
    }))
}

#[derive(Clone)]
pub struct Emitter {
    sender: mpsc::UnboundedSender<Message>,
    stop: CancellationToken,
}

impl Emitter {
    pub fn new(sender: mpsc::UnboundedSender<Message>, stop: CancellationToken) -> Self {
        Self { sender, stop }
    }

    pub fn message(&self, mut message: Message) -> Message {
        message.ensure_id();
        let _ = self.sender.send(message.clone());
        message
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.stop
    }

    pub async fn cancelled(&self) {
        self.stop.cancelled().await;
    }

    pub fn scoped(&self, stop: CancellationToken) -> Self {
        Self {
            sender: self.sender.clone(),
            stop,
        }
    }
}

#[async_trait]
pub trait Callback: Send + Sync {
    async fn run(
        &self,
        session: &Session,
        conversation: &[Message],
        emit: &Emitter,
    ) -> anyhow::Result<Outcome>;
}
