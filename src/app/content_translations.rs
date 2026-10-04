//! Display translations of administrator-written text: group names and
//! descriptions, announcements, and the pay service's plans and promotions.
//!
//! Fork addition. Render sites wrap the text they are about to show in
//! [`Waku::tx`], which answers from memory and queues anything it has not
//! seen; [`Waku::flush_content_translations`] — called after the root and
//! pane renders — sends the queue to the gateway's lookup in one background
//! request. The lookup only reads the server's cache, so asking costs the
//! administrator nothing.
//!
//! **Display only.** `tx` returns a new string; the account, routing and pay
//! structures keep their originals, because the Chinese-model lane is
//! classified from the raw names (`sub2api::model_routing`).

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use sub2api::content_translations as translations;

use super::*;

/// How long to wait before asking again about text the server is still
/// translating (or that failed to arrive).
const RETRY_DELAY: Duration = Duration::from_secs(10);
/// How often one text is asked again before its answer is taken as final for
/// this session.
const MAX_RETRIES: u8 = 3;

#[derive(Default)]
pub(super) struct ContentTranslationsState {
    /// The content language of the UI (`zh` / `en` / `ja`); `None` before the
    /// first reset and for a UI language the server cannot translate into.
    lang: Option<&'static str>,
    /// This session's answers: a translation, or `None` for "asked, there is
    /// none — show the original".
    map: HashMap<String, Option<String>>,
    /// The disk cache's translations, shown until this session's answer for
    /// the same text arrives.
    cached: HashMap<String, String>,
    /// Text a frame showed that has not been asked about yet. Renders only
    /// hold `&self`, hence the cell.
    pending: RefCell<BTreeSet<String>>,
    /// A lookup is on its way.
    in_flight: bool,
    /// Text in that lookup or waiting for a retry: never queued again.
    held: HashSet<String>,
    /// Text the next retry asks about again.
    retry_waiting: BTreeSet<String>,
    retry_scheduled: bool,
    /// How often each text has been asked again.
    retries: HashMap<String, u8>,
    /// Bumped on every reset, so a lookup for the previous language cannot
    /// land in the new one.
    generation: u64,
}

impl ContentTranslationsState {
    /// The translation to show for `text`, queueing it when it has not been
    /// asked about this session.
    fn translate(&self, text: &str) -> Option<String> {
        let lang = self.lang?;
        // Sources are registered trimmed (the pay service trims every one),
        // so the lookup key is too.
        let text = text.trim();
        if let Some(answer) = self.map.get(text) {
            return answer.clone();
        }
        if !self.held.contains(text) {
            let mut pending = self.pending.borrow_mut();
            if !pending.contains(text) && translations::wants_translation(lang, text) {
                pending.insert(text.to_owned());
            }
        }
        self.cached.get(text).cloned()
    }
}

impl Waku {
    /// `text` in the UI language when the gateway has a translation for it,
    /// otherwise `text` itself. For display only — never store the result
    /// where routing or account code reads it.
    pub(super) fn tx(&self, text: &str) -> String {
        self.content_translations
            .translate(text)
            .unwrap_or_else(|| text.to_owned())
    }

