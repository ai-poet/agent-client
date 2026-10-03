//! A one-line description for each row of the model picker.
//!
//! The lookup order — family rule, then size/speed tier, then nothing — comes
//! from dsh-claude-style's `model-descriptions.json` (MIT, © Nwflower); the
//! wording is our own. Each rule names a locale key (`model_copy.*` in
//! `locales/*.yml`), so the desktop renders it in the UI language.
//!
//! The copy describes a *product line*, never a version: an older or newer
//! model of the same family reads the same line, so no superlative ("the
//! strongest") can end up on a model it is no longer true of. A family a rule
//! does not know gets the tier line its name implies (`-flash`, `-pro`, …) or
//! no line at all — never a reseller's or a guess.

/// The locale key of `model_id`'s description, if any rule claims it.
///
/// `model_id` may carry a routing prefix (`deepseek::deepseek-v4-flash`,
/// `custom:<provider>::<model>`, `openrouter/anthropic/claude-…`); only the
/// last segment names the model. `name` is the catalog's display name, tried
/// when the id itself says nothing (a user endpoint's opaque id).
pub fn description_key(model_id: &str, name: Option<&str>) -> Option<&'static str> {
    let id = normalize(model_id);
    family_key(&id)
        .or_else(|| name.map(normalize).as_deref().and_then(family_key))
        .or_else(|| tier_key(&id))
}

fn normalize(raw: &str) -> String {
    let after_route = raw.rsplit("::").next().unwrap_or(raw);
    let last = after_route.rsplit('/').next().unwrap_or(after_route);
    last.trim().to_ascii_lowercase()
}

/// The id split on separators: `gpt-5.6-sol` → `gpt`, `5`, `6`, `sol`.
fn segments(id: &str) -> impl Iterator<Item = &str> {
    id.split(|c: char| matches!(c, '-' | '_' | '.' | ' ' | ':' | '@'))
        .filter(|segment| !segment.is_empty())
}

/// Whether a segment is `word`, or `word` followed by a version or size
/// (`flash2`, `max3`): `mini` must not match `minimax` or `gemini`.
fn has_word(id: &str, word: &str) -> bool {
    segments(id).any(|segment| {
        segment.strip_prefix(word).is_some_and(|rest| {
            rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit() || c == 'x')
        })
    })
}

fn has_any_word(id: &str, words: &[&str]) -> bool {
    words.iter().any(|word| has_word(id, word))
}

/// `o1`, `o3-mini`, `o4`: OpenAI's o-series, which has no vendor word.
fn is_o_series(id: &str) -> bool {
    segments(id).next().is_some_and(|first| {
        first.len() >= 2
            && first.starts_with('o')
            && first[1..].chars().all(|c| c.is_ascii_digit())
    })
}

/// A size spelled in billions as its own segment: `qwen3-32b`, `llama-70b`.
fn is_sized(id: &str) -> bool {
    segments(id).any(|segment| {
        segment
            .strip_suffix('b')
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
    })
}

/// A mixture-of-experts id spelling its active parameters: `235b-a22b`.
fn is_moe(id: &str) -> bool {
    segments(id).any(|segment| {
        segment.strip_prefix('a').is_some_and(|rest| {
            rest.strip_suffix('b')
                .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        })
    })
}

