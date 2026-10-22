//! What the bridge writes: one JSON object per line.
//!
//! Times are Unix milliseconds. A request's start is when the engine was
//! ready to send it — the previous step's end, a tool's end, a retry's wait —
//! since the engine does not report the moment a request leaves; it is
//! marked as inferred where it is shown. Large texts are kept as previews
//! with their full size alongside. A kind this build does not know reads as
//! [`Record::Unknown`], so an older desktop reads a newer file.

use serde::{Deserialize, Serialize};

/// The record format's version, in each segment's first line.
pub const SCHEMA: u32 = 1;

/// Token counts of one request, or of the session so far.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// The options a turn's requests went out with.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RequestOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

/// One tool of a catalog.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// What a request was for.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    #[default]
    Assistant,
    /// A sub-agent's request, under the parent's tool call.
    Subagent,
}

/// How a request ended.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    #[default]
    Running,
    Complete,
    Error,
    Cancelled,
}

/// One retry of a request.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RetryInfo {
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    pub delay_ms: u64,
    #[serde(default)]
    pub error: String,
}

/// Where a compaction is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPhase {
    Started,
    Finished,
    Failed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    /// A segment file's first line.
    Segment { schema: u32, session: String, at: i64 },
    /// The full system prompt, written when its fingerprint is new to the
    /// segment.
    System { fp: String, text: String },
    /// The full tool catalog, written when its fingerprint is new to the
    /// segment.
    Tools { fp: String, tools: Vec<ToolSpec> },
    /// A turn began: the person's prompt and the route its requests take.
    TurnStart {
        turn: u32,
        at: i64,
        prompt_preview: String,
        prompt_bytes: usize,
        #[serde(default)]
        images: usize,
        provider: String,
        model: String,
        #[serde(default)]
        api_base: String,
        #[serde(default)]
        wire_format: String,
        #[serde(default)]
        options: RequestOptions,
        system_fp: String,
        tools_fp: String,
    },
    /// One model request, written when it ends.
    Request {
        turn: u32,
        step: u32,
        #[serde(default)]
        purpose: Purpose,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_call_id: Option<String>,
        /// Inferred: when the engine was ready to send it.
        started_at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_token_at: Option<i64>,
        ended_at: i64,
        status: RequestStatus,
        #[serde(default)]
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(default)]
        model: String,
        #[serde(default)]
        text_preview: String,
        #[serde(default)]
        text_bytes: usize,
        #[serde(default)]
        thinking_preview: String,
        #[serde(default)]
        thinking_chars: usize,
        #[serde(default)]
        tool_calls: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        retries: Vec<RetryInfo>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    ToolStart {
        turn: u32,
        step: u32,
        call_id: String,
        name: String,
        args_preview: String,
        args_bytes: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_call_id: Option<String>,
        at: i64,
    },
    ToolEnd {
        call_id: String,
        is_error: bool,
        result_preview: String,
        result_bytes: usize,
        at: i64,
    },
    Compaction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<u32>,
        phase: CompactionPhase,
        automatic: bool,
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_after: Option<u64>,
        at: i64,
    },
    TurnEnd {
        turn: u32,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default)]
        session_usage: Usage,
        at: i64,
    },
    /// The conversation was rewound to its first `keep_turns` turns.
    Rewind { keep_turns: u32, at: i64 },
    /// This session was branched off `from` after its turn `through_turn`.
    Branched { from: String, through_turn: u32, at: i64 },
    /// Older segments were deleted to stay within the size cap.
    Dropped { segments: u32, at: i64 },
    #[serde(other)]
    Unknown,
}

/// The first `limit` bytes of `text`, cut on a character boundary.
pub fn preview(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// A short, stable fingerprint (FNV-1a, 64 bits) for telling prompts and
/// catalogs apart. Not a security property: only equality matters.
pub fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_unknown_kinds_are_tolerated() {
        let record = Record::ToolEnd {
            call_id: "c1".into(),
            is_error: false,
            result_preview: "ok".into(),
            result_bytes: 2,
            at: 5,
        };
        let line = serde_json::to_string(&record).unwrap();
        assert!(line.contains("\"type\":\"tool_end\""));
        assert_eq!(serde_json::from_str::<Record>(&line).unwrap(), record);
        let future: Record = serde_json::from_str(r#"{"type":"something_new","x":1}"#).unwrap();
        assert_eq!(future, Record::Unknown);
    }

    #[test]
    fn previews_cut_on_character_boundaries() {
        assert_eq!(preview("héllo", 2), "h");
        assert_eq!(preview("abc", 10), "abc");
    }

    #[test]
    fn fingerprints_tell_texts_apart() {
        assert_eq!(fingerprint("a"), fingerprint("a"));
        assert_ne!(fingerprint("a"), fingerprint("b"));
        assert_eq!(fingerprint("").len(), 16);
    }
}
