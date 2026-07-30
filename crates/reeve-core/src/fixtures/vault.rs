//! The in-memory [`Vault`]: a hermetic double for service tests.
//!
//! **It does not parse Markdown, and it must not.** Parsing is `reeve-infra`'s
//! job — the parser is a ring-3 dependency, and a second implementation here
//! would be a second answer to "what is a link?". So the fixture is a *store
//! and a recorder*: tests seed the [`RawDoc`] values a service should see, and
//! then assert which acts the service performed.
//!
//! The split that follows from that:
//!
//! - `read_node` serves what was seeded.
//! - `write_doc` replaces a document's raw content; the parsed fields it was
//!   seeded with stay as they were.
//! - `patch_front_matter` really applies the patch to the typed front-matter —
//!   that is data manipulation, not parsing — but does not re-serialize `raw`.
//! - `rewrite_region` is recorded only: the marker semantics are 09's, and
//!   `reeve-infra`'s own tests are where they are proved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::seams::vault::{
    node_name, Done, FrontMatter, FrontMatterPatch, NodeKind, Patch, RawDoc, RegionContent, Vault,
    VaultError, WikiLink,
};

/// One call the fixture saw, in order — the assertion surface for service tests
/// ("did `mark_done` patch the right file with the right patch?").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultCall {
    ReadNode {
        path: String,
    },
    WriteDoc {
        path: String,
        content: String,
    },
    PatchFrontMatter {
        path: String,
        patch: FrontMatterPatch,
    },
    RewriteRegion {
        path: String,
        region: RegionContent,
    },
    Scaffold {
        project: PathBuf,
    },
}

#[derive(Default)]
struct State {
    docs: BTreeMap<String, RawDoc>,
    calls: Vec<VaultCall>,
    scaffolded: Vec<PathBuf>,
}

/// A vault that lives entirely in memory. `project` is recorded but never
/// interpreted: a fixture serves one Project's worth of documents.
#[derive(Default)]
pub struct InMemoryVault {
    state: Mutex<State>,
}

