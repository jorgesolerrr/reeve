# ADR 0005: v1 scope — the minimal complete memory loop

**Status:** Accepted (2026-07-25)
**Ticket:** [Research gate: validate v1 scope and differentiators](https://github.com/jorgesolerrr/reeve/issues/5)

## Context

The research gate (ADR-0001) demands conscious integrate/imitate/drop decisions per part, because most of the category is commodity. The governing criterion for v1: **demonstrate the complete memory loop end to end** — a real error enters as a ticket, the agent starts with inherited context, and after merge the brain knows more than yesterday. Everything that does not serve that demonstration waits.

## Decision

### In v1

1. **Tickets as Markdown nodes** — manual + GitHub Issues sources (`TicketSource` abstraction), simple board/list. *Imitate* vibe-kanban's shape; no innovation here.
2. **Doc graph** — wiki-links + backlinks, thin viewer/editor. Docs are files in the repo. *This is the differentiator.*
3. **Push context assembly** — 1–2 hops + token budget → `AGENTS.md`, with a vault title index (ADR-0003).
4. **Agent execution** — profiles `{command, args, env, cwd}` + promptDelivery + timeout; agnostic PTY terminal; worktree-default `WorkspaceProvider` with reeve-owned Windows cleanup; Windows-first with startup preflight (long paths, git version). *Imitate* Emdash/Conductor, kept thin.
5. **Review** — diff viewer; workspace-derived verification (did anything change? + user's verify command); merge/push following the user's own git flow. The write-back note rides this diff (ADR-0004).
6. **Basic parallelism** — falls out of worktree-per-ticket; simple session UI, no sophisticated orchestration.

### Out of v1, consciously

| Item | Disposition | Reason |
|---|---|---|
| Inline diff comments feeding back to the agent | Defer (fog: loop/hook event model) | Requires session resumption → output parsing, which is excluded; v1 follow-up is manual via terminal |
| Automated loops/hooks | Defer (fog) | Depends on the event model |
| Linear / Sentry / Gmail connectors | Defer (fog) | `TicketSource` in v1 prepares them |
| Pull CLI + skill (`reeve docs`) | Defer, short | Lands right after v1, once push has validated the graph (ADR-0003) |
| Preview/dev-server per workspace | Defer | Commodity table stakes but expensive (ports, collisions); doesn't touch the differentiator |
| Container sandbox | Designed-for, not shipped | Per isolation research; `WorkspaceProvider` keeps it possible |
| Pairing mode (Sculptor-style) | Defer | Nice-to-have |
| Cost/token tracking | **Drop** | Unobtainable agnostically per invocation research; do not promise it |
| "What should I do now" prioritization | Fog | Needs a populated brain first; reinforces ADR-0001 |

## Consequences

- Requirements (issue #6) are written against this list.
- v1 makes no security-boundary claims: a workspace isolates work from other work, not from the machine (isolation research).
- The commodity parts are deliberately thin and may be revisited only after the memory loop demonstrably works.
