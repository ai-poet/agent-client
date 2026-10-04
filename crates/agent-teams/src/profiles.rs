// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Named team-profile templates: config types, normalization, invocation
//! parsing and the prompt directory.
//!
//! Pure functions only — no I/O, no spawning. A profile is checked from its
//! JSON form so the errors read exactly like the reference plugin's (an
//! unknown key comes with a "did you mean" hint). The typed
//! [`TeamProfileConfig`] keeps keys it does not know in `extra`, so a typo in
//! a hand-edited settings file is reported instead of silently dropped.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::key::{CAPTAIN_KEY, is_captain, sanitize_key};
use crate::types::{ReviewPolicy, TaskPlanning, TeamModelFallback, TeamProfileSnapshot};

/// Hard cap on named profiles so the usage prompt cannot grow without bound.
pub const MAX_TEAM_PROFILES: usize = 16;
/// Hard cap on seed tasks per profile.
pub const MAX_PROFILE_TASKS: usize = 32;
/// Protocol excerpt length (characters) in the prompt directory.
pub const PROFILE_PROTOCOL_PROMPT_LIMIT: usize = 240;

const PROFILE_KEYS: [&str; 8] = [
    "description",
    "protocol",
    "executionPrompt",
    "fallback",
    "members",
    "tasks",
    "taskPlanning",
    "reviewPolicy",
];
const REVIEW_POLICY_KEYS: [&str; 5] = [
    "requirementsMinRounds",
    "requirementsMaxRounds",
    "codeMaxRounds",
    "maxRepairAttempts",
    "requiredReviewers",
];
const MEMBER_KEYS: [&str; 7] = [
    "name",
    "role",
    "provider",
    "model",
    "reasoning_effort",
    "executionPrompt",
    "fallback",
];
const FALLBACK_KEYS: [&str; 2] = ["provider", "model"];
const TASK_KEYS: [&str; 5] = ["id", "subject", "description", "assignee", "dependencies"];

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

// ---------------------------------------------------------------------------
// Config types (the settings file / UI shape)
// ---------------------------------------------------------------------------

/// One member row of a profile template (unresolved).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMemberConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Snake case on the wire, as in the reference config.
    #[serde(
        rename = "reasoning_effort",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
    /// Keys this build does not know; normalization reports them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One seed-task row of a profile template (unresolved).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTaskConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    /// Keys this build does not know; normalization reports them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One named team-profile template.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
    #[serde(default)]
    pub members: Vec<ProfileMemberConfig>,
    /// `captain`: the profile supplies people and guardrails only and the
    /// captain derives the task graph from the goal. `seed` (the default)
    /// expands the fixed `tasks` as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_planning: Option<TaskPlanning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_policy: Option<ReviewPolicy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<ProfileTaskConfig>,
    /// Keys this build does not know; normalization reports them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ---------------------------------------------------------------------------
// Normalized shapes
// ---------------------------------------------------------------------------

/// A profile member after trimming, pairing and reserved-name checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedProfileMember {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<TeamModelFallback>,
}

/// A seed task after assignee canonicalization; `source_index` is its
/// position in the configured list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedProfileTask {
    pub id: String,
    pub subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
    pub source_index: usize,
}

/// A fully validated, topologically ordered team profile.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedTeamProfile {
    /// What a team created from this profile freezes: name, description,
    /// protocol, execution prompt, fallback, planning mode (always set) and
    /// review policy.
    pub snapshot: TeamProfileSnapshot,
    pub members: Vec<NormalizedProfileMember>,
    /// Seed tasks in dependency order (ties in source order); always empty
    /// under captain planning.
    pub tasks: Vec<NormalizedProfileTask>,
}

impl NormalizedTeamProfile {
    pub fn name(&self) -> &str {
        &self.snapshot.name
    }

    pub fn task_planning(&self) -> TaskPlanning {
        self.snapshot.task_planning.unwrap_or(TaskPlanning::Seed)
    }
}

/// The goal plus optional named profile taken from a slash / gesture line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTeamsInvocation {
    pub goal: String,
    pub profile: Option<String>,
}

/// One configured profile after key trim, for listing / lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedTeamProfile<'a> {
    pub name: String,
    pub config: &'a TeamProfileConfig,
}

// ---------------------------------------------------------------------------
// Listing, lookup and the prompt directory
// ---------------------------------------------------------------------------

/// Trim every profile key once, reject empty / colliding keys, and reject
/// more than [`MAX_TEAM_PROFILES`] entries. Profile bodies are not checked
/// here — that belongs to [`resolve_team_profile`].
pub fn list_configured_profiles(
    profiles: &BTreeMap<String, TeamProfileConfig>,
) -> Result<Vec<ListedTeamProfile<'_>>, String> {
    Ok(list_entries(profiles.iter().collect())?
        .into_iter()
        .map(|(name, config)| ListedTeamProfile { name, config })
        .collect())
}

fn list_entries<'a, T>(entries: Vec<(&'a String, &'a T)>) -> Result<Vec<(String, &'a T)>, String> {
    if entries.len() > MAX_TEAM_PROFILES {
        return Err(format!(
            "too many AgentTeams profiles ({}); the limit is {MAX_TEAM_PROFILES}",
            entries.len()
        ));
    }
    let mut seen = HashSet::new();
    let mut listed = Vec::with_capacity(entries.len());
    for (raw_key, config) in entries {
        let name = raw_key.trim();
        if name.is_empty() {
            return Err("configured AgentTeams profiles include an empty key".to_owned());
        }
        if !seen.insert(name) {
            return Err(format!(
                "configured AgentTeams profiles have duplicate key \"{name}\""
            ));
        }
        listed.push((name.to_owned(), config));
    }
    Ok(listed)
}

/// The prompt directory: one line per profile with its member count, task
/// count (or `captain planning`) and a protocol excerpt of at most
/// [`PROFILE_PROTOCOL_PROMPT_LIMIT`] characters. `""` when nothing is
/// configured, so callers can leave the section out.
pub fn format_profiles_for_prompt(
    profiles: &BTreeMap<String, TeamProfileConfig>,
) -> Result<String, String> {
    let listed = list_configured_profiles(profiles)?;
    if listed.is_empty() {
        return Ok(String::new());
    }
    let mut lines =
        vec!["Configured team profiles (pass profile= to agent_teams_create):".to_owned()];
    lines.extend(listed.iter().map(format_profile_listing_line));
    Ok(lines.join("\n"))
}

fn format_profile_listing_line(entry: &ListedTeamProfile<'_>) -> String {
    let graph = match resolve_profile_task_planning(Some(entry.config)) {
        TaskPlanning::Captain => "captain planning".to_owned(),
        TaskPlanning::Seed => count_label(entry.config.tasks.len(), "task"),
    };
    let counts = format!(
        "({}, {graph})",
        count_label(entry.config.members.len(), "member")
    );
    let summary = protocol_summary(entry.config.protocol.as_deref())
        .or_else(|| protocol_summary(entry.config.description.as_deref()));
    match summary {
        Some(summary) => format!("- {} {counts}: {summary}", entry.name),
        None => format!("- {} {counts}", entry.name),
    }
}

/// The planning mode a profile asks for; anything but `captain` is `seed`.
pub fn resolve_profile_task_planning(config: Option<&TeamProfileConfig>) -> TaskPlanning {
    match config.and_then(|config| config.task_planning) {
        Some(TaskPlanning::Captain) => TaskPlanning::Captain,
        _ => TaskPlanning::Seed,
    }
}

