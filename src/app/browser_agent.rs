//! The built-in agent drives the session's in-app browser.
//!
//! Fork addition. A `browser_*` tool call arrives as
//! `DriverEvent::BrowserRequest` (`{"op": …}`); this picks the session's
//! browser tab — opening one, and the panel, when the task on screen has
//! none — waits for its page to exist, does the operation through
//! `crate::browser::automation`, and answers with `Command::BrowserResult`.
//!
//! The browser of the task on screen is the one shown; a task in the
//! background can keep using a browser tab it already has (its page keeps
//! the size it was last drawn at) but cannot open one, because a page never
//! drawn has no size to lay out or photograph.
//!
//! A child of `right_panel`, whose private tab helpers it reuses.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use gpui::AsyncApp;
use serde_json::{Value, json};

use super::*;
use crate::browser::BrowserView;
use crate::browser::automation::{
    self, AgentPage, CONSOLE_READ, SNAPSHOT, page_result, page_script,
};

/// How long a page may take to load before the agent hears "still loading".
const LOAD_LIMIT: Duration = Duration::from_secs(30);
/// How long one script may run (a `browser_wait_for` waits up to a minute).
const SCRIPT_LIMIT: Duration = Duration::from_secs(70);
/// How long the browser may take to come up after its tab is opened.
const START_LIMIT: Duration = Duration::from_secs(20);
const GONE: &str = "the app closed the browser";
const NOT_SHOWN: &str = "this task has no in-app browser open, and one can only be opened for \
                         the task shown in the app — ask the user to switch to this task";

/// The requests already handled, so one replayed after a reconnect is not
/// done twice, and the browsers whose pages record their console.
#[derive(Default)]
pub(in crate::app) struct BrowserAgentState {
    handled: HashSet<String>,
    recording: HashSet<Uuid>,
}

impl Waku {
    pub(in crate::app) fn handle_browser_request(
        &mut self,
        session_id: Uuid,
        request_id: String,
        operation: Value,
        cx: &mut Context<Self>,
    ) {
        if self.browser_agent.handled.len() > 4096 {
            self.browser_agent.handled.clear();
        }
        if !self
            .browser_agent
            .handled
            .insert(format!("{session_id}:{request_id}"))
        {
            return;
        }
        cx.spawn(async move |waku, cx| {
            let result = run(&waku, session_id, &operation, cx).await;
            let _ = waku.update(cx, |waku, _| {
                waku.answer_browser_request(session_id, request_id, result);
            });
        })
        .detach();
    }

    fn answer_browser_request(
        &self,
        session_id: Uuid,
        request_id: String,
        result: Result<Value, String>,
    ) {
        let payload = match result {
            Ok(value) => json!({ "ok": true, "value": value }),
            Err(error) => json!({ "ok": false, "error": error }),
        };
        if let Some(runtime) = self.runtimes.get(&session_id) {
            runtime.driver.browser_result(request_id, payload);
        }
    }

    /// The session's browser tab, by id. For the task on screen: the active
    /// tab when it is a browser, else its first browser tab, else a new one
    /// — and the panel is shown, so the page is drawn and has a size.
    fn agent_browser_tab(&mut self, session_id: Uuid, cx: &mut Context<Self>) -> Result<Uuid, String> {
        if self.state.selected_session != Some(session_id) {
            let state = self
                .right_panel_session_states
                .get(&session_id)
                .ok_or(NOT_SHOWN)?;
            let active = state
                .active_surface
                .and_then(|index| state.surfaces.get(index))
                .and_then(RightPanelSurface::browser_id);
            return active
                .into_iter()
                .chain(state.surfaces.iter().filter_map(RightPanelSurface::browser_id))
                .find(|id| self.right_panel_browsers.contains_key(id))
                .ok_or_else(|| NOT_SHOWN.to_owned());
        }
        let active = self
            .right_panel_active_surface
            .and_then(|index| self.right_panel_surfaces.get(index))
            .and_then(RightPanelSurface::browser_id);
        let id = match active {
            Some(id) => id,
            None => match self
                .right_panel_surfaces
                .iter()
                .position(|surface| surface.browser_id().is_some())
            {
                Some(index) => {
                    self.right_panel_active_surface = Some(index);
                    self.reveal_right_panel_tab(index);
                    self.right_panel_surfaces[index]
                        .browser_id()
                        .expect("a browser tab")
                }
                None => self.agent_open_browser_tab(cx),
            },
        };
        if !self.right_panel_visible {
            self.set_right_panel_visible(true, cx);
        }
        cx.notify();
        Ok(id)
    }

