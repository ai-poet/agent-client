// Translated from dsh-auto-memory (MIT, Copyright (c) 2026 AskTheWay); see NOTICE.md.

//! The six memory tools. Descriptions are the reference's; store work runs
//! on a blocking thread (the store takes a file lock).
//!
//! Permission levels: reading and listing are `ReadOnly` (they run beside
//! other reads); writing and deleting one memory are `None` — memory files
//! are not the workspace, so they never prompt and work in plan mode, as in
//! the reference. The two bulk deletes gate themselves and the bridge asks
//! the person whatever the access mode (`crate::permission`): the model
//! cannot approve its own "forget everything".

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use auto_memory::links::expand_links;
use auto_memory::{MemoryDraft, MemoryScope, MemoryType, ScopeDir};
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};
use serde_json::{Value, json};

use super::{DELETE_ALL_TOOL, MemoryHost, PRUNE_TOOL};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Write,
    Read,
    List,
    Delete,
    Prune,
    DeleteAll,
}

const OPS: [Op; 6] = [Op::Write, Op::Read, Op::List, Op::Delete, Op::Prune, Op::DeleteAll];

pub(super) fn all(host: &Arc<MemoryHost>) -> Vec<Box<dyn Tool>> {
    OPS.into_iter()
        .map(|op| {
            Box::new(MemoryTool {
                host: Arc::downgrade(host),
                op,
            }) as Box<dyn Tool>
        })
        .collect()
}

