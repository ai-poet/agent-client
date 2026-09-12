//! The built-in agent's model catalog, taken from the signed-in account.
//!
//! Every CLI provider discovers its models by being asked; the daemon runs
//! `codex app-server` or `claude --list-models` and reads the answer. The
//! built-in agent has no command to ask, and what it can actually reach is
//! decided by the account it routes through — so its catalog comes from the
//! same gateway listing the Model Plaza page shows, filtered to the
//! Anthropic-API models the engine speaks, and lands in the provider probe the
//! picker already reads.
//!
//! Pure mapping plus one hook; the fetch is the Plaza's.

use sub2api::client::ModelCatalogItem;

use super::*;
// Explicit rather than relying on the glob: `ProviderModelOption` is used by
// no other view, so nothing guarantees `app.rs` re-exports it.
use crate::model::{ProviderModel, ProviderModelOption};

/// The models the built-in agent may offer, from the gateway catalog.
///
/// Anything on an Anthropic platform (or plainly named `claude-…`) qualifies.
/// Each gets the reasoning ladder the engine understands; models that accept
/// `xhigh` also get `ultracode`, the engine's top effort. The first Sonnet 5
/// is the default, since it is the engine's own default family.
pub(super) fn native_models_from_catalog(items: &[ModelCatalogItem]) -> Vec<ProviderModel> {
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<ProviderModel> = items
        .iter()
        .filter(|item| is_anthropic_model(item))
        .filter(|item| seen.insert(item.model.clone()))
        .map(|item| {
            let name = if item.display_name.trim().is_empty() {
                item.model.clone()
            } else {
                item.display_name.clone()
            };
            let mut ladder = vec!["low", "medium", "high", "xhigh", "max"];
            if supports_ultracode(&item.model) {
                ladder.push("ultracode");
            }
            ProviderModel::new(item.model.clone(), name).reasoning(
                ladder.into_iter().map(|effort| {
                    ProviderModelOption::new(effort, reasoning_effort_label(effort))
                }),
                "high",
            )
        })
        .collect();

    if let Some(default) = models
        .iter()
        .position(|model| model.id.contains("sonnet-5"))
        .or_else(|| (!models.is_empty()).then_some(0))
    {
        models[default].is_default = true;
    }
    models
}

fn is_anthropic_model(item: &ModelCatalogItem) -> bool {
    let platform = item.platform.to_ascii_lowercase();
    platform.contains("anthropic")
        || platform.contains("claude")
        || item.model.to_ascii_lowercase().starts_with("claude")
}

/// The engine resolves `ultracode` to its top reasoning budget, which only
/// the newest families accept; older ones clamp it back to `high` and the
/// entry would be inert.
fn supports_ultracode(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("opus-5") || model.contains("sonnet-5") || model.contains("fable")
}

fn reasoning_effort_label(effort: &str) -> String {
    match effort {
        "low" => tr!("model_option.low"),
        "medium" => tr!("model_option.medium"),
        "high" => tr!("model_option.high"),
        "xhigh" => tr!("model_option.extra_high"),
        "max" => tr!("model_option.max"),
        "ultracode" => tr!("model_option.ultracode"),
        other => other.to_owned(),
    }
}

impl Waku {
    /// Replace the built-in agent's model list with what the gateway
    /// catalog offers. Called when the Plaza catalog lands; an empty
    /// Anthropic selection leaves the fallback list in place rather than
    /// blanking the picker.
    pub(super) fn adopt_native_models_from_catalog(&mut self) {
        let models = native_models_from_catalog(&self.model_plaza.items);
        if models.is_empty() {
            return;
        }
        if let Some(probe) = self
            .probes
            .iter_mut()
            .find(|probe| probe.provider == ProviderKind::Native)
        {
            probe.models = models;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(model: &str, platform: &str) -> ModelCatalogItem {
        ModelCatalogItem {
            model: model.into(),
            display_name: String::new(),
            platform: platform.into(),
            ..ModelCatalogItem::default()
        }
    }

    #[test]
    fn only_anthropic_models_reach_the_built_in_agent() {
        let models = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("gpt-5.6-sol", "openai"),
            item("claude-haiku-4-5", "Claude"),
        ]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["claude-sonnet-5", "claude-haiku-4-5"]);
    }

    #[test]
    fn sonnet_5_is_the_default_when_present() {
        let models = native_models_from_catalog(&[
            item("claude-opus-5", "anthropic"),
            item("claude-sonnet-5", "anthropic"),
        ]);
        assert!(!models[0].is_default);
        assert!(models[1].is_default);
    }

    #[test]
    fn duplicates_across_groups_collapse_to_one_entry() {
        let models = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("claude-sonnet-5", "anthropic"),
        ]);
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn only_the_newest_families_get_ultracode() {
        let models = native_models_from_catalog(&[
            item("claude-opus-5", "anthropic"),
            item("claude-opus-4-6", "anthropic"),
        ]);
        let has_ultracode = |model: &ProviderModel| {
            model
                .reasoning_efforts
                .iter()
                .any(|option| option.id == "ultracode")
        };
        assert!(has_ultracode(&models[0]));
        assert!(!has_ultracode(&models[1]));
    }
}
