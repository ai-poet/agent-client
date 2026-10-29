// FileRead tool: read files with optional line range, image support, PDF page ranges.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

pub struct FileReadTool;

#[derive(Debug, Deserialize)]
struct FileReadInput {
    file_path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for FileReadTool {
    // Gates itself: calls `ctx.check_permission_for_path` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_FILE_READ
    }

    fn description(&self) -> &str {
        "Reads a file from the local filesystem. You can access any file directly. \
         By default reads up to 2000 lines from the beginning. Results are returned \
         with line numbers starting at 1. This tool can read images (PNG, JPG) and \
         PDF files."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to read"
                },
                "offset": {
                    "type": "number",
                    "description": "The line number to start reading from (1-based). Only provide if the file is too large to read at once."
                },
                "limit": {
                    "type": "number",
                    "description": "The number of lines to read. Only provide if the file is too large to read at once."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: FileReadInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        let path = ctx.resolve_path(&params.file_path);
        debug!(path = %path.display(), "Reading file");

        // Permission check
        if let Err(e) = ctx.check_permission_for_path(
            self.name(),
            &format!("Read {}", path.display()),
            path.clone(),
            true,
        ) {
            return ToolResult::error(e.to_string());
        }

        // Check if file exists
        if !path.exists() {
            return ToolResult::error(format!("File not found: {}", path.display()));
        }

        // Check if it's a directory
        if path.is_dir() {
            return ToolResult::error(format!(
                "{} is a directory, not a file. Use Bash with `ls` to list directory contents.",
                path.display()
            ));
        }

        // Detect binary / image files by extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let image_exts = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico"];
        if image_exts.contains(&ext.as_str()) {
            return ToolResult::success(format!(
                "[Image file: {}. The image content has been captured for visual analysis.]",
                path.display()
            ));
        }

        if ext == "pdf" {
            return ToolResult::success(format!(
                "[PDF file: {}. Use the `pages` parameter to read specific page ranges.]",
                path.display()
            ));
        }

        // Read text file
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                // Might be binary
                if e.kind() == std::io::ErrorKind::InvalidData {
                    return ToolResult::error(format!(
                        "File appears to be binary and cannot be displayed as text: {}",
                        path.display()
                    ));
                }
                return ToolResult::error(format!("Failed to read file: {}", e));
            }
        };

        // Fork (Waku): the model has now seen this file (`read_state`).
        crate::read_state::record(&ctx.session_id, &path, content.as_bytes());

        if content.is_empty() {
            return ToolResult::success(format!("[File {} exists but is empty]", path.display()));
        }

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let offset = params.offset.unwrap_or(0);
        let limit = params.limit.unwrap_or(2000);

        // Convert 1-based offset to 0-based index
        let start = if offset > 0 { offset - 1 } else { 0 };
        // Fork departure (Waku): saturating. A model asking for "everything"
        // with `limit: usize::MAX` overflowed here — a panic in a debug build,
        // and in release an `end` below `start` that panicked on the slice.
        let end = start.saturating_add(limit).min(total_lines);

        if start >= total_lines {
            return ToolResult::error(format!(
                "Offset {} exceeds total line count {} in {}",
                offset,
                total_lines,
                path.display()
            ));
        }

        let mut output = String::new();
        let width = format!("{}", end).len();

        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            output.push_str(&format!("{:>width$}\t{}\n", line_num, line, width = width));
        }

        if end < total_lines {
            output.push_str(&format!(
                "\n... ({} more lines, {} total. Use offset/limit to read more.)\n",
                total_lines - end,
                total_lines
            ));
        }

        ToolResult::success(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allow_all_context;

    /// Fork (Waku): a limit big enough to overflow `start + limit` reads to
    /// the end instead of panicking.
    #[tokio::test]
    async fn a_huge_limit_reads_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        let res = FileReadTool
            .execute(
                json!({
                    "file_path": path.to_string_lossy(),
                    "offset": 2,
                    "limit": u64::MAX,
                }),
                &ctx,
            )
            .await;

        assert!(!res.is_error, "read failed: {}", res.content);
        assert!(res.content.contains("beta"), "{}", res.content);
        assert!(res.content.contains("gamma"), "{}", res.content);
        assert!(!res.content.contains("alpha"), "{}", res.content);
    }
}
