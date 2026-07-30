//! The vault seam: Markdown I/O, front-matter, wiki-links, materialized regions.
//!
//! Signature fixed by 06-lld-graph.
//!
//! The vault is the only code in reeve that parses Markdown, and the only door
//! core has to the repository's files. Everything here is a *value*: the trait
//! hands back parsed data, never a handle, a file, or a `Path` into the repo.
//!
//! Implemented by `reeve-infra::vault`; the in-memory double lives in
//! [`crate::fixtures::vault`].

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Markdown I/O for one Project's repository.
///
/// Every method takes `project` — the absolute repository path, which *is* the
/// Project's identity (03-api) — plus a **repo-relative** `path` with `/`
/// separators on every OS (06's `nodes.path` form). Nothing is cached: disk is
/// truth, and every read hits it.
pub trait Vault: Send + Sync {
    /// Read a Node: raw bytes as text, parsed front-matter, derived title, and
    /// the wiki-links with their positions — one parse, everything the index
    /// stores (06, "Extraction rules").
    ///
    /// Errors with [`VaultError::NotANode`] for a path whose extension is not a
    /// Node kind: reeve opens those with the system application, never here.
    fn read_node(&self, project: &Path, path: &str) -> Result<RawDoc, VaultError>;

    /// Write a file **verbatim** — content authorship (`save_doc`, `create_doc`).
    /// Missing parent directories are created; the write is atomic, so a reader
    /// (or the watcher) never observes a half-written file.
    fn write_doc(&self, project: &Path, path: &str, content: &str) -> Result<(), VaultError>;

    /// Apply a structural act to the front-matter block, leaving the body
    /// **byte-identical** (06). Unknown keys survive; YAML comments inside the
    /// block do not — the documented trade-off against line-surgery on YAML.
    fn patch_front_matter(
        &self,
        project: &Path,
        path: &str,
        patch: FrontMatterPatch,
    ) -> Result<(), VaultError>;

    /// Replace the content between a region's markers, touching no other byte
    /// of the file. Semantics fixed by 09-lld-sources; the markers travel in
    /// [`RegionContent`] because the primitive is generic — a future region
    /// owner brings its own.
    fn rewrite_region(
        &self,
        project: &Path,
        path: &str,
        region: RegionContent,
    ) -> Result<(), VaultError>;

    /// Create the committed `.reeve/` layout (02) if absent. **Idempotent, and
    /// never destructive**: an existing `config.json` or `counters.json` is left
    /// exactly as it is.
    fn scaffold(&self, project: &Path) -> Result<(), VaultError>;
}

/// The three Node kinds of v1 (02), identified by file extension.
///
/// Only [`NodeKind::Markdown`] contributes outgoing edges (FR-2.5); the other
/// two are linkable leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Markdown,
    Excalidraw,
    Html,
}

impl NodeKind {
    /// Classify a path by extension, `None` when the file is not a Node at all.
    ///
    /// This is the one place the extension list lives: the index's walk filters
    /// candidate files with it, and `read_node` refuses what it rejects.
    pub fn from_path(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (_, ext) = name.rsplit_once('.')?;
        match ext.to_ascii_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "excalidraw" => Some(Self::Excalidraw),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }
}

/// A Node's **name**: the basename without extension (02). For Tickets and
/// Epics this is the id, which is why it lives beside [`NodeKind`] rather than
/// inside the implementation — the index derives it for every walked file.
pub fn node_name(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => base.to_string(),
    }
}

/// A Node as the vault sees it: the file plus everything one parse yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDoc {
    /// Repo-relative, `/`-separated — the index's primary key.
    pub path: String,
    /// Basename without extension. The Node's identity; for Tickets and Epics
    /// it *is* the id (02).
    pub name: String,
    pub kind: NodeKind,
    /// Front-matter `title` → first H1 → `name` (06). Never empty.
    pub title: String,
    /// The whole file, front-matter block included, exactly as on disk.
    pub raw: String,
    /// Byte offset in `raw` where the body begins — `0` when there is no
    /// front-matter block. Link positions are body-relative (06), so a
    /// file-absolute offset is `body_offset + byte_start`.
    pub body_offset: usize,
    pub front_matter: FrontMatter,
    /// Every wiki-link in document order. Always empty for a leaf kind.
    pub links: Vec<WikiLink>,
}