fn family_key(id: &str) -> Option<&'static str> {
    // Image models first: `gpt-image-2` is not a chat GPT, `grok-imagine`
    // not a chat Grok.
    if has_word(id, "image")
        || id.contains("imagine")
        || id.contains("nano-banana")
        || id.contains("nanobanana")
        || id.contains("dall-e")
    {
        return Some("model_copy.image");
    }
    if id.contains("deepseek") {
        return Some(if has_any_word(id, &["flash", "chat"]) {
            "model_copy.deepseek_flash"
        } else if has_any_word(id, &["pro", "reasoner"]) {
            "model_copy.deepseek_pro"
        } else {
            "model_copy.deepseek"
        });
    }
    if id.contains("claude") {
        return Some(if id.contains("fable") {
            "model_copy.claude_fable"
        } else if id.contains("opus") {
            "model_copy.claude_opus"
        } else if id.contains("sonnet") {
            "model_copy.claude_sonnet"
        } else if id.contains("haiku") {
            "model_copy.claude_haiku"
        } else {
            "model_copy.claude"
        });
    }
    if id.contains("codex") {
        return Some("model_copy.gpt_codex");
    }
    if id.starts_with("gpt") {
        // GPT-5 and later reason before answering, with an effort setting.
        let generation = segments(id)
            .skip_while(|segment| *segment != "gpt")
            .nth(1)
            .or_else(|| id.strip_prefix("gpt").filter(|rest| !rest.starts_with('-')))
            .and_then(|segment| segment.chars().next())
            .and_then(|c| c.to_digit(10));
        if id.contains("oss") {
            return Some("model_copy.gpt_oss");
        }
        return Some(if generation.is_some_and(|major| major >= 5) {
            "model_copy.gpt_reasoning"
        } else {
            "model_copy.gpt"
        });
    }
    if is_o_series(id) {
        return Some("model_copy.openai_o");
    }
    if id.contains("gemma") {
        return Some("model_copy.gemma");
    }
    if id.contains("gemini") {
        return Some(if has_any_word(id, &["pro", "ultra"]) {
            "model_copy.gemini_pro"
        } else if has_any_word(id, &["flash", "lite"]) {
            "model_copy.gemini_flash"
        } else {
            "model_copy.gemini"
        });
    }
    if id.contains("glm") {
        return Some(if has_any_word(id, &["flash", "flashx", "air"]) {
            "model_copy.glm_flash"
        } else {
            "model_copy.glm"
        });
    }
    if id.contains("kimi") || has_word(id, "k3") {
        return Some("model_copy.kimi");
    }
    if id.contains("qwen") {
        return Some(if has_any_word(id, &["coder", "code"]) {
            "model_copy.qwen_coder"
        } else if has_word(id, "max") {
            "model_copy.qwen_max"
        } else {
            "model_copy.qwen"
        });
    }
    if id.contains("minimax") {
        return Some("model_copy.minimax");
    }
    if id.contains("grok") {
        return Some(if has_word(id, "code") {
            "model_copy.grok_code"
        } else {
            "model_copy.grok"
        });
    }
    if id.contains("doubao") || has_word(id, "seed") {
        return Some("model_copy.doubao");
    }
    if id.contains("ernie") {
        return Some("model_copy.ernie");
    }
    if id.contains("hunyuan") {
        return Some("model_copy.hunyuan");
    }
    if id.contains("codestral") {
        return Some("model_copy.codestral");
    }
    if id.contains("mistral") || id.contains("magistral") || id.contains("devstral") {
        return Some("model_copy.mistral");
    }
    if id.contains("llama") {
        return Some("model_copy.llama");
    }
    None
}