    /// Start over for the current UI language: at launch and after every
    /// language change. Loads the disk cache off the UI thread.
    pub(super) fn reset_content_translations(&mut self, cx: &mut Context<Self>) {
        let lang = translations::content_lang(self.state.language.locale());
        let generation = self.content_translations.generation.wrapping_add(1);
        self.content_translations = ContentTranslationsState {
            lang,
            generation,
            ..ContentTranslationsState::default()
        };
        // The next frame queues what is on screen for the new language.
        cx.notify();
        let Some(lang) = lang else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let cached = cx
                .background_executor()
                .spawn(async move { translations::load_cached(lang) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.content_translations;
                if state.generation != generation || cached.is_empty() {
                    return;
                }
                state.cached = cached;
                this.relabel_translated_text();
                cx.notify();
            });
        })
        .detach();
    }

    /// Send the text frames queued through [`Self::tx`] to the gateway, one
    /// lookup at a time. Called at the end of every render; a frame that
    /// queued nothing new spawns nothing.
    pub(super) fn flush_content_translations(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.content_translations;
        if state.in_flight || state.pending.get_mut().is_empty() {
            return;
        }
        let Some(lang) = state.lang else {
            return;
        };
        // The lookup is public, but only an account's server knows its
        // groups and plans; signed out there is nothing to show anyway.
        let Some(endpoint) = self
            .cloud_account
            .credentials
            .as_ref()
            .map(|credentials| credentials.endpoint.clone())
        else {
            return;
        };
        let batch: Vec<String> = std::mem::take(state.pending.get_mut())
            .into_iter()
            .filter(|text| !state.map.contains_key(text))
            .collect();
        if batch.is_empty() {
            return;
        }
        state.in_flight = true;
        state.held.extend(batch.iter().cloned());
        let generation = state.generation;
        cx.spawn(async move |this, cx| {
            let texts = batch.clone();
            let result = cx
                .background_executor()
                .spawn(async move { translations::lookup(&endpoint, lang, &texts) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_content_translations(generation, lang, batch, result, cx);
            });
        })
        .detach();
    }

    fn apply_content_translations(
        &mut self,
        generation: u64,
        lang: &'static str,
        batch: Vec<String>,
        result: anyhow::Result<translations::Lookup>,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.content_translations;
        if state.generation != generation {
            return;
        }
        state.in_flight = false;
        let mut answers = Vec::new();
        let (found, pending) = match result {
            Ok(lookup) => (Some(lookup.translations), lookup.pending),
            Err(error) => {
                eprintln!("warning: content translations: {error:#}");
                (None, true)
            }
        };
        for text in batch {
            if let Some(translated) = found.as_ref().and_then(|found| found.get(&text)) {
                state.held.remove(&text);
                state.map.insert(text.clone(), Some(translated.clone()));
                answers.push((text, Some(translated.clone())));
                continue;
            }
            if pending {
                let retries = state.retries.entry(text.clone()).or_default();
                if *retries < MAX_RETRIES {
                    // Still held: the cached translation (if any) keeps
                    // showing until the next answer.
                    *retries += 1;
                    state.retry_waiting.insert(text);
                    continue;
                }
            }
            state.held.remove(&text);
            if found.is_some() {
                // The server's answer is final: show the original.
                state.map.insert(text.clone(), None);
                answers.push((text, None));
            } else {
                // The server could not be reached: keep what was cached.
                let cached = state.cached.get(&text).cloned();
                state.map.insert(text, cached);
            }
        }
        if !answers.is_empty() {
            self.relabel_translated_text();
            cx.background_executor()
                .spawn(async move {
                    if let Err(error) = translations::store(lang, &answers) {
                        eprintln!("warning: could not save content translations: {error:#}");
                    }
                })
                .detach();
        }
        self.schedule_content_translation_retry(cx);
        // Text queued while this lookup was out goes next.
        self.flush_content_translations(cx);
        cx.notify();
    }

    /// Text built once rather than per frame: the built-in agent's picker
    /// rows carry route notes naming groups (`native_routing`).
    fn relabel_translated_text(&mut self) {
        self.sync_native_models();
    }

    fn schedule_content_translation_retry(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.content_translations;
        if state.retry_scheduled || state.retry_waiting.is_empty() {
            return;
        }
        state.retry_scheduled = true;
        let generation = state.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RETRY_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.content_translations;
                if state.generation != generation {
                    return;
                }
                state.retry_scheduled = false;
                let waiting = std::mem::take(&mut state.retry_waiting);
                state.pending.get_mut().extend(waiting);
                this.flush_content_translations(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(lang: &'static str) -> ContentTranslationsState {
        ContentTranslationsState {
            lang: Some(lang),
            ..ContentTranslationsState::default()
        }
    }

    fn pending(state: &ContentTranslationsState) -> Vec<String> {
        state.pending.borrow().iter().cloned().collect()
    }

    #[test]
    fn unseen_text_is_queued_once_and_shown_as_is() {
        let state = state("en");
        assert_eq!(state.translate("国模分组"), None);
        assert_eq!(state.translate("  国模分组 "), None);
        // Already English, blank, or no language yet: nothing to ask.
        assert_eq!(state.translate("Claude Max"), None);
        assert_eq!(state.translate("   "), None);
        assert_eq!(pending(&state), vec!["国模分组".to_owned()]);
        assert_eq!(ContentTranslationsState::default().translate("国模分组"), None);
    }

    #[test]
    fn answers_win_over_the_disk_cache_and_stop_queueing() {
        let mut state = state("en");
        state.cached.insert("公告".into(), "Notice (cached)".into());
        state.cached.insert("旧文案".into(), "Old copy".into());
        // Cached but not yet revalidated: shown, and asked again.
        assert_eq!(state.translate("公告").as_deref(), Some("Notice (cached)"));
        assert_eq!(pending(&state), vec!["公告".to_owned()]);

        state.pending.borrow_mut().clear();
        state.map.insert("公告".into(), Some("Notice".into()));
        state.map.insert("旧文案".into(), None);
        assert_eq!(state.translate("公告").as_deref(), Some("Notice"));
        // The server said there is none: the original, not the stale copy.
        assert_eq!(state.translate("旧文案"), None);
        assert!(pending(&state).is_empty());
    }

    #[test]
    fn held_text_is_not_queued_again() {
        let mut state = state("ja");
        state.held.insert("月度套餐".into());
        state.cached.insert("月度套餐".into(), "月額プラン".into());
        assert_eq!(state.translate("月度套餐").as_deref(), Some("月額プラン"));
        assert!(pending(&state).is_empty());
        // A Japanese UI still asks about Han-only text, but not about kana.
        assert_eq!(state.translate("国模分组"), None);
        assert_eq!(state.translate("サブスク"), None);
        assert_eq!(pending(&state), vec!["国模分组".to_owned()]);
    }
}