impl RawDoc {
    /// The body — the file with its front-matter block removed.
    pub fn body(&self) -> &str {
        &self.raw[self.body_offset..]
    }
}

/// The exhaustive typed front-matter of 02. Anything else in the block is an
/// unknown key: preserved on write, invisible here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontMatter {
    #[serde(default)]
    pub title: Option<String>,
    /// `None` for anything that is not a Ticket — matching the nullable column
    /// in the index (06).
    #[serde(default)]
    pub done: Option<bool>,
    #[serde(default)]
    pub done_date: Option<String>,
    /// The one Epic this Ticket belongs to; membership points upward (02).
    #[serde(default)]
    pub epic: Option<String>,
    /// Present exactly for imported Tickets.
    #[serde(default)]
    pub source: Option<SourceRef>,
}

/// Where an imported Ticket came from (02's "source reference").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    /// The source kind, which is also its id in v1 (`"github"` — 09).
    pub kind: String,
    /// Source-side coordinates: the GitHub issue number as a string.
    pub key: String,
    pub url: String,
}

/// One `[[target]]` occurrence.
///
/// The alias form `[[target|display]]` is tolerated and extracts the same
/// `target`; the display half is never stored (06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The raw target text: a bare name (`T-42`) or a path form (`docs/api`).
    /// Resolution is the index's job, not the vault's.
    pub target: String,
    /// Occurrence order in the document, from 0.
    pub ord: u32,
    /// Body-relative byte range of the whole `[[…]]` span — the editor's
    /// decoration anchor (06 → 10).
    pub byte_start: usize,
    pub byte_end: usize,
}

/// A single front-matter field's fate in a patch: three states, because
/// "leave it alone" and "remove it" are different acts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    /// Not part of this act — the key keeps whatever it had.
    #[default]
    Leave,
    Set(T),
    /// Remove the key entirely. Absence is how reeve says "not set" (a
    /// `done: false` line would be state the board must then interpret).
    Clear,
}

/// The `done` stamp: the flag and its date move together or not at all (02),
/// so the type refuses to let them drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// ISO 8601 date, supplied by the caller — the vault never reads a clock.
    pub date: String,
}

/// A structural act on the front-matter block: `mark_done`, `reopen_ticket`,
/// `assign_epic`, and import's initial write are all shapes of this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatterPatch {
    pub title: Patch<String>,
    /// [`Patch::Set`] writes `done: true` **and** `doneDate`; [`Patch::Clear`]
    /// removes both.
    pub done: Patch<Done>,
    pub epic: Patch<String>,
    pub source: Patch<SourceRef>,
}

impl FrontMatterPatch {
    /// `mark_done` (03-api): the flag plus its date.
    pub fn mark_done(date: impl Into<String>) -> Self {
        Self {
            done: Patch::Set(Done { date: date.into() }),
            ..Self::default()
        }
    }

    /// `reopen_ticket`: Done is the only stored board state, so clearing it is
    /// the whole act.
    pub fn reopen() -> Self {
        Self {
            done: Patch::Clear,
            ..Self::default()
        }
    }

    /// `assign_epic(ticketId, epicId?)` — `None` clears the membership.
    pub fn assign_epic(epic: Option<String>) -> Self {
        Self {
            epic: match epic {
                Some(id) => Patch::Set(id),
                None => Patch::Clear,
            },
            ..Self::default()
        }
    }
}

/// The new content of a marker-delimited region, markers included.
///
/// Both markers must be whole lines in the target file (09). `content` travels
/// verbatim: escaping content that would look like a marker is the *renderer's*
/// job (09's escape rule), and the vault refuses content that skipped it rather
/// than writing a region it could never read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionContent {
    pub begin_marker: String,
    pub end_marker: String,
    pub content: String,
}

impl RegionContent {
    pub fn new(
        begin_marker: impl Into<String>,
        end_marker: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            begin_marker: begin_marker.into(),
            end_marker: end_marker.into(),
            content: content.into(),
        }
    }
}

