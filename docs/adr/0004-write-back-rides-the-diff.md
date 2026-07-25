# ADR 0004: Write-back rides the diff; merge is the approval

**Status:** Accepted (2026-07-25)
**Ticket:** [Research gate: validate v1 scope and differentiators](https://github.com/jorgesolerrr/reeve/issues/5)

## Context

Each run should leave a resolution note in the doc graph (the decision, the rejected approach, the discovered constraint). Unfiltered agent notes would poison the graph — and since the graph is the retrieval index (ADR-0003), bad notes degrade every future run. Options considered: (a) auto-commit notes (zero friction, guaranteed poisoning), (b) a separate note-approval inbox (new UX surface, a queue that decays into "approve all"), (c) the note travels in the diff.

## Decision

**Option (c).** Docs are files in the repo and the agent works in a worktree, so the agent writes its resolution note as a Markdown file in the workspace and **the note appears in the same diff as the code**. The user reviews, edits, or deletes it during normal diff review; **merging is the approval**. No new UX, no separate queue, and curation happens at the moment of maximum context.

Guardrails:

1. `AGENTS.md` fixes the note convention — location, front-matter with the ticket link, what it should link to — so the graph stays well-formed regardless of which agent wrote the note.
2. reeve validates wiki-links on merge and warns about broken links.

## Consequences

- Note quality control costs the user nothing beyond the review they already do.
- Agent transcripts are linked as opaque artifact pointers (paths), never parsed, consistent with the invocation research.
- Personal notes that should not live in the repo remain an open fog item ("docs outside the repo") on the map.
