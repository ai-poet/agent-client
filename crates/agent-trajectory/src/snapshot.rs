//! The ledger: records folded into turns and cells.
//!
//! One cell per thing worth a row, in the order it happened: a change of
//! system prompt or tool catalog (dsh's `system` record), the person's
//! prompt (`user`), each model request (`message`), each tool call (`tool`,
//! or `subtool` inside a sub-agent), each compaction (`compacted`). Every
//! cell knows its turn and step, when it started and how long it took, so
//! the timeline and the inspector read the same numbers.
//!
//! Translated in shape from DeepSeek Harness's `ui-trajectory`
//! (`trajectory-record.ts`, `trajectory-snapshot-builder.ts`, `layout.ts`)
//! — MIT License, Copyright (c) 2026 DeepSeek. See NOTICE.md.

use std::collections::HashMap;

use crate::record::{
    CompactionPhase, FailureInfo, Purpose, Record, RequestOptions, RequestStatus, RetryInfo, ToolSpec,
    Usage,
};

/// What a cell is, after dsh's `TrajectoryCellKind`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CellKind {
    /// The system prompt or tool catalog changed.
    System,
    /// The person's prompt.
    User,
    /// A model request.
    Message,
    /// A tool call.
    Tool,
    /// A tool call inside a sub-agent.
    Subtool,
    /// A compaction.
    Compacted,
}

