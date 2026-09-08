//! The HTTP client the Anthropic path runs on.
//!
//! # Fork change: no TLS impersonation
//!
//! Upstream Claurst built this client on `wreq`/BoringSSL so its ClientHello
//! matched the official Claude Code client (Bun), which is what lets a
//! Claude.ai Pro/Max OAuth token reach the API directly. Waku routes every
//! request through the managed gateway with an ordinary API key, so there is
//! no first-party client to imitate and nothing that inspects the
//! fingerprint.
//!
//! Dropping it removes a second TLS stack (BoringSSL alongside the rustls
//! that GPUI already links) and, with it, the C toolchain that stack needs on
//! every contributor's machine and CI runner. `reqwest` exposes the same
//! request builder, so the call sites are unchanged.
//!
//! The module keeps its name so a `git subtree pull` from Claurst upstream
//! conflicts here — in this one file — rather than silently reintroducing the
//! dependency across `lib.rs`.

use std::time::Duration;

/// The client every Anthropic-path request is made with.
///
/// HTTP/1.1 only: the streaming decoder in `lib.rs` reads an SSE byte stream
/// and never benefited from multiplexing, and pinning the version keeps the
/// wire behaviour identical to what the upstream client produced.
pub fn build_anthropic_client(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .http1_only()
        .timeout(timeout)
        .build()
}
