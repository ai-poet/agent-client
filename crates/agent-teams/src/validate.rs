// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The durable JSON boundary: a record is coerced (legacy shapes, blank
//! optionals), typed, then checked for the invariants the type cannot carry.
//! A record that fails is refused whole — a half-trusted team must never take
//! part in authorizing a tool call.

use std::collections::HashSet;

use anyhow::{Context as _, bail};
use serde_json::{Map, Value};

use crate::key::{CAPTAIN_KEY, sanitize_key};
use crate::types::{
    AcceptanceResult, CommandResult, ReviewFinding, ReviewPolicy, TaskEvidence, TaskRevision,
    TeamMessage, TeamState, TeamTask,
};

/// Optional scalar fields whose persisted values must be non-empty when
/// present. Some models fill optional tool parameters with `""` instead of
/// leaving them out; written as-is, one would brick the team on reload.
const BLANK_SENSITIVE_STRING_FIELDS: [&str; 3] = ["objective", "reviewedTaskId", "sourceTaskId"];
const BLANK_SENSITIVE_STRING_LIST_FIELDS: [&str; 9] = [
    "inScope",
    "outOfScope",
    "acceptance",
    "verify",
    "deliverables",
    "nonGoals",
    "changedPaths",
    "sourceFindingIds",
    "coverageOf",
];

fn is_blank(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.trim().is_empty())
}

/// Blank means absent: blank scalars are dropped, blank entries are filtered
/// out of string lists, and a list holding only blanks is dropped. Everything
/// else passes through, so validation afterwards stays strict.
pub fn normalize_blank_optional_task_fields(task: &mut Map<String, Value>) {
    for key in BLANK_SENSITIVE_STRING_FIELDS {
        if task.get(key).is_some_and(is_blank) {
            task.remove(key);
        }
    }
    for key in BLANK_SENSITIVE_STRING_LIST_FIELDS {
        let Some(Value::Array(items)) = task.get_mut(key) else {
            continue;
        };
        let before = items.len();
        items.retain(|item| !is_blank(item));
        if items.len() == before {
            continue;
        }
        if items.is_empty() {
            task.remove(key);
        }
    }
}

/// Upgrade legacy shapes before typing: a `profile` stored as a bare name, a
/// malformed `profile` (dropped), blank optional task fields, a blank
/// `profileSeedId`.
fn coerce_team_value(value: &mut Value) {
    let Some(record) = value.as_object_mut() else {
        return;
    };
    match record.get("profile") {
        Some(Value::String(name)) => {
            let name = name.trim().to_owned();
            if name.is_empty() {
                record.remove("profile");
            } else {
                record.insert(
                    "profile".to_owned(),
                    Value::Object(Map::from_iter([("name".to_owned(), Value::String(name))])),
                );
            }
        }
        Some(Value::Object(profile)) => {
            let valid = profile
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.trim().is_empty());
            if !valid {
                record.remove("profile");
            }
        }
        Some(Value::Null) | None => {
            record.remove("profile");
        }
        Some(_) => {
            record.remove("profile");
        }
    }
    if let Some(Value::Array(tasks)) = record.get_mut("tasks") {
        for task in tasks.iter_mut() {
            let Some(task) = task.as_object_mut() else {
                continue;
            };
            normalize_blank_optional_task_fields(task);
            let seed_blank = match task.get("profileSeedId") {
                Some(Value::String(seed)) => seed.trim().is_empty(),
                Some(_) => true,
                None => false,
            };
            if seed_blank {
                task.remove("profileSeedId");
            }
        }
    }
}

/// Parse one `team.json` text. `expected_id` is the directory name, which the
/// record's `id` must match.
pub fn parse_team_state(text: &str, expected_id: &str) -> anyhow::Result<TeamState> {
    let mut value: Value = serde_json::from_str(strip_bom(text))
        .with_context(|| format!("invalid AgentTeams state in team \"{expected_id}\""))?;
    coerce_team_value(&mut value);
    let team: TeamState = serde_json::from_value(value)
        .with_context(|| format!("invalid AgentTeams state in team \"{expected_id}\""))?;
    validate_team_state(&team, expected_id)
        .with_context(|| format!("invalid AgentTeams state in team \"{expected_id}\""))?;
    Ok(team)
}

/// Parse one mailbox line; `None` for a line that is not a valid message.
pub fn parse_message_line(line: &str) -> Option<TeamMessage> {
    let message: TeamMessage = serde_json::from_str(strip_bom(line)).ok()?;
    Some(message)
}

/// Remove the optional UTF-8 BOM some editors prepend to JSON text.
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn nonempty_option(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(nonempty)
}

fn nonempty_list(value: &Option<Vec<String>>) -> bool {
    value
        .as_ref()
        .is_none_or(|items| items.iter().all(|item| nonempty(item)))
}

