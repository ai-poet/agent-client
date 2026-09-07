//! The workflow planner: one model call through the gateway that says what
//! a run should do next.
//!
//! The planner is not an agent CLI. The desktop describes the run — goal,
//! the stages so far and what each reported, which agents are installed,
//! what budget is left — and asks a cheap model for a small JSON decision:
//! add stages, declare the goal met, ask the user, or give up. The desktop
//! validates the decision through [`crate::model::WorkflowRun::apply_patch`]
//! and the user approves it before anything runs, so a bad answer costs a
//! round trip, never a session.
//!
//! Everything here is pure except [`plan`], which sends the request; the
//! prompt, the request shape and the response parsing are testable offline.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Edge, EdgeKind, Node, NodeStatus, Patch, WorkflowRun};
use sub2api::gateway::{GatewayConfig, anthropic_base_url};
use sub2api::http;

/// A small, fast model is enough for a structured decision.
pub const DEFAULT_PLANNER_MODEL: &str = "claude-haiku-4-5";
/// `--max-time` is a hard kill, so leave the model room to answer.
pub const PLANNER_TIMEOUT_SECS: u32 = 120;
pub const PLANNER_MAX_TOKENS: u32 = 2000;
/// Stage reports are cut here before they reach the planner.
pub const SUMMARY_BRIEF_CAP: usize = 500;

// --- input ---------------------------------------------------------------

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AgentBrief {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NodeBrief {
    pub id: String,
    pub role: String,
    pub agent: String,
    pub status: String,
    pub writes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EdgeBrief {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BudgetBrief {
    pub stages_left: usize,
    pub planner_calls_left: u32,
}

/// What the planner is told. Deliberately small: no transcripts, no
/// patches — just the shape of the run and each stage's own report.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PlannerInput {
    pub goal: String,
    pub available_agents: Vec<AgentBrief>,
    /// The agents the user assigned to roles may not be swapped out.
    pub agents_fixed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub stages: Vec<NodeBrief>,
    pub edges: Vec<EdgeBrief>,
    pub budget: BudgetBrief,
}

impl PlannerInput {
    pub fn from_run(run: &WorkflowRun, available: &[AgentBrief], project: Option<String>) -> Self {
        Self {
            goal: run.goal.clone(),
            available_agents: available.to_vec(),
            agents_fixed: !run.planner_chooses_agents,
            project,
            stages: run
                .nodes
                .iter()
                .map(|node| NodeBrief {
                    id: node.id.clone(),
                    role: node.role.clone(),
                    agent: node.provider_id.clone(),
                    status: status_word(node.status).to_owned(),
                    writes: node.writes,
                    summary: node
                        .summary
                        .as_deref()
                        .map(|text| truncate(text, SUMMARY_BRIEF_CAP)),
                    changed_files: node.numstat.clone(),
                })
                .collect(),
            edges: run
                .edges
                .iter()
                .map(|edge| EdgeBrief {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    kind: match edge.kind {
                        EdgeKind::DependsOn => "depends_on".to_owned(),
                        EdgeKind::OnFailure => "on_failure".to_owned(),
                    },
                })
                .collect(),
            budget: BudgetBrief {
                stages_left: run.budget.max_nodes.saturating_sub(run.nodes.len()),
                planner_calls_left: run
                    .budget
                    .max_planner_calls
                    .saturating_sub(run.spent.planner_calls),
            },
        }
    }
}

fn status_word(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Proposed => "proposed",
        NodeStatus::Pending => "pending",
        NodeStatus::Ready => "ready",
        NodeStatus::Running => "running",
        NodeStatus::AwaitingInput => "awaiting_input",
        NodeStatus::Done => "done",
        NodeStatus::Failed => "failed",
        NodeStatus::Skipped => "skipped",
        NodeStatus::Canceled => "canceled",
    }
}

fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(cap.saturating_sub(1)).collect();
    out.push('…');
    out
}

