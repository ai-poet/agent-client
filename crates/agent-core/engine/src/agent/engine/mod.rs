//! Sailry's Agent state machine with an in-memory embedding host.
//! The stage order and operations are copied from Sailry's engine/mod.rs.
//! Node queue admission, SQL reloads, and remote child supervision are host concerns.

mod compact;
pub mod conversation;
mod effects;
pub mod inference;
pub mod operation;
mod operations;
pub mod prompt;
pub mod steer;
pub mod tokens;
pub mod tools;

pub use effects::{ConversationEffect, Effect};
pub use operation::{Callback, Emitter, Outcome, RunStatus};
pub use steer::SteerQueue;

use crate::agent::{catalog, controls::Controls, model};
use crate::store::agent::{Invocation, RuntimeSession as Session, Sessions};
use sailry_protocol::{ErrorCode, Fault};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub enum EngineEvent {
    Message(model::Message),
    Checkpoint(Session),
}

pub trait EngineEvents: Send + Sync {
    fn emit(&self, event: EngineEvent);
}

impl<F> EngineEvents for F
where
    F: Fn(EngineEvent) + Send + Sync,
{
    fn emit(&self, event: EngineEvent) {
        self(event);
    }
}

pub struct Run {
    pub sessions: Sessions,
    pub invocation: Arc<Invocation>,
    pub stop: CancellationToken,
    pub binding: model::Binding,
    pub summary: model::Binding,
    pub instruction: String,
    pub context: String,
    pub parts: Vec<String>,
    pub catalog: Arc<dyn catalog::ToolProvider<Session>>,
    pub callbacks: Vec<Arc<dyn Callback>>,
    pub deferred: BTreeSet<String>,
    pub context_limit: usize,
    pub max_turns: Option<u32>,
    pub steering: SteerQueue,
}

pub struct RunOutcome {
    pub status: RunStatus,
    pub output: Option<String>,
    pub error: Option<model::Error>,
}

pub async fn run(run: Run, events: &dyn EngineEvents) -> Result<RunOutcome, Fault> {
    let Run {
        sessions,
        invocation,
        stop,
        mut binding,
        mut summary,
        instruction,
        context: turn_context,
        parts,
        catalog,
        callbacks,
        deferred,
        context_limit,
        max_turns,
        steering,
    } = run;
    binding.provider = model::request::wrap(
        binding.provider,
        model::request::Purpose::Main,
        stop.clone(),
        stop.clone(),
    );
    summary.provider = model::request::wrap(
        summary.provider,
        model::request::Purpose::Compaction,
        stop.clone(),
        stop.clone(),
    );
    let preparer = Arc::new(prompt::Preparer {
        stop: stop.clone(),
        deferred,
        output: Default::default(),
    });
    let manual = invocation.turn.kind == sailry_protocol::conversation::RunKind::Compaction;
    let threshold = if context_limit == 0 {
        0.0
    } else {
        f64::from(invocation.compaction_percent) / 100.0
    };
    let engine = Engine {
        manual,
        compact: compact::Compaction {
            provider: summary.provider,
            config: summary.config,
            context: context_limit,
            threshold,
            preparer: preparer.clone(),
            manual,
        },
        approval: operations::Approval {
            required: BTreeSet::new(),
            bindings: BTreeMap::new(),
        },
        prompt: prompt::Prompt {
            instruction,
            context: turn_context,
            parts,
        },
        tools: tools::Tools::new(catalog, Arc::new(Controls), invocation.turn.id),
        remaining: operations::Remaining { timeline: None },
        steer: steer::Steer { queue: steering },
        max_turns: max_turns.map(operations::MaxTurns),
        callbacks,
        inference: inference::Inference::new(binding, preparer),
    };
    let id = invocation.turn.session.to_string();
    let (tx, mut messages) = tokio::sync::mpsc::unbounded_channel();
    let emit = Emitter::new(tx, stop.clone());
    let execution = async {
        loop {
            let session = sessions.load(&id).await?;
            let step = advance(engine.step(&session, &emit), &stop).await?;
            let (mut transition, approval) = step;
            sessions
                .apply_effects(&session, &mut transition.effects)
                .await?;
            events.emit(EngineEvent::Checkpoint(sessions.snapshot().await));
            if transition.status == RunStatus::Continuing || approval {
                continue;
            }
            break Ok::<_, anyhow::Error>(transition.status);
        }
    };
    tokio::pin!(execution);
    let result = loop {
        tokio::select! {
            result = &mut execution => break result,
            message = messages.recv() => {
                if let Some(message) = message {
                    events.emit(EngineEvent::Message(message));
                }
            }
        }
    };
    if result.is_err() {
        stop.cancel();
    }
    if stop.is_cancelled() {
        let session = sessions.load(&id).await?;
        let mut effects = engine.tools.finalize_cancellation();
        effects.extend(engine.inference.finalize_cancellation());
        effects.extend(engine.steer.finalize_cancellation());
        sessions
            .apply_effects(&session, &mut effects)
            .await
            .map_err(fault)?;
        events.emit(EngineEvent::Checkpoint(sessions.snapshot().await));
    }
    let status = result.map_err(fault)?;
    let session = sessions.load(&id).await?;
    let error = session.conversation.last().and_then(|message| {
        let kind = message.error_kind()?;
        Some(model::Error::new(kind, message.as_concat_text()))
    });
    Ok(RunOutcome {
        status,
        output: output(&session),
        error,
    })
}

