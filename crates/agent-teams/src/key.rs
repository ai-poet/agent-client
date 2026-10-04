// Translated from dsh-agent-teams (MIT, Copyright (c) 2026 程序员阿江(Relakkes)); see NOTICE.md.

//! Mailbox and directory keys.

use sha2::{Digest, Sha256};

/// Mailbox key of the captain, and the assignee name that means "the captain".
pub const CAPTAIN_KEY: &str = "captain";

/// Longest key emitted before truncating and appending a digest.
const MAX_KEY_LENGTH: usize = 48;

/// Short stable digest, used to keep otherwise-colliding keys distinct.
fn key_digest(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Fold a free-form name into a safe path/key segment.
///
/// Letters and digits survive, so CJK, Cyrillic and Greek names stay
/// distinct and readable; everything else — spaces, punctuation, path
/// separators, control characters — folds to `-`. A name with no letters or
/// digits at all gets a digest rather than a shared constant, and an
/// over-long name is truncated with a digest appended so names sharing a long
/// prefix stay distinct.
///
/// The reference also NFC-normalizes first; nothing here produces
/// decomposed names, so that step is left out rather than pulling in a
/// normalization table.
pub fn sanitize_key(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let mut cleaned = String::with_capacity(lowered.len());
    let mut in_gap = false;
    for ch in lowered.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            in_gap = false;
        } else if !in_gap {
            cleaned.push('-');
            in_gap = true;
        }
    }
    let cleaned = cleaned.trim_matches('-');
    if cleaned.is_empty() {
        return format!("k-{}", key_digest(name));
    }
    let points: Vec<char> = cleaned.chars().collect();
    if points.len() > MAX_KEY_LENGTH {
        let head: String = points[..MAX_KEY_LENGTH].iter().collect();
        return format!("{head}-{}", key_digest(name));
    }
    cleaned.to_owned()
}

/// Whether `name` designates the captain.
pub fn is_captain(name: &str) -> bool {
    sanitize_key(name) == CAPTAIN_KEY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_names_fold_punctuation_and_case() {
        assert_eq!(sanitize_key("  Backend Engineer!! "), "backend-engineer");
        assert_eq!(sanitize_key("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_key("--x--"), "x");
    }

    #[test]
    fn non_latin_names_stay_distinct() {
        assert_eq!(sanitize_key("前端 工程师"), "前端-工程师");
        assert_eq!(sanitize_key("Иван"), "иван");
        assert_ne!(sanitize_key("前端"), sanitize_key("后端"));
    }

    #[test]
    fn names_without_letters_get_a_digest() {
        let key = sanitize_key("🚀🚀");
        assert!(key.starts_with("k-"));
        assert_eq!(key.len(), 2 + 8);
        assert_ne!(key, sanitize_key("!!"));
    }

    #[test]
    fn long_names_are_truncated_with_a_digest() {
        let long = "a".repeat(60);
        let key = sanitize_key(&long);
        assert_eq!(key.chars().count(), MAX_KEY_LENGTH + 1 + 8);
        let other = format!("{}b", "a".repeat(59));
        assert_ne!(key, sanitize_key(&other));
    }

    #[test]
    fn captain_is_reserved() {
        assert!(is_captain(" Captain "));
        assert!(!is_captain("captain-2"));
    }
}