fn count_label(count: usize, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

fn protocol_summary(protocol: Option<&str>) -> Option<String> {
    let collapsed = protocol?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(
        collapsed
            .chars()
            .take(PROFILE_PROTOCOL_PROMPT_LIMIT)
            .collect(),
    )
}

/// Normalize and pre-validate one named profile out of `profiles`. Failures
/// come back before any caller should create a directory or spawn members.
pub fn resolve_team_profile(
    profiles: &BTreeMap<String, TeamProfileConfig>,
    profile_name: &str,
    max_members: usize,
) -> Result<NormalizedTeamProfile, String> {
    let listed = list_configured_profiles(profiles)?;
    let name = profile_name.trim();
    if name.is_empty() {
        return Err("AgentTeams profile name must be a non-empty string".to_owned());
    }
    let Some(entry) = listed.iter().find(|entry| entry.name == name) else {
        let available: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
        let shown = if available.is_empty() {
            "(none)".to_owned()
        } else {
            available.join(", ")
        };
        return Err(format!(
            "unknown AgentTeams profile \"{name}\" — configured profiles: {shown}"
        ));
    };
    normalize_listed_profile(name, &profile_value(entry.config)?, max_members)
}

/// Check one typed profile body against the hard member ceiling
/// ([`crate::config::MAX_MEMBERS_LIMIT`]). Use [`resolve_team_profile`] or
/// [`normalize_team_profile_value`] to apply a configured `maxMembers`.
pub fn normalize_team_profile(
    name: &str,
    profile: &TeamProfileConfig,
) -> Result<NormalizedTeamProfile, String> {
    normalize_team_profile_value(
        name,
        &profile_value(profile)?,
        crate::config::MAX_MEMBERS_LIMIT as usize,
    )
}

/// Check one profile body given as JSON — a configured profile or an inline
/// plan from a tool call — with the reference plugin's exact errors.
pub fn normalize_team_profile_value(
    name: &str,
    value: &Value,
    max_members: usize,
) -> Result<NormalizedTeamProfile, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("AgentTeams profile name must be a non-empty string".to_owned());
    }
    normalize_listed_profile(name, value, max_members)
}

/// Read a `profiles` map from JSON (for example a settings import), checking
/// the map and every body with the reference plugin's errors before typing
/// it. `null` reads as no profiles. Keys are kept as written.
pub fn parse_profiles(value: &Value) -> Result<BTreeMap<String, TeamProfileConfig>, String> {
    let record = match value {
        Value::Null => return Ok(BTreeMap::new()),
        Value::Object(record) => record,
        _ => return Err("AgentTeams profiles must be an object map of named templates".to_owned()),
    };
    list_entries(record.iter().collect())?;
    let mut profiles = BTreeMap::new();
    for (raw_key, raw) in record {
        let name = raw_key.trim();
        normalize_listed_profile(name, raw, crate::config::MAX_MEMBERS_LIMIT as usize)?;
        let config: TeamProfileConfig = serde_json::from_value(raw.clone())
            .map_err(|error| format!("profiles.{name}: {error}"))?;
        profiles.insert(raw_key.clone(), config);
    }
    Ok(profiles)
}

