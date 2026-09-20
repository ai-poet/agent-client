// Tool execution helpers: argument parsing, permission gating, and the
// single-tool / batch execution paths. Extracted from lib.rs (issue #232).
// Behavior-preserving move.

use crate::*;

/// Parse the accumulated JSON arguments of a streamed tool call.
///
/// Providers stream a tool call's arguments as a sequence of partial-JSON
/// deltas which we concatenate into a single buffer. A well-behaved
/// no-argument call yields an empty (or whitespace-only) buffer, which we
/// map to an empty object. Any *non-empty* buffer that fails to parse is
/// returned as an error rather than being silently replaced with `{}` — a
/// truncated stream must never cause a tool (e.g. Edit/Write) to run with
/// empty arguments (issue #215).
pub(crate) fn parse_tool_args(json_str: &str) -> Result<Value, serde_json::Error> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(trimmed)
}

/// Whether a `PermissionLevel` must be gated by the central backstop.
///
/// Only `None` and `ReadOnly` are exempt; every other level (`Write`,
/// `Execute`, `Dangerous`, `Forbidden`) represents a side-effecting action that
/// the backstop must confirm before it runs.
pub(crate) fn permission_level_is_gated(level: PermissionLevel) -> bool {
    !matches!(level, PermissionLevel::None | PermissionLevel::ReadOnly)
}

/// Synthesize a human-readable permission description for a tool that does not
/// gate itself, surfacing the tool name and a truncated preview of its input so
/// the user can see what is about to run.
pub(crate) fn synthesize_permission_description(name: &str, input: &Value) -> String {
    let rendered = serde_json::to_string(input).unwrap_or_default();
    let preview: String = rendered.chars().take(200).collect();
    if preview.is_empty() || preview == "{}" || preview == "null" {
        format!("Run tool '{}'", name)
    } else {
        format!("Run tool '{}' with input: {}", name, preview)
    }
}

/// Execute a single tool invocation.
/// Rewrite every float with nothing after the decimal point as an integer.
///
/// Fork: tools parse their arguments with `serde_json::from_value` into
/// concrete structs, so a field declared `usize` rejects `120.0` outright —
/// `invalid type: floating point `120.0`, expected usize`. The value is the
/// one the model meant; only its JSON spelling is wrong, and which spelling a
/// model reaches for is a property of the model rather than of the prompt.
/// Claude writes `120`; several others write `120.0` often enough that a long
/// session is likely to hit it, and instructing a model about number
/// formatting does not make it reliable. The built-in agent can be pointed at
/// any of them, so the repair belongs where the arguments arrive.
///
/// Placed at this one dispatch point because it is the only one: sub-agents
/// build their own tool sets but still call tools through here, so a fix in
/// the tool set alone would have left them broken.
///
/// The whole argument tree is rewritten rather than the fields a schema calls
/// `integer`, because it does not need to be that careful: serde reads `120`
/// into an `f64` field just as happily as `120.0`, and JSON has one number
/// type, so `120` is the same value to whatever parses it next — an MCP
/// server included. A float with an actual fraction is left alone: it was
/// never going to fit an integer field, and turning `0.5` into `0` would
/// answer a type error with a wrong number.
fn whole_floats_to_integers(value: Value) -> Value {
    match value {
        // `is_f64` is the question, not whether the value looks whole — an
        // integer that arrived as an integer is already right.
        Value::Number(number) if number.is_f64() => match number.as_f64() {
            // `as i64` saturates rather than wrapping, so without the range
            // check `1e30` would silently become `i64::MAX` — a number nobody
            // wrote. Out of range keeps its spelling and fails downstream,
            // which is the honest outcome.
            Some(float)
                if float.fract() == 0.0
                    && float >= i64::MIN as f64
                    && float <= i64::MAX as f64 =>
            {
                Value::Number(serde_json::Number::from(float as i64))
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => {
            Value::Array(items.into_iter().map(whole_floats_to_integers).collect())
        }
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, field)| (name, whole_floats_to_integers(field)))
                .collect(),
        ),
        other => other,
    }
}