/// How a request's prompt differed from the one before it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptChange {
    Initial,
    System,
    Tools,
    SystemAndTools,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequestView {
    /// 1-based, across the session.
    pub number: u32,
    pub turn: u32,
    pub step: u32,
    pub purpose: Purpose,
    pub parent_call_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub api_base: String,
    pub wire_format: String,
    pub options: RequestOptions,
    pub started_at: i64,
    pub first_token_at: Option<i64>,
    pub ended_at: i64,
    pub status: RequestStatus,
    pub stop_reason: String,
    pub usage: Option<Usage>,
    /// The session's total once this request ended.
    pub cumulative: Usage,
    pub text: String,
    pub text_bytes: usize,
    pub thinking: String,
    pub thinking_chars: usize,
    pub tool_calls: u32,
    pub retries: Vec<RetryInfo>,
    pub error: Option<String>,
    /// Fork: the failure it ended its turn with, when it gave up.
    pub failure: Option<FailureInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolView {
    pub call_id: String,
    pub name: String,
    pub args: String,
    pub args_bytes: usize,
    pub result: Option<String>,
    pub result_bytes: usize,
    pub is_error: bool,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub parent_call_id: Option<String>,
    /// The tool's input schema in the catalog its request ran with.
    pub schema: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CellDetail {
    System {
        change: PromptChange,
        system_fp: String,
        tools_fp: String,
        previous_system_fp: Option<String>,
        previous_tools_fp: Option<String>,
    },
    User {
        prompt: String,
        bytes: usize,
        images: usize,
    },
    Request(Box<RequestView>),
    Tool(Box<ToolView>),
    Compaction {
        automatic: bool,
        phase: CompactionPhase,
        tokens_before: u64,
        tokens_after: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Position in [`Snapshot::cells`].
    pub index: usize,
    pub kind: CellKind,
    pub turn: Option<u32>,
    pub step: Option<u32>,
    /// One line for the ledger's content column.
    pub text: String,
    pub started_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub is_error: bool,
    pub running: bool,
    pub detail: CellDetail,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnView {
    pub turn: u32,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub success: Option<bool>,
    pub summary: Option<String>,
    /// Rewound away afterwards: no longer part of the conversation.
    pub rewound: bool,
    pub first_cell: usize,
    pub cell_count: usize,
    pub steps: u32,
    pub tool_calls: u32,
    pub usage: Usage,
    /// Fork: the goal this turn worked on resumes on its own after it.
    pub goal_resume: Option<GoalResumeView>,
}

/// Fork: a goal's automatic resume after a failed turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoalResumeView {
    pub attempt: u32,
    pub max_attempts: Option<u32>,
    pub delay_ms: u64,
    pub at: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub cells: Vec<Cell>,
    pub turns: Vec<TurnView>,
    /// Full system prompts, by fingerprint.
    pub systems: HashMap<String, String>,
    /// Full tool catalogs, by fingerprint.
    pub catalogs: HashMap<String, Vec<ToolSpec>>,
    /// The session this one was branched off, and after which turn.
    pub branched: Option<(String, u32)>,
    /// Segments deleted for the size cap.
    pub dropped: u32,
    pub session_usage: Usage,
    pub requests: u32,
}

impl Snapshot {
    /// The cell of a tool call, for the transcript's Inspect.
    pub fn cell_for_call(&self, call_id: &str) -> Option<usize> {
        self.cells.iter().position(|cell| {
            matches!(&cell.detail, CellDetail::Tool(tool) if tool.call_id == call_id)
        })
    }

    /// The system prompt and catalog a turn's requests ran with, and the
    /// ones before them (for the diff).
    pub fn prompt_of(&self, cell: &Cell) -> Option<(Option<&str>, Option<&str>, Option<&[ToolSpec]>, Option<&[ToolSpec]>)> {
        let CellDetail::System {
            system_fp,
            tools_fp,
            previous_system_fp,
            previous_tools_fp,
            ..
        } = &cell.detail
        else {
            return None;
        };
        Some((
            self.systems.get(system_fp).map(String::as_str),
            previous_system_fp
                .as_ref()
                .and_then(|fp| self.systems.get(fp))
                .map(String::as_str),
            self.catalogs.get(tools_fp).map(Vec::as_slice),
            previous_tools_fp
                .as_ref()
                .and_then(|fp| self.catalogs.get(fp))
                .map(Vec::as_slice),
        ))
    }
}

/// Everything a turn's requests share.
#[derive(Clone, Default)]
struct Route {
    provider: String,
    model: String,
    api_base: String,
    wire_format: String,
    options: RequestOptions,
    tools_fp: String,
}

fn one_line(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        flat
    } else {
        let cut: String = flat.chars().take(limit).collect();
        format!("{cut}…")
    }
}

/// Fold `records` into the ledger.
pub fn build(records: &[Record]) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let mut route = Route::default();
    let mut previous_prompt: Option<(String, String)> = None;
    let mut current_turn: Option<usize> = None;
    let mut open_compaction: Option<usize> = None;
    let mut tool_cells: HashMap<String, usize> = HashMap::new();

    for record in records {
        match record {
            Record::Segment { .. } | Record::Unknown => {}
            Record::System { fp, text } => {
                snapshot.systems.insert(fp.clone(), text.clone());
            }
            Record::Tools { fp, tools } => {
                snapshot.catalogs.insert(fp.clone(), tools.clone());
            }
            Record::Branched {
                from, through_turn, ..
            } => snapshot.branched = Some((from.clone(), *through_turn)),
            Record::Dropped { segments, .. } => snapshot.dropped += segments,
            Record::GoalResume {
                turn,
                attempt,
                max_attempts,
                delay_ms,
                at,
            } => {
                if let Some(view) = snapshot.turns.iter_mut().rev().find(|view| view.turn == *turn) {
                    view.goal_resume = Some(GoalResumeView {
                        attempt: *attempt,
                        max_attempts: *max_attempts,
                        delay_ms: *delay_ms,
                        at: *at,
                    });
                }
            }
            Record::Rewind { keep_turns, .. } => {
                for turn in &mut snapshot.turns {
                    if turn.turn > *keep_turns {
                        turn.rewound = true;
                    }
                }
            }
            Record::TurnStart {
                turn,
                at,
                prompt_preview,
                prompt_bytes,
                images,
                provider,
                model,
                api_base,
                wire_format,
                options,
                system_fp,
                tools_fp,
            } => {
                route = Route {
                    provider: provider.clone(),
                    model: model.clone(),
                    api_base: api_base.clone(),
                    wire_format: wire_format.clone(),
                    options: options.clone(),
                    tools_fp: tools_fp.clone(),
                };
                snapshot.turns.push(TurnView {
                    turn: *turn,
                    started_at: *at,
                    ended_at: None,
                    success: None,
                    summary: None,
                    rewound: false,
                    first_cell: snapshot.cells.len(),
                    cell_count: 0,
                    steps: 0,
                    tool_calls: 0,
                    usage: Usage::default(),
                    goal_resume: None,
                });
                current_turn = Some(snapshot.turns.len() - 1);
                let change = match &previous_prompt {
                    None => Some(PromptChange::Initial),
                    Some((system, tools)) => match (system != system_fp, tools != tools_fp) {
                        (true, true) => Some(PromptChange::SystemAndTools),
                        (true, false) => Some(PromptChange::System),
                        (false, true) => Some(PromptChange::Tools),
                        (false, false) => None,
                    },
                };
                if let Some(change) = change {
                    let (previous_system_fp, previous_tools_fp) = match &previous_prompt {
                        Some((system, tools)) => (Some(system.clone()), Some(tools.clone())),
                        None => (None, None),
                    };
                    push(&mut snapshot, current_turn, Cell {
                        index: 0,
                        kind: CellKind::System,
                        turn: Some(*turn),
                        step: None,
                        text: String::new(),
                        started_at: Some(*at),
                        duration_ms: None,
                        is_error: false,
                        running: false,
                        detail: CellDetail::System {
                            change,
                            system_fp: system_fp.clone(),
                            tools_fp: tools_fp.clone(),
                            previous_system_fp,
                            previous_tools_fp,
                        },
                    });
                }
                previous_prompt = Some((system_fp.clone(), tools_fp.clone()));
                push(&mut snapshot, current_turn, Cell {
                    index: 0,
                    kind: CellKind::User,
                    turn: Some(*turn),
                    step: None,
                    text: one_line(prompt_preview, 160),
                    started_at: Some(*at),
                    duration_ms: None,
                    is_error: false,
                    running: false,
                    detail: CellDetail::User {
                        prompt: prompt_preview.clone(),
                        bytes: *prompt_bytes,
                        images: *images,
                    },
                });
            }
            Record::Request {
                turn,
                step,
                purpose,
                parent_call_id,
                started_at,
                first_token_at,
                ended_at,
                status,
                stop_reason,
                usage,
                model,
                text_preview,
                text_bytes,
                thinking_preview,
                thinking_chars,
                tool_calls,
                retries,
                error,
                failure,
            } => {
                snapshot.requests += 1;
                if let Some(usage) = usage {
                    snapshot.session_usage.add(usage);
                    if let Some(index) = current_turn {
                        snapshot.turns[index].usage.add(usage);
                    }
                }
                if let Some(index) = current_turn
                    && *purpose == Purpose::Assistant
                {
                    snapshot.turns[index].steps += 1;
                }
                let view = RequestView {
                    number: snapshot.requests,
                    turn: *turn,
                    step: *step,
                    purpose: *purpose,
                    parent_call_id: parent_call_id.clone(),
                    provider: route.provider.clone(),
                    model: if model.is_empty() { route.model.clone() } else { model.clone() },
                    api_base: route.api_base.clone(),
                    wire_format: route.wire_format.clone(),
                    options: route.options.clone(),
                    started_at: *started_at,
                    first_token_at: *first_token_at,
                    ended_at: *ended_at,
                    status: *status,
                    stop_reason: stop_reason.clone(),
                    usage: *usage,
                    cumulative: snapshot.session_usage,
                    text: text_preview.clone(),
                    text_bytes: *text_bytes,
                    thinking: thinking_preview.clone(),
                    thinking_chars: *thinking_chars,
                    tool_calls: *tool_calls,
                    retries: retries.clone(),
                    error: error.clone(),
                    failure: failure.clone(),
                };
                let text = if !text_preview.trim().is_empty() {
                    one_line(text_preview, 160)
                } else if let Some(error) = error {
                    one_line(error, 160)
                } else {
                    String::new()
                };
                push(&mut snapshot, current_turn, Cell {
                    index: 0,
                    kind: CellKind::Message,
                    turn: Some(*turn),
                    step: Some(*step),
                    text,
                    started_at: Some(*started_at),
                    duration_ms: Some((ended_at - started_at).max(0)),
                    is_error: *status == RequestStatus::Error,
                    running: *status == RequestStatus::Running,
                    detail: CellDetail::Request(Box::new(view)),
                });
            }
            Record::ToolStart {
                turn,
                step,
                call_id,
                name,
                args_preview,
                args_bytes,
                parent_call_id,
                at,
            } => {
                if let Some(index) = current_turn
                    && parent_call_id.is_none()
                {
                    snapshot.turns[index].tool_calls += 1;
                }
                let schema = snapshot
                    .catalogs
                    .get(&route.tools_fp)
                    .and_then(|tools| tools.iter().find(|tool| tool.name == *name))
                    .map(|tool| tool.input_schema.clone());
                let index = push(&mut snapshot, current_turn, Cell {
                    index: 0,
                    kind: if parent_call_id.is_some() { CellKind::Subtool } else { CellKind::Tool },
                    turn: Some(*turn),
                    step: Some(*step),
                    text: format!("{name}  {}", one_line(args_preview, 120)),
                    started_at: Some(*at),
                    duration_ms: None,
                    is_error: false,
                    running: true,
                    detail: CellDetail::Tool(Box::new(ToolView {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        args: args_preview.clone(),
                        args_bytes: *args_bytes,
                        result: None,
                        result_bytes: 0,
                        is_error: false,
                        started_at: *at,
                        ended_at: None,
                        parent_call_id: parent_call_id.clone(),
                        schema,
                    })),
                });
                tool_cells.insert(call_id.clone(), index);
            }
            Record::ToolEnd {
                call_id,
                is_error,
                result_preview,
                result_bytes,
                at,
            } => {
                let Some(index) = tool_cells.remove(call_id) else {
                    continue;
                };
                let cell = &mut snapshot.cells[index];
                cell.running = false;
                cell.is_error = *is_error;
                if let Some(started) = cell.started_at {
                    cell.duration_ms = Some((at - started).max(0));
                }
                if let CellDetail::Tool(tool) = &mut cell.detail {
                    tool.result = Some(result_preview.clone());
                    tool.result_bytes = *result_bytes;
                    tool.is_error = *is_error;
                    tool.ended_at = Some(*at);
                }
            }
            Record::Compaction {
                turn,
                phase,
                automatic,
                tokens_before,
                tokens_after,
                at,
            } => match (phase, open_compaction.take()) {
                (CompactionPhase::Started, _) => {
                    let index = push(&mut snapshot, current_turn.filter(|_| turn.is_some()), Cell {
                        index: 0,
                        kind: CellKind::Compacted,
                        turn: *turn,
                        step: None,
                        text: String::new(),
                        started_at: Some(*at),
                        duration_ms: None,
                        is_error: false,
                        running: true,
                        detail: CellDetail::Compaction {
                            automatic: *automatic,
                            phase: *phase,
                            tokens_before: *tokens_before,
                            tokens_after: None,
                        },
                    });
                    open_compaction = Some(index);
                }
                (_, Some(index)) => {
                    let cell = &mut snapshot.cells[index];
                    cell.running = false;
                    cell.is_error = *phase == CompactionPhase::Failed;
                    if let Some(started) = cell.started_at {
                        cell.duration_ms = Some((at - started).max(0));
                    }
                    if let CellDetail::Compaction {
                        phase: cell_phase,
                        tokens_after: after,
                        ..
                    } = &mut cell.detail
                    {
                        *cell_phase = *phase;
                        *after = *tokens_after;
                    }
                }
                (_, None) => {}
            },
            Record::TurnEnd {
                turn,
                success,
                summary,
                at,
                ..
            } => {
                if let Some(index) = current_turn.take()
                    && snapshot.turns[index].turn == *turn
                {
                    let view = &mut snapshot.turns[index];
                    view.ended_at = Some(*at);
                    view.success = Some(*success);
                    view.summary = summary.clone();
                }
                // A tool still open when its turn ended was stopped with it.
                for index in tool_cells.drain().map(|(_, index)| index) {
                    snapshot.cells[index].running = false;
                }
            }
        }
    }
    snapshot
}

/// Append `cell`, numbering it and counting it in its turn.
fn push(snapshot: &mut Snapshot, turn: Option<usize>, mut cell: Cell) -> usize {
    let index = snapshot.cells.len();
    cell.index = index;
    snapshot.cells.push(cell);
    if let Some(turn) = turn {
        snapshot.turns[turn].cell_count = index + 1 - snapshot.turns[turn].first_cell;
    }
    index
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn turn_start(turn: u32, at: i64, system: &str, tools: &str) -> Record {
        Record::TurnStart {
            turn,
            at,
            prompt_preview: format!("prompt {turn}"),
            prompt_bytes: 8,
            images: 0,
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            api_base: "https://gw".into(),
            wire_format: "messages".into(),
            options: RequestOptions::default(),
            system_fp: system.into(),
            tools_fp: tools.into(),
        }
    }

    pub(crate) fn request(turn: u32, step: u32, started: i64, first: i64, ended: i64, output: u64) -> Record {
        Record::Request {
            turn,
            step,
            purpose: Purpose::Assistant,
            parent_call_id: None,
            started_at: started,
            first_token_at: Some(first),
            ended_at: ended,
            status: RequestStatus::Complete,
            stop_reason: "tool_use".into(),
            usage: Some(Usage {
                input: 100,
                output,
                cache_read: 10,
                cache_write: 0,
            }),
            model: String::new(),
            text_preview: "looking".into(),
            text_bytes: 7,
            thinking_preview: String::new(),
            thinking_chars: 0,
            tool_calls: 1,
            retries: Vec::new(),
            error: None,
            failure: None,
        }
    }

    pub(crate) fn tool(call: &str, step: u32, start: i64, end: i64) -> [Record; 2] {
        [
            Record::ToolStart {
                turn: 1,
                step,
                call_id: call.into(),
                name: "Read".into(),
                args_preview: "{\"path\":\"a\"}".into(),
                args_bytes: 12,
                parent_call_id: None,
                at: start,
            },
            Record::ToolEnd {
                call_id: call.into(),
                is_error: false,
                result_preview: "ok".into(),
                result_bytes: 2,
                at: end,
            },
        ]
    }

    pub(crate) fn sample() -> Vec<Record> {
        let mut records = vec![
            Record::System { fp: "s1".into(), text: "be helpful".into() },
            Record::Tools { fp: "t1".into(), tools: vec![ToolSpec {
                name: "Read".into(),
                description: String::new(),
                input_schema: serde_json::json!({"type": "object"}),
            }] },
            turn_start(1, 1_000, "s1", "t1"),
            request(1, 1, 1_000, 1_400, 2_000, 50),
        ];
        records.extend(tool("c1", 1, 2_000, 2_300));
        records.push(request(1, 2, 2_300, 2_500, 3_000, 30));
        records.push(Record::TurnEnd {
            turn: 1,
            success: true,
            summary: None,
            session_usage: Usage::default(),
            at: 3_000,
        });
        records
    }

    #[test]
    fn a_turn_folds_into_system_user_request_tool_request() {
        let snapshot = build(&sample());
        let kinds: Vec<CellKind> = snapshot.cells.iter().map(|cell| cell.kind).collect();
        assert_eq!(
            kinds,
            [CellKind::System, CellKind::User, CellKind::Message, CellKind::Tool, CellKind::Message]
        );
        let turn = &snapshot.turns[0];
        assert_eq!((turn.steps, turn.tool_calls, turn.cell_count), (2, 1, 5));
        assert_eq!(turn.success, Some(true));
        assert_eq!(snapshot.requests, 2);
        assert_eq!(snapshot.session_usage.output, 80);
        let CellDetail::Request(second) = &snapshot.cells[4].detail else {
            panic!("a request");
        };
        assert_eq!(second.number, 2);
        assert_eq!(second.cumulative.output, 80);
        assert_eq!(second.model, "claude-opus-5");
        let CellDetail::Tool(read) = &snapshot.cells[3].detail else {
            panic!("a tool");
        };
        assert_eq!(read.result.as_deref(), Some("ok"));
        assert!(read.schema.is_some());
        assert_eq!(snapshot.cells[3].duration_ms, Some(300));
        assert_eq!(snapshot.cell_for_call("c1"), Some(3));
    }

    #[test]
    fn a_prompt_change_gets_a_system_cell_and_an_unchanged_one_does_not() {
        let mut records = sample();
        records.push(turn_start(2, 4_000, "s1", "t1"));
        records.push(turn_start(3, 5_000, "s2", "t1"));
        records.push(turn_start(4, 6_000, "s3", "t2"));
        let snapshot = build(&records);
        let changes: Vec<PromptChange> = snapshot
            .cells
            .iter()
            .filter_map(|cell| match &cell.detail {
                CellDetail::System { change, .. } => Some(*change),
                _ => None,
            })
            .collect();
        assert_eq!(
            changes,
            [PromptChange::Initial, PromptChange::System, PromptChange::SystemAndTools]
        );
    }

    #[test]
    fn a_rewind_marks_the_turns_it_dropped() {
        let mut records = sample();
        records.push(turn_start(2, 4_000, "s1", "t1"));
        records.push(Record::Rewind { keep_turns: 1, at: 5_000 });
        records.push(turn_start(2, 6_000, "s1", "t1"));
        let snapshot = build(&records);
        let rewound: Vec<(u32, bool)> = snapshot.turns.iter().map(|turn| (turn.turn, turn.rewound)).collect();
        assert_eq!(rewound, [(1, false), (2, true), (2, false)]);
    }

    #[test]
    fn a_compaction_is_one_cell_from_start_to_finish() {
        let records = vec![
            Record::Compaction {
                turn: None,
                phase: CompactionPhase::Started,
                automatic: false,
                tokens_before: 900,
                tokens_after: None,
                at: 10,
            },
            Record::Compaction {
                turn: None,
                phase: CompactionPhase::Finished,
                automatic: false,
                tokens_before: 900,
                tokens_after: Some(100),
                at: 60,
            },
        ];
        let snapshot = build(&records);
        assert_eq!(snapshot.cells.len(), 1);
        let cell = &snapshot.cells[0];
        assert_eq!(cell.kind, CellKind::Compacted);
        assert_eq!(cell.duration_ms, Some(50));
        assert!(!cell.running);
        assert!(matches!(cell.detail, CellDetail::Compaction { tokens_after: Some(100), .. }));
    }

    #[test]
    fn a_goal_resume_is_kept_on_the_turn_it_follows() {
        let mut records = vec![turn_start(1, 0, "s1", "t1")];
        records.push(Record::TurnEnd {
            turn: 1,
            success: false,
            summary: Some("HTTP 529: Overloaded".into()),
            session_usage: Usage::default(),
            at: 100,
        });
        records.push(Record::GoalResume {
            turn: 1,
            attempt: 2,
            max_attempts: Some(5),
            delay_ms: 60_000,
            at: 101,
        });
        let snapshot = build(&records);
        assert_eq!(
            snapshot.turns[0].goal_resume,
            Some(GoalResumeView {
                attempt: 2,
                max_attempts: Some(5),
                delay_ms: 60_000,
                at: 101,
            })
        );
    }
}
