//! Fork addition: the session's side of AgentTeams (`crate::team`).
//!
//! A child module of `session.rs`, so it reaches the session's private
//! state — the turn slot, its queue, the tool sets — without widening any of
//! it. Everything the team does to its captain goes through [`Port`].

use std::sync::{Arc, Weak};

use claurst_query::{CommandPriority, CommandQueue, QueuedCommand};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::background::{self, BackgroundEntry};
use crate::config::AgentStartOptions;
use crate::team::{CaptainPort, CaptainShared, Wake};

use super::{AgentSession, Inner, ToolSets, Turn, run_turn};

/// Who a turn's prompt came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptOrigin {
    /// Typed by the person (or a goal's continuation): the team may add its
    /// activation directive.
    User,
    /// Team mail waking the captain.
    Team,
}

pub(super) struct Port(pub Weak<Inner>);

impl CaptainPort for Port {
    fn options(&self) -> Option<AgentStartOptions> {
        Some(self.0.upgrade()?.options.lock().clone())
    }

    fn shared(&self) -> Option<CaptainShared> {
        let inner = self.0.upgrade()?;
        Some(CaptainShared {
            bridge: inner.bridge.clone(),
            cost_tracker: inner.cost_tracker.clone(),
            file_history: inner.file_history.clone(),
            mcp: inner.mcp.lock().clone(),
        })
    }

    fn wake(&self, text: String) -> Wake {
        let Some(inner) = self.0.upgrade() else {
            return Wake::Gone;
        };
        let Ok(rt) = crate::runtime::shared() else {
            return Wake::Gone;
        };
        let (cancel, queue) = {
            let mut guard = inner.turn.lock();
            if let Some(turn) = guard.as_mut() {
                if turn.compacting {
                    drop(guard);
                    inner.team.defer_wake(text);
                    return Wake::Deferred;
                }
                // Into the running turn without the steering bookkeeping: a
                // steer is echoed into the transcript as the person's own
                // message, which this is not.
                turn.queue.push(
                    QueuedCommand::InjectUserMessage(text.clone()),
                    CommandPriority::Normal,
                );
                drop(guard);
                inner.team.note_injected(text);
                return Wake::Injected;
            }
            let cancel = CancellationToken::new();
            let queue = CommandQueue::new();
            *guard = Some(Turn {
                cancel: cancel.clone(),
                queue: queue.clone(),
                steers: Arc::new(Mutex::new(Vec::new())),
                watching: false,
                compacting: false,
            });
            (cancel, queue)
        };
        rt.spawn(async move {
            run_turn(inner, text, PromptOrigin::Team, cancel, queue).await;
        });
        Wake::Started
    }

    fn is_busy(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|inner| inner.turn.lock().is_some())
    }

    fn cancel_turn(&self) {
        if let Some(inner) = self.0.upgrade() {
            AgentSession { inner }.cancel();
        }
    }

    fn refresh_tools(&self) {
        let Some(inner) = self.0.upgrade() else {
            return;
        };
        let disallowed = inner.config.lock().disallowed_tools.clone();
        let mcp = inner.mcp.lock().clone();
        *inner.tools.lock() = ToolSets::build(
            &disallowed,
            mcp.as_ref(),
            &inner.subagents,
            inner.browser.as_ref(),
            Some(&inner.team),
        );
    }

    fn refresh_background_work(&self) {
        if let Some(inner) = self.0.upgrade() {
            AgentSession { inner }.refresh_background_work();
        }
    }
}

/// The session's background work: the engine's registry as this session
/// owns it, plus its team members that are working.
pub(super) fn snapshot(inner: &Inner) -> Vec<BackgroundEntry> {
    let mut entries = background::snapshot_owned(&inner.subagents.owned_background());
    entries.extend(inner.team.background_entries());
    entries
}

/// What a finished turn's queue still holds — texts it never took.
pub(super) fn queued_texts(queue: &CommandQueue) -> Vec<String> {
    queue
        .drain()
        .into_iter()
        .filter_map(|command| match command {
            QueuedCommand::InjectUserMessage(text) => Some(text),
            _ => None,
        })
        .collect()
}
