//! **reeve-core — ring 2.**
//!
//! The services, the shared domain components, the seam traits, and the single
//! error envelope. This crate depends on nothing of ours and on no I/O crate:
//! it does not know it is inside a Tauri app, and it never touches a file, a
//! process, or a socket. Everything it needs from the outside world arrives
//! through a trait in [`seams`], implemented by ring 3.
//!
//! Enforcement of that rule is physical, not cultural — see 05-lld-skeleton:
//! the `Cargo.toml` dependency list (layer 1), `clippy.toml` next to this file
//! (layer 2), and `src-tauri/tests/ring_rule.rs` (layer 3).

pub mod domain;
pub mod dto;
pub mod seams;
pub mod services;

#[cfg(feature = "fixtures")]
pub mod fixtures;

pub use dto::{ApiError, ApiErrorCode, ErrorDetails};
