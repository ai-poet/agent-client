//! Lifecycle scenarios over a fake host, after the reference's
//! `lifecycle-verify`, `stress-verify` and `member-failure-tdd` scripts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::mailbox;
use crate::types::{MemberStatus, TaskStatus};

#[derive(Default)]
struct Recorded {
    spawned: Vec<(String, String)>,
    delivered: Vec<(String, String, DeliveryMode)>,
    drained: Vec<String>,
    steered: Vec<String>,
    cancels: usize,
    followups: Vec<String>,
    parked: Vec<String>,
    activity: HashMap<String, MemberActivity>,
    refuse_delivery: bool,
    refuse_spawn: bool,
    invalid_models: Vec<String>,
}

struct FakeHost {
    workspace: PathBuf,
    captain: String,
    state: Mutex<Recorded>,
}

impl FakeHost {
    fn new(workspace: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            workspace,
            captain: "captain-session".to_owned(),
            state: Mutex::new(Recorded::default()),
        })
    }

    fn set_activity(&self, member_id: &str, activity: MemberActivity) {
        self.state
            .lock()
            .activity
            .insert(member_id.to_owned(), activity);
    }
}

impl Host for FakeHost {
    fn captain_id(&self) -> String {
        self.captain.clone()
    }
    fn workspace(&self) -> PathBuf {
        self.workspace.clone()
    }
    fn captain_route(&self) -> CaptainRoute {
        CaptainRoute {
            provider: Some("anthropic".into()),
            model: Some("claude-opus-5-5".into()),
            reasoning_effort: Some("high".into()),
        }
    }
    fn validate_route(&self, route: &MemberRoute) -> Result<(), String> {
        if self.state.lock().invalid_models.contains(&route.model) {
            return Err(format!("unknown member model \"{}\"", route.model));
        }
        Ok(())
    }
    fn member_activity(&self, member_id: &str) -> MemberActivity {
        self.state
            .lock()
            .activity
            .get(member_id)
            .copied()
            .unwrap_or(MemberActivity::Ready)
    }
    fn spawn_member(
        &self,
        _team: &TeamState,
        member: &TeamMember,
        prompt: &str,
    ) -> Result<String, String> {
        let mut state = self.state.lock();
        if state.refuse_spawn {
            return Err("no route".into());
        }
        let id = format!("m-{}", member.name);
        state.spawned.push((member.name.clone(), prompt.to_owned()));
        state.activity.insert(id.clone(), MemberActivity::Running);
        Ok(id)
    }
    fn deliver(
        &self,
        _team: &TeamState,
        member: &TeamMember,
        text: &str,
        mode: DeliveryMode,
    ) -> bool {
        let mut state = self.state.lock();
        if state.refuse_delivery {
            return false;
        }
        state
            .delivered
            .push((member.name.clone(), text.to_owned(), mode));
        state
            .activity
            .insert(member.id.clone(), MemberActivity::Running);
        true
    }
    fn drain_members(&self, member_ids: &[String]) -> Result<(), String> {
        let mut state = self.state.lock();
        for id in member_ids {
            state.drained.push(id.clone());
            state.activity.insert(id.clone(), MemberActivity::Idle);
        }
        Ok(())
    }
    fn steer_captain(&self, text: &str) -> bool {
        self.state.lock().steered.push(text.to_owned());
        true
    }
    fn cancel_captain(&self) {
        self.state.lock().cancels += 1;
    }
    fn followup_captain(&self, text: &str) -> bool {
        self.state.lock().followups.push(text.to_owned());
        true
    }
    fn park_captain_context(&self, text: &str) {
        self.state.lock().parked.push(text.to_owned());
    }
}

const CAPTAIN: &str = "captain-session";

fn setup() -> (tempfile::TempDir, Arc<FakeHost>, TeamRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let host = FakeHost::new(dir.path().to_path_buf());
    let runtime = TeamRuntime::new(host.clone(), RuntimeConfig::default());
    (dir, host, runtime)
}

