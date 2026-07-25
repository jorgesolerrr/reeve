# ADR 0003: Deterministic push context assembly; pull as CLI + skill, not MCP

**Status:** Accepted (2026-07-25)
**Ticket:** [Research gate: validate v1 scope and differentiators](https://github.com/jorgesolerrr/reeve/issues/5)

## Context

The doc graph grows without bound; an agent run must receive bounded, relevant context, never the whole vault. Two delivery mechanisms exist: **push** (reeve assembles context before the run) and **pull** (the agent queries the graph on demand). Pull via MCP inherits a documented failure mode (Copilot CLI #3064: MCP server fails to start, agent silently runs with no tools), costs tokens for tool schemas, and adds a server lifecycle.

## Decision

- **v1 ships push, deterministic, with no LLM in the retrieval path:** start at the ticket node, follow outgoing wiki-links and incoming backlinks **1–2 hops**, rank by hop distance, cut to a **token budget**, and write the resulting subgraph into the workspace via `AGENTS.md` (the universal steering channel already adopted). The context package also includes a title/path index of the vault so the agent can read further files itself with its own filesystem tools — docs are plain files in the repo.
- **Pull is delivered as an agent-invocable CLI + a curated skill, not as an MCP server.** reeve exposes commands (e.g. `reeve docs search`, `reeve docs links <note>`) that any agent can run in a terminal like `grep`; `AGENTS.md` teaches their use. This is agent-agnostic, has no server lifecycle, and costs a fraction of MCP's token overhead. Its exact landing slot is immediately after v1 (see ADR-0005); the HLD must account for reeve shipping a CLI surface.

Retrieval quality improves with use: every write-back (ADR-0004) adds links, so the loop curates its own index.

## Consequences

- Retrieval is reproducible and debuggable ("why did the agent know X?" → follow the links). No vector DB, no embeddings in v1.
- The graph module must expose link-traversal queries that both the push assembler and the future CLI wrap.
- MCP remains out of the agent-profile design entirely, consistent with the invocation research.