struct MemoryTool {
    host: Weak<MemoryHost>,
    op: Op,
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        match self.op {
            Op::Write => "memory_write",
            Op::Read => "memory_read",
            Op::List => "memory_list",
            Op::Delete => "memory_delete",
            Op::Prune => PRUNE_TOOL,
            Op::DeleteAll => DELETE_ALL_TOOL,
        }
    }

    fn description(&self) -> &str {
        match self.op {
            Op::Write => {
                "Write or update one persistent memory (a fact that should survive across sessions). \
                 Reuse an existing name to UPDATE that memory instead of creating a near-duplicate. \
                 Write when: the user states who they are or their preferences (user); the user corrects or confirms \
                 how you should work (feedback — include **Why:** and **How to apply:** lines); ongoing work, goals or \
                 constraints emerge (project — absolute dates only); an external resource is worth returning to (reference). \
                 Do NOT store what the codebase or AGENTS.md/CLAUDE.md already records."
            }
            Op::Read => {
                "Read one persistent memory by name (full body, with one level of [[name]] cross-links resolved). \
                 Search the injected memory index for the name first."
            }
            Op::List => {
                "List persistent memories (name, description, type, scope). Use before writing to avoid duplicates."
            }
            Op::Delete => {
                "Delete one persistent memory by name. Use when a memory turned out wrong or obsolete."
            }
            Op::Prune => {
                "List (dry-run, default) or delete memories not updated within olderThanDays. \
                 Use to keep the store healthy: propose a dry-run first, show the candidates to the user, \
                 then delete only with their consent. Pinned memories and memories without lifecycle metadata are never matched."
            }
            Op::DeleteAll => {
                "Delete ALL memories in a scope (or both). Destructive — requires confirm=true and \
                 an explicit statement from the user that they want everything forgotten."
            }
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        match self.op {
            Op::Read | Op::List => PermissionLevel::ReadOnly,
            Op::Write | Op::Delete | Op::Prune | Op::DeleteAll => PermissionLevel::None,
        }
    }

    /// The bulk deletes ask for themselves, with what they would delete.
    fn self_gates(&self) -> bool {
        matches!(self.op, Op::Prune | Op::DeleteAll)
    }

    fn input_schema(&self) -> Value {
        let scope = |what: &str| json!({ "type": "string", "enum": ["project", "user"], "description": what });
        match self.op {
            Op::Write => json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "kebab-case identifier (e.g. \"user-prefers-python\"); also the storage key — reuse to update" },
                    "description": { "type": "string", "description": "One-line summary shown in the injected memory index; keep it under ~160 chars" },
                    "type": { "type": "string", "enum": ["user", "feedback", "project", "reference"], "description": "user=who the user is; feedback=how to work (Why/How to apply); project=ongoing work/goals; reference=external pointers" },
                    "body": { "type": "string", "description": "The fact itself, in markdown. Cross-link with [[other-name]]. feedback type: end with **Why:** and **How to apply:** lines" },
                    "title": { "type": "string", "description": "Optional human-readable heading shown in the index (any language); defaults to name" },
                    "pinned": { "type": "boolean", "description": "Pin this memory: it sorts first in the index, survives budget truncation, and is never hidden by staleness eviction. Use when the user explicitly says to keep something forever; unpin by passing false" },
                    "scope": scope("project: only this workspace's sessions; user: all sessions of this user. Default: update the layer where this name already exists, else project"),
                },
                "required": ["name", "description", "type", "body"],
            }),
            Op::Read => json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Memory name from the index (kebab-case)" },
                    "scope": scope("Limit to one scope; default searches user then project"),
                },
                "required": ["name"],
            }),
            Op::List => json!({
                "type": "object",
                "properties": { "scope": scope("Limit to one scope; default lists both") },
            }),
            Op::Delete => json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Memory name to delete (kebab-case)" },
                    "scope": scope("Scope to delete from; default searches user then project"),
                },
                "required": ["name"],
            }),
            Op::Prune => json!({
                "type": "object",
                "properties": {
                    "olderThanDays": { "type": "integer", "description": "Match memories whose last update is older than this many days" },
                    "scope": scope("Limit to one scope; default both"),
                    "dryRun": { "type": "boolean", "description": "true (default): only list candidates; false: delete them" },
                },
                "required": ["olderThanDays"],
            }),
            Op::DeleteAll => json!({
                "type": "object",
                "properties": {
                    "scope": scope("Scope to clear; default both"),
                    "confirm": { "type": "boolean", "description": "Must be explicitly true to delete" },
                },
                "required": ["confirm"],
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let Some(host) = self.host.upgrade() else {
            return ToolResult::error("Memory is not available in this session.");
        };
        let scopes = match target_scopes(&host, &input) {
            Ok(scopes) => scopes,
            Err(error) => return ToolResult::error(error),
        };
        let result = match self.op {
            Op::Write => blocking(host, move |host| write(&host, &input, &scopes)).await,
            Op::Read => blocking(host, move |host| read(&host, &input, &scopes)).await,
            Op::List => blocking(host, move |host| Ok(list(&host, &scopes))).await,
            Op::Delete => blocking(host, move |host| delete(&host, &input, &scopes)).await,
            Op::Prune => prune(host, &input, scopes, ctx).await,
            Op::DeleteAll => delete_all(host, &input, scopes, ctx).await,
        };
        match result {
            Ok(text) => ToolResult::success(text),
            Err(error) => ToolResult::error(error),
        }
    }
}