fn plan_team(runtime: &TeamRuntime, approval: &str) -> CreateResult {
    runtime
        .create(
            CAPTAIN,
            &json!({
                "name": "Audit",
                "description": "audit the repo",
                "approval": approval,
                "plan": {
                    "members": [{"name": "dev", "role": "engineer"}, {"name": "qa"}],
                    "tasks": [
                        {"id": "a", "subject": "survey", "assignee": "dev"},
                        {"id": "b", "subject": "check", "assignee": "qa", "dependencies": ["a"]}
                    ]
                }
            }),
        )
        .unwrap()
}

fn team(runtime: &TeamRuntime) -> TeamState {
    runtime.current_team().unwrap().unwrap()
}

/// Walk a member through claim → in_progress → completed and its report.
fn finish(runtime: &TeamRuntime, host: &FakeHost, member: &str, task_id: &str, output: &str) {
    let member_id = format!("m-{member}");
    let claim = runtime
        .claim_task(&member_id, &json!({ "task_id": task_id }))
        .unwrap();
    let attempt = claim.attempt_id.unwrap();
    runtime
        .update_task(
            &member_id,
            &json!({ "task_id": task_id, "attempt_id": attempt, "status": "in_progress" }),
        )
        .unwrap();
    runtime
        .update_task(
            &member_id,
            &json!({ "task_id": task_id, "attempt_id": attempt, "status": "completed", "output": output }),
        )
        .unwrap();
    runtime
        .send_message(
            &member_id,
            &json!({ "to": "captain", "content": format!("{task_id} done"), "source_task_id": task_id, "source_attempt_id": attempt }),
        )
        .unwrap();
    host.set_activity(&member_id, MemberActivity::Idle);
    runtime.member_status_edge("audit", &member_id, false);
}

#[test]
fn a_staged_plan_spawns_nothing_until_approved() {
    let (_dir, host, runtime) = setup();
    let created = plan_team(&runtime, "required");
    assert_eq!(created.phase, "staged");
    assert_eq!(
        created.tasks.as_ref().unwrap()[1].dependencies,
        vec!["t1".to_owned()]
    );
    assert!(created.render().contains("It is staged"));
    assert!(host.state.lock().spawned.is_empty());
    let staged = team(&runtime);
    assert_eq!(
        staged.member("dev").unwrap().model.as_deref(),
        Some("claude-opus-5-5")
    );
    assert_eq!(
        staged.member("dev").unwrap().reasoning_effort.as_deref(),
        Some("high")
    );

    let approved = runtime.approve_from_panel("audit").unwrap();
    assert_eq!((approved.members, approved.tasks), (2, 2));
    let state = host.state.lock();
    // Only the member whose task is ready starts.
    assert_eq!(state.spawned.len(), 1);
    assert_eq!(state.spawned[0].0, "dev");
    assert!(state.spawned[0].1.contains("Task: t1 [a] — survey"));
    assert!(state.steered[0].contains("approved the staged AgentTeams plan \"Audit\""));
}

#[test]
fn completing_work_unlocks_the_next_member_and_wakes_the_captain() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    assert_eq!(host.state.lock().spawned.len(), 1);
    finish(&runtime, &host, "dev", "t1", "found 3 issues");
    let state = host.state.lock();
    assert!(
        state
            .steered
            .iter()
            .any(|text| text.contains("AgentTeams message from member dev"))
    );
    assert_eq!(state.spawned.len(), 2);
    assert_eq!(state.spawned[1].0, "qa");
    assert!(
        state.spawned[1]
            .1
            .contains("- t1 [a] survey:\n  found 3 issues")
    );
    drop(state);
    let current = team(&runtime);
    assert_eq!(current.member("dev").unwrap().status, MemberStatus::Idle);
    assert_eq!(current.task("t2").unwrap().status, TaskStatus::Claimed);
}

