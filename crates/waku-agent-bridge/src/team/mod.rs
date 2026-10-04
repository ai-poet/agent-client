//! Fork addition: AgentTeams for the built-in agent.
//!
//! The session becomes a team's **captain**: its model gets the
//! `agent_teams_*` tools and the captain protocol, and the team's members
//! run here as persistent sub-agents — each its own conversation, route and
//! permission scope, kept across many turns. The rules, the durable state
//! and the prompts live in the `agent-teams` crate; this module hosts them:
//!
//! - [`TeamHost`] is one per session. It holds the session's team runtime,
//!   its member runtimes, and the captain-side bookkeeping (mail injected
//!   into a running turn, contexts parked for the next user message).
//! - `host.rs` implements the runtime's [`agent_teams::runtime::Host`] over
//!   it: start, feed and stop members, wake the captain.
//! - `member.rs` runs one member's turns.
//! - `tools.rs` exposes the runtime's operations as engine tools.
//!
//! **Loaded on demand.** A session starts without the team tools and the
//! captain protocol, so a session that never uses a team pays nothing. The
//! first message that starts with `/agent-teams` — or a team already on disk
//! for this captain — activates them for the rest of the session; that
//! costs one prompt-cache miss, after which the section is byte-identical on
//! every turn.

mod host;
mod member;
mod tools;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use agent_teams::command::{ControlAction, decode_control};
use agent_teams::config::TeamsConfig;
use agent_teams::profiles::TeamProfileConfig;
use agent_teams::runtime::{RuntimeConfig, TeamRuntime};
use claurst_core::CostTracker;
use claurst_query::QueryConfig;
use claurst_tools::Tool;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::background::{BackgroundEntry, BackgroundKind, BackgroundStatus};
use crate::config::AgentStartOptions;
use crate::events::{AgentEvent, EventSink};
use crate::permission::PermissionBridge;

use member::MemberRuntime;

/// What the captain's session lends a member.
#[derive(Clone)]
pub(crate) struct CaptainShared {
    pub bridge: Arc<PermissionBridge>,
    pub cost_tracker: Arc<CostTracker>,
    pub file_history: Arc<parking_lot::Mutex<claurst_core::file_history::FileHistory>>,
    pub mcp: Option<Arc<claurst_mcp::McpManager>>,
}

/// How a wake reached the captain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wake {
    /// A turn of its own started with the text.
    Started,
    /// Joined the running turn at its next step.
    Injected,
    /// The session is compacting; the text waits for the slot.
    Deferred,
    /// The session is gone.
    Gone,
}

/// The session's side, implemented in `session.rs` where its private state
/// is reachable.
pub(crate) trait CaptainPort: Send + Sync {
    fn options(&self) -> Option<AgentStartOptions>;
    fn shared(&self) -> Option<CaptainShared>;
    /// A team turn when idle, a silent injection into the running turn
    /// otherwise. Never a user message: the transcript must not show team
    /// mail as something the person typed.
    fn wake(&self, text: String) -> Wake;
    fn is_busy(&self) -> bool;
    /// Stop the captain's running turn, leaving its members' dialogs.
    fn cancel_turn(&self);
    /// Rebuild the session's tool sets, now that the team tools are in.
    fn refresh_tools(&self);
    /// Publish the session's background work afresh.
    fn refresh_background_work(&self);
}

/// One session's team.
pub(crate) struct TeamHost {
    enabled: bool,
    slash_command: bool,
    config: TeamsConfig,
    profiles: BTreeMap<String, TeamProfileConfig>,
    /// The captain section, rendered once so it never changes.
    section: String,
    active: AtomicBool,
    events: EventSink,
    captain_id: String,
    cwd: PathBuf,
    port: OnceLock<Box<dyn CaptainPort>>,
    runtime: OnceLock<Arc<TeamRuntime>>,
    members: Mutex<HashMap<String, Arc<MemberRuntime>>>,
    /// Cancels every member when the session goes.
    shutdown: CancellationToken,
    /// Texts injected into the captain's running turn, until it ends.
    injected: Mutex<Vec<String>>,
    /// Texts waiting for the captain's turn slot.
    pending_wake: Mutex<Vec<String>>,
    /// Appended to the next message the person writes.
    parked_context: Mutex<Vec<String>>,
    this: OnceLock<Weak<TeamHost>>,
}

/// Where AgentTeams keeps its settings: beside the engine's own file.
fn config_path() -> PathBuf {
    claurst_core::config::Settings::global_settings_path()
        .parent()
        .map(|dir| dir.join(agent_teams::config::CONFIG_FILE))
        .unwrap_or_else(|| PathBuf::from(agent_teams::config::CONFIG_FILE))
}

