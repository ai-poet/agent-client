//! Which of the account's groups each model is sent through.
//!
//! A gateway key belongs to exactly one group, and a group serves only the
//! models its accounts map. Routing by platform alone — one key for
//! "anthropic", one for "openai", one for everything else — therefore sends a
//! model to whatever group happens to hold that platform's key, and a model
//! that group's upstream does not really carry comes back as "no available
//! channel". The model catalog (`GET /api/v1/models/catalog`) lists every
//! (model, group) pair the account can reach, so the choice can be made per
//! model instead.
//!
//! The rule, in order:
//!
//! 1. A model whose family has a bound group — Claude Code's for `anthropic`,
//!    Codex's for `openai`, the general binding when its group is on the
//!    model's own platform — goes through that group when the group lists it.
//!    When the bound group lists nothing at all, the catalog cannot say what
//!    it serves (accounts without a model mapping contribute no entries), so
//!    the binding is trusted and the model is left to the per-platform key,
//!    exactly as before this existed. The same holds for a family whose slot
//!    is still the account default.
//! 2. Otherwise through an active subscription group that lists it.
//! 3. Otherwise through any group that lists it.
//!
//! Within 2 and 3 a group on the model's own platform beats a composite one,
//! which beats any other; then the lower rate wins, then the lower id, so the
//! answer is stable between refreshes.

use std::collections::{BTreeMap, BTreeSet};

use crate::auth::Credentials;
use crate::client::ModelCatalogItem;

/// One group serving one model.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub model: String,
    pub group_id: i64,
    /// The group's platform (`openai`, `deepseek`, `composite`, …).
    pub platform: String,
    pub rate_multiplier: f64,
}

impl Offer {
    /// Every pair the catalog lists. Each catalog item is one (model, group)
    /// pair — its `best_group` is simply its own group — so the items alone
    /// cover every group; `other_groups` repeats them.
    pub fn from_catalog(items: &[ModelCatalogItem]) -> Vec<Offer> {
        items
            .iter()
            .filter(|item| !item.model.trim().is_empty() && item.best_group.id > 0)
            .map(|item| Offer {
                model: item.model.trim().to_owned(),
                group_id: item.best_group.id,
                platform: item.platform.trim().to_ascii_lowercase(),
                rate_multiplier: item.best_group.rate_multiplier,
            })
            .collect()
    }

    /// A group's own listing (`GET /v1/models` with its key), for a
    /// subscription group the catalog says nothing about.
    ///
    /// Only kept where it can be believed: that endpoint falls back to a
    /// platform's default list when the group maps nothing — for a DeepSeek
    /// group that default is Claude's — so a model counts only when its
    /// family is the group's platform, or the group is composite and routes
    /// by family itself.
    pub fn from_group_listing(
        models: &[String],
        group_id: i64,
        platform: &str,
        rate_multiplier: f64,
    ) -> Vec<Offer> {
        let platform = platform.trim().to_ascii_lowercase();
        models
            .iter()
            .map(|model| model.trim())
            .filter(|model| !model.is_empty())
            .filter(|model| platform == "composite" || model_family(model) == Some(platform.as_str()))
            .map(|model| Offer {
                model: model.to_owned(),
                group_id,
                platform: platform.clone(),
                rate_multiplier,
            })
            .collect()
    }
}

/// The groups the user has bound, per CLI slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    /// Claude Code's group — the `anthropic` family's.
    pub claude: Option<i64>,
    /// Codex's group — the `openai` family's.
    pub codex: Option<i64>,
    /// The general key's group, which every other CLI shares.
    pub general: Option<i64>,
    /// The platform of `general`, which decides the family it binds.
    pub general_platform: Option<String>,
}

impl Bindings {
    pub fn from_credentials(credentials: &Credentials, general_platform: Option<String>) -> Self {
        Self {
            claude: credentials.claude_group_id,
            codex: credentials.codex_group_id,
            general: credentials.group_id,
            general_platform: general_platform.map(|platform| platform.trim().to_ascii_lowercase()),
        }
    }
}