/// Run store work off the async runtime.
async fn blocking(
    host: Arc<MemoryHost>,
    work: impl FnOnce(Arc<MemoryHost>) -> Result<String, String> + Send + 'static,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || work(host))
        .await
        .unwrap_or_else(|error| Err(format!("memory: the call did not finish ({error})")))
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

/// The scopes a call may touch: the one named, else every scope this
/// session reaches (user first). Naming the user scope while it is
/// switched off is an error, as in the reference.
fn target_scopes(host: &MemoryHost, input: &Value) -> Result<Vec<ScopeDir>, String> {
    match text(input, "scope").map(str::trim).filter(|scope| !scope.is_empty()) {
        None => Ok(host.scopes()),
        Some(raw) => match MemoryScope::parse(raw) {
            Some(MemoryScope::Project) => Ok(vec![host.project_dir()]),
            Some(MemoryScope::User) => host
                .user_dir()
                .map(|dir| vec![dir])
                .ok_or_else(|| "user scope is disabled in the memory settings; use 'project'".to_owned()),
            None => Err(format!(
                "invalid scope: {} (expected 'project' or 'user')",
                serde_json::to_string(raw).unwrap_or_default()
            )),
        },
    }
}

fn write(host: &MemoryHost, input: &Value, scopes: &[ScopeDir]) -> Result<String, String> {
    let name = auto_memory::name::normalize_name(text(input, "name").unwrap_or_default())?;
    let kind = text(input, "type")
        .and_then(MemoryType::parse)
        .ok_or("invalid memory type (expected one of user/feedback/project/reference)")?;
    let description = text(input, "description").unwrap_or_default().trim().to_owned();
    if description.is_empty() {
        return Err("description must not be empty".to_owned());
    }
    let body = text(input, "body").unwrap_or_default().to_owned();
    // No scope named: update wherever the name already lives, else project.
    let explicit = text(input, "scope").is_some_and(|scope| !scope.trim().is_empty());
    let existing = host
        .store
        .find_in(&name, &host.scopes())
        .map_err(|error| error.to_string())?;
    let target = if explicit {
        scopes[0].clone()
    } else {
        existing
            .as_ref()
            .map(|(dir, _)| dir.clone())
            .unwrap_or_else(|| host.project_dir())
    };
    let draft = MemoryDraft {
        name,
        title: text(input, "title").map(str::to_owned),
        description,
        kind,
        body,
        pinned: input.get("pinned").and_then(Value::as_bool),
    };
    let (record, created) = host
        .store
        .write(&target, draft)
        .map_err(|error| format!("{error:#}"))?;
    Ok(format!(
        "Memory {}: {} ({}{})",
        if created { "created" } else { "updated" },
        record.name,
        target.scope.as_str(),
        if record.pinned { ", pinned" } else { "" }
    ))
}

fn read(host: &MemoryHost, input: &Value, scopes: &[ScopeDir]) -> Result<String, String> {
    let name = text(input, "name").unwrap_or_default();
    let Some((dir, record)) = host
        .store
        .find_in(name, scopes)
        .map_err(|error| error.to_string())?
    else {
        return Err(format!(
            "memory not found: {} — call memory_list to see available names",
            serde_json::to_string(name).unwrap_or_default()
        ));
    };
    host.store.touch(&dir, &record.name);
    let all_scopes = host.scopes();
    let (linked, _) = expand_links(&record.body, &record.name, |link| {
        host.store
            .find_in(link, &all_scopes)
            .ok()
            .flatten()
            .map(|(_, linked)| linked.description)
    });
    let mut text = format!(
        "--- name: {}\ndescription: {}\ntype: {}\nscope: {}\n---\n\n{}",
        record.name,
        record.description,
        record.kind.as_str(),
        dir.scope.as_str(),
        record.body
    );
    if !linked.is_empty() {
        text.push_str("\n\nLinked memories:");
        for link in linked {
            text.push_str(&format!("\n- {} — {}", link.name, link.description));
        }
    }
    Ok(text)
}

fn list(host: &MemoryHost, scopes: &[ScopeDir]) -> String {
    let lines = scopes
        .iter()
        .flat_map(|dir| host.store.list(dir))
        .map(|record| {
            format!(
                "- {}{} ({}/{}) — {}",
                record.name,
                if record.pinned { " 📌" } else { "" },
                record.scope.as_str(),
                record.kind.as_str(),
                record.description
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "No memories yet.".to_owned()
    } else {
        lines.join("\n")
    }
}

fn delete(host: &MemoryHost, input: &Value, scopes: &[ScopeDir]) -> Result<String, String> {
    let name = auto_memory::name::normalize_name(text(input, "name").unwrap_or_default())?;
    for dir in scopes {
        if host
            .store
            .delete(dir, &name)
            .map_err(|error| format!("{error:#}"))?
        {
            return Ok(format!("Deleted memory: {name} ({})", dir.scope.as_str()));
        }
    }
    Err(format!(
        "memory not found: {}",
        serde_json::to_string(&name).unwrap_or_default()
    ))
}

async fn prune(
    host: Arc<MemoryHost>,
    input: &Value,
    scopes: Vec<ScopeDir>,
    ctx: &ToolContext,
) -> Result<String, String> {
    let days = input
        .get("olderThanDays")
        .and_then(Value::as_i64)
        .filter(|days| *days >= 1 && *days <= i64::from(u32::MAX))
        .ok_or("olderThanDays must be an integer >= 1")? as u32;
    let dry_run = input.get("dryRun").and_then(Value::as_bool) != Some(false);
    let candidates = {
        let host = host.clone();
        let scopes = scopes.clone();
        tokio::task::spawn_blocking(move || {
            host.store
                .prune_candidates(&scopes, days, auto_memory::now_ms())
        })
        .await
        .map_err(|error| format!("memory: the call did not finish ({error})"))?
    };
    let lines = candidates
        .iter()
        .map(|candidate| {
            format!(
                "- {} ({}, {}d since update) — {}",
                candidate.name,
                candidate.scope.as_str(),
                candidate.days_since_update,
                candidate.description
            )
        })
        .collect::<Vec<_>>();
    if dry_run || candidates.is_empty() {
        let head = format!(
            "Prune dry-run: {} candidate(s) older than {days} days{}",
            candidates.len(),
            if candidates.is_empty() { "" } else { " — re-run with dryRun=false to delete" }
        );
        return Ok(if lines.is_empty() {
            format!("{head}.")
        } else {
            format!("{head}\n{}", lines.join("\n"))
        });
    }
    let summary = format!("Delete {} memories older than {days} days", candidates.len());
    if let Err(refused) = ctx.check_permission_with_details(PRUNE_TOOL, &summary, &lines.join("\n"), false) {
        return Err(refused.to_string());
    }
    let deleted = tokio::task::spawn_blocking(move || {
        candidates
            .iter()
            .filter(|candidate| {
                scopes
                    .iter()
                    .find(|dir| dir.scope == candidate.scope)
                    .is_some_and(|dir| matches!(host.store.delete(dir, &candidate.name), Ok(true)))
            })
            .count()
    })
    .await
    .map_err(|error| format!("memory: the call did not finish ({error})"))?;
    Ok(format!(
        "Pruned {deleted} of {} candidate(s)\n{}",
        lines.len(),
        lines.join("\n")
    ))
}

async fn delete_all(
    host: Arc<MemoryHost>,
    input: &Value,
    scopes: Vec<ScopeDir>,
    ctx: &ToolContext,
) -> Result<String, String> {
    if input.get("confirm").and_then(Value::as_bool) != Some(true) {
        return Err(
            "memory_delete_all requires confirm=true (destructive); ask the user first".to_owned(),
        );
    }
    let detail = {
        let host = host.clone();
        let scopes = scopes.clone();
        tokio::task::spawn_blocking(move || {
            scopes
                .iter()
                .map(|dir| {
                    let names = host
                        .store
                        .list(dir)
                        .into_iter()
                        .map(|record| record.name)
                        .collect::<Vec<_>>();
                    format!(
                        "{} ({}): {}",
                        dir.scope.as_str(),
                        names.len(),
                        if names.is_empty() { "—".to_owned() } else { names.join(", ") }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .await
        .map_err(|error| format!("memory: the call did not finish ({error})"))?
    };
    let summary = format!(
        "Delete every memory in {}",
        scopes.iter().map(|dir| dir.scope.as_str()).collect::<Vec<_>>().join(" and ")
    );
    if let Err(refused) = ctx.check_permission_with_details(DELETE_ALL_TOOL, &summary, &detail, false) {
        return Err(refused.to_string());
    }
    let results = tokio::task::spawn_blocking(move || {
        scopes
            .iter()
            .map(|dir| {
                host.store
                    .clear(dir)
                    .map(|deleted| format!("{deleted} in {}", dir.scope.as_str()))
                    .map_err(|error| format!("{error:#}"))
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| format!("memory: the call did not finish ({error})"))??;
    Ok(format!("Deleted {}", results.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use auto_memory::MemoryConfig;
    use std::path::Path;

    fn host(dir: &Path, user_scope: bool) -> Arc<MemoryHost> {
        MemoryHost::with_config(
            MemoryConfig {
                mirror_to_claude_code: false,
                enable_user_scope: user_scope,
                ..MemoryConfig::default()
            },
            dir,
            Path::new("/work/app"),
            "session",
        )
    }

    #[test]
    fn write_updates_where_the_name_lives_and_reports_it() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), true);
        let input = |scope: Option<&str>, description: &str| {
            let mut value = json!({"name": "Prefers PNPM", "description": description, "type": "user", "body": "b"});
            if let Some(scope) = scope {
                value["scope"] = json!(scope);
            }
            value
        };
        let user = target_scopes(&host, &input(Some("user"), "x")).unwrap();
        assert_eq!(
            write(&host, &input(Some("user"), "first"), &user).unwrap(),
            "Memory created: prefers-pnpm (user)"
        );
        // No scope: the update lands where the name already is.
        let all = target_scopes(&host, &input(None, "x")).unwrap();
        assert_eq!(
            write(&host, &input(None, "second"), &all).unwrap(),
            "Memory updated: prefers-pnpm (user)"
        );
        assert!(host.store.list(&host.project_dir()).is_empty());
    }

    #[test]
    fn a_switched_off_user_scope_cannot_be_named() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), false);
        assert!(target_scopes(&host, &json!({"scope": "user"})).is_err());
        assert!(target_scopes(&host, &json!({"scope": "other"})).is_err());
        assert_eq!(target_scopes(&host, &json!({})).unwrap().len(), 1);
    }

    #[test]
    fn read_counts_and_expands_links() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), true);
        let scopes = host.scopes();
        write(
            &host,
            &json!({"name": "a", "description": "about a", "type": "project", "body": "see [[b]]"}),
            &scopes,
        )
        .unwrap();
        write(
            &host,
            &json!({"name": "b", "description": "about b", "type": "project", "body": "x"}),
            &scopes,
        )
        .unwrap();
        let text = read(&host, &json!({"name": "a"}), &scopes).unwrap();
        assert!(text.contains("Linked memories:\n- b — about b"), "{text}");
        let (_, record) = host.store.find_in("a", &scopes).unwrap().unwrap();
        assert_eq!(record.reads, Some(1));
        assert!(read(&host, &json!({"name": "zz"}), &scopes).is_err());
    }

    #[test]
    fn list_and_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), true);
        let scopes = host.scopes();
        assert_eq!(list(&host, &scopes), "No memories yet.");
        write(
            &host,
            &json!({"name": "a", "description": "d", "type": "feedback", "body": "x", "pinned": true}),
            &scopes,
        )
        .unwrap();
        assert_eq!(list(&host, &scopes), "- a 📌 (project/feedback) — d");
        assert_eq!(delete(&host, &json!({"name": "a"}), &scopes).unwrap(), "Deleted memory: a (project)");
        assert!(delete(&host, &json!({"name": "a"}), &scopes).is_err());
    }

    #[test]
    fn only_the_bulk_deletes_gate_themselves() {
        let tmp = tempfile::tempdir().unwrap();
        let host = host(tmp.path(), true);
        let gated = all(&host)
            .iter()
            .filter(|tool| tool.self_gates())
            .map(|tool| tool.name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(gated, vec![PRUNE_TOOL, DELETE_ALL_TOOL]);
        assert!(super::super::needs_human_approval(PRUNE_TOOL));
        assert!(!super::super::needs_human_approval("memory_write"));
    }
}
