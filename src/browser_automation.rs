//! What the built-in agent's browser tools do to the page.
//!
//! Fork addition. The agent's `browser_*` tools (`waku-agent-bridge`'s
//! `browser.rs`) reach the desktop as requests; `app/browser_agent.rs` picks
//! the session's browser tab and calls these. Everything that reads or acts
//! on the page is JavaScript evaluated in it, so the result is the same on
//! every engine; the scripts are built here.
//!
//! Windows evaluates through the Chrome DevTools Protocol
//! (`Runtime.evaluate`, which awaits a promise and returns its value) and
//! takes screenshots with `Page.captureScreenshot`. WKWebView has neither,
//! and this build cannot verify the AppKit side, so other platforms answer
//! "not supported" and the daemon does not offer the tools there.

use futures::channel::oneshot;
use serde_json::{Value, json};

use super::*;

/// Why a request cannot run on this platform.
pub const UNSUPPORTED: &str = "the in-app browser cannot be automated on this platform yet";
const NOT_READY: &str = "the in-app browser has not finished starting";

/// What the page is showing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentPage {
    pub url: String,
    pub title: String,
    pub loading: bool,
}

impl BrowserView {
    /// Whether the page can be driven yet, or why it never will be.
    pub fn agent_ready(&self) -> Result<bool, String> {
        if let Some(error) = &self.host_error {
            return Err(error.clone());
        }
        if cfg!(not(target_os = "windows")) {
            return Err(UNSUPPORTED.to_owned());
        }
        Ok(self.host.is_some())
    }

    pub fn agent_page(&self) -> AgentPage {
        AgentPage {
            url: self.current_url.clone().unwrap_or_default(),
            title: self.page_title.clone().unwrap_or_default(),
            loading: self.loading,
        }
    }

    /// Open `raw` — resolved the way the address bar resolves what is typed
    /// into it — without taking the keyboard from wherever the person is.
    pub fn agent_navigate(&mut self, raw: &str, cx: &mut Context<Self>) -> Result<String, String> {
        let url = match resolve_address(raw) {
            Some(AddressTarget::Url(url)) => url,
            Some(AddressTarget::Search(query)) => search_url(&query),
            None => return Err("an address is required".to_owned()),
        };
        self.agent_load(url, cx)
    }

    #[cfg(target_os = "windows")]
    fn agent_load(&mut self, url: String, cx: &mut Context<Self>) -> Result<String, String> {
        let Some(host) = &self.host else {
            return Err(NOT_READY.to_owned());
        };
        host.webview
            .load_url(&url)
            .map_err(|error| error.to_string())?;
        self.navigation_requested = true;
        self.loading = true;
        self.current_url = Some(url.clone());
        self.address_dirty = false;
        self.echo_page_url(cx);
        cx.notify();
        Ok(url)
    }

    #[cfg(not(target_os = "windows"))]
    fn agent_load(&mut self, _url: String, _cx: &mut Context<Self>) -> Result<String, String> {
        Err(UNSUPPORTED.to_owned())
    }

    pub fn agent_back(&mut self, cx: &mut Context<Self>) {
        self.go_back(cx);
    }

