//! Fork addition (Waku): a reminder when the model keeps making the same call.
//!
//! Weaker models — and strong ones on a bad day — get stuck re-running one
//! command or re-reading one file with the same arguments, each time getting
//! the same answer, until the turn or the budget runs out. Nothing in the
//! loop noticed. This counts consecutive identical calls (tool name plus
//! arguments, key order ignored) and, at the 3rd, 5th and every 4th after the
//! 8th, appends a `<system-reminder>` to that call's result telling the model
//! to change course. It never blocks a call: a repeat can be legitimate
//! (polling a build), and the model is the one that knows.
//!
//! One guard lives for one run of the loop, which is one prompt from the
//! person: their next message starts the count again.

use serde_json::Value;

/// How much of the arguments the detailed reminder quotes.
const ARGUMENTS_PREVIEW_CHARS: usize = 500;

#[derive(Default)]
pub(crate) struct RepeatGuard {
    last: Option<String>,
    streak: u32,
}

impl RepeatGuard {
    /// Note a call, in the order the model made it. Returns the reminder to
    /// append to its result, when this call is one too many.
    pub(crate) fn observe(&mut self, name: &str, input: &Value) -> Option<String> {
        let key = format!("{name}\u{0}{}", canonical(input));
        if self.last.as_deref() == Some(key.as_str()) {
            self.streak += 1;
        } else {
            self.last = Some(key);
            self.streak = 1;
        }
        let n = self.streak;
        if n == 3 {
            Some(format!(
                "<system-reminder>You have now called {name} with exactly the same input \
                 {n} times in a row. Look at what the earlier calls returned before calling \
                 it again: if it is not getting you anywhere, take a different approach or \
                 say what is blocking you.</system-reminder>"
            ))
        } else if n == 5 || (n >= 8 && (n - 8) % 4 == 0) {
            Some(format!(
                "<system-reminder>{name} has been called with the same input {n} times in a \
                 row and keeps giving the same result. Do not call it again with these \
                 arguments: {}. Change the approach, look for different information, or stop \
                 and tell the user what is blocking you.</system-reminder>",
                preview(input)
            ))
        } else {
            None
        }
    }
}

/// `result` with the guard's reminder for this call appended, if it has one.
pub(crate) fn with_repeat_note(
    guard: &mut RepeatGuard,
    name: &str,
    input: &Value,
    result: String,
) -> String {
    match guard.observe(name, input) {
        Some(note) if result.is_empty() => note,
        Some(note) => format!("{result}\n\n{note}"),
        None => result,
    }
}

/// The arguments with object keys sorted, so `{"a":1,"b":2}` and
/// `{"b":2,"a":1}` are the same call. `serde_json` keeps insertion order in
/// this build (`preserve_order`), so its own rendering would not do.
fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            out.push('{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(value, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn preview(input: &Value) -> String {
    let text = canonical(input);
    match text.char_indices().nth(ARGUMENTS_PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reminders(guard: &mut RepeatGuard, name: &str, input: &Value, calls: u32) -> usize {
        (0..calls)
            .filter(|_| guard.observe(name, input).is_some())
            .count()
    }

    #[test]
    fn identical_calls_are_reminded_at_three_five_and_then_every_fourth() {
        let mut guard = RepeatGuard::default();
        let input = json!({"command": "npm test"});
        let mut fired = Vec::new();
        for n in 1..=16 {
            if guard.observe("Bash", &input).is_some() {
                fired.push(n);
            }
        }
        assert_eq!(fired, [3, 5, 8, 12, 16]);
    }

    #[test]
    fn key_order_does_not_make_a_different_call() {
        let mut guard = RepeatGuard::default();
        guard.observe("Read", &json!({"file_path": "a.rs", "offset": 1}));
        guard.observe("Read", &json!({"offset": 1, "file_path": "a.rs"}));
        let third = guard.observe("Read", &json!({"file_path": "a.rs", "offset": 1}));
        assert!(third.is_some(), "the same call three times, keys in any order");
    }

    #[test]
    fn a_different_call_starts_the_count_again() {
        let mut guard = RepeatGuard::default();
        let same = json!({"command": "ls"});
        assert_eq!(reminders(&mut guard, "Bash", &same, 2), 0);
        assert!(guard.observe("Bash", &json!({"command": "pwd"})).is_none());
        assert!(guard.observe("Bash", &same).is_none());
        assert!(guard.observe("Bash", &same).is_none());
        assert!(guard.observe("Bash", &same).is_some());
    }

    #[test]
    fn the_note_rides_on_the_result() {
        let mut guard = RepeatGuard::default();
        let input = json!({"pattern": "TODO"});
        for _ in 0..2 {
            assert_eq!(
                with_repeat_note(&mut guard, "Grep", &input, "no matches".into()),
                "no matches"
            );
        }
        let third = with_repeat_note(&mut guard, "Grep", &input, "no matches".into());
        assert!(third.starts_with("no matches\n\n<system-reminder>"), "{third}");
        assert!(third.contains("Grep"), "{third}");
    }

    #[test]
    fn the_preview_is_cut_on_a_character() {
        let long = "中".repeat(800);
        let text = preview(&json!({ "q": long }));
        assert!(text.ends_with('…'));
        assert!(text.chars().count() <= ARGUMENTS_PREVIEW_CHARS + 1);
    }
}
