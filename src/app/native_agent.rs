//! The built-in agent's model catalog, taken from the signed-in account.
//!
//! Every CLI provider discovers its models by being asked; the daemon runs
//! `codex app-server` or `claude --list-models` and reads the answer. The
//! built-in agent has no command to ask, and what it can actually reach is
//! decided by the account it routes through — so its catalog is the gateway
//! listing the Model Plaza page shows, every platform of it, and lands in the
//! provider probe the picker already reads.
//!
//! The id carries the platform ahead of a `::`, which is how the daemon
//! knows which of the account's keys to use; the picker never shows an id,
//! only the name and the brand-and-platform subtitle. The wire format —
//! Anthropic Messages, OpenAI Responses, OpenAI Chat Completions — is not a
//! per-model option here: the picker sections the built-in agent's list by
//! format and writes the chosen section into the session's tier slot, so
//! model and API are always chosen together. The gateway translates each
//! format for every platform, which is what makes that a free choice.
//!
//! Pure mapping plus one hook; the fetch is the Plaza's.

use sub2api::client::ModelCatalogItem;

use super::*;
// Explicit rather than relying on the glob: `ProviderModelOption` is used by
// no other view, so nothing guarantees `app.rs` re-exports it.
use crate::model::{ProviderModel, ProviderModelOption};

/// The models the built-in agent may offer, from the gateway catalog.
///
/// Token-billed models on every platform qualify; image and per-request
/// products are not something a coding agent can drive. Anthropic and OpenAI
/// families get the reasoning ladder the engine understands, with
/// `ultracode` on the ones that accept `xhigh`. The first Sonnet 5 is the
/// default, since it is the engine's own default family.
pub(super) fn native_models_from_catalog(items: &[ModelCatalogItem]) -> Vec<ProviderModel> {
    let mut seen = std::collections::HashSet::new();
    let mut models: Vec<ProviderModel> = items
        .iter()
        .filter(|item| is_token_model(item) && !item.model.trim().is_empty())
        .filter(|item| seen.insert((platform_of(item), item.model.clone())))
        .map(|item| {
            let platform = platform_of(item);
            let name = if item.display_name.trim().is_empty() {
                item.model.clone()
            } else {
                item.display_name.clone()
            };
            let mut model = ProviderModel::new(format!("{platform}::{}", item.model), name);
            model.sub_provider = Some(platform.clone());
            if has_reasoning_ladder(&platform, &item.model) {
                let mut ladder = vec!["low", "medium", "high", "xhigh", "max"];
                if supports_ultracode(&item.model) {
                    ladder.push("ultracode");
                }
                model = model.reasoning(
                    ladder.into_iter().map(|effort| {
                        ProviderModelOption::new(effort, reasoning_effort_label(effort))
                    }),
                    "high",
                );
            }
            model
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

fn platform_of(item: &ModelCatalogItem) -> String {
    let platform = item.platform.trim().to_ascii_lowercase();
    if platform.is_empty() {
        if item.model.to_ascii_lowercase().starts_with("claude") {
            "anthropic".to_owned()
        } else {
            "default".to_owned()
        }
    } else {
        platform
    }
}

fn is_token_model(item: &ModelCatalogItem) -> bool {
    matches!(item.billing_mode.trim(), "" | "token")
}

fn has_reasoning_ladder(platform: &str, model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    platform == "anthropic" || platform == "openai" || model.starts_with("claude")
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
    /// selection leaves the fallback list in place rather than blanking the
    /// picker.
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
    fn every_token_model_on_every_platform_is_offered_with_its_platform_in_the_id() {
        let models = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("gpt-5.6-sol", "openai"),
            item("gemini-3-pro", "gemini"),
        ]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            ["anthropic::claude-sonnet-5", "openai::gpt-5.6-sol", "gemini::gemini-3-pro"]
        );
        assert_eq!(models[2].sub_provider.as_deref(), Some("gemini"));
    }

    #[test]
    fn image_products_are_not_offered_to_a_coding_agent() {
        let mut image = item("gpt-image-2", "openai");
        image.billing_mode = "image".into();
        let models = native_models_from_catalog(&[image, item("gpt-5.6-sol", "openai")]);
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn the_format_is_the_pickers_section_not_a_per_model_tier() {
        let models = native_models_from_catalog(&[item("gpt-5.6-sol", "openai")]);
        assert!(models[0].service_tiers.is_empty());
        assert!(models[0].default_service_tier.is_none());
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
