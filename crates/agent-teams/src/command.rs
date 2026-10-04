// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! The `/agent-teams` command, its per-profile aliases, the activation
//! directive the captain receives, and the control sentinel the Team panel
//! sends through the chat (the sentinel is original to this port).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::profiles::{
    AgentTeamsInvocation, TeamProfileConfig, parse_profile_invocation,
    resolve_profile_task_planning,
};
use crate::types::TaskPlanning;

/// The command name, without its slash.
pub const AGENT_TEAMS_COMMAND: &str = "agent-teams";
/// Prefix of the per-profile alias commands (`agent-teams-<profile>`).
pub const PROFILE_COMMAND_PREFIX: &str = "agent-teams-";
/// Menu description of `/agent-teams`.
pub const COMMAND_DESCRIPTION: &str = "run a goal with a multi-agent team (you become the captain)";
/// Input hint of `/agent-teams`.
pub const COMMAND_HINT: &str = "[--profile <name>] <goal>";
/// Input hint of a profile alias.
pub const PROFILE_COMMAND_HINT: &str = "<goal>";

/// A parsed command line: the goal and the optional profile.
pub type CommandInvocation = AgentTeamsInvocation;

/// Characters that end the command token (the reference's `[\t\n\r ]`).
const TOKEN_END: [char; 4] = [' ', '\t', '\n', '\r'];

/// The alias command for a profile key, `agent-teams-<key>`. Only keys made
/// of lowercase ASCII letters, digits and single inner dashes (after trim and
/// lowercasing) get one, so `foo bar`, `foo_bar` or non-ASCII keys never
/// produce an ambiguous alias.
pub fn profile_command_name(profile_name: &str) -> Option<String> {
    let normalized = profile_name.trim().to_lowercase();
    let valid = normalized.split('-').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
    });
    valid.then(|| format!("{PROFILE_COMMAND_PREFIX}{normalized}"))
}

/// Menu description of a profile alias.
pub fn profile_command_description(profile_name: &str) -> String {
    format!("run a goal with the AgentTeams {profile_name} profile")
}

/// The profile an alias command stands for, only when exactly one
/// configured key maps to it. Returned trimmed, as profile lookup expects.
pub fn profile_for_command<I, S>(command_name: &str, profile_names: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut matches = profile_names
        .into_iter()
        .filter(|name| profile_command_name(name.as_ref()).as_deref() == Some(command_name));
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.as_ref().trim().to_owned())
}

/// Recognize `/agent-teams [--profile <name>] <goal>` or a profile alias
/// `/agent-teams-<profile> <goal>` at the start of a user message.
///
/// `Ok(None)`: not a command (ordinary prose, a mid-sentence mention, an
/// alias of no configured profile). `Err`: a malformed `--profile` flag.
pub fn parse_command_text<I, S>(
    text: &str,
    profile_names: I,
) -> Result<Option<CommandInvocation>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix('/')
        .and_then(|rest| rest.strip_prefix(AGENT_TEAMS_COMMAND))
        && (rest.is_empty() || rest.starts_with(TOKEN_END))
    {
        return parse_profile_invocation(rest.trim()).map(Some);
    }
    let Some(body) = trimmed.strip_prefix('/') else {
        return Ok(None);
    };
    if !body.starts_with(PROFILE_COMMAND_PREFIX) {
        return Ok(None);
    }
    let token_end = body.find(TOKEN_END).unwrap_or(body.len());
    let Some(profile) = profile_for_command(&body[..token_end], profile_names) else {
        return Ok(None);
    };
    Ok(Some(CommandInvocation {
        goal: body[token_end..].trim().to_owned(),
        profile: Some(profile),
    }))
}

/// What the `/agent-teams` command handler checks before activating:
/// a parsable flag, a configured profile, and a goal unless a profile is
/// named. The error is the text to show the user.
pub fn check_command_input<I, S>(
    raw_input: &str,
    profile_names: I,
) -> Result<CommandInvocation, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parsed = parse_profile_invocation(raw_input.trim())?;
    if let Some(profile) = &parsed.profile
        && !profile_names
            .into_iter()
            .any(|name| name.as_ref().trim() == profile)
    {
        return Err(format!("unknown AgentTeams profile \"{profile}\""));
    }
    if parsed.profile.is_none() && parsed.goal.is_empty() {
        return Err(format!("Usage: /{AGENT_TEAMS_COMMAND} {COMMAND_HINT}"));
    }
    Ok(parsed)
}

