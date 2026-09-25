//! The built-in agent's model catalog, taken from the signed-in account.
//!
//! Every CLI provider discovers its models by being asked; the daemon runs
//! `codex app-server` or `claude --list-models` and reads the answer. The
//! built-in agent has no command to ask, and what it can actually reach is
//! decided by the account it routes through — so its catalog is the gateway
//! listing the Model Plaza page shows, every platform of it, and lands in the
//! provider probe the picker already reads.
//!
//! The id carries the model's family ahead of a `::` (`deepseek::…`), one
//! row per model however many groups serve it. The key it goes out with is
//! not the id's business: the routing writer files one per model, from the
//! group [`sub2api::model_routing`] picked for it, and the family only
//! decides the fallback. The picker never shows an id, only the name and the
//! brand-and-platform subtitle — which also names the subscription a model
//! goes through, when it goes through one.
//!
//! The wire format — Anthropic Messages, OpenAI Responses, OpenAI Chat
//! Completions — is a property of the model, and each model has exactly one.
//! Claude is served over Messages; the GPT and Grok families over Responses;
//! DeepSeek, Kimi, GLM and MiniMax over Chat Completions, which is also where
//! the models a user declared on their own endpoint go. A model belonging to
//! none of those is not offered at all — there is no API to send it over, so
//! listing it would only promise something that fails.
//!
//! That one format lands in the model's "service tier" slot, which is what
//! the composer's traits menu names. The picker does not file the list by
//! it: people look for DeepSeek, not for Chat Completions, so its column
//! groups the models by vendor ([`native_vendor_of`]) and the format simply
//! rides along with whichever model is picked.
//!
//! Pure mapping plus the hooks that apply it; the fetch is the Plaza's.

use sub2api::client::ModelCatalogItem;
use sub2api::providers::ModelEntry;

use super::*;
// Explicit rather than relying on the glob: `ProviderModelOption` is used by
// no other view, so nothing guarantees `app.rs` re-exports it.
use crate::model::{ProviderModel, ProviderModelOption};

/// Where the built-in agent's models are sent, for labelling them: the
/// group model routing picked for each (`Credentials::model_routes`), the
/// pay-as-you-go group of each model a subscription also serves
/// (`Credentials::payg_routes`), and the account's subscription groups, by
/// id, with their names — plus which of them have run out.
#[derive(Clone, Debug, Default)]
pub(super) struct NativeRouting {
    pub routes: std::collections::BTreeMap<String, i64>,
    pub payg: std::collections::BTreeMap<String, i64>,
    pub subscriptions: std::collections::BTreeMap<i64, String>,
    pub exhausted: std::collections::BTreeSet<i64>,
}

impl NativeRouting {
    /// What a model's plain row says about the subscription behind it, if
    /// one is: the one it goes through, that one having run out, or — when a
    /// spent subscription has handed it to pay-as-you-go — that.
    ///
    /// `groups` are every group the catalog lists the model under.
    fn subscription_note(&self, model: &str, groups: &[i64]) -> Option<String> {
        let routed = self.routes.get(model).copied();
        if let Some(group) = routed
            && let Some(name) = self.subscriptions.get(&group)
        {
            return Some(if self.exhausted.contains(&group) {
                tr!("native.subscription_spent", group = name.clone())
            } else {
                tr!("native.via_subscription", group = name.clone())
            });
        }
        groups
            .iter()
            .find(|group| self.exhausted.contains(*group))
            .and_then(|group| self.subscriptions.get(group))
            .map(|name| tr!("native.subscription_spent_payg", group = name.clone()))
    }
}

/// What the picker appends to the platform of a model's "pay as you go" row:
/// `deepseek+payg::deepseek-v4.1-flash`. Kept in step with
/// `waku_agent_bridge::PAY_AS_YOU_GO_MARK`, which the desktop does not link;
/// the bridge strips it and sends that row with its pay-as-you-go key.
pub(super) const PAY_AS_YOU_GO_MARK: &str = "+payg";

