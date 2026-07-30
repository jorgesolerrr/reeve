//! The single error envelope every operation returns (03-api, "Error model").

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `Result<T, ApiError>` is the shape of every operation in the API.
///
/// Rules that hold for every value of this type (03-api):
///
/// - **Warnings are not errors** — advisories ride inside the *result*, never here.
/// - **No secrets** (NFR-2) — no environment values or tokens in `message` or `details`.
/// - **Unexpected failures collapse to [`ApiErrorCode::Internal`]** with a pointer to
///   the log file; a stacktrace never crosses the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// Stable and machine-readable. The UI branches on this, never on `message`.
    pub code: ApiErrorCode,
    /// Human and actionable: what happened, and what to do about it.
    pub message: String,
    /// Structured payload, present only when the UI must decide something with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ErrorDetails>,
}

/// The closed code enum, grouped by area (03-api).
///
/// Subsystem LLDs extend this list with the codes their area needs; the grouping
/// prefixes (`git/`, `source/`, `workspace/`, `fs/`, plus the bare `internal`)
/// are fixed by 03-api and do not grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ApiErrorCode {
    /// The working tree has changes that block the operation; `details` carries the files.
    #[serde(rename = "git/dirty")]
    GitDirty,
    /// A Source could not be reached. Local state is untouched; the user retries (NFR-5).
    #[serde(rename = "source/network")]
    SourceNetwork,
    /// A Run is live in the Workspace; the UI may offer to kill it.
    #[serde(rename = "workspace/run_active")]
    WorkspaceRunActive,
    /// A bug or a panic, collapsed. `message` points at the log file.
    #[serde(rename = "internal")]
    Internal,
}

/// The structured half of an error, discriminated by the code that carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ErrorDetails {
    /// Accompanies [`ApiErrorCode::GitDirty`].
    #[serde(rename_all = "camelCase")]
    DirtyFiles { files: Vec<String> },
    /// Accompanies [`ApiErrorCode::SourceNetwork`]: which Source operation failed.
    #[serde(rename_all = "camelCase")]
    SourceOperation { operation: String },
    /// Accompanies [`ApiErrorCode::WorkspaceRunActive`]: the Run holding the Workspace.
    #[serde(rename_all = "camelCase")]
    ActiveRun { run_id: String },
}

impl ApiError {
    /// An error with a code and a message and nothing else — the common case.
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// The collapse point for anything unexpected.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Internal, message)
    }

    /// Attach the structured payload the UI needs to decide.
    pub fn with_details(mut self, details: ErrorDetails) -> Self {
        self.details = Some(details);
        self
    }
}

impl fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::GitDirty => "git/dirty",
            Self::SourceNetwork => "source/network",
            Self::WorkspaceRunActive => "workspace/run_active",
            Self::Internal => "internal",
        };
        f.write_str(code)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire form of a code is the slash-grouped string, not the Rust variant
    /// name — the UI's `switch` is written against these literals.
    #[test]
    fn codes_serialize_to_their_grouped_wire_form() {
        let json = serde_json::to_string(&ApiErrorCode::WorkspaceRunActive).unwrap();
        assert_eq!(json, "\"workspace/run_active\"");
        assert_eq!(ApiErrorCode::GitDirty.to_string(), "git/dirty");
    }

    /// `details` is optional and absent by default: an error with nothing
    /// structured to say must not put a `null` on the wire.
    #[test]
    fn details_is_omitted_when_absent() {
        let err = ApiError::internal("see the log file");
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("details"), "{json}");
    }

    #[test]
    fn details_round_trips_when_present() {
        let err = ApiError::new(ApiErrorCode::GitDirty, "commit or stash first").with_details(
            ErrorDetails::DirtyFiles {
                files: vec!["docs/a.md".into()],
            },
        );
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(serde_json::from_str::<ApiError>(&json).unwrap(), err);
    }
}