/// The settings, or the defaults when the file cannot be read.
pub(crate) fn load_config() -> TeamsConfig {
    agent_teams::config::load(&config_path()).unwrap_or_else(|error| {
        tracing::warn!(%error, "agent-teams: settings unreadable; using defaults");
        TeamsConfig::default()
    })
}

/// `/agent-teams` and the profile aliases, for the composer's command list.
/// Empty when the feature or its command is switched off.
pub fn slash_commands() -> Vec<(String, Option<String>)> {
    let config = load_config();
    if !config.enabled || !config.slash_command {
        return Vec::new();
    }
    let mut commands = vec![(agent_teams::command::AGENT_TEAMS_COMMAND.to_owned(), None)];
    for name in config.effective_profiles().keys() {
        if let Some(alias) = agent_teams::command::profile_command_name(name) {
            commands.push((alias, Some(name.clone())));
        }
    }
    commands
}

impl TeamHost {
    pub fn new(options: &AgentStartOptions, captain_id: &str, events: EventSink) -> Arc<Self> {
        Self::with_config(load_config(), options, captain_id, events)
    }

    fn with_config(
        config: TeamsConfig,
        options: &AgentStartOptions,
        captain_id: &str,
        events: EventSink,
    ) -> Arc<Self> {
        let profiles = config.effective_profiles();
        let profiles_text = agent_teams::profiles::format_profiles_for_prompt(&profiles)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "agent-teams: profiles left out of the protocol");
                String::new()
            });
        let host = Arc::new(Self {
            enabled: config.enabled,
            slash_command: config.slash_command,
            section: agent_teams::prompts::captain_section(&profiles_text),
            profiles,
            config,
            active: AtomicBool::new(false),
            events,
            captain_id: captain_id.to_owned(),
            cwd: options.cwd.clone(),
            port: OnceLock::new(),
            runtime: OnceLock::new(),
            members: Mutex::new(HashMap::new()),
            shutdown: CancellationToken::new(),
            injected: Mutex::new(Vec::new()),
            pending_wake: Mutex::new(Vec::new()),
            parked_context: Mutex::new(Vec::new()),
            this: OnceLock::new(),
        });
        let _ = host.this.set(Arc::downgrade(&host));
        host
    }

    fn this(&self) -> Option<Arc<TeamHost>> {
        self.this.get().and_then(Weak::upgrade)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn port(&self) -> Option<&dyn CaptainPort> {
        self.port.get().map(|port| port.as_ref())
    }

    pub(crate) fn runtime(&self) -> Option<Arc<TeamRuntime>> {
        self.runtime.get().cloned()
    }

    pub(crate) fn captain_id(&self) -> &str {
        &self.captain_id
    }

    pub(crate) fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    pub(crate) fn events(&self) -> &EventSink {
        &self.events
    }

    pub(crate) fn config(&self) -> &TeamsConfig {
        &self.config
    }

    pub(crate) fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }

    /// Hook the session in, then pick up a team this captain already leads —
    /// after a restart its members resume where they were.
    pub fn attach(self: &Arc<Self>, port: Box<dyn CaptainPort>) {
        let _ = self.port.set(port);
        if !self.enabled {
            return;
        }
        let runtime = Arc::new(TeamRuntime::new(
            Arc::new(host::BridgeHost::new(Arc::downgrade(self))),
            RuntimeConfig {
                state_dir: self.config.state_dir().to_owned(),
                member_model: self.config.member_model.clone(),
                member_reasoning_effort: self.config.member_reasoning_effort.clone(),
                execution_prompt: self.config.execution_prompt.clone(),
                fallback: self.config.fallback.clone(),
                member_max_depth: self.config.member_max_depth(),
                max_members: self.config.max_members() as usize,
                profiles: self.profiles.clone(),
            },
        ));
        let _ = self.runtime.set(runtime.clone());
        let host = self.clone();
        self.blocking(move || {
            if let Ok(Some(_)) = runtime.current_team() {
                host.activate();
                runtime.resume_after_start();
            }
        });
    }

    /// Run `f` on a blocking thread: the runtime's operations do file I/O
    /// and may wait on members.
    pub(crate) fn blocking(&self, f: impl FnOnce() + Send + 'static) {
        match crate::runtime::shared() {
            Ok(rt) => {
                rt.spawn_blocking(f);
            }
            Err(error) => tracing::warn!(%error, "agent-teams: no runtime for team work"),
        }
    }

    /// Turn the team tools and protocol on for the rest of the session.
    pub fn activate(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.active.swap(true, Ordering::AcqRel) {
            return true;
        }
        if let Some(port) = self.port() {
            port.refresh_tools();
        }
        true
    }

    /// The captain's tools, while active.
    pub fn captain_tools(self: &Arc<Self>) -> Vec<Box<dyn Tool>> {
        if !self.is_active() {
            return Vec::new();
        }
        tools::captain_tools(self)
    }

    /// Append the captain protocol to this turn's rules, while active.
    pub fn extend_rules(&self, query: &mut QueryConfig) {
        if !self.is_active() {
            return;
        }
        query.append_system_prompt = Some(match query.append_system_prompt.take() {
            Some(rules) if !rules.trim().is_empty() => format!("{rules}\n\n{}", self.section),
            _ => self.section.clone(),
        });
    }

    /// The text a person's message reaches the model with: `/agent-teams`
    /// activates the team and gets the activation directive; a context the
    /// Team panel parked (a discarded plan) goes along once.
    pub fn augment_prompt(&self, text: String) -> String {
        if !self.enabled {
            return text;
        }
        let mut extra = Vec::new();
        if self.slash_command {
            let names = self.profiles.keys().map(String::as_str);
            match agent_teams::command::parse_command_text(&text, names) {
                Ok(Some(invocation)) => {
                    self.activate();
                    extra.push(agent_teams::command::directive_for_invocation(
                        &invocation,
                        &self.profiles,
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    self.activate();
                    extra.push(agent_teams::command::parse_failure_directive(&error));
                }
            }
        }
        extra.extend(self.parked_context.lock().drain(..));
        if extra.is_empty() {
            return text;
        }
        format!("{text}\n\n{}", extra.join("\n\n"))
    }

    /// A Team panel control sent as a prompt. `true` when `text` was one —
    /// it is then handled here and starts no turn of its own.
    pub fn intercept(&self, text: &str) -> bool {
        let Some(control) = decode_control(text) else {
            return false;
        };
        if !self.enabled {
            self.events.emit(AgentEvent::Error(
                "AgentTeams is switched off in Settings → Agent.".to_owned(),
            ));
            return true;
        }
        self.activate();
        let (Some(runtime), Some(host)) = (self.runtime(), self.this()) else {
            return true;
        };
        self.blocking(move || {
            let team_id = control.team_id.as_str();
            let outcome = match control.action {
                ControlAction::Approve => runtime.approve_from_panel(team_id).map(|_| ()),
                ControlAction::Revise => runtime.continue_staged_planning(team_id).map(|_| ()),
                ControlAction::Discard => runtime.discard_staged(team_id).map(|_| ()),
                ControlAction::Stop => runtime.halt(team_id).map(|halted| {
                    if !halted.already_halted {
                        host.park_context(agent_teams::prompts::halted_context(
                            &halted.team_name,
                            halted.cancelled_tasks,
                        ));
                    }
                }),
                ControlAction::Kick => {
                    runtime.kick_team(team_id);
                    runtime.flush_captain_mail();
                    Ok(())
                }
            };
            if let Err(error) = outcome {
                host.events
                    .emit(AgentEvent::Error(format!("AgentTeams: {error}")));
            }
            host.refresh_snapshot();
        });
        true
    }

    pub(crate) fn park_context(&self, text: String) {
        self.parked_context.lock().push(text);
    }

    /// Record a text injected into the captain's running turn.
    pub(crate) fn note_injected(&self, text: String) {
        self.injected.lock().push(text);
    }

    /// Queue a text for when the captain's turn slot frees.
    pub(crate) fn defer_wake(&self, text: String) {
        self.pending_wake.lock().push(text);
    }

    /// The captain's turn ended. `leftovers` are the injected texts its queue
    /// still held — they never reached the model and go again; `continuing`
    /// is set when another turn follows at once (a goal).
    pub fn after_turn(&self, leftovers: Vec<String>, continuing: bool) {
        let injected: Vec<String> = std::mem::take(&mut *self.injected.lock());
        let mut again: Vec<String> = leftovers
            .into_iter()
            .filter(|text| injected.contains(text))
            .collect();
        again.extend(self.pending_wake.lock().drain(..));
        let Some(runtime) = self.runtime().filter(|_| self.is_active()) else {
            return;
        };
        let Some(host) = self.this() else {
            return;
        };
        self.blocking(move || {
            if !continuing {
                runtime.captain_idle_edge();
            }
            // A wake deferred again (compacting) comes back here when the
            // compaction ends.
            if !again.is_empty()
                && let Some(port) = host.port()
            {
                port.wake(again.join("\n\n"));
            }
            if !continuing {
                runtime.flush_captain_mail();
            }
        });
    }

    /// The members whose turn is running, as live background work: an entry
    /// missing from a snapshot would read as lost.
    pub fn background_entries(&self) -> Vec<BackgroundEntry> {
        self.members
            .lock()
            .values()
            .filter_map(|member| member.live_entry())
            .collect()
    }

    /// Emit a fresh background-work level signal; the Team panel reads its
    /// cue to look again from it.
    pub(crate) fn refresh_snapshot(&self) {
        if let Some(port) = self.port() {
            port.refresh_background_work();
        }
    }

    /// Stop one member's running turn from the panel. Its open attempt stays
    /// parked for the captain to continue or reassign.
    pub fn stop_member(&self, id: &str) -> bool {
        let member = self.members.lock().get(id).cloned();
        match member {
            Some(member) => {
                member.interrupt();
                true
            }
            None => false,
        }
    }

    pub(crate) fn member(&self, id: &str) -> Option<Arc<MemberRuntime>> {
        self.members.lock().get(id).cloned()
    }

    pub(crate) fn insert_member(&self, member: Arc<MemberRuntime>) {
        self.members.lock().insert(member.id().to_owned(), member);
    }

    /// The session is going away: so are its members.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        let members: Vec<Arc<MemberRuntime>> =
            self.members.lock().drain().map(|(_, m)| m).collect();
        for member in members {
            member.interrupt();
        }
    }
}

