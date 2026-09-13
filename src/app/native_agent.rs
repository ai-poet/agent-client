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
//! Completions — belongs to the model, not to the session. Each platform is
//! served over the route its upstream actually speaks, and two combinations
//! do not exist at all: the gateway has no Responses translator for Gemini
//! groups, and the engine's Chat client refuses `gpt-5*`/`o3*`/`o4*`. So
//! every model carries the formats that can carry *it*, in the "service
//! tier" slot the picker and the composer's traits menu both read.
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
        .map(|item| {
            let platform = platform_of(item);
            let name = if item.display_name.trim().is_empty() {
                item.model.clone()
            } else {
                item.display_name.clone()
            };
            let mut model = ProviderModel::new(format!("{platform}::{}", item.model), name);
            model.sub_provider = Some(platform.clone());
            let formats = native_wire_formats(&platform, &item.model);
            model = model.service_tiers(
                formats.iter().map(|format| {
                    ProviderModelOption::new(format.id, crate::i18n::translate(format.label))
                        .description(crate::i18n::translate(format.description))
                }),
                native_default_wire_format(&platform, &item.model),
            );
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

/// One wire format, as the picker and the traits menu name it.
///
/// Kept in step with `waku_agent_bridge::WireFormat`, which the desktop does
/// not link — the bridge is the daemon's dependency, not the app's. The
/// bridge clamps whatever it receives, so a disagreement here costs a
/// surprising default, never a broken request. `available_for` there is the
/// same rule as [`native_wire_formats`] below.
pub(super) struct WireFormatOption {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

// A `static`, not a `const`: `native_wire_formats` hands out `'static`
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

/// Platforms the gateway forwards through its OpenAI stack. Everything else
/// it serves as Anthropic, which is also the safe way to be wrong about a
/// platform this build has never heard of.
fn openai_compatible_platform(platform: &str) -> bool {
    matches!(
        platform.trim().to_ascii_lowercase().as_str(),
        "openai" | "grok" | "kimi" | "zhipu" | "deepseek" | "minimax" | "opencode_go" | "opencodego"
    )
}

/// Whether one format can carry one model on one platform.
///
/// Both exclusions are facts about code that exists: the gateway has no
/// Responses translator for Gemini groups, and the engine's Chat client
/// refuses the models OpenAI serves over Responses. Messages on an OpenAI
/// group is *not* excluded — whether that group accepts it is a per-group
/// setting the desktop cannot see, so it stays offered.
pub(super) fn native_format_supported(format: &str, platform: &str, model: &str) -> bool {
    let platform = platform.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    match format {
        "responses" => platform != "gemini",
        "chat" => {
            !(model.starts_with("gpt-5") || model.starts_with("o3") || model.starts_with("o4"))
        }
        _ => true,
    }
}

/// The formats offered for one model, in listing order. Never empty:
/// Messages has no exclusions.
pub(super) fn native_wire_formats(platform: &str, model: &str) -> Vec<&'static WireFormatOption> {
    NATIVE_WIRE_FORMATS
        .iter()
        .filter(|format| native_format_supported(format.id, platform, model))
        .collect()
}

/// The route this model's platform is served over, when it can carry the
/// model; otherwise the first format that can.
pub(super) fn native_default_wire_format(platform: &str, model: &str) -> &'static str {
    let native = if openai_compatible_platform(platform) {
        "responses"
    } else {
        "messages"
    };
    if native_format_supported(native, platform, model) {
        return native;
    }
    native_wire_formats(platform, model)
        .first()
        .map_or("messages", |format| format.id)
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

/// What the built-in agent's probe lists: the catalog when the account
/// offers one, the engine's fallback list otherwise. Never empty, so a
/// signed-out picker still has rows and a signed-in one never blanks
/// between a sign-out and the next catalog.
pub(super) fn native_probe_models(items: &[ModelCatalogItem]) -> Vec<ProviderModel> {
    let models = native_models_from_catalog(items);
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
        let models = native_probe_models(&self.model_plaza.items);
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
    fn each_model_carries_the_formats_that_can_carry_it() {
        let tiers = |model: &ProviderModel| -> Vec<String> {
            model
                .service_tiers
                .iter()
                .map(|tier| tier.id.clone())
                .collect()
        };

        // OpenAI serves gpt-5 over Responses; the engine's Chat client
        // refuses it outright, so Chat is not offered.
        let openai = native_models_from_catalog(&[item("gpt-5.6-sol", "openai")]);
        assert_eq!(tiers(&openai[0]), ["messages", "responses"]);
        assert_eq!(openai[0].default_service_tier.as_deref(), Some("responses"));

        // The gateway has no Responses translator for Gemini groups.
        let gemini = native_models_from_catalog(&[item("gemini-3-pro", "gemini")]);
        assert_eq!(tiers(&gemini[0]), ["messages", "chat"]);
        assert_eq!(gemini[0].default_service_tier.as_deref(), Some("messages"));

        // Grok is OpenAI-compatible at the gateway, so Responses is its
        // native route. This is the combination that used to default to
        // Messages and get hijacked to the engine's own xai provider.
        let grok = native_models_from_catalog(&[item("grok-4.6", "grok")]);
        assert_eq!(tiers(&grok[0]), ["messages", "responses", "chat"]);
        assert_eq!(grok[0].default_service_tier.as_deref(), Some("responses"));

        let claude = native_models_from_catalog(&[item("claude-sonnet-5", "anthropic")]);
        assert_eq!(claude[0].default_service_tier.as_deref(), Some("messages"));
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
        assert_eq!(ids(native_probe_models(&[])), fallback);
        // A catalog with nothing a coding agent can drive counts as empty.
        let mut image = item("gpt-image-2", "openai");
        image.billing_mode = "image".into();
        assert_eq!(ids(native_probe_models(&[image])), fallback);
    }

    #[test]
    fn a_catalog_replaces_the_fallback_list_outright() {
        let models = native_probe_models(&[item("gemini-3-pro", "gemini")]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["gemini::gemini-3-pro"]);
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