    pub fn agent_reload(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    /// Evaluate a [`page_script`] and hand back the string it settles to.
    pub fn agent_evaluate(&self, expression: String) -> oneshot::Receiver<Result<String, String>> {
        let (sender, receiver) = oneshot::channel();
        self.agent_devtools(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
                "userGesture": true,
            }),
            Box::new(move |answer| {
                let _ = sender.send(answer.and_then(|json| evaluation_value(&json)));
            }),
        );
        receiver
    }

    /// A PNG of the visible page, base64-encoded.
    pub fn agent_screenshot(&self) -> oneshot::Receiver<Result<String, String>> {
        let (sender, receiver) = oneshot::channel();
        self.agent_devtools(
            "Page.captureScreenshot",
            json!({ "format": "png" }),
            Box::new(move |answer| {
                let _ = sender.send(answer.and_then(|json| screenshot_data(&json)));
            }),
        );
        receiver
    }

    /// Record console messages from the start of every page this view loads
    /// from now on; [`CONSOLE_HOOK`] in each evaluation covers the page that
    /// is already open.
    pub fn agent_record_console(&self) {
        self.agent_devtools(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": CONSOLE_HOOK }),
            Box::new(|_| {}),
        );
    }

    #[cfg(target_os = "windows")]
    fn agent_devtools(
        &self,
        method: &str,
        params: Value,
        done: Box<dyn FnOnce(Result<String, String>)>,
    ) {
        match &self.host {
            Some(host) => host
                .webview
                .call_devtools(method, &params.to_string(), done),
            None => done(Err(NOT_READY.to_owned())),
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn agent_devtools(
        &self,
        _method: &str,
        _params: Value,
        done: Box<dyn FnOnce(Result<String, String>)>,
    ) {
        done(Err(UNSUPPORTED.to_owned()));
    }
}

/// `Runtime.evaluate`'s answer: the value, or why evaluating threw.
fn evaluation_value(json: &str) -> Result<String, String> {
    let answer: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    if let Some(details) = answer.get("exceptionDetails") {
        let message = details
            .pointer("/exception/description")
            .or_else(|| details.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("the script threw");
        return Err(message.to_owned());
    }
    match answer.pointer("/result/value") {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Ok(other.to_string()),
        None => Ok("null".to_owned()),
    }
}

fn screenshot_data(json: &str) -> Result<String, String> {
    let answer: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    answer
        .get("data")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "the screenshot came back empty".to_owned())
}

/// What a [`page_script`] settled to: its value, or the error it threw.
pub fn page_result(raw: &str) -> Result<Value, String> {
    let settled: Value = serde_json::from_str(raw)
        .map_err(|_| format!("the page answered something unexpected: {raw}"))?;
    if settled.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(settled.get("value").cloned().unwrap_or(Value::Null))
    } else {
        Err(settled
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the page script failed")
            .to_owned())
    }
}

/// Records console output and uncaught errors into `window.__wakuConsole`,
/// once per page.
pub const CONSOLE_HOOK: &str = r#"(() => {
  if (window.__wakuConsole) return;
  window.__wakuConsole = [];
  const push = (level, args) => {
    try {
      const text = args.map((a) => {
        if (typeof a === 'string') return a;
        if (a instanceof Error) return a.stack || String(a);
        try { return JSON.stringify(a); } catch (e) { return String(a); }
      }).join(' ').slice(0, 2000);
      window.__wakuConsole.push({ level, text, at: Date.now() });
      if (window.__wakuConsole.length > 200) window.__wakuConsole.shift();
    } catch (e) {}
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    console[level] = function (...args) { push(level, args); return original.apply(this, args); };
  }
  window.addEventListener('error', (e) => push('error', [e.message + (e.filename ? ` (${e.filename}:${e.lineno})` : '')]));
  window.addEventListener('unhandledrejection', (e) => push('error', ['Unhandled rejection: ' + ((e.reason && e.reason.stack) || e.reason)]));
})();"#;

