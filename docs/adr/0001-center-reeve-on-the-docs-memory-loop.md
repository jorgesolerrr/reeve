# ADR 0001: Center reeve on the docs-memory loop

**Status:** Accepted (2026-07-25)
**Ticket:** [Research gate: validate v1 scope and differentiators](https://github.com/jorgesolerrr/reeve/issues/5)

## Context

The competitive research ([competitive-landscape.md](../research/competitive-landscape.md)) found that ~70% of reeve's originally stated scope — worktree isolation, parallel sessions, diff review, agent-agnostic CLI wrapping, local SQLite state, kanban-over-agents, ticket→agent bridges — is a solved commodity, shipped free and open source (Emdash being the closest overlap), in a category with heavy attrition (Terragon, Crystal, Bloop all dead or deprecated in early 2026). Linear became agent-native, dissolving the "you must leave your tool" premise from the tracker side, and Anthropic is absorbing the workstation layer into Claude Code desktop.

The one verifiably empty space: **no tool in the category has any documentation story.** Every competitor treats a task as prompt → diff → merge → forget. Durable project memory — the reasoning, the rejected approach, the constraint discovered on attempt three — does not survive anywhere except a hand-maintained flat `CLAUDE.md`.

## Decision

reeve's thesis is re-centered from "Linear + Obsidian + Composer in one app" to the narrow, defensible version:

> A ticket, its docs, and its agent transcripts are one durable, linked, local Markdown artifact — and the doc graph is both the retrieval context for the next run and a deliverable of the last one.

Concretely, this is a **hybrid**:

- **UX**: the visible flow stays intake → board → agent → review → merge (the Emdash-like flow the user wants). reeve is not a wiki app; the user mostly does not "use docs".
- **Data model**: tickets are **Markdown nodes in a linked doc graph** from day one (wiki-links + backlinks), not rows in a table with a description field. The minimal memory loop — assemble context from links before a run, write learnings back on close — is a mandatory v1 feature, even while the docs-browsing UI stays thin.
- **Framing**: reeve does not claim to replace GitHub or Linear. It "closes the work↔memory↔agents loop for one builder" — a learning and dogfooding project, honestly labelled.
- **Orchestrator**: built (it is needed to close the loop) but deliberately thin, imitating the proven shapes (vibe-kanban, sortie, Emdash) without innovating there.

## Consequences

- The doc graph is a foundational data-model commitment; retrofitting it later was judged more expensive than building on it now.
- Every subsequent design decision (requirements, domain model, HLD) treats the memory loop as the load-bearing wall and the orchestration layer as borrowed commodity.
- The future "what should I do now" prioritization idea depends on this layer being populated, which reinforces the choice.
- Windows-first support remains the secondary differentiator (most competitors are Mac/Linux-only).