fn profile_value(profile: &TeamProfileConfig) -> Result<Value, String> {
    serde_json::to_value(profile).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Invocation parsing
// ---------------------------------------------------------------------------

/// Walk `raw_input` from the front and eat standalone profile flags. Only
/// `--profile <name>`, `--profile=<name>` and `profile=<name>` count; the
/// first ordinary token stops the scan so a mid-sentence `profile=` stays in
/// the goal. A leading ordinary token is never treated as a profile name.
///
/// `--profile "name"` strips one matching pair of quotes. A repeated flag and
/// a `--profile` with no name are errors.
pub fn parse_profile_invocation(raw_input: &str) -> Result<AgentTeamsInvocation, String> {
    let tokens: Vec<&str> = raw_input.split_whitespace().collect();
    let mut index = 0;
    let mut profile = None;
    while let Some(token) = tokens.get(index) {
        let Some((name, consumed)) = parse_leading_profile_flag(token, tokens.get(index + 1))?
        else {
            break;
        };
        if profile.is_some() {
            return Err("duplicate AgentTeams profile flag".to_owned());
        }
        profile = Some(name);
        index += consumed;
    }
    Ok(AgentTeamsInvocation {
        goal: tokens[index..].join(" "),
        profile,
    })
}

fn parse_leading_profile_flag(
    token: &str,
    next_token: Option<&&str>,
) -> Result<Option<(String, usize)>, String> {
    if token == "--profile" {
        let Some(next) = next_token else {
            return Err("--profile flag is missing a profile name".to_owned());
        };
        return Ok(Some((read_profile_token(next)?, 2)));
    }
    if let Some(rest) = token.strip_prefix("--profile=") {
        return Ok(Some((read_profile_token(rest)?, 1)));
    }
    if let Some(rest) = token.strip_prefix("profile=") {
        return Ok(Some((read_profile_token(rest)?, 1)));
    }
    Ok(None)
}

fn read_profile_token(raw: &str) -> Result<String, String> {
    let name = strip_one_quote_pair(raw).trim();
    if name.is_empty() {
        return Err("--profile flag is missing a profile name".to_owned());
    }
    Ok(name.to_owned())
}

/// Strip a single matching pair of `"` or `'`; unmatched quotes stay.
fn strip_one_quote_pair(value: &str) -> &str {
    if value.len() < 2 {
        return value;
    }
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

// ---------------------------------------------------------------------------
// Body normalization
// ---------------------------------------------------------------------------

fn normalize_listed_profile(
    name: &str,
    value: &Value,
    max_members: usize,
) -> Result<NormalizedTeamProfile, String> {
    let path = format!("profiles.{name}");
    let raw = as_record(Some(value), &path)?;
    assert_allowed_keys(raw, &PROFILE_KEYS, &path)?;

    let description =
        optional_non_empty_string(raw.get("description"), &format!("{path}.description"))?;
    let protocol = optional_non_empty_string(raw.get("protocol"), &format!("{path}.protocol"))?;
    let execution_prompt = optional_non_empty_string(
        raw.get("executionPrompt"),
        &format!("{path}.executionPrompt"),
    )?;
    let fallback = normalize_fallback(raw.get("fallback"), &format!("{path}.fallback"))?;
    let task_planning =
        normalize_task_planning(raw.get("taskPlanning"), &format!("{path}.taskPlanning"))?;
    let review_policy =
        normalize_review_policy(raw.get("reviewPolicy"), &format!("{path}.reviewPolicy"))?;
    let members_raw = match raw.get("members") {
        Some(Value::Array(items)) if !items.is_empty() => items,
        _ => return Err(format!("AgentTeams profile \"{name}\" has no members")),
    };
    if members_raw.len() > max_members {
        return Err(format!(
            "profile \"{name}\" has {} members but maxMembers is {max_members}",
            members_raw.len()
        ));
    }

    let mut members = Vec::with_capacity(members_raw.len());
    let mut member_by_name: HashMap<String, String> = HashMap::new();
    let mut member_by_key: HashMap<String, String> = HashMap::new();
    for (index, item) in members_raw.iter().enumerate() {
        let member = normalize_member(item, &format!("{path}.members[{index}]"), name)?;
        let key = sanitize_key(&member.name);
        if let Some(colliding) = member_by_key.get(&key) {
            return Err(format!(
                "profile members \"{colliding}\" and \"{}\" collapse to the same name",
                member.name
            ));
        }
        member_by_name.insert(member.name.clone(), member.name.clone());
        member_by_key.insert(key, member.name.clone());
        members.push(member);
    }

    let snapshot = TeamProfileSnapshot {
        name: name.to_owned(),
        description,
        protocol,
        execution_prompt,
        fallback,
        task_planning: Some(task_planning),
        review_policy,
    };

    let tasks_raw = match raw.get("tasks") {
        None => {
            return Ok(NormalizedTeamProfile {
                snapshot,
                members,
                tasks: Vec::new(),
            });
        }
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("{path}.tasks must be an array")),
    };
    if tasks_raw.len() > MAX_PROFILE_TASKS {
        return Err(format!(
            "profile \"{name}\" has {} tasks but the limit is {MAX_PROFILE_TASKS}",
            tasks_raw.len()
        ));
    }

    let require_assignee = !tasks_raw.is_empty();
    let mut draft_tasks = Vec::with_capacity(tasks_raw.len());
    let mut task_ids: HashSet<String> = HashSet::new();
    let mut task_by_key: HashMap<String, String> = HashMap::new();
    for (index, item) in tasks_raw.iter().enumerate() {
        let task = normalize_task(
            item,
            &format!("{path}.tasks[{index}]"),
            name,
            index,
            &member_by_name,
            &member_by_key,
            require_assignee,
        )?;
        if task_ids.contains(&task.id) {
            return Err(format!(
                "profile \"{name}\" has duplicate task id \"{}\"",
                task.id
            ));
        }
        let key = sanitize_key(&task.id);
        if let Some(colliding) = task_by_key.get(&key) {
            return Err(format!(
                "profile tasks \"{colliding}\" and \"{}\" collapse to the same id",
                task.id
            ));
        }
        task_ids.insert(task.id.clone());
        task_by_key.insert(key, task.id.clone());
        draft_tasks.push(task);
    }

    let tasks = topo_sort_tasks(draft_tasks)?;
    Ok(NormalizedTeamProfile {
        snapshot,
        members,
        tasks: match task_planning {
            TaskPlanning::Captain => Vec::new(),
            TaskPlanning::Seed => tasks,
        },
    })
}

fn normalize_task_planning(value: Option<&Value>, path: &str) -> Result<TaskPlanning, String> {
    match value {
        None => Ok(TaskPlanning::Seed),
        Some(Value::String(text)) if text == "captain" => Ok(TaskPlanning::Captain),
        Some(Value::String(text)) if text == "seed" => Ok(TaskPlanning::Seed),
        Some(_) => Err(format!("{path} must be \"captain\" or \"seed\"")),
    }
}

fn normalize_review_policy(
    value: Option<&Value>,
    path: &str,
) -> Result<Option<ReviewPolicy>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = as_record(Some(value), path)?;
    assert_allowed_keys(raw, &REVIEW_POLICY_KEYS, path)?;
    let requirements_min_rounds = optional_positive_int(
        raw.get("requirementsMinRounds"),
        &format!("{path}.requirementsMinRounds"),
    )?;
    let requirements_max_rounds = optional_positive_int(
        raw.get("requirementsMaxRounds"),
        &format!("{path}.requirementsMaxRounds"),
    )?;
    let code_max_rounds =
        optional_positive_int(raw.get("codeMaxRounds"), &format!("{path}.codeMaxRounds"))?;
    let max_repair_attempts = optional_positive_int(
        raw.get("maxRepairAttempts"),
        &format!("{path}.maxRepairAttempts"),
    )?;
    if let (Some(min), Some(max)) = (requirements_min_rounds, requirements_max_rounds)
        && min > max
    {
        return Err(format!(
            "{path}.requirementsMinRounds must be <= requirementsMaxRounds"
        ));
    }
    let required_reviewers = match raw.get("requiredReviewers") {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| match item.as_str().map(str::trim) {
                    Some(reviewer) if !reviewer.is_empty() => Ok(reviewer.to_owned()),
                    _ => Err(format!(
                        "{path}.requiredReviewers[{index}] must be a non-empty string"
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => {
            return Err(format!(
                "{path}.requiredReviewers must be an array of strings"
            ));
        }
    };
    let policy = ReviewPolicy {
        requirements_min_rounds,
        requirements_max_rounds,
        code_max_rounds,
        max_repair_attempts,
        required_reviewers,
    };
    // One bound alone is checked against the other's default as well: the
    // team record refuses an inverted effective range on reload.
    if !crate::validate::is_valid_review_policy(&policy) {
        return Err(format!(
            "{path}.requirementsMinRounds must be <= requirementsMaxRounds (defaults: {}..={})",
            crate::quality::DEFAULT_REQUIREMENTS_MIN_ROUNDS,
            crate::quality::DEFAULT_REQUIREMENTS_MAX_ROUNDS
        ));
    }
    Ok(Some(policy))
}

/// `Number.isSafeInteger(value) && value >= 1`, bounded to `u32` here.
fn optional_positive_int(value: Option<&Value>, path: &str) -> Result<Option<u32>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let integer = match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.fract() == 0.0 && *float >= 1.0 && *float <= MAX_SAFE_INTEGER)
                .map(|float| float as u64)
        }),
        _ => None,
    };
    match integer.filter(|integer| *integer >= 1).map(u32::try_from) {
        Some(Ok(integer)) => Ok(Some(integer)),
        _ => Err(format!("{path} must be a positive integer")),
    }
}

fn normalize_member(
    value: &Value,
    path: &str,
    profile_name: &str,
) -> Result<NormalizedProfileMember, String> {
    let raw = as_record(Some(value), path)?;
    assert_allowed_keys(raw, &MEMBER_KEYS, path)?;
    let name = required_non_empty_string(raw.get("name"), &format!("{path}.name"), || {
        format!("profile \"{profile_name}\" has a member with an empty name")
    })?;
    if is_captain_name(&name) {
        return Err(format!(
            "member name \"{name}\" is reserved for the captain"
        ));
    }
    let role = optional_non_empty_string(raw.get("role"), &format!("{path}.role"))?;
    let provider = optional_non_empty_string(raw.get("provider"), &format!("{path}.provider"))?;
    let model = optional_non_empty_string(raw.get("model"), &format!("{path}.model"))?;
    let reasoning_effort = optional_non_empty_string(
        raw.get("reasoning_effort"),
        &format!("{path}.reasoning_effort"),
    )?;
    let execution_prompt = optional_non_empty_string(
        raw.get("executionPrompt"),
        &format!("{path}.executionPrompt"),
    )?;
    let fallback = normalize_fallback(raw.get("fallback"), &format!("{path}.fallback"))?;
    if provider.is_some() && model.is_none() {
        return Err(format!(
            "profile member \"{name}\" sets provider without model"
        ));
    }
    Ok(NormalizedProfileMember {
        name,
        role,
        provider,
        model,
        reasoning_effort,
        execution_prompt,
        fallback,
    })
}

fn normalize_fallback(
    value: Option<&Value>,
    path: &str,
) -> Result<Option<TeamModelFallback>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let raw = as_record(Some(value), path)?;
    assert_allowed_keys(raw, &FALLBACK_KEYS, path)?;
    let provider =
        required_non_empty_string(raw.get("provider"), &format!("{path}.provider"), || {
            format!("{path}.provider must not be empty")
        })?;
    let model = required_non_empty_string(raw.get("model"), &format!("{path}.model"), || {
        format!("{path}.model must not be empty")
    })?;
    Ok(Some(TeamModelFallback { provider, model }))
}