async fn advance(
    work: impl Future<Output = anyhow::Result<(operation::Transition, bool)>>,
    stop: &CancellationToken,
) -> anyhow::Result<(operation::Transition, bool)> {
    let result = tokio::select! {
        biased;
        result = work, if !stop.is_cancelled() => Some(result),
        _ = stop.cancelled() => None,
    };
    match result {
        Some(Ok(transition)) => Ok(transition),
        Some(Err(error)) if !stop.is_cancelled() => Err(error),
        _ => Ok((
            operation::Transition {
                effects: Vec::new(),
                status: RunStatus::Cancelled,
            },
            false,
        )),
    }
}

struct Engine {
    manual: bool,
    compact: compact::Compaction,
    approval: operations::Approval,
    prompt: prompt::Prompt,
    tools: tools::Tools,
    remaining: operations::Remaining,
    steer: steer::Steer,
    max_turns: Option<operations::MaxTurns>,
    callbacks: Vec<Arc<dyn Callback>>,
    inference: inference::Inference,
}

impl Engine {
    async fn step(
        &self,
        session: &Session,
        emit: &Emitter,
    ) -> anyhow::Result<(operation::Transition, bool)> {
        let conversation = &session.conversation;
        macro_rules! stage {
            ($call:expr, $approval:expr) => {
                if let Outcome::Applied(transition) = $call.await? {
                    return Ok((transition, $approval));
                }
            };
        }
        if self.manual {
            stage!(self.compact.run(session, conversation, emit), false);
        } else {
            stage!(
                operations::PlanReview.run(session, conversation, emit),
                false
            );
            stage!(self.approval.run(session, conversation, emit), true);
            stage!(self.tools.run(session, conversation, emit), false);
            stage!(self.remaining.run(session, conversation, emit), false);
            stage!(self.steer.run(session, conversation, emit), false);
            stage!(self.compact.run(session, conversation, emit), false);
            stage!(
                operations::EmptyResponse.run(session, conversation, emit),
                false
            );
            stage!(
                operations::ExitOnError.run(session, conversation, emit),
                false
            );
            if let Some(limit) = &self.max_turns {
                stage!(limit.run(session, conversation, emit), false);
            }
            for callback in &self.callbacks {
                stage!(callback.run(session, conversation, emit), false);
            }
            let input = async {
                let mut input = prompt::InferenceInput {
                    tools: self.tools.definitions(session).await?,
                    instructions: self.prompt.instructions(session).await?,
                    contexts: self.prompt.contexts(session).await?,
                };
                if let Some(limit) = &self.max_turns {
                    input.contexts.extend(limit.contexts(conversation).await?);
                }
                Ok(input)
            };
            stage!(self.inference.run(session, input, emit), false);
        }
        Ok((
            operation::Transition {
                effects: Vec::new(),
                status: RunStatus::Yielded,
            },
            false,
        ))
    }
}

fn output(session: &Session) -> Option<String> {
    session
        .conversation
        .iter()
        .rev()
        .find(|message| {
            message.role == model::Role::Assistant
                && message.is_user_visible()
                && !message.as_concat_text().is_empty()
        })
        .map(|message| message.as_concat_text())
}

fn fault(error: anyhow::Error) -> Fault {
    error
        .downcast_ref::<Fault>()
        .cloned()
        .unwrap_or_else(|| Fault::new(ErrorCode::Unavailable, error.to_string()))
}
