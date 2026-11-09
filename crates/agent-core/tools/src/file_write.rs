// FileWrite tool: write/create files.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncWriteExt;
use tracing::debug;

pub struct FileWriteTool;

/// Keeps concurrent writes in this process from sharing one temp file name
/// (`write_atomic` keys only on the process id, so two in-flight writes of the
/// same destination from one process would race on the same temp path).
static TMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// On-disk identity of a file as observed by an earlier read: size plus
/// modification time. Cheap to capture, cheap to compare.
type FileIdentity = (u64, Option<std::time::SystemTime>);

fn identity_of(meta: &std::fs::Metadata) -> FileIdentity {
    (meta.len(), meta.modified().ok())
}

/// Write `contents` to `path` through an fsynced temp file that is atomically
/// renamed over the destination — and never silently over a concurrent
/// change. Sailry `runtime-core files/save/target.rs` does the same three
/// observations:
///
/// 1. The temp file is fully written and `sync_all`ed (fsync) before the
///    rename, so a crash can never surface a half-written destination.
/// 2. The destination is re-stat'ed right before the rename and compared with
///    `identity` (the file as previously read); a change — or a file
///    appearing where the tool saw none — aborts with the temp removed.
/// 3. The destination is re-read right after the rename and must hold exactly
///    `contents`; anything else is reported as an error instead of claimed.
///
/// The check-then-rename window cannot be closed without locks; like Sailry,
/// this narrows it to the smallest practical race and detects its own losses.
async fn write_verified(
    path: &std::path::Path,
    contents: &[u8],
    identity: Option<FileIdentity>,
) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let tmp = path.with_file_name(format!(
        ".{}.claurst-tmp-{}-{}",
        file_name,
        std::process::id(),
        TMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let write_result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(contents).await?;
        // Preserve the original file's permissions (e.g. the executable bit
        // on Unix), which a fresh temp file would otherwise reset.
        if let Ok(meta) = tokio::fs::metadata(path).await {
            let _ = tokio::fs::set_permissions(&tmp, meta.permissions()).await;
        }
        // fsync the temp before it becomes the destination.
        file.sync_all().await?;
        Ok(())
    }
    .await;
    if let Err(e) = write_result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    // Observation 2: the destination must still be the file we read.
    match identity {
        Some((len, modified)) => match tokio::fs::metadata(path).await {
            Ok(meta) => {
                if identity_of(&meta) != (len, modified) {
                    let _ = tokio::fs::remove_file(&tmp).await;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("{} changed during write", path.display()),
                    ));
                }
            }
            // Concurrently deleted: renaming over the gap is safe.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(e);
            }
        },
        None => {
            // The tool saw no file here; refuse to clobber whatever created
            // one in between.
            if path.exists() {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("{} was created concurrently", path.display()),
                ));
            }
        }
    }

    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    // Best-effort directory fsync so the rename itself survives a crash.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    // Observation 3: the destination must now hold exactly what we wrote.
    match tokio::fs::read(path).await {
        Ok(written) if written == contents => Ok(()),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "{} does not hold the written content immediately after the write",
                path.display()
            ),
        )),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Deserialize)]
struct FileWriteInput {
    file_path: String,
    content: String,
}

#[async_trait]
impl Tool for FileWriteTool {
    // Gates itself: calls `ctx.check_permission` in `execute()` (#210).
    fn self_gates(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        agent_core::constants::TOOL_NAME_FILE_WRITE
    }

    fn description(&self) -> &str {
        "Writes a file to the local filesystem. This tool will overwrite the existing \
         file if there is one; an existing file must be read with the Read tool first. \
         Prefer the Edit tool for modifying existing files. \
         Only use this tool to create new files or for complete rewrites."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: FileWriteInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {}", e)),
        };

        let path = ctx.resolve_path(&params.file_path);
        debug!(path = %path.display(), "Writing file");

