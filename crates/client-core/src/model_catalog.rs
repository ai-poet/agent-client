//! Built-in-agent fallback model metadata.
//!
//! The authoritative catalog is fetched from the selected gateway origin by the
//! desktop. This list only keeps a signed-out or offline picker usable; no CLI
//! process is queried for models.

use crate::model::{ProviderAgentPreset, ProviderKind, ProviderModel, ProviderModelOption};

pub fn fallback_models(provider: ProviderKind) -> Vec<ProviderModel> {
    if provider != ProviderKind::Native {
        return Vec::new();
    }
    vec![
        claude_ultracode_model("claude-sonnet-5-5", "Claude Sonnet 5.5").default(),
        claude_ultracode_model("claude-fable-5-1", "Claude Fable 5.1"),
        claude_ultracode_model("claude-opus-5", "Claude Opus 5"),
        claude_ultracode_model("claude-sonnet-5", "Claude Sonnet 5"),
        claude_reasoning_model("claude-opus-4-6", "Claude Opus 4.6"),
        claude_reasoning_model("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        ProviderModel::new("claude-haiku-4-5", "Claude Haiku 4.5"),
    ]
}

pub fn fallback_agent_presets(_provider: ProviderKind) -> Vec<ProviderAgentPreset> {
    Vec::new()
}

fn reasoning_options<const N: usize>(efforts: [&str; N]) -> Vec<ProviderModelOption> {
    efforts
        .into_iter()
        .map(|effort| ProviderModelOption::new(effort, reasoning_effort_label(effort)))
        .collect()
}

fn reasoning_effort_label(effort: &str) -> String {
    match effort {
        "none" => tr!("model_option.none"),
        "minimal" => tr!("model_option.minimal"),
        "low" => tr!("model_option.low"),
        "medium" => tr!("model_option.medium"),
        "high" => tr!("model_option.high"),
        "xhigh" => tr!("model_option.extra_high"),
        "max" => tr!("model_option.max"),
        "ultra" => tr!("model_option.ultra"),
        "ultracode" => tr!("model_option.ultracode"),
        other => other.to_owned(),
    }
}

fn claude_reasoning_model(id: &str, name: &str) -> ProviderModel {
    ProviderModel::new(id, name).reasoning(
        reasoning_options(["low", "medium", "high", "xhigh", "max"]),
        "high",
    )
}

fn claude_ultracode_model(id: &str, name: &str) -> ProviderModel {
    ProviderModel::new(id, name).reasoning(
        reasoning_options(["low", "medium", "high", "xhigh", "max", "ultracode"]),
        "high",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_native_has_a_fallback_catalog() {
        assert!(fallback_models(ProviderKind::Native)[0].is_default);
        assert!(fallback_models(ProviderKind::Claude).is_empty());
        assert!(fallback_agent_presets(ProviderKind::Native).is_empty());
    }
}