/// Helpers every script can use: finding the element a tool names, and
/// pressing a key on one.
const PRELUDE: &str = r#"
const __clean = (s) => String(s == null ? '' : s).replace(/\s+/g, ' ').trim();
const __name = (el) => __clean(el.getAttribute('aria-label') || (el.labels && el.labels[0] && el.labels[0].innerText) || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || (el.tagName === 'INPUT' ? el.value : el.tagName === 'SELECT' ? '' : el.innerText) || el.getAttribute('name') || '').slice(0, 80);
const __find = (t) => {
  if (t.ref) {
    const el = document.querySelector('[data-waku-ref="' + CSS.escape(t.ref) + '"]');
    if (!el) throw new Error('no element ' + t.ref + ' on the page any more; take a new browser_snapshot');
    return el;
  }
  if (t.selector) {
    const el = document.querySelector(t.selector);
    if (!el) throw new Error('nothing on the page matches ' + t.selector);
    return el;
  }
  if (t.text) {
    const want = __clean(t.text).toLowerCase();
    const all = [...document.querySelectorAll('a,button,input,textarea,select,label,summary,[role],[onclick],[contenteditable]')];
    let el = all.find((e) => __name(e).toLowerCase() === want) || all.find((e) => __name(e).toLowerCase().includes(want));
    if (el && el.tagName === 'LABEL' && el.control) el = el.control;
    if (!el) throw new Error('no element with the text "' + t.text + '"');
    return el;
  }
  return document.activeElement || document.body;
};
const __describe = (el) => el.tagName.toLowerCase() + (__name(el) ? ' "' + __name(el) + '"' : '');
const __press = (el, key) => {
  const o = { key, code: key.length === 1 ? 'Key' + key.toUpperCase() : key, bubbles: true, cancelable: true };
  const go = el.dispatchEvent(new KeyboardEvent('keydown', o));
  if (key.length === 1 || key === 'Enter') el.dispatchEvent(new KeyboardEvent('keypress', o));
  el.dispatchEvent(new KeyboardEvent('keyup', o));
  if (go && key === 'Enter' && el.form && el.tagName !== 'TEXTAREA') {
    if (el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit();
  }
};
"#;

/// An expression that runs `body` as an async function in the page and
/// settles to `{"ok":true,"value":…}` or `{"ok":false,"error":…}` as a JSON
/// string — the one shape every caller reads with [`page_result`].
pub fn page_script(body: &str) -> String {
    format!(
        "(async () => {{\n{CONSOLE_HOOK}\n{PRELUDE}\ntry {{\n  const __v = await (async () => {{\n{body}\n  }})();\n  return JSON.stringify({{ ok: true, value: __v === undefined ? null : __v }});\n}} catch (e) {{\n  return JSON.stringify({{ ok: false, error: String((e && e.message) || e) }});\n}}\n}})()"
    )
}

/// The element a tool call names, as a JavaScript object literal.
pub fn target_literal(input: &Value) -> String {
    let field = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    };
    json!({ "ref": field("ref"), "selector": field("selector"), "text": field("text") }).to_string()
}

pub const SNAPSHOT: &str = r#"
const MAX = 300;
const sel = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=radio],[role=tab],[role=menuitem],[role=option],[role=switch],[role=combobox],[role=textbox],[contenteditable=""],[contenteditable="true"],[onclick],[tabindex]:not([tabindex="-1"])';
const visible = (el) => {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return false;
  const s = getComputedStyle(el);
  return s.visibility !== 'hidden' && s.display !== 'none' && Number(s.opacity) !== 0;
};
const role = (el) => {
  const r = el.getAttribute('role');
  if (r) return r;
  const t = el.tagName.toLowerCase();
  if (t === 'a') return 'link';
  if (t === 'input') {
    const type = (el.getAttribute('type') || 'text').toLowerCase();
    return ({ checkbox: 'checkbox', radio: 'radio', submit: 'button', button: 'button', reset: 'button', image: 'button', range: 'slider', file: 'file' })[type] || 'textbox';
  }
  if (t === 'textarea' || el.isContentEditable) return 'textbox';
  if (t === 'select') return 'combobox';
  if (t === 'summary') return 'button';
  return t;
};
window.__wakuRefSeq = window.__wakuRefSeq || 0;
const lines = [];
for (const el of document.querySelectorAll(sel)) {
  if (!visible(el)) continue;
  if (lines.length >= MAX) { lines.push('… more elements not listed'); break; }
  let ref = el.getAttribute('data-waku-ref');
  if (!ref) { ref = 'e' + (++window.__wakuRefSeq); el.setAttribute('data-waku-ref', ref); }
  const r = role(el);
  let line = '[' + ref + '] ' + r + ' "' + __name(el) + '"';
  if (r === 'textbox' || r === 'combobox' || r === 'slider') {
    const value = el.isContentEditable ? el.innerText : el.tagName === 'SELECT' ? (el.selectedOptions[0] ? el.selectedOptions[0].text : '') : el.value;
    if (value) line += ' value="' + __clean(value).slice(0, 60) + '"';
  }
  if (r === 'checkbox' || r === 'radio' || r === 'switch') {
    line += (el.checked || el.getAttribute('aria-checked') === 'true') ? ' checked' : ' unchecked';
  }
  if (r === 'link' && el.getAttribute('href')) line += ' -> ' + el.getAttribute('href').slice(0, 100);
  if (el.disabled || el.getAttribute('aria-disabled') === 'true') line += ' disabled';
  lines.push(line);
}
const text = (document.body ? document.body.innerText : '').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim();
return { url: location.href, title: document.title, elements: lines, text: text.slice(0, 8000), truncated: text.length > 8000 };
"#;

