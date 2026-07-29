//! The event emitter: four events, closed list (03-api, "Events catalog").
//!
//! - `graph_changed { paths }` and `workspace_changed { ticketId }` carry
//!   **scope, not data** — the UI re-queries whatever it has on screen.
//! - `run_exited { project, ticketId, runKind, exitCode }` is the one event
//!   carrying an extra datum, so the UI can react without a re-query.
//! - `pty_output` is the declared exception: high-frequency terminal data going
//!   straight to xterm.js, never through the query cache.
//!
//! Emitters arrive with the subsystem ticket that owns the fact being announced.