/// A picker id taken apart: the platform ahead of the `::` (lowercased, no
/// [`PAY_AS_YOU_GO_MARK`]), the model after it, and whether it is a "pay as
/// you go" row. A bare id carries no platform — that is what a model the
/// user declared on their own endpoint looks like.
pub(super) fn native_route_parts(id: &str) -> (String, &str, bool) {
    match id.split_once("::") {
        Some((platform, model)) => {
            let (platform, pay_as_you_go) = match platform.strip_suffix(PAY_AS_YOU_GO_MARK) {
                Some(platform) => (platform, true),
                None => (platform, false),
            };
            (platform.trim().to_ascii_lowercase(), model, pay_as_you_go)
        }
        None => (String::new(), id, false),
    }
}

/// [`native_models_routed`] with nothing routed, as the tests describe the
/// catalog.
#[cfg(test)]
pub(super) fn native_models_from_catalog(items: &[ModelCatalogItem]) -> Vec<ProviderModel> {
    native_models_routed(items, &NativeRouting::default())
}

/// The models the built-in agent may offer, from the gateway catalog.
///
/// Token-billed models on every platform qualify; image and per-request
/// products are not something a coding agent can drive. Anthropic and OpenAI
/// families get the reasoning ladder the engine understands, with
/// `ultracode` on the ones that accept `xhigh`. The first Sonnet 5 is the
/// default, since it is the engine's own default family.
///
/// The catalog lists a model once per group that serves it; the picker
/// lists it once — twice when a subscription and a pay-as-you-go group both
/// serve it, the second row pinned to pay-as-you-go. The plain row comes
/// from the entry of the group routing sends it through, when routing has
/// picked one, so its name and platform are that group's.
pub(super) fn native_models_routed(
    items: &[ModelCatalogItem],
    routing: &NativeRouting,
) -> Vec<ProviderModel> {
    let mut order: Vec<String> = Vec::new();
    let mut offered: std::collections::HashMap<String, Vec<&ModelCatalogItem>> =
        std::collections::HashMap::new();
    for item in items
        .iter()
        .filter(|item| is_chat_model(item) && !item.model.trim().is_empty())
    {
        let key = item.model.trim().to_ascii_lowercase();
        let entries = offered.entry(key.clone()).or_default();
        if entries.is_empty() {
            order.push(key);
        }
        entries.push(item);
    }
    let mut models: Vec<ProviderModel> = Vec::new();
    for key in &order {
        let entries = &offered[key];
        let model_id = entries[0].model.trim();
        let through = |group: Option<&i64>| {
            group.and_then(|group| {
                entries
                    .iter()
                    .copied()
                    .find(|item| item.best_group.id == *group)
            })
        };
        let item = through(routing.routes.get(model_id)).unwrap_or(entries[0]);
        let platform = native_platform(item);
        let groups: Vec<i64> = entries.iter().map(|item| item.best_group.id).collect();
        let note = routing.subscription_note(model_id, &groups);
        // No API to send it over means it is not a choice, however well it
        // reads in a catalog.
        let Some(row) = native_row(item, &platform, format!("{platform}::{model_id}"), note) else {
            continue;
        };
        models.push(row);
        if let Some(payg) = through(routing.payg.get(model_id)) {
            let platform = native_platform(payg);
            models.extend(native_row(
                payg,
                &platform,
                format!("{platform}{PAY_AS_YOU_GO_MARK}::{model_id}"),
                Some(tr!("native.pay_as_you_go")),
            ));
        }
    }

    if let Some(default) = models
        .iter()
        .position(|model| model.id.contains("sonnet-5"))
        .or_else(|| (!models.is_empty()).then_some(0))
    {
        models[default].is_default = true;
    }
    models
}

