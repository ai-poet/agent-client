//! Each model's context window, from a public model list.
//!
//! The footer's context ring needs a window to measure against, and the
//! agents only report theirs once a turn has finished — some never. The
//! gateway catalog carries a window for only part of what it serves. So the
//! desktop looks the model up in OpenRouter's public model list
//! (`/api/v1/models`, no key needed), which covers every family the gateway
//! routes, and keeps a reduced copy under `~/.cheaprouter` for a day.
//!
//! The figure is the ring's fallback only: a window the agent reports for
//! the model it is running is the one it compacts at, and wins.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::brand;
use crate::global_config::atomic_write_private;

/// The public list every lookup comes from.
pub const SOURCE_URL: &str = "https://openrouter.ai/api/v1/models";

/// How long a fetched list is trusted before it is fetched again.
pub const TTL_SECONDS: u64 = 24 * 60 * 60;

/// Below this a listed figure is an output cap or a typo, not a window.
const MIN_WINDOW: u64 = 8_192;

/// Vendors that publish their own models on the list. When one name is
/// listed by several, theirs is the window the model really has.
const FIRST_PARTY: [&str; 11] = [
    "anthropic",
    "openai",
    "google",
    "x-ai",
    "deepseek",
    "z-ai",
    "moonshotai",
    "qwen",
    "minimax",
    "mistralai",
    "meta-llama",
];

/// The reduced list: one window per model name, by [`loose_key`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelWindows {
    /// Unix seconds of the fetch this came from; 0 for none.
    #[serde(default)]
    pub fetched_at: u64,
    #[serde(default)]
    pub windows: BTreeMap<String, u64>,
}

impl ModelWindows {
    /// The window of `model`, however the app spells it.
    pub fn lookup(&self, model: &str) -> Option<u64> {
        self.windows.get(&loose_key(model)?).copied()
    }

    /// Nothing fetched yet, or fetched too long ago.
    pub fn is_stale(&self, now: u64) -> bool {
        self.windows.is_empty() || now.saturating_sub(self.fetched_at) >= TTL_SECONDS
    }
}

/// Where the reduced list is kept between runs.
pub fn cache_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join("model-windows.json"))
}

/// The list kept from the last fetch; empty when there is none.
pub fn load_cached() -> ModelWindows {
    cache_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(windows: &ModelWindows) -> Result<()> {
    let path = cache_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let encoded = serde_json::to_string(windows).context("could not encode model windows")?;
    atomic_write_private(&path, encoded.as_bytes())
}

/// Fetch the public list, keep it on disk, and return it. Blocks; run it off
/// the UI thread.
pub fn refresh() -> Result<ModelWindows> {
    let response = crate::http::Request::new()
        .timeout_seconds(30)
        .send(SOURCE_URL)
        .context("could not fetch the public model list")?;
    if !response.is_success() {
        return Err(anyhow!(
            "the public model list answered HTTP {}",
            response.status
        ));
    }
    let windows = ModelWindows {
        fetched_at: unix_now(),
        windows: parse_openrouter(&response.body)?,
    };
    if windows.windows.is_empty() {
        return Err(anyhow!("the public model list listed no context windows"));
    }
    save(&windows)?;
    Ok(windows)
}

/// Reduce OpenRouter's listing to one window per model name.
///
/// Variants (`:batch`, `:free`, …) are skipped — they are the same model
/// sold differently. Where several vendors list one name, a first-party
/// vendor's figure wins, then the largest.
pub fn parse_openrouter(body: &str) -> Result<BTreeMap<String, u64>> {
    let root: Value = serde_json::from_str(body).context("could not parse the model list")?;
    let entries = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("the model list has no `data` array"))?;
    let mut ranked: BTreeMap<String, (u8, u64)> = BTreeMap::new();
    for entry in entries {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.contains(':') {
            continue;
        }
        let window = [
            entry.get("context_length"),
            entry
                .get("top_provider")
                .and_then(|provider| provider.get("context_length")),
        ]
        .into_iter()
        .flatten()
        .find_map(as_window);
        let (Some(window), Some(key)) = (window, loose_key(id)) else {
            continue;
        };
        let vendor = id.split_once('/').map(|(vendor, _)| vendor).unwrap_or("");
        let rank = u8::from(!FIRST_PARTY.contains(&vendor));
        let better = match ranked.get(&key) {
            None => true,
            Some(&(held_rank, held)) => rank < held_rank || (rank == held_rank && window > held),
        };
        if better {
            ranked.insert(key, (rank, window));
        }
    }
    Ok(ranked
        .into_iter()
        .map(|(key, (_, window))| (key, window))
        .collect())
}

