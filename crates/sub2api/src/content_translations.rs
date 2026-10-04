//! Display translations of administrator-written text.
//!
//! Group names and descriptions, announcements and the pay service's plans
//! are typed by the administrator in one language and cannot go through
//! `rust-i18n`. The gateway translates the ones it knows about with a cheap
//! model and keeps the results; `POST /api/v1/content-translations/lookup`
//! reads that cache (no sign-in, never a model call — an unknown text simply
//! has no translation). See `docs/CONTENT_TRANSLATION.md` in the main
//! repository for the contract.
//!
//! **Display only.** The desktop sends the original text it is about to show
//! and shows the translation when there is one. Nothing translated is ever
//! written back into the structures the account, routing and payment code
//! read: `model_routing` classifies the Chinese models' lane by the raw group
//! and plan names ("国模"), and a translated name would route wrongly.
//!
//! A small copy of the answers is kept in `~/.cheaprouter/content-translations.json`
//! so the first frames after a launch are already translated; every entry is
//! still asked again once per session, which only reads the server's cache.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::brand;
use crate::client::{Client, envelope_data, null_to_default};
use crate::global_config::atomic_write_private;
use crate::http::{Request, Response};

/// The most texts one lookup request may carry.
pub const MAX_TEXTS_PER_REQUEST: usize = 200;
/// The server ignores longer texts, so they are never asked about.
pub const MAX_TEXT_CHARS: usize = 20_000;
/// Entries kept on disk per language.
pub const CACHE_CAP_PER_LANG: usize = 2_000;

const LOOKUP_PATH: &str = "content-translations/lookup";
/// A display nicety: a slow server must not hold a batch for long.
const TIMEOUT_SECONDS: u32 = 10;
const CACHE_FILE: &str = "content-translations.json";

/// The content language for a UI locale (`en`, `zh-CN`, `ja`, …), or `None`
/// when the server translates into nothing that fits it.
pub fn content_lang(locale: &str) -> Option<&'static str> {
    let locale = locale.trim().to_ascii_lowercase();
    if locale.starts_with("zh") {
        Some("zh")
    } else if locale.starts_with("ja") {
        Some("ja")
    } else if locale.starts_with("en") {
        Some("en")
    } else {
        None
    }
}

/// Whether `text` is worth asking about for `lang`: not blank, not too long
/// for the server, and not already written in that language.
///
/// The test is by script: Chinese is Han without kana, Japanese has kana,
/// English has neither (nor Hangul). A Japanese UI still asks about Han-only
/// text, which is far more likely to be Chinese than Japanese without a
/// single kana. Text with no letters at all (`$10`, `—`) is never asked.
pub fn wants_translation(lang: &str, text: &str) -> bool {
    if text.trim().is_empty() || text.chars().count() > MAX_TEXT_CHARS {
        return false;
    }
    let mut letters = false;
    let mut han = false;
    let mut kana = false;
    let mut hangul = false;
    for c in text.chars() {
        if is_kana(c) {
            kana = true;
        } else if is_han(c) {
            han = true;
        } else if is_hangul(c) {
            hangul = true;
        }
        letters |= c.is_alphabetic();
    }
    if !letters && !han && !kana {
        return false;
    }
    let already = match lang {
        "zh" => han && !kana,
        "ja" => kana,
        "en" => !han && !kana && !hangul,
        _ => true,
    };
    !already
}

fn is_han(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FA1F}')
}

fn is_kana(c: char) -> bool {
    // Hiragana, katakana, katakana extensions and half-width katakana. The
    // prolonged sound mark sits in the katakana block.
    matches!(c,
        '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{31F0}'..='\u{31FF}'
        | '\u{FF66}'..='\u{FF9F}')
}

fn is_hangul(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'
        | '\u{3130}'..='\u{318F}'
        | '\u{AC00}'..='\u{D7AF}')
}

/// What one lookup found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lookup {
    /// Original text, exactly as sent → its translation. A text with no
    /// translation (unknown, already in the language, not done yet) is
    /// absent.
    pub translations: HashMap<String, String>,
    /// Some text is queued for translation on the server; asking again in a
    /// few seconds may find it.
    pub pending: bool,
}

#[derive(Debug, Default, Deserialize)]
struct LookupData {
    #[serde(default, deserialize_with = "null_to_default")]
    translations: HashMap<String, String>,
    #[serde(default)]
    pending: bool,
}

