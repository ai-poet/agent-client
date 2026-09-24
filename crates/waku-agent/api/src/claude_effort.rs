//! How a Claude request carries its reasoning effort.
//!
//! Fork departure (Waku). The engine turned every effort level into a
//! thinking budget (`thinking: {type: "enabled", budget_tokens}`). The current
//! Claude families take effort differently — adaptive thinking plus
//! `output_config.effort`, which is what Claude Code sends — and Opus 5.5
//! rejects a budget outright. Gateways read the effort from that field too,
//! so a budget-only request reported no effort at all.
//!
//! [`effort_levels`] is the list of families that take `output_config.effort`
//! and the levels each accepts, kept in step with the gateway's own table
//! (`backend/internal/pkg/claude/effort_catalog.go`). Opus 4.5 is left out on
//! purpose: its effort still needs a beta header, so it keeps its budget.
//! Every model not in the list keeps the budget too.

use claurst_core::effort::EffortLevel;

use crate::types::{OutputConfig, ThinkingConfig};

const LOW_MEDIUM_HIGH_MAX: &[&str] = &["low", "medium", "high", "max"];
const LOW_TO_MAX_WITH_XHIGH: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Families that take `output_config.effort`, most specific first.
const FAMILIES: &[(&str, &[&str])] = &[
    ("claude-mythos-preview", LOW_MEDIUM_HIGH_MAX),
    ("claude-mythos-5", LOW_TO_MAX_WITH_XHIGH),
    ("claude-fable-5", LOW_TO_MAX_WITH_XHIGH),
    ("claude-sonnet-4-6", LOW_MEDIUM_HIGH_MAX),
    ("claude-sonnet-5", LOW_TO_MAX_WITH_XHIGH),
    ("claude-opus-4-8", LOW_TO_MAX_WITH_XHIGH),
    ("claude-opus-4-7", LOW_TO_MAX_WITH_XHIGH),
    ("claude-opus-4-6", LOW_MEDIUM_HIGH_MAX),
    ("claude-opus-5-5", LOW_TO_MAX_WITH_XHIGH),
    ("claude-opus-5", LOW_TO_MAX_WITH_XHIGH),
];

/// The order effort names rank in, lightest first.
const RANK: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// What a Claude request sends for reasoning.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeReasoning {
    /// Adaptive thinking at an effort the model accepts.
    Effort(String),
    /// A thinking budget: models outside [`effort_levels`].
    Budget(u32),
    /// Neither — the model's own default.
    Default,
}

impl ClaudeReasoning {
    /// The request fields for this choice: `thinking`, then `output_config`.
    pub fn fields(&self) -> (Option<ThinkingConfig>, Option<OutputConfig>) {
        match self {
            Self::Effort(effort) => (
                Some(ThinkingConfig::adaptive()),
                Some(OutputConfig {
                    effort: effort.clone(),
                }),
            ),
            Self::Budget(budget) => (Some(ThinkingConfig::enabled(*budget)), None),
            Self::Default => (None, None),
        }
    }

    /// Whether the request runs with thinking on, which rules out a
    /// temperature other than the default.
    pub fn thinks(&self) -> bool {
        !matches!(self, Self::Default)
    }
}

/// The effort levels `model` accepts through `output_config.effort`, or
/// `None` when it takes a thinking budget instead.
pub fn effort_levels(model: &str) -> Option<&'static [&'static str]> {
    let id = normalize_model_id(model);
    FAMILIES
        .iter()
        .find(|(family, _)| id == *family || id.starts_with(&format!("{family}-")))
        .map(|(_, levels)| *levels)
}

/// Decide what a request to `model` sends for reasoning. `effort` is the
/// picker's level; `budget` is the budget the engine would otherwise send (an
/// explicit setting, or the level's own).
pub fn claude_reasoning(
    model: &str,
    effort: Option<EffortLevel>,
    budget: Option<u32>,
) -> ClaudeReasoning {
    let Some(levels) = effort_levels(model) else {
        return budget.map_or(ClaudeReasoning::Default, ClaudeReasoning::Budget);
    };
    match effort.and_then(|effort| clamp(effort, levels)) {
        Some(level) => ClaudeReasoning::Effort(level.to_string()),
        None => ClaudeReasoning::Default,
    }
}

/// The accepted level for `effort`: itself when the model has it, else the
/// highest accepted level below it, else the lowest. `None` for no effort.
fn clamp(effort: EffortLevel, levels: &'static [&'static str]) -> Option<&'static str> {
    let wanted = match effort {
        EffortLevel::None => return None,
        EffortLevel::Minimal | EffortLevel::Low => "low",
        EffortLevel::Medium => "medium",
        EffortLevel::High => "high",
        EffortLevel::XHigh => "xhigh",
        EffortLevel::Max | EffortLevel::Ultracode => "max",
    };
    let rank = |level: &str| RANK.iter().position(|candidate| *candidate == level);
    let wanted_rank = rank(wanted)?;
    levels
        .iter()
        .copied()
        .filter(|level| rank(level).is_some_and(|r| r <= wanted_rank))
        .next_back()
        .or_else(|| levels.first().copied())
}

