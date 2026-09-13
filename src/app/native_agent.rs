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
//! only the name and the brand-and-platform subtitle.
//!
//! The wire format — Anthropic Messages, OpenAI Responses, OpenAI Chat
//! Completions — is a property of the model, and each model has exactly one.
//! Claude is served over Messages; the GPT and Grok families over Responses;
//! Chat Completions is for models the user declared on their own endpoint,
//! and is empty until they do. A model belonging to none of those is not
//! offered at all — there is no API to send it over, so listing it would
//! only promise something that fails.
//!
//! That one format lands in the model's "service tier" slot, which is what
//! the picker's format bar partitions the list by and what the composer's
//! traits menu names.
//!
//! Pure mapping plus the hooks that apply it; the fetch is the Plaza's.

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
        .filter(|item| is_chat_model(item) && !item.model.trim().is_empty())
        .filter(|item| seen.insert((platform_of(item), item.model.clone())))
        .filter_map(|item| {
            let platform = platform_of(item);
            // No API to send it over means it is not a choice, however well
            // it reads in a catalog.
            let format = native_format_for_model(&platform, &item.model)?;
            let name = if item.display_name.trim().is_empty() {
                item.model.clone()
            } else {
                item.display_name.clone()
            };
            let mut model = ProviderModel::new(format!("{platform}::{}", item.model), name);
            model.sub_provider = Some(platform.clone());
            if let Some(entry) = native_format_option(format) {
                model = model.service_tiers(
                    [
                        ProviderModelOption::new(entry.id, crate::i18n::translate(entry.label))
                            .description(crate::i18n::translate(entry.description)),
                    ],
                    entry.id,
                );
            }
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
            Some(model)
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

/// Whether this catalog entry is something a coding agent can hold a
/// conversation with.
///
/// The catalog has no modality field, so this reads three weaker signals in
/// order. The first two are what the service knows; the third is what its own
/// gateway does — `IsGPTImageGenerationModel` there is a name prefix too,
/// because a picture model billed by the token looks like a chat model from
/// every angle except its name.
fn is_chat_model(item: &ModelCatalogItem) -> bool {
    let mode = item.billing_mode.trim().to_ascii_lowercase();
    // Empty means token; the service normalizes it that way on the way out.
    if !matches!(mode.as_str(), "" | "token") {
        return false;
    }
    // Priced per picture and not per token: whatever it is billed as, it is
    // not answering questions.
    let pricing = &item.effective_pricing_usd;
    if pricing.per_image_usd.is_some()
        && pricing.input_per_mtok_usd.is_none()
        && pricing.output_per_mtok_usd.is_none()
    {
        return false;
    }
    !is_non_conversational_name(&item.model)
}

/// Families that produce pictures, video, speech or vectors. Matched as
/// prefixes and whole hyphen-separated words rather than substrings, so a
/// conversational model whose name merely mentions images is left alone.
fn is_non_conversational_name(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    const PREFIXES: [&str; 7] = [
        "gpt-image",
        "dall-e",
        "sora",
        "tts-",
        "gpt-4o-mini-tts",
        "whisper",
        "grok-image",
    ];
    if PREFIXES.iter().any(|prefix| model.starts_with(prefix)) {
        return true;
    }
    model
        .split(['-', '.', '/', ':'])
        .any(|word| matches!(word, "embedding" | "embeddings" | "moderation" | "rerank"))
}

/// A reasoning ladder only where the engine maps effort onto a request field
/// the upstream understands: the Anthropic and OpenAI families. Grok takes
/// the Responses route but has no effort field, and a ladder that changed
/// nothing would look connected while doing nothing.
///
/// Name first, platform second, for the same reason the API is chosen that
/// way: a composite group reports `composite` for every model in it.
fn has_reasoning_ladder(platform: &str, model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    if model.starts_with("claude")
        || model.starts_with("gpt-")
        || model.starts_with("gpt5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("codex")
    {
        return true;
    }
    if model.starts_with("grok") {
        return false;
    }
    platform == "anthropic" || platform == "openai"
}

/// The engine resolves `ultracode` to its top reasoning budget, which only
/// the newest families accept; older ones clamp it back to `high` and the
/// entry would be inert.
fn supports_ultracode(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("opus-5") || model.contains("sonnet-5") || model.contains("fable")
}

/// One wire format, as the picker and the traits menu name it.
///
/// Kept in step with `waku_agent_bridge::WireFormat`, which the desktop does
/// not link — the bridge is the daemon's dependency, not the app's. The
/// bridge clamps whatever it receives, so a disagreement here costs a
/// surprising default, never a broken request. `WireFormat::resolve` there
/// is the same rule as [`native_format_for_model`] below.
pub(super) struct WireFormatOption {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

// A `static`, not a `const`: `native_format_option` hands out `'static`
// references into it, and a const would be copied into a temporary at
// every use site with nothing to borrow from.
pub(super) static NATIVE_WIRE_FORMATS: [WireFormatOption; 3] = [
    WireFormatOption {
        id: "messages",
        label: "model_option.wire_messages",
        description: "model_option.wire_messages_description",
    },
    WireFormatOption {
        id: "responses",
        label: "model_option.wire_responses",
        description: "model_option.wire_responses_description",
    },
    WireFormatOption {
        id: "chat",
        label: "model_option.wire_chat",
        description: "model_option.wire_chat_description",
    },
];

/// The one API a catalog model is reachable over, or `None` when it is
/// reachable over none of them and should not be offered.
///
/// The model's own family decides, not the group's platform: a composite
/// group reports `composite` for everything in it, so the platform is only
/// consulted as a tie-breaker for a name that gives nothing away.
///
/// Chat Completions is deliberately absent here. It carries the models a
/// user declared on their own endpoint ([`native_custom_models`]), nothing
/// from the managed catalog.
pub(super) fn native_format_for_model(platform: &str, model: &str) -> Option<&'static str> {
    let model = model.trim().to_ascii_lowercase();
    if model.starts_with("claude") {
        return Some("messages");
    }
    if model.starts_with("gpt-")
        || model.starts_with("gpt5")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("codex")
        || model.starts_with("grok")
    {
        return Some("responses");
    }
    // A name that says nothing: fall back to the group's platform, for the
    // rare model whose id carries no family at all.
    match platform.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Some("messages"),
        "openai" | "grok" => Some("responses"),
        _ => None,
    }
}

/// The format entry for one id, for labelling.
pub(super) fn native_format_option(format: &str) -> Option<&'static WireFormatOption> {
    NATIVE_WIRE_FORMATS.iter().find(|entry| entry.id == format)
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

/// The models a user declared on their own endpoint for the built-in agent.
///
/// These are the Chat Completions list: the managed catalog never puts
/// anything there, because the gateway's own model families each have a
/// better route. An endpoint the user points somewhere else is the one case
/// where this app cannot know what the models are called or what they speak,
/// so it takes their word for it.
pub(super) fn native_custom_models(models: &[String]) -> Vec<ProviderModel> {
    let mut seen = std::collections::HashSet::new();
    models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .filter(|model| seen.insert(model.to_string()))
        .map(|model| {
            let mut entry = ProviderModel::new(model, model);
            entry.sub_provider = Some("custom".to_owned());
            entry.service_tiers(
                [ProviderModelOption::new("chat", crate::i18n::translate("model_option.wire_chat"))
                    .description(crate::i18n::translate("model_option.wire_chat_description"))],
                "chat",
            )
        })
        .collect()
}

/// What the built-in agent's probe lists: the catalog plus the user's own
/// endpoint models, or the engine's fallback list when both are empty. Never
/// empty, so a signed-out picker still has rows and a signed-in one never
/// blanks between a sign-out and the next catalog.
pub(super) fn native_probe_models(
    items: &[ModelCatalogItem],
    custom: &[String],
) -> Vec<ProviderModel> {
    let mut models = native_models_from_catalog(items);
    models.extend(native_custom_models(custom));
    if models.is_empty() {
        crate::model_catalog::fallback_models(ProviderKind::Native)
    } else {
        models
    }
}

impl Waku {
    /// Re-derive the built-in agent's model list from the catalog held in
    /// `model_plaza.items`, the one source of truth for it.
    ///
    /// Idempotent and cheap, so it runs after anything that could have
    /// replaced the probe's list: a catalog landing, a sign-out clearing
    /// it, a daemon probe answering with the fallback list, a language
    /// change relabelling the reasoning ladder.
    pub(super) fn sync_native_models(&mut self) {
        let custom = self
            .custom_api_snapshot()
            .get("native")
            .map(|endpoint| endpoint.models.clone())
            .unwrap_or_default();
        let models = native_probe_models(&self.model_plaza.items, &custom);
        if let Some(probe) = self
            .probes
            .iter_mut()
            .find(|probe| probe.provider == ProviderKind::Native)
        {
            probe.models = models;
        }
    }

    /// Bring the built-in agent's catalog up to date with the account.
    ///
    /// Signed in, this is the Plaza's own fetch — same request, same token
    /// adoption, same freshness window, so the picker and the Plaza page
    /// never disagree; `force` skips the window for events that changed
    /// what the account can reach (a sign-in, a group switch). Signed out,
    /// the list is re-derived at once so it drops back to the fallback.
    pub(super) fn refresh_native_catalog(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.cloud_account.credentials.is_some() {
            self.load_model_plaza_if_needed(force, cx);
        } else {
            self.sync_native_models();
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

    /// The easy half: an operator priced it as pictures. This passed the
    /// whole time the picker was listing `gpt-image-2`, because the live
    /// catalog does not mark them — the name-based test below is that shape.
    #[test]
    fn image_products_are_not_offered_to_a_coding_agent() {
        let mut image = item("gpt-image-2", "openai");
        image.billing_mode = "image".into();
        let models = native_models_from_catalog(&[image, item("gpt-5.6-sol", "openai")]);
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn each_model_carries_the_one_api_it_is_served_over() {
        let format = |models: &[ProviderModel]| -> Option<String> {
            let model = models.first()?;
            let tiers: Vec<&str> = model.service_tiers.iter().map(|t| t.id.as_str()).collect();
            assert_eq!(tiers.len(), 1, "a model has exactly one API");
            assert_eq!(model.default_service_tier.as_deref(), Some(tiers[0]));
            Some(tiers[0].to_owned())
        };

        assert_eq!(
            format(&native_models_from_catalog(&[item("claude-sonnet-5", "anthropic")])),
            Some("messages".into())
        );
        assert_eq!(
            format(&native_models_from_catalog(&[item("gpt-5.6-sol", "openai")])),
            Some("responses".into())
        );
        // The combination that used to default to Messages and get hijacked
        // to the engine's own xai provider.
        assert_eq!(
            format(&native_models_from_catalog(&[item("grok-4.6", "grok")])),
            Some("responses".into())
        );
        // The family beats the group's platform, which is what makes a
        // composite group — every model in it reports `composite` — work.
        assert_eq!(
            format(&native_models_from_catalog(&[item("grok-4.6", "composite")])),
            Some("responses".into())
        );
        assert_eq!(
            format(&native_models_from_catalog(&[item("claude-sonnet-5", "composite")])),
            Some("messages".into())
        );
    }

    #[test]
    fn a_model_with_no_api_to_send_it_over_is_not_offered() {
        // There is no Gemini route here: the gateway has no Responses
        // translator for those groups, and this app holds no Gemini key.
        // Listing it would promise something that fails.
        assert!(native_models_from_catalog(&[item("gemini-3-pro", "gemini")]).is_empty());
        assert!(native_models_from_catalog(&[item("some-unknown-model", "")]).is_empty());
    }

    #[test]
    fn the_chat_section_holds_the_users_own_models_and_nothing_else() {
        // Nothing from the managed catalog lands in Chat Completions.
        let catalog = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("gpt-5.6-sol", "openai"),
            item("grok-4.6", "grok"),
        ]);
        assert!(
            catalog
                .iter()
                .all(|model| model.default_service_tier.as_deref() != Some("chat")),
            "the catalog never fills Chat Completions"
        );

        let declared = native_custom_models(&["my-model".into(), " ".into(), "my-model".into()]);
        let ids: Vec<&str> = declared.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["my-model"], "blank and duplicate entries are dropped");
        assert_eq!(declared[0].default_service_tier.as_deref(), Some("chat"));
        // A bare id, with no platform ahead of a `::`: nothing on the
        // gateway claims it.
        assert!(!declared[0].id.contains("::"));
    }

    #[test]
    fn picture_and_speech_products_are_not_offered_to_a_coding_agent() {
        // The shape the live catalog actually has: the service only marks a
        // model `image` when an operator priced it that way, so these arrive
        // billed by the token and have to be recognised by name.
        for name in [
            "gpt-image-2",
            "gpt-image-2.5-flare",
            "gpt-image-2.5-sunburst",
            "dall-e-3",
            "sora-2",
            "whisper-1",
            "tts-1-hd",
            "text-embedding-3-large",
        ] {
            assert!(
                native_models_from_catalog(&[item(name, "openai")]).is_empty(),
                "{name} should not be offered"
            );
        }

        // A conversational model is caught by none of that.
        for name in ["gpt-5.6-sol", "claude-sonnet-5", "gemini-3-pro"] {
            assert_eq!(
                native_models_from_catalog(&[item(name, "openai")]).len(),
                1,
                "{name} should be offered"
            );
        }
    }

    #[test]
    fn a_model_priced_only_per_picture_is_not_offered() {
        let mut priced = item("some-new-renderer", "openai");
        priced.effective_pricing_usd.per_image_usd = Some(0.04);
        assert!(native_models_from_catalog(&[priced]).is_empty());
    }

    #[test]
    fn video_billing_is_excluded_too() {
        let mut video = item("some-video-model", "grok");
        video.billing_mode = "video".into();
        assert!(native_models_from_catalog(&[video]).is_empty());
    }

    #[test]
    fn an_empty_catalog_lists_the_fallback_rather_than_nothing() {
        let ids = |models: Vec<ProviderModel>| -> Vec<String> {
            models.into_iter().map(|model| model.id).collect()
        };
        let fallback = ids(crate::model_catalog::fallback_models(ProviderKind::Native));
        assert!(!fallback.is_empty());
        assert_eq!(ids(native_probe_models(&[], &[])), fallback);
        // A catalog with nothing a coding agent can drive counts as empty.
        let mut image = item("gpt-image-2", "openai");
        image.billing_mode = "image".into();
        assert_eq!(ids(native_probe_models(&[image], &[])), fallback);
    }

    #[test]
    fn a_catalog_replaces_the_fallback_list_outright() {
        let models = native_probe_models(&[item("claude-sonnet-5", "anthropic")], &[]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["anthropic::claude-sonnet-5"]);
    }

    #[test]
    fn the_users_own_models_are_enough_to_replace_the_fallback() {
        // Signed out of the managed service but pointed at an endpoint of
        // their own: the picker lists what they declared, not the built-in
        // Anthropic list they cannot reach.
        let models = native_probe_models(&[], &["my-model".into()]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["my-model"]);
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
    fn grok_gets_no_reasoning_ladder_it_cannot_use() {
        let ladder = |models: &[ProviderModel]| !models[0].reasoning_efforts.is_empty();
        assert!(!ladder(&native_models_from_catalog(&[item("grok-4.6", "grok")])));
        assert!(ladder(&native_models_from_catalog(&[item("claude-sonnet-5", "anthropic")])));
        // The family carries it through a composite group, where the
        // platform says nothing.
        assert!(ladder(&native_models_from_catalog(&[item("gpt-5.6-sol", "composite")])));
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