/// Ask `endpoint` (the service origin) for `texts` in `lang`, in requests of
/// at most [`MAX_TEXTS_PER_REQUEST`]. Blocks; run it off the UI thread. Any
/// failed request fails the whole lookup, so the caller retries all of it.
pub fn lookup(endpoint: &str, lang: &str, texts: &[String]) -> Result<Lookup> {
    let url = Client::new(endpoint).api_url(LOOKUP_PATH);
    let mut found = Lookup::default();
    for chunk in texts.chunks(MAX_TEXTS_PER_REQUEST) {
        let response = Request::new()
            .timeout_seconds(TIMEOUT_SECONDS)
            .json_body(request_body(lang, chunk))
            .send(&url)
            .context("could not look up content translations")?;
        let part = parse_lookup(&response)?;
        found.translations.extend(part.translations);
        found.pending |= part.pending;
    }
    Ok(found)
}

fn request_body(lang: &str, texts: &[String]) -> String {
    serde_json::json!({ "lang": lang, "texts": texts }).to_string()
}

/// Read a lookup answer out of the service's `{code, message, data}`
/// envelope. An empty translation is no translation.
pub fn parse_lookup(response: &Response) -> Result<Lookup> {
    let data: LookupData = envelope_data(response)?.unwrap_or_default();
    let mut translations = data.translations;
    translations.retain(|_, translated| !translated.trim().is_empty());
    Ok(Lookup {
        translations,
        pending: data.pending,
    })
}

// ── Disk cache ────────────────────────────────────────────────────────────

/// `{ "<lang>": { "<original>": "<translated>" } }`.
type CacheFile = BTreeMap<String, BTreeMap<String, String>>;

/// Serializes the read-modify-write of the cache file: two lookups landing
/// together would otherwise race for the same temporary file.
static CACHE_LOCK: Mutex<()> = Mutex::new(());

pub fn cache_path() -> Option<PathBuf> {
    brand::data_dir().map(|dir| dir.join(CACHE_FILE))
}