    fn agent_open_browser_tab(&mut self, cx: &mut Context<Self>) -> Uuid {
        let id = Uuid::new_v4();
        self.open_right_panel_surface(RightPanelSurface::Browser(id), cx);
        id
    }

    /// The task's browser tabs in panel order: (surface index, browser id).
    fn agent_browser_tabs(&self, session_id: Uuid) -> (Vec<(usize, Uuid)>, Option<usize>) {
        let (surfaces, active) = if self.state.selected_session == Some(session_id) {
            (&self.right_panel_surfaces, self.right_panel_active_surface)
        } else {
            match self.right_panel_session_states.get(&session_id) {
                Some(state) => (&state.surfaces, state.active_surface),
                None => return (Vec::new(), None),
            }
        };
        let tabs = surfaces
            .iter()
            .enumerate()
            .filter_map(|(index, surface)| surface.browser_id().map(|id| (index, id)))
            .collect();
        (tabs, active)
    }
}

/// Do one request against the session's browser.
async fn run(
    waku: &WeakEntity<Waku>,
    session_id: Uuid,
    operation: &Value,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let op = operation
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if op == "tabs" {
        return tabs(waku, session_id, operation, cx).await;
    }
    let browser = agent_browser(waku, session_id, cx).await?;
    match op {
        "navigate" => {
            let raw = operation
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let url = browser
                .update(cx, |view, cx| view.agent_navigate(raw, cx))?;
            let page = settle(&browser, cx, false).await;
            Ok(text(format!("Opened {url}.\n{}", describe_page(&page))))
        }
        "snapshot" => {
            let value = evaluate(&browser, page_script(SNAPSHOT), cx).await?;
            Ok(text(format_snapshot(&value)))
        }
        "click" | "type" | "select" | "press_key" => {
            let script = match op {
                "click" => automation::click_script(operation),
                "type" => automation::type_script(operation),
                "select" => automation::select_script(operation),
                _ => automation::key_script(operation),
            };
            let acted_on = evaluate(&browser, script, cx).await?;
            let page = settle(&browser, cx, true).await;
            let what = acted_on.as_str().unwrap_or("the element");
            let done = match op {
                "click" => format!("Clicked {what}."),
                "type" => format!("Typed into {what}."),
                "select" => format!("Chose \"{what}\"."),
                _ => format!("Pressed the key on {what}."),
            };
            Ok(text(format!("{done}\n{}", describe_page(&page))))
        }
        "evaluate" => {
            let script = operation
                .get("script")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value = evaluate(&browser, page_script(script), cx).await?;
            Ok(text(match value {
                Value::Null => "The script returned nothing.".to_owned(),
                Value::String(text) => text,
                other => serde_json::to_string_pretty(&other).unwrap_or_default(),
            }))
        }
        "wait_for" => {
            let value = evaluate(&browser, automation::wait_script(operation), cx).await?;
            let found = value.get("found").and_then(Value::as_bool) == Some(true);
            let waited = value.get("waited_ms").and_then(Value::as_u64).unwrap_or_default();
            Ok(text(if found {
                format!("It happened after {waited} ms.")
            } else {
                format!("It did not happen within {waited} ms.")
            }))
        }
        "console" => {
            let value = evaluate(&browser, page_script(CONSOLE_READ), cx).await?;
            Ok(text(format_console(&value)))
        }
        "screenshot" => screenshot(&browser, cx).await,
        "back" | "reload" => {
            browser
                .update(cx, |view, cx| {
                    if op == "back" {
                        view.agent_back(cx);
                    } else {
                        view.agent_reload(cx);
                    }
                });
            let page = settle(&browser, cx, true).await;
            Ok(text(describe_page(&page)))
        }
        other => Err(format!("unknown browser operation {other:?}")),
    }
}

fn text(text: String) -> Value {
    json!({ "text": text })
}

/// The session's browser view, once its page exists.
async fn agent_browser(
    waku: &WeakEntity<Waku>,
    session_id: Uuid,
    cx: &mut AsyncApp,
) -> Result<Entity<BrowserView>, String> {
    let browser_id = waku
        .update(cx, |waku, cx| waku.agent_browser_tab(session_id, cx))
        .map_err(|_| GONE.to_owned())??;
    wait_for_browser(waku, browser_id, cx).await
}