pub fn is_valid_finding(finding: &ReviewFinding) -> bool {
    nonempty(&finding.id)
        && nonempty(&finding.problem)
        && nonempty(&finding.required_fix)
        && nonempty_option(&finding.file)
}

pub fn is_valid_acceptance_result(result: &AcceptanceResult) -> bool {
    nonempty(&result.criterion)
}

pub fn is_valid_command_result(result: &CommandResult) -> bool {
    nonempty(&result.command)
}

pub fn is_valid_revision(revision: &TaskRevision) -> bool {
    nonempty(&revision.by)
        && nonempty(&revision.reason)
        && !revision.fields.is_empty()
        && revision.fields.iter().all(|field| nonempty(field))
}

/// Persisted evidence must stay loadable after a restart.
pub fn is_valid_evidence(evidence: &TaskEvidence) -> bool {
    let acceptance = evidence.acceptance_results.as_deref().unwrap_or_default();
    let commands = evidence.commands_run.as_deref().unwrap_or_default();
    nonempty(&evidence.by)
        && nonempty_option(&evidence.attempt_id)
        && nonempty_option(&evidence.note)
        && acceptance.iter().all(is_valid_acceptance_result)
        && commands.iter().all(is_valid_command_result)
        && (evidence.note.is_some() || !acceptance.is_empty() || !commands.is_empty())
}

/// The review-loop limits: every count at least one, the requirements range
/// ordered, reviewer names non-blank.
pub fn is_valid_review_policy(policy: &ReviewPolicy) -> bool {
    let counts = [
        policy.requirements_min_rounds,
        policy.requirements_max_rounds,
        policy.code_max_rounds,
        policy.max_repair_attempts,
    ];
    if counts.iter().flatten().any(|count| *count < 1) {
        return false;
    }
    let min = policy
        .requirements_min_rounds
        .unwrap_or(crate::quality::DEFAULT_REQUIREMENTS_MIN_ROUNDS);
    let max = policy
        .requirements_max_rounds
        .unwrap_or(crate::quality::DEFAULT_REQUIREMENTS_MAX_ROUNDS);
    if min > max {
        return false;
    }
    nonempty_list(&policy.required_reviewers)
}

/// The quality fields of one task (`hasValidQualityTaskFields`).
pub fn has_valid_quality_task_fields(task: &TeamTask) -> bool {
    if task.round.is_some_and(|round| round < 1) {
        return false;
    }
    if !nonempty_option(&task.objective)
        || !nonempty_option(&task.reviewed_task_id)
        || !nonempty_option(&task.source_task_id)
    {
        return false;
    }
    let lists = [
        &task.in_scope,
        &task.out_of_scope,
        &task.acceptance,
        &task.verify,
        &task.deliverables,
        &task.non_goals,
        &task.changed_paths,
        &task.source_finding_ids,
        &task.coverage_of,
    ];
    if !lists.into_iter().all(nonempty_list) {
        return false;
    }
    if let Some(findings) = &task.findings {
        if !findings.iter().all(is_valid_finding) {
            return false;
        }
        let ids: HashSet<&str> = findings.iter().map(|finding| finding.id.as_str()).collect();
        if ids.len() != findings.len() {
            return false;
        }
    }
    task.acceptance_results
        .as_deref()
        .unwrap_or_default()
        .iter()
        .all(is_valid_acceptance_result)
        && task
            .commands_run
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(is_valid_command_result)
        && task
            .supplemental_evidence
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(is_valid_evidence)
        && task
            .revisions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .all(is_valid_revision)
}

