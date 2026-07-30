//! Markdown I/O: front-matter, wiki-links, materialized regions, `.reeve/` scaffold.
//!
//! Implemented against 06-lld-graph.
//!
//! This module is the **only** code in reeve that parses Markdown. It parses
//! with `pulldown-cmark` and its native `ENABLE_WIKILINKS` extension, so link
//! extraction rides the real parser: a `[[target]]` inside a code fence or
//! inline code is not a link, which no regex gets right.
//!
//! Two rules run through everything below:
//!
//! - **Reading is lenient, writing is strict.** The whole repository is the
//!   Graph (02), so the vault reads front-matter written by Jekyll, Obsidian or
//!   nobody in particular: a field of the wrong type is simply not that field.
//!   [`FsVault::patch_front_matter`], which *rewrites* the block, refuses to
//!   touch YAML it cannot parse.
//! - **Every write is atomic** — temp sibling then rename — so no reader and no
//!   watcher ever sees half a file. The temp name carries a non-Node extension
//!   on purpose: the index's filter drops it without a parse.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use reeve_core::seams::vault::{
    Done, FrontMatter, FrontMatterPatch, NodeKind, Patch, RawDoc, RegionContent, SourceRef, Vault,
    VaultError, WikiLink,
};
use serde_yaml_ng::{Mapping, Value};

/// The filesystem `Vault`. Stateless: `project` arrives with every call, so one
/// instance serves every Project (03-api's stateless operations).
#[derive(Debug, Clone, Copy, Default)]
pub struct FsVault;

impl FsVault {
    pub fn new() -> Self {
        Self
    }
}

/// The committed `.reeve/` layout of 02, plus the id counters 09 added.
const SCAFFOLD_DIRS: [&str; 3] = [".reeve/tickets", ".reeve/epics", ".reeve/notes"];

/// Git cannot commit an empty directory, and the layout is committed — so the
/// three content folders are pinned until their first real file lands.
const GITKEEP: &str = ".gitkeep";

/// The default Project configuration (02, amended by 06 and 09). Written only
/// when absent; the typed `ProjectConfig` and its read/write path belong to the
/// `projects` service, which owns `get_project_config`.
const DEFAULT_CONFIG: &str = r#"{
  "baseBranch": null,
  "verifyCommand": null,
  "defaultAgentProfile": null,
  "autoCommit": true,
  "contextTokenBudget": 32000,
  "sources": {}
}
"#;

/// Id counters are **truth**, not cache: never-reuse is not derivable from the
/// filesystem's present (09). Both start at zero, so the first ids are T-1/E-1.
const DEFAULT_COUNTERS: &str = r#"{
  "ticket": 0,
  "epic": 0
}
"#;

impl Vault for FsVault {
    fn read_node(&self, project: &Path, path: &str) -> Result<RawDoc, VaultError> {
        let rel = relative(path)?;
        let kind =
            NodeKind::from_path(&rel).ok_or_else(|| VaultError::NotANode { path: rel.clone() })?;
        let raw = read_to_string(&project.join(&rel), &rel)?;
        let name = node_name(&rel);

        // `.excalidraw` and `.html` are leaf Nodes: no parse, no edges, title is
        // the name (FR-2.5). Their content is still returned — the frontend's
        // embedded editor and sandboxed viewer need it.
        if kind != NodeKind::Markdown {
            return Ok(RawDoc {
                path: rel,
                title: name.clone(),
                name,
                kind,
                raw,
                body_offset: 0,
                front_matter: FrontMatter::default(),
                links: Vec::new(),
            });
        }

        let (block, body_offset) = split_front_matter(&raw);
        let front_matter = block.map(read_front_matter).unwrap_or_default();
        let parsed = parse_body(&raw[body_offset..]);
        let title = front_matter
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .or(parsed.h1.as_deref())
            .filter(|t| !t.is_empty())
            .unwrap_or(&name)
            .to_string();

        Ok(RawDoc {
            path: rel,
            name,
            kind,
            title,
            raw,
            body_offset,
            front_matter,
            links: parsed.links,
        })
    }

    fn write_doc(&self, project: &Path, path: &str, content: &str) -> Result<(), VaultError> {
        let rel = relative(path)?;
        write_atomic(&project.join(&rel), &rel, content)
    }