/// The confirmation a command handler shows after activating.
pub fn activation_notice(profile: Option<&str>) -> String {
    match profile {
        Some(profile) => format!(
            "AgentTeams activated with profile {profile} — the captain will assemble the team."
        ),
        None => "AgentTeams activated — the captain will assemble the team.".to_owned(),
    }
}

/// The note the captain gets when a command line could not be parsed.
pub fn parse_failure_directive(error: &str) -> String {
    format!("AgentTeams profile parsing failed: {error}")
}

/// The directive for a recognized command line: the activation directive, or
/// a refusal when the named profile is not configured.
pub fn directive_for_invocation(
    invocation: &CommandInvocation,
    profiles: &BTreeMap<String, TeamProfileConfig>,
) -> String {
    let Some(profile) = invocation.profile.as_deref() else {
        return build_activation_directive(&invocation.goal, None, TaskPlanning::Seed);
    };
    match profiles.iter().find(|(key, _)| key.trim() == profile) {
        Some((_, config)) => build_activation_directive(
            &invocation.goal,
            Some(profile),
            resolve_profile_task_planning(Some(config)),
        ),
        None => {
            let available: Vec<&str> = profiles.keys().map(String::as_str).collect();
            let shown = if available.is_empty() {
                "(none)".to_owned()
            } else {
                available.join(", ")
            };
            format!(
                "AgentTeams profile \"{profile}\" does not exist. Available profiles: {shown}. Do not create a team."
            )
        }
    }
}

/// The directive injected after the user's command line, steering the
/// captain into the staged-plan protocol.
pub fn build_activation_directive(
    goal: &str,
    profile: Option<&str>,
    task_planning: TaskPlanning,
) -> String {
    let mut lines = vec![
        "The user invoked an AgentTeams slash command. Follow the AgentTeams protocol already in your system instructions. Inspect existing team state with agent_teams_status when needed.".to_owned(),
        "Respect the current team state. Continue an existing plan or team without recreating it. Only when no current team exists, call agent_teams_create with approval=\"required\". Build the complete staged roster and DAG, then stop and ask the user to review the plan in the Team panel. Do not approve or start it in this same turn.".to_owned(),
    ];
    if let Some(profile) = profile {
        lines.push(format!(
            "Use profile=\"{profile}\" when creating a new team."
        ));
        match task_planning {
            TaskPlanning::Captain => lines.extend([
                "This profile supplies the roster and guardrails. After create, do not recreate members.".to_owned(),
                "Derive the smallest useful task graph from the goal while the team is staged; do not ask the user whether to split, merge, serialize, or parallelize.".to_owned(),
                "Independent supplemental work must become separate ready tasks so idle members can run in parallel. Add dependencies only for genuine prerequisites and later synthesis.".to_owned(),
            ]),
            TaskPlanning::Seed => {
                lines.push("Do not recreate the same members or seed tasks manually.".to_owned());
            }
        }
    }
    lines.push(if goal.is_empty() {
        "The goal was not given — ask the user what the team should accomplish.".to_owned()
    } else {
        format!("Goal: {goal}")
    });
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Control sentinel (original to this port)
// ---------------------------------------------------------------------------

/// Prefix of a control message the app sends through the chat and the
/// bridge intercepts before it reaches the model.
pub const CONTROL_PREFIX: &str = "<waku:agent-teams>";

/// What the user asked the Team panel to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlAction {
    /// Approve the staged plan and start it.
    Approve,
    /// Return the staged plan to the captain for changes.
    Revise,
    /// Discard the staged plan.
    Discard,
    /// Halt a running team.
    Stop,
    /// Nudge the captain / scheduler to continue.
    Kick,
}

/// One control message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub action: ControlAction,
    #[serde(rename = "teamId")]
    pub team_id: String,
}

