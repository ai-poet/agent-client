// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The team runtime: the tools' business rules and the scheduler, over the
//! durable state, driving members through a [`Host`].
//!
//! Synchronous on purpose. Every operation is a few small file reads and
//! writes under a keyed lock, and the host's primitives — start a member,
//! hand it a message, stop it, wake the captain — return at once (a member
//! turn runs elsewhere). The bridge runs these on a blocking thread; tests
//! drive them with a fake host and replay the reference's lifecycle
//! scenarios.

pub mod args;
mod locks;
mod ops;
pub mod render;
mod sched;
pub mod schema;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::key::CAPTAIN_KEY;
use crate::profiles::TeamProfileConfig;
use crate::store::StateRoot;
use crate::types::{TeamMember, TeamModelFallback, TeamState, TeamTask};

pub use locks::{KeyedLocks, global_locks};
pub use ops::*;

/// How a text reaches a member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryMode {
    /// A turn of its own, after the current one: task assignments.
    Queue,
    /// Joins the running turn at its next model step, or starts one when
    /// idle: messages.
    Steer,
}

/// What a member's runtime is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberActivity {
    Running,
    Idle,
    /// Not resident in this process (never started here, or stopped); it
    /// starts again from its saved conversation when work arrives.
    Ready,
}

impl MemberActivity {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberActivity::Running => "running",
            MemberActivity::Idle => "idle",
            MemberActivity::Ready => "ready",
        }
    }
}

/// The captain's current model route, from its session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptainRoute {
    /// The gateway platform (`anthropic`, `openai`, …) or the endpoint id.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

/// A requested member route, before resolution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteRequest {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fallback: Option<TeamModelFallback>,
}

/// A member's resolved route, snapshotted into `team.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRoute {
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub fallback: Option<TeamModelFallback>,
}

/// What the runtime needs from the process that hosts the captain.
pub trait Host: Send + Sync {
    /// The captain's session id, as `team.json` records it.
    fn captain_id(&self) -> String;
    /// The captain's working directory; the state root lives under it.
    fn workspace(&self) -> PathBuf;
    /// The captain's current route, which members inherit by default.
    fn captain_route(&self) -> CaptainRoute;
    /// Whether a member route can run at all (its key resolves, its
    /// endpoint exists). The error names what is missing.
    fn validate_route(&self, route: &MemberRoute) -> Result<(), String>;
    /// What a member's runtime is doing, by member session id.
    fn member_activity(&self, member_id: &str) -> MemberActivity;
    /// Start a member whose record carries its route, with its first prompt.
    /// Returns the new member session id.
    fn spawn_member(
        &self,
        team: &TeamState,
        member: &TeamMember,
        prompt: &str,
    ) -> Result<String, String>;
    /// Hand a started member a text. A member that is not resident starts
    /// again from its saved conversation. `false` when it cannot take input
    /// now.
    fn deliver(
        &self,
        team: &TeamState,
        member: &TeamMember,
        text: &str,
        mode: DeliveryMode,
    ) -> bool;
    /// Stop the members' current work, drop their queued input, and wait
    /// (bounded) until they are quiet.
    fn drain_members(&self, member_ids: &[String]) -> Result<(), String>;
    /// Wake the captain with mail: joins a running turn, or starts one.
    fn steer_captain(&self, text: &str) -> bool;
    /// Stop the captain's running turn, keeping queued user input.
    fn cancel_captain(&self);
    /// A turn of the captain's own after the current one ends.
    fn followup_captain(&self, text: &str) -> bool;
    /// Context the captain should see with the next user message.
    fn park_captain_context(&self, text: &str);
    /// The team's record changed; a UI may want to look again.
    fn team_changed(&self, _team_id: &str) {}
}

/// The settings a runtime runs with, snapshotted at session start.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeConfig {
    pub state_dir: String,
    pub member_model: Option<String>,
    pub member_reasoning_effort: Option<String>,
    pub execution_prompt: Option<String>,
    pub fallback: Option<TeamModelFallback>,
    pub member_max_depth: u32,
    pub max_members: usize,
    pub profiles: BTreeMap<String, TeamProfileConfig>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            state_dir: crate::store::DEFAULT_STATE_DIR.to_owned(),
            member_model: None,
            member_reasoning_effort: None,
            execution_prompt: None,
            fallback: None,
            member_max_depth: 0,
            max_members: 8,
            profiles: BTreeMap::new(),
        }
    }
}

/// One captain's team runtime.
pub struct TeamRuntime {
    host: Arc<dyn Host>,
    config: RuntimeConfig,
    root: StateRoot,
    /// Open attempts this process saw their member go idle with, by
    /// `team\0member`. Empty after a restart, which is what lets each durable
    /// open attempt be recovered exactly once.
    parked: Mutex<HashMap<String, String>>,
}

/// The model-visible error of a tool call.
pub type OpResult<T> = Result<T, String>;

fn text<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn parked_key(team_id: &str, member_name: &str) -> String {
    format!("{team_id}\u{0}{member_name}")
}

/// Which side of the team a caller is on, re-derived from fresh state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    Captain,
    Member(String),
}

impl Identity {
    pub fn name(&self) -> &str {
        match self {
            Identity::Captain => CAPTAIN_KEY,
            Identity::Member(name) => name,
        }
    }

    pub fn is_captain(&self) -> bool {
        matches!(self, Identity::Captain)
    }
}