pub fn click_script(input: &Value) -> String {
    page_script(&format!(
        r#"
const el = __find({target});
el.scrollIntoView({{ block: 'center', inline: 'center' }});
if (el.disabled) throw new Error(__describe(el) + ' is disabled');
const r = el.getBoundingClientRect();
const o = {{ bubbles: true, cancelable: true, view: window, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, button: 0 }};
el.dispatchEvent(new PointerEvent('pointerdown', o));
el.dispatchEvent(new MouseEvent('mousedown', o));
if (el.focus) el.focus();
el.dispatchEvent(new PointerEvent('pointerup', o));
el.dispatchEvent(new MouseEvent('mouseup', o));
el.click();
return __describe(el);
"#,
        target = target_literal(input)
    ))
}

pub fn type_script(input: &Value) -> String {
    let options = json!({
        "text": input.get("value").and_then(Value::as_str).unwrap_or_default(),
        "append": input.get("append").and_then(Value::as_bool).unwrap_or(false),
        "submit": input.get("submit").and_then(Value::as_bool).unwrap_or(false),
    });
    page_script(&format!(
        r#"
const el = __find({target});
const v = {options};
el.scrollIntoView({{ block: 'center' }});
if (el.focus) el.focus();
if (el.disabled || el.readOnly) throw new Error(__describe(el) + ' does not accept typing');
const before = el.isContentEditable ? el.innerText : (el.value || '');
const value = v.append ? before + v.text : v.text;
if (el.isContentEditable) {{
  el.textContent = value;
}} else if ('value' in el) {{
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
  setter.call(el, value);
}} else {{
  throw new Error(__describe(el) + ' is not a field');
}}
el.dispatchEvent(new InputEvent('input', {{ bubbles: true, inputType: 'insertText', data: v.text }}));
el.dispatchEvent(new Event('change', {{ bubbles: true }}));
if (v.submit) __press(el, 'Enter');
return __describe(el);
"#,
        target = target_literal(input)
    ))
}

pub fn select_script(input: &Value) -> String {
    let option = json!(input.get("option").and_then(Value::as_str).unwrap_or_default());
    page_script(&format!(
        r#"
const el = __find({target});
const want = {option};
if (el.tagName !== 'SELECT') throw new Error(__describe(el) + ' is not a <select>');
const options = [...el.options];
const opt = options.find((o) => o.value === want) || options.find((o) => __clean(o.text) === want) || options.find((o) => __clean(o.text).toLowerCase().includes(String(want).toLowerCase()));
if (!opt) throw new Error('no option "' + want + '"; the options are: ' + options.map((o) => __clean(o.text)).join(', '));
el.value = opt.value;
el.dispatchEvent(new Event('input', {{ bubbles: true }}));
el.dispatchEvent(new Event('change', {{ bubbles: true }}));
return __clean(opt.text);
"#,
        target = target_literal(input)
    ))
}

