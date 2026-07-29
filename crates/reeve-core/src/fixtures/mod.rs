//! In-memory, deterministic implementations of the [`crate::seams`] traits.
//!
//! They live next to the traits they implement so a trait and its fixture move
//! together, and they sit behind the `fixtures` cargo feature so they can never
//! reach a production binary. Any crate that wants them takes reeve-core as a
//! dev-dependency with the feature on:
//!
//! ```toml
//! [dev-dependencies]
//! reeve-core = { workspace = true, features = ["fixtures"] }
//! ```
//!
//! Each subsystem LLD adds the fixture for the seam it defines.
