//! The YAML frontmatter of a memory file — the subset memories use.
//!
//! No YAML library is in the workspace, and a memory needs very little:
//! `key: value` lines with plain, single- or double-quoted strings,
//! integers and booleans, folded continuation lines, and one level of
//! nesting (Claude Code keeps the type under `metadata:`). Anything else —
//! block scalars, flow collections, deeper nesting — makes that key
//! unreadable; a record whose name or description is unreadable is skipped,
//! never fatal. Writing quotes every free-text string as JSON, which is
//! valid YAML.

use std::collections::BTreeMap;

/// One frontmatter value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    Map(BTreeMap<String, Value>),
    /// Present but in a form this parser does not read.
    Unreadable,
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(map) => Some(map),
            _ => None,
        }
    }
}

/// Parsed frontmatter and the body after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub fields: BTreeMap<String, Value>,
    pub body: String,
}

/// Split `raw` into frontmatter and body. `None` when the text does not
/// open with a `---` line closed by another (a leading BOM and CRLF line
/// ends are fine).
pub fn parse(raw: &str) -> Option<Document> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let mut lines = raw.split_inclusive('\n');
    let first = lines.next()?;
    if trim_line_end(first) != "---" {
        return None;
    }
    let mut offset = first.len();
    let mut block = Vec::new();
    let mut closed = false;
    for line in lines {
        offset += line.len();
        if trim_line_end(line) == "---" {
            closed = true;
            break;
        }
        block.push(trim_line_end(line));
    }
    if !closed {
        return None;
    }
    Some(Document {
        fields: parse_block(&block),
        body: raw[offset..].to_owned(),
    })
}

fn trim_line_end(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn parse_block(lines: &[&str]) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    // The last top-level key, and whether its value was a plain scalar that
    // a continuation line may extend.
    let mut last: Option<(String, bool)> = None;
    let mut nested: Nested = None;

    for &line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        if indent > 0 {
            // A nested map entry under a key whose own value was empty.
            if let Some((_, base, map)) = nested.as_mut() {
                if *base == 0 || indent == *base {
                    *base = indent;
                    if let Some((key, value)) = split_entry(trimmed) {
                        map.insert(key, parse_scalar(value));
                    }
                    continue;
                }
                // Deeper than one level: the whole map is out of reach.
                if let Some((key, _, _)) = nested.take() {
                    fields.insert(key, Value::Unreadable);
                }
                continue;
            }
            // Otherwise a folded continuation of the previous plain scalar.
            if let Some((key, true)) = &last
                && let Some(Value::Text(text)) = fields.get_mut(key)
            {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(trimmed);
            }
            continue;
        }

        finish_nested(&mut nested, &mut fields);
        let Some((key, value)) = split_entry(trimmed) else {
            last = None;
            continue;
        };
        if value.is_empty() {
            nested = Some((key.clone(), 0, BTreeMap::new()));
            last = None;
            continue;
        }
        let plain = !matches!(value.chars().next(), Some('"' | '\''));
        let parsed = parse_scalar(value);
        let extendable = plain && matches!(parsed, Value::Text(_));
        fields.insert(key.clone(), parsed);
        last = Some((key, extendable));
    }
    finish_nested(&mut nested, &mut fields);
    fields
}

type Nested = Option<(String, usize, BTreeMap<String, Value>)>;

/// Close an open nested map. A key with an empty value and nothing under it
/// is an empty string.
fn finish_nested(nested: &mut Nested, fields: &mut BTreeMap<String, Value>) {
    if let Some((key, _, map)) = nested.take() {
        let value = if map.is_empty() {
            Value::Text(String::new())
        } else {
            Value::Map(map)
        };
        fields.insert(key, value);
    }
}

fn split_entry(line: &str) -> Option<(String, &str)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    let rest = &line[colon + 1..];
    if key.is_empty() || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some((key.to_owned(), rest.trim()))
}

fn parse_scalar(raw: &str) -> Value {
    let raw = raw.trim();
    match raw.chars().next() {
        Some('"') => parse_double_quoted(raw),
        Some('\'') => parse_single_quoted(raw),
        Some('|' | '>' | '[' | '{' | '&' | '*' | '!') => Value::Unreadable,
        None => Value::Text(String::new()),
        Some(_) => {
            let plain = strip_comment(raw);
            match plain {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "~" | "null" => Value::Text(String::new()),
                _ => match plain.parse::<i64>() {
                    Ok(number) => Value::Int(number),
                    Err(_) => Value::Text(plain.to_owned()),
                },
            }
        }
    }
}