/// The view is created when its tab is first drawn, and its page a moment
/// after; wait for both.
async fn wait_for_browser(
    waku: &WeakEntity<Waku>,
    browser_id: Uuid,
    cx: &mut AsyncApp,
) -> Result<Entity<BrowserView>, String> {
    let deadline = Instant::now() + START_LIMIT;
    loop {
        let state = waku
            .update(cx, |waku, cx| {
                let browser = waku.right_panel_browsers.get(&browser_id).cloned()?;
                let ready = browser.read(cx).agent_ready();
                // Record the console from the first page the agent touches.
                if matches!(ready, Ok(true)) && waku.browser_agent.recording.insert(browser_id) {
                    browser.read(cx).agent_record_console();
                }
                Some((browser, ready))
            })
            .map_err(|_| GONE.to_owned())?;
        match state {
            Some((browser, Ok(true))) => return Ok(browser),
            Some((_, Err(error))) => return Err(error),
            _ => {}
        }
        if Instant::now() > deadline {
            return Err("the in-app browser did not start in time".to_owned());
        }
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
}

/// Run a page script and read what it settled to.
async fn evaluate(
    browser: &Entity<BrowserView>,
    script: String,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let receiver = browser
        .read_with(cx, |view, _| view.agent_evaluate(script));
    let timer = cx.background_executor().timer(SCRIPT_LIMIT);
    let answer = futures_lite::future::or(async { Some(receiver.await) }, async {
        timer.await;
        None
    })
    .await;
    match answer {
        Some(Ok(answer)) => {
            let raw = answer?;
            page_result(&raw)
        }
        Some(Err(_)) => Err(GONE.to_owned()),
        None => Err("the page did not answer in time".to_owned()),
    }
}

/// Give an action's consequences a moment to start, then wait out a load it
/// started. An action that loads nothing costs only the first pause.
async fn settle(browser: &Entity<BrowserView>, cx: &mut AsyncApp, after_action: bool) -> AgentPage {
    if after_action {
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
    }
    let deadline = Instant::now() + LOAD_LIMIT;
    loop {
        let page = browser
            .read_with(cx, |view, _| view.agent_page());
        if !page.loading || Instant::now() > deadline {
            return page;
        }
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
    }
}

fn describe_page(page: &AgentPage) -> String {
    let mut line = match (page.title.trim(), page.url.trim()) {
        ("", "") => "The page is blank.".to_owned(),
        ("", url) => format!("Now at {url}."),
        (title, url) => format!("Now at \"{title}\" — {url}."),
    };
    if page.loading {
        line.push_str(" It is still loading.");
    }
    line
}

async fn screenshot(browser: &Entity<BrowserView>, cx: &mut AsyncApp) -> Result<Value, String> {
    let receiver = browser
        .read_with(cx, |view, _| view.agent_screenshot());
    let data = receiver.await.map_err(|_| GONE.to_owned())??;
    let page = browser
        .read_with(cx, |view, _| view.agent_page());
    let bytes = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&data)
            .map_err(|error| error.to_string())?
    };
    let path = cx
        .background_executor()
        .spawn(async move {
            let directory = std::env::temp_dir().join("waku-browser-screenshots");
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!(
                "screenshot-{}.png",
                chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
            ));
            std::fs::write(&path, bytes)?;
            anyhow::Ok(path)
        })
        .await
        .map_err(|error| format!("could not save the screenshot: {error}"))?;
    Ok(json!({
        "text": format!(
            "Saved a screenshot of {} to {}. The user can see it in the conversation.",
            if page.url.is_empty() { "the page" } else { page.url.as_str() },
            path.display()
        ),
        "png_base64": data,
    }))
}