#[test]
fn claimed_cannot_jump_to_completed_and_stale_attempts_are_refused() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let claim = runtime
        .claim_task("m-dev", &json!({"task_id": "t1"}))
        .unwrap();
    let attempt = claim.attempt_id.clone().unwrap();
    let jump = runtime
        .update_task(
            "m-dev",
            &json!({"task_id": "t1", "attempt_id": attempt, "status": "completed"}),
        )
        .unwrap_err();
    assert!(
        jump.contains("cannot move from \"claimed\" to \"completed\""),
        "{jump}"
    );
    let missing = runtime
        .update_task("m-dev", &json!({"task_id": "t1", "status": "in_progress"}))
        .unwrap_err();
    assert!(
        missing.starts_with("missing attempt_id for task t1."),
        "{missing}"
    );

    // The captain hands the task to qa: dev's attempt is revoked and dev is
    // drained before qa starts.
    let reassigned = runtime
        .reassign_task(
            CAPTAIN,
            &json!({"task_id": "t1", "assignee": "qa", "reason": "faster"}),
        )
        .unwrap();
    assert_eq!(reassigned.previous_assignee, "dev");
    assert!(host.state.lock().drained.contains(&"m-dev".to_owned()));
    let stale = runtime
        .update_task(
            "m-dev",
            &json!({"task_id": "t1", "attempt_id": attempt, "status": "in_progress"}),
        )
        .unwrap_err();
    assert!(
        stale.contains("is assigned to \"qa\", not you") || stale.starts_with("stale attempt"),
        "{stale}"
    );
    let current = team(&runtime);
    assert_eq!(current.task("t1").unwrap().assignee.as_deref(), Some("qa"));
    assert!(!current.task("t1").unwrap().is_reassigning());
    assert!(!current.member("dev").unwrap().is_stopping());
}

#[test]
fn a_captain_takeover_lasts_one_turn() {
    let (_dir, _host, runtime) = setup();
    plan_team(&runtime, "automatic");
    runtime
        .reassign_task(CAPTAIN, &json!({"task_id": "t1", "assignee": "captain"}))
        .unwrap();
    let taken = team(&runtime);
    assert_eq!(taken.task("t1").unwrap().status, TaskStatus::InProgress);
    runtime.captain_idle_edge();
    let back = team(&runtime);
    let task = back.task("t1").unwrap();
    // Back in the pool, then claimed again by its next idle member.
    assert_ne!(task.assignee.as_deref(), Some("captain"));
}

#[test]
fn halt_cancels_work_stops_everyone_and_resume_needs_a_reason() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let halted = runtime.halt("audit").unwrap();
    assert_eq!(halted.cancelled_tasks, 2);
    assert_eq!(host.state.lock().cancels, 2);
    assert!(host.state.lock().drained.contains(&"m-dev".to_owned()));
    let current = team(&runtime);
    assert!(current.is_halted());
    assert!(
        current
            .tasks
            .iter()
            .all(|task| task.status == TaskStatus::Cancelled)
    );
    let refused = runtime
        .send_message(CAPTAIN, &json!({"to": "dev", "content": "go"}))
        .unwrap_err();
    assert!(refused.contains("is halted"), "{refused}");
    assert!(runtime.resume(CAPTAIN, &json!({"reason": " "})).is_err());
    let resumed = runtime
        .resume(CAPTAIN, &json!({"reason": "user asked"}))
        .unwrap();
    assert_eq!(resumed.status, "resumed");
    assert_eq!(
        runtime
            .resume(CAPTAIN, &json!({"reason": "again"}))
            .unwrap()
            .status,
        "already_running"
    );
}

