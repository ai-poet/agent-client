// FileRead tool: read files with optional line range, image support, PDF page ranges.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use agent_core::types::ImageSource;
use async_trait::async_trait;
use base64::Engine as _;
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

/// The `FileLines` shape Sailry protocol defines (path, lines, size, revision,
/// offset, next_offset, truncated), built with plain serde_json — this crate
/// must not depend on sailry-protocol. `lines` holds `{number, text}` rows
/// with one-based numbers; `revision` is the full-content BLAKE3 hash, so it
/// identifies the whole file even though `lines` carries only one page.
fn file_lines_value(
    path: &std::path::Path,
    lines: Vec<Value>,
    size: u64,
    revision: String,
    offset: usize,
    next_offset: Option<usize>,
    truncated: bool,
) -> Value {
    json!({
        "kind": "file_lines",
        "data": {
            "path": path.display().to_string(),
            "lines": lines,
            "size": size,
            "revision": revision,
            "offset": offset,
            "next_offset": next_offset,
            "truncated": truncated,
        }
    })
}

#[async_trait]
impl Tool for FileReadTool {
    // Gates itself: calls `ctx.check_permission_for_path` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        agent_core::constants::TOOL_NAME_FILE_READ
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
            let text = format!(
                "[Image file: {}. The image content has been captured for visual analysis.]",
                path.display()
            );
            if ext == "svg" {
                return ToolResult::success(text);
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    return ToolResult::error(format!("Failed to read image: {error}"));
                }
            };
            return match image_source(bytes) {
                Some(source) => ToolResult::success(text).with_images(vec![source]),
                None => ToolResult::error(format!(
                    "Failed to decode image {} as a supported raster format",
                    path.display()
                )),
            };
        }

        if ext == "pdf" {
            return ToolResult::success(format!(
                "[PDF file: {}. Use the `pages` parameter to read specific page ranges.]",
                path.display()
            ));
        }

        // Read raw bytes, not `read_to_string`: the revision must cover the
        // file exactly as stored, and `read_state` must compare on-disk
        // identity (Sailry `runtime-core files/lines.rs` hashes the whole file
        // while retaining only the requested page).
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Failed to read file: {}", e)),
        };

        // Full-content BLAKE3 revision: one hash covers every byte, including
        // lines outside this page — a later edit or write can prove it is
        // working on this exact file.
        let revision = blake3::hash(&bytes).to_hex().to_string();
        let size = bytes.len() as u64;

        // Fork (Waku): the model has now seen this file (`read_state`). The
        // raw bytes are recorded so the digest compares against what is on
        // disk, regardless of how the text below was decoded.
        crate::read_state::record(&ctx.session_id, &path, &bytes);

        // Strict UTF-8, as before. On Windows, a legacy-encoded text file (the
        // GBK family in the console code page) used to be rejected outright
        // even though command output is already decoded line by line there
        // (`capture::decode_output`); reuse that per-line code-page decoding
        // there and there only. Elsewhere the failure stays a binary error —
        // no guessing at an encoding (Sailry's lines kernel is strict UTF-8
        // too, with no automatic GBK detection).
        let content = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                #[cfg(windows)]
                {
                    crate::capture::decode_output(error.as_bytes())
                }
                #[cfg(not(windows))]
                {
                    let _ = error;
                    return ToolResult::error(format!(
                        "File appears to be binary and cannot be displayed as text: {}",
                        path.display()
                    ));
                }
            }
        };

        // NUL bytes mark binary content even when the surrounding bytes are
        // valid UTF-8 (Sailry `files/lines.rs` rejects them the same way).
        if content.contains('\0') {
            return ToolResult::error(format!(
                "File appears to be binary and cannot be displayed as text: {}",
                path.display()
            ));
        }

        if content.is_empty() {
            return ToolResult::success(format!("[File {} exists but is empty]", path.display()))
                .with_structured_content(file_lines_value(
                    &path,
                    Vec::new(),
                    size,
                    revision,
                    0,
                    None,
                    false,
                ));
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
        let mut page_lines: Vec<Value> = Vec::with_capacity(end - start);
        let mut next_offset: Option<usize> = None;

        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            output.push_str(&format!("{:>width$}\t{}\n", line_num, line, width = width));
            page_lines.push(json!({"number": line_num, "text": line}));
        }

        if end < total_lines {
            output.push_str(&format!(
                "\n... ({} more lines, {} total. Use offset/limit to read more.)\n",
                total_lines - end,
                total_lines
            ));
            // Zero-based index of the next unread line, the Sailry
            // `next_offset` semantic; `None` when the page reaches EOF. The
            // line-numbered text above keeps this tool's 1-based offset.
            next_offset = Some(end);
        }

        // Pages stop at a line boundary, never mid-line: `truncated` (a line
        // cut by a byte budget) stays false, as in Sailry's lines kernel when
        // only the line count limits the page.
        ToolResult::success(output).with_structured_content(file_lines_value(
            &path,
            page_lines,
            size,
            revision,
            start,
            next_offset,
            false,
        ))
    }
}