pub(crate) fn identity_of(team: &TeamState, session_id: &str) -> Option<Identity> {
    if team.captain_session_id == session_id {
        return Some(Identity::Captain);
    }
    team.member_by_id(session_id)
        .map(|member| Identity::Member(member.name.clone()))
}

/// A live (non-removed) member by display name.
pub(crate) fn require_member<'a>(team: &'a TeamState, name: &str) -> OpResult<&'a TeamMember> {
    team.members
        .iter()
        .find(|member| member.name == name && !member.is_removed())
        .ok_or_else(|| {
            format!(
                "no active member named \"{name}\" in team \"{}\"",
                team.name
            )
        })
}

pub(crate) fn require_member_mut<'a>(
    team: &'a mut TeamState,
    name: &str,
) -> OpResult<&'a mut TeamMember> {
    let team_name = team.name.clone();
    team.members
        .iter_mut()
        .find(|member| member.name == name && !member.is_removed())
        .ok_or_else(|| format!("no active member named \"{name}\" in team \"{team_name}\""))
}

pub(crate) fn require_task<'a>(team: &'a TeamState, task_id: &str) -> OpResult<&'a TeamTask> {
    team.task(task_id).ok_or_else(|| {
        format!(
            "no task \"{task_id}\" in team \"{}\" — use agent_teams_status to list tasks",
            team.name
        )
    })
}

pub(crate) fn task_index(team: &TeamState, task_id: &str) -> OpResult<usize> {
    team.tasks
        .iter()
        .position(|task| task.id == task_id)
        .ok_or_else(|| {
            format!(
                "no task \"{task_id}\" in team \"{}\" — use agent_teams_status to list tasks",
                team.name
            )
        })
}

pub(crate) fn trimmed_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl TeamRuntime {
    pub fn new(host: Arc<dyn Host>, config: RuntimeConfig) -> Self {
        let root = StateRoot::new(&host.workspace(), &config.state_dir);
        Self {
            host,
            config,
            root,
            parked: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn root(&self) -> &StateRoot {
        &self.root
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    fn captain_id(&self) -> String {
        self.host.captain_id()
    }

    pub(crate) fn team_lock_key(&self, team_id: &str) -> String {
        format!("team:{}:{team_id}", self.root.lock_key())
    }

    fn captain_lock_key(&self) -> String {
        format!("captain:{}:{}", self.root.lock_key(), self.captain_id())
    }

    fn member_lock_key(&self, team_id: &str, member_name: &str) -> String {
        format!(
            "member:{}\u{0}{team_id}\u{0}{member_name}",
            self.root.lock_key()
        )
    }

    /// Run `f` holding the team's lock.
    pub(crate) fn with_team_lock<T>(&self, team_id: &str, f: impl FnOnce() -> T) -> T {
        global_locks().with(&self.team_lock_key(team_id), f)
    }

    /// Fresh state for a team that still exists.
    pub(crate) fn require_fresh(&self, team_id: &str) -> OpResult<TeamState> {
        self.root
            .read_team(team_id)
            .map_err(text)?
            .ok_or_else(|| format!("team \"{team_id}\" is no longer active"))
    }

    /// Fresh state with the captain's authority re-checked.
    pub(crate) fn require_fresh_captain(&self, team_id: &str) -> OpResult<TeamState> {
        let team = self.require_fresh(team_id)?;
        if team.captain_session_id != self.captain_id() {
            return Err(format!(
                "only the captain of team \"{}\" may perform this operation",
                team.name
            ));
        }
        Ok(team)
    }

    /// Fresh state plus the caller's identity, re-derived.
    pub(crate) fn require_fresh_participant(
        &self,
        team_id: &str,
        caller: &str,
    ) -> OpResult<(TeamState, Identity)> {
        let team = self.require_fresh(team_id)?;
        let identity = identity_of(&team, caller).ok_or_else(|| {
            format!(
                "you are no longer an active participant in team \"{}\"",
                team.name
            )
        })?;
        Ok((team, identity))
    }

    pub(crate) fn write(&self, team: &TeamState) -> OpResult<()> {
        self.root.write_team(team).map_err(text)?;
        self.host.team_changed(&team.id);
        Ok(())
    }

    /// The team the captain leads, or the error the model should see.
    pub(crate) fn require_captain_team(&self, caller: &str) -> OpResult<TeamState> {
        if caller != self.captain_id() {
            return Err(
                "you are not leading any team yet — call agent_teams_create first".to_owned(),
            );
        }
        self.root
            .find_team_by_captain(caller)
            .map_err(text)?
            .ok_or_else(|| {
                "you are not leading any team yet — call agent_teams_create first".to_owned()
            })
    }

    pub(crate) fn require_participant_team(&self, caller: &str) -> OpResult<TeamState> {
        self.root
            .find_team_by_participant(caller)
            .map_err(text)?
            .ok_or_else(|| "you do not lead or belong to any active team yet".to_owned())
    }

    /// The team this captain leads, if any.
    pub fn current_team(&self) -> OpResult<Option<TeamState>> {
        self.root
            .find_team_by_captain(&self.captain_id())
            .map_err(text)
    }

    fn parked_attempt(&self, team_id: &str, member_name: &str) -> Option<String> {
        self.parked
            .lock()
            .get(&parked_key(team_id, member_name))
            .cloned()
    }

    fn set_parked(&self, team_id: &str, member_name: &str, attempt: Option<String>) {
        let key = parked_key(team_id, member_name);
        let mut parked = self.parked.lock();
        match attempt {
            Some(attempt) => {
                parked.insert(key, attempt);
            }
            None => {
                parked.remove(&key);
            }
        }
    }
}
