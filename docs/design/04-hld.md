# 04 — High-Level Design

**Status:** Signed off (2026-07-26)
**Ticket:** [Design: high-level design (04-hld.md)](https://github.com/jorgesolerrr/reeve/issues/10)
**Grounded in:** [01-requirements.md](./01-requirements.md) · [02-domain-model.md](./02-domain-model.md) · [03-api.md](./03-api.md) · [ADR-0006](../adr/0006-app-shell-and-tech-stack.md)
**Visual companion:** [04-hld.html](./04-hld.html) — the component architecture and the application flow as one self-contained page.

## Purpose & scope

This document fixes reeve's internal structure: the layering rule, the module inventory, how data flows, how processes are owned, where everything lives on disk, and the extension points. It guarantees the domain/UI separation that keeps a future server extraction evolutionary (NFR-3). Interchangeable components — Markdown crate, diff viewer, frontend state library, monorepo layout — stay deferred to the low-level design per ADR-0006.

## The three rings

The Rust core is organized in three rings with a strict inward dependency rule:

1. **Adapters** (edge) — Tauri commands and the event emitter. The only code that knows Tauri exists. Each command is the mechanical 1:1 binding over a neutral operation (03-api). A future HTTP server is a second adapter in this ring — nothing below it changes.
2. **Core** (middle) — the service modules exposing the ~35 operations, the shared domain components, and the seam traits. Domain law lives here: derived board states, Run sequentiality, the commit policy, deterministic link resolution. No Tauri types, no SQL, no process spawning.
3. **Infrastructure** (bottom) — the concrete wrappers over the outside world: system git, SQLite, the repository filesystem, PTY, the `gh` CLI. Infrastructure implements the traits the core defines (dependency inversion): the core owns the interface, infrastructure owns the mechanism.

**The rule, stated verifiably:** no `use tauri::` outside adapters; no `rusqlite`, `portable_pty`, or `std::process` outside infrastructure. A violation is a design bug by definition, greppable in review. This rule — not any framework — is what makes server extraction "replace ring 1, touch nothing else."

## Module inventory

### Ring 1 — adapters

| Module | Responsibility |
|---|---|
| `commands` | One `#[tauri::command]` per operation; maps args, delegates, serializes result, translates `ApiError`. Anything beyond delegation is a design bug (03-api). |
| `events` | Emits the four events (`graph_changed`, `workspace_changed`, `run_exited`, `pty_output`) to the webview. The single place event transport is known. |

### Ring 2 — core

**Services**, 1:1 with the API areas: `projects`, `graph`, `tickets`, `epics`, `sources`, `workspaces`, `runs`, `review`, `system`. Each owns its area's operations and nothing else's.

**Shared domain components** — not API-visible, used by several services, first-class boxes because they are reeve's differentiating logic:

| Component | Responsibility | Used by |
|---|---|---|
| `context_assembler` | The deterministic Context Package algorithm (FR-3.1): from the Ticket node, follow links/backlinks 1–2 hops, rank by hop distance, cut to token budget, render `AGENTS.md` (including the Resolution Note convention) plus the graph index. A pure function of the graph — no LLM, no I/O decisions of its own. | `runs.preview_context`, `runs.start_run` |
| `board_derivation` | The derived-state rules, existing exactly once: In Progress = live Workspace; In Review = live Workspace ∧ last Run exited ∧ diff non-empty; Backlog by elimination; Done from front-matter. | `tickets.get_board`, `workspaces.get_workspace` |

**Seam traits** (NFR-3), defined in the core, implemented in infrastructure:

- `TicketSource` — the Strategy behind the `sources` service. v1 strategies: `manual` (degenerate: always present, never configured) and `github`. Registration is static in v1 — a match arm plus a config-union variant; the public plugin architecture remains mapped fog.
- `WorkspaceProvider` — workspace isolation. v1 implementation: git worktree + branch `reeve/T-<n>`. A container sandbox is a later implementation behind the same trait, no caller changes.

### Ring 3 — infrastructure

| Module | Responsibility |
|---|---|
| `git` | Shell-out to system git (`--porcelain`, `-z` as the parsing contract). Worktree lifecycle including the Windows-safe destroy sequence (kill → delete with retries → prune → verify). Implements `WorkspaceProvider` for worktrees. |
| `index` | The SQLite link index (`rusqlite`) plus the file watcher and incremental reindexing. Purely derived, per-Project, deletable (ADR-0006). |
| `vault` | Repository filesystem access: read/write Markdown, parse front-matter, extract wiki-links, rewrite Materialized Regions, `.reeve/` scaffold. |
| `pty` | `portable-pty`: spawn, stream, resize, kill; writes each Run's raw log file. |
| `gh_client` | GitHub operations over the `gh` CLI's existing auth (NFR-2). Implements the `github` `TicketSource` strategy. |

### Frontend

React + TypeScript in the Tauri webview. Five surfaces, no more:

1. **Board** — the four fixed columns, Epic grouping, intake (create / import).
2. **Node** — viewer/editor by kind: Markdown (editor, clickable wiki-links, backlinks panel), Excalidraw (embedded editor), HTML (sandboxed, view-only). An open Ticket is this surface with its front-matter presented.
3. **Workspace** — a Ticket's working surface, two tabs: **Terminal** (xterm.js, Run history, launch/relaunch/verify, Context Package preview) and **Review** (diff, verify signal, link warnings, and the three endings: merge / push / discard). Review is a tab, not a surface: under 1:1:1 a review is always *this* workspace's state, and launch → inspect → relaunch is a constant ping-pong.
4. **Settings** — Agent Profiles (machine), Project config (Verify Command, Source, `autoCommit`), Project registry.
5. **Preflight** — the FR-4.4 diagnostics, actionable.

The data layer is a thin **query cache** keyed by operation + args, subscribed to the invalidation events: `graph_changed { paths }` invalidates queries touching those paths; `workspace_changed { ticketId }` invalidates that ticket's board/workspace queries. On invalidation the UI re-queries what is on screen — truth is always re-derived by the core at query time. Pure UI state (active project, open tabs, per-launch Context Package adjustments) lives only in the frontend; the core holds no session state. The concrete state library is an LLD choice.

## Data flow

- **Reads** are snapshot queries; derived truth (board columns, workspace existence) is recomputed at query time from git and process state — nothing crossing the boundary can go stale silently.
- **Invalidation** is coarse and scope-carrying, never data-carrying. `graph_changed` has exactly one emission path: the index's file watcher. Reeve's own structural acts are file writes too, so external edits, merges landing Resolution Notes, and reeve's own commands all invalidate through the same channel — no special case.
- **Writes** split per the commit policy: structural acts are dedicated commands whose file rewrites the core owns (auto-committed under `.reeve/` when the flag is on); content authorship is `save_doc`, written verbatim, committed by the user.
- **The one push exception** is `pty_output`, the high-frequency stream feeding xterm.js — ephemeral session data, not state.

## The index subsystem

- **Lazy start:** a Project's watcher starts on the first operation scoped to that Project (opening it in the UI already queries) and lives until app close. Watchers are cache maintenance, not session state — truth still derives at query time.
- **Reconciliation on watcher start:** a cheap directory walk comparing mtime/size against the indexed state; only divergent files are re-parsed. At the 5,000-node ceiling this respects the < 3 s cold start (NFR-1).
- **Full rescan** is the explicit recovery path ("delete the DB", ADR-0006), never a per-start cost.
- **Incremental thereafter:** watcher events re-index only changed files and emit `graph_changed { paths }`, debounced.

## Process model

- **The run registry** lives in memory inside the `runs` service: a map of ticketId → live process, one lock per Workspace. This is where sequentiality is law — `start_run` fails with `workspace/run_active` while an entry exists.
- **Run history** (kind, profile, timestamps, exit code, log path) persists in the Project's SQLite — operational metadata in the rebuildable cache, per the domain model. **Raw PTY logs** are plain files under `~/.reeve`, outside the repo, outside the Graph.
- **Reeve owns its children.** Closing the app kills live Run processes, with a UI warning when any are live. Orphaned agents writing to worktrees with no observer would falsify board derivation (In Review requires "last Run exited") and have no re-attach path — a PTY cannot be re-adopted across process lifetimes.
- **Crash reconciliation:** on startup, any Run row with no exit code and no live process is marked **interrupted**. The in-memory registry starts empty, so board derivation is correct by construction; the worktree keeps whatever the agent wrote — the diff remains reviewable.

## On-disk placement

One visible home, `~/.reeve/`, instead of the platform app-data tree — files-first (inspectable, hand-editable), short paths (a real budget on Windows, where worktree builds nest deep), and consistent with sibling tools. This consciously refines the domain model's "machine's app-data directory" wording for the cache location.

```
~/.reeve/
├── config.json                    # Project registry + app settings
├── profiles.json                  # Agent Profiles (machine-level, env names only)
├── projects/<slug>/
│   ├── index.sqlite               # derived cache — deletable, per Project
│   └── logs/T-42/<timestamp>.log  # raw PTY log per Run
└── worktrees/<slug>/T-42/         # the managed WorkspaceProvider root
```

`<slug>` is the repo directory name plus a short hash of the absolute path — readable and collision-free. One SQLite per Project: "no cross-Project anything" is domain law, and the delete-the-DB recovery path stays scoped to one Project.

The repository itself keeps the committed `.reeve/` layout fixed in the domain model (config, tickets, epics, notes); files in the repo remain the only truth.

## Startup sequence

1. **Preflight** (FR-4.4): long paths, git version, PTY availability, agent CLIs on PATH — actionable diagnostics before anything else.
2. Load the machine config (Project registry, Agent Profiles).
3. **Crash reconciliation** over run history (mark interrupted).
4. UI opens; opening a Project triggers its watcher start + mtime reconciliation, then the board query.

## Server extraction, concretely

The guarantee NFR-3 asks for, restated against this design: ring 2 and ring 3 compile without Tauri; every operation is stateless with `project` explicit; events are plain values handed to ring 1 for transport. Extraction = write an HTTP/socket adapter in ring 1, re-bind the same operations and events, redefine the `project` identity format. The UI's query-cache-plus-invalidation pattern works identically over a socket.

## Decisions handed to the LLD

Per-module internals (schemas, crate choices, exact trait signatures), the Markdown parser, the diff viewer component, the frontend state library, the monorepo layout, and the enforcement mechanics of the ring rule (lint/CI). The LLD tickets graduate from the map's fog on this document's sign-off, including the code-reading research on Emdash, vibe-kanban, and sortie (ADR-0002).

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-26