fn tier_key(id: &str) -> Option<&'static str> {
    if has_any_word(
        id,
        &[
            "flash", "mini", "lite", "nano", "turbo", "air", "small", "fast", "instant",
            "highspeed",
        ],
    ) {
        return Some("model_copy.tier_fast");
    }
    if has_any_word(
        id,
        &[
            "pro", "max", "ultra", "plus", "large", "thinking", "reasoner", "heavy",
        ],
    ) {
        return Some("model_copy.tier_large");
    }
    // The MoE spelling is tested first: `235b-a22b` also has a sized segment.
    if is_moe(id) {
        return Some("model_copy.tier_moe");
    }
    if is_sized(id) {
        return Some("model_copy.tier_small");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: &str) -> Option<&'static str> {
        description_key(id, None)
    }

    #[test]
    fn families_in_our_catalogs_have_a_line() {
        let cases = [
            ("claude-fable-5-1", "model_copy.claude_fable"),
            ("claude-opus-5-5", "model_copy.claude_opus"),
            ("claude-sonnet-5-5", "model_copy.claude_sonnet"),
            ("claude-haiku-4-5-20251001", "model_copy.claude_haiku"),
            ("gpt-5.6-sol", "model_copy.gpt_reasoning"),
            ("gpt-6.1-sol", "model_copy.gpt_reasoning"),
            ("gpt-6-astra", "model_copy.gpt_reasoning"),
            ("gpt-5.3-codex", "model_copy.gpt_codex"),
            ("gpt-4o-mini", "model_copy.gpt"),
            ("gpt-oss-120b", "model_copy.gpt_oss"),
            ("o3", "model_copy.openai_o"),
            ("o4-mini", "model_copy.openai_o"),
            ("deepseek-v4-flash", "model_copy.deepseek_flash"),
            ("deepseek-v4.1-flash", "model_copy.deepseek_flash"),
            ("deepseek-v4-pro", "model_copy.deepseek_pro"),
            ("deepseek-v4", "model_copy.deepseek"),
            ("glm-5.3", "model_copy.glm"),
            ("glm-5.3-flashx", "model_copy.glm_flash"),
            ("kimi-k3", "model_copy.kimi"),
            ("kimi-k2.6", "model_copy.kimi"),
            ("minimax-m3", "model_copy.minimax"),
            ("qwen3-coder-plus", "model_copy.qwen_coder"),
            ("qwen3.8-max", "model_copy.qwen_max"),
            ("qwen3-235b-a22b", "model_copy.qwen"),
            ("grok-4.7", "model_copy.grok"),
            ("grok-code-fast", "model_copy.grok_code"),
            ("gemini-3-pro", "model_copy.gemini_pro"),
            ("gemini-3.8-flash", "model_copy.gemini_flash"),
            ("gemma-4-27b", "model_copy.gemma"),
            ("gpt-image-2", "model_copy.image"),
            ("grok-imagine-image", "model_copy.image"),
        ];
        for (id, expected) in cases {
            assert_eq!(key(id), Some(expected), "{id}");
        }
    }

    #[test]
    fn routing_prefixes_and_case_are_ignored() {
        assert_eq!(
            key("deepseek::deepseek-v4-flash"),
            Some("model_copy.deepseek_flash")
        );
        assert_eq!(
            key("custom:my-endpoint::Claude-Opus-5-5"),
            Some("model_copy.claude_opus")
        );
        assert_eq!(
            key("openrouter/anthropic/claude-sonnet-5"),
            Some("model_copy.claude_sonnet")
        );
    }

    #[test]
    fn tier_words_match_whole_segments_only() {
        // `mini` inside `gemini` or `minimax` is not the mini tier.
        assert_eq!(key("some-gemini-like"), Some("model_copy.gemini"));
        assert_eq!(key("acme-mini"), Some("model_copy.tier_fast"));
        assert_eq!(key("acme-flash2"), Some("model_copy.tier_fast"));
        assert_eq!(key("acme-max"), Some("model_copy.tier_large"));
        assert_eq!(key("acme-maxwell"), None);
    }

    #[test]
    fn sizes_read_as_dense_or_mixture_of_experts() {
        assert_eq!(key("acme-235b-a22b"), Some("model_copy.tier_moe"));
        assert_eq!(key("acme-32b"), Some("model_copy.tier_small"));
        assert_eq!(key("acme-b"), None);
    }

    #[test]
    fn the_display_name_answers_for_an_opaque_id() {
        assert_eq!(
            description_key("ep-20261004-abc", Some("Claude Sonnet 5.5")),
            Some("model_copy.claude_sonnet")
        );
        assert_eq!(description_key("ep-20261004-abc", Some("My model")), None);
    }

    #[test]
    fn unknown_models_get_no_line() {
        assert_eq!(key("acme-7"), None);
        assert_eq!(key(""), None);
    }
}
