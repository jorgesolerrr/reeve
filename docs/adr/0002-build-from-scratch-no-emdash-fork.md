# ADR 0002: Build from scratch; study Emdash, do not fork it

**Status:** Accepted (2026-07-25)
**Ticket:** [Research gate: validate v1 scope and differentiators](https://github.com/jorgesolerrr/reeve/issues/5)

## Context

Emdash (Apache-2.0) already ships roughly 70% of reeve's orchestration scope: worktree-per-task, 9 agent CLIs auto-detected, local SQLite, tracker imports, cross-platform. Forking it is legal without ambiguity (Apache-2.0 permits forks with license/NOTICE preservation and change attribution). The question was whether forking is the right starting point.

## Decision

**Build reeve from scratch. Do not fork Emdash.** Use Emdash, vibe-kanban, and sortie as study references during low-level design.

Reasons:

1. **reeve's differentiator is a foundations change, not a facade change.** ADR-0001 makes tickets Markdown nodes in a linked graph with a memory loop at the core. Emdash's data model is the opposite (SQLite rows, ephemeral tasks, no docs). Retrofitting a foundational data model into a foreign 8,500-commit codebase is typically more work than greenfield.
2. **Forking decides the tech stack by the back door.** Emdash is Electron + Node; the stack decision (issue #7) is deliberately open and should be made on its own merits after the research gate.
3. **The project's declared purpose is learning and portfolio.** The 70% Emdash already solves is the commodity part — the least instructive and the part Claude Code desktop is commoditizing further.

## Consequences

- Code-reading research tickets (Emdash: CLI detection, worktree lifecycle, tracker imports; vibe-kanban: Rust architecture and its documented destructive-cleanup bug; sortie: single-binary + SQLite shape) will be created when low-level design tickets graduate.
- reeve copies proven shapes from these tools rather than innovating in the orchestration layer.
