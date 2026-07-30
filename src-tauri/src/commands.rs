//! One `#[tauri::command]` per operation in the 03-api catalog.
//!
//! A command unwraps its arguments, calls exactly one core service method, and
//! returns `Result<T, ApiError>`. Anything beyond that delegation is a design
//! bug (03-api): logic belongs to the service, not the adapter. Keeping the
//! layer this thin is what makes it mechanically replaceable by an HTTP layer.
//!
//! Commands arrive with the service ticket that implements them, and are
//! registered in `main.rs`'s `generate_handler!` in the same change.