/// Convert an image file to the base64 content block the model receives.
/// Formats providers do not consistently accept are decoded and rewritten as PNG.
fn image_source(bytes: Vec<u8>) -> Option<ImageSource> {
    let direct_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    let (bytes, media_type) = match direct_type {
        Some(media_type) => (bytes, media_type),
        None => {
            let decoded = image::load_from_memory(&bytes).ok()?;
            let mut encoded = std::io::Cursor::new(Vec::new());
            decoded
                .write_to(&mut encoded, image::ImageFormat::Png)
                .ok()?;
            (encoded.into_inner(), "image/png")
        }
    };
    Some(ImageSource {
        source_type: "base64".into(),
        media_type: Some(media_type.into()),
        data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allow_all_context;

    #[tokio::test]
    async fn image_reads_return_real_bytes_and_transcode_bmp_to_png() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = allow_all_context(dir.path().to_path_buf());
        for (name, format) in [
            ("picture.png", image::ImageFormat::Png),
            ("picture.bmp", image::ImageFormat::Bmp),
        ] {
            let path = dir.path().join(name);
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 1)
                .write_to(&mut encoded, format)
                .unwrap();
            std::fs::write(&path, encoded.into_inner()).unwrap();

            let result = FileReadTool
                .execute(json!({"file_path": path.to_string_lossy()}), &ctx)
                .await;
            assert!(!result.is_error, "{}", result.content);
            assert_eq!(result.images.len(), 1);
            let source = &result.images[0];
            assert_eq!(source.source_type, "base64");
            assert_eq!(source.media_type.as_deref(), Some("image/png"));
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(source.data.as_deref().unwrap())
                .unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(
                image::guess_format(&bytes).unwrap(),
                image::ImageFormat::Png
            );
        }
    }

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

    /// The structured result carries the Sailry `FileLines` shape: one
    /// full-content BLAKE3 revision, the requested zero-based page offset, and
    /// the zero-based next unread line — while the numbered text output keeps
    /// this tool's 1-based offset.
    #[tokio::test]
    async fn structured_content_is_a_sailry_file_lines_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("paged.txt");
        std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        let res = FileReadTool
            .execute(
                json!({
                    "file_path": path.to_string_lossy(),
                    "offset": 2,
                    "limit": 2,
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "read failed: {}", res.content);

        let value = res
            .structured_content
            .expect("read must produce structured content");
        assert_eq!(value["kind"], "file_lines");
        let data = &value["data"];
        assert_eq!(data["path"], path.to_string_lossy().as_ref());
        assert_eq!(data["size"], 24);
        // Full-content revision, not just the page's.
        assert_eq!(
            data["revision"],
            blake3::hash(b"one\ntwo\nthree\nfour\nfive\n")
                .to_hex()
                .to_string()
        );
        assert_eq!(data["offset"], 1); // zero-based start of the page
        assert_eq!(data["next_offset"], 3); // zero-based next unread line
        assert_eq!(data["truncated"], false);
        let lines = data["lines"].as_array().expect("lines array");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], json!({"number": 2, "text": "two"}));
        assert_eq!(lines[1], json!({"number": 3, "text": "three"}));

        // The text output is unchanged from before structured content.
        assert!(res.content.contains("2\ttwo"), "{}", res.content);
        assert!(res.content.contains("3\tthree"), "{}", res.content);
        assert!(res.content.contains("2 more lines"), "{}", res.content);
    }

    /// A read that reaches EOF reports `next_offset: null`.
    #[tokio::test]
    async fn structured_content_next_offset_is_null_at_eof() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.txt");
        std::fs::write(&path, "only\n").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        let res = FileReadTool
            .execute(json!({"file_path": path.to_string_lossy()}), &ctx)
            .await;
        assert!(!res.is_error, "read failed: {}", res.content);
        let data = &res.structured_content.expect("structured content")["data"];
        assert_eq!(data["next_offset"], Value::Null);
        assert_eq!(data["offset"], 0);
        assert_eq!(data["lines"][0], json!({"number": 1, "text": "only"}));
    }

    /// An empty file is a valid zero-line page, with the revision of the
    /// empty content.
    #[tokio::test]
    async fn empty_file_structured_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        let res = FileReadTool
            .execute(json!({"file_path": path.to_string_lossy()}), &ctx)
            .await;
        assert!(!res.is_error, "read failed: {}", res.content);
        let value = res.structured_content.expect("structured content");
        assert_eq!(value["kind"], "file_lines");
        assert_eq!(value["data"]["size"], 0);
        assert_eq!(
            value["data"]["revision"],
            blake3::hash(b"").to_hex().to_string()
        );
        assert_eq!(value["data"]["lines"].as_array().unwrap().len(), 0);
    }

    /// A NUL byte marks binary content even when the rest is valid UTF-8;
    /// reading it must fail, not print garbage (Sailry's lines kernel rejects
    /// NUL bytes the same way).
    #[tokio::test]
    async fn a_nul_byte_is_reported_as_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nul.bin");
        std::fs::write(&path, b"text\0with nul").unwrap();

        let ctx = allow_all_context(dir.path().to_path_buf());
        let res = FileReadTool
            .execute(json!({"file_path": path.to_string_lossy()}), &ctx)
            .await;
        assert!(res.is_error, "NUL file must not be displayed");
        assert!(
            res.content.contains("binary"),
            "expected a binary error, got: {}",
            res.content
        );
    }
}