    fn patch_front_matter(
        &self,
        project: &Path,
        path: &str,
        patch: FrontMatterPatch,
    ) -> Result<(), VaultError> {
        let rel = relative(path)?;
        let full = project.join(&rel);
        let raw = read_to_string(&full, &rel)?;
        let (block, body_offset) = split_front_matter(&raw);

        let mut mapping = match block {
            Some(text) => parse_mapping(text).ok_or_else(|| VaultError::FrontMatterMalformed {
                path: rel.clone(),
                message: "the block is not a YAML mapping".into(),
            })?,
            None => Mapping::new(),
        };
        apply_patch(&mut mapping, patch);

        // The body is copied byte-for-byte: a structural act rewrites the block
        // and nothing else (06).
        let body = &raw[body_offset..];
        let updated = if mapping.is_empty() {
            body.to_string()
        } else {
            let yaml = serde_yaml_ng::to_string(&Value::Mapping(mapping)).map_err(|err| {
                VaultError::FrontMatterMalformed {
                    path: rel.clone(),
                    message: err.to_string(),
                }
            })?;
            format!("---\n{}---\n{body}", yaml.trim_start_matches("---\n"))
        };
        write_atomic(&full, &rel, &updated)
    }

    fn rewrite_region(
        &self,
        project: &Path,
        path: &str,
        region: RegionContent,
    ) -> Result<(), VaultError> {
        let rel = relative(path)?;
        let full = project.join(&rel);
        let raw = read_to_string(&full, &rel)?;

        // Belt and braces for 09's escape rule, which the region's *renderer*
        // applies: content holding a marker line would truncate the region on
        // the next read, so we refuse it rather than write a file we could never
        // read back.
        if content_lines(&region.content)
            .any(|line| line == region.begin_marker || line == region.end_marker)
        {
            return Err(VaultError::RegionContentContainsMarker { path: rel });
        }

        let lines = line_spans(&raw);
        let begin = lines
            .iter()
            .position(|line| line.text(&raw) == region.begin_marker);
        let end = lines
            .iter()
            .position(|line| line.text(&raw) == region.end_marker);
        // Zero markers, a begin without an end, or an end before its begin: all
        // one answer — reeve reports and does not guess (09). Repair is manual.
        let (begin, end) = match (begin, end) {
            (Some(begin), Some(end)) if begin < end => (begin, end),
            _ => return Err(VaultError::RegionMalformed { path: rel }),
        };

        let inner = match region.content.as_str() {
            "" => String::new(),
            text if text.ends_with('\n') => text.to_string(),
            text => format!("{text}\n"),
        };
        let updated = format!(
            "{}{inner}{}",
            &raw[..lines[begin].next],
            &raw[lines[end].start..]
        );
        write_atomic(&full, &rel, &updated)
    }

    fn scaffold(&self, project: &Path) -> Result<(), VaultError> {
        create_dir(&project.join(".reeve"), ".reeve")?;
        for dir in SCAFFOLD_DIRS {
            create_dir(&project.join(dir), dir)?;
            let keep = format!("{dir}/{GITKEEP}");
            create_if_absent(&project.join(&keep), &keep, "")?;
        }
        create_if_absent(
            &project.join(".reeve/config.json"),
            ".reeve/config.json",
            DEFAULT_CONFIG,
        )?;
        create_if_absent(
            &project.join(".reeve/counters.json"),
            ".reeve/counters.json",
            DEFAULT_COUNTERS,
        )
    }
}

// --- paths ---------------------------------------------------------------

/// Normalize a caller's path to the repo-relative, `/`-separated form the index
/// keys on — and refuse anything that could leave the Project.
///
/// Backslashes are accepted and folded: this is an interop seam and Windows
/// callers exist, but only one form is ever stored or returned.
fn relative(path: &str) -> Result<String, VaultError> {
    let outside = || VaultError::PathOutsideProject {
        path: path.to_string(),
    };
    let unified = path.trim().replace('\\', "/");
    if unified.is_empty() || unified.starts_with('/') {
        return Err(outside());
    }
    // A Windows drive prefix (`C:/…`) is an absolute path in disguise.
    if unified.as_bytes().get(1) == Some(&b':') {
        return Err(outside());
    }
    let mut parts = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(outside()),
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return Err(outside());
    }
    Ok(parts.join("/"))
}

/// The Node's name: basename without extension (02 — for Tickets and Epics this
/// is the id).
fn node_name(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    match base.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => base.to_string(),
    }
}

// --- front-matter --------------------------------------------------------

