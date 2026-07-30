//! The seams: traits core defines and ring 3 implements.
//!
//! Two of them are *domain* seams with more than one production implementation
//! ([`ticket_source`], [`workspace_provider`]); the rest are infrastructure
//! seams that exist so core can stay free of I/O. Dependency inversion means
//! the trait lives here and the implementation lives in `reeve-infra` — never
//! the reverse.

pub mod gh;
pub mod git;
pub mod index;
pub mod pty;
pub mod ticket_source;
pub mod vault;
pub mod workspace_provider;
