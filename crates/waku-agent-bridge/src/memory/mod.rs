//! Fork addition: auto-memory for the built-in agent.
//!
//! A session gets six memory tools and, when there are memories, one prompt
//! section holding the user and project indexes (`auto_memory`, translated
//! from dsh-auto-memory). Only the root session has either: sub-agents and
//! team members build their tool sets from `engine_tools`, and the section is
//! appended after the sub-agents' copy of the turn's query is taken.
//!
//! With consolidation on, the session also keeps a short record of what was
//! said, and every few turns — and when the session is torn down — asks its
//! own model which new facts are worth keeping (`consolidate`).

mod consolidate;
mod tools;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use auto_memory::consolidate::{CaptureBuffer, Speaker};
use auto_memory::{MemoryConfig, MemoryStore, ScopeDir};
use claurst_api::AnthropicClient;
use claurst_core::config::{Config, Settings};
use claurst_core::types::{Message, Role};
use claurst_query::QueryConfig;
use claurst_tools::Tool;
use parking_lot::Mutex;

use crate::history;
use crate::runtime;

/// Deletes everything in a scope.
pub(crate) const DELETE_ALL_TOOL: &str = "memory_delete_all";
/// Deletes old memories (when not a dry run).
pub(crate) const PRUNE_TOOL: &str = "memory_prune";

/// The tools whose deletes only a person may approve: the permission
/// bridge asks for these whatever the access mode says.
pub(crate) fn needs_human_approval(tool_name: &str) -> bool {
    tool_name == DELETE_ALL_TOOL || tool_name == PRUNE_TOOL
}