#[test]
fn a_restart_recovers_an_open_attempt_exactly_once() {
    let (dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let first = team(&runtime)
        .task("t1")
        .unwrap()
        .attempt_id
        .clone()
        .unwrap();
    drop(runtime);
    // A new process: the member is not resident and nothing was parked.
    host.set_activity("m-dev", MemberActivity::Ready);
    let restarted = TeamRuntime::new(host.clone(), RuntimeConfig::default());
    restarted.resume_after_start();
    let recovered = team(&restarted).task("t1").unwrap().clone();
    assert_ne!(recovered.attempt_id.as_deref(), Some(first.as_str()));
    assert_eq!(recovered.attempt, Some(2));
    // The recovery is parked: further kicks do not spend more attempts.
    host.set_activity("m-dev", MemberActivity::Ready);
    restarted.kick_team("audit");
    restarted.kick_team("audit");
    assert_eq!(team(&restarted).task("t1").unwrap().attempt, Some(2));
    drop(dir);
}

#[test]
fn concurrent_kicks_give_each_task_one_owner() {
    let (_dir, host, runtime) = setup();
    runtime
        .create(
            CAPTAIN,
            &json!({
                "name": "Audit",
                "plan": {
                    "members": [{"name": "a"}, {"name": "b"}, {"name": "c"}],
                    "tasks": [
                        {"id": "x", "subject": "one", "assignee": "a"},
                        {"id": "y", "subject": "two", "assignee": "b"}
                    ]
                }
            }),
        )
        .unwrap();
    // Free the members and add pool work, then let many kicks race.
    for name in ["a", "b", "c"] {
        host.set_activity(&format!("m-{name}"), MemberActivity::Idle);
    }
    for index in 0..4 {
        runtime
            .create_task(CAPTAIN, &json!({"subject": format!("pool {index}")}))
            .unwrap();
    }
    let runtime = Arc::new(runtime);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let runtime = runtime.clone();
            std::thread::spawn(move || runtime.kick_team("audit"))
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let current = team(&runtime);
    for member in ["a", "b", "c"] {
        let open = current
            .tasks
            .iter()
            .filter(|task| {
                task.assignee.as_deref() == Some(member) && task.status.is_open_attempt()
            })
            .count();
        assert!(open <= 1, "{member} owns {open} open tasks");
    }
}

#[test]
fn a_removed_member_can_no_longer_write() {
    let (_dir, _host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let removed = runtime
        .remove_member(CAPTAIN, &json!({"name": "dev"}))
        .unwrap();
    assert_eq!(removed.requeued_tasks, vec!["t1".to_owned()]);
    let refused = runtime
        .claim_task("m-dev", &json!({"task_id": "t1"}))
        .unwrap_err();
    assert!(refused.contains("do not lead or belong"), "{refused}");
    let retired = runtime.root().read_retired_member_ids().unwrap();
    assert!(retired.contains("m-dev"));
}

#[test]
fn a_failed_member_turn_fails_its_task_and_tells_the_captain() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let observed = runtime.observe_member_attempt("audit", "m-dev");
    host.set_activity("m-dev", MemberActivity::Idle);
    assert!(runtime.fail_member_open_attempt(
        "audit",
        "m-dev",
        observed.as_ref(),
        "rate limited (code RATE_LIMIT)"
    ));
    let current = team(&runtime);
    let task = current.task("t1").unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.output.as_deref(),
        Some("rate limited (code RATE_LIMIT)")
    );
    assert!(
        host.state
            .lock()
            .steered
            .iter()
            .any(|text| text.contains("unrecoverable turn failure"))
    );
    // A stale observation (the task moved on) changes nothing.
    assert!(!runtime.fail_member_open_attempt("audit", "m-dev", observed.as_ref(), "late"));
}