/// One picker row for a catalog entry: `id` as the picker sends it, the
/// subtitle the platform plus `note`, the one API the model is reachable
/// over and its reasoning ladder. `None` when there is no API to send it
/// over.
fn native_row(
    item: &ModelCatalogItem,
    platform: &str,
    id: String,
    note: Option<String>,
) -> Option<ProviderModel> {
    let model_id = item.model.trim();
    let format = native_format_for_model(platform, model_id)?;
    let name = if item.display_name.trim().is_empty() {
        model_id.to_owned()
    } else {
        item.display_name.clone()
    };
    let mut model = ProviderModel::new(id, name);
    model.sub_provider = Some(match note {
        Some(note) => format!("{platform} \u{00b7} {note}"),
        None => platform.to_owned(),
    });
    if let Some(entry) = native_format_option(format) {
        model = model.service_tiers(
            [ProviderModelOption::new(entry.id, crate::i18n::translate(entry.label))
                .description(crate::i18n::translate(entry.description))],
            entry.id,
        );
    }
    if let Some(ladder) = reasoning_ladder(platform, model_id) {
        model = model.reasoning(
            ladder
                .into_iter()
                .map(|effort| ProviderModelOption::new(effort, reasoning_effort_label(effort))),
            "high",
        );
    }
    Some(model)
}

/// The platform a catalog model is filed under in the picker: its family,
/// read from the name the way the gateway reads it, or — for a name that
/// gives nothing away — the platform of the group that listed it.
fn native_platform(item: &ModelCatalogItem) -> String {
    sub2api::model_routing::model_family(&item.model)
        .map(str::to_owned)
        .unwrap_or_else(|| platform_of(item))
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

/// A reasoning ladder wherever the engine maps effort onto a request field
/// the upstream understands.
///
/// That is every family the picker offers over Messages and Responses
/// (DeepSeek's, over Chat Completions, is [`reasoning_ladder`]'s): Claude
/// over Messages turns it into a thinking budget, and the GPT and Grok
/// families over Responses turn it into `reasoning.effort`. Grok was the exception until the engine
/// stopped leaving it out of that second list; the gateway normalizes the
/// value per model and drops it for the ones that cannot use it, so the
/// ladder is honest for all of them.
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
        || model.starts_with("grok")
    {
        return true;
    }
    platform == "anthropic" || platform == "openai" || platform == "grok"
}

/// The efforts one model is offered, or `None` for a model without a
/// reasoning choice.
///
/// DeepSeek, GLM from 4.5 on and Kimi's K3 family share one ladder of three
/// (`sub2api::model_routing::has_three_step_effort`): `low`, `high`, `max`.
/// The engine sends each in its API's own shape on the Chat Completions
/// request — DeepSeek and GLM as a `thinking` switch plus `reasoning_effort`,
/// `low` being thinking off; K3 as `reasoning_effort` alone, since it always
/// thinks — and the gateway forwards them. Kimi's K2 line and MiniMax have
/// no depth to choose, so they get no ladder.
fn reasoning_ladder(platform: &str, model: &str) -> Option<Vec<&'static str>> {
    if sub2api::model_routing::has_three_step_effort(model) {
        return Some(vec!["low", "high", "max"]);
    }
    if !has_reasoning_ladder(platform, model) {
        return None;
    }
    let mut ladder = vec!["low", "medium", "high", "xhigh", "max"];
    if supports_ultracode(model) {
        ladder.push("ultracode");
    }
    Some(ladder)
}

/// The engine resolves `ultracode` to its top reasoning budget, which only
/// the newest families accept; older ones clamp it back to `high` and the
/// entry would be inert.
fn supports_ultracode(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("opus-5") || model.contains("sonnet-5") || model.contains("fable")
}

/// One wire format, as the traits menu names it.
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
/// Chat Completions carries DeepSeek, Kimi, GLM and MiniMax: the gateway
/// serves them from accounts that speak it and forwards it as it is. The
/// models a user declared on their own endpoint ([`native_custom_models`])
/// land there too, by another road.
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
    if matches!(
        sub2api::model_routing::model_family(&model),
        Some("deepseek" | "kimi" | "zhipu" | "minimax")
    ) {
        return Some("chat");
    }
    // A name that says nothing: fall back to the group's platform, for the
    // rare model whose id carries no family at all.
    match platform.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Some("messages"),
        "openai" | "grok" => Some("responses"),
        "deepseek" | "kimi" | "zhipu" | "minimax" | "opencode_go" | "composite" => Some("chat"),
        _ => None,
    }
}

