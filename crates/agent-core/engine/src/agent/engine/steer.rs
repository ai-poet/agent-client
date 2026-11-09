//! Sailry's steer stage with host-owned queued input.
use super::{Effect, Session, conversation::*, operation::*};
use crate::agent::model::Message;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct SteerQueue(Arc<Mutex<VecDeque<String>>>);

impl SteerQueue {
    pub fn push(&self, message: impl Into<String>) {
        self.0.lock().unwrap().push_back(message.into());
    }

    fn drain(&self) -> Vec<String> {
        self.0.lock().unwrap().drain(..).collect()
    }
}

pub struct Steer {
    pub queue: SteerQueue,
}

impl Steer {
    pub fn finalize_cancellation(&self) -> Vec<Effect> {
        Vec::new()
    }

    pub async fn run(
        &self,
        _session: &Session,
        conversation: &[Message],
        emit: &Emitter,
    ) -> anyhow::Result<Outcome> {
        let messages = messages_since_kickoff(conversation)?;
        if !ends_turn(messages) && last_effective_role(messages)? != EffectiveRole::Tool {
            return not_applicable();
        }
        let inputs = self.queue.drain();
        if inputs.is_empty() {
            return not_applicable();
        }
        applied(
            inputs
                .into_iter()
                .map(|input| emit.message(Message::user().with_text(input)).into()),
        )
    }
}