/// Split the leading `---\n … \n---` block off byte-wise, before any Markdown
/// parsing (06). Returns the block's text and the byte offset where the body
/// begins; an unterminated block is no block at all, so the whole file is body.
fn split_front_matter(raw: &str) -> (Option<&str>, usize) {
    // A BOM before the opening fence is common on Windows and must not hide it.
    let start = if raw.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let (first, mut cursor) = line_at(raw, start);
    if first != "---" {
        return (None, 0);
    }
    let block_start = cursor;
    while cursor < raw.len() {
        let (line, next) = line_at(raw, cursor);
        if line == "---" {
            return (Some(&raw[block_start..cursor]), next);
        }
        cursor = next;
    }
    (None, 0)
}

/// The lenient read path: every typed field of 02, best-effort. A value of the
/// wrong type is not an error — it is simply not that field.
fn read_front_matter(block: &str) -> FrontMatter {
    let Some(mapping) = parse_mapping(block) else {
        return FrontMatter::default();
    };
    let string = |key: &str| match mapping.get(Value::from(key)) {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    };
    FrontMatter {
        title: string("title"),
        done: match mapping.get(Value::from("done")) {
            Some(Value::Bool(done)) => Some(*done),
            _ => None,
        },
        done_date: string("doneDate"),
        epic: string("epic"),
        source: mapping
            .get(Value::from("source"))
            .cloned()
            .and_then(|value| serde_yaml_ng::from_value::<SourceRef>(value).ok()),
    }
}

/// Parse the block into its raw mapping — the shape the write path patches, so
/// that unknown keys survive (06). `None` when it is not a mapping at all.
fn parse_mapping(block: &str) -> Option<Mapping> {
    match serde_yaml_ng::from_str::<Value>(block) {
        // An empty block is an empty mapping, not a failure.
        Ok(Value::Null) => Some(Mapping::new()),
        Ok(Value::Mapping(mapping)) => Some(mapping),
        _ => None,
    }
}

fn apply_patch(mapping: &mut Mapping, patch: FrontMatterPatch) {
    apply_string(mapping, "title", patch.title);
    apply_string(mapping, "epic", patch.epic);
    match patch.done {
        Patch::Leave => {}
        // The flag and its date travel together, both ways.
        Patch::Set(Done { date }) => {
            mapping.insert(Value::from("done"), Value::from(true));
            mapping.insert(Value::from("doneDate"), Value::from(date));
        }
        Patch::Clear => {
            mapping.remove(Value::from("done"));
            mapping.remove(Value::from("doneDate"));
        }
    }
    match patch.source {
        Patch::Leave => {}
        Patch::Set(source) => {
            if let Ok(value) = serde_yaml_ng::to_value(&source) {
                mapping.insert(Value::from("source"), value);
            }
        }
        Patch::Clear => {
            mapping.remove(Value::from("source"));
        }
    }
}

fn apply_string(mapping: &mut Mapping, key: &str, patch: Patch<String>) {
    match patch {
        Patch::Leave => {}
        Patch::Set(value) => {
            mapping.insert(Value::from(key), Value::from(value));
        }
        Patch::Clear => {
            mapping.remove(Value::from(key));
        }
    }
}

// --- Markdown ------------------------------------------------------------

/// Everything one parse of the body yields (06: "One parse produces everything
/// the index stores").
struct ParsedBody {
    /// The first H1's text, the title's second fallback.
    h1: Option<String>,
    links: Vec<WikiLink>,
}

fn parse_body(body: &str) -> ParsedBody {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_WIKILINKS);

    let mut h1 = None;
    let mut in_h1 = false;
    let mut heading = String::new();
    let mut links: Vec<WikiLink> = Vec::new();

    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) if h1.is_none() => {
                in_h1 = true;
                heading.clear();
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) if in_h1 => {
                in_h1 = false;
                h1 = Some(heading.trim().to_string());
            }
            // A wiki-link's *event* is the parser's judgement: inside a fence or
            // inline code, this never fires.
            Event::Start(Tag::Link {
                link_type: LinkType::WikiLink { .. },
                dest_url,
                ..
            }) => links.push(WikiLink {
                target: dest_url.trim().to_string(),
                ord: links.len() as u32,
                byte_start: range.start,
                byte_end: range.end,
            }),
            Event::Text(text) | Event::Code(text) if in_h1 => heading.push_str(&text),
            _ => {}
        }
    }

    ParsedBody { h1, links }
}

// --- lines ---------------------------------------------------------------

