//! What changed between one request's prompt and the next: a line diff of
//! the system prompt, and the tools added, removed or changed.
//!
//! After DeepSeek Harness's `ui-trajectory` Diff tab — MIT License,
//! Copyright (c) 2026 DeepSeek. See NOTICE.md.

use similar::{ChangeTag, TextDiff};

use crate::record::ToolSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineTag {
    Same,
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffLine {
    pub tag: LineTag,
    pub text: String,
}

/// The lines of `old` → `new`.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    TextDiff::from_lines(old, new)
        .iter_all_changes()
        .map(|change| DiffLine {
            tag: match change.tag() {
                ChangeTag::Equal => LineTag::Same,
                ChangeTag::Insert => LineTag::Added,
                ChangeTag::Delete => LineTag::Removed,
            },
            text: change.value().trim_end_matches('\n').to_owned(),
        })
        .collect()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same name, different description or schema.
    pub changed: Vec<String>,
}

impl ToolChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// The tools of `new` that `old` lacked, the reverse, and the ones that
/// changed.
pub fn tool_changes(old: &[ToolSpec], new: &[ToolSpec]) -> ToolChanges {
    let mut changes = ToolChanges::default();
    for tool in new {
        match old.iter().find(|candidate| candidate.name == tool.name) {
            None => changes.added.push(tool.name.clone()),
            Some(before) if before != tool => changes.changed.push(tool.name.clone()),
            Some(_) => {}
        }
    }
    for tool in old {
        if !new.iter().any(|candidate| candidate.name == tool.name) {
            changes.removed.push(tool.name.clone());
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_line_diff_marks_what_moved() {
        let diff = line_diff("a\nb\n", "a\nc\n");
        let tags: Vec<LineTag> = diff.iter().map(|line| line.tag).collect();
        assert_eq!(tags, [LineTag::Same, LineTag::Removed, LineTag::Added]);
        assert_eq!(diff[2].text, "c");
    }

    #[test]
    fn tools_are_compared_by_name_then_by_content() {
        let changes = tool_changes(
            &[tool("Read", "x"), tool("Bash", "y")],
            &[tool("Read", "x2"), tool("Grep", "z")],
        );
        assert_eq!(changes.added, ["Grep"]);
        assert_eq!(changes.removed, ["Bash"]);
        assert_eq!(changes.changed, ["Read"]);
        assert!(tool_changes(&[tool("A", "")], &[tool("A", "")]).is_empty());
    }
}
