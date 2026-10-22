//! The session's trajectory: a timed record of every request and tool call.
//!
//! Fork addition, a port of DeepSeek Harness's Trajectory (the desktop draws
//! it, `src/app/trajectory_panel.rs`; the record and the view model are
//! `agent-trajectory`). dsh's agent loop writes these facts as durable
//! events; this engine reports them only as they happen, so the recorder
//! watches the same `QueryEvent`s `forward_events` decodes and stamps them:
//!
//! - a request starts when the engine was ready to send it — the turn's
//!   start, the previous request's end, a tool's end — since the engine does
//!   not report the moment it leaves; its first token is the first delta of
//!   any kind; it ends at `TurnComplete`, with that request's usage;
//! - a retry stays on the request it retries, so its wait counts toward the
//!   time to first token, as dsh counts it;
//! - the system prompt is rebuilt from the turn's config the way the engine
//!   builds it (`claurst_query::runner::prompt`), without the per-turn
//!   addenda; the desktop labels it as such.
//!
//! Records go to `agent-sessions/<id>.trajectory/` through a writer thread,
//! so observing never blocks the event loop. A session without a directory
//! (a one-shot prompt, a sub-agent of its own) records nothing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use agent_trajectory::record::{
    CompactionPhase, FailureInfo, Purpose, Record, RequestOptions, RequestStatus, RetryInfo, ToolSpec,
    Usage, fingerprint, preview,
};
use agent_trajectory::writer::Writer;
use claurst_core::types::{ContentBlock, Message, MessageContent, UsageInfo};
use claurst_query::{QueryConfig, QueryEvent};
use claurst_tools::Tool;
use parking_lot::Mutex;

/// Previews kept of a request's text, its thinking, and a tool's payload and
/// result.
const TEXT_PREVIEW: usize = 32 * 1024;
const THINKING_PREVIEW: usize = 8 * 1024;
const TOOL_PREVIEW: usize = 16 * 1024;

/// Where a session's trajectory lives, beside its transcript file.
pub fn directory(sessions_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir.join(format!("{session_id}.trajectory"))
}