        // Permission check
        if let Err(e) =
            ctx.check_permission(self.name(), &format!("Write {}", path.display()), false)
        {
            return ToolResult::error(e.to_string());
        }

        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return ToolResult::error(format!(
                        "Failed to create directory {}: {}",
                        parent.display(),
                        e
                    ));
                }
            }
        }

        let existed = path.exists();
        // Fork (Waku): an existing file is replaced only once the model has
        // read it and it has not changed since (`read_state`); a new one may
        // always be created.
        if existed {
            if let Some(reason) = crate::read_state::refusal(&ctx.session_id, &path) {
                return ToolResult::error(reason);
            }
        }
        let before_content = if existed {
            match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    return ToolResult::error(format!(
                        "Failed to read existing file {}: {}",
                        path.display(),
                        e
                    ))
                }
            }
        } else {
            Vec::new()
        };
        // On-disk identity of the file we just read, re-checked by
        // `write_verified` immediately before the rename so a concurrent
        // change between the read and the write is never overwritten.
        let identity = if existed {
            tokio::fs::metadata(&path)
                .await
                .ok()
                .map(|m| identity_of(&m))
        } else {
            None
        };
        let is_new = !existed;

        // Write the file: fsynced temp file, atomic rename, destination
        // identity re-checked before the rename and content verified after it.
        if let Err(e) = write_verified(&path, params.content.as_bytes(), identity).await {
            return ToolResult::error(format!("Failed to write file {}: {}", path.display(), e));
        }

        ctx.record_file_change(
            path.clone(),
            &before_content,
            params.content.as_bytes(),
            self.name(),
        );

        // Run any configured formatter for this file type.
        crate::try_format_file(&path.to_string_lossy(), ctx).await;

        let line_count = params.content.lines().count();
        let byte_count = params.content.len();

        // Structured result: the `FileWritten` shape Sailry protocol defines.
        // A whole-file write has no excerpt (`excerpt: null`), and the
        // revision covers exactly the bytes written.
        let revision = blake3::hash(params.content.as_bytes()).to_hex().to_string();

        let action = if is_new { "Created" } else { "Wrote" };
        ToolResult::success(format!(
            "{} {} ({} lines, {} bytes)",
            action,
            path.display(),
            line_count,
            byte_count
        ))
        .with_metadata(json!({
            "file_path": path.display().to_string(),
            "is_new": is_new,
            "lines": line_count,
            "bytes": byte_count,
        }))
        .with_structured_content(json!({
            "kind": "file_written",
            "data": {
                "path": path.display().to_string(),
                "revision": revision,
                "size": byte_count as u64,
                "excerpt": null,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::allow_all_context;

    fn temp_context(dir: &tempfile::TempDir) -> ToolContext {
        allow_all_context(dir.path().to_path_buf())
    }

    #[tokio::test]
    async fn creates_a_new_file_with_a_file_written_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("created.txt");

        let ctx = temp_context(&dir);
        let res = FileWriteTool
            .execute(
                json!({
                    "file_path": path.to_string_lossy(),
                    "content": "hello\nworld\n",
                }),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "write failed: {}", res.content);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nworld\n");

        let value = res
            .structured_content
            .expect("write must produce structured content");
        assert_eq!(value["kind"], "file_written");
        let data = &value["data"];
        assert_eq!(data["path"], path.to_string_lossy().as_ref());
        assert_eq!(data["size"], 12);
        assert_eq!(
            data["revision"],
            blake3::hash(b"hello\nworld\n").to_hex().to_string()
        );
        assert_eq!(data["excerpt"], Value::Null);

        // The fsync+rename path must not leave its temp file behind.
        let tmp_left = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".claurst-tmp-"));
        assert!(
            !tmp_left,
            "verified write must not leave a temp file behind"
        );
    }

    #[tokio::test]
    async fn overwrites_an_existing_file_once_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "before\n").unwrap();

        let ctx = temp_context(&dir);
        // Not read yet: refused.
        let res = FileWriteTool
            .execute(
                json!({"file_path": path.to_string_lossy(), "content": "after\n"}),
                &ctx,
            )
            .await;
        assert!(
            res.is_error && res.content.contains("has not been read"),
            "{}",
            res.content
        );

        crate::read_state::mark_seen(&ctx.session_id, &path);
        let res = FileWriteTool
            .execute(
                json!({"file_path": path.to_string_lossy(), "content": "after\n"}),
                &ctx,
            )
            .await;
        assert!(!res.is_error, "write failed: {}", res.content);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
        assert_eq!(
            res.structured_content.expect("structured content")["data"]["revision"],
            blake3::hash(b"after\n").to_hex().to_string()
        );
    }

    #[tokio::test]
    async fn write_verified_refuses_a_file_changed_since_it_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raced.txt");
        std::fs::write(&path, "original\n").unwrap();

        // Capture the identity, then change the file behind the writer's back.
        let identity = tokio::fs::metadata(&path)
            .await
            .ok()
            .map(|m| identity_of(&m));
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "someone else\n").unwrap();

        let result = write_verified(&path, b"clobber\n", identity).await;
        let error = result.expect_err("a changed file must not be overwritten");
        assert!(
            error.to_string().contains("changed during write"),
            "{}",
            error
        );
        // Both the temp file and the concurrent change survive untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else\n");
        let tmp_left = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".claurst-tmp-"));
        assert!(!tmp_left, "a refused write must clean up its temp file");
    }

    #[tokio::test]
    async fn write_verified_refuses_to_clobber_a_concurrently_created_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appeared.txt");
        std::fs::write(&path, "created meanwhile\n").unwrap();

        // The tool saw no file here (identity None), so the write must fail
        // rather than replace the file that appeared.
        let result = write_verified(&path, b"mine\n", None).await;
        let error = result.expect_err("a concurrently created file must not be overwritten");
        assert!(
            error.to_string().contains("created concurrently"),
            "{}",
            error
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "created meanwhile\n"
        );
    }

    #[tokio::test]
    async fn write_verified_preserves_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let identity = tokio::fs::metadata(&path)
            .await
            .ok()
            .map(|m| identity_of(&m));
        write_verified(&path, b"#!/bin/sh\necho hi\n", identity)
            .await
            .expect("rewrite");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o111,
                0o111,
                "executable bit must survive the rewrite"
            );
        }
    }
}