/// The platform a model belongs to, from its name — the same reading the
/// gateway's composite groups use to route (`DetectModelPlatform` in the
/// backend), so both sides agree on what a name means. `None` for a name
/// that gives nothing away.
pub fn model_family(model: &str) -> Option<&'static str> {
    let mut name = model.trim().to_ascii_lowercase();
    if let Some(rest) = name.strip_prefix("models/") {
        name = rest.to_owned();
    }
    if let Some((provider, rest)) = name.split_once('/') {
        let family = match provider.trim() {
            "anthropic" | "claude" => Some("anthropic"),
            "openai" | "chatgpt" => Some("openai"),
            "google" | "google-ai-studio" | "gemini" => Some("gemini"),
            "xai" | "x-ai" | "grok" => Some("grok"),
            "kimi" | "moonshot" => Some("kimi"),
            "zhipu" | "glm" | "bigmodel" => Some("zhipu"),
            "deepseek" => Some("deepseek"),
            "minimax" => Some("minimax"),
            _ => None,
        };
        if family.is_some() {
            return family;
        }
        if !rest.trim().is_empty() {
            name = rest.trim().trim_start_matches("models/").to_owned();
        }
    }
    let name = name.as_str();
    let openai_series = ["o1", "o3", "o4", "o5"]
        .iter()
        .any(|prefix| name == *prefix || name.starts_with(&format!("{prefix}-")));
    if name.starts_with("claude-") || name.starts_with("anthropic.claude-") {
        Some("anthropic")
    } else if name.starts_with("gpt-")
        || name.starts_with("chatgpt-")
        || name.starts_with("codex-")
        || openai_series
    {
        Some("openai")
    } else if name.starts_with("gemini-") || name.starts_with("learnlm-") {
        Some("gemini")
    } else if name == "grok" || name.starts_with("grok-") {
        Some("grok")
    } else if name == "k3"
        || name == "k3-256k"
        || name.starts_with("kimi-")
        || name.starts_with("moonshot-")
    {
        Some("kimi")
    } else if name.starts_with("glm-") {
        Some("zhipu")
    } else if name.starts_with("deepseek-") {
        Some("deepseek")
    } else if name.starts_with("minimax-")
        || name.starts_with("abab5")
        || name.starts_with("abab6")
        || name.starts_with("abab7")
    {
        Some("minimax")
    } else {
        None
    }
}

/// Whether a model takes the three-step effort — `low`, `high`, `max` —
/// that its API and the gateway both know: DeepSeek (`low` is its thinking
/// switched off), GLM from 4.5 on (the same switch, and z.ai's high/max
/// scale), and Kimi's K3 family (`reasoning_effort` itself; K3 always
/// thinks). Kimi's K2 line and MiniMax have no depth to choose. The engine's
/// `provider_options` sends each in its own API's shape; this is the
/// picker's half of the same rule.
pub fn has_three_step_effort(model: &str) -> bool {
    let name = model.trim().to_ascii_lowercase();
    let name = name.rsplit('/').next().unwrap_or(&name);
    if name.starts_with("deepseek-") {
        return true;
    }
    if name == "k3" || name == "k3-256k" || name.starts_with("kimi-k3") {
        return true;
    }
    name.strip_prefix("glm-").is_some_and(|version| {
        let number: String = version
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect();
        number.trim_end_matches('.').parse::<f64>().is_ok_and(|version| version >= 4.5)
    })
}

/// Where rule 1 stands for one family.
enum Slot {
    /// A group the user bound for this family.
    Bound(i64),
    /// The family's slot holds the account default key, whose group is
    /// unknown: leave the model to the per-platform key.
    AccountDefault,
    /// No slot speaks for this family.
    Unbound,
}

fn slot_for(family: Option<&str>, bindings: &Bindings) -> Slot {
    let bound = |group: Option<i64>| group.map_or(Slot::AccountDefault, Slot::Bound);
    match family {
        Some("anthropic") => bound(bindings.claude),
        Some("openai") => bound(bindings.codex),
        Some(family) => match bindings.general {
            Some(group) if bindings.general_platform.as_deref() == Some(family) => Slot::Bound(group),
            // The general key is Grok's; left as the account default it
            // still routes Grok the way it always has.
            None if family == "grok" => Slot::AccountDefault,
            _ => Slot::Unbound,
        },
        None => Slot::Unbound,
    }
}

