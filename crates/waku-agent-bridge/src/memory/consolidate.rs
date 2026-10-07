//! One consolidation pass: ask the session's own model which new facts in
//! the recent conversation are worth keeping, and file them.
//!
//! Detached from the session on purpose — a pass started as the session is
//! torn down must not hold it up — and silent on every failure: the worst a
//! failed pass does is keep nothing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use auto_memory::consolidate::{apply_candidates, build_consolidation_prompt, parse_candidates};
use auto_memory::{MemoryStore, ScopeDir};
use tokio_util::sync::CancellationToken;

use super::ConsolidationRoute;

/// A pass that has not answered by then is abandoned.
const TIMEOUT: Duration = Duration::from_secs(120);

pub(super) struct Pass {
    pub store: Arc<MemoryStore>,
    pub user: Option<ScopeDir>,
    pub project: ScopeDir,
    pub cwd: PathBuf,
    pub session_id: String,
    pub max_memories: usize,
    pub max_tokens: u32,
}

impl Pass {
    pub(super) async fn run(self, transcript: String, route: ConsolidationRoute) {
        let names = {
            let store = self.store.clone();
            let scopes = self
                .user
                .iter()
                .cloned()
                .chain(std::iter::once(self.project.clone()))
                .collect::<Vec<_>>();
            match tokio::task::spawn_blocking(move || {
                scopes
                    .iter()
                    .flat_map(|dir| store.list(dir))
                    .map(|record| record.name)
                    .collect::<Vec<_>>()
            })
            .await
            {
                Ok(names) => names,
                Err(error) => {
                    tracing::warn!("agent: memory consolidation skipped: {error}");
                    return;
                }
            }
        };
        let prompt = build_consolidation_prompt(&names, &transcript, self.max_memories);

        // One plain answer: no tools, no session rules, no reasoning budget.
        let mut query = route.query;
        query.max_turns = 1;
        query.max_tokens = self.max_tokens;
        query.system_prompt = None;
        query.append_system_prompt = None;
        query.thinking_budget = None;
        query.effort_level = None;
        query.command_queue = None;
        query.skill_index = None;
        query.agent_name = None;
        query.agent_definition = None;
        query.managed_agents = None;
        query.enabled_tools = None;
        query.continuation = claurst_query::continuation::ContinuationMode::Default;

        let answer = tokio::time::timeout(
            TIMEOUT,
            crate::oneshot::ask_once(
                route.client.as_ref(),
                &query,
                route.config,
                &self.cwd,
                format!("{}::memory-consolidate", self.session_id),
                &prompt,
                CancellationToken::new(),
            ),
        )
        .await;
        let text = match answer {
            Ok(Ok(text)) => text,
            Ok(Err(error)) => {
                tracing::warn!("agent: memory consolidation failed: {error:#}");
                return;
            }
            Err(_) => {
                tracing::warn!("agent: memory consolidation timed out");
                return;
            }
        };
        let Some(candidates) = parse_candidates(&text) else {
            tracing::warn!("agent: memory consolidation answered without a JSON array");
            return;
        };
        let store = self.store;
        let (user, project, max) = (self.user, self.project, self.max_memories);
        match tokio::task::spawn_blocking(move || {
            apply_candidates(&store, user.as_ref(), &project, &candidates, max)
        })
        .await
        {
            Ok(0) => {}
            Ok(written) => tracing::info!("agent: consolidated {written} memories"),
            Err(error) => tracing::warn!("agent: consolidated memories not filed: {error}"),
        }
    }
}
