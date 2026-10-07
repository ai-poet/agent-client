//! Fork addition: how the built-in agent's memory tools read in the
//! transcript and in their approval dialog (`waku_agent_bridge`'s
//! `memory`). Without this a call rows as its raw name, or — worse — as
//! the `title` argument of `memory_write`, which is the memory's heading.

use serde_json::Value;

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The row title of a memory tool call; `None` for any other tool.
pub(super) fn tool_title(name: &str, input: &Value) -> Option<String> {
    let memory = || text(input, "title").or_else(|| text(input, "name")).unwrap_or("…");
    Some(match name {
        "memory_write" => tr!("memory.tool.write", name = memory()),
        "memory_read" => tr!("memory.tool.read", name = memory()),
        "memory_list" => tr!("memory.tool.list"),
        "memory_delete" => tr!("memory.tool.delete", name = memory()),
        "memory_prune" => tr!(
            "memory.tool.prune",
            days = input
                .get("olderThanDays")
                .and_then(Value::as_i64)
                .unwrap_or_default()
        ),
        "memory_delete_all" => tr!("memory.tool.delete_all"),
        _ => return None,
    })
}

/// The approval dialog's title for a memory bulk delete.
pub(super) fn approval_title(tool_name: &str) -> Option<String> {
    match tool_name {
        "memory_prune" => Some(tr!("memory.approval.prune_title")),
        "memory_delete_all" => Some(tr!("memory.approval.delete_all_title")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn memory_calls_get_titles_of_their_own() {
        let write = tool_title(
            "memory_write",
            &json!({"name": "use-pnpm", "title": "Uses pnpm", "description": "d"}),
        )
        .unwrap();
        assert!(write.contains("Uses pnpm"), "{write}");
        let read = tool_title("memory_read", &json!({"name": "use-pnpm"})).unwrap();
        assert!(read.contains("use-pnpm"), "{read}");
        let prune = tool_title("memory_prune", &json!({"olderThanDays": 30})).unwrap();
        assert!(prune.contains("30"), "{prune}");
        for name in ["memory_list", "memory_delete", "memory_delete_all"] {
            assert!(tool_title(name, &json!({})).is_some(), "{name}");
        }
        assert!(tool_title("Bash", &json!({"title": "x"})).is_none());
    }

    #[test]
    fn only_bulk_deletes_get_an_approval_title() {
        assert!(approval_title("memory_prune").is_some());
        assert!(approval_title("memory_delete_all").is_some());
        assert!(approval_title("memory_delete").is_none());
    }
}