/// A model id as the families above name it: no provider or `anthropic.`
/// prefix, no `-thinking` suffix, no date stamp.
fn normalize_model_id(model: &str) -> String {
    let mut id = model.trim().to_ascii_lowercase();
    if let Some((_, rest)) = id.split_once("::") {
        id = rest.to_string();
    }
    if let Some((_, rest)) = id.rsplit_once('/') {
        id = rest.to_string();
    }
    let id = id.strip_prefix("anthropic.").unwrap_or(&id).to_string();
    let id = id.strip_suffix("-thinking").unwrap_or(&id).to_string();
    let bytes = id.as_bytes();
    if bytes.len() > 9
        && bytes[bytes.len() - 9] == b'-'
        && bytes[bytes.len() - 8..].iter().all(u8::is_ascii_digit)
    {
        return id[..id.len() - 9].to_string();
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_claude_families_send_adaptive_thinking_with_an_effort() {
        assert_eq!(
            claude_reasoning("claude-opus-5-5", Some(EffortLevel::High), Some(10_000)),
            ClaudeReasoning::Effort("high".into())
        );
        assert_eq!(
            claude_reasoning("claude-opus-5", Some(EffortLevel::Ultracode), Some(20_000)),
            ClaudeReasoning::Effort("max".into())
        );
        assert_eq!(
            claude_reasoning("claude-sonnet-5", Some(EffortLevel::Low), None),
            ClaudeReasoning::Effort("low".into())
        );
    }

    #[test]
    fn a_level_the_model_lacks_falls_to_the_nearest_below() {
        assert_eq!(
            claude_reasoning("claude-sonnet-4-6", Some(EffortLevel::XHigh), Some(16_000)),
            ClaudeReasoning::Effort("high".into())
        );
        assert_eq!(
            claude_reasoning("claude-opus-4-6", Some(EffortLevel::Minimal), Some(1_024)),
            ClaudeReasoning::Effort("low".into())
        );
    }

    #[test]
    fn no_effort_leaves_the_model_to_its_default() {
        assert_eq!(
            claude_reasoning("claude-opus-5-5", None, None),
            ClaudeReasoning::Default
        );
        // Even with a budget from settings: Opus 5.5 would reject it.
        assert_eq!(
            claude_reasoning("claude-opus-5-5", None, Some(8_000)),
            ClaudeReasoning::Default
        );
        assert_eq!(
            claude_reasoning("claude-opus-5", Some(EffortLevel::None), None),
            ClaudeReasoning::Default
        );
    }

    #[test]
    fn older_models_keep_their_thinking_budget() {
        assert_eq!(
            claude_reasoning(
                "claude-haiku-4-5-20251001",
                Some(EffortLevel::High),
                Some(10_000)
            ),
            ClaudeReasoning::Budget(10_000)
        );
        assert_eq!(
            claude_reasoning("claude-opus-4-5", Some(EffortLevel::High), Some(10_000)),
            ClaudeReasoning::Budget(10_000)
        );
        assert_eq!(
            claude_reasoning("claude-sonnet-4-5", None, None),
            ClaudeReasoning::Default
        );
    }

    #[test]
    fn model_ids_are_matched_however_they_are_written() {
        for id in [
            "claude-opus-5-20260101",
            "anthropic/claude-opus-5",
            "anthropic::claude-opus-5",
            "anthropic.claude-opus-5",
            "claude-opus-5-thinking",
            "Claude-Opus-5",
        ] {
            assert!(effort_levels(id).is_some(), "{id} should take an effort");
        }
        // Opus 5.5 is its own family, not Opus 5 with a suffix.
        assert_eq!(
            effort_levels("claude-opus-5-5"),
            Some(LOW_TO_MAX_WITH_XHIGH)
        );
        assert!(effort_levels("claude-opus-50").is_none());
    }

    #[test]
    fn the_request_fields_match_what_claude_code_sends() {
        let (thinking, output) = ClaudeReasoning::Effort("high".into()).fields();
        let thinking = serde_json::to_value(thinking.unwrap()).unwrap();
        assert_eq!(thinking, serde_json::json!({"type": "adaptive"}));
        let output = serde_json::to_value(output.unwrap()).unwrap();
        assert_eq!(output, serde_json::json!({"effort": "high"}));

        let (thinking, output) = ClaudeReasoning::Budget(5_000).fields();
        assert_eq!(
            serde_json::to_value(thinking.unwrap()).unwrap(),
            serde_json::json!({"type": "enabled", "budget_tokens": 5000})
        );
        assert!(output.is_none());
    }
}
