//! The window the footer's context ring measures against before — or
//! without — the agent reporting one.
//!
//! Fork addition. The agents report a window only once a turn has finished
//! (Amp never does), and a model switch left the previous model's figure in
//! place. Now every switch drops the old figure and looks the new model up
//! in a public model list (`sub2api::model_windows`, cached for a day), and
//! the ring falls back to that until the agent reports the window it really
//! runs with. The fallback is computed on render and never written into the
//! session, so a reported window is never overwritten by a guess.

use std::time::{Duration, Instant};

use sub2api::model_windows::{self as windows, ModelWindows};

use super::*;

/// A model the list does not carry is not refetched for sooner than this.
const MISS_REFETCH: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub(super) struct ModelWindowsState {
    table: ModelWindows,
    fetching: bool,
    last_attempt: Option<Instant>,
}

impl Waku {
    /// At startup: the list from the last run, refreshed when it is a day old.
    pub(super) fn load_model_windows(&mut self, cx: &mut Context<Self>) {
        self.model_windows.table = windows::load_cached();
        if self.model_windows.table.is_stale(windows::unix_now()) {
            self.fetch_model_windows(cx);
        }
    }

    fn fetch_model_windows(&mut self, cx: &mut Context<Self>) {
        if self.model_windows.fetching {
            return;
        }
        self.model_windows.fetching = true;
        self.model_windows.last_attempt = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { windows::refresh() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.model_windows.fetching = false;
                match fetched {
                    Ok(table) => {
                        this.model_windows.table = table;
                        cx.notify();
                    }
                    // The ring keeps its bare token count; nothing to tell
                    // the user about.
                    Err(error) => eprintln!("warning: {error:#}"),
                }
            });
        })
        .detach();
    }

    /// Every switch of model or context option: the window the agent
    /// reported belongs to the previous choice, so it goes, and the new
    /// model is looked up — from memory when the list has it and is fresh,
    /// from the network otherwise.
    pub(super) fn note_model_changed(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        let Some(session) = self.state.session_mut(session_id) else {
            return;
        };
        if let Some(usage) = session.context_usage.as_mut()
            && usage.window.take().is_some()
        {
            self.state.mark_session_dirty(session_id);
        }
        let Some(session) = self.state.sessions.iter().find(|session| session.id == session_id)
        else {
            return;
        };
        let missing = self.listed_context_window(session).is_none();
        let state = &self.model_windows;
        let stale = state.table.is_stale(windows::unix_now());
        let recently_tried = state
            .last_attempt
            .is_some_and(|at| at.elapsed() < MISS_REFETCH);
        if stale || (missing && !recently_tried) {
            self.fetch_model_windows(cx);
        }
    }

    /// The usage the ring and its panel show: the session's own, with the
    /// window the agent reported — or, until it has, the one the user picked
    /// or the public list gives for the model.
    pub(super) fn ring_context_usage(&self, session: &AgentSession) -> Option<ContextUsage> {
        with_fallback_window(session.context_usage, || {
            self.picked_context_window(session)
                .or_else(|| self.listed_context_window(session))
        })
    }

    /// The model the session runs, as the provider's catalog describes it
    /// when it does: the picked one, or the provider's default.
    fn ring_model<'a>(&'a self, session: &'a AgentSession) -> Option<(&'a str, Option<&'a ProviderModel>)> {
        match session.model.as_deref() {
            Some(model) => Some((model, self.probe_model(session.provider, model))),
            None => {
                let default = self
                    .provider_probe(session.provider)?
                    .models
                    .iter()
                    .find(|model| model.is_default)?;
                Some((default.id.as_str(), Some(default)))
            }
        }
    }

    /// A window the user chose: the session's context option (Claude Code's
    /// 200k / 1M) — only on a model that offers the choice — or `[1m]` on
    /// the id.
    fn picked_context_window(&self, session: &AgentSession) -> Option<u64> {
        let (model, catalog) = self.ring_model(session)?;
        let option = catalog
            .filter(|catalog| !catalog.context_windows.is_empty())
            .and_then(|catalog| {
                session
                    .context_window
                    .as_deref()
                    .or(catalog.default_context_window.as_deref())
            });
        windows::explicit_window(option, model)
    }

    /// The model's window on the public list, by id and then by the name the
    /// picker shows — a CLI alias like `opus` only matches by name.
    fn listed_context_window(&self, session: &AgentSession) -> Option<u64> {
        let (model, catalog) = self.ring_model(session)?;
        let table = &self.model_windows.table;
        table
            .lookup(model)
            .or_else(|| catalog.and_then(|catalog| table.lookup(&catalog.name)))
    }
}

/// A reported window stands; otherwise `fallback` fills one in. A session
/// with no usage yet still gets a ring — empty, but sized.
fn with_fallback_window(
    usage: Option<ContextUsage>,
    fallback: impl FnOnce() -> Option<u64>,
) -> Option<ContextUsage> {
    let reported = usage
        .and_then(|usage| usage.window)
        .filter(|window| *window > 0);
    let window = reported.or_else(fallback);
    match usage {
        Some(mut usage) => {
            usage.window = window;
            Some(usage)
        }
        None => window.map(|window| ContextUsage {
            window: Some(window),
            ..ContextUsage::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(tokens: u64, window: Option<u64>) -> ContextUsage {
        ContextUsage {
            tokens,
            window,
            ..ContextUsage::default()
        }
    }

    #[test]
    fn a_reported_window_outranks_the_public_list() {
        let shown = with_fallback_window(Some(usage(50_000, Some(200_000))), || Some(1_000_000));
        assert_eq!(shown, Some(usage(50_000, Some(200_000))));
    }

    #[test]
    fn the_list_fills_a_missing_window_and_keeps_the_tokens() {
        let shown = with_fallback_window(Some(usage(50_000, None)), || Some(1_050_000));
        assert_eq!(shown, Some(usage(50_000, Some(1_050_000))));
        // A zero window is no window.
        let shown = with_fallback_window(Some(usage(1, Some(0))), || Some(204_800));
        assert_eq!(shown, Some(usage(1, Some(204_800))));
    }

    #[test]
    fn a_new_session_gets_an_empty_but_sized_ring() {
        assert_eq!(
            with_fallback_window(None, || Some(1_000_000)),
            Some(usage(0, Some(1_000_000)))
        );
        assert_eq!(with_fallback_window(None, || None), None);
        assert_eq!(
            with_fallback_window(Some(usage(7, None)), || None),
            Some(usage(7, None))
        );
    }
}