/// A background entry for a running member.
pub(crate) fn live_entry(id: &str, title: &str, started_at_ms: u64) -> BackgroundEntry {
    BackgroundEntry {
        id: id.to_owned(),
        kind: BackgroundKind::Subagent,
        title: title.to_owned(),
        status: BackgroundStatus::Running,
        detail: None,
        pid: None,
        output: None,
        started_at_ms,
        finished_at_ms: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(enabled: bool) -> Arc<TeamHost> {
        let config = TeamsConfig {
            enabled,
            ..TeamsConfig::default()
        };
        TeamHost::with_config(
            config,
            &AgentStartOptions::default(),
            "captain",
            EventSink::new(|_| {}),
        )
    }

    #[test]
    fn a_session_starts_without_the_team() {
        let team = host(true);
        assert!(!team.is_active());
        assert!(team.captain_tools().is_empty());
        let mut query = QueryConfig::default();
        query.append_system_prompt = Some("house rules".into());
        team.extend_rules(&mut query);
        assert_eq!(query.append_system_prompt.as_deref(), Some("house rules"));
        assert_eq!(team.augment_prompt("hello".into()), "hello");
        assert!(!team.is_active());
    }

    #[test]
    fn the_command_activates_the_team_in_the_same_turn() {
        let team = host(true);
        let text = team.augment_prompt("/agent-teams audit the repo".into());
        assert!(team.is_active());
        assert!(
            text.starts_with("/agent-teams audit the repo\n\n"),
            "{text}"
        );
        assert!(text.len() > "/agent-teams audit the repo".len() + 20);
        let names: Vec<String> = team
            .captain_tools()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names, agent_teams::prompts::TEAM_TOOL_NAMES);
        let mut first = QueryConfig::default();
        team.extend_rules(&mut first);
        let mut second = QueryConfig::default();
        team.extend_rules(&mut second);
        let section = first.append_system_prompt.unwrap();
        assert!(section.starts_with(agent_teams::prompts::TEAM_ACTIVATION_PROMPT));
        assert_eq!(Some(section), second.append_system_prompt);
    }

    #[test]
    fn a_switched_off_team_never_activates() {
        let team = host(false);
        let text = team.augment_prompt("/agent-teams audit".into());
        assert_eq!(text, "/agent-teams audit");
        assert!(!team.activate());
        assert!(team.captain_tools().is_empty());
    }

    #[test]
    fn members_get_exactly_the_four_member_tools() {
        let team = host(true);
        let names: Vec<String> = tools::member_tools(&team)
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect();
        assert_eq!(names, agent_teams::prompts::MEMBER_TOOL_NAMES);
    }

    #[test]
    fn a_parked_context_rides_the_next_message_once() {
        let team = host(true);
        team.park_context("the plan was discarded".into());
        assert_eq!(
            team.augment_prompt("hi".into()),
            "hi\n\nthe plan was discarded"
        );
        assert_eq!(team.augment_prompt("again".into()), "again");
    }

    #[test]
    fn controls_are_intercepted_and_ordinary_text_is_not() {
        let team = host(true);
        assert!(!team.intercept("/agent-teams go"));
        let control = agent_teams::command::encode_control(&agent_teams::command::Control {
            action: ControlAction::Kick,
            team_id: "alpha".into(),
        });
        assert!(team.intercept(&control));
    }
}
