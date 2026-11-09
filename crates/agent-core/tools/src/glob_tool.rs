// Glob tool: fast file pattern matching.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::debug;

pub struct GlobTool;

#[derive(Debug, Deserialize)]
struct GlobInput {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for GlobTool {
    // Gates itself: calls `ctx.check_permission_for_path` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        agent_core::constants::TOOL_NAME_GLOB
    }

    fn description(&self) -> &str {
        "Fast file pattern matching tool that works with any codebase size. \
         Supports glob patterns like \"**/*.rs\" or \"src/**/*.ts\". Returns \
         matching file paths sorted by modification time. Use this tool when \
         you need to find files by name patterns."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. Defaults to working directory."
                },
                "offset": {"type":"number","description":"Skip this many matching files"},
                "limit": {"type":"number","description":"Maximum files in this page"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: GlobInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        let base_dir = params
            .path
            .as_ref()
            .map(|p| ctx.resolve_path(p))
            .unwrap_or_else(|| ctx.working_dir.clone());

        if let Err(e) = ctx.check_permission_for_path(
            self.name(),
            &format!("Glob {} in {}", params.pattern, base_dir.display()),
            base_dir.clone(),
            true,
        ) {
            return ToolResult::error(e.to_string());
        }

        debug!(pattern = %params.pattern, dir = %base_dir.display(), "Running glob");

        if !base_dir.exists() || !base_dir.is_dir() {
            return ToolResult::error(format!("Directory not found: {}", base_dir.display()));
        }

        // Build the full glob pattern
        let full_pattern = base_dir.join(&params.pattern);
        let pattern_str = full_pattern.to_string_lossy().to_string();

        // On Windows, normalize backslashes to forward slashes for the glob crate
        let pattern_str = pattern_str.replace('\\', "/");

        let entries: Vec<PathBuf> = match glob::glob(&pattern_str) {
            Ok(paths) => {
                let mut out = Vec::new();
                for path in paths.filter_map(|p| p.ok()) {
                    if !ctx.path_is_within_workspace(&path) {
                        if let Err(e) = ctx.check_permission_for_path(
                            self.name(),
                            &format!("Glob result {}", path.display()),
                            path.clone(),
                            true,
                        ) {
                            return ToolResult::error(e.to_string());
                        }
                    }
                    out.push(path);
                }
                out
            }
            Err(e) => {
                return ToolResult::error(format!("Invalid glob pattern: {}", e));
            }
        };

        if entries.is_empty() {
            return ToolResult::success(format!(
                "No files matched pattern \"{}\" in {}",
                params.pattern,
                base_dir.display()
            ))
            .with_structured_content(glob_results(Vec::new(), 0, 0, None, false));
        }

        // Sort by modification time (most recent first) — fall back to name sort
        let mut entries_with_time: Vec<(PathBuf, std::time::SystemTime)> = entries
            .into_iter()
            .filter_map(|p| {
                let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
                Some((p, mtime))
            })
            .collect();

        entries_with_time.sort_by_key(|b| std::cmp::Reverse(b.1));

        let total = entries_with_time.len();
        let offset = params.offset.unwrap_or(0).min(total);
        let limit = params.limit.unwrap_or(250).max(1);
        let end = offset.saturating_add(limit).min(total);
        let next_offset = (end < total).then_some(end);
        let truncated = next_offset.is_some() || offset > 0;
        let paths = entries_with_time[offset..end]
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>();

        let mut output = String::new();
        for path in &paths {
            output.push_str(path);
            output.push('\n');
        }

        if truncated {
            output.push_str(&format!(
                "\n... and {} more files (showing first {})\n",
                total - end,
                paths.len(),
            ));
        }

        ToolResult::success(output).with_structured_content(glob_results(
            paths,
            total,
            offset,
            next_offset,
            truncated,
        ))
    }
}

fn glob_results(
    paths: Vec<String>,
    scanned: usize,
    offset: usize,
    next_offset: Option<usize>,
    truncated: bool,
) -> Value {
    json!({
        "kind":"search_results",
        "data":{
            "mode":"files",
            "paths":paths,
            "scanned_files":scanned,
            "skipped":0,
            "truncated":truncated,
            "partial":false,
            "offset":offset,
            "next_offset":next_offset,
            "diagnostics":[],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allow_all_context;

    #[tokio::test]
    async fn glob_results_share_the_search_cursor() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        let context = allow_all_context(directory.path().to_path_buf());
        let mut next = Vec::new();
        for offset in [0, 1, 2] {
            let result = GlobTool
                .execute(
                    json!({"path":directory.path(),"pattern":"*.txt","offset":offset,"limit":1}),
                    &context,
                )
                .await;
            let data = result.structured_content.unwrap()["data"].clone();
            assert_eq!(data["paths"].as_array().unwrap().len(), 1);
            next.push(data["next_offset"].clone());
        }
        assert_eq!(next, [json!(1), json!(2), Value::Null]);
    }
}
