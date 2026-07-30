//! **reeve-infra — ring 3.**
//!
//! Five modules, each implementing the [`reeve_core::seams`] trait it serves.
//! This is the only crate allowed to name `rusqlite` or `portable_pty`, and the
//! only one that talks to the filesystem, to git, or to a process — enforced by
//! `src-tauri/tests/ring_rule.rs`.
//!
//! The arrow points inward: infra depends on core so that core does not depend
//! on infra. Nothing here is referenced by name outside the composition root.

pub mod gh_client;
pub mod git;
pub mod index;
pub mod pty;
pub mod vault;
