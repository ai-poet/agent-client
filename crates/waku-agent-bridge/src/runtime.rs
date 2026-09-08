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

/// Worker threads for the engine. Turns are I/O bound (one streaming HTTP
/// request plus tool subprocesses), so this is about concurrent sessions and
/// parallel tool execution rather than CPU parallelism. Four is enough for
/// several live sessions without competing with GPUI's own pools in the
/// desktop process — and the daemon, where this actually runs, has no
/// rendering to protect.
const WORKER_THREADS: usize = 4;

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