#[test]
fn a_failed_review_opens_repair_and_rereview() {
    let (_dir, host, runtime) = setup();
    runtime
        .create(CAPTAIN, &json!({"name": "Audit", "plan": {"members": [{"name": "dev"}, {"name": "qa"}], "tasks": []}}))
        .unwrap();
    let implementation = runtime
        .create_task(
            CAPTAIN,
            &json!({
                "subject": "implement", "kind": "implementation", "assignee": "dev",
                "objective": "add the flag", "acceptance": ["flag works"],
                "inScope": ["src/"], "verify": ["cargo test"]
            }),
        )
        .unwrap();
    let id = implementation.task_id;
    let claim = runtime
        .claim_task("m-dev", &json!({"task_id": id}))
        .unwrap();
    let attempt = claim.attempt_id.unwrap();
    runtime
        .update_task(
            "m-dev",
            &json!({"task_id": id, "attempt_id": attempt, "status": "in_progress"}),
        )
        .unwrap();
    let incomplete = runtime
        .update_task(
            "m-dev",
            &json!({"task_id": id, "attempt_id": attempt, "status": "completed", "output": "done"}),
        )
        .unwrap_err();
    assert!(!incomplete.is_empty());
    runtime
        .update_task(
            "m-dev",
            &json!({
                "task_id": id, "attempt_id": attempt, "status": "completed", "output": "done",
                "changedPaths": ["src/flag.rs"],
                "acceptanceResults": [{"criterion": "flag works", "status": "passed"}],
                "commandsRun": [{"command": "cargo test", "status": "passed", "exitCode": 0}]
            }),
        )
        .unwrap();
    let review = runtime
        .create_task(
            CAPTAIN,
            &json!({
                "subject": "review", "kind": "review", "assignee": "qa", "reviewedTaskId": id,
                "dependencies": [id], "objective": "judge the flag", "acceptance": ["the flag is tested"]
            }),
        )
        .unwrap();
    let rid = review.task_id;
    let claim = runtime
        .claim_task("m-qa", &json!({"task_id": rid}))
        .unwrap();
    let attempt = claim.attempt_id.unwrap();
    runtime
        .update_task(
            "m-qa",
            &json!({"task_id": rid, "attempt_id": attempt, "status": "in_progress"}),
        )
        .unwrap();
    let failed = runtime
        .update_task(
            "m-qa",
            &json!({
                "task_id": rid, "attempt_id": attempt, "status": "failed", "verdict": "needs_revision",
                "findings": [{"id": "F1", "severity": "high", "problem": "no test", "requiredFix": "add a test in src/flag.rs", "file": "src/flag.rs"}]
            }),
        )
        .unwrap();
    let follow_up = failed.follow_up.expect("a follow-up summary");
    assert!(
        follow_up.starts_with(&format!("Automatic quality follow-up for {rid}:")),
        "{follow_up}"
    );
    let current = team(&runtime);
    let kinds: Vec<_> = current.tasks.iter().filter_map(|task| task.kind).collect();
    assert!(kinds.contains(&crate::types::TaskKind::Repair));
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == crate::types::TaskKind::Review)
            .count(),
        2
    );
    assert!(
        host.state
            .lock()
            .steered
            .iter()
            .any(|text| text.contains("Automatic quality follow-up"))
    );
}

#[test]
fn the_panel_controls_revise_and_discard_a_staged_plan() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "required");
    let revised = runtime.continue_staged_planning("audit").unwrap();
    assert!(!revised.already_waiting);
    assert!(
        runtime
            .continue_staged_planning("audit")
            .unwrap()
            .already_waiting
    );
    assert_eq!(
        team(&runtime).plan_review_state,
        Some(crate::types::PlanReviewState::AwaitingFeedback)
    );
    assert!(host.state.lock().followups[0].contains("Return to chat and revise"));
    // Editing the draft puts it back up for review.
    runtime
        .edit_plan(CAPTAIN, &json!({"operations": [{"action": "add_task", "subject": "extra", "dependencies": ["t2"]}]}))
        .unwrap();
    assert_eq!(
        team(&runtime).plan_review_state,
        Some(crate::types::PlanReviewState::AwaitingReview)
    );
    runtime.discard_staged("audit").unwrap();
    assert!(runtime.current_team().unwrap().is_none());
    assert!(
        runtime
            .root()
            .read_archived_team("audit")
            .unwrap()
            .is_some()
    );
    assert!(host.state.lock().parked[0].contains("discarded the staged AgentTeams plan"));
}

#[test]
fn edit_plan_is_atomic() {
    let (_dir, _host, runtime) = setup();
    plan_team(&runtime, "required");
    let error = runtime
        .edit_plan(
            CAPTAIN,
            &json!({"operations": [
                {"action": "add_task", "subject": "fine"},
                {"action": "remove_task", "task_id": "t1"}
            ]}),
        )
        .unwrap_err();
    assert!(error.contains("is still required by \"t2\""), "{error}");
    assert_eq!(team(&runtime).tasks.len(), 2);
}