/// What the agent reads of a page.
fn format_snapshot(value: &Value) -> String {
    let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut out = format!("Title: {}\nURL: {}\n", field("title"), field("url"));
    let elements: Vec<&str> = value
        .get("elements")
        .and_then(Value::as_array)
        .map(|elements| elements.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if elements.is_empty() {
        out.push_str("\nNo interactive elements are visible.\n");
    } else {
        out.push_str("\nInteractive elements (pass the [ref] to browser_click / browser_type):\n");
        for element in elements {
            out.push_str(element);
            out.push('\n');
        }
    }
    let page_text = field("text");
    if !page_text.is_empty() {
        out.push_str("\nVisible text:\n");
        out.push_str(page_text);
        if value.get("truncated").and_then(Value::as_bool) == Some(true) {
            out.push_str("\n… (cut short)");
        }
    }
    out
}

fn format_console(value: &Value) -> String {
    let entries = value.as_array().cloned().unwrap_or_default();
    if entries.is_empty() {
        return "The console has no messages since the agent first used this page.".to_owned();
    }
    entries
        .iter()
        .map(|entry| {
            format!(
                "[{}] {}",
                entry.get("level").and_then(Value::as_str).unwrap_or("log"),
                entry.get("text").and_then(Value::as_str).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `browser_tabs`: list, open, switch, close.
async fn tabs(
    waku: &WeakEntity<Waku>,
    session_id: Uuid,
    operation: &Value,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let action = operation
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list");
    let index = operation
        .get("index")
        .and_then(Value::as_u64)
        .map(|index| index as usize);
    match action {
        "list" => {}
        "new" => {
            let id = waku
                .update(cx, |waku, cx| {
                    if waku.state.selected_session != Some(session_id) {
                        return Err(NOT_SHOWN.to_owned());
                    }
                    let id = waku.agent_open_browser_tab(cx);
                    if !waku.right_panel_visible {
                        waku.set_right_panel_visible(true, cx);
                    }
                    Ok(id)
                })
                .map_err(|_| GONE.to_owned())??;
            let browser = wait_for_browser(waku, id, cx).await?;
            if let Some(url) = operation.get("url").and_then(Value::as_str) {
                browser
                    .update(cx, |view, cx| view.agent_navigate(url, cx))?;
                settle(&browser, cx, false).await;
            }
        }
        "select" | "close" => {
            let index = index.ok_or("give the tab's index, as browser_tabs lists it")?;
            waku.update(cx, |waku, cx| {
                if waku.state.selected_session != Some(session_id) {
                    return Err(NOT_SHOWN.to_owned());
                }
                let (tabs, _) = waku.agent_browser_tabs(session_id);
                let (surface, _) = *tabs
                    .get(index)
                    .ok_or_else(|| format!("there is no browser tab {index}"))?;
                if action == "close" {
                    waku.close_right_panel_surface(surface, cx);
                } else {
                    waku.right_panel_active_surface = Some(surface);
                    waku.reveal_right_panel_tab(surface);
                    if !waku.right_panel_visible {
                        waku.set_right_panel_visible(true, cx);
                    }
                    cx.notify();
                }
                Ok(())
            })
            .map_err(|_| GONE.to_owned())??;
        }
        other => return Err(format!("unknown tabs action {other:?}")),
    }
    let listing = waku
        .update(cx, |waku, cx| {
            let (tabs, active) = waku.agent_browser_tabs(session_id);
            tabs.iter()
                .enumerate()
                .map(|(number, (surface, id))| {
                    let page = waku
                        .right_panel_browsers
                        .get(id)
                        .map(|browser| browser.read(cx).agent_page())
                        .unwrap_or_default();
                    let marker = if active == Some(*surface) { " (selected)" } else { "" };
                    format!("{number}: {}{marker}", describe_page(&page))
                })
                .collect::<Vec<_>>()
        })
        .map_err(|_| GONE.to_owned())?;
    Ok(text(if listing.is_empty() {
        "This task has no in-app browser tabs. browser_navigate opens one.".to_owned()
    } else {
        format!("Browser tabs:\n{}", listing.join("\n"))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_reads_as_refs_then_text() {
        let snapshot = format_snapshot(&json!({
            "title": "Login",
            "url": "http://localhost:3000/login",
            "elements": ["[e1] textbox \"Email\"", "[e2] button \"Sign in\""],
            "text": "Welcome back",
            "truncated": true,
        }));
        assert!(snapshot.starts_with("Title: Login\nURL: http://localhost:3000/login\n"));
        assert!(snapshot.contains("[e2] button \"Sign in\"\n"));
        assert!(snapshot.contains("Visible text:\nWelcome back\n… (cut short)"));
        assert!(format_snapshot(&json!({"elements": []})).contains("No interactive elements"));
    }

    #[test]
    fn pages_are_described_in_one_line() {
        let page = |url: &str, title: &str, loading: bool| AgentPage {
            url: url.to_owned(),
            title: title.to_owned(),
            loading,
        };
        assert_eq!(describe_page(&page("", "", false)), "The page is blank.");
        assert_eq!(describe_page(&page("http://a", "", false)), "Now at http://a.");
        assert_eq!(
            describe_page(&page("http://a", "A", true)),
            "Now at \"A\" — http://a. It is still loading."
        );
    }

    #[test]
    fn console_entries_carry_their_level() {
        assert_eq!(
            format_console(&json!([{"level": "error", "text": "boom"}, {"text": "hi"}])),
            "[error] boom\n[log] hi"
        );
        assert!(format_console(&json!([])).starts_with("The console has no messages"));
    }
}
