//! The one Tokio runtime the vendored engine runs on.
//!
//! Waku's daemon is a synchronous, thread-per-connection server; the engine is
//! `async` throughout. Rather than colouring the daemon, every Native session
//! borrows a single multi-threaded runtime created on first use and kept for
//! the life of the process.
//!
//! It has to be multi-threaded for a specific reason, not just for throughput:
//! [`crate::permission`] blocks a worker while the user decides on a tool, and
//! [`tokio::task::block_in_place`] — the only way to do that without stalling
//! every other task on the thread — panics on a current-thread runtime.

use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Worker threads for the engine.
///
/// The number that matters is not throughput — turns are I/O bound — but how
/// many approvals and questions can be *pending at once*. Each one parks the
/// worker its tool is running on (`block_in_place` moves the worker's other
/// tasks away, but the thread itself is spent until the user answers), so
/// with N workers the N+1th session to raise a dialog would stall until one
/// of the others is answered. Sixteen idle threads cost nothing measurable;
/// a session that cannot start because four dialogs are open would be a bug
/// report.
const WORKER_THREADS: usize = 16;

/// Borrow the shared runtime, creating it on first call.
///
/// Returns an error rather than panicking: a runtime that cannot be built
/// (thread limits, a sandbox refusing to spawn) should surface as a failed
/// session start with a readable message, not as a crashed daemon.
pub fn shared() -> anyhow::Result<&'static Runtime> {
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let runtime = Builder::new_multi_thread()
        .worker_threads(WORKER_THREADS)
        .thread_name("waku-agent")
        .enable_all()
        .build()?;
    // A racing caller may have won; theirs is just as good.
    Ok(RUNTIME.get_or_init(|| runtime))
}
