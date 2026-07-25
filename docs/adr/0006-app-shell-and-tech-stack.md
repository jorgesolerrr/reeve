# ADR 0006: App shell and core tech stack — Tauri v2, Rust backend, React frontend

**Status:** Accepted (2026-07-25)
**Ticket:** [Decision: app shell and tech stack](https://github.com/jorgesolerrr/reeve/issues/7)

## Context

The requirements ([01-requirements.md](../design/01-requirements.md)) fix the constraints: embedded PTY terminal, rich web UI (diff viewer, Excalidraw canvas, Markdown rendering), git + filesystem access, cross-platform desktop with **Windows as Tier 1**, offline-first, no stored secrets. The competitive landscape shows the field split between Electron (Emdash, Crystal), Rust server + React browser UI (vibe-kanban), and single Go binary (sortie, but headless — no rich UI). The author's declared goals include learning and portfolio value, with no language vetoes.

Two facts narrow the space:

- **Excalidraw is a React component**, so the frontend is React/TypeScript regardless of shell. The real decision is the backend/shell.
- **PTY on Windows is solved in both worlds** — `node-pty` (ConPTY) for Electron, `portable-pty` (WezTerm's crate, ConPTY) for Rust. Neither path is blocked.

## Decision

### Shell: Tauri v2 — Rust backend, React + TypeScript frontend

- On Windows (Tier 1) Tauri renders via **WebView2**, preinstalled on Windows 11 — Tauri's classic weak spot (WebKitGTK on Linux) does not touch our primary platform.
- Small binaries and trivial compliance with the < 3 s cold-start budget (NFR-1).
- A Rust backend (PTY, git orchestration, incremental link index) serves the learning/portfolio goal far better than another Electron app; vibe-kanban proves Rust works in this exact domain.
- Rejected: **Electron** (safe, single-language, but ~150 MB binaries and no learning delta), **local server + browser** (no packaging, but weak desktop integration and an orphan-server lifecycle problem).

### State placement: files are truth, SQLite is cache, git is the workspace registry

- **Docs and tickets**: Markdown files in the repository, versioned by git (already fixed by FR-2.6) — the only source of truth.
- **Graph link index**: **SQLite via `rusqlite`** — a purely **derived, rebuildable cache**. Deleting the database file is always safe; reeve reindexes from the `.md` files. It never holds data that is not in the files (lesson from vibe-kanban `#3434`: derived state scattered across stores desynchronizes).
- **Configuration** (agent profiles, app settings, per-repo verify command): **human-editable JSON files** — readable, hand-editable, backupable; no lock-in in a binary store. Profiles reference environment variables by name (NFR-2), so these files are secret-free by construction.
- **Workspace registry: derived from git, never stored.** The branch name encodes the ticket id (branch `reeve/<ticket-id>`, worktree directory named accordingly under a managed root). Both directions are queries: `git worktree list` enumerates live workspaces; the existence of branch + worktree answers whether a ticket is In Progress. This is what lets FR-1.3 derive board columns from real state — a manually deleted worktree simply disappears from the next query instead of leaving an orphan record to repair.

### Git access: shell out to the system `git`, no embedded library

- libgit2's worktree support is notoriously incomplete and gitoxide does not yet cover merge/worktree fully — a library path would end as a library+CLI hybrid.
- The requirements already assume system git: the FR-4.4 preflight checks a minimum git version, and FR-5.2's "no imposed policy" merge is only honest if the user's own git (config, hooks, autocrlf, signing) performs it.
- The Windows-safe cleanup sequence (kill → delete with retries → `git worktree prune` → verify) is process orchestration either way; a library does not simplify it.
- Stable-output flags (`--porcelain`, `-z`) are the parsing contract.

### Default components

- **Async runtime**: `tokio` (Tauri v2 already runs on it).
- **Terminal**: `portable-pty` (backend) + `xterm.js` (frontend) — pure passthrough, per NFR-1.
- **Frontend build**: Vite.

### Explicitly deferred to the low-level design

Interchangeable details, not structural commitments: Markdown parsing crate, diff viewer component, frontend state-management library, exact monorepo layout.

## Consequences

- The high-level design (04-hld.md) is written against Tauri's process model: Rust core exposing commands/events to a React webview.
- Ticket ids must be stable and branch-name-safe — a constraint handed to the domain-model ticket.
- reeve carries a hard runtime dependency on an installed `git` (≥ minimum version) and, on Windows, WebView2; both are preflight diagnostics (FR-4.4), not silent failures.
- The whole derived state of a vault lives in one deletable SQLite file; "delete the DB" is a supported recovery path, never data loss.
