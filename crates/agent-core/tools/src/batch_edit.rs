// BatchEdit tool: apply multiple file edits atomically.
//
// All edits are validated before any change is written.  If any pre-check
// fails the tool returns an error and leaves every file untouched.  If a write
// fails after some files have already been written, the tool attempts to
// restore those files from in-memory backups.
//
// Within one file, every old_string is matched against the file as originally
// read — before any edit of the batch is applied — and the matched regions
// must not overlap (Sailry `node-runtime files/edit.rs`). The replacements
// are spliced in one ascending pass, so untouched regions keep their exact
// bytes and line endings.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ops::Range;
use tracing::debug;

pub struct BatchEditTool;

/// Total byte budget for the excerpts appended to a batch result. The
/// per-file excerpts are already bounded (see `line_endings::edit_excerpt`);
/// this keeps a large multi-file batch from flooding the tool output.
const BATCH_EXCERPT_BUDGET: usize = 16 * 1024;

#[derive(Debug, Deserialize)]
struct SingleEdit {
    file_path: String,
    old_string: String,
    new_string: String,
}

#[derive(Debug, Deserialize)]
struct BatchEditInput {
    edits: Vec<SingleEdit>,
    #[serde(default)]
    description: Option<String>,
}

#[async_trait]
impl Tool for BatchEditTool {
    // Gates itself: calls `ctx.check_permission` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        agent_core::constants::TOOL_NAME_BATCH_EDIT
    }

    fn description(&self) -> &str {
        "Apply multiple file edits atomically. All edits are validated before any \
         file is modified. If any edit would fail (old_string not found or not \
         unique) the entire batch is rejected with no changes made. If a write \
         fails mid-batch, already-written files are rolled back."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "edits": {
                    "type": "array",
                    "description": "List of edits to apply atomically",
                    "items": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Absolute path to the file to modify"
                            },
                            "old_string": {
                                "type": "string",
                                "description": "Text to replace (must occur exactly once in the file)"
                            },
                            "new_string": {
                                "type": "string",
                                "description": "Replacement text"
                            }
                        },
                        "required": ["file_path", "old_string", "new_string"]
                    }
                },
                "description": {
                    "type": "string",
                    "description": "Optional human-readable description of what this batch edit does"
                }
            },
            "required": ["edits"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: BatchEditInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        if params.edits.is_empty() {
            return ToolResult::error("edits array must not be empty".to_string());
        }

        // Permission check (one check covers the whole batch).
        let description = params.description.as_deref().unwrap_or("batch file edits");
        if let Err(e) =
            ctx.check_permission(self.name(), &format!("BatchEdit: {}", description), false)
        {
            return ToolResult::error(e.to_string());
        }

        // Group the edits by resolved path, keeping the original edit indices
        // for error messages and the file's first-appearance order.
        let mut file_order: Vec<String> = Vec::new();
        let mut by_file: HashMap<String, Vec<(usize, &SingleEdit)>> = HashMap::new();
        for (index, edit) in params.edits.iter().enumerate() {
            let path = ctx.resolve_path(&edit.file_path);
            let key = path.display().to_string();
            by_file
                .entry(key.clone())
                .or_insert_with(|| {
                    file_order.push(key.clone());
                    Vec::new()
                })
                .push((index, edit));
        }

        // ----------------------------------------------------------------
        // Phase 1: read every file once and validate the whole batch before
        // any write. Each file's edits all match against its original
        // content, in one pass (`replace_many_preserving_eol`).
        // ----------------------------------------------------------------

        // (resolved_path, original_content, new_content, replaced_ranges)
        let mut prepared: Vec<(String, String, String, Vec<Range<usize>>)> =
            Vec::with_capacity(file_order.len());
        let mut pre_check_errors: Vec<String> = Vec::new();

        for key in &file_order {
            let file_edits = &by_file[key];
            let first_index = file_edits[0].0;

            // Basic per-edit validation, reported with the original indices.
            let mut file_is_valid = true;
            for (original_index, edit) in file_edits {
                if edit.old_string.is_empty() {
                    pre_check_errors.push(format!(
                        "Edit {}: old_string must not be empty",
                        original_index
                    ));
                    file_is_valid = false;
                }
            }
            if !file_is_valid {
                continue;
            }

            let path = std::path::Path::new(key);
            debug!(path = %path.display(), "BatchEdit pre-check");

            // Fork (Waku): as for Edit (`read_state`), once per file.
            if let Some(reason) = crate::read_state::refusal(&ctx.session_id, path) {
                pre_check_errors.push(format!("Edit {}: {}", first_index, reason));
                continue;
            }
            let original = match tokio::fs::read_to_string(path).await {
                Ok(c) => c,
                Err(e) => {
                    pre_check_errors
                        .push(format!("Edit {}: cannot read {}: {}", first_index, key, e));
                    continue;
                }
            };

            // Detect the current content's dominant line ending BEFORE
            // editing so it survives the write (#225). Matching runs on an
            // LF-normalized view while the spliced-in bytes keep every
            // untouched region's exact endings.
            let eol = crate::line_endings::LineEnding::detect(&original);
            let specs: Vec<crate::line_endings::EditReplacement> = file_edits
                .iter()
                .map(|(_, edit)| crate::line_endings::EditReplacement {
                    old: &edit.old_string,
                    new: &edit.new_string,
                })
                .collect();
            match crate::line_endings::replace_many_preserving_eol(&original, &specs, eol) {
                Ok((new_content, _count, ranges)) => {
                    prepared.push((key.clone(), original, new_content, ranges));
                }
                Err(error) => pre_check_errors.push(edit_error_message(error, file_edits, key)),
            }
        }

        if !pre_check_errors.is_empty() {
            return ToolResult::error(format!(
                "BatchEdit aborted — {} validation error(s):\n{}",
                pre_check_errors.len(),
                pre_check_errors.join("\n")
            ));
        }

        let edit_count = params.edits.len();
        let file_count = prepared.len();

        // ----------------------------------------------------------------
        // Phase 2: write all files; roll back on any failure
        // ----------------------------------------------------------------

        for (i, (path_str, original, new_content, _)) in prepared.iter().enumerate() {
            let path = std::path::Path::new(path_str);
            match crate::write_atomic(path, new_content.as_bytes()).await {
                Ok(()) => {
                    ctx.record_file_change(
                        path.to_path_buf(),
                        original.as_bytes(),
                        new_content.as_bytes(),
                        self.name(),
                    );
                }
                Err(e) => {
                    // Attempt rollback of already-written files.
                    let mut rollback_errors: Vec<String> = Vec::new();

                    // rollback in reverse order to preserve original file state
                    for (rb_path, rb_original, rb_new_content, _) in prepared[0..i].iter().rev() {
                        if let Err(re) = crate::write_atomic(
                            std::path::Path::new(rb_path),
                            rb_original.as_bytes(),
                        )
                        .await
                        {
                            rollback_errors.push(format!("  rollback {}: {}", rb_path, re));
                        } else {
                            let rb_path = std::path::Path::new(rb_path);
                            ctx.record_file_change(
                                rb_path.to_path_buf(),
                                rb_new_content.as_bytes(),
                                rb_original.as_bytes(),
                                self.name(),
                            );
                        }
                    }

                    let mut msg = format!(
                        "BatchEdit failed while writing {} ({}). Rolled back {} file(s).",
                        path_str, e, i
                    );
                    if !rollback_errors.is_empty() {
                        msg.push_str(&format!(
                            "\nRollback errors:\n{}",
                            rollback_errors.join("\n")
                        ));
                    }
                    return ToolResult::error(msg);
                }
            }
        }

        // ----------------------------------------------------------------
        // Build success response
        // ----------------------------------------------------------------

        let mut response = format!(
            "BatchEdit applied {} edit{} across {} file{}.",
            edit_count,
            if edit_count != 1 { "s" } else { "" },
            file_count,
            if file_count != 1 { "s" } else { "" },
        );

        // Bounded edit excerpts as a plain-text aid: the caller can verify
        // the batch without reading every file back. The whole batch shares
        // one budget, so a large batch cannot flood the tool output.
        let mut budget = BATCH_EXCERPT_BUDGET;
        for (path_str, _, new_content, ranges) in prepared.iter() {
            if budget == 0 {
                response.push_str("\n[Further edit excerpts omitted]\n");
                break;
            }
            let mut text = crate::line_endings::edit_excerpt(new_content, ranges).render_text();
            if text.len() > budget {
                truncate_at_boundary(&mut text, budget);
                text.push_str("[Excerpt truncated]\n");
            }
            budget -= text.len();
            response.push_str(&format!("\n--- {} ---\n{}", path_str, text));
        }

        let files = prepared
            .iter()
            .map(|(path, _, content, ranges)| {
                json!({
                    "path":path,
                    "destination":null,
                    "kind":"modified",
                    "status":"applied",
                    "revision":blake3::hash(content.as_bytes()).to_hex().to_string(),
                    "size":content.len() as u64,
                    "excerpt":crate::line_endings::edit_excerpt(content, ranges).to_value(),
                    "error":null,
                })
            })
            .collect::<Vec<_>>();
        ToolResult::success(response)
            .with_metadata(json!({
                "edits_applied": edit_count,
                "files_modified": file_count,
                "files": prepared.iter().map(|(p, _, _, _)| p).collect::<Vec<_>>(),
            }))
            .with_structured_content(json!({
                "kind":"file_changes",
                "data":{"status":"applied","files":files}
            }))
    }
}