/// The format entry for one id, for labelling.
pub(super) fn native_format_option(format: &str) -> Option<&'static WireFormatOption> {
    NATIVE_WIRE_FORMATS.iter().find(|entry| entry.id == format)
}

/// One entry of the vendor column the picker draws beside the built-in
/// agent's list.
pub(super) struct NativeVendor {
    pub id: &'static str,
    /// Locale key of the name the column shows.
    pub label: &'static str,
    pub icon: &'static str,
    /// The vendor's brand hue, or `None` for a mark drawn in the theme's ink.
    pub color: Option<u32>,
}

const OTHER_VENDOR: &str = "other";
/// Where the models a user declared on their own endpoint are filed.
pub(super) const CUSTOM_VENDOR: &str = "custom";

/// The vendors in the order the column lists them. Ids are the families
/// `sub2api::model_routing::model_family` reads off a name, plus Qwen, which
/// only ever arrives through a composite group, and the two catch-alls.
/// `custom` stays last: it is always drawn, as the place to find out where
/// models on the user's own endpoint go.
pub(super) static NATIVE_VENDORS: [NativeVendor; 11] = [
    NativeVendor {
        id: "anthropic",
        label: "native.vendor.anthropic",
        icon: "icons/provider-claude.svg",
        color: Some(0xD97757),
    },
    NativeVendor {
        id: "openai",
        label: "native.vendor.openai",
        icon: "icons/provider-openai.svg",
        color: None,
    },
    NativeVendor {
        id: "gemini",
        label: "native.vendor.gemini",
        icon: "icons/provider-gemini.svg",
        color: Some(0x4285F4),
    },
    NativeVendor {
        id: "grok",
        label: "native.vendor.grok",
        icon: "icons/provider-grok.svg",
        color: None,
    },
    NativeVendor {
        id: "deepseek",
        label: "native.vendor.deepseek",
        icon: "icons/provider-deepseek.svg",
        color: Some(0x4D6BFE),
    },
    NativeVendor {
        id: "zhipu",
        label: "native.vendor.zhipu",
        icon: "icons/provider-zhipu.svg",
        color: Some(0x3859FF),
    },
    NativeVendor {
        id: "kimi",
        label: "native.vendor.kimi",
        icon: "icons/provider-kimi.svg",
        color: None,
    },
    NativeVendor {
        id: "minimax",
        label: "native.vendor.minimax",
        icon: "icons/provider-minimax.svg",
        color: Some(0xF23F5D),
    },
    NativeVendor {
        id: "qwen",
        label: "native.vendor.qwen",
        icon: "icons/provider-qwen.svg",
        color: Some(0x615EFF),
    },
    NativeVendor {
        id: OTHER_VENDOR,
        label: "native.vendor.other",
        icon: "icons/sparkle.svg",
        color: None,
    },
    NativeVendor {
        id: CUSTOM_VENDOR,
        label: "native.vendor.custom",
        icon: "icons/server.svg",
        color: None,
    },
];

/// The column entry for one vendor id; an id the table does not know is
/// filed under "other".
pub(super) fn native_vendor(id: &str) -> &'static NativeVendor {
    NATIVE_VENDORS
        .iter()
        .find(|vendor| vendor.id == id)
        .or_else(|| NATIVE_VENDORS.iter().find(|vendor| vendor.id == OTHER_VENDOR))
        .expect("the vendor table lists `other`")
}

/// Which vendor the picker files one of the built-in agent's models under.
///
/// A model on the user's own endpoint is theirs whatever it is called — its
/// route and key are the user's, not the vendor's — so it goes under
/// `custom`. Otherwise the name decides, the way it decides the API, and the
/// platform ahead of the `::` only for a name that gives nothing away: a
/// composite group reports `composite` for everything in it.
pub(super) fn native_vendor_of(model: &ProviderModel) -> &'static str {
    if !model.id.contains("::") {
        if model.sub_provider.as_deref() == Some("custom") {
            return CUSTOM_VENDOR;
        }
        return vendor_by_name(&model.id).unwrap_or(OTHER_VENDOR);
    }
    let (platform, name, _) = native_route_parts(&model.id);
    if let Some(vendor) = vendor_by_name(name) {
        return vendor;
    }
    NATIVE_VENDORS
        .iter()
        .map(|vendor| vendor.id)
        .find(|id| *id == platform && *id != CUSTOM_VENDOR)
        .unwrap_or(OTHER_VENDOR)
}