// --- decision ------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Add the listed stages (and nothing else).
    AddNodes,
    /// The goal is met once the current stages settle.
    Done,
    /// Something only the user can decide; `message` says what.
    AskUser,
    /// The goal cannot be met; `message` says why.
    Abort,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProposedNode {
    pub id: String,
    pub role: String,
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub on_failure_of: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Decision {
    #[serde(default)]
    pub reasoning: String,
    pub action: Action,
    #[serde(default)]
    pub nodes: Vec<ProposedNode>,
    #[serde(default)]
    pub message: Option<String>,
}

impl Decision {
    /// The nodes and edges to validate against the run. Edges come from
    /// each node's own dependency lists.
    pub fn into_patch(self) -> Patch {
        let mut patch = Patch::default();
        for proposed in self.nodes {
            let mut node = Node::new(
                &proposed.id,
                &proposed.role,
                &proposed.agent,
                &proposed.prompt,
            )
            .writes(proposed.writes)
            .plan_mode(proposed.plan_mode);
            node.model = proposed.model;
            for from in &proposed.depends_on {
                patch.edges.push(Edge::depends(from, &proposed.id));
            }
            for from in &proposed.on_failure_of {
                patch.edges.push(Edge::on_failure(from, &proposed.id));
            }
            patch.nodes.push(node);
        }
        patch
    }
}

// --- prompt and request --------------------------------------------------

pub fn system_prompt() -> &'static str {
    "You plan a multi-stage software workflow. Each stage is one coding-agent \
session working in a shared git worktree; stages run in dependency order and \
only one stage that writes files runs at a time. You will be given the goal, \
the stages so far with what each reported, the installed agents, and the \
remaining budget.\n\
\n\
Reply with ONE JSON object and nothing else, of this shape:\n\
{\"reasoning\": \"one or two sentences\",\n\
 \"action\": \"add_nodes\" | \"done\" | \"ask_user\" | \"abort\",\n\
 \"nodes\": [{\"id\": \"n7\", \"role\": \"Review\", \"agent\": \"claude\", \
\"model\": null, \"prompt\": \"...\", \"writes\": false, \"plan_mode\": true, \
\"depends_on\": [\"n6\"], \"on_failure_of\": []}],\n\
 \"message\": \"only for ask_user / abort\"}\n\
\n\
Rules:\n\
- Use \"done\" when the goal is met or nothing useful remains; do not add \
stages just to be thorough.\n\
- New stage ids must be unused; use the next free n<k>. Dependencies may only \
name existing or newly proposed ids, and must not form a cycle.\n\
- Pick \"agent\" only from available_agents. When agents_fixed is true, reuse \
the agent of the stage with the same role, or of the closest existing stage.\n\
- A stage that edits files sets \"writes\": true. Reviews and analyses set \
\"writes\": false and \"plan_mode\": true. Do not propose two writing stages \
that could run at once.\n\
- A stage meant to fix what another stage found failing lists that stage in \
\"on_failure_of\", not in \"depends_on\".\n\
- Prompts are instructions to the agent: concrete, self-contained, and \
mentioning files by path when a previous stage named them.\n\
- Stay within the budget. If you cannot decide safely, use \"ask_user\"."
}

pub fn user_message(input: &PlannerInput) -> String {
    let state = serde_json::to_string_pretty(input).unwrap_or_default();
    format!(
        "Current run:\n```json\n{state}\n```\n\nDecide the next step. Reply with the JSON object only."
    )
}

/// The Messages API call, ready to send: `(url, request)`.
pub fn build_request(
    endpoint: &str,
    api_key: &str,
    model: &str,
    input: &PlannerInput,
) -> (String, http::Request) {
    let url = format!("{}/v1/messages", anthropic_base_url(endpoint));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": PLANNER_MAX_TOKENS,
        "system": system_prompt(),
        "messages": [{ "role": "user", "content": user_message(input) }],
    });
    let request = http::Request::new()
        .timeout_seconds(PLANNER_TIMEOUT_SECS)
        .header("anthropic-version", "2023-06-01")
        .header("x-api-key", api_key)
        .json_body(body.to_string());
    (url, request)
}

/// The text of a Messages API response: its text blocks joined.
pub fn response_text(body: &str) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(body).context("the planner's reply was not JSON")?;
    let blocks = value
        .get("content")
        .and_then(|content| content.as_array())
        .ok_or_else(|| anyhow!("the planner's reply carried no content"))?;
    let text = blocks
        .iter()
        .filter(|block| block.get("type").and_then(|kind| kind.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|text| text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        bail!("the planner's reply had no text");
    }
    Ok(text)
}

