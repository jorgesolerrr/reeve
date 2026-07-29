# 06 — LLD: Graph — vault, index, context_assembler

**Status:** Signed off (2026-07-27)
**Ticket:** [LLD: graph subsystem — vault, index, context_assembler](https://github.com/jorgesolerrr/reeve/issues/13)
**Grounded in:** [05-lld-skeleton.md](./05-lld-skeleton.md) · [04-hld.md](./04-hld.md) · [02-domain-model.md](./02-domain-model.md) · [03-api.md](./03-api.md) · [reference-implementations.md](../research/reference-implementations.md)
**Visual companion:** [lld-atlas.html](./lld-atlas.html) — View 3.

## Purpose & scope

This document fixes the internals of the graph pipeline: how Markdown becomes Nodes and edges (parser choice, extraction rules), how the derived index stores them (SQLite schema, lifecycle), how it stays fresh (watcher, debounce, reconciliation, incremental re-index), how links resolve, and how the Context Package is assembled and rendered. Materialized-Region *semantics* belong to 09 (sources); the Run tables sharing this SQLite file belong to 08 (runs); the editor/renderer consuming the resolution table belongs to 10 (frontend).

## Module map

Responsibilities split exactly along the HLD inventory:

| Module | Crate | Owns |
|---|---|---|
| `vault` | `reeve-infra` | Reading/writing repository files; **understanding Markdown**: front-matter parse/patch, title derivation, wiki-link extraction, region rewrite primitive. The only code that parses Markdown. |
| `index` | `reeve-infra` | The per-Project SQLite cache; the watcher; reconciliation and incremental re-index; the resolution and search queries. Calls `vault` to parse — never parses itself. |
| `services/graph` | `reeve-core` | The six `graph` operations (03-api), composing the two seams: `get_node` = vault read + index resolution table. |
| `domain/context_assembler` | `reeve-core` | The deterministic Context Package algorithm and the `AGENTS.md` renderer. Pure: reads only through the `GraphIndex` seam, writes nothing (the `runs` service writes the rendered file into the Workspace). |

## Format & crate decisions

| Decision | Choice | Why |
|---|---|---|
| Markdown parser | **`pulldown-cmark`** with `ENABLE_WIKILINKS` (+ heading events for the H1 title fallback) | Pull parser, fast, battle-tested (mdBook). Native `[[target]]` wikilink events mean link extraction rides the real parser — a `[[x]]` inside a code fence or inline code is **not** a link, which no regex approach gets right. We never render HTML in core (03-api: the core resolves, the frontend renders), so comrak's full-GFM AST buys nothing here. |
| Front-matter delimiting | `vault` splits the leading `---\n … \n---` block byte-wise before the Markdown parser runs | Trivial, exact, and keeps the body byte-offsets stable for link positions. |
| YAML | **`serde_yaml_ng`** (maintained continuation of the archived `serde_yaml`) | Parsed into a raw mapping first, then into the typed `FrontMatter`; unknown keys are preserved (see write path). Swappable inside `vault` if the ecosystem shifts. |
| Watcher | **`notify`** + **`notify-debouncer-full`** | Cross-platform (ReadDirectoryChangesW on Windows — Tier 1), and the full debouncer coalesces bursts and stitches atomic-save rename pairs (editors write temp-then-rename). |
| Ignore rules | **`ignore`** crate (the ripgrep walker) for both the reconciliation walk and per-event filtering | One implementation of `.gitignore` semantics for both paths; the Graph respects `.gitignore` by domain law (02). |
| SQLite | `rusqlite` (bundled), WAL, `synchronous=NORMAL`, `foreign_keys=ON` | It is a cache: throughput over durability; corruption is answered by rebuild, not repair. |

All five are `reeve-infra` dependencies only — the crate graph (skeleton, layer 1) already forbids them elsewhere.

## Extraction rules (vault)

One parse produces everything the index stores. For a Markdown Node:

- **Front-matter** → typed fields per the exhaustive 02 list: `title`, `done` (+ date), `epic`, source reference. Unknown keys are retained as an opaque mapping.
- **Title** → front-matter `title`, else the first H1 event, else the name. Computed at parse time, stored in the index.
- **Links** → every wikilink event: raw target text (`T-42`, `docs/api`), occurrence order, and byte range in the body (the editor's decoration anchors). An alias form (`[[target|display]]`) is tolerated; target extraction is identical, the alias is display-only and never stored in the index.
- `.excalidraw` and `.html` files are **leaf Nodes**: indexed with path/name/kind/title = name, no parsing, no outgoing edges (FR-2.5).

**Front-matter write path.** Structural acts (`mark_done`, `assign_epic`) go through `vault.patch_front_matter(path, patch)` *(amended by 09-lld-sources: region refresh performs no front-matter bookkeeping in v1 — it rewrites only the region; `title` is local-owned after import)*: parse the mapping, apply the patch, re-serialize the whole block, leave the body byte-identical. Unknown keys survive; YAML comments inside the front-matter block do not — documented trade-off, preferred over fragile line-surgery on YAML.

## SQLite schema

One file per Project (`~/.reeve/projects/<slug>/index.sqlite`). The graph owns two tables; 08 adds its Run-history tables to the same file.

```sql
PRAGMA user_version = 1;   -- schema version; mismatch ⇒ rebuild, never migrate

CREATE TABLE nodes (
  path        TEXT PRIMARY KEY,  -- repo-relative, '/' separators on every OS
  name        TEXT NOT NULL,     -- basename without extension; = id for T-/E-
  kind        TEXT NOT NULL CHECK (kind IN ('markdown','excalidraw','html')),
  title       TEXT NOT NULL,     -- front-matter title → first H1 → name
  mtime_ns    INTEGER NOT NULL,
  size        INTEGER NOT NULL,  -- also the token-estimate input (see assembler)
  done        INTEGER,           -- tickets: 0/1; NULL for non-tickets
  done_date   TEXT,
  epic        TEXT,              -- tickets: the owning E-<n>
  source_kind TEXT, source_key TEXT, source_url TEXT
);
CREATE INDEX nodes_name ON nodes (name COLLATE NOCASE);
CREATE INDEX nodes_epic ON nodes (epic) WHERE epic IS NOT NULL;

CREATE TABLE links (
  source_path TEXT NOT NULL REFERENCES nodes(path) ON DELETE CASCADE,
  ord         INTEGER NOT NULL,  -- occurrence order in the document
  target      TEXT NOT NULL,     -- raw stored target: bare name or path form
  byte_start  INTEGER NOT NULL,
  byte_end    INTEGER NOT NULL,
  PRIMARY KEY (source_path, ord)
);
CREATE INDEX links_target ON links (target COLLATE NOCASE);
```

Policies, all consequences of *cache, not truth*:

- **No migration framework.** `user_version` mismatch, corruption, or explicit recovery ⇒ delete the file and rebuild from the repository. vibe-kanban's 60-migration folder is the counterexample this buys out of; sortie's four-table discipline is the ceiling this stays under.
- **Front-matter fields are explicit columns**, not JSON — 02 fixed the list as exhaustive, so the schema is stable; board and epic queries (`get_board`, `list_epic_tickets`) become indexed scans.
- **Search** (`search_nodes`) is `LIKE` over `name`/`title` with `COLLATE NOCASE` — quick-open and autocomplete over ≤ 5,000 rows needs no FTS5, and the API's search is title/name only.
- **Concurrency:** one connection per Project owned by a dedicated blocking task; the index handle serializes writes through it. Queries at this scale are sub-millisecond; no pool.

## Watcher lifecycle

Per-Project, lazy (HLD): started by the first Project-scoped operation, lives until app close.

```
dormant ──first operation──▶ buffering ──walk done──▶ live
                    (watcher registered,     (drain buffered
                     reconciliation walk)     events, then stream)
```

1. **Register the watcher first**, buffering its events — then walk. Walking first would lose edits that land mid-walk; this ordering makes the race harmless (a double re-parse at worst).
2. **Reconciliation walk** (`ignore` walker, respects `.gitignore`, skips `.git/`): collect `(path, mtime_ns, size)` for every Node-kind file; diff against `nodes`; re-parse new/divergent files, delete vanished rows. Equal `mtime_ns` + `size` ⇒ assumed unchanged — the documented approximation; the escape hatch for pathological cases is the rebuild path, never a per-start full parse (NFR-1: cold start < 3 s — a stat-only walk of 5,000 files is well inside it).
3. **Drain** the buffered events through the normal incremental path; go live.

**Debounce: 250 ms quiet window** (`notify-debouncer-full`). An agent merge landing twenty files emits one batch, one transaction, one event. Filtering per event: non-Node extensions are dropped; `.gitignore`-matched paths are dropped; **an event on any `.gitignore` file re-runs the reconciliation walk** (the ignore set itself changed — cheaper to re-diff than to compute rule deltas).

**Incremental re-index** of a debounced batch, one transaction: for each path — deleted ⇒ `DELETE` (links cascade); created/modified ⇒ `vault` parse + `UPSERT` node + replace its links; renames arrive as delete + create. Then emit `graph_changed { paths }` — the single emission path (HLD): reeve's own writes re-enter through the same watcher, so external edits and structural acts are indistinguishable downstream, by design.

The index pushes `GraphChanged` values on a broadcast channel; the composition root forwards them to the `events` emitter (ring 1 owns transport, per the skeleton).

## Link resolution

The four domain rules (02), implemented as one query path over the index. Input: raw target text.

1. **Path-qualified** (contains `/`): match `nodes.path` with the extension stripped, case-insensitive. Zero rows ⇒ broken.
2. **Bare name:** `SELECT path FROM nodes WHERE name = ? COLLATE NOCASE`. One row ⇒ resolved; zero ⇒ broken; more ⇒ **ambiguous**, candidates returned, never a guess.
3. **Case-insensitive on purpose:** Windows filesystems are; two files differing only by case cannot coexist on Tier 1, so case-sensitivity would be a portability trap, not a feature.
4. Backlinks of Node X: `links` rows whose target resolves to X — the bare-name form (when X's name is unambiguous) union the path form. Returned with source titles (the display rule: store names, render titles).

`get_node` composes the contract of 03-api: `vault` reads the file (disk is truth), `index` supplies the per-occurrence resolution table `{ raw, targetPath?, title?, exists, ambiguous, candidates? }` in document order. `validate_links` and `save_doc` warnings reuse the same query path — one resolver, three callers.

## context_assembler

A pure function in `reeve-core`, fed only through the `GraphIndex` seam — deterministic by construction, testable against the in-memory fixture (skeleton), no LLM anywhere (ADR-0003).

**Inputs:** ticket id, per-launch adjustments (`exclude: [names]`, `include: [paths]`), token budget. **Output:** a `ContextPlan` — the DTO both `preview_context` and `start_run` consume — plus a `render_agents_md(plan) -> String` step.

### Candidate set → rank → cut

1. **Hop 0:** the Ticket itself — always included, unconditionally. Its `epic` front-matter counts as an outgoing edge (membership is an edge in spirit).
2. **BFS, 2 hops fixed**, over the union of outgoing links and backlinks, visited-set deduped, minimum hop kept.
3. **Rank**, fully ordered and deterministic: hop ascending → edge kind (outgoing before backlink) → path bytes ascending. No scores, no heuristics beyond hop distance (FR-3.1).
4. **Adjustments:** `exclude` removes candidates before the cut; `include` paths enter as **manual** rank — immediately after the Ticket, ahead of hop 1 (explicit user intent outranks graph inference) — and are included unconditionally.
5. **Token estimate:** `ceil(size / 4)` from the index — a deliberate, labeled approximation. A real tokenizer would be agent-specific (which tokenizer?) and adds a heavy dependency for a number the preview already calls approximate.
6. **Cut — greedy fill in rank order:** include each candidate whose estimate fits the remaining budget; mark it *cut* otherwise and keep scanning (a 50k-token hop-1 doc must not starve every smaller doc behind it). Whole files only — a truncated doc misleads the agent worse than an absent one. Only Markdown bodies are inlined; reachable `.excalidraw`/`.html` Nodes are listed as references with their path.
7. **Budget:** governs the context docs. The Ticket, the conventions section, and the graph index are fixed overhead, reported separately in the preview.

### AGENTS.md rendering

Deterministic string assembly: stable section order, `\n` newlines on every OS, **no timestamp** — the same graph state renders byte-identical output, so a re-preview diff means the graph moved, never that the clock did.

```
# reeve Context Package — T-42 — <title>
## Task                      ← ticket body verbatim (Materialized Region included)
## Resolution Note — required ← the write-back convention (below)
## Context                   ← per included doc: "### <path> — <title> (hop N, link|backlink|manual)" + verbatim content
## Graph index               ← "- <path> — <title>" for every Node, path-sorted; "read further files with your own tools"
```

**Resolution Note convention, fixed here:** write or update `.reeve/notes/T-<n>.md`; it must contain a `[[T-<n>]]` wiki-link in its opening paragraph — that link is what makes the merged note a backlink of its Ticket, i.e. what closes the loop (ADR-0004) with zero special-casing in the graph: the note is just a Node with an edge.

## Seam traits (core-side signatures)

```rust
pub trait Vault: Send + Sync {
    fn read_node(&self, project: &Path, path: &str) -> Result<RawDoc>;     // raw + front-matter + links w/ positions
    fn write_doc(&self, project: &Path, path: &str, content: &str) -> Result<()>;
    fn patch_front_matter(&self, project: &Path, path: &str, patch: FrontMatterPatch) -> Result<()>;
    fn rewrite_region(&self, project: &Path, path: &str, region: RegionContent) -> Result<()>; // semantics: 09
    fn scaffold(&self, project: &Path) -> Result<()>;                      // .reeve/ layout
}

pub trait GraphIndex: Send + Sync {
    fn resolve(&self, project: &Path, target: &str) -> Result<Resolution>;
    fn node_meta(&self, project: &Path, name_or_path: &str) -> Result<Option<NodeMeta>>;
    fn out_links(&self, project: &Path, path: &str) -> Result<Vec<ResolvedLink>>;
    fn backlinks(&self, project: &Path, path: &str) -> Result<Vec<NodeMeta>>;
    fn all_nodes(&self, project: &Path) -> Result<Vec<NodeMeta>>;          // graph-index listing, board, epics
    fn search(&self, project: &Path, query: &str) -> Result<Vec<NodeMeta>>;
    fn ensure_fresh(&self, project: &Path) -> Result<()>;                  // lazy watcher start + reconciliation
}
```

`NodeMeta` carries the indexed columns (path, name, kind, title, size, ticket fields) — enough for the board, the assembler, and every listing query without touching disk.

## Domain amendment

Signed at this ticket, amending [02-domain-model.md](./02-domain-model.md): **Project configuration gains an optional `contextTokenBudget`** (default **32,000**) in `.reeve/config.json` — FR-3.1 requires a budget; the number belongs to the Project because vault sizes differ, and per-launch adjustment already covers the run-level case.

## Handovers

- **08 runs:** Run-history tables live in this same `index.sqlite`; same rebuild policy applies (history is operational metadata, losable by decree of 02).
- **09 sources:** `rewrite_region` primitive is defined here; marker format, refresh semantics, and curated-import flow are 09's.
- **10 frontend:** the resolution table and `byte_start/byte_end` anchors are the editor's decoration contract; `search_nodes` backs quick-open and `[[` autocomplete.

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-27