/// One line's byte spans. `start..end` excludes the line ending; `next` is where
/// the following line begins.
struct LineSpan {
    start: usize,
    end: usize,
    next: usize,
}

impl LineSpan {
    fn text<'a>(&self, raw: &'a str) -> &'a str {
        &raw[self.start..self.end]
    }
}

fn line_spans(raw: &str) -> Vec<LineSpan> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < raw.len() {
        let (text, next) = line_at(raw, cursor);
        spans.push(LineSpan {
            start: cursor,
            end: cursor + text.len(),
            next,
        });
        cursor = next;
    }
    spans
}

/// The line starting at `from`, without its line ending, and where the next one
/// begins. `\r\n` is folded so a CRLF checkout compares like an LF one.
fn line_at(text: &str, from: usize) -> (&str, usize) {
    match text[from..].find('\n') {
        Some(offset) => {
            let end = from + offset;
            (text[from..end].trim_end_matches('\r'), end + 1)
        }
        None => (text[from..].trim_end_matches('\r'), text.len()),
    }
}

fn content_lines(content: &str) -> impl Iterator<Item = &str> {
    content.split('\n').map(|line| line.trim_end_matches('\r'))
}

// --- filesystem ----------------------------------------------------------

fn read_to_string(full: &Path, rel: &str) -> Result<String, VaultError> {
    fs::read_to_string(full).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => VaultError::NotFound {
            path: rel.to_string(),
        },
        _ => io_error(rel, &err),
    })
}

/// Write through a temp sibling and rename: a reader either sees the old file or
/// the new one. `rename` replaces the destination on both Tier 1 platforms.
fn write_atomic(full: &Path, rel: &str, content: &str) -> Result<(), VaultError> {
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error(rel, &err))?;
    }
    let temp = temp_sibling(full);
    fs::write(&temp, content).map_err(|err| io_error(rel, &err))?;
    fs::rename(&temp, full).map_err(|err| {
        let _ = fs::remove_file(&temp);
        io_error(rel, &err)
    })
}

/// `docs/api.md` → `docs/.api.md.reeve-tmp`. Same directory, so the rename stays
/// on one filesystem; a non-Node extension, so the watcher's filter drops it.
fn temp_sibling(full: &Path) -> PathBuf {
    let name = full
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    full.with_file_name(format!(".{name}.reeve-tmp"))
}

fn create_dir(full: &Path, rel: &str) -> Result<(), VaultError> {
    fs::create_dir_all(full).map_err(|err| io_error(rel, &err))
}

/// The scaffold's law: create what is missing, never overwrite what is there.
fn create_if_absent(full: &Path, rel: &str, content: &str) -> Result<(), VaultError> {
    if full.exists() {
        return Ok(());
    }
    write_atomic(full, rel, content)
}

