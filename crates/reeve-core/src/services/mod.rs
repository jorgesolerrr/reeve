//! The nine services, 1:1 with the API areas of 03-api.
//!
//! A service owns its area's operations and nothing else; cross-area logic that
//! two services need lives in [`crate::domain`]. Interiors are filled in by the
//! subsystem LLDs — each module below names the one that owns it.

pub mod epics;
pub mod graph;
pub mod projects;
pub mod review;
pub mod runs;
pub mod sources;
pub mod system;
pub mod tickets;
pub mod workspaces;