fn as_window(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|number| *number > 0.0).map(|number| number as u64))
        .filter(|window| *window >= MIN_WINDOW)
}

/// One spelling for every way a model is named in the app and on the list:
/// `custom:<id>::glm-5.1`, `anthropic::claude-sonnet-5.5`,
/// `claude-opus-5-5[1m]`, `anthropic/claude-opus-5.5`,
/// `claude-haiku-4-5-20251001` and the picker's `Claude Opus 5.5` all come
/// out as their bare, lowercase name with the version's dots as dashes.
pub fn loose_key(model: &str) -> Option<String> {
    let mut id = model.trim();
    if let Some((_, rest)) = id.rsplit_once("::") {
        id = rest;
    }
    if let Some(cut) = id.find(['[', '(']) {
        id = &id[..cut];
    }
    if let Some((_, rest)) = id.rsplit_once('/') {
        id = rest;
    }
    if let Some((head, _)) = id.split_once(':') {
        id = head;
    }
    let lowered = id.trim().to_ascii_lowercase();
    let words: Vec<&str> = lowered.split_whitespace().collect();
    let mut key = words.join("-");
    if let Some(stripped) = key.strip_suffix("-latest") {
        key = stripped.to_owned();
    }
    key = strip_date_suffix(&key).to_owned();

    let characters: Vec<char> = key.chars().collect();
    let key: String = characters
        .iter()
        .enumerate()
        .map(|(index, &character)| {
            let between_digits = index > 0
                && characters[index - 1].is_ascii_digit()
                && characters
                    .get(index + 1)
                    .is_some_and(char::is_ascii_digit);
            if character == '.' && between_digits {
                '-'
            } else {
                character
            }
        })
        .collect();
    let key = key.trim_matches('-').to_owned();
    (!key.is_empty()).then_some(key)
}

/// `name-20251001` and `name-2025-10-01` are dated snapshots of `name`.
fn strip_date_suffix(key: &str) -> &str {
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if let Some((head, tail)) = key.rsplit_once('-')
        && tail.len() == 8
        && digits(tail)
    {
        return head;
    }
    if key.len() > 11 && key.is_char_boundary(key.len() - 11) {
        let (head, tail) = key.split_at(key.len() - 11);
        let parts: Vec<&str> = tail.split('-').collect();
        if parts.len() == 4
            && parts[0].is_empty()
            && parts[1].len() == 4
            && parts[2].len() == 2
            && parts[3].len() == 2
            && parts[1..].iter().all(|part| digits(part))
        {
            return head;
        }
    }
    key
}