/// `CONTROL_PREFIX` followed by the control as compact JSON.
pub fn encode_control(control: &Control) -> String {
    let body = serde_json::to_string(control).unwrap_or_default();
    format!("{CONTROL_PREFIX}{body}")
}

/// The control in `text`, when the (trimmed) text is exactly the prefix
/// followed by a control's JSON. Anything else is ordinary text.
pub fn decode_control(text: &str) -> Option<Control> {
    let body = text.trim().strip_prefix(CONTROL_PREFIX)?;
    let control: Control = serde_json::from_str(body).ok()?;
    (!control.team_id.trim().is_empty()).then_some(control)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: [&str; 1] = ["demo-delivery"];

    fn parse(text: &str) -> Option<CommandInvocation> {
        parse_command_text(text, LIVE).unwrap()
    }

    // --- lifecycle-verify.mjs ---------------------------------------------

    #[test]
    fn profile_command_names_are_closed() {
        assert_eq!(
            profile_command_name("demo-delivery").as_deref(),
            Some("agent-teams-demo-delivery")
        );
        assert_eq!(profile_command_name("delivery team"), None);
        assert_eq!(profile_command_name("delivery_team"), None);
        assert_eq!(
            profile_command_name(" Demo2 ").as_deref(),
            Some("agent-teams-demo2")
        );
        assert_eq!(profile_command_name("前端"), None);
        assert_eq!(profile_command_name("-x"), None);
        assert_eq!(profile_command_name("a--b"), None);
        assert_eq!(profile_command_name(""), None);
        assert!(profile_command_description("demo-delivery").contains("demo-delivery"));
    }

    #[test]
    fn the_gesture_needs_a_leading_command_token() {
        let parsed = parse("/agent-teams ship a CLI").unwrap();
        assert_eq!(parsed.goal, "ship a CLI");
        assert_eq!(parsed.profile, None);
        assert_eq!(parse("  /agent-teams").unwrap().goal, "");
        assert_eq!(parse("/agent-teams\n  goal  ").unwrap().goal, "goal");
        assert_eq!(parse("how do I use /agent-teams here?"), None);
        assert_eq!(parse("/agent-teamsx do"), None);
        assert_eq!(parse("agent-teams do"), None);
    }

    #[test]
    fn a_leading_ordinary_token_is_never_a_profile() {
        let parsed = parse("/agent-teams research this bug").unwrap();
        assert_eq!(parsed.goal, "research this bug");
        assert_eq!(parsed.profile, None);
        let parsed = parse("/agent-teams --profile demo-delivery ship").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo-delivery"));
        assert_eq!(parsed.goal, "ship");
        assert_eq!(
            parse_command_text("/agent-teams --profile", LIVE).unwrap_err(),
            "--profile flag is missing a profile name"
        );
    }

    #[test]
    fn profile_aliases_select_their_profile() {
        let parsed = parse("/agent-teams-demo-delivery ship a CLI").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo-delivery"));
        assert_eq!(parsed.goal, "ship a CLI");
        let parsed = parse("/agent-teams-demo-delivery").unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo-delivery"));
        assert_eq!(parsed.goal, "");
        assert_eq!(parse("/agent-teams-missing ship a CLI"), None);
        // Two keys that fold to one alias make it ambiguous.
        assert_eq!(
            parse_command_text("/agent-teams-demo go", ["demo", "Demo"]).unwrap(),
            None
        );
        let parsed = parse_command_text("/agent-teams-demo go", [" Demo "])
            .unwrap()
            .unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("Demo"));
    }

    #[test]
    fn activation_directive_text() {
        let goal = "ship a tiny CLI";
        assert!(
            build_activation_directive(goal, None, TaskPlanning::Seed)
                .contains("AgentTeams protocol")
        );
        let text = build_activation_directive("", Some("demo-delivery"), TaskPlanning::Seed);
        assert!(text.contains("The goal was not given"));
        assert!(text.contains("Use profile=\"demo-delivery\" when creating a new team"));
        assert!(text.contains("Inspect existing team state with agent_teams_status"));
        assert!(text.contains("seed tasks"));
        let text =
            build_activation_directive("ship it", Some("dynamic-delivery"), TaskPlanning::Captain);
        assert!(text.contains("approval=\"required\""));
        assert!(text.contains("review the plan in the Team panel"));
        assert!(text.contains("run in parallel"));
        assert!(!text.contains("seed tasks"));
        assert!(text.ends_with("\nGoal: ship it"));
        assert_eq!(
            build_activation_directive("g", None, TaskPlanning::Seed)
                .lines()
                .count(),
            3
        );
    }

    #[test]
    fn command_input_checks() {
        assert_eq!(
            check_command_input("   ", LIVE).unwrap_err(),
            "Usage: /agent-teams [--profile <name>] <goal>"
        );
        assert_eq!(
            check_command_input("--profile missing 做X", LIVE).unwrap_err(),
            "unknown AgentTeams profile \"missing\""
        );
        let parsed = check_command_input("--profile demo-delivery", LIVE).unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("demo-delivery"));
        assert_eq!(parsed.goal, "");
        assert_eq!(
            check_command_input("  do it  ", LIVE).unwrap().goal,
            "do it"
        );
        assert_eq!(
            activation_notice(Some("demo-delivery")),
            "AgentTeams activated with profile demo-delivery — the captain will assemble the team."
        );
        assert_eq!(
            activation_notice(None),
            "AgentTeams activated — the captain will assemble the team."
        );
    }

    #[test]
    fn directives_for_known_and_unknown_profiles() {
        let profiles = crate::profiles::builtin_profiles();
        let invocation = CommandInvocation {
            goal: "ship".to_owned(),
            profile: Some("implement-test-fix".to_owned()),
        };
        let text = directive_for_invocation(&invocation, &profiles);
        assert!(text.contains("run in parallel"), "{text}");
        let invocation = CommandInvocation {
            goal: "ship".to_owned(),
            profile: Some("nope".to_owned()),
        };
        assert_eq!(
            directive_for_invocation(&invocation, &profiles),
            "AgentTeams profile \"nope\" does not exist. Available profiles: dual-review, implement-test-fix, prd-implement-review. Do not create a team."
        );
        assert!(
            directive_for_invocation(&invocation, &BTreeMap::new())
                .contains("Available profiles: (none).")
        );
        let invocation = CommandInvocation {
            goal: "ship".to_owned(),
            profile: None,
        };
        assert_eq!(
            directive_for_invocation(&invocation, &profiles),
            build_activation_directive("ship", None, TaskPlanning::Seed)
        );
    }

    // --- control sentinel ----------------------------------------------------

    #[test]
    fn controls_round_trip() {
        for action in [
            ControlAction::Approve,
            ControlAction::Revise,
            ControlAction::Discard,
            ControlAction::Stop,
            ControlAction::Kick,
        ] {
            let control = Control {
                action,
                team_id: "alpha".to_owned(),
            };
            let text = encode_control(&control);
            assert!(text.starts_with(CONTROL_PREFIX));
            assert!(!text.contains('\n'));
            assert_eq!(decode_control(&text), Some(control.clone()));
            assert_eq!(decode_control(&format!("  {text}\n")), Some(control));
        }
        assert_eq!(
            encode_control(&Control {
                action: ControlAction::Approve,
                team_id: "alpha".to_owned()
            }),
            r#"<waku:agent-teams>{"action":"approve","teamId":"alpha"}"#
        );
    }

    #[test]
    fn ordinary_text_is_not_a_control() {
        assert_eq!(decode_control("approve the plan"), None);
        assert_eq!(decode_control("/agent-teams goal"), None);
        assert_eq!(decode_control("<waku:agent-teams>"), None);
        assert_eq!(
            decode_control("<waku:agent-teams>{\"action\":\"launch\",\"teamId\":\"a\"}"),
            None
        );
        assert_eq!(
            decode_control("<waku:agent-teams>{\"action\":\"stop\",\"teamId\":\" \"}"),
            None
        );
        assert_eq!(
            decode_control("please <waku:agent-teams>{\"action\":\"stop\",\"teamId\":\"a\"}"),
            None
        );
        assert_eq!(
            decode_control("<waku:agent-teams>{\"action\":\"stop\",\"teamId\":\"a\"} trailing"),
            None
        );
    }
}