fn io_error(rel: &str, err: &io::Error) -> VaultError {
    VaultError::Io {
        path: rel.to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const BEGIN: &str = "<!-- reeve:source:begin -->";
    const END: &str = "<!-- reeve:source:end -->";

    /// A Project on disk, because a vault tested without a filesystem proves
    /// nothing about the seam it implements.
    struct Project {
        dir: TempDir,
    }

    impl Project {
        fn new() -> Self {
            Self {
                dir: TempDir::new().expect("a temp dir"),
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn with(rel: &str, content: &str) -> Self {
            let project = Self::new();
            project.put(rel, content);
            project
        }

        fn put(&self, rel: &str, content: &str) {
            let full = self.path().join(rel);
            fs::create_dir_all(full.parent().expect("a parent")).expect("create parents");
            fs::write(full, content).expect("write fixture");
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.path().join(rel)).expect("read back")
        }
    }

    fn read(project: &Project, rel: &str) -> RawDoc {
        FsVault::new()
            .read_node(project.path(), rel)
            .expect("a readable node")
    }

    // --- extraction ------------------------------------------------------

    #[test]
    fn front_matter_fields_parse_into_their_typed_form() {
        let project = Project::with(
            ".reeve/tickets/T-42.md",
            "---\ntitle: Ship the vault\ndone: true\ndoneDate: 2026-07-29\nepic: E-7\n\
             source:\n  kind: github\n  key: \"123\"\n  url: https://example.test/123\n---\nbody\n",
        );
        let doc = read(&project, ".reeve/tickets/T-42.md");

        assert_eq!(doc.name, "T-42");
        assert_eq!(doc.kind, NodeKind::Markdown);
        assert_eq!(doc.front_matter.title.as_deref(), Some("Ship the vault"));
        assert_eq!(doc.front_matter.done, Some(true));
        assert_eq!(doc.front_matter.done_date.as_deref(), Some("2026-07-29"));
        assert_eq!(doc.front_matter.epic.as_deref(), Some("E-7"));
        assert_eq!(
            doc.front_matter.source,
            Some(SourceRef {
                kind: "github".into(),
                key: "123".into(),
                url: "https://example.test/123".into(),
            })
        );
        assert_eq!(doc.body(), "body\n");
    }

    /// The whole repository is the Graph, so the vault meets front-matter it did
    /// not write. A field of the wrong type is not that field — and nothing else
    /// in the document is lost over it.
    #[test]
    fn alien_front_matter_is_read_leniently() {
        let project = Project::with(
            "posts/hello.md",
            "---\ntitle: [not, a, string]\ndone: yes please\ntags:\n  - rust\n---\n# Hello\n",
        );
        let doc = read(&project, "posts/hello.md");

        assert_eq!(doc.front_matter, FrontMatter::default());
        assert_eq!(doc.title, "Hello", "the H1 still supplies the title");
    }

    #[test]
    fn title_falls_back_front_matter_then_h1_then_name() {
        let project = Project::new();
        project.put("a.md", "---\ntitle: From front-matter\n---\n# From H1\n");
        project.put("b.md", "# From H1\n\n# Ignored second H1\n");
        project.put("c.md", "no heading here\n");

        assert_eq!(read(&project, "a.md").title, "From front-matter");
        assert_eq!(read(&project, "b.md").title, "From H1");
        assert_eq!(read(&project, "c.md").title, "c");
    }

    #[test]
    fn wiki_links_carry_order_and_body_relative_positions() {
        let body = "See [[T-42]] and [[docs/api]].\n";
        let project = Project::with("note.md", &format!("---\ntitle: Note\n---\n{body}"));
        let doc = read(&project, "note.md");

        let targets: Vec<&str> = doc.links.iter().map(|link| link.target.as_str()).collect();
        assert_eq!(targets, ["T-42", "docs/api"]);
        assert_eq!(doc.links[0].ord, 0);
        assert_eq!(doc.links[1].ord, 1);

        // Positions are body-relative (06); the editor rebuilds file offsets
        // with `body_offset`.
        let first = &doc.links[0];
        assert_eq!(&doc.body()[first.byte_start..first.byte_end], "[[T-42]]");
        let absolute = doc.body_offset + first.byte_start;
        assert_eq!(&doc.raw[absolute..absolute + 8], "[[T-42]]");
    }

    /// `[[target|display]]` is the tolerated alias form: the target is extracted
    /// identically and the display half is never stored (06).
    #[test]
    fn alias_links_store_the_target_not_the_display_text() {
        let project = Project::with("note.md", "[[docs/api|the API surface]]\n");
        let doc = read(&project, "note.md");

        assert_eq!(doc.links.len(), 1);
        assert_eq!(doc.links[0].target, "docs/api");
    }

    /// The whole reason extraction rides a real Markdown parser instead of a
    /// regex: these three are not links.
    #[test]
    fn brackets_in_code_are_not_links() {
        let project = Project::with(
            "note.md",
            "Real: [[T-1]]\n\nInline `[[T-2]]` here.\n\n```\n[[T-3]]\n```\n\n    [[T-4]]\n",
        );
        let doc = read(&project, "note.md");

        let targets: Vec<&str> = doc.links.iter().map(|link| link.target.as_str()).collect();
        assert_eq!(targets, ["T-1"]);
    }

    #[test]
    fn leaf_kinds_are_indexed_without_parsing() {
        let project = Project::new();
        project.put("sketch.excalidraw", "{\"elements\": [\"[[T-1]]\"]}");
        project.put("report.html", "<h1>Report</h1><p>[[T-2]]</p>");

        let sketch = read(&project, "sketch.excalidraw");
        assert_eq!(sketch.kind, NodeKind::Excalidraw);
        assert_eq!(sketch.title, "sketch", "a leaf's title is its name");
        assert!(sketch.links.is_empty(), "leaves have no outgoing edges");
        assert_eq!(sketch.body_offset, 0);
        assert!(sketch.raw.contains("[[T-1]]"), "content is still returned");

        let report = read(&project, "report.html");
        assert_eq!(report.kind, NodeKind::Html);
        assert!(report.links.is_empty());
    }

    #[test]
    fn a_file_without_front_matter_is_all_body() {
        let project = Project::with("note.md", "# Title\n\n[[T-1]]\n");
        let doc = read(&project, "note.md");

        assert_eq!(doc.body_offset, 0);
        assert_eq!(doc.body(), doc.raw);
        assert_eq!(doc.links.len(), 1);
    }

    /// An opening fence with no closing one is not a front-matter block: the
    /// file keeps every byte as body rather than swallowing it into YAML.
    #[test]
    fn an_unterminated_block_is_not_front_matter() {
        let project = Project::with("note.md", "---\ntitle: dangling\n\n# Heading\n");
        let doc = read(&project, "note.md");

        assert_eq!(doc.body_offset, 0);
        assert_eq!(doc.front_matter, FrontMatter::default());
        assert_eq!(doc.title, "Heading");
    }

    /// Windows is Tier 1 (NFR-4): a CRLF checkout must parse identically.
    #[test]
    fn crlf_files_parse_like_lf_files() {
        let project = Project::with(
            "note.md",
            "---\r\ntitle: Windows\r\n---\r\n# Ignored\r\n\r\n[[T-9]]\r\n",
        );
        let doc = read(&project, "note.md");

        assert_eq!(doc.title, "Windows");
        assert_eq!(doc.links.len(), 1);
        assert_eq!(doc.links[0].target, "T-9");
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_front_matter() {
        let project = Project::with("note.md", "\u{feff}---\ntitle: With BOM\n---\nbody\n");
        assert_eq!(read(&project, "note.md").title, "With BOM");
    }

    #[test]
    fn reading_a_non_node_extension_is_refused() {
        let project = Project::with("logo.png", "not markdown");
        assert_eq!(
            FsVault::new().read_node(project.path(), "logo.png"),
            Err(VaultError::NotANode {
                path: "logo.png".into()
            })
        );
    }

    #[test]
    fn reading_a_missing_file_is_not_found() {
        let project = Project::new();
        assert_eq!(
            FsVault::new().read_node(project.path(), "docs/gone.md"),
            Err(VaultError::NotFound {
                path: "docs/gone.md".into()
            })
        );
    }

    // --- paths -----------------------------------------------------------

    /// The vault writes inside one Project or nowhere. Every one of these is a
    /// path that could leave it.
    #[test]
    fn paths_that_escape_the_project_are_refused() {
        let project = Project::new();
        let vault = FsVault::new();
        for path in [
            "../outside.md",
            "docs/../../outside.md",
            "/etc/passwd",
            "C:/Windows/win.ini",
            "",
            "  ",
        ] {
            let result = vault.write_doc(project.path(), path, "x");
            assert_eq!(
                result,
                Err(VaultError::PathOutsideProject {
                    path: path.to_string()
                }),
                "{path} should be refused"
            );
        }
    }

    /// Windows callers exist; one stored form does not.
    #[test]
    fn backslash_paths_are_folded_to_the_stored_form() {
        let project = Project::with("docs\\api.md", "# API\n");
        assert_eq!(read(&project, "docs\\api.md").path, "docs/api.md");
        assert_eq!(read(&project, "./docs/api.md").path, "docs/api.md");
    }

    // --- write_doc -------------------------------------------------------

    #[test]
    fn write_doc_writes_verbatim_and_creates_parents() {
        let project = Project::new();
        let content = "---\ntitle: New\n---\n\nbody with trailing space \n";
        FsVault::new()
            .write_doc(project.path(), "docs/deep/new.md", content)
            .expect("written");

        assert_eq!(project.read("docs/deep/new.md"), content);
    }

    /// The atomic write must leave no residue — a stray temp file would show up
    /// in the user's `git status`.
    #[test]
    fn write_doc_leaves_no_temp_file_behind() {
        let project = Project::new();
        FsVault::new()
            .write_doc(project.path(), "note.md", "hello\n")
            .expect("written");

        let names: Vec<String> = fs::read_dir(project.path())
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
            .collect();
        assert_eq!(names, ["note.md"]);
    }

    // --- patch_front_matter ----------------------------------------------

    #[test]
    fn marking_done_writes_the_flag_and_its_date() {
        let project = Project::with(".reeve/tickets/T-1.md", "---\ntitle: Ship it\n---\nbody\n");
        FsVault::new()
            .patch_front_matter(
                project.path(),
                ".reeve/tickets/T-1.md",
                FrontMatterPatch::mark_done("2026-07-29"),
            )
            .expect("patched");

        let doc = read(&project, ".reeve/tickets/T-1.md");
        assert_eq!(doc.front_matter.done, Some(true));
        assert_eq!(doc.front_matter.done_date.as_deref(), Some("2026-07-29"));
        assert_eq!(doc.body(), "body\n", "the body is byte-identical");
    }

    /// Reopening clears both halves of the stamp: absence is how reeve says
    /// "not done", so no `done: false` line survives.
    #[test]
    fn reopening_removes_both_halves_of_the_done_stamp() {
        let project = Project::with(
            "T-1.md",
            "---\ntitle: Ship it\ndone: true\ndoneDate: 2026-07-29\n---\nbody\n",
        );
        FsVault::new()
            .patch_front_matter(project.path(), "T-1.md", FrontMatterPatch::reopen())
            .expect("patched");

        let raw = project.read("T-1.md");
        assert!(!raw.contains("done"), "{raw}");
        assert!(raw.contains("title: Ship it"));
    }

    #[test]
    fn assigning_and_clearing_an_epic_round_trips() {
        let project = Project::with("T-1.md", "---\ntitle: Ship it\n---\nbody\n");
        let vault = FsVault::new();

        vault
            .patch_front_matter(
                project.path(),
                "T-1.md",
                FrontMatterPatch::assign_epic(Some("E-7".into())),
            )
            .expect("assigned");
        assert_eq!(
            read(&project, "T-1.md").front_matter.epic.as_deref(),
            Some("E-7")
        );

        vault
            .patch_front_matter(
                project.path(),
                "T-1.md",
                FrontMatterPatch::assign_epic(None),
            )
            .expect("cleared");
        assert_eq!(read(&project, "T-1.md").front_matter.epic, None);
    }

    /// The write path re-serializes the whole block, so this is the law that
    /// keeps it safe for files reeve did not author (06).
    #[test]
    fn unknown_keys_survive_a_patch() {
        let project = Project::with(
            "note.md",
            "---\ntags:\n  - rust\nlayout: post\ntitle: Keep me\n---\n# Body\n\n[[T-1]]\n",
        );
        FsVault::new()
            .patch_front_matter(
                project.path(),
                "note.md",
                FrontMatterPatch::mark_done("2026-07-29"),
            )
            .expect("patched");

        let raw = project.read("note.md");
        assert!(raw.contains("layout: post"), "{raw}");
        assert!(raw.contains("- rust"), "{raw}");
        assert!(raw.ends_with("# Body\n\n[[T-1]]\n"), "{raw}");
    }

    #[test]
    fn patching_a_file_without_front_matter_adds_the_block() {
        let project = Project::with("note.md", "# Body\n");
        FsVault::new()
            .patch_front_matter(
                project.path(),
                "note.md",
                FrontMatterPatch::mark_done("2026-07-29"),
            )
            .expect("patched");

        assert_eq!(
            project.read("note.md"),
            "---\ndone: true\ndoneDate: 2026-07-29\n---\n# Body\n"
        );
    }

    /// Reading is lenient; rewriting is not. YAML the vault cannot parse is YAML
    /// it refuses to re-serialize, because that would silently drop content.
    #[test]
    fn patching_unparseable_front_matter_is_refused() {
        let project = Project::with("note.md", "---\n: : :\n---\nbody\n");
        let result = FsVault::new().patch_front_matter(
            project.path(),
            "note.md",
            FrontMatterPatch::mark_done("2026-07-29"),
        );

        assert!(
            matches!(result, Err(VaultError::FrontMatterMalformed { .. })),
            "{result:?}"
        );
        assert_eq!(project.read("note.md"), "---\n: : :\n---\nbody\n");
    }

    // --- rewrite_region --------------------------------------------------

    fn imported(region_body: &str) -> String {
        format!(
            "---\ntitle: Imported\n---\n{BEGIN}\n{region_body}{END}\n\nMy own notes with [[T-1]].\n"
        )
    }

    #[test]
    fn rewriting_a_region_replaces_only_what_lies_between_the_markers() {
        let project = Project::with("T-9.md", &imported("old snapshot\n"));
        FsVault::new()
            .rewrite_region(
                project.path(),
                "T-9.md",
                RegionContent::new(BEGIN, END, "new snapshot\nwith two lines"),
            )
            .expect("rewritten");

        assert_eq!(
            project.read("T-9.md"),
            imported("new snapshot\nwith two lines\n")
        );
    }

    #[test]
    fn an_empty_region_content_leaves_the_markers_adjacent() {
        let project = Project::with("T-9.md", &imported("old\n"));
        FsVault::new()
            .rewrite_region(project.path(), "T-9.md", RegionContent::new(BEGIN, END, ""))
            .expect("rewritten");

        assert_eq!(project.read("T-9.md"), imported(""));
    }

    /// Zero markers, a begin without an end, an end before its begin: one
    /// answer, and the file is left exactly as the user broke it (09).
    #[test]
    fn malformed_regions_are_reported_and_never_repaired() {
        let vault = FsVault::new();
        let cases = [
            ("no markers at all\n", "none"),
            (&format!("{BEGIN}\nbody without an end\n"), "begin only"),
            (&format!("{END}\nbody\n{BEGIN}\n"), "end before begin"),
        ];

        for (content, case) in cases {
            let project = Project::with("T-9.md", content);
            let result = vault.rewrite_region(
                project.path(),
                "T-9.md",
                RegionContent::new(BEGIN, END, "new"),
            );
            assert_eq!(
                result,
                Err(VaultError::RegionMalformed {
                    path: "T-9.md".into()
                }),
                "{case}"
            );
            assert_eq!(project.read("T-9.md"), content, "{case}");
        }
    }

    /// Content holding a marker line would truncate the region on the next read.
    /// 09's renderer escapes it; the vault refuses what skipped the escape.
    #[test]
    fn region_content_containing_a_marker_is_refused() {
        let project = Project::with("T-9.md", &imported("old\n"));
        let result = FsVault::new().rewrite_region(
            project.path(),
            "T-9.md",
            RegionContent::new(BEGIN, END, format!("sneaky\n{END}\nrest")),
        );

        assert_eq!(
            result,
            Err(VaultError::RegionContentContainsMarker {
                path: "T-9.md".into()
            })
        );
        assert_eq!(project.read("T-9.md"), imported("old\n"));
    }

    /// The primitive is generic (09): a future region owner brings its own
    /// markers and the vault does not care what they say.
    #[test]
    fn any_marker_pair_works() {
        let project = Project::with("note.md", "before\n<!-- a -->\nold\n<!-- b -->\nafter\n");
        FsVault::new()
            .rewrite_region(
                project.path(),
                "note.md",
                RegionContent::new("<!-- a -->", "<!-- b -->", "new"),
            )
            .expect("rewritten");

        assert_eq!(
            project.read("note.md"),
            "before\n<!-- a -->\nnew\n<!-- b -->\nafter\n"
        );
    }

    // --- scaffold --------------------------------------------------------

    #[test]
    fn scaffold_creates_the_committed_layout() {
        let project = Project::new();
        FsVault::new().scaffold(project.path()).expect("scaffolded");

        for dir in SCAFFOLD_DIRS {
            assert!(project.path().join(dir).is_dir(), "{dir}");
            assert!(
                project.path().join(dir).join(GITKEEP).is_file(),
                "{dir} needs a .gitkeep to be committable"
            );
        }
        let config: serde_json::Value =
            serde_json::from_str(&project.read(".reeve/config.json")).expect("valid JSON");
        assert_eq!(config["autoCommit"], serde_json::json!(true));
        assert_eq!(config["contextTokenBudget"], serde_json::json!(32000));

        let counters: serde_json::Value =
            serde_json::from_str(&project.read(".reeve/counters.json")).expect("valid JSON");
        assert_eq!(counters["ticket"], serde_json::json!(0));
        assert_eq!(counters["epic"], serde_json::json!(0));
    }

    /// `register_project` scaffolds "if absent" — on an already-configured
    /// repository that must be a no-op, not a reset.
    #[test]
    fn scaffold_never_overwrites_existing_files() {
        let project = Project::new();
        project.put(".reeve/config.json", "{\"autoCommit\": false}\n");
        project.put(".reeve/counters.json", "{\"ticket\": 12, \"epic\": 3}\n");

        let vault = FsVault::new();
        vault.scaffold(project.path()).expect("first");
        vault.scaffold(project.path()).expect("idempotent");

        assert_eq!(
            project.read(".reeve/config.json"),
            "{\"autoCommit\": false}\n"
        );
        assert_eq!(
            project.read(".reeve/counters.json"),
            "{\"ticket\": 12, \"epic\": 3}\n"
        );
    }
}
