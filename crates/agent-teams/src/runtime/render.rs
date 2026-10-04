// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! `agent_teams_status`: the snapshot and its compact text for the model.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::quality::{CoverageRow, DeliveryResult};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusMember {
    pub name: String,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: String,
    pub status: String,
    pub activity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusTask {
    pub id: String,
    pub subject: String,
    pub status: String,
    pub assignee: String,
    pub dependencies: Vec<String>,
    pub attempt: u64,
    pub attempt_id: String,
    pub reassigning: bool,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplemental_evidence: Option<String>,
    pub findings_open: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusInboxItem {
    pub from: String,
    pub content: String,
    pub ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusInboxSummary {
    pub count: usize,
    pub latest: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusProfile {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_planning: Option<String>,
}

/// What `agent_teams_status` returns.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusView {
    pub team_id: String,
    pub team_name: String,
    pub description: String,
    pub phase: String,
    pub halted: bool,
    pub escalated: bool,
    pub loop_state: String,
    pub loop_summary: String,
    pub deliverable: bool,
    pub coverage: Vec<CoverageRow>,
    pub delivery: DeliveryResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<StatusProfile>,
    pub viewer: String,
    pub members: Vec<StatusMember>,
    pub tasks: Vec<StatusTask>,
    pub captain_inbox: Vec<StatusInboxItem>,
    pub member_inbox: Vec<StatusInboxItem>,
    pub member_inboxes: BTreeMap<String, StatusInboxSummary>,
    pub mailbox_warnings: Vec<String>,
    pub mailbox_warning_count: usize,
}

fn prefix(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

impl StatusView {
    /// The text the model reads.
    pub fn render(&self) -> String {
        let mut flags = Vec::new();
        if self.halted {
            flags.push("halted".to_owned());
        }
        if self.escalated {
            flags.push("escalated".to_owned());
        }
        if self.deliverable {
            flags.push("deliverable".to_owned());
        }
        if !self.loop_state.is_empty()
            && !matches!(self.loop_state.as_str(), "running" | "halted" | "escalated")
        {
            flags.push(self.loop_state.clone());
        }
        let description = if self.description.is_empty() {
            String::new()
        } else {
            format!(" — {}", self.description)
        };
        let flags = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(", "))
        };
        let mut lines = vec![format!("Team \"{}\"{description}{flags}", self.team_name)];
        if let Some(profile) = &self.profile {
            let planning = profile
                .task_planning
                .as_ref()
                .map(|planning| format!(" [{planning}]"))
                .unwrap_or_default();
            let protocol = profile
                .protocol
                .as_ref()
                .filter(|protocol| !protocol.is_empty())
                .map(|protocol| format!(" — {protocol}"))
                .unwrap_or_default();
            lines.push(format!("Profile: {}{planning}{protocol}", profile.name));
        }
        if !self.loop_summary.is_empty() {
            if self.loop_state.is_empty() {
                lines.push(format!("Loop: {}", self.loop_summary));
            } else {
                lines.push(format!("Loop: {} — {}", self.loop_state, self.loop_summary));
            }
        }
        lines.push(format!("Viewing as: {}", self.viewer));
        lines.push(format!("Members ({}):", self.members.len()));
        for member in &self.members {
            let route = if !member.provider.is_empty() && !member.model.is_empty() {
                format!(" · {}/{}", member.provider, member.model)
            } else {
                String::new()
            };
            let effort = if member.reasoning_effort.is_empty() {
                String::new()
            } else {
                format!(" · reasoning {}", member.reasoning_effort)
            };
            let failure = member
                .spawn_error
                .as_ref()
                .map(|error| format!("\n      start failed: {}", prefix(error, 400)))
                .unwrap_or_default();
            lines.push(format!(
                "  - {} [{}] {}/{}{route}{effort}{failure}",
                member.name, member.role, member.status, member.activity
            ));
        }
        lines.push(format!("Tasks ({}):", self.tasks.len()));
        for task in &self.tasks {
            let deps = if task.dependencies.is_empty() {
                String::new()
            } else {
                format!(" (deps: {})", task.dependencies.join(","))
            };
            let output = task
                .output
                .as_ref()
                .map(|output| format!("\n      output: {}", prefix(output, 300)))
                .unwrap_or_default();
            let evidence = task
                .supplemental_evidence
                .as_ref()
                .filter(|evidence| !evidence.is_empty())
                .map(|evidence| {
                    format!(
                        "\n      Supplemental observations (original verdict unchanged): {evidence}"
                    )
                })
                .unwrap_or_default();
            let handoff = if task.reassigning {
                " (reassigning)"
            } else {
                ""
            };
            let seed = task
                .seed_id
                .as_ref()
                .filter(|seed| !seed.is_empty())
                .map(|seed| format!(" seed {seed}"))
                .unwrap_or_default();
            let kind = if task.kind.is_empty() {
                String::new()
            } else {
                format!(" {}", task.kind)
            };
            let round = task
                .round
                .map(|round| format!(" r{round}"))
                .unwrap_or_default();
            let verdict = task
                .verdict
                .as_ref()
                .map(|verdict| format!(" verdict {verdict}"))
                .unwrap_or_default();
            let assignee = if task.assignee.is_empty() {
                "unassigned"
            } else {
                &task.assignee
            };
            lines.push(format!(
                "  - {} [{}]{kind}{round}{verdict} attempt {}{handoff}{seed} {} → {assignee}{deps}{output}{evidence}",
                task.id, task.status, task.attempt, task.subject
            ));
        }
        if !self.coverage.is_empty() {
            lines.push("Coverage:".to_owned());
            for row in &self.coverage {
                let ids = if row.task_ids.is_empty() {
                    "none".to_owned()
                } else {
                    row.task_ids.join(",")
                };
                lines.push(format!(
                    "  - {}: {} ({ids})",
                    row.goal_item,
                    row.status.as_str()
                ));
            }
        }
        lines.push(format!(
            "Delivery: {}",
            if self.delivery.ok {
                "ok".to_owned()
            } else {
                format!("blocked ({})", self.delivery.blockers.join("; "))
            }
        ));
        lines.push(format!("Captain inbox ({}):", self.captain_inbox.len()));
        for message in &self.captain_inbox {
            lines.push(format!("  - [{}] {}", message.from, message.content));
        }
        for message in &self.member_inbox {
            lines.push(format!("  - [{}] {}", message.from, message.content));
        }
        for (name, inbox) in &self.member_inboxes {
            lines.push(format!(
                "Member inbox {name} ({}): latest — {}",
                inbox.count,
                prefix(&inbox.latest, 120)
            ));
        }
        if self.mailbox_warning_count > 0 {
            lines.push(format!(
                "Mailbox warnings ({}; malformed lines were skipped; showing up to 10):",
                self.mailbox_warning_count
            ));
            for warning in &self.mailbox_warnings {
                lines.push(format!("  - {warning}"));
            }
        }
        lines.join("\n")
    }
}