pub(crate) async fn execute_tool(
    name: &str,
    input: &Value,
    tools: &[Box<dyn Tool>],
    ctx: &ToolContext,
) -> ToolResult {
    let tool = tools.iter().find(|t| t.name() == name);

    match tool {
        Some(tool) => {
            debug!(tool = name, "Executing tool");
            // Central permission backstop (issue #210): if a tool does not gate
            // itself (`self_gates() == false`) and requires a gated permission
            // level, prompt here BEFORE executing. On denial, return a blocked
            // result WITHOUT running the tool. Tools that already prompt
            // internally opt out via `self_gates() == true` (no double-prompt),
            // and read-only / no-permission tools are skipped. This makes a tool
            // that forgets to gate itself secure by default.
            if !tool.self_gates() && permission_level_is_gated(tool.permission_level()) {
                let description = synthesize_permission_description(name, input);
                if let Err(e) = ctx.check_permission(name, &description, false) {
                    warn!(tool = name, "Tool blocked by central permission backstop");
                    return ToolResult::error(e.to_string());
                }
            }
            tool.execute(whole_floats_to_integers(input.clone()), ctx).await
        }
        None => {
            warn!(tool = name, "Unknown tool requested");
            ToolResult::error(format!("Unknown tool: {}", name))
        }
    }
}

/// Run a batch of tool-execution futures concurrently, abandoning them promptly
/// if `cancel_token` fires (issue #218).
///
/// Returns exactly one `ToolResult` per input future, in order, plus a bool that
/// is `true` iff the batch was cancelled before every tool finished. On the
/// happy path (no cancellation) this is `join_all` and the results are the real
/// tool outputs. On cancellation the in-flight futures are dropped (abandoned)
/// and every position is filled with a synthetic cancelled `ToolResult` so the
/// caller can still answer every `tool_use` and keep the message history valid.
pub(crate) async fn run_tool_batch<F>(
    exec_futures: Vec<F>,
    cancel_token: &tokio_util::sync::CancellationToken,
) -> (Vec<ToolResult>, bool)
where
    F: std::future::Future<Output = ToolResult>,
{
    let count = exec_futures.len();
    tokio::select! {
        results = futures::future::join_all(exec_futures) => (results, false),
        _ = cancel_token.cancelled() => {
            let cancelled = (0..count)
                .map(|_| ToolResult::error(TOOL_CANCELLED_MSG))
                .collect();
            (cancelled, true)
        }
    }
}

/// Load persisted todos for `session_id` and return a nudge string if any are
/// incomplete (status != "completed"). Returns empty string otherwise.
pub(crate) fn build_todo_nudge(session_id: &str) -> String {
    let todos = claurst_tools::todo_write::load_todos(session_id);
    let incomplete_count = todos
        .iter()
        .filter(|t| t["status"].as_str() != Some("completed"))
        .count();
    if incomplete_count == 0 {
        String::new()
    } else {
        format!(
            "You have {} incomplete task{} in your TodoWrite list. \
             Make sure to complete all tasks before ending your response.",
            incomplete_count,
            if incomplete_count == 1 { "" } else { "s" }
        )
    }
}

#[cfg(test)]
mod fork_number_repair_tests {
    use super::*;
    use serde_json::json;

    /// The three shapes this was reported as: a top-level `limit`, a nested
    /// one, and numbers inside an array.
    #[test]
    fn whole_floats_become_integers_anywhere_in_the_tree() {
        let repaired = whole_floats_to_integers(json!({
            "limit": 120.0,
            "nested": {"head_limit": 80.0},
            "offset": [1.0, 2.0],
        }));
        assert_eq!(
            repaired,
            json!({"limit": 120, "nested": {"head_limit": 80}, "offset": [1, 2]})
        );
        // And the repaired form is what the failing parse wanted.
        assert_eq!(
            serde_json::from_value::<usize>(repaired["limit"].clone()).unwrap(),
            120
        );
    }

    /// Rounding it would answer a type error with a wrong number. The call
    /// still fails, and its message still says what was wrong.
    #[test]
    fn a_real_fraction_is_left_alone() {
        assert_eq!(
            whole_floats_to_integers(json!({"ratio": 0.5})),
            json!({"ratio": 0.5})
        );
    }

    #[test]
    fn nothing_else_is_touched() {
        let original = json!({
            "path": r"D:\\Projects\\types.ts",
            "count": 7,
            "enabled": true,
            "missing": null,
        });
        assert_eq!(whole_floats_to_integers(original.clone()), original);
    }

    /// `as i64` saturates, so an out-of-range float would otherwise become
    /// `i64::MAX` — a number nobody wrote.
    #[test]
    fn an_out_of_range_float_keeps_its_own_value() {
        let repaired = whole_floats_to_integers(json!({"huge": 1e30}));
        assert_eq!(repaired["huge"].as_f64(), Some(1e30));
    }

    /// Why the whole tree can be rewritten without consulting each schema.
    #[test]
    fn a_float_field_still_reads_a_repaired_integer() {
        let repaired = whole_floats_to_integers(json!({"seconds": 30.0}));
        assert_eq!(
            serde_json::from_value::<f64>(repaired["seconds"].clone()).unwrap(),
            30.0
        );
    }
}
