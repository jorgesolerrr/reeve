//! **Ring-rule enforcement, layer 3** (05-lld-skeleton).
//!
//! The HLD's greppable rule, made executable: a plain `cargo test` on any
//! machine, not a CI-only shell script. Layer 1 (the crate graph) already makes
//! most violations fail to compile; this test catches the change that *creates*
//! the violation — a crate added to the wrong `Cargo.toml` — and says why, in a
//! PR, instead of leaving a reviewer to notice.
//!
//! It lives in `reeve-app` because layer 2 bans `std::fs` inside `reeve-core`:
//! the tool that enforces the rule must not break it.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// A name that may appear in exactly one place in the workspace.
struct Confinement {
    /// The token as it is written in Rust source (`tauri::`, `rusqlite`).
    rust_token: &'static str,
    /// The token as it is written in a `Cargo.toml` dependency key.
    manifest_key: &'static str,
    /// The only directory, relative to the workspace root, allowed to name it.
    allowed_in: &'static str,
    /// Why — printed on failure, because a rule without its reason gets deleted.
    reason: &'static str,
}

const CONFINEMENTS: &[Confinement] = &[
    Confinement {
        rust_token: "tauri::",
        manifest_key: "tauri",
        allowed_in: "src-tauri",
        reason: "ring 1 is the only ring that knows it is inside a Tauri app",
    },
    Confinement {
        rust_token: "rusqlite",
        manifest_key: "rusqlite",
        allowed_in: "crates/reeve-infra",
        reason: "the SQLite index is infrastructure; core sees the index seam",
    },
    Confinement {
        rust_token: "portable_pty",
        manifest_key: "portable-pty",
        allowed_in: "crates/reeve-infra",
        reason: "the PTY is infrastructure; core sees the pty seam",
    },
];

/// Member manifests, in ring order. The workspace root is deliberately absent:
/// `[workspace.dependencies]` centralizes versions without granting anyone use.
const MEMBER_MANIFESTS: &[&str] = &[
    "src-tauri/Cargo.toml",
    "crates/reeve-core/Cargo.toml",
    "crates/reeve-infra/Cargo.toml",
];

const SOURCE_ROOTS: &[&str] = &["src-tauri/src", "crates"];

#[test]
fn no_ring_names_a_crate_from_a_ring_below_it() {
    let root = workspace_root();
    let mut violations = Vec::new();

    for source in rust_sources(&root) {
        // This file names every banned token by definition; it is the enforcer.
        if source.ends_with("tests/ring_rule.rs") {
            continue;
        }
        let text = read(&root, &source);
        for line in strip_comments(&text, "//") {
            for rule in CONFINEMENTS {
                if line.contains(rule.rust_token) && !source.starts_with(rule.allowed_in) {
                    violations.push(format!(
                        "{source} names `{}` — allowed only under {}/ ({})",
                        rule.rust_token, rule.allowed_in, rule.reason
                    ));
                }
            }
        }
    }

    for manifest in MEMBER_MANIFESTS {
        let text = read(&root, manifest);
        for line in strip_comments(&text, "#") {
            for rule in CONFINEMENTS {
                if declares_dependency(&line, rule.manifest_key)
                    && !manifest.starts_with(rule.allowed_in)
                {
                    violations.push(format!(
                        "{manifest} depends on `{}` — allowed only in {}/Cargo.toml ({})",
                        rule.manifest_key, rule.allowed_in, rule.reason
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the ring rule is broken:\n  {}\n",
        violations.join("\n  ")
    );
}

/// A guard on the guard: if the walk stops finding sources — a moved folder, a
/// renamed crate — the test would pass by vacuum. Loudly, instead.
#[test]
fn the_walk_actually_reaches_every_crate() {
    let root = workspace_root();
    let sources = rust_sources(&root);
    for crate_dir in ["src-tauri/src", "crates/reeve-core", "crates/reeve-infra"] {
        assert!(
            sources.iter().any(|s| s.starts_with(crate_dir)),
            "the ring test found no Rust sources under {crate_dir}; \
             the layout moved and this test has stopped enforcing anything"
        );
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri always has a workspace root above it")
        .to_path_buf()
}

/// Every `.rs` file in the workspace, as forward-slashed paths relative to the
/// root — so the same string comparisons work on Windows and Linux.
fn rust_sources(root: &Path) -> Vec<String> {
    let mut sources = Vec::new();
    for source_root in SOURCE_ROOTS {
        for entry in WalkDir::new(root.join(source_root))
            .into_iter()
            .filter_entry(|e| e.file_name() != "target")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path
                    .strip_prefix(root)
                    .expect("walked from the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                sources.push(relative);
            }
        }
    }
    sources.sort();
    sources
}

fn read(root: &Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative))
        .unwrap_or_else(|err| panic!("cannot read {relative}: {err}"))
}

/// Prose is not a violation: the design documents these names, and so do the
/// doc comments that point at them. Only code counts.
///
/// The marker is per-language on purpose — stripping `#` from Rust would blind
/// the test to `#[tauri::command]`, which is exactly the kind of line it exists
/// to catch.
fn strip_comments(text: &str, marker: &str) -> Vec<String> {
    text.lines()
        .map(|line| match line.find(marker) {
            Some(at) => line[..at].to_string(),
            None => line.to_string(),
        })
        .collect()
}

/// `name = "1"` or `name = { ... }` at the head of a line — a dependency key,
/// not a mention inside some other value.
fn declares_dependency(line: &str, name: &str) -> bool {
    let trimmed = line.trim();
    trimmed
        .strip_prefix(name)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}
