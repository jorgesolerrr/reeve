# 03 — API Surface

**Status:** Signed off (2026-07-26)
**Ticket:** [Design: API surface (03-api.md)](https://github.com/jorgesolerrr/reeve/issues/9)
**Grounded in:** [01-requirements.md](./01-requirements.md) · [02-domain-model.md](./02-domain-model.md) · [ADR-0006](../adr/0006-app-shell-and-tech-stack.md)

## Purpose & scope

This document fixes the boundary between reeve's UI and its core: what operations exist, what crosses the boundary, how errors and events flow, and how the boundary stays extractable. It decides *the contract*, not the implementation — module internals belong to the high-level and low-level designs.

## Boundary model

**The API is a set of transport-neutral operations; Tauri commands are a mechanical binding.**

The operations defined here live in the Rust core as plain service modules, with no Tauri types in their signatures. Each `#[tauri::command]` is a 1:1 adapter: map arguments, delegate, serialize the result, translate the error. **If a command does anything beyond delegating, that is a design bug** — the anti-ceremony rule. A future HTTP server (the multi-user door: closed but oiled, NFR-3) would be a second adapter over the same operations, an evolution rather than a rewrite.

Operation names are `verb_noun` in snake_case, identical in the neutral operation and its Tauri command (`create_ticket`, `get_board`, `start_run`), so the 1:1 binding is verifiable at a glance. Modules group the areas; names carry no area prefix.

## Interaction patterns

Exactly **two** patterns, deliberately. If a future operation fits neither, that is the signal to revisit this section — not to add a third pattern silently.

1. **Request/response.** Every command is an awaitable promise, even when it takes seconds (workspace creation < 10 s, Windows-safe discard, network refresh). No job ids, no queues, no progress polling. **No cancellation**: the only killable thing is a Run, and killing it is a domain operation (`kill_run`), not command cancellation.
2. **Process-attached.** A Run: `start_run` returns as soon as the process starts; liveness flows through the PTY event stream and terminates with a `run_exited` event.

### Reads: pull + scoped invalidation

All reads are queries returning snapshots. The core never pushes state; it emits **coarse invalidation events carrying scope, not data** (`graph_changed { paths }`, `workspace_changed { ticketId }`), and the UI re-queries whatever it has on screen. Truth is always re-derived at query time — derived board states, git-backed workspace existence — so nothing that crosses the boundary can go stale silently (the vibe-kanban lesson, per ADR-0006).

**One declared exception:** the PTY stream. Terminal output flows core → xterm.js over a dedicated high-frequency event channel (`pty_output`), bypassing the query/invalidation cycle. It is ephemeral session data, not state.

## Addressing & identity

Every Project-scoped operation takes an explicit `project` parameter — the **absolute path of the repository**, which is the Project's identity (1:1 with the repo; unique and readable on a single-user machine). The core holds no "active project" session state: selection is a UI notion. Moving a repository on disk means re-registering it — a rare, accepted cost in v1. The installation's Project registry lives in the machine-level JSON config alongside Agent Profiles.

## Content contract

Opening a Node returns **raw Markdown plus structured metadata**: the raw content, the parsed front-matter (title, done, epic, source reference), and the document's **link resolution table** — for each `[[name]]`: target path, display title, exists?, ambiguous?. **The core resolves; the frontend renders.** Resolution applies the four deterministic rules of the domain model against the SQLite index; rendering (clickable links, hover previews, the editor) is React's job. The core never pre-renders HTML.

Writes are split along the same line the commit policy draws:

- **Structural acts are dedicated commands** — `mark_done`, `assign_epic`, `refresh_source`, `import_items`. The core rewrites front-matter or materialized regions itself (and auto-commits under `.reeve/` when the flag is on). The API never expresses "mark done" as "you edit the YAML".
- **Content authorship is `save_doc`** — the file is written exactly as sent; committing it is the user's business. The user may still edit any file by hand outside reeve; files-first means the file is the truth either way.

## Error model

Every operation returns `Result<T, ApiError>` with a single envelope:

```
ApiError = { code, message, details? }
```

- **`code`** — a stable machine-readable value from one enum, grouped by area: `git/*`, `source/*`, `workspace/*`, `fs/*`, `internal`. The UI branches on codes, never on message text.
- **`message`** — human and actionable, in the spirit of the preflight (FR-4.4): what happened and what to do, never cryptic.
- **`details`** — structured payload when the UI must decide: `workspace/run_active` (a Run is live; offer kill), `source/network { operation }` (retry), `git/dirty { files }`.

Rules:

- **Warnings are not errors.** Operations whose happy path can carry advisories (`merge` link validation per FR-5.3, `save_doc` with an ambiguous link) return `warnings: []` inside the *result*. Warnings never block and never travel on the error channel.
- **Network failures are explicit and non-blocking** (NFR-5): `source/*` network errors leave local state intact and the user retries. Nothing local ever waits on the network.
- **No secrets in errors** (NFR-2): no env values, no tokens, ever, in `message` or `details`.
- **Unexpected failures** (bugs, panics) collapse to `internal` with a pointer to the log file — a stacktrace never crosses the boundary.

## Operation catalog

Eight Project-scoped areas mirroring the core's service modules, plus one machine-level area. `project` (the repo path) is implicit in every signature below except **system**.

### projects

| Operation | Contract |
|---|---|
| `register_project(path)` | Registers an existing git repository; creates `.reeve/` scaffold if absent. |
| `unregister_project()` | Removes from the registry; touches no files in the repository. |
| `list_projects()` | Registered Projects with basic health (path exists, is a git repo). Machine-level, no `project` param. |
| `get_project_config()` / `update_project_config(config)` | Read/write `.reeve/config.json`: Verify Command, default Agent Profile (by name), Source configuration, `autoCommit` flag, `baseBranch` *(amended by 07-lld-workspaces)*. |

### graph

| Operation | Contract |
|---|---|
| `get_node(name_or_path)` | Raw content + parsed front-matter + link resolution table (see [Content contract](#content-contract)). |
| `get_backlinks(name)` | Nodes linking to this one, with titles for display. |
| `search_nodes(query)` | Title/name search over the index (quick-open, link autocomplete). |
| `get_graph_index()` | The title/path index of all Nodes — the same index the Context Package embeds. |
| `create_doc(path, content?)` | New Markdown or Excalidraw Doc at a user-chosen path. |
| `save_doc(path, content)` | Writes the file verbatim; result may carry link warnings. |

### tickets

| Operation | Contract |
|---|---|
| `create_ticket({ title, body?, epic? })` | Assigns the next `T-<n>`, writes `.reeve/tickets/T-<n>.md`, auto-commits per policy. |
| `get_board()` | The four fixed columns with card DTOs (id, title, epic, source badge). In Progress / In Review are derived at query time from git + process state. |
| `mark_done(ticketId)` / `reopen_ticket(ticketId)` | Sets/clears `done` (+ date) in front-matter. On done of an imported Ticket, the UI follows with the close-remote dialog (see sources). |
| `assign_epic(ticketId, epicId?)` | Writes/clears `epic` in the Ticket's front-matter (membership points upward; `null` clears). |

### epics

| Operation | Contract |
|---|---|
| `create_epic({ title, body? })` | Assigns `E-<n>`, writes `.reeve/epics/E-<n>.md`. |
| `list_epics()` | All Epics with title (board grouping, assignment picker). |
| `list_epic_tickets(epicId)` | Inverse view of front-matter membership. |

Deliberately thin: membership lives on the Ticket (domain model), so the mutating verb is `assign_epic` under **tickets**.

### sources

The area where the `TicketSource` seam surfaces (see [TicketSource seam](#the-ticketsource-seam)). Operations speak *sources*, never GitHub.

| Operation | Contract |
|---|---|
| `list_sources()` | Configured Sources: kind, config, status. Manual is implicit and never listed. |
| `configure_source(config)` / `remove_source(sourceId)` | v1: at most one GitHub source, defaulting to the `origin` remote's issues. |
| `list_source_items(sourceId)` | Remote items in agnostic shape — external id, title, state, URL, `imported: ticketId?` — the single door for both initial import and later discovery. |
| `import_items(sourceId, externalIds[])` | Materializes each item as a new Ticket (id assigned, region written, auto-commit per policy). |
| `refresh_source(sourceId, ticketId?)` | **Curated refresh**: rewrites the Materialized Regions of already-imported Tickets wholesale; *new* remote items appear in `list_source_items` as importable — never auto-imported. Optional `ticketId` scopes to one Ticket. |
| `close_source_item(ticketId, comment?)` | Closes the remote issue with an optional linking comment. Invoked only after the user accepts the offer-to-close dialog — reeve offers, never closes on its own (FR-1.2). |

### workspaces

| Operation | Contract |
|---|---|
| `create_workspace(ticketId)` | Worktree + branch `reeve/T-<n>` (1:1:1). Awaitable, budgeted < 10 s. |
| `get_workspace(ticketId)` | Derived status from git + processes: exists?, path, branch, live Run?, diff non-empty?. |
| `discard_workspace(ticketId)` | Windows-safe destroy: kill processes → delete with retries → prune → verify. Removes worktree **and** branch — a failed experiment leaves no residue. Ticket returns to Backlog. |

### runs

| Operation | Contract |
|---|---|
| `preview_context(ticketId, adjustments?)` | **Pure function**: the package plan — included nodes with hop distance, approximate token cost, total, what the budget cut — with nothing created in the core. Adjustments (`exclude: [names]`, `include: [paths]`) live in UI state while the user tweaks. |
| `start_run(ticketId, kind)` | `kind = agent { profile?, adjustments? } \| terminal \| verify`. Returns once the process starts; output flows via `pty_output`, death via `run_exited`. Only `agent` regenerates `AGENTS.md` (adjustments travel here, whole — the launched package reflects the graph at launch time, never a stale preview); `terminal` and `verify` leave it untouched. Fails with `workspace/run_active` if a Run is live — sequentiality is the core's law. Relaunch is not an operation: it is `start_run` again. |
| `kill_run(ticketId)` | Kills the live Run's process. The only cancellable thing in the API. |
| `write_stdin(ticketId, data)` / `resize_pty(ticketId, cols, rows)` | Terminal I/O; `ticketId` suffices because at most one Run is live per Workspace. |
| `list_runs(ticketId)` | The **ticket's** sequential Run history: kind, profile (agent runs), start, exit code, log file. Survives merge/push — a `done` ticket's history stays readable; discard erases it *(amended by 08-lld-runs)*. |
| `read_run_log(ticketId, runId, tailBytes?)` | Log content as lossy UTF-8, bounded tail (2 MiB) by default — history inspection and scrollback restore without a second data path around the API *(added by 08-lld-runs)*. |

### review

| Operation | Contract |
|---|---|
| `get_diff(ticketId)` | Summary against base: files with status and +/- counts. |
| `get_file_diff(ticketId, path)` | One file's patch, fetched lazily (large diffs stay cheap). |
| `validate_links(ticketId)` | Pre-decision query: broken/ambiguous wiki-links the diff would introduce; shown alongside the diff. |
| `merge(ticketId, baseBranch)` | Merges with the user's own git — no imposed policy, no generated messages. Sets `done`, destroys the Workspace (worktree + branch), re-validates links and returns warnings in the result. **Never blocks on broken links** (FR-5.3: warn, not prevent). For imported Tickets the UI chains the offer-to-close dialog. |
| `push(ticketId, remote?)` | Pushes the branch; removes the worktree but **keeps the local branch** (the PR opened outside reeve references it); does **not** set `done` — the Ticket derives back to Backlog until the user marks it done. |

The deliberate asymmetry among the three Workspace endings: merge and discard delete the branch; push preserves it. That is their only difference, and it follows from each verb's purpose.

### system (machine-level, no `project` param)

| Operation | Contract |
|---|---|
| `run_preflight()` | FR-4.4 diagnostics: long paths (Windows), git minimum version, PTY availability, configured agent CLIs on PATH. Actionable results, not booleans. |
| `list_profiles()` / `save_profile(profile)` / `delete_profile(name)` | Agent Profile CRUD against the machine-level JSON config. Env entries are variable *names*, never values (NFR-2). |

## Events catalog

Four events, closed list:

| Event | Payload | Emitted by |
|---|---|---|
| `graph_changed` | `{ project, paths[] }` | The incremental index's file watcher: external edits, merges landing Resolution Notes, reeve's own structural acts (which are file writes too). |
| `workspace_changed` | `{ project, ticketId }` | Workspace created / ended / discarded — anything that moves derived board columns. |
| `run_exited` | `{ project, ticketId, runKind, exitCode }` | Death of any Run's process. The one event carrying an extra datum: the UI needs `exitCode` immediately for the review prompt (agent) or the pass/fail signal (verify) without a re-query. |
| `pty_output` | `{ project, ticketId, data }` | The declared exception: the high-frequency stream feeding xterm.js. |

Deliberately absent: `run_started` (the UI initiated it and already knows from the command's response), source events (refresh is request/response; its outcome returns on the promise), progress events (no jobs). One fact can fan out to two events for two subscribers: a Run's death emits `run_exited` (immediate UI reaction) *and* `workspace_changed` (board invalidation — In Progress → In Review is pure derivation).

## The TicketSource seam

The seam (NFR-3) is a **Strategy pattern**, visible at the API boundary:

- The *operations* and *DTOs* are generic — `list_source_items` returns the same agnostic item shape for any kind; `refresh_source` has one signature. Adding Linear changes **zero** API operations: a new strategy behind the seam, a registration, and one more variant in the config union.
- The *config* is typed per kind — a discriminated union (`github: { repo }`), not opaque JSON. With two kinds in v1 we do not pay for a plugin system (that is mapped fog: plugin architecture); we only keep the signatures stable.

Manual is the degenerate strategy: always present, never configured, never listed, contributes no source reference.

## Server extraction

What "closed but oiled" means concretely, given this design:

- Every operation is transport-neutral and stateless (no active project, no session, no draft objects); `project` is an explicit parameter everywhere — the exact property a multi-tenant server needs.
- The Tauri command layer is mechanically replaceable by an HTTP layer: same operations, same envelope, same events (server-sent or socket-pushed).
- The identity format (`project` = local path) is the one thing a server would redefine — accepted, because the oiled door requires the *parameter* to exist, not its format to be eternal.

## Domain model amendment

Signed at this ticket, amending [02-domain-model.md](./02-domain-model.md) and [CONTEXT.md](../../CONTEXT.md): **Run kinds are `agent | terminal | verify`**. Running the Verify Command is a Run — a process running in the Workspace whose outcome matters — giving it the PTY stream (tests visible live), the exit code as the pass/fail signal accompanying the diff (FR-5.1), sequentiality (no verify against a half-written tree), and a place in the Run history ("when did tests last pass?") with no new pattern. Like `terminal`, a `verify` Run never regenerates `AGENTS.md`.

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-26