/// The first balanced JSON object in a reply, with any ```json fence or
/// prose around it dropped.
pub fn extract_json(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in raw[start..].char_indices() {
        if in_string {
            match ch {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..start + offset + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse_decision(raw: &str) -> Result<Decision> {
    let json =
        extract_json(raw).ok_or_else(|| anyhow!("the planner's reply held no JSON object"))?;
    serde_json::from_str(json).context("the planner's decision did not match the expected shape")
}

/// Ask the planner. Blocks for up to [`PLANNER_TIMEOUT_SECS`]; callers run
/// it off the UI thread.
pub fn plan(config: &GatewayConfig, model: &str, input: &PlannerInput) -> Result<Decision> {
    if !config.is_usable() {
        bail!("the gateway is not available for planning");
    }
    let key = config
        .key_for("claude")
        .ok_or_else(|| anyhow!("no gateway key for the planner"))?;
    let (url, request) = build_request(&config.endpoint, key, model, input);
    let response = request.send(&url)?;
    if !response.is_success() {
        let detail: String = response.body.chars().take(300).collect();
        bail!(
            "the planner answered HTTP {}: {}",
            response.status,
            detail.trim()
        );
    }
    let text = response_text(&response.body)?;
    parse_decision(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Edge, Node, Patch};

    fn sample_run() -> WorkflowRun {
        let mut run = WorkflowRun::new("add search", "p");
        run.apply_patch(
            Patch {
                nodes: vec![
                    Node::new("n1", "Implement", "codex", "do").writes(true),
                    Node::new("n2", "Review", "claude", "check"),
                ],
                edges: vec![Edge::depends("n1", "n2")],
            },
            &["codex", "claude"],
        )
        .unwrap();
        run.approve_proposed();
        run
    }

    fn agents() -> Vec<AgentBrief> {
        vec![
            AgentBrief {
                id: "claude".into(),
                name: "Claude Code".into(),
            },
            AgentBrief {
                id: "codex".into(),
                name: "Codex CLI".into(),
            },
        ]
    }

    #[test]
    fn build_request_targets_messages_endpoint() {
        let input = PlannerInput::from_run(&sample_run(), &agents(), None);
        let (url, _) = build_request("https://gw.example.org/", "k", "m", &input);
        assert_eq!(url, "https://gw.example.org/v1/messages");
    }

    #[test]
    fn build_request_sets_auth_and_version_headers() {
        let input = PlannerInput::from_run(&sample_run(), &agents(), None);
        let (_, request) = build_request("https://gw.example.org", "sk-test", "m", &input);
        let headers = request.header_lines();
        assert!(
            headers
                .iter()
                .any(|line| line == "anthropic-version: 2023-06-01")
        );
        assert!(headers.iter().any(|line| line == "x-api-key: sk-test"));
        assert!(
            headers
                .iter()
                .any(|line| line == "Content-Type: application/json")
        );
        assert_eq!(request.timeout(), Some(PLANNER_TIMEOUT_SECS));
    }

    #[test]
    fn planner_input_truncates_summaries_and_counts_budget() {
        let mut run = sample_run();
        run.node_mut("n1").unwrap().summary = Some("x".repeat(SUMMARY_BRIEF_CAP + 20));
        run.spent.planner_calls = 3;
        let input = PlannerInput::from_run(&run, &agents(), Some("proj".into()));
        assert_eq!(
            input.stages[0].summary.as_ref().unwrap().chars().count(),
            SUMMARY_BRIEF_CAP
        );
        assert_eq!(input.budget.stages_left, run.budget.max_nodes - 2);
        assert_eq!(
            input.budget.planner_calls_left,
            run.budget.max_planner_calls - 3
        );
        assert!(input.agents_fixed);
        assert_eq!(input.edges[0].kind, "depends_on");
        assert!(user_message(&input).contains("\"goal\": \"add search\""));
    }

    #[test]
    fn extract_json_strips_fence() {
        let raw = "```json\n{\"action\": \"done\", \"reasoning\": \"ok\"}\n```";
        assert_eq!(
            extract_json(raw),
            Some("{\"action\": \"done\", \"reasoning\": \"ok\"}")
        );
    }

    #[test]
    fn extract_json_handles_prose_and_nested_braces() {
        let raw = "Sure! Here you go: {\"a\": {\"b\": \"}\"}, \"c\": [1, 2]} trailing";
        assert_eq!(
            extract_json(raw),
            Some("{\"a\": {\"b\": \"}\"}, \"c\": [1, 2]}")
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json("{unbalanced"), None);
    }

    #[test]
    fn decision_parses_add_nodes_and_builds_patch() {
        let raw = r#"{"reasoning":"needs tests","action":"add_nodes","nodes":[
            {"id":"n3","role":"Test","agent":"codex","prompt":"run tests","writes":true,"depends_on":["n2"]},
            {"id":"n4","role":"Fix","agent":"codex","prompt":"fix","writes":true,"on_failure_of":["n3"]}
        ]}"#;
        let decision = parse_decision(raw).unwrap();
        assert_eq!(decision.action, Action::AddNodes);
        let patch = decision.into_patch();
        assert_eq!(patch.nodes.len(), 2);
        assert_eq!(patch.edges.len(), 2);
        assert_eq!(patch.edges[0], Edge::depends("n2", "n3"));
        assert_eq!(patch.edges[1], Edge::on_failure("n3", "n4"));
        let mut run = sample_run();
        run.apply_patch(patch, &["codex", "claude"]).expect("valid");
        assert_eq!(run.nodes.len(), 4);
    }

    #[test]
    fn decision_parses_done_without_nodes() {
        let decision = parse_decision("{\"action\":\"done\"}").unwrap();
        assert_eq!(decision.action, Action::Done);
        assert!(decision.nodes.is_empty());
        assert!(decision.into_patch().nodes.is_empty());
    }

    #[test]
    fn decision_rejects_unknown_action() {
        assert!(parse_decision("{\"action\":\"dance\"}").is_err());
        assert!(parse_decision("{\"reasoning\":\"x\"}").is_err());
    }

    #[test]
    fn response_text_concatenates_text_blocks() {
        let body = r#"{"content":[{"type":"text","text":"{\"action\":"},{"type":"tool_use","id":"x"},{"type":"text","text":"\"done\"}"}]}"#;
        let text = response_text(body).unwrap();
        assert_eq!(parse_decision(&text).unwrap().action, Action::Done);
        assert!(response_text("{\"content\":[]}").is_err());
        assert!(response_text("nope").is_err());
    }

    #[test]
    fn plan_refuses_without_a_usable_gateway() {
        let config = GatewayConfig {
            enabled: false,
            ..GatewayConfig::default()
        };
        let input = PlannerInput::from_run(&sample_run(), &agents(), None);
        assert!(plan(&config, DEFAULT_PLANNER_MODEL, &input).is_err());
    }
}
