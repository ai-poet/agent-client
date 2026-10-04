//! The team runtime's view of this process: members are started, fed and
//! stopped here, and the captain is woken through its session.

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use agent_teams::runtime::{CaptainRoute, DeliveryMode, Host, MemberActivity, MemberRoute};
use agent_teams::types::{TeamMember, TeamState};

use super::member::{MemberRuntime, build_engine};
use super::{TeamHost, Wake};

pub(super) struct BridgeHost {
    team: Weak<TeamHost>,
    captain_id: String,
    workspace: PathBuf,
}

impl BridgeHost {
    pub fn new(team: Weak<TeamHost>) -> Self {
        let (captain_id, workspace) = team
            .upgrade()
            .map(|team| (team.captain_id().to_owned(), team.cwd().clone()))
            .unwrap_or_default();
        Self {
            team,
            captain_id,
            workspace,
        }
    }

    fn team(&self) -> Option<Arc<TeamHost>> {
        self.team.upgrade()
    }

    /// A member identity: unique per start, so a same-named member of a
    /// later team is never mistaken for a retired one.
    fn new_member_id(&self, team_id: &str, member: &TeamMember) -> String {
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        format!(
            "{}::team:{team_id}:{}:{suffix}",
            self.captain_id,
            agent_teams::sanitize_key(&member.name)
        )
    }
}

impl Host for BridgeHost {
    fn captain_id(&self) -> String {
        self.captain_id.clone()
    }

    fn workspace(&self) -> PathBuf {
        self.workspace.clone()
    }

    fn captain_route(&self) -> CaptainRoute {
        let Some(options) = self
            .team()
            .and_then(|team| team.port().and_then(|port| port.options()))
        else {
            return CaptainRoute::default();
        };
        CaptainRoute {
            // A bare model id reads as Anthropic, as `split_model` reads it.
            provider: Some(
                options
                    .platform
                    .clone()
                    .unwrap_or_else(|| "anthropic".to_owned()),
            ),
            model: options.model.clone(),
            reasoning_effort: options.reasoning_effort.clone(),
        }
    }

    fn validate_route(&self, route: &MemberRoute) -> Result<(), String> {
        let options = self
            .team()
            .and_then(|team| team.port().and_then(|port| port.options()))
            .ok_or("the captain session is gone")?;
        build_engine(&options, route, None)
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "member route {}/{} cannot run: {error:#}",
                    route.provider, route.model
                )
            })
    }

    fn member_activity(&self, member_id: &str) -> MemberActivity {
        self.team()
            .and_then(|team| team.member(member_id))
            .map_or(MemberActivity::Ready, |member| member.activity())
    }

    fn spawn_member(
        &self,
        team: &TeamState,
        member: &TeamMember,
        prompt: &str,
    ) -> Result<String, String> {
        let host = self.team().ok_or("the captain session is gone")?;
        let id = self.new_member_id(&team.id, member);
        let runtime = MemberRuntime::create(&host, team, member, &id)?;
        host.insert_member(runtime.clone());
        if !runtime.deliver(prompt.to_owned(), DeliveryMode::Queue) {
            return Err("the member could not take its first prompt".to_owned());
        }
        Ok(id)
    }

    fn deliver(
        &self,
        team: &TeamState,
        member: &TeamMember,
        text: &str,
        mode: DeliveryMode,
    ) -> bool {
        let Some(host) = self.team() else {
            return false;
        };
        let runtime = match host.member(&member.id) {
            Some(runtime) => runtime,
            // Not resident: the app restarted, or the session was reopened.
            // It picks up from its saved conversation.
            None => match MemberRuntime::create(&host, team, member, &member.id) {
                Ok(runtime) => {
                    host.insert_member(runtime.clone());
                    runtime
                }
                Err(error) => {
                    tracing::warn!(%error, member = %member.name, "agent-teams: member could not resume");
                    return false;
                }
            },
        };
        runtime.deliver(text.to_owned(), mode)
    }

    fn drain_members(&self, member_ids: &[String]) -> Result<(), String> {
        let Some(host) = self.team() else {
            return Ok(());
        };
        let mut failures = Vec::new();
        for id in member_ids {
            if let Some(member) = host.member(id)
                && let Err(error) = member.drain_blocking()
            {
                failures.push(error);
            }
        }
        host.refresh_snapshot();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn steer_captain(&self, text: &str) -> bool {
        let Some(host) = self.team() else {
            return false;
        };
        match host.port().map(|port| port.wake(text.to_owned())) {
            Some(Wake::Started | Wake::Injected | Wake::Deferred) => true,
            Some(Wake::Gone) | None => false,
        }
    }

    fn cancel_captain(&self) {
        if let Some(team) = self.team()
            && let Some(port) = team.port()
        {
            port.cancel_turn();
        }
    }

    fn followup_captain(&self, text: &str) -> bool {
        let Some(host) = self.team() else {
            return false;
        };
        let Some(port) = host.port() else {
            return false;
        };
        if port.is_busy() {
            // After the turn being cancelled ends.
            host.defer_wake(text.to_owned());
            true
        } else {
            port.wake(text.to_owned()) != Wake::Gone
        }
    }

    fn park_captain_context(&self, text: &str) {
        if let Some(host) = self.team() {
            host.park_context(text.to_owned());
        }
    }

    fn team_changed(&self, _team_id: &str) {
        if let Some(host) = self.team() {
            host.refresh_snapshot();
        }
    }
}