/// A window the user picked rather than one the model has: the session's
/// context option (`200k`, `1m`), or a `[1m]` suffix on the model id.
/// Claude Code runs a 1M-capable model at 200k unless asked.
pub fn explicit_window(option: Option<&str>, model: &str) -> Option<u64> {
    if model.to_ascii_lowercase().contains("[1m]") {
        return Some(1_000_000);
    }
    let option = option?.trim().to_ascii_lowercase();
    let (number, scale) = if let Some(number) = option.strip_suffix('k') {
        (number, 1_000.0)
    } else if let Some(number) = option.strip_suffix('m') {
        (number, 1_000_000.0)
    } else {
        (option.as_str(), 1.0)
    };
    let value = number.trim().parse::<f64>().ok()? * scale;
    (value >= MIN_WINDOW as f64).then(|| value.round() as u64)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"data":[
        {"id":"openai/gpt-5.6-sol","context_length":1050000},
        {"id":"openai/gpt-5.6-sol:batch","context_length":2000000},
        {"id":"openai/gpt-5.6-sol-pro","context_length":1050000},
        {"id":"anthropic/claude-opus-5.5","context_length":1000000},
        {"id":"someone/claude-opus-5.5","context_length":200000},
        {"id":"anthropic/claude-haiku-4.5","context_length":null,"top_provider":{"context_length":200000}},
        {"id":"z-ai/glm-5.1","context_length":204800},
        {"id":"relay-a/kimi-k3","context_length":262144},
        {"id":"relay-b/kimi-k3","context_length":1048576},
        {"id":"tiny/model-x","context_length":4096},
        {"id":"broken/no-window"}
    ]}"#;

    fn sample() -> ModelWindows {
        ModelWindows {
            fetched_at: 1,
            windows: parse_openrouter(SAMPLE).unwrap(),
        }
    }

    #[test]
    fn the_listing_reduces_to_one_window_per_name() {
        let windows = parse_openrouter(SAMPLE).unwrap();
        assert_eq!(windows.get("gpt-5-6-sol"), Some(&1_050_000), "the variant is skipped");
        assert_eq!(windows.get("gpt-5-6-sol-pro"), Some(&1_050_000));
        assert_eq!(windows.get("claude-opus-5-5"), Some(&1_000_000), "first party wins");
        assert_eq!(windows.get("claude-haiku-4-5"), Some(&200_000), "top provider fallback");
        assert_eq!(windows.get("kimi-k3"), Some(&1_048_576), "the larger of equals");
        assert!(!windows.contains_key("model-x"), "too small to be a window");
        assert!(!windows.contains_key("no-window"));
        assert!(parse_openrouter("{}").is_err());
    }

    #[test]
    fn every_spelling_finds_its_model() {
        let windows = sample();
        for model in [
            "gpt-5.6-sol",
            "codex::gpt-5.6-sol",
            "openai/gpt-5.6-sol",
            "GPT-5.6 Sol",
        ] {
            assert_eq!(windows.lookup(model), Some(1_050_000), "{model}");
        }
        for model in [
            "claude-opus-5-5",
            "claude-opus-5.5",
            "claude-opus-5-5[1m]",
            "anthropic::claude-opus-5.5",
            "Claude Opus 5.5",
            "claude-opus-5-5-20260815",
            "claude-opus-5.5-latest",
        ] {
            assert_eq!(windows.lookup(model), Some(1_000_000), "{model}");
        }
        assert_eq!(windows.lookup("custom:relay::glm-5.1"), Some(204_800));
        assert_eq!(windows.lookup("claude-haiku-4-5-2025-10-01"), Some(200_000));
        assert_eq!(windows.lookup("opus"), None);
        assert_eq!(windows.lookup(""), None);
    }

    #[test]
    fn a_picked_window_is_read_as_given() {
        assert_eq!(explicit_window(Some("200k"), "claude-opus-5-5"), Some(200_000));
        assert_eq!(explicit_window(Some("1m"), "claude-opus-5-5"), Some(1_000_000));
        assert_eq!(explicit_window(None, "claude-opus-5-5[1m]"), Some(1_000_000));
        assert_eq!(explicit_window(Some("200k"), "claude-opus-5-5[1M]"), Some(1_000_000));
        assert_eq!(explicit_window(None, "claude-opus-5-5"), None);
        assert_eq!(explicit_window(Some("auto"), "claude-opus-5-5"), None);
        assert_eq!(explicit_window(Some("0k"), "claude-opus-5-5"), None);
    }

    #[test]
    fn staleness_and_the_cache_round_trip() {
        let windows = sample();
        assert!(!windows.is_stale(1 + TTL_SECONDS - 1));
        assert!(windows.is_stale(1 + TTL_SECONDS));
        assert!(ModelWindows::default().is_stale(0), "nothing fetched is stale");
        let encoded = serde_json::to_string(&windows).unwrap();
        assert_eq!(serde_json::from_str::<ModelWindows>(&encoded).unwrap(), windows);
        assert_eq!(
            serde_json::from_str::<ModelWindows>("{}").unwrap(),
            ModelWindows::default()
        );
    }

    /// Against the live list. Ignored by default: it needs the network.
    #[test]
    #[ignore]
    fn the_live_list_covers_the_gateways_models() {
        let response = crate::http::Request::new()
            .timeout_seconds(30)
            .send(SOURCE_URL)
            .expect("fetch");
        let windows = ModelWindows {
            fetched_at: unix_now(),
            windows: parse_openrouter(&response.body).expect("parse"),
        };
        for model in [
            "gpt-5.6-sol",
            "gpt-6-astra",
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-sonnet-5.5",
            "glm-5.1",
            "kimi-k3",
            "minimax-m3",
        ] {
            let window = windows.lookup(model);
            println!("{model}: {window:?}");
            assert!(window.is_some(), "{model} is not on the list");
        }
    }
}
