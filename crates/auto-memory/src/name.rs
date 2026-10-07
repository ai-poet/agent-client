// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! Memory names and project directory keys.
//!
//! A name is the file name, so it is squeezed to `[a-z0-9-]`: no path can be
//! smuggled through it. A project key turns a workspace path into one safe
//! directory name.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Longest memory name.
pub const NAME_MAX: usize = 64;
/// Longest project key slug before it is shortened with a digest, so that
/// `<config>/auto-memory/projects/<key>/<name>.md` stays under Windows'
/// 260-character path limit.
const PROJECT_SLUG_MAX: usize = 96;

/// Names that would collide with something on disk: `memory` is
/// `MEMORY.md` on a case-insensitive filesystem (the write would succeed and
/// destroy the index), and Windows' device names cannot be file names.
const RESERVED: &[&str] = &[
    "memory", "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Squeeze a name to kebab-case `[a-z0-9-]`, at most [`NAME_MAX`] long.
/// Fails when nothing is left or the result is reserved; the message is
/// written for the model, which picks the name.
pub fn normalize_name(input: &str) -> Result<String, String> {
    let mut slug = String::new();
    let mut dash = false;
    for c in input.trim().to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
    }
    if slug.len() > NAME_MAX {
        slug.truncate(NAME_MAX);
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        return Err(format!(
            "memory name must normalize to kebab-case [a-z0-9-] (got: {})",
            serde_json::to_string(input).unwrap_or_default()
        ));
    }
    if RESERVED.contains(&slug.as_str()) {
        return Err(format!(
            "memory name \"{slug}\" is reserved (it collides with MEMORY.md or a Windows device name); pick a more specific name"
        ));
    }
    Ok(slug)
}

/// The directory name for a workspace: `/`, `\` and `:` fold to `-`, other
/// characters outside `[A-Za-z0-9._-]` become `~XXXX` (one per UTF-16 unit),
/// the whole wrapped as `--slug--` — the reference's algorithm, which is
/// also dsh's session directory naming. On top of it: trailing separators
/// are ignored, a Windows drive letter is lowercased (`C:\x` and `c:\x` are
/// one workspace), and a long slug is cut with a digest of the full path.
pub fn project_key(cwd: &Path) -> String {
    project_key_for(&cwd.to_string_lossy(), cfg!(windows))
}

/// [`project_key`] for a path spelled as text, `windows` choosing the
/// drive-letter rule.
pub fn project_key_for(path: &str, windows: bool) -> String {
    let mut text = path.to_owned();
    while text.len() > 1 && (text.ends_with('/') || text.ends_with('\\')) {
        text.pop();
    }
    if windows {
        let mut chars = text.chars();
        if let (Some(drive), Some(':')) = (chars.next(), chars.next())
            && drive.is_ascii_alphabetic()
        {
            text.replace_range(0..1, &drive.to_ascii_lowercase().to_string());
        }
    }

    let mut readable = String::new();
    let mut separator_run = false;
    for c in text.chars() {
        if matches!(c, '/' | '\\' | ':') {
            if !separator_run {
                readable.push('-');
            }
            separator_run = true;
            continue;
        }
        separator_run = false;
        if c != '~' && (c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            readable.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                readable.push_str(&format!("~{unit:04X}"));
            }
        }
    }
    let mut slug = readable.trim_start_matches('-').to_owned();
    if slug.is_empty() {
        slug = "root".to_owned();
    }
    if slug.len() > PROJECT_SLUG_MAX {
        // Every character here is ASCII, so any byte offset is a boundary.
        slug.truncate(PROJECT_SLUG_MAX);
        let digest = Sha256::digest(text.as_bytes());
        let hex: String = digest
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        slug = format!("{slug}-{hex}");
    }
    format!("--{slug}--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_squeezed_to_kebab_case() {
        assert_eq!(normalize_name("User Prefers Python").unwrap(), "user-prefers-python");
        assert_eq!(normalize_name("  --a__b--  ").unwrap(), "a-b");
        assert_eq!(normalize_name("压测 PostgreSQL 2026").unwrap(), "postgresql-2026");
        assert!(normalize_name("中文").is_err());
        assert!(normalize_name("").is_err());
    }

    #[test]
    fn a_cut_at_the_limit_never_ends_in_a_dash() {
        let long = format!("{}-b", "a".repeat(63));
        let name = normalize_name(&long).unwrap();
        assert_eq!(name, "a".repeat(63));
        assert!(name.len() <= NAME_MAX);
    }

    #[test]
    fn reserved_names_are_refused_in_any_case() {
        for reserved in ["memory", "MEMORY", "Memory", "con", "NUL", "com1", "lpt9"] {
            assert!(normalize_name(reserved).is_err(), "{reserved}");
        }
        assert_eq!(normalize_name("memory-notes").unwrap(), "memory-notes");
    }

    #[test]
    fn project_keys_follow_the_reference_naming() {
        assert_eq!(project_key_for("/home/ada/app", false), "--home-ada-app--");
        assert_eq!(project_key_for("D:\\a\\b", false), "--D-a-b--");
        assert_eq!(project_key_for("/srv/~x", false), "--srv-~007Ex--");
        assert_eq!(project_key_for("/", false), "--root--");
        assert_eq!(project_key_for("/中", false), "--~4E2D--");
    }

    #[test]
    fn windows_drive_letters_and_trailing_separators_do_not_split_a_workspace() {
        assert_eq!(
            project_key_for("C:\\Projects\\app\\", true),
            project_key_for("c:\\Projects\\app", true)
        );
        assert_eq!(project_key_for("C:\\Projects\\app", true), "--c-Projects-app--");
    }

    #[test]
    fn long_paths_are_cut_with_a_digest() {
        let base = format!("/{}", "segment/".repeat(30));
        let key = project_key_for(&base, false);
        assert!(key.len() <= 2 + PROJECT_SLUG_MAX + 9 + 2);
        let other = project_key_for(&format!("{base}x"), false);
        assert_ne!(key, other);
    }
}