fn normalize_task(
    value: &Value,
    path: &str,
    profile_name: &str,
    source_index: usize,
    member_by_name: &HashMap<String, String>,
    member_by_key: &HashMap<String, String>,
    require_assignee: bool,
) -> Result<NormalizedProfileTask, String> {
    let raw = as_record(Some(value), path)?;
    assert_allowed_keys(raw, &TASK_KEYS, path)?;
    let id = required_non_empty_string(raw.get("id"), &format!("{path}.id"), || {
        format!("profile \"{profile_name}\" has a task with an empty id")
    })?;
    let subject =
        required_non_empty_string(raw.get("subject"), &format!("{path}.subject"), || {
            format!("profile task \"{id}\" is missing a subject")
        })?;
    let description =
        optional_non_empty_string(raw.get("description"), &format!("{path}.description"))?;
    let dependencies = normalize_dependencies(
        raw.get("dependencies"),
        &format!("{path}.dependencies"),
        &id,
    )?;
    let assignee = resolve_task_assignee(
        raw.get("assignee"),
        &format!("{path}.assignee"),
        &id,
        require_assignee,
        member_by_name,
        member_by_key,
    )?;
    Ok(NormalizedProfileTask {
        id,
        subject,
        description,
        assignee,
        dependencies,
        source_index,
    })
}

fn normalize_dependencies(
    value: Option<&Value>,
    path: &str,
    task_id: &str,
) -> Result<Vec<String>, String> {
    let items = match value {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("{path} must be an array of task ids")),
    };
    let mut dependencies = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let Some(item) = item.as_str() else {
            return Err(format!("{path}[{index}] must be a string"));
        };
        let dependency = item.trim();
        if dependency.is_empty() {
            return Err(format!("{path}[{index}] must not be empty"));
        }
        if dependency == task_id {
            return Err(format!(
                "profile task \"{task_id}\" cannot depend on itself"
            ));
        }
        if seen.insert(dependency) {
            dependencies.push(dependency.to_owned());
        }
    }
    Ok(dependencies)
}

fn resolve_task_assignee(
    value: Option<&Value>,
    path: &str,
    task_id: &str,
    required: bool,
    member_by_name: &HashMap<String, String>,
    member_by_key: &HashMap<String, String>,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        if required {
            return Err(format!("profile task \"{task_id}\" is missing an assignee"));
        }
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(format!("{path} must be a string"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("profile task \"{task_id}\" is missing an assignee"));
    }
    if is_captain_name(trimmed) {
        return Err(format!(
            "profile task \"{task_id}\" cannot assign work to the captain"
        ));
    }
    if let Some(name) = member_by_name.get(trimmed) {
        return Ok(Some(name.clone()));
    }
    if let Some(name) = member_by_key.get(&sanitize_key(trimmed)) {
        return Ok(Some(name.clone()));
    }
    Err(format!(
        "profile task \"{task_id}\" assignee \"{trimmed}\" is not a profile member"
    ))
}

/// Kahn's algorithm; among ready tasks the one listed first goes first.
fn topo_sort_tasks(
    tasks: Vec<NormalizedProfileTask>,
) -> Result<Vec<NormalizedProfileTask>, String> {
    let position: HashMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.id.as_str(), index))
        .collect();
    for task in &tasks {
        for dependency in &task.dependencies {
            if !position.contains_key(dependency.as_str()) {
                return Err(format!(
                    "profile task \"{}\" depends on unknown task \"{dependency}\"",
                    task.id
                ));
            }
        }
    }

    let mut indegree = vec![0usize; tasks.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    for (index, task) in tasks.iter().enumerate() {
        for dependency in &task.dependencies {
            indegree[index] += 1;
            outgoing[position[dependency.as_str()]].push(index);
        }
    }

    let by_source = |index: &usize| tasks[*index].source_index;
    let mut ready: Vec<usize> = (0..tasks.len())
        .filter(|index| indegree[*index] == 0)
        .collect();
    ready.sort_by_key(by_source);
    let mut ordered = Vec::with_capacity(tasks.len());
    while !ready.is_empty() {
        let next = ready.remove(0);
        ordered.push(next);
        for &child in &outgoing[next] {
            indegree[child] -= 1;
            if indegree[child] == 0 {
                ready.push(child);
                ready.sort_by_key(by_source);
            }
        }
    }

    if ordered.len() != tasks.len() {
        let done: HashSet<usize> = ordered.iter().copied().collect();
        let cyclic: Vec<&str> = tasks
            .iter()
            .enumerate()
            .filter(|(index, _)| !done.contains(index))
            .map(|(_, task)| task.id.as_str())
            .collect();
        return Err(format_cycle_error(&cyclic));
    }
    let mut slots: Vec<Option<NormalizedProfileTask>> = tasks.into_iter().map(Some).collect();
    Ok(ordered
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect())
}

fn format_cycle_error(cyclic: &[&str]) -> String {
    match cyclic {
        [] => "profile task \"unknown\" forms a dependency cycle".to_owned(),
        [only] => format!("profile task \"{only}\" forms a dependency cycle"),
        [first, second] => {
            format!("profile task \"{first}\" and \"{second}\" form a dependency cycle")
        }
        [head @ .., tail] => {
            let head: Vec<String> = head.iter().map(|id| format!("\"{id}\"")).collect();
            format!(
                "profile tasks {}, and \"{tail}\" form a dependency cycle",
                head.join(", ")
            )
        }
    }
}

fn is_captain_name(name: &str) -> bool {
    name.trim().to_lowercase() == CAPTAIN_KEY || is_captain(name)
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn as_record<'a>(value: Option<&'a Value>, path: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{path} must be an object"))
}

fn assert_allowed_keys(
    value: &Map<String, Value>,
    allowed: &[&'static str],
    path: &str,
) -> Result<(), String> {
    for key in value.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        let hint = suggest_field(key, allowed)
            .map(|suggestion| format!("; did you mean {suggestion}?"))
            .unwrap_or_default();
        return Err(format!("{path}.{key} is unknown{hint}"));
    }
    Ok(())
}