impl InMemoryVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a document, builder style.
    pub fn with_doc(self, doc: RawDoc) -> Self {
        self.insert(doc);
        self
    }

    /// Seed a Markdown Node from its path, title and link targets — enough for
    /// most service tests, and coherent: the generated body really does contain
    /// the links at the byte ranges reported.
    pub fn with_markdown(self, path: &str, title: &str, link_targets: &[&str]) -> Self {
        self.with_doc(markdown_doc(path, title, link_targets))
    }

    pub fn insert(&self, doc: RawDoc) {
        self.lock().docs.insert(doc.path.clone(), doc);
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<VaultCall> {
        self.lock().calls.clone()
    }

    /// The current raw content of a document, `None` when it does not exist.
    pub fn content(&self, path: &str) -> Option<String> {
        self.lock().docs.get(path).map(|doc| doc.raw.clone())
    }

    pub fn front_matter(&self, path: &str) -> Option<FrontMatter> {
        self.lock()
            .docs
            .get(path)
            .map(|doc| doc.front_matter.clone())
    }

    pub fn is_scaffolded(&self, project: &Path) -> bool {
        self.lock().scaffolded.iter().any(|seen| seen == project)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// A [`RawDoc`] as the real vault would have produced it for a body that is
/// nothing but its links, one per line.
pub fn markdown_doc(path: &str, title: &str, link_targets: &[&str]) -> RawDoc {
    let mut body = String::new();
    let mut links = Vec::new();
    for (ord, target) in link_targets.iter().enumerate() {
        let byte_start = body.len();
        body.push_str(&format!("[[{target}]]\n"));
        links.push(WikiLink {
            target: (*target).to_string(),
            ord: ord as u32,
            byte_start,
            byte_end: body.len() - 1,
        });
    }
    let front_matter = FrontMatter {
        title: Some(title.to_string()),
        ..FrontMatter::default()
    };
    let raw = format!("---\ntitle: {title}\n---\n{body}");
    RawDoc {
        body_offset: raw.len() - body.len(),
        path: path.to_string(),
        name: node_name(path),
        kind: NodeKind::from_path(path).unwrap_or(NodeKind::Markdown),
        title: title.to_string(),
        raw,
        front_matter,
        links,
    }
}

impl Vault for InMemoryVault {
    fn read_node(&self, _project: &Path, path: &str) -> Result<RawDoc, VaultError> {
        let mut state = self.lock();
        state.calls.push(VaultCall::ReadNode {
            path: path.to_string(),
        });
        state
            .docs
            .get(path)
            .cloned()
            .ok_or_else(|| VaultError::NotFound {
                path: path.to_string(),
            })
    }

    fn write_doc(&self, _project: &Path, path: &str, content: &str) -> Result<(), VaultError> {
        let mut state = self.lock();
        state.calls.push(VaultCall::WriteDoc {
            path: path.to_string(),
            content: content.to_string(),
        });
        match state.docs.get_mut(path) {
            Some(doc) => doc.raw = content.to_string(),
            None => {
                let name = node_name(path);
                let doc = RawDoc {
                    path: path.to_string(),
                    title: name.clone(),
                    name,
                    kind: NodeKind::from_path(path).ok_or_else(|| VaultError::NotANode {
                        path: path.to_string(),
                    })?,
                    raw: content.to_string(),
                    body_offset: 0,
                    front_matter: FrontMatter::default(),
                    links: Vec::new(),
                };
                state.docs.insert(doc.path.clone(), doc);
            }
        }
        Ok(())
    }

    fn patch_front_matter(
        &self,
        _project: &Path,
        path: &str,
        patch: FrontMatterPatch,
    ) -> Result<(), VaultError> {
        let mut state = self.lock();
        state.calls.push(VaultCall::PatchFrontMatter {
            path: path.to_string(),
            patch: patch.clone(),
        });
        let doc = state
            .docs
            .get_mut(path)
            .ok_or_else(|| VaultError::NotFound {
                path: path.to_string(),
            })?;
        apply(&mut doc.front_matter, patch);
        Ok(())
    }

    fn rewrite_region(
        &self,
        _project: &Path,
        path: &str,
        region: RegionContent,
    ) -> Result<(), VaultError> {
        let mut state = self.lock();
        state.calls.push(VaultCall::RewriteRegion {
            path: path.to_string(),
            region,
        });
        if !state.docs.contains_key(path) {
            return Err(VaultError::NotFound {
                path: path.to_string(),
            });
        }
        Ok(())
    }

    fn scaffold(&self, project: &Path) -> Result<(), VaultError> {
        let mut state = self.lock();
        state.calls.push(VaultCall::Scaffold {
            project: project.to_path_buf(),
        });
        state.scaffolded.push(project.to_path_buf());
        Ok(())
    }
}

/// The typed half of a patch, applied to the typed front-matter.
fn apply(front_matter: &mut FrontMatter, patch: FrontMatterPatch) {
    match patch.title {
        Patch::Leave => {}
        Patch::Set(title) => front_matter.title = Some(title),
        Patch::Clear => front_matter.title = None,
    }
    match patch.epic {
        Patch::Leave => {}
        Patch::Set(epic) => front_matter.epic = Some(epic),
        Patch::Clear => front_matter.epic = None,
    }
    match patch.done {
        Patch::Leave => {}
        Patch::Set(Done { date }) => {
            front_matter.done = Some(true);
            front_matter.done_date = Some(date);
        }
        Patch::Clear => {
            front_matter.done = None;
            front_matter.done_date = None;
        }
    }
    match patch.source {
        Patch::Leave => {}
        Patch::Set(source) => front_matter.source = Some(source),
        Patch::Clear => front_matter.source = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> PathBuf {
        PathBuf::from("/projects/reeve")
    }

    #[test]
    fn seeded_documents_are_served_and_missing_ones_are_not_found() {
        let vault = InMemoryVault::new().with_markdown("T-1.md", "Ship it", &["T-2", "docs/api"]);

        let doc = vault.read_node(&project(), "T-1.md").expect("seeded");
        assert_eq!(doc.title, "Ship it");
        assert_eq!(doc.name, "T-1");
        let targets: Vec<&str> = doc.links.iter().map(|link| link.target.as_str()).collect();
        assert_eq!(targets, ["T-2", "docs/api"]);

        assert_eq!(
            vault.read_node(&project(), "gone.md"),
            Err(VaultError::NotFound {
                path: "gone.md".into()
            })
        );
    }

    /// The seeded body and the reported ranges agree, so a test that slices the
    /// body by a link's range gets the link back.
    #[test]
    fn seeded_link_ranges_address_the_seeded_body() {
        let vault = InMemoryVault::new().with_markdown("T-1.md", "Ship it", &["T-2"]);
        let doc = vault.read_node(&project(), "T-1.md").expect("seeded");
        let link = &doc.links[0];

        assert_eq!(&doc.body()[link.byte_start..link.byte_end], "[[T-2]]");
    }

    #[test]
    fn every_act_is_recorded_in_order() {
        let vault = InMemoryVault::new().with_markdown("T-1.md", "Ship it", &[]);
        let patch = FrontMatterPatch::mark_done("2026-07-29");

        vault.scaffold(&project()).expect("scaffolded");
        vault
            .write_doc(&project(), "notes/T-1.md", "note\n")
            .expect("written");
        vault
            .patch_front_matter(&project(), "T-1.md", patch.clone())
            .expect("patched");

        assert_eq!(
            vault.calls(),
            vec![
                VaultCall::Scaffold { project: project() },
                VaultCall::WriteDoc {
                    path: "notes/T-1.md".into(),
                    content: "note\n".into()
                },
                VaultCall::PatchFrontMatter {
                    path: "T-1.md".into(),
                    patch
                },
            ]
        );
        assert!(vault.is_scaffolded(&project()));
    }

    /// A service test asserts against state as well as calls, so the typed patch
    /// really lands.
    #[test]
    fn patching_updates_the_typed_front_matter() {
        let vault = InMemoryVault::new().with_markdown("T-1.md", "Ship it", &[]);

        vault
            .patch_front_matter(
                &project(),
                "T-1.md",
                FrontMatterPatch::mark_done("2026-07-29"),
            )
            .expect("patched");
        let front_matter = vault.front_matter("T-1.md").expect("seeded");
        assert_eq!(front_matter.done, Some(true));
        assert_eq!(front_matter.done_date.as_deref(), Some("2026-07-29"));

        vault
            .patch_front_matter(&project(), "T-1.md", FrontMatterPatch::reopen())
            .expect("reopened");
        let front_matter = vault.front_matter("T-1.md").expect("seeded");
        assert_eq!(front_matter.done, None);
        assert_eq!(front_matter.done_date, None);
    }

    #[test]
    fn writing_an_unseeded_document_creates_it() {
        let vault = InMemoryVault::new();

        vault
            .write_doc(&project(), "notes/T-1.md", "note\n")
            .expect("written");
        assert_eq!(vault.content("notes/T-1.md").as_deref(), Some("note\n"));

        vault
            .write_doc(&project(), "notes/T-1.md", "revised\n")
            .expect("rewritten");
        assert_eq!(vault.content("notes/T-1.md").as_deref(), Some("revised\n"));
    }
}