pub fn key_script(input: &Value) -> String {
    let key = json!(input.get("key").and_then(Value::as_str).unwrap_or("Enter"));
    page_script(&format!(
        r#"
const el = document.activeElement || document.body;
__press(el, {key});
return __describe(el);
"#
    ))
}

pub fn wait_script(input: &Value) -> String {
    let options = json!({
        "text": input.get("text").and_then(Value::as_str),
        "selector": input.get("selector").and_then(Value::as_str),
        "gone": input.get("gone").and_then(Value::as_bool).unwrap_or(false),
        "timeout": input.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10_000).clamp(100, 60_000),
    });
    page_script(&format!(
        r#"
const v = {options};
if (!v.text && !v.selector) throw new Error('give text or a selector to wait for');
const started = Date.now();
const present = () => v.selector ? !!document.querySelector(v.selector) : !!(document.body && document.body.innerText.includes(v.text));
for (;;) {{
  const now = present();
  if (v.gone ? !now : now) return {{ found: true, waited_ms: Date.now() - started }};
  if (Date.now() - started > v.timeout) return {{ found: false, waited_ms: Date.now() - started }};
  await new Promise((resolve) => setTimeout(resolve, 150));
}}
"#
    ))
}

pub const CONSOLE_READ: &str = r#"
return (window.__wakuConsole || []).slice(-100);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_evaluation_answers_its_value_or_its_exception() {
        assert_eq!(
            evaluation_value(r#"{"result":{"type":"string","value":"{\"ok\":true}"}}"#),
            Ok(r#"{"ok":true}"#.to_owned())
        );
        assert_eq!(
            evaluation_value(r#"{"result":{"type":"number","value":3}}"#),
            Ok("3".to_owned())
        );
        assert_eq!(
            evaluation_value(
                r#"{"result":{"type":"object"},"exceptionDetails":{"text":"Uncaught","exception":{"description":"ReferenceError: x is not defined"}}}"#
            ),
            Err("ReferenceError: x is not defined".to_owned())
        );
        assert!(evaluation_value("not json").is_err());
    }

    #[test]
    fn a_page_script_settles_to_a_value_or_an_error() {
        assert_eq!(
            page_result(r#"{"ok":true,"value":{"found":true}}"#),
            Ok(json!({"found": true}))
        );
        assert_eq!(page_result(r#"{"ok":true,"value":null}"#), Ok(Value::Null));
        assert_eq!(
            page_result(r#"{"ok":false,"error":"no element e3"}"#),
            Err("no element e3".to_owned())
        );
        assert!(page_result("garbage").is_err());
    }

    #[test]
    fn targets_are_json_literals_so_quotes_cannot_escape() {
        let literal = target_literal(&json!({"text": "Say \"hi\"</script>", "ref": ""}));
        let parsed: Value = serde_json::from_str(&literal).unwrap();
        assert_eq!(parsed["text"], "Say \"hi\"</script>");
        assert_eq!(parsed["ref"], Value::Null);
    }

    #[test]
    fn scripts_wrap_their_body_and_report_through_one_shape() {
        let script = click_script(&json!({"ref": "e7"}));
        assert!(script.starts_with("(async () => {"));
        assert!(script.contains(r#"__find({"ref":"e7","selector":null,"text":null})"#));
        assert!(script.contains("JSON.stringify({ ok: true"));
        assert!(script.contains("window.__wakuConsole"));
        let typed = type_script(&json!({"ref": "e2", "value": "a\nb", "submit": true}));
        assert!(typed.contains(r#""text":"a\nb""#), "{typed}");
        let waited = wait_script(&json!({"text": "Done", "timeout_ms": 999_999}));
        assert!(waited.contains(r#""timeout":60000"#), "{waited}");
    }

    #[test]
    fn screenshots_answer_their_data() {
        assert_eq!(screenshot_data(r#"{"data":"iVBOR"}"#), Ok("iVBOR".to_owned()));
        assert!(screenshot_data(r#"{}"#).is_err());
    }
}
