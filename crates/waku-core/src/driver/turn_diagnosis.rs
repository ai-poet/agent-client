//! Naming the cause of a turn that ended cleanly and said nothing.
//!
//! Fork addition. An ACP agent can answer `session/prompt` with a normal
//! `end_turn` after its own upstream request failed. The protocol says the
//! turn ended fine, the transcript holds nothing, and the user is left with
//! the empty-turn fallback line — "Turn completed" — which reads as success
//! and gives them nothing to go on. Grok Build does this; it is the shape any
//! swallowed provider failure takes.
//!
//! The evidence exists, it was just never read. Kimi writes a turn record
//! that upstream already consults. Every ACP agent writes to stderr, which
//! the driver already captures for the process-death path. This reads both,
//! and only for a turn that produced nothing, so a healthy turn pays nothing.

use parking_lot::Mutex;

/// The agent's recent stderr. The buffer is a ring, so a count of every line
/// ever written is kept beside it: that is what lets one turn read back its
/// own lines, however much came before or was dropped.
#[derive(Default)]
pub(super) struct ProviderStderr {
    lines: Vec<String>,
    written: usize,
}

impl ProviderStderr {
    const CAPACITY: usize = 128;

    pub(super) fn push(&mut self, line: String) {
        if self.lines.len() == Self::CAPACITY {
            self.lines.remove(0);
        }
        self.lines.push(line);
        self.written += 1;
    }

    /// Everything still held, for a caller that wants the whole tail.
    pub(super) fn tail(&self) -> Vec<String> {
        self.lines.clone()
    }

    /// A mark to read back from later.
    pub(super) fn mark(&self) -> usize {
        self.written
    }

    /// Everything written since `mark`, minus whatever the ring has dropped.
    fn since(&self, mark: usize) -> Vec<String> {
        let dropped = self.written - self.lines.len();
        let start = mark.saturating_sub(dropped).min(self.lines.len());
        self.lines[start..].to_vec()
    }
}

/// Why a turn that ended cleanly produced nothing, when the agent left a
/// trace of it. `kimi_turn` is the session id and the offset its wire log
/// stood at when the turn began, for the one provider that keeps such a
/// record; `stderr_mark` is where this turn's stderr starts.
///
/// Call only for a turn that produced no content: Kimi's lookup polls
/// briefly for a record that lands just after the response.
pub(super) fn empty_turn_failure(
    stderr: &Mutex<ProviderStderr>,
    stderr_mark: usize,
    kimi_turn: Option<(&str, u64)>,
) -> Option<String> {
    kimi_turn
        .and_then(|(session_id, offset)| crate::kimi_session::turn_failure(session_id, offset))
        .or_else(|| super::support::provider_stderr_error(stderr.lock().since(stderr_mark)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turns_own_stderr_names_the_failure() {
        let stderr = Mutex::new(ProviderStderr::default());
        stderr.lock().push("chatter from an earlier turn".into());
        let mark = stderr.lock().mark();
        stderr
            .lock()
            .push("error: xai request failed: 429 rate limited".into());

        let failure = empty_turn_failure(&stderr, mark, None).expect("stderr names it");
        assert!(failure.contains("429 rate limited"), "{failure}");
    }

    #[test]
    fn an_earlier_turns_error_is_not_borrowed_for_this_one() {
        let stderr = Mutex::new(ProviderStderr::default());
        stderr
            .lock()
            .push("error: something failed last turn".into());
        let mark = stderr.lock().mark();
        stderr.lock().push("just noise".into());

        assert!(empty_turn_failure(&stderr, mark, None).is_none());
    }

    #[test]
    fn read_back_survives_the_ring_wrapping_around() {
        let mut stderr = ProviderStderr::default();
        let start = stderr.mark();
        for index in 0..ProviderStderr::CAPACITY * 2 {
            stderr.push(format!("line {index}"));
        }
        // A mark older than anything still held returns what is left, not a
        // panic and not an empty slice.
        assert_eq!(stderr.since(start).len(), ProviderStderr::CAPACITY);

        let mark = stderr.mark();
        stderr.push("after the mark".into());
        assert_eq!(stderr.since(mark), vec!["after the mark".to_owned()]);
    }
}