fn strip_comment(raw: &str) -> &str {
    match raw.find(" #") {
        Some(at) => raw[..at].trim_end(),
        None => raw,
    }
}

fn parse_double_quoted(raw: &str) -> Value {
    let mut escaped = false;
    let mut end = None;
    for (index, c) in raw.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            end = Some(index);
            break;
        }
    }
    let Some(end) = end else {
        return Value::Unreadable;
    };
    let rest = raw[end + 1..].trim();
    if !(rest.is_empty() || rest.starts_with('#')) {
        return Value::Unreadable;
    }
    // JSON escapes are a subset of YAML's; a YAML-only escape makes the
    // value unreadable rather than wrong.
    match serde_json::from_str::<String>(&raw[..=end]) {
        Ok(text) => Value::Text(text),
        Err(_) => Value::Unreadable,
    }
}

fn parse_single_quoted(raw: &str) -> Value {
    let inner = &raw[1..];
    let mut text = String::new();
    let mut chars = inner.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        if c == '\'' {
            if let Some((_, '\'')) = chars.peek() {
                text.push('\'');
                chars.next();
                continue;
            }
            let rest = inner[index + 1..].trim();
            return if rest.is_empty() || rest.starts_with('#') {
                Value::Text(text)
            } else {
                Value::Unreadable
            };
        }
        text.push(c);
    }
    Value::Unreadable
}

/// A string as a YAML scalar: plain when it is a simple token, JSON-quoted
/// otherwise.
pub fn scalar(text: &str) -> String {
    let simple = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !matches!(text, "true" | "false" | "null" | "~")
        && text.parse::<i64>().is_err();
    if simple {
        text.to_owned()
    } else {
        quoted(text)
    }
}

/// A string as a JSON-quoted YAML scalar.
pub fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_quoted_and_typed_values() {
        let doc = parse(
            "---\nname: a-b\ndescription: \"says: \\\"hi\\\" — ok\"\ntitle: 'it''s'\nreads: 3\npinned: true\n---\n\nBody line\n",
        )
        .unwrap();
        assert_eq!(doc.fields["name"].as_text(), Some("a-b"));
        assert_eq!(doc.fields["description"].as_text(), Some("says: \"hi\" — ok"));
        assert_eq!(doc.fields["title"].as_text(), Some("it's"));
        assert_eq!(doc.fields["reads"].as_int(), Some(3));
        assert_eq!(doc.fields["pinned"].as_bool(), Some(true));
        assert_eq!(doc.body, "\nBody line\n");
    }

    #[test]
    fn tolerates_crlf_bom_comments_and_continuations() {
        let doc = parse("\u{feff}---\r\nname: x # note\r\ndescription: first\r\n  second\r\n---\r\nbody").unwrap();
        assert_eq!(doc.fields["name"].as_text(), Some("x"));
        assert_eq!(doc.fields["description"].as_text(), Some("first second"));
        assert_eq!(doc.body, "body");
    }

    #[test]
    fn reads_one_level_of_nesting() {
        let doc = parse(
            "---\nname: a\nmetadata:\n  node_type: memory\n  type: project\n  modified: 2026-09-26T16:00:24.015Z\n---\nx",
        )
        .unwrap();
        let metadata = doc.fields["metadata"].as_map().unwrap();
        assert_eq!(metadata["type"].as_text(), Some("project"));
        assert_eq!(metadata["modified"].as_text(), Some("2026-09-26T16:00:24.015Z"));
    }

    #[test]
    fn deeper_or_block_values_are_unreadable_not_fatal() {
        let doc = parse("---\nbody: |\n  text\nmeta:\n  a:\n    b: 1\nname: ok\n---\n").unwrap();
        assert_eq!(doc.fields["body"], Value::Unreadable);
        assert_eq!(doc.fields["name"].as_text(), Some("ok"));
    }

    #[test]
    fn unclosed_or_missing_frontmatter_is_none() {
        assert!(parse("no frontmatter").is_none());
        assert!(parse("---\nname: x\n").is_none());
    }

    #[test]
    fn scalars_round_trip() {
        for text in ["plain-token", "has: colon", "\"quotes\"", "true", "42", "", "多语言 — ok"] {
            let line = format!("---\nk: {}\n---\n", scalar(text));
            let doc = parse(&line).unwrap();
            assert_eq!(doc.fields["k"].as_text(), Some(text), "{text}");
        }
    }
}