/// What can go wrong in the vault, classified so callers can decide.
///
/// [`VaultError::RegionMalformed`] is the one 09 leans on by name: a broken
/// region is skipped with a warning in the refresh result, never guessed at.
/// The mapping onto `ApiError`'s `fs/*` codes belongs to the services that
/// surface these across the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// No such file. `path` is the repo-relative form the caller passed.
    NotFound { path: String },
    /// The extension is not one of the three Node kinds (02).
    NotANode { path: String },
    /// The path is absolute, or climbs out of the repository with `..`. The
    /// vault writes inside one Project or nowhere.
    PathOutsideProject { path: String },
    /// The front-matter block is not a YAML mapping.
    FrontMatterMalformed { path: String, message: String },
    /// Zero markers, a begin without an end, or an end before its begin (09).
    RegionMalformed { path: String },
    /// The new region content contains a line byte-equal to a marker, which
    /// would truncate the region on the next read (09's escape rule, unapplied).
    RegionContentContainsMarker { path: String },
    /// Anything the filesystem refused.
    Io { path: String, message: String },
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "{path}: no such file in this project"),
            Self::NotANode { path } => write!(
                f,
                "{path}: not a Node — only Markdown, Excalidraw and HTML files join the graph"
            ),
            Self::PathOutsideProject { path } => {
                write!(f, "{path}: path must be relative to the project root")
            }
            Self::FrontMatterMalformed { path, message } => {
                write!(f, "{path}: front-matter is not valid YAML: {message}")
            }
            Self::RegionMalformed { path } => write!(
                f,
                "{path}: the imported region's markers are missing or out of order — repair the file to refresh it"
            ),
            Self::RegionContentContainsMarker { path } => write!(
                f,
                "{path}: refusing to write region content that contains a marker line"
            ),
            Self::Io { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for VaultError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_classify_the_three_node_kinds() {
        assert_eq!(NodeKind::from_path("docs/api.md"), Some(NodeKind::Markdown));
        assert_eq!(
            NodeKind::from_path("sketch.excalidraw"),
            Some(NodeKind::Excalidraw)
        );
        assert_eq!(NodeKind::from_path("report.html"), Some(NodeKind::Html));
    }

    /// Windows filesystems are case-insensitive; an uppercase extension is the
    /// same Node kind, not a non-Node (06's portability law).
    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(NodeKind::from_path("README.MD"), Some(NodeKind::Markdown));
    }

    /// Anything else is opened by the system application, so it must not
    /// classify as a Node — including a dotted directory above a bare filename.
    #[test]
    fn non_node_files_are_not_classified() {
        assert_eq!(NodeKind::from_path("logo.png"), None);
        assert_eq!(NodeKind::from_path("Makefile"), None);
        assert_eq!(NodeKind::from_path("a.b/README"), None);
    }

    /// The name is the identity wiki-links store, so its derivation is exact:
    /// the basename, one extension off, dotfiles kept whole.
    #[test]
    fn names_are_basenames_without_their_extension() {
        assert_eq!(node_name(".reeve/tickets/T-42.md"), "T-42");
        assert_eq!(node_name("docs/design/06-lld-graph.md"), "06-lld-graph");
        assert_eq!(node_name("notes/my.notes.md"), "my.notes");
        assert_eq!(node_name(".gitignore"), ".gitignore");
    }

    /// The constructors are the API's structural acts spelled out; each must
    /// touch exactly its own field and leave the rest alone.
    #[test]
    fn patch_constructors_touch_one_field_each() {
        let done = FrontMatterPatch::mark_done("2026-07-29");
        assert_eq!(
            done.done,
            Patch::Set(Done {
                date: "2026-07-29".into()
            })
        );
        assert_eq!(done.epic, Patch::Leave);
        assert_eq!(FrontMatterPatch::reopen().done, Patch::Clear);
        assert_eq!(FrontMatterPatch::assign_epic(None).epic, Patch::Clear);
        assert_eq!(
            FrontMatterPatch::assign_epic(Some("E-7".into())).epic,
            Patch::Set("E-7".into())
        );
    }
}