/// The engine's config directory (beside its `settings.json`).
fn config_dir() -> PathBuf {
    Settings::global_settings_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The settings, or the defaults when the file cannot be read.
fn load_config() -> MemoryConfig {
    let path = config_dir().join(auto_memory::config::CONFIG_FILE);
    auto_memory::config::load(&path).unwrap_or_else(|error| {
        tracing::warn!("agent: auto-memory settings not read: {error:#}");
        MemoryConfig::default()
    })
}

/// The route a consolidation pass asks through: the session's own, as it
/// stands when the pass starts.
pub(crate) struct ConsolidationRoute {
    pub client: Arc<AnthropicClient>,
    pub query: QueryConfig,
    pub config: Config,
}

#[derive(Default)]
struct Capture {
    buffer: CaptureBuffer,
    /// Turns the person started since the last pass.
    user_turns: u32,
}

/// One session's memory.
pub(crate) struct MemoryHost {
    config: MemoryConfig,
    store: Arc<MemoryStore>,
    cwd: PathBuf,
    session_id: String,
    capture: Mutex<Capture>,
    consolidating: Arc<AtomicBool>,
}

impl MemoryHost {
    pub(crate) fn new(cwd: &Path, session_id: &str) -> Arc<Self> {
        Self::with_config(load_config(), &config_dir(), cwd, session_id)
    }

    fn with_config(config: MemoryConfig, config_dir: &Path, cwd: &Path, session_id: &str) -> Arc<Self> {
        let store = Arc::new(auto_memory::store_for(config_dir, &config));
        let host = Arc::new(Self {
            config,
            store,
            cwd: cwd.to_path_buf(),
            session_id: session_id.to_owned(),
            capture: Mutex::new(Capture::default()),
            consolidating: Arc::new(AtomicBool::new(false)),
        });
        // Eviction is decided when an index is rebuilt; a store nobody
        // writes to would never evict, so each session refreshes once.
        if host.config.enabled
            && host.store.stale_after_days() > 0
            && let Ok(rt) = runtime::shared()
        {
            let store = host.store.clone();
            let dirs = host.scopes();
            rt.spawn_blocking(move || {
                for dir in &dirs {
                    store.refresh_index(dir);
                }
            });
        }
        host
    }

    pub(crate) fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn user_dir(&self) -> Option<ScopeDir> {
        self.config
            .enable_user_scope
            .then(|| self.store.user_dir())
    }

    fn project_dir(&self) -> ScopeDir {
        self.store.project_dir(&self.cwd)
    }

    /// The scopes this session reaches, user first.
    fn scopes(&self) -> Vec<ScopeDir> {
        self.user_dir()
            .into_iter()
            .chain(std::iter::once(self.project_dir()))
            .collect()
    }

    /// The memory tools, unless memory is off; any the user disallowed by
    /// name are left out.
    pub(crate) fn tools(self: &Arc<Self>, disallowed: &[String]) -> Vec<Box<dyn Tool>> {
        if !self.enabled() {
            return Vec::new();
        }
        tools::all(self)
            .into_iter()
            .filter(|tool| !disallowed.iter().any(|name| name == tool.name()))
            .collect()
    }

    /// Append this turn's memory section. Read once per turn: a memory
    /// written mid-turn reaches the next one, and a turn that changed
    /// nothing renders the same bytes, so the prompt cache holds.
    pub(crate) fn extend_rules(&self, query: &mut QueryConfig) {
        if !self.enabled() {
            return;
        }
        let user = self.user_dir().and_then(|dir| self.store.read_index(&dir));
        let project = self.store.read_index(&self.project_dir());
        let section = auto_memory::prompt::render_section(
            user.as_deref(),
            project.as_deref(),
            self.config.max_bytes(),
        );
        if section.is_empty() {
            return;
        }
        query.append_system_prompt =
            crate::subagent::join_prompts(query.append_system_prompt.take(), Some(section));
    }

    fn consolidates(&self) -> bool {
        self.config.enabled && self.config.auto_summarize
    }

    /// Record what the person asked (a turn they started).
    pub(crate) fn note_user_prompt(&self, text: &str) {
        if !self.consolidates() {
            return;
        }
        let mut capture = self.capture.lock();
        capture.buffer.push(Speaker::User, text);
        capture.user_turns += 1;
    }

    /// Record what the agent answered in the turn marked `turn_mark`.
    pub(crate) fn note_turn_output(&self, messages: &[Message], turn_mark: &str) {
        if !self.consolidates() {
            return;
        }
        let Some(start) = history::position_of(messages, turn_mark) else {
            return;
        };
        let mut capture = self.capture.lock();
        for message in &messages[start + 1..] {
            if message.role == Role::Assistant {
                capture.buffer.push(Speaker::Assistant, &history::visible_text(message));
            }
        }
    }

    /// After a turn: a pass once enough turns have gone by and none is
    /// already running.
    pub(crate) fn after_turn(&self, route: impl FnOnce() -> ConsolidationRoute) {
        if !self.consolidates() || self.consolidating.load(Ordering::Acquire) {
            return;
        }
        let transcript = {
            let mut capture = self.capture.lock();
            if capture.user_turns < self.config.auto_summarize_every_turns()
                || !capture.buffer.has_user_text()
            {
                return;
            }
            capture.user_turns = 0;
            capture.buffer.take()
        };
        self.spawn_pass(transcript, route());
    }

    /// The session is being torn down: whatever was said since the last
    /// pass gets one.
    pub(crate) fn on_session_end(&self, route: impl FnOnce() -> ConsolidationRoute) {
        if !self.consolidates() {
            return;
        }
        let transcript = {
            let mut capture = self.capture.lock();
            if !capture.buffer.has_user_text() {
                return;
            }
            capture.user_turns = 0;
            capture.buffer.take()
        };
        self.spawn_pass(transcript, route());
    }

    fn spawn_pass(&self, transcript: String, route: ConsolidationRoute) {
        let Ok(rt) = runtime::shared() else {
            return;
        };
        self.consolidating.store(true, Ordering::Release);
        let pass = consolidate::Pass {
            store: self.store.clone(),
            user: self.user_dir(),
            project: self.project_dir(),
            cwd: self.cwd.clone(),
            session_id: self.session_id.clone(),
            max_memories: self.config.auto_summarize_max_memories(),
            max_tokens: self.config.auto_summarize_max_tokens(),
        };
        let running = self.consolidating.clone();
        rt.spawn(async move {
            pass.run(transcript, route).await;
            running.store(false, Ordering::Release);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(dir: &Path, config: MemoryConfig) -> Arc<MemoryHost> {
        MemoryHost::with_config(config, dir, Path::new("/work/app"), "session")
    }

    fn quiet() -> MemoryConfig {
        MemoryConfig {
            mirror_to_claude_code: false,
            ..MemoryConfig::default()
        }
    }

    #[test]
    fn the_section_appears_with_memories_and_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), quiet());
        let mut query = QueryConfig::default();
        host.extend_rules(&mut query);
        assert_eq!(query.append_system_prompt, None, "no memories, no section");

        host.store
            .write(
                &host.project_dir(),
                auto_memory::MemoryDraft {
                    name: "use-pnpm".into(),
                    title: None,
                    description: "The project uses pnpm".into(),
                    kind: auto_memory::MemoryType::Project,
                    body: "b".into(),
                    pinned: None,
                },
            )
            .unwrap();
        let mut first = QueryConfig {
            append_system_prompt: Some("rules".into()),
            ..QueryConfig::default()
        };
        host.extend_rules(&mut first);
        let text = first.append_system_prompt.clone().unwrap();
        assert!(text.starts_with("rules\n\n# Persistent memory index"));
        assert!(text.contains("[use-pnpm](use-pnpm.md)"));
        let mut second = QueryConfig {
            append_system_prompt: Some("rules".into()),
            ..QueryConfig::default()
        };
        host.extend_rules(&mut second);
        assert_eq!(first.append_system_prompt, second.append_system_prompt);
    }

    #[test]
    fn switched_off_memory_offers_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(
            tmp.path(),
            MemoryConfig {
                enabled: false,
                ..quiet()
            },
        );
        assert!(host.tools(&[]).is_empty());
        let mut query = QueryConfig::default();
        host.extend_rules(&mut query);
        assert_eq!(query.append_system_prompt, None);
    }

    #[test]
    fn tools_respect_the_disallowed_list() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), quiet());
        let names = host
            .tools(&["memory_prune".into()])
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "memory_write",
                "memory_read",
                "memory_list",
                "memory_delete",
                "memory_delete_all"
            ]
        );
    }

    #[test]
    fn a_pass_waits_for_enough_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(
            tmp.path(),
            MemoryConfig {
                auto_summarize_every_turns: 3,
                ..quiet()
            },
        );
        let fired = std::cell::Cell::new(0);
        let route = || {
            fired.set(fired.get() + 1);
            unreachable_route()
        };
        host.note_user_prompt("one");
        host.after_turn(route);
        host.note_user_prompt("two");
        host.after_turn(route);
        assert_eq!(fired.get(), 0);
        assert_eq!(host.capture.lock().user_turns, 2);
    }

    /// A route that is never built: the tests above stop before a pass.
    fn unreachable_route() -> ConsolidationRoute {
        panic!("no pass expected")
    }
}