/// Map a batch replacement error onto the batch's message style, translating
/// the per-file edit position back to the original edit index.
fn edit_error_message(
    error: crate::line_endings::ReplaceManyError,
    file_edits: &[(usize, &SingleEdit)],
    path: &str,
) -> String {
    let original_index = |local: usize| file_edits[local].0;
    match error {
        crate::line_endings::ReplaceManyError::NotFound(local) => format!(
            "Edit {}: old_string not found in {}. Make sure the string matches exactly, \
             including whitespace and indentation.",
            original_index(local),
            path
        ),
        crate::line_endings::ReplaceManyError::NotUnique(local, count) => format!(
            "Edit {}: old_string appears {} times in {} (must be unique)",
            original_index(local),
            count,
            path
        ),
        crate::line_endings::ReplaceManyError::Overlap(a, b) => format!(
            "Edit {} and Edit {}: old_strings match overlapping regions in {}; \
             make them disjoint so the batch can be applied",
            original_index(a),
            original_index(b),
            path
        ),
    }
}

/// Truncate `text` to at most `limit` bytes without splitting a UTF-8 char.
fn truncate_at_boundary(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allow_all_context;

    /// #225: a multi-edit batch on a CRLF file keeps CRLF throughout; only the
    /// edited lines change.
    #[tokio::test]
    async fn batch_edit_crlf_file_preserves_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crlf.txt");
        let original = "alpha\r\nbeta\r\ngamma\r\n";
        std::fs::write(&path, original).unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": path.to_string_lossy(), "old_string": "alpha", "new_string": "ALPHA" },
                        { "file_path": path.to_string_lossy(), "old_string": "gamma", "new_string": "GAMMA" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "batch edit failed: {}", res.content);

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "ALPHA\r\nbeta\r\nGAMMA\r\n");
        assert_eq!(after.matches('\n').count(), after.matches("\r\n").count());
    }

    /// #226: BatchEdit writes (and its rollback path) go through `write_atomic`.
    /// A successful multi-file batch must land the right content in every file
    /// and leave no `.claurst-tmp-*` scratch file behind. Because each file is
    /// swapped in atomically via rename, a mid-write crash can never leave one
    /// of these files partially written — so the rollback only ever has to
    /// restore fully-written files, never repair a torn one.
    #[tokio::test]
    async fn batch_edit_writes_atomically_no_tmp_left() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "one\n").unwrap();
        std::fs::write(&b, "two\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &a);
        crate::read_state::mark_seen(&ctx.session_id, &b);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": a.to_string_lossy(), "old_string": "one", "new_string": "ONE" },
                        { "file_path": b.to_string_lossy(), "old_string": "two", "new_string": "TWO" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "batch edit failed: {}", res.content);

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "ONE\n");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "TWO\n");
        let tmp_left = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".claurst-tmp-"));
        assert!(!tmp_left, "atomic write must not leave a temp file behind");
    }

    /// Every old_string is matched against the file as originally read. An
    /// edit whose text only exists inside another edit's replacement does not
    /// match (no sequential cascading), and the whole batch is rejected
    /// without touching the file.
    #[tokio::test]
    async fn edits_match_the_original_content_not_previous_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chain.txt");
        std::fs::write(&path, "a\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": path.to_string_lossy(), "old_string": "a", "new_string": "b" },
                        { "file_path": path.to_string_lossy(), "old_string": "b", "new_string": "c" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(res.is_error, "a chained match must be rejected");
        assert!(
            res.content.contains("Edit 1") && res.content.contains("not found"),
            "{}",
            res.content
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\n");
    }

    /// Overlapping old_strings are detected and reported with both edit
    /// indices; nothing is written.
    #[tokio::test]
    async fn overlapping_edits_are_rejected_with_both_indices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlap.txt");
        std::fs::write(&path, "alpha beta gamma\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": path.to_string_lossy(), "old_string": "beta", "new_string": "B" },
                        { "file_path": path.to_string_lossy(), "old_string": "beta ga", "new_string": "X" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(res.is_error, "overlapping matches must be rejected");
        assert!(
            res.content.contains("Edit 0") && res.content.contains("Edit 1"),
            "{}",
            res.content
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha beta gamma\n"
        );
    }

    /// Adjacent (touching but disjoint) matches are fine and splice in order.
    #[tokio::test]
    async fn adjacent_edits_apply_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adjacent.txt");
        std::fs::write(&path, "abcdef\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": path.to_string_lossy(), "old_string": "abc", "new_string": "ABC" },
                        { "file_path": path.to_string_lossy(), "old_string": "def", "new_string": "DEF" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "batch edit failed: {}", res.content);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ABCDEF\n");
    }

    /// A successful batch appends a bounded excerpt of each file so the model
    /// can verify the edits without reading the files back.
    #[tokio::test]
    async fn success_response_includes_bounded_excerpts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippet.txt");
        std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\nsix\nseven\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = BatchEditTool
            .execute(
                json!({
                    "edits": [
                        { "file_path": path.to_string_lossy(), "old_string": "four", "new_string": "FOUR" }
                    ]
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "batch edit failed: {}", res.content);

        // The edit is on line 4; three lines of context on each side show
        // lines 1..=7, numbered.
        assert!(
            res.content
                .contains("BatchEdit applied 1 edit across 1 file."),
            "{}",
            res.content
        );
        assert!(res.content.contains("1\tone"), "{}", res.content);
        assert!(res.content.contains("4\tFOUR"), "{}", res.content);
        assert!(res.content.contains("7\tseven"), "{}", res.content);
        assert!(!res.content.contains("8\t"), "{}", res.content);
    }
}