/// The invariants `isTeamState` checks beyond the shape.
pub fn validate_team_state(team: &TeamState, expected_id: &str) -> anyhow::Result<()> {
    if team.id != expected_id {
        bail!("team id \"{}\" does not match its directory", team.id);
    }
    if !nonempty(&team.name) {
        bail!("team name is blank");
    }
    if team.captain_session_id.is_empty() {
        bail!("captain session id is blank");
    }
    if let Some(profile) = &team.profile {
        if !nonempty(&profile.name) {
            bail!("profile name is blank");
        }
        if let Some(policy) = &profile.review_policy
            && !is_valid_review_policy(policy)
        {
            bail!("invalid profile review policy");
        }
    }
    if let Some(policy) = &team.review_policy
        && !is_valid_review_policy(policy)
    {
        bail!("invalid review policy");
    }
    let mut member_ids = HashSet::new();
    let mut member_keys = HashSet::new();
    for member in &team.members {
        if !nonempty(&member.name) {
            bail!("member name is blank");
        }
        let key = sanitize_key(&member.name);
        if key == CAPTAIN_KEY || !member_keys.insert(key) {
            bail!("member \"{}\" collides with another name", member.name);
        }
        if !member.id.is_empty() && !member_ids.insert(member.id.as_str()) {
            bail!("member session id \"{}\" is used twice", member.id);
        }
    }
    let mut task_ids = HashSet::new();
    for task in &team.tasks {
        if task.id.is_empty() || !task_ids.insert(task.id.as_str()) {
            bail!("task id \"{}\" is blank or repeated", task.id);
        }
        if task
            .profile_seed_id
            .as_deref()
            .is_some_and(|seed| !nonempty(seed))
        {
            bail!("task {} has a blank profile seed id", task.id);
        }
        if !has_valid_quality_task_fields(task) {
            bail!("task {} has invalid quality fields", task.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_team() -> Value {
        serde_json::json!({
            "name": "Alpha",
            "id": "alpha",
            "captainSessionId": "cap",
            "createdAt": 1,
            "members": [{"id": "", "name": "dev", "joinedAt": 1, "status": "idle"}],
            "tasks": [{
                "id": "t1", "subject": "do", "status": "pending",
                "dependencies": [], "createdAt": 1, "updatedAt": 1
            }],
            "taskSeq": 1
        })
    }

    #[test]
    fn a_minimal_team_parses_and_round_trips_unknown_keys() {
        let mut value = minimal_team();
        value["futureField"] = serde_json::json!({"x": 1});
        let team = parse_team_state(&value.to_string(), "alpha").unwrap();
        assert_eq!(
            team.extra.get("futureField"),
            Some(&serde_json::json!({"x": 1}))
        );
        let written = serde_json::to_value(&team).unwrap();
        assert_eq!(written["futureField"], serde_json::json!({"x": 1}));
        assert!(written.get("phase").is_none());
    }

    #[test]
    fn a_leading_bom_is_tolerated() {
        let text = format!("\u{feff}{}", minimal_team());
        assert!(parse_team_state(&text, "alpha").is_ok());
    }

    #[test]
    fn a_legacy_string_profile_is_upgraded() {
        let mut value = minimal_team();
        value["profile"] = serde_json::json!(" review ");
        let team = parse_team_state(&value.to_string(), "alpha").unwrap();
        assert_eq!(team.profile.unwrap().name, "review");
    }

    #[test]
    fn blank_optional_task_fields_are_dropped_on_load() {
        let mut value = minimal_team();
        value["tasks"][0]["objective"] = serde_json::json!("  ");
        value["tasks"][0]["inScope"] = serde_json::json!(["", "src/"]);
        value["tasks"][0]["acceptance"] = serde_json::json!([""]);
        value["tasks"][0]["profileSeedId"] = serde_json::json!("");
        let team = parse_team_state(&value.to_string(), "alpha").unwrap();
        let task = &team.tasks[0];
        assert_eq!(task.objective, None);
        assert_eq!(task.in_scope, Some(vec!["src/".to_owned()]));
        assert_eq!(task.acceptance, None);
        assert_eq!(task.profile_seed_id, None);
    }

    #[test]
    fn an_invalid_verdict_refuses_the_whole_team() {
        let mut value = minimal_team();
        value["tasks"][0]["verdict"] = serde_json::json!("maybe");
        assert!(parse_team_state(&value.to_string(), "alpha").is_err());
    }

    #[test]
    fn a_member_named_captain_is_refused() {
        let mut value = minimal_team();
        value["members"][0]["name"] = serde_json::json!("Captain");
        assert!(parse_team_state(&value.to_string(), "alpha").is_err());
    }

    #[test]
    fn a_mismatched_id_is_refused() {
        assert!(parse_team_state(&minimal_team().to_string(), "beta").is_err());
    }

    #[test]
    fn duplicate_finding_ids_are_refused() {
        let mut value = minimal_team();
        let finding = serde_json::json!({
            "id": "F1", "severity": "low", "problem": "p", "requiredFix": "f"
        });
        value["tasks"][0]["findings"] = serde_json::json!([finding.clone(), finding]);
        assert!(parse_team_state(&value.to_string(), "alpha").is_err());
    }

    #[test]
    fn review_policy_rejects_unknown_keys_and_inverted_ranges() {
        let mut value = minimal_team();
        value["reviewPolicy"] = serde_json::json!({"codeMaxRounds": 2, "bogus": 1});
        assert!(parse_team_state(&value.to_string(), "alpha").is_err());
        value["reviewPolicy"] =
            serde_json::json!({"requirementsMinRounds": 5, "requirementsMaxRounds": 2});
        assert!(parse_team_state(&value.to_string(), "alpha").is_err());
    }

    #[test]
    fn malformed_message_lines_are_skipped() {
        assert!(parse_message_line("{not json").is_none());
        assert!(parse_message_line("{}").is_none());
        let line = r#"{"id":"m","from":"a","to":"captain","content":"hi","ts":3}"#;
        assert_eq!(parse_message_line(line).unwrap().content, "hi");
    }
}