/// The group each model goes through. Models absent from the result keep
/// the per-platform key.
pub fn resolve(
    offers: &[Offer],
    bindings: &Bindings,
    subscriptions: &BTreeSet<i64>,
) -> BTreeMap<String, i64> {
    let listing_groups: BTreeSet<i64> = offers.iter().map(|offer| offer.group_id).collect();
    let mut by_model: BTreeMap<&str, Vec<&Offer>> = BTreeMap::new();
    for offer in offers {
        by_model.entry(offer.model.as_str()).or_default().push(offer);
    }

    let mut routes = BTreeMap::new();
    for (model, candidates) in by_model {
        let family = model_family(model);
        match slot_for(family, bindings) {
            Slot::Bound(group) if candidates.iter().any(|offer| offer.group_id == group) => {
                routes.insert(model.to_owned(), group);
                continue;
            }
            // The bound group lists nothing, so nothing says it does not
            // serve this model: trust the binding.
            Slot::Bound(group) if !listing_groups.contains(&group) => continue,
            Slot::AccountDefault => continue,
            _ => {}
        }
        let rank = |offer: &&&Offer| {
            (
                Some(offer.platform.as_str()) != family,
                offer.platform != "composite",
                // Rates are compared as integers so the ordering is total;
                // a missing rate (0) sorts as cheapest, like the gateway's own
                // "group default".
                (offer.rate_multiplier.max(0.0) * 1_000_000.0).round() as i64,
                offer.group_id,
            )
        };
        let subscribed = candidates
            .iter()
            .filter(|offer| subscriptions.contains(&offer.group_id))
            .min_by_key(rank);
        if let Some(offer) = subscribed.or_else(|| candidates.iter().min_by_key(rank)) {
            routes.insert(model.to_owned(), offer.group_id);
        }
    }
    routes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(model: &str, group_id: i64, platform: &str, rate: f64) -> Offer {
        Offer {
            model: model.into(),
            group_id,
            platform: platform.into(),
            rate_multiplier: rate,
        }
    }

    /// Codex group 7 (openai), Grok group 9 bound as the general key,
    /// subscription group 20 (deepseek).
    fn bindings() -> Bindings {
        Bindings {
            claude: Some(3),
            codex: Some(7),
            general: Some(9),
            general_platform: Some("grok".into()),
        }
    }

    /// The report that started this: the Codex group's upstream also lists
    /// `deepseek-v4.1-flash` and answers 503 for it. A DeepSeek model is not
    /// the Codex binding's to route, so the subscription group that serves it
    /// wins — while GPT stays on Codex and Grok on its own group even though
    /// the subscription lists them too.
    #[test]
    fn a_family_without_a_binding_goes_to_the_subscription() {
        let offers = [
            offer("gpt-5.6-sol", 7, "openai", 1.0),
            offer("gpt-5.6-sol", 20, "composite", 0.5),
            offer("deepseek-v4.1-flash", 7, "openai", 1.0),
            offer("deepseek-v4.1-flash", 20, "composite", 0.5),
            offer("grok-4.6", 9, "grok", 1.0),
            offer("grok-4.6", 20, "composite", 0.5),
        ];
        let routes = resolve(&offers, &bindings(), &BTreeSet::from([20]));
        assert_eq!(routes.get("gpt-5.6-sol"), Some(&7));
        assert_eq!(routes.get("grok-4.6"), Some(&9));
        assert_eq!(routes.get("deepseek-v4.1-flash"), Some(&20));
    }

    /// With no subscription serving it, a group on the model's own platform
    /// beats a composite one, which beats the Codex group that merely lists
    /// it; the cheaper rate only breaks ties within that order.
    #[test]
    fn without_a_subscription_the_models_own_platform_wins_then_the_rate() {
        let offers = [
            offer("deepseek-v4.1-flash", 7, "openai", 0.1),
            offer("deepseek-v4.1-flash", 30, "composite", 0.2),
            offer("deepseek-v4.1-flash", 31, "deepseek", 0.9),
            offer("deepseek-v4.1-flash", 32, "deepseek", 0.6),
            offer("glm-5", 7, "openai", 0.1),
            offer("glm-5", 30, "composite", 0.2),
        ];
        let routes = resolve(&offers, &bindings(), &BTreeSet::new());
        assert_eq!(routes.get("deepseek-v4.1-flash"), Some(&32));
        assert_eq!(routes.get("glm-5"), Some(&30));
    }

    /// A bound group that lists nothing — accounts without a model mapping
    /// contribute no catalog entries — is trusted, and the model keeps the
    /// per-platform key. A bound group that lists other models but not this
    /// one demonstrably lacks it, so the model moves.
    #[test]
    fn a_silent_bound_group_is_trusted_and_a_listing_one_is_checked() {
        let offers = [
            offer("gpt-5.6-sol", 20, "composite", 0.5),
            offer("claude-opus-5-5", 20, "composite", 0.5),
            offer("claude-sonnet-5", 3, "anthropic", 1.0),
        ];
        let routes = resolve(&offers, &bindings(), &BTreeSet::from([20]));
        // Codex group 7 lists nothing: GPT stays on the Codex key.
        assert_eq!(routes.get("gpt-5.6-sol"), None);
        // Claude group 3 lists Sonnet but not Opus 5.5: Opus moves.
        assert_eq!(routes.get("claude-sonnet-5"), Some(&3));
        assert_eq!(routes.get("claude-opus-5-5"), Some(&20));
    }

    /// A slot still on the account default has no known group, so its
    /// family is left alone; a general binding on another platform does not
    /// speak for Grok.
    #[test]
    fn account_default_slots_keep_their_key() {
        let offers = [
            offer("gpt-5.6-sol", 20, "composite", 0.5),
            offer("grok-4.6", 20, "composite", 0.5),
        ];
        let defaults = Bindings::default();
        let routes = resolve(&offers, &defaults, &BTreeSet::from([20]));
        assert!(routes.is_empty(), "{routes:?}");

        let general_elsewhere = Bindings {
            general: Some(40),
            general_platform: Some("deepseek".into()),
            ..Bindings::default()
        };
        let routes = resolve(&offers, &general_elsewhere, &BTreeSet::from([20]));
        assert_eq!(routes.get("grok-4.6"), Some(&20));
    }

    /// Same reading of a name as the gateway's composite routing.
    #[test]
    fn families_follow_the_gateways_reading_of_a_name() {
        assert_eq!(model_family("claude-opus-5-5"), Some("anthropic"));
        assert_eq!(model_family("anthropic/claude-sonnet-5"), Some("anthropic"));
        assert_eq!(model_family("gpt-6-sol"), Some("openai"));
        assert_eq!(model_family("o3"), Some("openai"));
        assert_eq!(model_family("o4-mini"), Some("openai"));
        assert_eq!(model_family("grok-4.7"), Some("grok"));
        assert_eq!(model_family("deepseek-v4.1-flash"), Some("deepseek"));
        assert_eq!(model_family("DeepSeek-V4.1-Flash"), Some("deepseek"));
        assert_eq!(model_family("kimi-k3"), Some("kimi"));
        assert_eq!(model_family("k3"), Some("kimi"));
        assert_eq!(model_family("glm-5"), Some("zhipu"));
        assert_eq!(model_family("minimax-m3"), Some("minimax"));
        assert_eq!(model_family("gemini-3-pro"), Some("gemini"));
        assert_eq!(model_family("openrouter/deepseek-v4.1-flash"), Some("deepseek"));
        assert_eq!(model_family("qwen3-coder"), None);
        assert_eq!(model_family("o3x"), None);
    }

    #[test]
    fn the_three_step_effort_covers_deepseek_glm_and_kimi_k3() {
        for model in ["deepseek-v4.1-flash", "glm-4.5-air", "glm-5", "glm-5.3", "kimi-k3", "k3", "k3-256k", "moonshot/kimi-k3"] {
            assert!(has_three_step_effort(model), "{model}");
        }
        for model in ["glm-4-plus", "glm-z1", "kimi-k2.6", "kimi-k2-thinking", "minimax-m3", "gpt-5.6-sol"] {
            assert!(!has_three_step_effort(model), "{model}");
        }
    }

    /// A group's own `/v1/models` is believed only where it cannot be a
    /// platform default in disguise.
    #[test]
    fn a_group_listing_keeps_only_what_it_can_vouch_for() {
        let listed = ["deepseek-v4.1-flash".to_owned(), "claude-sonnet-5".to_owned()];
        let deepseek = Offer::from_group_listing(&listed, 20, "deepseek", 0.5);
        assert_eq!(deepseek, [offer("deepseek-v4.1-flash", 20, "deepseek", 0.5)]);
        let composite = Offer::from_group_listing(&listed, 21, "Composite", 0.5);
        assert_eq!(composite.len(), 2);
    }

    #[test]
    fn catalog_items_become_offers() {
        let item = ModelCatalogItem {
            model: " deepseek-v4.1-flash ".into(),
            platform: "Composite".into(),
            best_group: crate::client::GroupRef {
                id: 20,
                name: "DeepSeek 订阅".into(),
                rate_multiplier: 0.5,
                rate_source: String::new(),
            },
            ..ModelCatalogItem::default()
        };
        let orphan = ModelCatalogItem {
            model: "gpt-5.6-sol".into(),
            ..ModelCatalogItem::default()
        };
        assert_eq!(
            Offer::from_catalog(&[item, orphan]),
            [offer("deepseek-v4.1-flash", 20, "composite", 0.5)]
        );
    }
}