fn read_cache_file() -> CacheFile {
    cache_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// The translations kept for `lang`; empty when there are none.
pub fn load_cached(lang: &str) -> HashMap<String, String> {
    let _guard = CACHE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    read_cache_file()
        .remove(lang)
        .map(|entries| entries.into_iter().collect())
        .unwrap_or_default()
}

/// Fold one lookup's answers for `lang` into the cache file: a translation is
/// kept, an answer without one drops what was kept. Blocks; run it off the UI
/// thread.
pub fn store(lang: &str, answers: &[(String, Option<String>)]) -> Result<()> {
    if answers.is_empty() {
        return Ok(());
    }
    let _guard = CACHE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = cache_path().ok_or_else(|| anyhow!("could not locate the home directory"))?;
    let mut file = read_cache_file();
    merge_answers(
        file.entry(lang.to_owned()).or_default(),
        answers,
        CACHE_CAP_PER_LANG,
    );
    let encoded = serde_json::to_string(&file).context("could not encode content translations")?;
    atomic_write_private(&path, encoded.as_bytes())
}

/// Apply `answers` to one language's entries and trim them to `cap`, giving
/// up older entries before the ones just answered.
pub fn merge_answers(
    entries: &mut BTreeMap<String, String>,
    answers: &[(String, Option<String>)],
    cap: usize,
) {
    for (original, translated) in answers {
        match translated {
            Some(translated) => {
                entries.insert(original.clone(), translated.clone());
            }
            None => {
                entries.remove(original);
            }
        }
    }
    if entries.len() <= cap {
        return;
    }
    let fresh: std::collections::HashSet<&str> =
        answers.iter().map(|(original, _)| original.as_str()).collect();
    let mut excess = entries.len() - cap;
    let stale: Vec<String> = entries
        .keys()
        .filter(|original| !fresh.contains(original.as_str()))
        .take(excess)
        .cloned()
        .collect();
    for original in stale {
        entries.remove(&original);
        excess -= 1;
    }
    while excess > 0 {
        let Some(first) = entries.keys().next().cloned() else {
            break;
        };
        entries.remove(&first);
        excess -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: &str) -> Response {
        Response {
            status: 200,
            body: body.to_owned(),
        }
    }

    #[test]
    fn ui_locales_map_onto_content_languages() {
        assert_eq!(content_lang("en"), Some("en"));
        assert_eq!(content_lang("en-US"), Some("en"));
        assert_eq!(content_lang("zh-CN"), Some("zh"));
        assert_eq!(content_lang("zh-Hant"), Some("zh"));
        assert_eq!(content_lang("ja"), Some("ja"));
        assert_eq!(content_lang("ja-JP"), Some("ja"));
        assert_eq!(content_lang("ZH-cn"), Some("zh"));
        assert_eq!(content_lang("fr"), None);
        assert_eq!(content_lang(""), None);
    }

    #[test]
    fn text_already_in_the_ui_language_is_not_asked_about() {
        // Chinese UI: Han without kana is already Chinese.
        assert!(!wants_translation("zh", "国模 DeepSeek 分组"));
        assert!(wants_translation("zh", "Claude Max group"));
        assert!(wants_translation("zh", "サブスクリプション"));
        // Japanese UI: kana means Japanese, Han alone is most likely Chinese.
        assert!(!wants_translation("ja", "月額プラン"));
        assert!(wants_translation("ja", "月度套餐"));
        assert!(wants_translation("ja", "Monthly plan"));
        // English UI: anything without CJK is left alone.
        assert!(!wants_translation("en", "Claude Max group"));
        assert!(wants_translation("en", "国模分组"));
        assert!(wants_translation("en", "月額プラン"));
        assert!(wants_translation("en", "한국어"));
    }

    #[test]
    fn blank_letterless_and_oversized_text_is_never_asked_about() {
        for lang in ["zh", "en", "ja"] {
            assert!(!wants_translation(lang, ""));
            assert!(!wants_translation(lang, "   \n"));
            assert!(!wants_translation(lang, "$10 / 30"));
        }
        let long = "字".repeat(MAX_TEXT_CHARS + 1);
        assert!(!wants_translation("en", &long));
        let limit = "字".repeat(MAX_TEXT_CHARS);
        assert!(wants_translation("en", &limit));
        assert!(!wants_translation("xx", "anything"));
    }

    #[test]
    fn parses_the_lookup_envelope() {
        let lookup = parse_lookup(&ok(
            r#"{"code":0,"message":"success","data":{"lang":"en",
                "translations":{"分组描述原文":"Group description","空":"  "},
                "pending":true}}"#,
        ))
        .expect("parse");
        assert_eq!(
            lookup.translations.get("分组描述原文").map(String::as_str),
            Some("Group description")
        );
        // An empty translation is no translation.
        assert!(!lookup.translations.contains_key("空"));
        assert!(lookup.pending);

        // Nothing found: `translations` may come back null or absent.
        let empty = parse_lookup(&ok(
            r#"{"code":0,"data":{"lang":"ja","translations":null}}"#,
        ))
        .expect("parse");
        assert!(empty.translations.is_empty());
        assert!(!empty.pending);
        let bare = parse_lookup(&ok(r#"{"code":0,"data":null}"#)).expect("parse");
        assert_eq!(bare, Lookup::default());
    }

    #[test]
    fn a_refused_lookup_is_an_error() {
        let error = parse_lookup(&Response {
            status: 400,
            body: r#"{"code":400,"message":"unsupported lang"}"#.to_owned(),
        })
        .expect_err("400");
        assert!(error.to_string().contains("unsupported lang"));
        let error = parse_lookup(&ok(r#"{"code":500,"message":"busy"}"#)).expect_err("code");
        assert!(error.to_string().contains("busy"));
    }

    #[test]
    fn the_request_body_carries_the_language_and_texts() {
        let body: serde_json::Value =
            serde_json::from_str(&request_body("zh", &["Plan \"A\"".to_owned()])).unwrap();
        assert_eq!(body["lang"], "zh");
        assert_eq!(body["texts"][0], "Plan \"A\"");
    }

    #[test]
    fn answers_update_the_cache_and_the_cap_drops_older_entries_first() {
        let mut entries = BTreeMap::new();
        entries.insert("a".to_owned(), "A".to_owned());
        entries.insert("b".to_owned(), "B".to_owned());
        entries.insert("gone".to_owned(), "G".to_owned());
        merge_answers(
            &mut entries,
            &[
                ("gone".to_owned(), None),
                ("z".to_owned(), Some("Z".to_owned())),
                ("y".to_owned(), Some("Y".to_owned())),
            ],
            3,
        );
        // `gone` was answered without a translation; `a` is the oldest
        // entry that was not just answered.
        assert_eq!(
            entries.into_iter().collect::<Vec<_>>(),
            vec![
                ("b".to_owned(), "B".to_owned()),
                ("y".to_owned(), "Y".to_owned()),
                ("z".to_owned(), "Z".to_owned()),
            ]
        );

        // More fresh answers than the cap still ends at the cap.
        let mut entries = BTreeMap::new();
        let answers: Vec<_> = (0..5)
            .map(|index| (format!("t{index}"), Some(format!("T{index}"))))
            .collect();
        merge_answers(&mut entries, &answers, 2);
        assert_eq!(entries.len(), 2);
    }
}
