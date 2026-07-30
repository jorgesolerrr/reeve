//! The DTOs that cross the API boundary, and the one error envelope they share.
//!
//! Every type here derives `TS`; the frontend sees it only after it is listed
//! in the explicit registry in `src-tauri/src/bin/generate_types.rs`. Nothing
//! reaches `app/src/generated/types.ts` by accident.

mod error;

pub use error::{ApiError, ApiErrorCode, ErrorDetails};