/// The `trajectory.json` beside the engine's `settings.json` says whether
/// trajectories are recorded at all (`{"enabled": false}` turns them off).
/// Its own file, as AgentTeams' and auto-memory's are: the engine's save
/// drops keys it does not know.
pub fn enabled() -> bool {
    let Some(dir) = claurst_core::config::Settings::global_settings_path()
        .parent()
        .map(Path::to_path_buf)
    else {
        return true;
    };
    std::fs::read_to_string(dir.join("trajectory.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// Delete a session's trajectory along with its transcript.
pub fn delete(sessions_dir: &Path, session_id: &str) {
    agent_trajectory::writer::delete(&directory(sessions_dir, session_id));
}

/// Note in a branch's trajectory where it came from.
pub fn note_branch(sessions_dir: &Path, branch_id: &str, from: &str, through_turn: u32) {
    let at = now_ms();
    if let Ok(mut writer) = Writer::open(&directory(sessions_dir, branch_id), branch_id) {
        let _ = writer.append(
            &Record::Branched {
                from: from.to_owned(),
                through_turn,
                at,
            },
            at,
        );
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

enum Outbox {
    /// The prompt texts a turn's `TurnStart` names, written before it in any
    /// segment that does not have them yet.
    Prompt {
        system_fp: String,
        system: String,
        tools_fp: String,
        tools: Vec<ToolSpec>,
    },
    Record(Record, i64),
}

/// A request being answered.
#[derive(Default)]
struct Pending {
    started_at: i64,
    first_token_at: Option<i64>,
    model: String,
    text: String,
    text_bytes: usize,
    thinking: String,
    thinking_chars: usize,
    tool_calls: u32,
    retries: Vec<RetryInfo>,
    error: Option<String>,
    failure: Option<FailureInfo>,
}

#[derive(Default)]
struct Stream {
    /// When the next request is ready to go.
    boundary: i64,
    step: u32,
    pending: Option<Pending>,
}

#[derive(Default)]
struct State {
    turn: u32,
    main: Stream,
    /// Sub-agents' streams, by the parent's tool call.
    children: HashMap<String, Stream>,
}

pub struct Recorder {
    tx: Option<mpsc::Sender<Outbox>>,
    state: Mutex<State>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl Recorder {
    /// A recorder writing under `dir`, or one that records nothing.
    pub fn new(dir: Option<PathBuf>, session_id: &str) -> Self {
        let tx = dir.filter(|_| enabled()).and_then(|dir| spawn_writer(dir, session_id.to_owned()));
        Self::with(tx, Box::new(now_ms))
    }

    fn with(tx: Option<mpsc::Sender<Outbox>>, clock: Box<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            tx,
            state: Mutex::new(State::default()),
            clock,
        }
    }

    fn send(&self, message: Outbox) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(message);
        }
    }

    fn record(&self, record: Record) {
        let at = (self.clock)();
        self.send(Outbox::Record(record, at));
    }

    /// A turn starts: the person's message, and the route and prompt its
    /// requests use.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_turn(
        &self,
        turn: u32,
        message: Option<&Message>,
        provider: &str,
        api_base: &str,
        wire_format: &str,
        query: &QueryConfig,
        tools: &[Box<dyn Tool>],
    ) {
        if self.tx.is_none() {
            return;
        }
        let tools: Vec<claurst_core::types::ToolDefinition> =
            tools.iter().map(|tool| tool.to_definition()).collect();
        let (prompt, images) = message.map(prompt_of).unwrap_or_default();
        self.begin_turn_with(turn, &prompt, images, provider, api_base, wire_format, query, &tools);
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_turn_with(
        &self,
        turn: u32,
        prompt: &str,
        images: usize,
        provider: &str,
        api_base: &str,
        wire_format: &str,
        query: &QueryConfig,
        tools: &[claurst_core::types::ToolDefinition],
    ) {
        let at = (self.clock)();
        {
            let mut state = self.state.lock();
            state.turn = turn;
            state.main = Stream {
                boundary: at,
                ..Stream::default()
            };
            state.children.clear();
        }
        let system = system_prompt_of(query, tools);
        let tools: Vec<ToolSpec> = tools
            .iter()
            .map(|tool| ToolSpec {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect();
        let system_fp = fingerprint(&system);
        let tools_fp = fingerprint(&serde_json::to_string(&tools).unwrap_or_default());
        self.send(Outbox::Prompt {
            system_fp: system_fp.clone(),
            system,
            tools_fp: tools_fp.clone(),
            tools,
        });
        self.record(Record::TurnStart {
            turn,
            at,
            prompt_preview: preview(prompt, TEXT_PREVIEW),
            prompt_bytes: prompt.len(),
            images,
            provider: provider.to_owned(),
            model: query.model.clone(),
            api_base: api_base.to_owned(),
            wire_format: wire_format.to_owned(),
            options: RequestOptions {
                max_tokens: Some(query.max_tokens),
                effort: query
                    .effort_level
                    .map(|effort| format!("{effort:?}").to_ascii_lowercase()),
                thinking_budget: query.thinking_budget,
                temperature: query.temperature,
                context_window: query.context_window,
            },
            system_fp,
            tools_fp,
        });
    }

    /// One event of the turn's own stream.
    pub fn observe(&self, event: &QueryEvent) {
        if self.tx.is_none() {
            return;
        }
        let mut state = self.state.lock();
        let turn = state.turn;
        let records = observe_stream(&mut state.main, event, turn, None, (self.clock)());
        drop(state);
        for record in records {
            self.record(record);
        }
    }

    /// One event of a sub-agent started by the call `parent_call_id`.
    pub fn observe_child(&self, parent_call_id: &str, event: &QueryEvent) {
        if self.tx.is_none() {
            return;
        }
        let now = (self.clock)();
        let mut state = self.state.lock();
        let turn = state.turn;
        let stream = state
            .children
            .entry(parent_call_id.to_owned())
            .or_insert_with(|| Stream {
                boundary: now,
                ..Stream::default()
            });
        let records = observe_stream(stream, event, turn, Some(parent_call_id), now);
        drop(state);
        for record in records {
            self.record(record);
        }
    }

    /// A compaction asked for outside the loop (`/compact`).
    pub fn compaction(&self, phase: CompactionPhase, tokens_before: u64, tokens_after: Option<u64>) {
        let at = (self.clock)();
        self.record(Record::Compaction {
            turn: None,
            phase,
            automatic: false,
            tokens_before,
            tokens_after,
            at,
        });
    }

    /// The turn is over. A request still open was cut off by it.
    pub fn end_turn(&self, success: bool, summary: Option<String>, session_usage: Usage) {
        if self.tx.is_none() {
            return;
        }
        let now = (self.clock)();
        let mut state = self.state.lock();
        let turn = state.turn;
        let mut records = Vec::new();
        if let Some(record) = finish(&mut state.main, turn, None, now, if success {
            RequestStatus::Cancelled
        } else {
            RequestStatus::Error
        }, None)
        {
            records.push(record);
        }
        let children: Vec<(String, Stream)> = state.children.drain().collect();
        drop(state);
        for (parent, mut stream) in children {
            if let Some(record) = finish(&mut stream, turn, Some(&parent), now, RequestStatus::Cancelled, None) {
                records.push(record);
            }
        }
        records.push(Record::TurnEnd {
            turn,
            success,
            summary,
            session_usage,
            at: now,
        });
        for record in records {
            self.record(record);
        }
    }

    /// Fork: the turn just ended failed, and its goal resumes on its own
    /// after `delay_ms` (`session_goal_resume`).
    pub fn goal_resume(&self, attempt: u32, max_attempts: Option<u32>, delay_ms: u64) {
        if self.tx.is_none() {
            return;
        }
        let turn = self.state.lock().turn;
        let at = (self.clock)();
        self.record(Record::GoalResume {
            turn,
            attempt,
            max_attempts,
            delay_ms,
            at,
        });
    }

    /// The conversation was rewound to its first `keep_turns` turns.
    pub fn rewind(&self, keep_turns: u32) {
        let at = (self.clock)();
        self.record(Record::Rewind { keep_turns, at });
    }
}

/// What one event means for a stream, as records to write.
fn observe_stream(
    stream: &mut Stream,
    event: &QueryEvent,
    turn: u32,
    parent: Option<&str>,
    now: i64,
) -> Vec<Record> {
    use claurst_api::AnthropicStreamEvent as Frame;
    use claurst_api::streaming::ContentDelta as Delta;

    let mut records = Vec::new();
    match event {
        QueryEvent::Stream(Frame::MessageStart { model, .. }) => {
            // A retry's new attempt carries on the request it retries.
            let pending = stream.pending.get_or_insert_with(|| Pending {
                started_at: stream.boundary,
                ..Pending::default()
            });
            pending.model = model.clone();
        }
        QueryEvent::Stream(Frame::ContentBlockStart { content_block, .. }) => {
            let pending = open(stream, now);
            if matches!(content_block, ContentBlock::ToolUse { .. }) {
                pending.tool_calls += 1;
                pending.first_token_at.get_or_insert(now);
            }
        }
        QueryEvent::Stream(Frame::ContentBlockDelta { delta, .. }) => {
            let pending = open(stream, now);
            pending.first_token_at.get_or_insert(now);
            match delta {
                Delta::TextDelta { text } => {
                    pending.text_bytes += text.len();
                    if pending.text.len() < TEXT_PREVIEW {
                        pending.text.push_str(text);
                    }
                }
                Delta::ThinkingDelta { thinking } => {
                    pending.thinking_chars += thinking.chars().count();
                    if pending.thinking.len() < THINKING_PREVIEW {
                        pending.thinking.push_str(thinking);
                    }
                }
                _ => {}
            }
        }
        QueryEvent::Stream(Frame::Error { message, .. }) => {
            open(stream, now).error = Some(message.clone());
        }
        QueryEvent::Retry(notice) => {
            let pending = open(stream, now);
            pending.retries.push(RetryInfo {
                attempt: notice.attempt,
                max_attempts: notice.max_attempts,
                delay_ms: notice.delay_ms,
                error: notice.detail.clone(),
                reason: notice.reason.as_str().to_owned(),
                discarded_output: notice.discarded_output,
                request_id: notice.request_id.clone(),
            });
            if notice.discarded_output {
                pending.text.clear();
                pending.text_bytes = 0;
                pending.thinking.clear();
                pending.thinking_chars = 0;
                pending.tool_calls = 0;
            }
            pending.first_token_at = None;
        }
        // Fork: the failure the turn is about to end with. The request it
        // ended is closed by the turn's end; a request refused before it
        // streamed anything gets its record here too.
        QueryEvent::RequestFailed(failure) => {
            let pending = open(stream, now);
            pending.error = Some(failure.to_string());
            pending.failure = Some(FailureInfo {
                class: failure.class.as_str().to_owned(),
                reason: failure.reason.as_str().to_owned(),
                status: failure.status,
                code: failure.code.clone(),
                request_id: failure.request_id.clone(),
            });
        }
        QueryEvent::TurnComplete { stop_reason, usage, .. } => {
            if let Some(record) = finish(
                stream,
                turn,
                parent,
                now,
                RequestStatus::Complete,
                Some((stop_reason.clone(), usage.as_ref().map(usage_of))),
            ) {
                records.push(record);
            }
        }
        QueryEvent::ToolStart {
            tool_name,
            tool_id,
            input_json,
        } => {
            records.push(Record::ToolStart {
                turn,
                step: stream.step,
                call_id: tool_id.clone(),
                name: tool_name.clone(),
                args_preview: preview(input_json, TOOL_PREVIEW),
                args_bytes: input_json.len(),
                parent_call_id: parent.map(str::to_owned),
                at: now,
            });
        }
        QueryEvent::ToolEnd {
            tool_id,
            result,
            is_error,
            ..
        } => {
            stream.boundary = now;
            records.push(Record::ToolEnd {
                call_id: tool_id.clone(),
                is_error: *is_error,
                result_preview: preview(result, TOOL_PREVIEW),
                result_bytes: result.len(),
                at: now,
            });
        }
        QueryEvent::Compaction {
            phase,
            automatic,
            tokens_before,
            tokens_after,
        } if parent.is_none() => {
            if !matches!(phase, claurst_query::CompactionPhase::Started) {
                stream.boundary = now;
            }
            records.push(Record::Compaction {
                turn: Some(turn),
                phase: match phase {
                    claurst_query::CompactionPhase::Started => CompactionPhase::Started,
                    claurst_query::CompactionPhase::Finished => CompactionPhase::Finished,
                    claurst_query::CompactionPhase::Failed => CompactionPhase::Failed,
                },
                automatic: *automatic,
                tokens_before: *tokens_before,
                tokens_after: *tokens_after,
                at: now,
            });
        }
        QueryEvent::Error(message) => {
            if let Some(record) = finish(stream, turn, parent, now, RequestStatus::Error, None) {
                records.push(record);
            } else if let Some(pending) = stream.pending.as_mut() {
                pending.error = Some(message.clone());
            }
        }
        _ => {}
    }
    records
}

/// The request being answered, opened at the stream's boundary if no
/// `MessageStart` came first.
fn open(stream: &mut Stream, now: i64) -> &mut Pending {
    let boundary = stream.boundary.min(now);
    stream.pending.get_or_insert_with(|| Pending {
        started_at: boundary,
        ..Pending::default()
    })
}

/// Close the open request, if any.
fn finish(
    stream: &mut Stream,
    turn: u32,
    parent: Option<&str>,
    now: i64,
    status: RequestStatus,
    ended: Option<(String, Option<Usage>)>,
) -> Option<Record> {
    let pending = stream.pending.take()?;
    stream.step += 1;
    stream.boundary = now;
    let (stop_reason, usage) = ended.unwrap_or_default();
    let status = if pending.error.is_some() && status != RequestStatus::Complete {
        RequestStatus::Error
    } else {
        status
    };
    Some(Record::Request {
        turn,
        step: stream.step,
        purpose: if parent.is_some() {
            Purpose::Subagent
        } else {
            Purpose::Assistant
        },
        parent_call_id: parent.map(str::to_owned),
        started_at: pending.started_at,
        first_token_at: pending.first_token_at,
        ended_at: now,
        status,
        stop_reason,
        usage,
        model: pending.model,
        text_preview: pending.text,
        text_bytes: pending.text_bytes,
        thinking_preview: pending.thinking,
        thinking_chars: pending.thinking_chars,
        tool_calls: pending.tool_calls,
        retries: pending.retries,
        error: pending.error,
        failure: pending.failure,
    })
}

/// The text of the person's message and how many pictures came with it.
fn prompt_of(message: &Message) -> (String, usize) {
    match &message.content {
        MessageContent::Text(text) => (text.clone(), 0),
        MessageContent::Blocks(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let images = blocks
                .iter()
                .filter(|block| matches!(block, ContentBlock::Image { .. }))
                .count();
            (text, images)
        }
    }
}

/// Everything the engine's cost tracker counted this session.
pub(crate) fn session_usage(tracker: &claurst_core::CostTracker) -> Usage {
    Usage {
        input: tracker.input_tokens(),
        output: tracker.output_tokens(),
        cache_read: tracker.cache_read_tokens(),
        cache_write: tracker.cache_creation_tokens(),
    }
}

pub(crate) fn compaction_phase(phase: &crate::events::CompactionPhase) -> CompactionPhase {
    match phase {
        crate::events::CompactionPhase::Started => CompactionPhase::Started,
        crate::events::CompactionPhase::Finished => CompactionPhase::Finished,
        crate::events::CompactionPhase::Failed => CompactionPhase::Failed,
    }
}

fn usage_of(usage: &UsageInfo) -> Usage {
    Usage {
        input: usage.input_tokens,
        output: usage.output_tokens,
        cache_read: usage.cache_read_input_tokens,
        cache_write: usage.cache_creation_input_tokens,
    }
}

/// The system prompt the turn's requests carry, built the way the engine
/// builds it from the same config, without the per-turn addenda.
fn system_prompt_of(query: &QueryConfig, tools: &[claurst_core::types::ToolDefinition]) -> String {
    use claurst_core::system_prompt::SystemPromptOptions;
    let options = SystemPromptOptions {
        custom_system_prompt: query.system_prompt.clone(),
        append_system_prompt: query.append_system_prompt.clone(),
        output_style: query.output_style,
        custom_output_style_prompt: query.output_style_prompt.clone(),
        working_directory: query.working_directory.clone(),
        enabled_tools: Some(
            query
                .enabled_tools
                .clone()
                .unwrap_or_else(|| tools.iter().map(|tool| tool.name.clone()).collect()),
        ),
        ..Default::default()
    };
    claurst_core::system_prompt::build_system_prompt(&options)
}

/// The writer thread: owns the files, so observing never waits on them.
fn spawn_writer(dir: PathBuf, session: String) -> Option<mpsc::Sender<Outbox>> {
    let (tx, rx) = mpsc::channel::<Outbox>();
    std::thread::Builder::new()
        .name(format!("waku-trajectory-{session}"))
        .spawn(move || {
            let Ok(mut writer) = Writer::open(&dir, &session) else {
                return;
            };
            let mut segment = writer.segment();
            // Prompt texts this segment already holds, and the current turn's.
            let mut known: HashSet<String> = HashSet::new();
            let mut current: Vec<Record> = Vec::new();
            for message in rx {
                match message {
                    Outbox::Prompt {
                        system_fp,
                        system,
                        tools_fp,
                        tools,
                    } => {
                        current = vec![
                            Record::System {
                                fp: system_fp,
                                text: system,
                            },
                            Record::Tools { fp: tools_fp, tools },
                        ];
                    }
                    Outbox::Record(record, at) => {
                        if matches!(record, Record::TurnStart { .. }) {
                            write_prompts(&mut writer, &current, &mut known, at);
                        }
                        let _ = writer.append(&record, at);
                        if writer.segment() != segment {
                            // A new segment starts knowing nothing: give it the
                            // prompts of the turn in progress.
                            segment = writer.segment();
                            known.clear();
                            write_prompts(&mut writer, &current, &mut known, at);
                        }
                    }
                }
            }
        })
        .ok()?;
    Some(tx)
}

fn write_prompts(writer: &mut Writer, prompts: &[Record], known: &mut HashSet<String>, at: i64) {
    for prompt in prompts {
        let fp = match prompt {
            Record::System { fp, .. } | Record::Tools { fp, .. } => fp,
            _ => continue,
        };
        if known.insert(fp.clone()) {
            let _ = writer.append(prompt, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use claurst_api::AnthropicStreamEvent as Frame;
    use claurst_api::streaming::ContentDelta;

    use super::*;

    fn recorder() -> (Recorder, mpsc::Receiver<Outbox>, Arc<AtomicI64>) {
        let (tx, rx) = mpsc::channel();
        let clock = Arc::new(AtomicI64::new(1_000));
        let tick = clock.clone();
        let recorder = Recorder::with(Some(tx), Box::new(move || tick.load(Ordering::SeqCst)));
        (recorder, rx, clock)
    }

    fn records(rx: &mpsc::Receiver<Outbox>) -> Vec<Record> {
        rx.try_iter()
            .filter_map(|message| match message {
                Outbox::Record(record, _) => Some(record),
                Outbox::Prompt { .. } => None,
            })
            .collect()
    }

    fn text(text: &str) -> QueryEvent {
        QueryEvent::Stream(Frame::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta { text: text.into() },
        })
    }

    fn start() -> QueryEvent {
        QueryEvent::Stream(Frame::MessageStart {
            id: "m".into(),
            model: "claude-opus-5".into(),
            usage: UsageInfo::default(),
        })
    }

    fn complete(output: u64) -> QueryEvent {
        QueryEvent::TurnComplete {
            turn: 1,
            stop_reason: "end_turn".into(),
            usage: Some(UsageInfo {
                input_tokens: 10,
                output_tokens: output,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 5,
            }),
        }
    }

    #[test]
    fn a_request_is_timed_from_its_boundary_to_its_completion() {
        let (recorder, rx, clock) = recorder();
        recorder.begin_turn_with(1, "fix it", 0, "anthropic", "https://gw", "messages", &QueryConfig::default(), &[]);
        clock.store(1_300, Ordering::SeqCst);
        recorder.observe(&start());
        clock.store(1_500, Ordering::SeqCst);
        recorder.observe(&text("hello"));
        clock.store(2_000, Ordering::SeqCst);
        recorder.observe(&complete(40));
        let all = records(&rx);
        assert!(matches!(all[0], Record::TurnStart { turn: 1, at: 1_000, .. }));
        match &all[1] {
            Record::Request {
                step,
                started_at,
                first_token_at,
                ended_at,
                usage,
                text_preview,
                model,
                ..
            } => {
                assert_eq!((*step, *started_at, *first_token_at, *ended_at), (1, 1_000, Some(1_500), 2_000));
                assert_eq!(usage.unwrap().output, 40);
                assert_eq!(usage.unwrap().cache_read, 5);
                assert_eq!(text_preview, "hello");
                assert_eq!(model, "claude-opus-5");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tool_call_sets_the_next_requests_start_and_a_retry_stays_on_its_request() {
        let (recorder, rx, clock) = recorder();
        recorder.begin_turn_with(1, "go", 0, "anthropic", "", "messages", &QueryConfig::default(), &[]);
        recorder.observe(&start());
        recorder.observe(&text("x"));
        clock.store(1_100, Ordering::SeqCst);
        recorder.observe(&complete(1));
        recorder.observe(&QueryEvent::ToolStart {
            tool_name: "Read".into(),
            tool_id: "c1".into(),
            input_json: "{}".into(),
        });
        clock.store(1_400, Ordering::SeqCst);
        recorder.observe(&QueryEvent::ToolEnd {
            tool_name: "Read".into(),
            tool_id: "c1".into(),
            result: "ok".into(),
            is_error: false,
            metadata: None,
        });
        clock.store(1_450, Ordering::SeqCst);
        recorder.observe(&start());
        recorder.observe(&QueryEvent::Retry(claurst_query::RetryNotice {
            attempt: 1,
            max_attempts: Some(10),
            delay_ms: 2_000,
            reason: claurst_api::retry_policy::RetryReason::Overloaded,
            discarded_output: false,
            detail: "overloaded".into(),
            request_id: None,
        }));
        clock.store(3_600, Ordering::SeqCst);
        recorder.observe(&start());
        recorder.observe(&text("done"));
        clock.store(3_900, Ordering::SeqCst);
        recorder.observe(&complete(2));
        recorder.end_turn(true, None, Usage::default());
        let all = records(&rx);
        let second = all
            .iter()
            .filter_map(|record| match record {
                Record::Request { step: 2, started_at, first_token_at, retries, .. } => {
                    Some((*started_at, *first_token_at, retries.len()))
                }
                _ => None,
            })
            .next()
            .unwrap();
        assert_eq!(second, (1_400, Some(3_600), 1));
        assert!(matches!(all.last(), Some(Record::TurnEnd { success: true, .. })));
    }

    #[test]
    fn a_retry_records_its_reason_and_discarded_output() {
        let (recorder, rx, _) = recorder();
        recorder.begin_turn_with(1, "go", 0, "anthropic", "", "messages", &QueryConfig::default(), &[]);
        recorder.observe(&start());
        recorder.observe(&text("half an ans"));
        recorder.observe(&QueryEvent::Retry(claurst_query::RetryNotice {
            attempt: 1,
            max_attempts: Some(10),
            delay_ms: 2_000,
            reason: claurst_api::retry_policy::RetryReason::StreamInterrupted,
            discarded_output: true,
            detail: "connection reset".into(),
            request_id: Some("req_1".into()),
        }));
        recorder.observe(&start());
        recorder.observe(&text("whole"));
        recorder.observe(&complete(1));
        recorder.end_turn(true, None, Usage::default());
        let retry = records(&rx)
            .into_iter()
            .find_map(|record| match record {
                Record::Request { retries, text_preview, .. } => Some((retries, text_preview)),
                _ => None,
            })
            .unwrap();
        assert_eq!(retry.1, "whole");
        assert_eq!(retry.0[0].reason, "stream_interrupted");
        assert!(retry.0[0].discarded_output);
        assert_eq!(retry.0[0].request_id.as_deref(), Some("req_1"));
    }

    #[test]
    fn a_failed_request_keeps_its_failure_and_request_id() {
        let (recorder, rx, _) = recorder();
        recorder.begin_turn_with(1, "go", 0, "anthropic", "", "messages", &QueryConfig::default(), &[]);
        // Refused before anything streamed: the failure alone opens it.
        recorder.observe(&QueryEvent::RequestFailed(claurst_api::retry_policy::ModelFailure {
            code: Some("permission_error".into()),
            request_id: Some("req_refused".into()),
            ..claurst_api::retry_policy::ModelFailure::fatal(Some(403), "no access to this model")
        }));
        recorder.end_turn(false, Some("HTTP 403".into()), Usage::default());
        let (status, error, failure) = records(&rx)
            .into_iter()
            .find_map(|record| match record {
                Record::Request { status, error, failure, .. } => Some((status, error, failure)),
                _ => None,
            })
            .expect("the refused request has a record");
        assert_eq!(status, RequestStatus::Error);
        assert_eq!(error.as_deref(), Some("HTTP 403: no access to this model"));
        let failure = failure.unwrap();
        assert_eq!(failure.class, "fatal");
        assert_eq!(failure.status, Some(403));
        assert_eq!(failure.code.as_deref(), Some("permission_error"));
        assert_eq!(failure.request_id.as_deref(), Some("req_refused"));
    }

    #[test]
    fn a_goal_resume_is_recorded_after_its_turn() {
        let (recorder, rx, _) = recorder();
        recorder.begin_turn_with(3, "go", 0, "anthropic", "", "messages", &QueryConfig::default(), &[]);
        recorder.end_turn(false, Some("Overloaded".into()), Usage::default());
        recorder.goal_resume(1, Some(5), 30_000);
        let all = records(&rx);
        assert!(matches!(
            all.last(),
            Some(Record::GoalResume { turn: 3, attempt: 1, max_attempts: Some(5), delay_ms: 30_000, .. })
        ), "{all:?}");
    }

    #[test]
    fn a_recorder_without_a_directory_records_nothing() {
        let recorder = Recorder::with(None, Box::new(|| 0));
        recorder.begin_turn(1, None, "a", "", "messages", &QueryConfig::default(), &[]);
        recorder.observe(&start());
        recorder.end_turn(true, None, Usage::default());
        assert!(recorder.state.lock().main.pending.is_none());
    }
}