#[test]
fn messages_are_deduplicated_and_routed() {
    let (_dir, host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let first = runtime
        .send_message(CAPTAIN, &json!({"to": "dev", "content": "use the cache"}))
        .unwrap();
    assert_eq!(first.delivered, "wake");
    let again = runtime
        .send_message(CAPTAIN, &json!({"to": "dev", "content": "use the cache"}))
        .unwrap();
    assert_eq!(again.delivered, "duplicate");
    assert_eq!(again.message_id, first.message_id);
    assert!(
        host.state
            .lock()
            .delivered
            .iter()
            .any(|(name, text, mode)| name == "dev"
                && text.contains("use the cache")
                && *mode == DeliveryMode::Steer)
    );
    let forged = runtime
        .send_message(
            "m-dev",
            &json!({"to": "captain", "content": "x", "from": "qa"}),
        )
        .unwrap_err();
    assert!(forged.contains("must be your own identity"), "{forged}");
}

#[test]
fn status_lists_the_team_and_acknowledges_shown_mail() {
    let (_dir, _host, runtime) = setup();
    plan_team(&runtime, "automatic");
    let team_id = "audit";
    let message = crate::types::TeamMessage::new("dev", crate::key::CAPTAIN_KEY, "fyi");
    mailbox::append(runtime.root(), team_id, crate::key::CAPTAIN_KEY, &message).unwrap();
    let view = runtime.status(CAPTAIN, &Value::Null).unwrap();
    let text = view.render();
    assert!(
        text.starts_with("Team \"Audit\" — audit the repo"),
        "{text}"
    );
    assert!(text.contains("Members (2):"));
    assert!(text.contains("Captain inbox (1):\n  - [dev] fyi"));
    let again = runtime.status(CAPTAIN, &Value::Null).unwrap();
    assert!(again.captain_inbox.is_empty());
}

#[test]
fn a_failed_start_is_recorded_on_the_member() {
    let (_dir, host, runtime) = setup();
    host.state.lock().refuse_spawn = true;
    plan_team(&runtime, "automatic");
    let current = team(&runtime);
    let dev = current.member("dev").unwrap();
    assert!(!dev.is_spawned());
    assert_eq!(dev.spawn_error.as_deref(), Some("no route"));
    // The claim was rolled back to the pool for a later try.
    assert_eq!(current.task("t1").unwrap().status, TaskStatus::Pending);
}

#[test]
fn an_invalid_member_model_is_refused_at_planning() {
    let (_dir, host, runtime) = setup();
    host.state.lock().invalid_models.push("nope".into());
    runtime
        .create(CAPTAIN, &json!({"name": "Audit", "approval": "required"}))
        .unwrap();
    let error = runtime
        .add_member(CAPTAIN, &json!({"name": "dev", "model": "nope"}))
        .unwrap_err();
    assert!(error.contains("unknown member model"), "{error}");
}

#[test]
fn member_routes_follow_the_reference_rules() {
    let captain = CaptainRoute {
        provider: Some("anthropic".into()),
        model: Some("opus".into()),
        reasoning_effort: Some("high".into()),
    };
    let same = resolve_member_route(&captain, &RouteRequest::default(), None).unwrap();
    assert_eq!(same.reasoning_effort.as_deref(), Some("high"));
    let other = resolve_member_route(
        &captain,
        &RouteRequest {
            model: Some("openai::gpt-6-sol".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(
        (other.provider.as_str(), other.model.as_str()),
        ("openai", "gpt-6-sol")
    );
    assert_eq!(other.reasoning_effort, None);
    let forced = resolve_member_route(
        &captain,
        &RouteRequest {
            reasoning_effort: Some("default".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(forced.reasoning_effort, None);
    let configured = resolve_member_route(
        &captain,
        &RouteRequest {
            default_model: Some("deepseek::v4".into()),
            ..Default::default()
        },
        Some("medium"),
    )
    .unwrap();
    assert_eq!(configured.model, "v4");
    assert_eq!(configured.reasoning_effort.as_deref(), Some("medium"));
    assert!(
        resolve_member_route(
            &captain,
            &RouteRequest {
                provider: Some("openai".into()),
                ..Default::default()
            },
            None
        )
        .is_err()
    );
}

#[test]
fn every_tool_name_dispatches() {
    let (_dir, _host, runtime) = setup();
    for spec in super::schema::tool_specs() {
        let error = runtime.call_tool(spec.name, "nobody", &json!({})).err();
        // Each reaches its own argument or identity checks, never "unknown".
        assert!(
            error.is_none_or(|error| !error.starts_with("unknown AgentTeams tool")),
            "{}",
            spec.name
        );
    }
    assert!(
        runtime
            .call_tool("agent_teams_nope", CAPTAIN, &json!({}))
            .is_err()
    );
}