fn vendor_by_name(model: &str) -> Option<&'static str> {
    if let Some(family) = sub2api::model_routing::model_family(model) {
        return Some(family);
    }
    let name = model.trim().to_ascii_lowercase();
    let name = name.rsplit('/').next().unwrap_or(&name);
    (name.starts_with("qwen") || name.starts_with("qwq")).then_some("qwen")
}

/// The vendors the column draws for a list, in table order, each with how
/// many models it holds. A vendor with none is left out, except `custom`,
/// which is always there to say where the user's own models go.
pub(super) fn native_vendors_present(
    models: &[ProviderModel],
) -> Vec<(&'static NativeVendor, usize)> {
    let mut counts = vec![0usize; NATIVE_VENDORS.len()];
    for model in models {
        let id = native_vendor_of(model);
        if let Some(index) = NATIVE_VENDORS.iter().position(|vendor| vendor.id == id) {
            counts[index] += 1;
        }
    }
    NATIVE_VENDORS
        .iter()
        .zip(counts)
        .filter(|(vendor, count)| *count > 0 || vendor.id == CUSTOM_VENDOR)
        .collect()
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
pub(super) fn native_custom_models(models: &[ModelEntry]) -> Vec<ProviderModel> {
    let mut seen = std::collections::HashSet::new();
    models
        .iter()
        .filter(|model| !model.id.trim().is_empty())
        .filter(|model| seen.insert(model.id.trim().to_string()))
        .map(|model| {
            let id = model.id.trim();
            let mut entry = ProviderModel::new(id, model.display_name());
            entry.sub_provider = Some("custom".to_owned());
            let entry = entry.service_tiers(
                [ProviderModelOption::new("chat", crate::i18n::translate("model_option.wire_chat"))
                    .description(crate::i18n::translate("model_option.wire_chat_description"))],
                "chat",
            );
            // The tiers the user declared, in the order they wrote them. An
            // endpoint that does not reason declares none, and the traits
            // menu then offers no ladder rather than one that is refused.
            if model.reasoning_efforts.is_empty() {
                return entry;
            }
            let default = model
                .default_reasoning_effort()
                .unwrap_or(&model.reasoning_efforts[0])
                .to_owned();
            entry.reasoning(
                model.reasoning_efforts.iter().map(|effort| {
                    ProviderModelOption::new(effort.clone(), reasoning_effort_label(effort))
                }),
                default,
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
    routing: &NativeRouting,
    custom: &[ModelEntry],
) -> Vec<ProviderModel> {
    let mut models = native_models_routed(items, routing);
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
        // Through the binding, so the models the picker offers are the ones
        // on the endpoint that actually routes — not a copy a slot happens
        // to still carry.
        let stored = self.custom_api_snapshot();
        let custom = stored
            .bound_provider("native_chat")
            .filter(|entry| entry.is_routable())
            .map(|entry| entry.models.clone())
            .unwrap_or_default();
        let models = native_probe_models(&self.model_plaza.items, &self.native_routing(), &custom);
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

    /// Every model the agent has an API for is offered, with the platform in
    /// its id — and a model it has none for is left out rather than listed
    /// and then failing on the wire. The rule moved here when the wire
    /// format became a property of the model family; this test was written
    /// before that and expected Gemini to be listed.
    #[test]
    fn a_model_is_offered_only_when_there_is_an_api_to_send_it_over() {
        let models = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("gpt-5.6-sol", "openai"),
            // Neither Messages nor Responses nor the user's own Chat
            // Completions list: this product configures no Google route.
            item("gemini-3-pro", "gemini"),
        ]);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["anthropic::claude-sonnet-5", "openai::gpt-5.6-sol"]);
        assert_eq!(models[0].sub_provider.as_deref(), Some("anthropic"));
        assert_eq!(models[1].sub_provider.as_deref(), Some("openai"));
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
    fn the_chat_section_holds_the_chat_families_and_the_users_own_models() {
        // Of the managed catalog, only the families the gateway serves over
        // Chat Completions land there.
        let catalog = native_models_from_catalog(&[
            item("claude-sonnet-5", "anthropic"),
            item("gpt-5.6-sol", "openai"),
            item("grok-4.6", "grok"),
            item("deepseek-v4.1-flash", "deepseek"),
        ]);
        let chat: Vec<&str> = catalog
            .iter()
            .filter(|model| model.default_service_tier.as_deref() == Some("chat"))
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(chat, ["deepseek::deepseek-v4.1-flash"]);

        let declared = native_custom_models(&[
            ModelEntry::new("my-model"),
            ModelEntry::new(" "),
            ModelEntry::new("my-model"),
        ]);
        let ids: Vec<&str> = declared.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["my-model"], "blank and duplicate entries are dropped");
        assert_eq!(declared[0].default_service_tier.as_deref(), Some("chat"));
        // A bare id, with no platform ahead of a `::`: nothing on the
        // gateway claims it.
        assert!(!declared[0].id.contains("::"));
        // Nothing declared, so no ladder is offered — one whose tiers the
        // endpoint refuses is worse than none.
        assert!(declared[0].reasoning_efforts.is_empty());
        assert_eq!(declared[0].name, "my-model");
    }

    /// What the user typed about their own models is the only thing anything
    /// knows about them, so it has to reach the picker intact.
    #[test]
    fn a_declared_name_and_reasoning_ladder_reach_the_picker() {
        let declared = native_custom_models(&[ModelEntry {
            name: "My relay's Sonnet".to_owned(),
            reasoning_efforts: vec!["low".to_owned(), "high".to_owned()],
            default_reasoning: Some("high".to_owned()),
            ..ModelEntry::new("relay-sonnet")
        }]);
        assert_eq!(declared[0].id, "relay-sonnet");
        assert_eq!(declared[0].name, "My relay's Sonnet");
        let tiers: Vec<&str> = declared[0]
            .reasoning_efforts
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        assert_eq!(tiers, ["low", "high"]);
        assert_eq!(declared[0].default_reasoning_effort.as_deref(), Some("high"));

        // A default naming a tier that is not offered falls back to the
        // first, rather than starting the session on something refused.
        let stray = native_custom_models(&[ModelEntry {
            reasoning_efforts: vec!["low".to_owned()],
            default_reasoning: Some("max".to_owned()),
            ..ModelEntry::new("relay-sonnet")
        }]);
        assert_eq!(stray[0].default_reasoning_effort.as_deref(), Some("low"));
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
        assert_eq!(ids(native_probe_models(&[], &NativeRouting::default(), &[])), fallback);
        // A catalog with nothing a coding agent can drive counts as empty.
        let mut image = item("gpt-image-2", "openai");
        image.billing_mode = "image".into();
        assert_eq!(ids(native_probe_models(&[image], &NativeRouting::default(), &[])), fallback);
    }

    #[test]
    fn a_catalog_replaces_the_fallback_list_outright() {
        let models = native_probe_models(
            &[item("claude-sonnet-5", "anthropic")],
            &NativeRouting::default(),
            &[],
        );
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["anthropic::claude-sonnet-5"]);
    }

    #[test]
    fn the_users_own_models_are_enough_to_replace_the_fallback() {
        // Signed out of the managed service but pointed at an endpoint of
        // their own: the picker lists what they declared, not the built-in
        // Anthropic list they cannot reach.
        let models = native_probe_models(&[], &NativeRouting::default(), &[ModelEntry::new("my-model")]);
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

    /// The report that started per-model routing: the Codex group also
    /// listed a DeepSeek model, so it was offered as `openai::…` and sent
    /// with the Codex key. Filed by family, it is one `deepseek::` row built
    /// from the group routing picked — a subscription, which the subtitle
    /// names.
    #[test]
    fn a_model_is_filed_by_its_family_and_names_its_subscription() {
        let mut codex = item("deepseek-v4.1-flash", "openai");
        codex.best_group.id = 7;
        let mut subscription = item("deepseek-v4.1-flash", "composite");
        subscription.best_group.id = 20;
        subscription.display_name = "DeepSeek V4.1 Flash".into();
        let routing = NativeRouting {
            routes: std::collections::BTreeMap::from([("deepseek-v4.1-flash".to_owned(), 20)]),
            subscriptions: std::collections::BTreeMap::from([(20, "DeepSeek 包月".to_owned())]),
            ..NativeRouting::default()
        };
        let models = native_models_routed(&[codex, subscription], &routing);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "deepseek::deepseek-v4.1-flash");
        assert_eq!(models[0].name, "DeepSeek V4.1 Flash");
        assert_eq!(models[0].default_service_tier.as_deref(), Some("chat"));
        let subtitle = models[0].sub_provider.as_deref().unwrap_or_default();
        assert!(subtitle.starts_with("deepseek"), "{subtitle}");
        assert!(subtitle.contains("DeepSeek 包月"), "{subtitle}");

        // Without a subscription behind it, the subtitle is the family alone.
        let models = native_models_from_catalog(&[item("glm-5", "composite")]);
        assert_eq!(models[0].id, "zhipu::glm-5");
        assert_eq!(models[0].sub_provider.as_deref(), Some("zhipu"));
    }

    /// The live shape: a DeepSeek subscription and the "国模按量付费分组"
    /// (an `openai` group) both serve the model. The picker lists it twice —
    /// the subscription row, and one pinned to pay-as-you-go — and once the
    /// subscription runs out the plain row says so.
    #[test]
    fn a_model_a_subscription_and_pay_as_you_go_both_serve_is_listed_twice() {
        let mut subscription = item("deepseek-v4.1-flash", "composite");
        subscription.best_group.id = 20;
        let mut payg = item("deepseek-v4.1-flash", "openai");
        payg.best_group.id = 40;
        let mut routing = NativeRouting {
            routes: std::collections::BTreeMap::from([("deepseek-v4.1-flash".to_owned(), 20)]),
            payg: std::collections::BTreeMap::from([("deepseek-v4.1-flash".to_owned(), 40)]),
            subscriptions: std::collections::BTreeMap::from([(20, "DeepSeek 包月".to_owned())]),
            ..NativeRouting::default()
        };
        let catalog = [subscription, payg];

        let models = native_models_routed(&catalog, &routing);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            ["deepseek::deepseek-v4.1-flash", "deepseek+payg::deepseek-v4.1-flash"]
        );
        assert!(models[0].sub_provider.as_deref().unwrap_or_default().contains("DeepSeek 包月"));
        let payg_subtitle = models[1].sub_provider.as_deref().unwrap_or_default();
        assert_eq!(payg_subtitle, format!("deepseek \u{00b7} {}", tr!("native.pay_as_you_go")));
        // Same API and ladder as the plain row, and the same vendor.
        assert_eq!(models[1].default_service_tier.as_deref(), Some("chat"));
        assert_eq!(models[1].reasoning_efforts.len(), models[0].reasoning_efforts.len());
        assert_eq!(native_vendor_of(&models[1]), "deepseek");
        assert_eq!(
            native_route_parts(&models[1].id),
            ("deepseek".to_owned(), "deepseek-v4.1-flash", true)
        );

        // Spent: routing moved the plain row to pay-as-you-go, and it says why.
        routing.routes.insert("deepseek-v4.1-flash".to_owned(), 40);
        routing.exhausted.insert(20);
        let models = native_models_routed(&catalog, &routing);
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0].sub_provider.as_deref(),
            Some(
                format!(
                    "deepseek \u{00b7} {}",
                    tr!("native.subscription_spent_payg", group = "DeepSeek 包月".to_owned())
                )
                .as_str()
            )
        );
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
    fn every_offered_family_carries_a_reasoning_ladder() {
        let ladder = |models: &[ProviderModel]| !models[0].reasoning_efforts.is_empty();
        // Grok reaches the same Responses adapter as the GPT family and the
        // gateway accepts an effort for it, so it gets the ladder too.
        assert!(ladder(&native_models_from_catalog(&[item("grok-4.6", "grok")])));
        assert!(ladder(&native_models_from_catalog(&[item("claude-sonnet-5", "anthropic")])));
        // The family carries it through a composite group, where the
        // platform says nothing.
        assert!(ladder(&native_models_from_catalog(&[item("gpt-5.6-sol", "composite")])));
        assert!(ladder(&native_models_from_catalog(&[item("grok-4.6", "composite")])));
    }

    /// DeepSeek, GLM and Kimi K3 share three efforts, as their APIs and the
    /// gateway know them; Kimi K2 and MiniMax have none to offer.
    #[test]
    fn chat_families_offer_three_efforts_where_their_api_has_them() {
        let efforts = |model: &ProviderModel| {
            model
                .reasoning_efforts
                .iter()
                .map(|option| option.id.clone())
                .collect::<Vec<_>>()
        };
        let models = native_models_from_catalog(&[
            item("deepseek-v4.1-flash", "deepseek"),
            item("glm-5", "zhipu"),
            item("kimi-k3", "kimi"),
            item("kimi-k2.6", "kimi"),
            item("minimax-m3", "minimax"),
        ]);
        for model in &models[..3] {
            assert_eq!(efforts(model), ["low", "high", "max"], "{}", model.id);
            assert_eq!(model.default_reasoning_effort.as_deref(), Some("high"));
        }
        assert!(efforts(&models[3]).is_empty());
        assert!(efforts(&models[4]).is_empty());
    }

    #[test]
    fn a_model_is_filed_under_its_vendor() {
        let vendor = |id: &str| native_vendor_of(&ProviderModel::new(id, id));
        assert_eq!(vendor("zhipu::glm-5"), "zhipu");
        // The name beats the group's platform, as it does for the API.
        assert_eq!(vendor("composite::glm-4.6"), "zhipu");
        assert_eq!(vendor("composite::qwen3-coder"), "qwen");
        assert_eq!(vendor("deepseek::deepseek-v4"), "deepseek");
        assert_eq!(vendor("grok::grok-4.6"), "grok");
        assert_eq!(vendor("openai::gemini-3-pro"), "gemini");
        // A name that says nothing falls back to a platform the table knows,
        // and past that to "other".
        assert_eq!(vendor("openai::some-new-model"), "openai");
        assert_eq!(vendor("opencode_go::mystery"), "other");
        // The signed-out fallback list: bare ids, read by name.
        assert_eq!(vendor("claude-sonnet-5"), "anthropic");

        // The user's own endpoint keeps its models whatever they are called.
        let declared = native_custom_models(&[ModelEntry::new("deepseek-chat")]);
        assert_eq!(native_vendor_of(&declared[0]), CUSTOM_VENDOR);
    }

    #[test]
    fn the_vendor_column_lists_vendors_with_models_in_table_order() {
        let mut models = native_models_from_catalog(&[
            item("deepseek-v4.1-flash", "deepseek"),
            item("glm-5", "composite"),
            item("claude-sonnet-5", "anthropic"),
            item("claude-opus-5", "anthropic"),
        ]);
        let column = |models: &[ProviderModel]| -> Vec<(&str, usize)> {
            native_vendors_present(models)
                .into_iter()
                .map(|(vendor, count)| (vendor.id, count))
                .collect()
        };
        assert_eq!(
            column(&models),
            [("anthropic", 2), ("deepseek", 1), ("zhipu", 1), ("custom", 0)]
        );

        // Custom stays last, and counts what the user declared.
        models.extend(native_custom_models(&[ModelEntry::new("my-model")]));
        assert_eq!(column(&models).last(), Some(&("custom", 1)));

        // An id the table does not know reads as "other".
        assert_eq!(native_vendor("nope").id, "other");
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
