//! # escapement
//!
//! A Claude Code agent is not a persistent process — it exists only during a
//! turn. So there is nowhere to hang a callback, and the only wake primitive the
//! harness offers is *"a background task exited."* That gives you a **one-shot**
//! event listener, and a one-shot listener must be re-registered after it fires.
//!
//! An agent that goes idle without an armed listener suffers a **lost wakeup** —
//! the classic concurrency bug. `escapement` makes that impossible: a `Stop` hook
//! refuses to let the agent park until its listener is re-armed.
//!
//! Like a watch escapement, it locks the mechanism, releases exactly one impulse,
//! and re-locks.
//!
//! ## Pieces (each behind a feature)
//!
//! - [`hook`] (**default**) — the Stop hook that re-arms the listener.
//! - [`bus`] — one event source: a per-recipient long-poll queue over HTTPS.
//! - [`mcp`] — helpers for an MCP server that proxies to a local HTTP service.
//!
//! Binaries: `escapement-hook` (`hook`), `escapement-bus` (`bus`), and `duet`
//! (`mcp`) — the two-agent demo.

#[cfg(feature = "bus")]
pub mod bus;

#[cfg(feature = "hook")]
pub mod hook;

#[cfg(feature = "mcp")]
pub mod mcp;

/// Install the process-default rustls crypto provider (ring). Idempotent — extra
/// calls are ignored. Must run before any TLS client is built.
#[cfg(any(feature = "hook", feature = "mcp"))]
pub fn install_crypto() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
}