fn suggest_field(unknown: &str, allowed: &[&'static str]) -> Option<&'static str> {
    let lower = unknown.to_lowercase();
    if let Some(exact) = allowed
        .iter()
        .find(|candidate| candidate.to_lowercase() == lower)
    {
        return Some(*exact);
    }
    let mut best = None;
    let mut best_distance = usize::MAX;
    for candidate in allowed {
        let distance = levenshtein(&lower, &candidate.to_lowercase());
        if distance < best_distance {
            best_distance = distance;
            best = Some(*candidate);
        }
    }
    best.filter(|_| best_distance <= 2)
}

fn levenshtein(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.iter().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (col, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[col + 1] = (previous[col + 1] + 1)
                .min(current[col] + 1)
                .min(previous[col] + cost);
        }
        previous = current;
    }
    previous[right.len()]
}

fn required_non_empty_string(
    value: Option<&Value>,
    path: &str,
    empty_message: impl FnOnce() -> String,
) -> Result<String, String> {
    let Some(text) = value.and_then(Value::as_str) else {
        return Err(format!("{path} must be a string"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(empty_message());
    }
    Ok(trimmed.to_owned())
}

fn optional_non_empty_string(value: Option<&Value>, path: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(format!("{path} must be a string"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{path} must not be empty"));
    }
    Ok(Some(trimmed.to_owned()))
}

// ---------------------------------------------------------------------------
// Built-in profiles
// ---------------------------------------------------------------------------

/// The profiles a fresh install starts with (the former workflow templates).
/// None sets a provider or model: members inherit the captain's route.
pub fn builtin_profiles() -> BTreeMap<String, TeamProfileConfig> {
    BTreeMap::from([
        ("prd-implement-review".to_owned(), prd_implement_review()),
        ("implement-test-fix".to_owned(), implement_test_fix()),
        ("dual-review".to_owned(), dual_review()),
    ])
}

fn builtin_member(name: &str, role: &str) -> ProfileMemberConfig {
    ProfileMemberConfig {
        name: name.to_owned(),
        role: Some(role.to_owned()),
        ..ProfileMemberConfig::default()
    }
}

fn builtin_task(
    id: &str,
    subject: &str,
    assignee: &str,
    dependencies: &[&str],
    description: &str,
) -> ProfileTaskConfig {
    ProfileTaskConfig {
        id: id.to_owned(),
        subject: subject.to_owned(),
        description: Some(description.to_owned()),
        assignee: Some(assignee.to_owned()),
        dependencies: dependencies.iter().map(|id| (*id).to_owned()).collect(),
        extra: Map::new(),
    }
}

fn prd_implement_review() -> TeamProfileConfig {
    TeamProfileConfig {
        description: Some(
            "Write a PRD for the goal, implement it, then review the changes against it."
                .to_owned(),
        ),
        protocol: Some(
            "Fixed three-step workflow. The planner writes the requirements to docs/prd.md and \
             writes no code; the engineer implements what the PRD asks for; the reviewer checks \
             the changes against the PRD without editing files and ends with a clear verdict."
                .to_owned(),
        ),
        task_planning: Some(TaskPlanning::Seed),
        members: vec![
            builtin_member(
                "planner",
                "Product planner: turns the goal into a concise PRD; never writes code.",
            ),
            builtin_member(
                "engineer",
                "Engineer: implements the PRD with focused changes that follow the project's conventions.",
            ),
            builtin_member(
                "reviewer",
                "Reviewer: checks the changes against the PRD; never edits files.",
            ),
        ],
        tasks: vec![
            builtin_task(
                "prd",
                "Draft the PRD",
                "planner",
                &[],
                "Write a concise product requirements document for the team goal into \
                 docs/prd.md: scope, user-facing behaviour, acceptance criteria, and what is \
                 explicitly out of scope. Do not write code.",
            ),
            builtin_task(
                "implement",
                "Implement the PRD",
                "engineer",
                &["prd"],
                "Implement the requirements in docs/prd.md. Keep changes focused, follow the \
                 project's conventions, and finish with a short summary of what you changed.",
            ),
            builtin_task(
                "review",
                "Review against the PRD",
                "reviewer",
                &["implement"],
                "Review the uncommitted changes in the workspace against docs/prd.md. Report \
                 bugs, missing acceptance criteria, and leftovers. Do not edit files; end with a \
                 clear verdict.",
            ),
        ],
        ..TeamProfileConfig::default()
    }
}

fn implement_test_fix() -> TeamProfileConfig {
    TeamProfileConfig {
        description: Some(
            "Implement, verify and review with an automatic repair loop; the captain plans the graph."
                .to_owned(),
        ),
        protocol: Some(
            "Captain-planned quality workflow. While the team is staged, plan the full quality \
             order: implementation (engineer) → verification (tester) → review (reviewer) → \
             integration, with integration depending on review round 1. Derive inScope and verify \
             commands from the workspace. The tester writes or extends tests, runs the project's \
             suite and reports exactly what fails; the reviewer judges the latest implementation \
             together with those results and returns needs_revision with findings when anything \
             fails. Do not plan fix tasks up front: a needs_revision review automatically adds a \
             repair task and the next review round, and rewires pending downstream gates."
                .to_owned(),
        ),
        task_planning: Some(TaskPlanning::Captain),
        members: vec![
            builtin_member(
                "engineer",
                "Engineer: implements the goal and repairs review findings with focused changes.",
            ),
            builtin_member(
                "tester",
                "Tester: writes or extends tests, runs the suite and reports exactly what fails.",
            ),
            builtin_member(
                "reviewer",
                "Reviewer: judges the latest implementation and test results; never edits files.",
            ),
        ],
        ..TeamProfileConfig::default()
    }
}

fn dual_review() -> TeamProfileConfig {
    TeamProfileConfig {
        description: Some(
            "Implement, review from two angles in parallel, then merge the reviews.".to_owned(),
        ),
        protocol: Some(
            "Fixed workflow. The engineer implements the goal; reviewer-a (correctness and edge \
             cases) and reviewer-b (readability, naming, consistency) review the changes in \
             parallel without editing files; the summarizer merges both reviews into one \
             prioritized list of changes."
                .to_owned(),
        ),
        task_planning: Some(TaskPlanning::Seed),
        members: vec![
            builtin_member(
                "engineer",
                "Engineer: implements the goal with focused changes.",
            ),
            builtin_member(
                "reviewer-a",
                "Reviewer for correctness and edge cases; never edits files.",
            ),
            builtin_member(
                "reviewer-b",
                "Reviewer for readability, naming and consistency with the codebase; never edits files.",
            ),
            builtin_member(
                "summarizer",
                "Summarizer: merges the reviews into one prioritized list; never edits files.",
            ),
        ],
        tasks: vec![
            builtin_task(
                "implement",
                "Implement",
                "engineer",
                &[],
                "Implement the team goal. Keep changes focused and finish with a short summary.",
            ),
            builtin_task(
                "review-a",
                "Review correctness",
                "reviewer-a",
                &["implement"],
                "Review the uncommitted changes in the workspace for correctness and edge cases. \
                 Do not edit files.",
            ),
            builtin_task(
                "review-b",
                "Review readability",
                "reviewer-b",
                &["implement"],
                "Review the uncommitted changes in the workspace for readability, naming, and \
                 consistency with the codebase. Do not edit files.",
            ),
            builtin_task(
                "summary",
                "Summarize the reviews",
                "summarizer",
                &["review-a", "review-b"],
                "Combine the two reviews into one prioritized list of changes to make. Do not \
                 edit files.",
            ),
        ],
        ..TeamProfileConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profiles(value: Value) -> BTreeMap<String, TeamProfileConfig> {
        serde_json::from_value(value).unwrap()
    }

    fn resolve_json(value: Value, name: &str) -> Result<NormalizedTeamProfile, String> {
        normalize_team_profile_value(name, &value, 8)
    }

    fn demo_profiles() -> BTreeMap<String, TeamProfileConfig> {
        profiles(json!({
            " demo ": {
                "protocol": "a".repeat(300),
                "members": [
                    {"name": " Implementer ", "role": "builder", "model": "m"},
                    {"name": "Reviewer", "model": "r"}
                ],
                "tasks": [
                    {"id": "design", "subject": "Design", "assignee": "implementer"},
                    {"id": "review", "subject": "Review", "assignee": " reviewer ", "dependencies": ["design"]}
                ]
            }
        }))
    }

    // --- verify.mjs ---------------------------------------------------------

    #[test]
    fn profile_keys_trim_and_assignees_canonicalize() {
        let normalized = resolve_team_profile(&demo_profiles(), "demo", 8).unwrap();
        assert_eq!(normalized.name(), "demo");
        assert_eq!(normalized.members[0].name, "Implementer");
        assert_eq!(normalized.tasks[1].assignee.as_deref(), Some("Reviewer"));
        assert_eq!(normalized.tasks[0].id, "design");
        assert_eq!(normalized.tasks[1].id, "review");
        assert_eq!(normalized.task_planning(), TaskPlanning::Seed);
        assert_eq!(normalized.snapshot.task_planning, Some(TaskPlanning::Seed));
    }

    #[test]
    fn profile_invocation_supports_equals_and_leaves_mid_goal_text() {
        let parsed = parse_profile_invocation("--profile=demo ship it").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo"));
        assert_eq!(parsed.goal, "ship it");
        let parsed = parse_profile_invocation("research profile=prod config").unwrap();
        assert_eq!(parsed.goal, "research profile=prod config");
        assert_eq!(parsed.profile, None);
    }

    #[test]
    fn profile_prompt_truncates_the_protocol() {
        let text = format_profiles_for_prompt(&demo_profiles()).unwrap();
        assert!(text.contains("demo"));
        assert!(text.len() < 400, "{text}");
        assert!(text.contains("- demo (2 members, 2 tasks): "));
        assert!(text.ends_with(&"a".repeat(PROFILE_PROTOCOL_PROMPT_LIMIT)));
        assert!(!text.contains(&"a".repeat(PROFILE_PROTOCOL_PROMPT_LIMIT + 1)));
    }

    #[test]
    fn profile_prompt_is_empty_without_profiles() {
        assert_eq!(format_profiles_for_prompt(&BTreeMap::new()).unwrap(), "");
    }

    #[test]
    fn profile_directory_falls_back_to_the_description() {
        let text = format_profiles_for_prompt(&profiles(json!({
            "named": {"description": "  Review\n  the UI  ", "members": [{"name": "reviewer"}]}
        })))
        .unwrap();
        assert!(text.contains("Review the UI"), "{text}");
        assert!(text.contains("- named (1 member, 0 tasks): Review the UI"));
    }

    #[test]
    fn captain_planning_keeps_the_roster_and_drops_seed_tasks() {
        let normalized = resolve_json(
            json!({
                "taskPlanning": "captain",
                "members": [{"name": "analyst", "model": "a"}, {"name": "reviewer", "model": "r"}],
                "tasks": [
                    {"id": "requirements", "subject": "Requirements", "assignee": "analyst"},
                    {"id": "review", "subject": "Review", "assignee": "reviewer", "dependencies": ["requirements"]}
                ]
            }),
            "dynamic",
        )
        .unwrap();
        assert_eq!(normalized.task_planning(), TaskPlanning::Captain);
        assert_eq!(normalized.members.len(), 2);
        assert!(normalized.tasks.is_empty());
    }

    #[test]
    fn profile_prompt_marks_captain_planning() {
        let text = format_profiles_for_prompt(&profiles(json!({
            "dynamic": {
                "taskPlanning": "captain",
                "members": [{"name": "solo", "model": "m"}],
                "tasks": [{"id": "work", "subject": "Work", "assignee": "solo"}]
            }
        })))
        .unwrap();
        assert!(text.contains("captain planning"), "{text}");
    }

    // --- capabilities.test.mjs ---------------------------------------------

    #[test]
    fn profile_directory_line_format() {
        let text = format_profiles_for_prompt(&profiles(json!({
            "demo": {
                "taskPlanning": "captain",
                "members": [{"name": "reviewer"}],
                "protocol": "Review prepared work independently."
            }
        })))
        .unwrap();
        assert_eq!(
            text,
            "Configured team profiles (pass profile= to agent_teams_create):\n\
             - demo (1 member, captain planning): Review prepared work independently."
        );
    }

    // --- fallback-tdd.mjs ---------------------------------------------------

    #[test]
    fn fallback_and_prompt_config_are_normalized() {
        let normalized = resolve_json(
            json!({
                "executionPrompt": "PROFILE_PROMPT",
                "fallback": {"provider": "grok", "model": "backup-profile"},
                "members": [{
                    "name": "worker", "model": "primary", "executionPrompt": "MEMBER_PROMPT",
                    "fallback": {"provider": "openai", "model": "backup-member"}
                }],
                "tasks": [{"id": "work", "subject": "Work", "assignee": "worker"}]
            }),
            "demo",
        )
        .unwrap();
        assert_eq!(
            normalized.members[0]
                .fallback
                .as_ref()
                .map(|f| f.model.as_str()),
            Some("backup-member")
        );
        assert_eq!(
            normalized
                .snapshot
                .fallback
                .as_ref()
                .map(|f| f.model.as_str()),
            Some("backup-profile")
        );
        assert_eq!(
            normalized.members[0].execution_prompt.as_deref(),
            Some("MEMBER_PROMPT")
        );
        assert_eq!(
            normalized.snapshot.execution_prompt.as_deref(),
            Some("PROFILE_PROMPT")
        );
    }

    #[test]
    fn fallback_requires_provider_and_model() {
        let error = resolve_json(
            json!({"members": [{"name": "w", "fallback": {"model": "x"}}]}),
            "bad",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.bad.members[0].fallback.provider must be a string"
        );
        let error = resolve_json(
            json!({"members": [{"name": "w", "fallback": {"provider": "x"}}]}),
            "bad",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.bad.members[0].fallback.model must be a string"
        );
        let error = resolve_json(
            json!({"members": [{"name": "w", "fallback": {"provider": " ", "model": "x"}}]}),
            "bad",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.bad.members[0].fallback.provider must not be empty"
        );
    }

    // --- error messages -----------------------------------------------------

    #[test]
    fn unknown_keys_come_with_suggestions() {
        let error = resolve_json(
            json!({"members": [{"name": "w", "reasoningEffort": "high"}]}),
            "p",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.p.members[0].reasoningEffort is unknown; did you mean reasoning_effort?"
        );
        let error = resolve_json(
            json!({"members": [{"name": "w"}], "exectionPrompt": "x"}),
            "p",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.p.exectionPrompt is unknown; did you mean executionPrompt?"
        );
        let error =
            resolve_json(json!({"members": [{"name": "w"}], "Tasks": []}), "p").unwrap_err();
        assert_eq!(error, "profiles.p.Tasks is unknown; did you mean tasks?");
        let error =
            resolve_json(json!({"members": [{"name": "w"}], "zzzzzz": 1}), "p").unwrap_err();
        assert_eq!(error, "profiles.p.zzzzzz is unknown");
        let error = resolve_json(
            json!({"members": [{"name": "w"}], "reviewPolicy": {"codeMaxRound": 2}}),
            "p",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.p.reviewPolicy.codeMaxRound is unknown; did you mean codeMaxRounds?"
        );
    }

    #[test]
    fn unknown_keys_in_typed_profiles_are_kept_and_reported() {
        let parsed: BTreeMap<String, TeamProfileConfig> = profiles(json!({
            "p": {"members": [{"name": "w", "reasoningEffort": "high"}]}
        }));
        assert_eq!(
            parsed["p"].members[0].extra.get("reasoningEffort"),
            Some(&json!("high"))
        );
        let error = normalize_team_profile("p", &parsed["p"]).unwrap_err();
        assert!(error.contains("did you mean reasoning_effort?"), "{error}");
    }

    #[test]
    fn typed_profiles_round_trip_the_settings_shape() {
        let value = json!({
            "description": "d",
            "protocol": "p",
            "executionPrompt": "e",
            "fallback": {"provider": "openai", "model": "m2"},
            "members": [{
                "name": "w", "role": "r", "provider": "openai", "model": "m",
                "reasoning_effort": "high", "executionPrompt": "x",
                "fallback": {"provider": "openai", "model": "m3"}
            }],
            "taskPlanning": "seed",
            "reviewPolicy": {"codeMaxRounds": 2, "requiredReviewers": ["w"]},
            "tasks": [{"id": "a", "subject": "A", "description": "d", "assignee": "w", "dependencies": []}]
        });
        let typed: TeamProfileConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(typed.members[0].reasoning_effort.as_deref(), Some("high"));
        let written = serde_json::to_value(&typed).unwrap();
        let mut expected = value;
        // An empty dependency list is the same as an absent one.
        expected["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("dependencies");
        assert_eq!(written, expected);
        assert!(normalize_team_profile("p", &typed).is_ok());
    }

    #[test]
    fn member_errors_match_the_reference() {
        let cases = [
            (json!({}), "AgentTeams profile \"p\" has no members"),
            (
                json!({"members": []}),
                "AgentTeams profile \"p\" has no members",
            ),
            (
                json!({"members": "x"}),
                "AgentTeams profile \"p\" has no members",
            ),
            (
                json!({"members": [5]}),
                "profiles.p.members[0] must be an object",
            ),
            (
                json!({"members": [{}]}),
                "profiles.p.members[0].name must be a string",
            ),
            (
                json!({"members": [{"name": "  "}]}),
                "profile \"p\" has a member with an empty name",
            ),
            (
                json!({"members": [{"name": " Captain "}]}),
                "member name \"Captain\" is reserved for the captain",
            ),
            (
                json!({"members": [{"name": "w", "provider": "openai"}]}),
                "profile member \"w\" sets provider without model",
            ),
            (
                json!({"members": [{"name": "w", "role": ""}]}),
                "profiles.p.members[0].role must not be empty",
            ),
            (
                json!({"members": [{"name": "w", "model": 3}]}),
                "profiles.p.members[0].model must be a string",
            ),
            (
                json!({"members": [{"name": "Front End"}, {"name": "front-end"}]}),
                "profile members \"Front End\" and \"front-end\" collapse to the same name",
            ),
            (
                json!({"members": [{"name": "w"}], "taskPlanning": "auto"}),
                "profiles.p.taskPlanning must be \"captain\" or \"seed\"",
            ),
            (
                json!({"members": [{"name": "w"}], "description": 1}),
                "profiles.p.description must be a string",
            ),
            (json!("text"), "profiles.p must be an object"),
        ];
        for (value, expected) in cases {
            assert_eq!(resolve_json(value, "p").unwrap_err(), expected);
        }
    }

    #[test]
    fn too_many_members_respects_max_members() {
        let members: Vec<Value> = (0..3).map(|i| json!({"name": format!("m{i}")})).collect();
        let error = normalize_team_profile_value("p", &json!({"members": members}), 2).unwrap_err();
        assert_eq!(error, "profile \"p\" has 3 members but maxMembers is 2");
    }

    #[test]
    fn task_errors_match_the_reference() {
        let member = json!([{"name": "Worker"}]);
        let cases = [
            (json!({"tasks": {}}), "profiles.p.tasks must be an array"),
            (
                json!({"tasks": [{}]}),
                "profiles.p.tasks[0].id must be a string",
            ),
            (
                json!({"tasks": [{"id": " "}]}),
                "profile \"p\" has a task with an empty id",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": ""}]}),
                "profile task \"a\" is missing a subject",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A"}]}),
                "profile task \"a\" is missing an assignee",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": " "}]}),
                "profile task \"a\" is missing an assignee",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": 1}]}),
                "profiles.p.tasks[0].assignee must be a string",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "captain"}]}),
                "profile task \"a\" cannot assign work to the captain",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "nobody"}]}),
                "profile task \"a\" assignee \"nobody\" is not a profile member",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "worker", "dependencies": "b"}]}),
                "profiles.p.tasks[0].dependencies must be an array of task ids",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "worker", "dependencies": [1]}]}),
                "profiles.p.tasks[0].dependencies[0] must be a string",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "worker", "dependencies": [" "]}]}),
                "profiles.p.tasks[0].dependencies[0] must not be empty",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "worker", "dependencies": ["a"]}]}),
                "profile task \"a\" cannot depend on itself",
            ),
            (
                json!({"tasks": [{"id": "a", "subject": "A", "assignee": "worker", "dependencies": ["z"]}]}),
                "profile task \"a\" depends on unknown task \"z\"",
            ),
            (
                json!({"tasks": [
                    {"id": "a", "subject": "A", "assignee": "worker"},
                    {"id": "a", "subject": "B", "assignee": "worker"}
                ]}),
                "profile \"p\" has duplicate task id \"a\"",
            ),
            (
                json!({"tasks": [
                    {"id": "Step One", "subject": "A", "assignee": "worker"},
                    {"id": "step-one", "subject": "B", "assignee": "worker"}
                ]}),
                "profile tasks \"Step One\" and \"step-one\" collapse to the same id",
            ),
        ];
        for (mut value, expected) in cases {
            value["members"] = member.clone();
            assert_eq!(resolve_json(value, "p").unwrap_err(), expected);
        }
    }

    #[test]
    fn too_many_tasks_are_refused() {
        let tasks: Vec<Value> = (0..33)
            .map(|i| json!({"id": format!("t{i}"), "subject": "S", "assignee": "w"}))
            .collect();
        let error =
            resolve_json(json!({"members": [{"name": "w"}], "tasks": tasks}), "p").unwrap_err();
        assert_eq!(error, "profile \"p\" has 33 tasks but the limit is 32");
    }

    #[test]
    fn cycles_are_named() {
        let task = |id: &str, deps: &[&str]| json!({"id": id, "subject": id, "assignee": "w", "dependencies": deps});
        let error = resolve_json(
            json!({"members": [{"name": "w"}], "tasks": [task("a", &["b"]), task("b", &["a"])]}),
            "p",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profile task \"a\" and \"b\" form a dependency cycle"
        );
        let error = resolve_json(
            json!({"members": [{"name": "w"}], "tasks": [
                task("a", &["c"]), task("b", &["a"]), task("c", &["b"])
            ]}),
            "p",
        )
        .unwrap_err();
        assert_eq!(
            error,
            "profile tasks \"a\", \"b\", and \"c\" form a dependency cycle"
        );
        assert_eq!(
            format_cycle_error(&["x"]),
            "profile task \"x\" forms a dependency cycle"
        );
    }

    #[test]
    fn topological_ties_keep_source_order() {
        let task = |id: &str, deps: &[&str]| json!({"id": id, "subject": id, "assignee": "w", "dependencies": deps});
        let normalized = resolve_json(
            json!({"members": [{"name": "w"}], "tasks": [
                task("x", &["z"]), task("y", &[]), task("z", &[]), task("w", &["y", "y"])
            ]}),
            "p",
        )
        .unwrap();
        let order: Vec<&str> = normalized
            .tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect();
        assert_eq!(order, ["y", "z", "x", "w"]);
        assert_eq!(normalized.tasks[3].dependencies, ["y"]);
        assert_eq!(normalized.tasks[2].source_index, 0);
    }

    #[test]
    fn review_policy_is_checked() {
        let base = |policy: Value| json!({"members": [{"name": "w"}], "reviewPolicy": policy});
        let ok = resolve_json(
            base(json!({"codeMaxRounds": 2, "requiredReviewers": [" w "]})),
            "p",
        )
        .unwrap();
        let policy = ok.snapshot.review_policy.unwrap();
        assert_eq!(policy.code_max_rounds, Some(2));
        assert_eq!(policy.required_reviewers, Some(vec!["w".to_owned()]));
        let cases = [
            (
                json!({"codeMaxRounds": 0}),
                "profiles.p.reviewPolicy.codeMaxRounds must be a positive integer",
            ),
            (
                json!({"codeMaxRounds": 1.5}),
                "profiles.p.reviewPolicy.codeMaxRounds must be a positive integer",
            ),
            (
                json!({"maxRepairAttempts": "2"}),
                "profiles.p.reviewPolicy.maxRepairAttempts must be a positive integer",
            ),
            (
                json!({"requirementsMinRounds": 3, "requirementsMaxRounds": 2}),
                "profiles.p.reviewPolicy.requirementsMinRounds must be <= requirementsMaxRounds",
            ),
            (
                json!({"requiredReviewers": "w"}),
                "profiles.p.reviewPolicy.requiredReviewers must be an array of strings",
            ),
            (
                json!({"requiredReviewers": ["w", ""]}),
                "profiles.p.reviewPolicy.requiredReviewers[1] must be a non-empty string",
            ),
            (json!([]), "profiles.p.reviewPolicy must be an object"),
        ];
        for (policy, expected) in cases {
            assert_eq!(resolve_json(base(policy), "p").unwrap_err(), expected);
        }
        // An integral float is a safe integer in the reference.
        assert!(resolve_json(base(json!({"codeMaxRounds": 2.0})), "p").is_ok());
        // One bound alone may not invert the effective range either.
        let error = resolve_json(base(json!({"requirementsMinRounds": 99})), "p").unwrap_err();
        assert!(error.starts_with("profiles.p.reviewPolicy.requirementsMinRounds must be <="));
    }

    #[test]
    fn listing_errors_match_the_reference() {
        let one = TeamProfileConfig {
            members: vec![builtin_member("w", "r")],
            ..TeamProfileConfig::default()
        };
        let mut map = BTreeMap::new();
        map.insert(" ".to_owned(), one.clone());
        assert_eq!(
            list_configured_profiles(&map).unwrap_err(),
            "configured AgentTeams profiles include an empty key"
        );
        let mut map = BTreeMap::new();
        map.insert("a".to_owned(), one.clone());
        map.insert(" a ".to_owned(), one.clone());
        assert_eq!(
            list_configured_profiles(&map).unwrap_err(),
            "configured AgentTeams profiles have duplicate key \"a\""
        );
        let map: BTreeMap<String, TeamProfileConfig> =
            (0..17).map(|i| (format!("p{i}"), one.clone())).collect();
        assert_eq!(
            list_configured_profiles(&map).unwrap_err(),
            "too many AgentTeams profiles (17); the limit is 16"
        );
    }

    #[test]
    fn resolve_reports_unknown_and_blank_names() {
        let error = resolve_team_profile(&demo_profiles(), "missing", 8).unwrap_err();
        assert_eq!(
            error,
            "unknown AgentTeams profile \"missing\" — configured profiles: demo"
        );
        let error = resolve_team_profile(&BTreeMap::new(), "x", 8).unwrap_err();
        assert_eq!(
            error,
            "unknown AgentTeams profile \"x\" — configured profiles: (none)"
        );
        let error = resolve_team_profile(&demo_profiles(), "  ", 8).unwrap_err();
        assert_eq!(error, "AgentTeams profile name must be a non-empty string");
    }

    // --- invocation parsing --------------------------------------------------

    #[test]
    fn profile_invocation_flags() {
        let parsed = parse_profile_invocation("--profile \"demo\" ship it").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo"));
        assert_eq!(parsed.goal, "ship it");
        let parsed = parse_profile_invocation("profile='x'   do   the thing ").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("x"));
        assert_eq!(parsed.goal, "do the thing");
        let parsed = parse_profile_invocation("--profile demo").unwrap();
        assert_eq!(parsed.goal, "");
        let parsed = parse_profile_invocation("   ").unwrap();
        assert_eq!(parsed, AgentTeamsInvocation::default());
        let parsed = parse_profile_invocation("--profile \"demo ship").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("\"demo"));
        let parsed = parse_profile_invocation("research this bug").unwrap();
        assert_eq!(parsed.profile, None);
        assert_eq!(parsed.goal, "research this bug");
    }

    #[test]
    fn profile_invocation_errors() {
        assert_eq!(
            parse_profile_invocation("--profile").unwrap_err(),
            "--profile flag is missing a profile name"
        );
        assert_eq!(
            parse_profile_invocation("--profile= goal").unwrap_err(),
            "--profile flag is missing a profile name"
        );
        assert_eq!(
            parse_profile_invocation("--profile \"\" goal").unwrap_err(),
            "--profile flag is missing a profile name"
        );
        assert_eq!(
            parse_profile_invocation("--profile a profile=b goal").unwrap_err(),
            "duplicate AgentTeams profile flag"
        );
    }

    // --- parse_profiles -------------------------------------------------------

    #[test]
    fn parse_profiles_checks_then_types() {
        assert!(parse_profiles(&Value::Null).unwrap().is_empty());
        assert_eq!(
            parse_profiles(&json!([])).unwrap_err(),
            "AgentTeams profiles must be an object map of named templates"
        );
        let parsed = parse_profiles(&json!({
            "solo": {"members": [{"name": "w", "reasoning_effort": "low"}]}
        }))
        .unwrap();
        assert_eq!(
            parsed["solo"].members[0].reasoning_effort.as_deref(),
            Some("low")
        );
        let error = parse_profiles(&json!({
            "solo": {"members": [{"name": "w"}], "taskPlaning": "seed"}
        }))
        .unwrap_err();
        assert_eq!(
            error,
            "profiles.solo.taskPlaning is unknown; did you mean taskPlanning?"
        );
    }

    // --- built-ins ------------------------------------------------------------

    #[test]
    fn builtin_profiles_all_normalize() {
        let builtins = builtin_profiles();
        assert_eq!(builtins.len(), 3);
        for (name, profile) in &builtins {
            let normalized = normalize_team_profile(name, profile)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                normalized
                    .members
                    .iter()
                    .all(|member| member.provider.is_none()
                        && member.model.is_none()
                        && member.role.is_some())
            );
            assert!(normalized.snapshot.description.is_some());
            assert!(normalized.snapshot.protocol.is_some());
        }
        resolve_team_profile(&builtins, "dual-review", 8).unwrap();
        let text = format_profiles_for_prompt(&builtins).unwrap();
        assert!(text.contains("- implement-test-fix (3 members, captain planning): "));
        assert!(text.contains("- dual-review (4 members, 4 tasks): "));
        assert!(text.contains("- prd-implement-review (3 members, 3 tasks): "));
    }

    #[test]
    fn builtin_graphs_have_the_template_shape() {
        let builtins = builtin_profiles();
        let prd = normalize_team_profile("prd-implement-review", &builtins["prd-implement-review"])
            .unwrap();
        let order: Vec<(&str, Option<&str>)> = prd
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task.assignee.as_deref()))
            .collect();
        assert_eq!(
            order,
            [
                ("prd", Some("planner")),
                ("implement", Some("engineer")),
                ("review", Some("reviewer"))
            ]
        );
        assert_eq!(prd.tasks[2].dependencies, ["implement"]);

        let fix =
            normalize_team_profile("implement-test-fix", &builtins["implement-test-fix"]).unwrap();
        assert_eq!(fix.task_planning(), TaskPlanning::Captain);
        assert!(fix.tasks.is_empty());
        let names: Vec<&str> = fix.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["engineer", "tester", "reviewer"]);

        let dual = normalize_team_profile("dual-review", &builtins["dual-review"]).unwrap();
        let order: Vec<&str> = dual.tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(order, ["implement", "review-a", "review-b", "summary"]);
        assert_eq!(dual.tasks[1].dependencies, ["implement"]);
        assert_eq!(dual.tasks[2].dependencies, ["implement"]);
        assert_eq!(dual.tasks[3].dependencies, ["review-a", "review-b"]);
    }

    #[test]
    fn suggestion_distance_is_bounded() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(suggest_field("MODEL", &MEMBER_KEYS), Some("model"));
        assert_eq!(suggest_field("rol", &MEMBER_KEYS), Some("role"));
        assert_eq!(suggest_field("xyzxyz", &MEMBER_KEYS), None);
    }
}
