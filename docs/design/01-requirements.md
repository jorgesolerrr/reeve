# 01 — Requirements

**Status:** Signed off (2026-07-25)
**Ticket:** [Requirements: functional and non-functional requirements](https://github.com/jorgesolerrr/reeve/issues/6)
**Grounded in:** [ADR-0001](../adr/0001-center-reeve-on-the-docs-memory-loop.md) · [ADR-0002](../adr/0002-build-from-scratch-no-emdash-fork.md) · [ADR-0003](../adr/0003-deterministic-push-context-retrieval.md) · [ADR-0004](../adr/0004-write-back-rides-the-diff.md) · [ADR-0005](../adr/0005-v1-scope.md)

## Purpose & scope

reeve is an open-source, free, single-user, local-first tool that closes the work↔memory↔agents loop for one builder: tickets, docs, and agent transcripts are one durable, linked, local Markdown graph, and that graph is both the retrieval context for the next agent run and a deliverable of the last one (ADR-0001). The visible UX is an Emdash-like flow — intake → board → agent → review → merge — over a doc-graph data model. reeve does not replace GitHub or Linear.

This document fixes what the v1 system must do (functional requirements) and how it must behave (non-functional requirements). Entity modeling, API design, and architecture belong to the subsequent design documents.

## Core flow

A unit of work enters as a ticket (typed in or imported), appears on the board, gets an isolated workspace where an agent starts with context assembled from the graph, produces a diff that includes a resolution note, and is reviewed and merged by the user — at which point the note joins the graph and the next run starts knowing more.

## Functional requirements

Requirements are phrased as "the user can …" / "reeve …". Areas marked ★ are the **core three** — they constitute the memory loop, reeve's differentiator (ADR-0001). The unmarked areas are supporting commodity, deliberately kept thin (ADR-0005).

### FR-1 — Ticketing

- **FR-1.1** The user can create a ticket manually. A ticket is a Markdown node in the doc graph (title, body, front-matter), not a row with a description field.
- **FR-1.2** The user can import GitHub Issues from a repository as tickets. The source contract is: **import** (issue → Markdown node with front-matter pointing at repo + number + URL), **refresh on demand** (new issues appear; edited titles/bodies update; no automatic polling), **offer to close** the GitHub issue when the local ticket reaches Done (with an optional comment linking the work — reeve offers, never closes on its own), and **never write content to the remote** (local notes and links are a local layer; no remote body edits, no bidirectional sync).
- **FR-1.3** The user can see tickets on a board with exactly four fixed states: **Backlog**, **In Progress**, **In Review**, **Done**. Columns are not configurable. In Progress and In Review are derived from real workspace state (a live workspace exists / a diff awaits review), not hand-maintained.
- **FR-1.4** The user can group tickets one level deep within a repository (a feature/milestone-like grouping; the repository itself is the implicit project). The exact modeling of the grouping is decided in the domain-model document, not here.
- **FR-1.5** The user can relate tickets to other tickets and docs with plain wiki-links. There are no typed relations (blocks, duplicates, parent-of) in v1; backlinks provide the inverse direction.

### FR-2 — Doc graph ★

- **FR-2.1** The user can view, edit, and create **Markdown** docs in-app, rendered with clickable wiki-links and a backlinks panel.
- **FR-2.2** The user can view, edit, and create **Excalidraw** docs in-app (embedded editor).
- **FR-2.3** The user can view **HTML** docs in-app, rendered sandboxed (see NFR-2). HTML is view-only; it is created outside reeve or by agents.
- **FR-2.4** Any other file type is not a doc: reeve opens it with the system default application. Agents see all files as plain files regardless.
- **FR-2.5** Wiki-links and backlinks span the whole graph, but **only Markdown files contribute edges** (outgoing links are parsed from `.md` only). Excalidraw and HTML files are linkable nodes — they receive links and appear in backlinks — but their contents are not parsed in v1.
- **FR-2.6** Docs are files in the repository, versioned by git; reeve is a viewer/editor on top, never a silo.

### FR-3 — Context assembly ★

- **FR-3.1** Before an agent run, reeve assembles a context package deterministically, with no LLM in the retrieval path (ADR-0003): starting at the ticket node, follow outgoing wiki-links and incoming backlinks 1–2 hops, rank by hop distance, cut to a token budget, and write the result into the workspace as `AGENTS.md`, together with a title/path index of the vault so the agent can read further files with its own tools.
- **FR-3.2** The user can preview the assembled package before launch — which nodes made it in and their approximate token cost — and adjust it **per-launch**: exclude a node or add a specific doc for this run only. Adjustments are not persisted. Launching with the default package is a single confirmation (Enter).

### FR-4 — Agent execution & workspaces

- **FR-4.1** The user can define, edit, and delete **agent profiles**: name, command, args, env (references to user environment variables, per NFR-2), prompt delivery, and completion timeout. reeve stays agent-agnostic: no output parsing, no per-agent protocols (invocation research).
- **FR-4.2** Starting work on a ticket creates one isolated workspace (git worktree on its own branch — `WorkspaceProvider` interface, worktree implementation in v1). The relation is **1:1:1 ticket ↔ workspace ↔ branch**; a ticket has at most one live workspace. Parallelism is across tickets, not within one.
- **FR-4.3** Inside a workspace the user can launch the profile's agent, relaunch it, and open an agnostic terminal (PTY passthrough) for manual follow-up — sequentially, in the same workspace.
- **FR-4.4** On startup reeve runs a **preflight**: long paths enabled (Windows), minimum git version, PTY availability, configured agent CLIs found on PATH. Failures produce actionable diagnostics, not cryptic errors.

### FR-5 — Review & merge ★

- **FR-5.1** The user can review the workspace diff against the base branch, alongside a verification signal: did anything change, plus the output of the user's own verify command (per-repo configuration).
- **FR-5.2** After review the user can: **merge locally** into the base branch of their choice (no imposed policy — no forced squash, no generated messages), **push the branch** to the remote, or **discard** the workspace (destroy it with the Windows-safe cleanup sequence: kill processes → delete with retries → prune → verify; a failed experiment leaves no residue). Opening a PR is not a reeve action in v1 — push, then use `gh` in the terminal reeve already provides.
- **FR-5.3** The agent's resolution note **rides the diff** (ADR-0004): `AGENTS.md` fixes the note convention (location, front-matter with the ticket link), the note appears in the same diff as the code, and merging is the approval. On merge, reeve validates wiki-links and warns about broken ones.

## Non-functional requirements

### NFR-1 — Performance

- UI interactions (board, opening a ticket, following a link): **< 100 ms** perceived.
- Graph operations (context assembly, backlink computation): **< 1 s** with a vault of up to **5,000 nodes**. Design consequence: the link index must be incremental (re-index only changed files), never a full rescan.
- Cold start to usable board: **< 3 s**.
- Workspace creation (worktree + `AGENTS.md`): **< 10 s** on a typical (~500 MB) repository — the one acceptable slow operation, paid once per ticket.
- The embedded terminal is PTY passthrough; its latency is the agent's, not reeve's.

### NFR-2 — Security

- HTML docs render in a **sandbox** with no filesystem access and no access to app APIs — agent-generated HTML is untrusted input by definition.
- reeve **stores no secrets**: agent profiles reference user environment variables by name, never values; GitHub operations ride the `gh` CLI's existing authentication — reeve neither requests nor stores tokens.
- **No telemetry, no own network calls** other than the GitHub operations the user explicitly invokes.
- Explicit non-claim (isolation research): a workspace isolates work from other work, **not** the agent from the machine. v1 makes no security-boundary claims.

### NFR-3 — Extensibility

- Ticket sources sit behind a single internal `TicketSource` interface, and **both** v1 sources (manual, GitHub) are implemented against it — proof by construction that the seam exists. Adding a source (e.g. Linear) is new code plus registration, zero core changes.
- Workspace isolation sits behind `WorkspaceProvider` — worktree is the v1 implementation; a container sandbox fits later without touching callers.
- Domain and UI stay separated cleanly enough that a future server extraction is evolution, not rewrite.
- These are internal seams, stable in shape; a **public plugin API is out of v1** (fog: plugin/extension architecture).

### NFR-4 — Portability

- **Tier 1 — Windows**: dogfooded daily; every FR works here; Windows-safe worktree cleanup and the startup preflight are requirements, not enhancements.
- **Tier 2 — macOS and Linux**: supported with full feature parity, verified best-effort (CI + issues), not daily dogfooding.
- **No platform-exclusive features in either direction**: if something cannot work on Windows, it does not enter v1.

### NFR-5 — Offline

- Everything local works with no network: board, docs, graph, context assembly, workspaces, terminal, diff, local merge.
- Network operations (GitHub import/refresh/close-offer, push) **fail explicitly and never block**: clear message, local state intact, the user retries. No offline queue in v1.
- Agents need their own network for their APIs; that is the agent's concern — reeve launches the process regardless and the outcome is visible in the terminal.
- As a rule: **reeve never blocks a local operation waiting on the network, and no local feature degrades offline.**

## Scale assumptions

Single user, one machine. Vault ceiling for design purposes: **5,000 nodes**. Typical repository: ~500 MB. Tickets in flight: tens, not hundreds. The only capacity consideration that changes a design decision is the vault ceiling, which forces the incremental link index (NFR-1); everything else is small by the definition of single-user local-first.

## Out of scope (v1)

Inherited from [ADR-0005](../adr/0005-v1-scope.md) and the map:

| Item | Disposition |
|---|---|
| Inline diff comments feeding back to the agent | Deferred — loop/hook event model |
| Automated loops/hooks | Deferred — loop/hook event model |
| Linear / Sentry / Gmail connectors | Deferred — `TicketSource` prepares them |
| Pull CLI + skill (`reeve docs`) | Deferred, lands right after v1 (ADR-0003) |
| Preview/dev-server per workspace | Deferred |
| Container sandbox | Designed-for, not shipped |
| Pairing mode | Deferred |
| Cost/token tracking | **Dropped** (unobtainable agnostically) |
| "What should I do now" prioritization | Fog — needs a populated brain |
| Opening PRs as a reeve action | Not in v1 — push + `gh` in the terminal |
| Multi-user / collaborative server, Jira/GitLab, monetization | Out of scope for the effort (map) |

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-25
