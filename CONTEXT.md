# reeve

reeve is a single-user, local-first agent workstation that closes the work↔memory↔agents loop: tickets, docs, and agent transcripts form one durable, linked, local Markdown graph that is both the retrieval context for the next agent run and a deliverable of the last one.

## Language

**Project**:
A git repository registered in reeve, 1:1 with the repository. Each Project is a closed context: it owns its board, its tickets, and its doc graph. Wiki-links never cross Projects.
_Avoid_: Repo (when meaning the reeve-level concept), workspace, vault

**Graph**:
A Project's memory: every Node in the repository plus the wiki-link edges between them. The whole repository participates (respecting `.gitignore`); reeve-created Nodes live under the committed `.reeve/` directory, and the rebuildable index cache lives outside the repository.
_Avoid_: Vault, wiki, knowledge base

**Node**:
Anything addressable in a Project's Graph: a file that can receive wiki-links and appear in backlinks. Every Ticket, Doc, and Epic is a Node. A Node's **name** (filename without extension; the id for Tickets and Epics) is its identity and what links store; its **title** is what the UI always displays — humans never read raw ids.
_Avoid_: Page, document (as the umbrella term)

**Ticket**:
A unit of work; a Markdown Node whose front-matter carries ticket metadata. Identified by a per-Project sequential id (`T-42`) that is stable, never reused, and independent of any source-side number. The board is a view over Tickets, not a separate store.
_Avoid_: Issue (reserved for the GitHub-side object), task, card

**Board**:
The per-Project view of Tickets across four fixed states: Backlog, In Progress, In Review, Done. In Progress and In Review are derived from live workspace state; Done is the only stored state, written to the Ticket's front-matter on merge or by hand.
_Avoid_: Kanban, pipeline

**Epic**:
The one-level grouping of Tickets: a Markdown Node describing a feature or milestone, identified as `E-<n>`. A Ticket belongs to at most one Epic, declared in the Ticket's front-matter; Epics never nest and never belong to other Epics.
_Avoid_: Milestone, feature, group

**Doc**:
A knowledge Node that is not a Ticket: Markdown, Excalidraw, or HTML. Only Markdown Docs contribute outgoing edges to the graph.
_Avoid_: Note, page

**Source**:
An origin of work configured in a Project. v1 kinds: Manual (always present, implicit) and GitHub (at most one per Project, defaulting to the Project repository's own issues). An imported Ticket carries a source reference in its front-matter (kind, coordinates, URL); a manual Ticket carries none.
_Avoid_: Connector, integration, provider

**Materialized Region**:
The marker-delimited section of an imported Ticket's body owned by its Source: title, body, and comments copied verbatim from the remote and rewritten wholesale on refresh. Everything outside the region is local — notes, wiki-links, agent additions — and refresh never touches it.
_Avoid_: Synced section, remote body

**Workspace**:
The isolated place where a Ticket's work happens: a git worktree on the Ticket's own branch (`reeve/T-<n>`). Strictly 1:1:1 Ticket ↔ Workspace ↔ branch; a Ticket has at most one live Workspace.
_Avoid_: Sandbox, environment, worktree (when meaning the domain concept)

**Agent Profile**:
A machine-level definition of how to launch an agent: name, command, args, env references, prompt delivery, completion timeout. Profiles belong to the reeve installation, not to a Project; each Project picks a default profile, overridable per Run.
_Avoid_: Agent (for the configuration), integration

**Verify Command**:
A Project-level command (e.g. `npm test`) whose output accompanies the diff during review as the verification signal. Configured per Project because each repository has its own way of checking itself.
_Avoid_: Test command, CI

**Run**:
One process launched inside a Workspace, of kind `agent` (the profile's agent), `terminal` (manual PTY passthrough), or `verify` (the Verify Command; exit code = pass/fail). A Workspace accumulates Runs sequentially, never concurrently. Runs are operational metadata, not Nodes; a Run's raw PTY log is kept for inspection but stays outside the graph.
_Avoid_: Session, execution, job

**Context Package**:
The deterministically assembled payload an agent starts from: the Ticket's neighborhood of Nodes ranked by hop distance, cut to a token budget, plus a title/path index of the Graph, written into the Workspace as `AGENTS.md`. Ephemeral and derived — regenerated per Run, never a Node.
_Avoid_: Prompt, context window, briefing

**Resolution Note**:
The Markdown note an agent writes as part of its diff, following the convention fixed by the workspace's `AGENTS.md` (location, front-matter linking the Ticket). It is a Run's distilled contribution to the graph; merging it is what makes the loop close. Raw transcripts never enter the graph.
_Avoid_: Transcript, summary, report
