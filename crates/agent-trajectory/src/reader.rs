//! Reading segments back, a little at a time.
//!
//! The desktop reads a segment as it grows: from the byte it stopped at,
//! complete lines only — a final line without its newline is still being
//! written and is read next time. A line that does not parse is skipped.

use crate::record::Record;

/// The records in `text` from byte `from` on, and how far the complete lines
/// reach (the next call's `from`).
pub fn parse_lines(text: &str, from: usize) -> (Vec<Record>, usize) {
    let mut from = from.min(text.len());
    while !text.is_char_boundary(from) {
        from -= 1;
    }
    let rest = &text[from..];
    let complete = rest.rfind('\n').map_or(0, |index| index + 1);
    let records = rest[..complete]
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    (records, from + complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unfinished_line_waits_for_the_next_read() {
        let full = r#"{"type":"rewind","keep_turns":1,"at":2}"#;
        let text = format!("{full}\n{}", &full[..10]);
        let (records, consumed) = parse_lines(&text, 0);
        assert_eq!(records.len(), 1);
        assert_eq!(consumed, full.len() + 1);
        let text = format!("{full}\n{full}\n");
        let (records, consumed) = parse_lines(&text, consumed);
        assert_eq!(records.len(), 1);
        assert_eq!(consumed, text.len());
    }

    #[test]
    fn a_broken_line_is_skipped() {
        let text = "not json\n{\"type\":\"rewind\",\"keep_turns\":0,\"at\":1}\n";
        assert_eq!(parse_lines(text, 0).0.len(), 1);
    }
}
