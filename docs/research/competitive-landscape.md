# Competitive Landscape: The Agent Workstation / Agent Manager Space

**Date:** 2026-07-25
**Question:** What already exists in the "agent workstation / agent manager" space, what can reeve learn or borrow, and where is the actual gap reeve would fill?

> **Read the [Verdict](#verdict) first if you only read one section.** It is not flattering.

---

## 1. Scope and method

reeve's thesis is a single-user, local-first "builder workstation" that fuses three things:

1. **Linear-style ticketing** — structured work items with state.
2. **Obsidian-style docs** — a local, linked Markdown knowledge base.
3. **Composer-style agent-driven implementation** — agents that pick up work and produce diffs.

This document surveys what already exists along each of those three axes, and — critically — along the *combination*. Sources are primary where possible (repos, official docs, shutdown announcements, issue trackers), secondary where primary sources are thin (comparison articles, community discussion).

A note on the size of the field before we start. The community-maintained [awesome-agent-orchestrators](https://github.com/andyrewlee/awesome-agent-orchestrators) list catalogues roughly **150 tools** across categories like "Parallel Coding Agents — Terminal", "Parallel Coding Agents — Desktop & Web", "Multi-Agent Swarms", "Autonomous Loop Runners", and "Autonomous Task Runners". It also has a section literally titled **"Resting (Inactive)"**. That framing — a curated list that needs a graveyard section — is the single most important fact in this report.

---

## 2. Market map

The space splits into five clusters. reeve as described sits at the intersection of clusters B, D, and E — which is unusual, but each individual cluster is saturated.

| Cluster | What it is | Representative tools |
|---|---|---|
| **A. Agent runtimes** | The agent itself (CLI/model) | Claude Code, Codex, Cursor Composer, Amp, Gemini CLI, OpenCode |
| **B. Local agent managers** | Desktop/TUI shells that run N agents in parallel on isolated checkouts | Conductor, Crystal/Nimbalyst, claude-squad, Emdash, Pane, mux, Sculptor, vibe-kanban |
| **C. Cloud/background agents** | Fire-and-forget agents in remote sandboxes producing PRs | Cursor Cloud Agents, Devin, OpenHands Cloud, Terragon (dead), Codex Cloud |
| **D. Ticket→agent bridges** | Take an existing tracker's issue and turn it into an autonomous run | Linear Agent directory, sortie, symphony, Emdash's importers, Cyrus |
| **E. Spec/docs-driven development** | Durable written artifacts as the primary input to agents | GitHub Spec Kit, AWS Kiro, OpenSpec, BMAD, Tessl, `CLAUDE.md`/skills conventions |

---

## 3. Tool-by-tool

### 3.1 Cursor (Composer + Background / Cloud Agents)

- **What it does.** Agent-first IDE. Since Cursor 2.0 (Oct 2025) the editor's navigation is organised around *agents* rather than files; Cursor 3 (2026) added a dedicated Agents window and Cloud Agents.
- **Task model.** No ticket model. A "task" is a prompt in an agent pane. There is no backlog, no state machine, no persistent work item.
- **Agent integration.** First-party only (Composer, plus hosted frontier models). Not a host for third-party CLIs.
- **Isolation.** Local **git worktrees** for parallel agents (fan out to ~8), or remote cloud VMs on a dedicated branch. Cloud environments are Dockerfile-configurable with build secrets and multi-repo support.
- **Review flow.** In-editor diff review; cloud runs land as pull requests. Notably supports **handoff**: start local, push to cloud, pull back local.
- **Docs story.** Rules files (`.cursor/rules`). No knowledge base, no linked notes.
- **Stack / license / model.** Proprietary, VS Code fork, subscription ($20/mo Pro tier and up).
- **Pain points.** Vendor lock-in to Cursor's own models for the cheapest path; parallelism ceiling is a review-bandwidth problem, not a tooling problem.

Sources: [Cursor 3 deep dive](https://www.digitalapplied.com/blog/cursor-3-deep-dive-agents-composer-review-2026), [Cursor 3 agents window guide](https://www.digitalapplied.com/blog/cursor-3-agents-window-complete-guide), [Cursor 3 review](https://chatforest.com/reviews/cursor-3-anysphere-ai-ide-agent-runtime-composer-2-5-review/)

### 3.2 Conductor (Melty Labs, YC S24)

- **What it does.** Mac app to "run parallel Claude Code, Codex, and Cursor agents in isolated workspaces."
- **Task model.** Workspaces, not tickets. Auto-generated workspace names, persistent history per workstream. No backlog or board.
- **Agent integration.** Wraps existing agent CLIs; **bring your own subscription/API key** — Conductor never sells inference.
- **Isolation.** Git worktrees, one per workspace.
- **Review flow.** Built-in diff viewer, then review and merge. Diff-first UI is its recognised strength.
- **Docs story.** None.
- **Stack / license / model.** Closed source, native macOS. **Free.** No published revenue model — which is itself a risk signal given what happened to Bloop (§4).
- **Pain points.** **Mac-only**; Windows is a waitlist, not a product. Closed source, so no self-hosting or forking. No ticket layer.

Sources: [conductor.build](https://www.conductor.build/), [Conductor changelog](https://www.conductor.build/changelog), [user review](https://alearningjourney.substack.com/p/i-love-using-conductor), [Windows gap](https://runpane.com/alternatives/conductor-windows)

### 3.3 vibe-kanban (BloopAI) — **the most important case study for reeve**

- **What it does.** A kanban board built specifically for AI coding agents: plan on the board → spin up an isolated workspace → agent executes → review diffs inline → merge. Includes a built-in browser preview and click-to-edit.
- **Task model.** **Real kanban issues** with columns and prioritisation — the closest existing thing to reeve's ticketing axis.
- **Agent integration.** Agent-agnostic: 10+ CLIs (Claude Code, Codex, Gemini CLI, Copilot, Amp, Cursor, OpenCode, Droid, CCR, Qwen Code).
- **Isolation.** Git worktree per workspace, each with its own branch, terminal, and dev server.
- **Review flow.** Inline diff comments that feed back to the agent without leaving the UI; PR creation with AI-generated descriptions.
- **Docs story.** None.
- **Stack / license / model.** **Rust backend + TypeScript/React frontend**, pnpm workspace, Node ≥20. **Apache-2.0.** Optional PostHog telemetry, disabled by default. Was free OSS + a paid cloud/Pro tier.
- **Status.** **Bloop, the company, shut down on 10 April 2026.** The stated reason: *"the vast majority are free users and we couldn't find a business model that we could get excited about."* Remote services ran 30 more days, then the product reverted to a fully local architecture. The repo (~26k stars) continues as community-maintained Apache-2.0.
- **Pain points (from the live issue tracker).** Security issues (`#3430` organization invitation redeemable by any account; `#3429` OAuth account-takeover risk), install failures (`#3435` `npx vibe-kanban failed`), auth state that survives DB deletion (`#3434`), a **destructive data bug** (`#3406` "Git repository is wiped after deleting Vibe Kanban workspace"), and `#3408` "is this project dead?".

Sources: [repo](https://github.com/BloopAI/vibe-kanban), [shutdown announcement](https://www.vibekanban.com/blog/shutdown), [issues](https://github.com/BloopAI/vibe-kanban/issues)

### 3.4 Crystal → Nimbalyst (Stravu)

- **What it did.** Electron desktop app to run multiple Claude Code / Codex sessions in parallel git worktrees, compare approaches, track diffs, with git rebase/squash integration and desktop notifications.
- **Task model.** Sessions, not tickets. Session persistence and resume.
- **Isolation.** Worktree per session, one branch each.
- **Stack / license.** Electron + TypeScript, pnpm workspace, Playwright tests. **MIT.**
- **Status.** **Deprecated February 2026**, superseded by **Nimbalyst** — the same team's commercial-ish successor. Nimbalyst manages sessions **on a kanban board**, makes worktrees "a first-class citizen of the session object", ships 7 embedded editors and MCP support, and is **free for individuals with no feature limits**; a Team plan is "coming soon, pricing TBA".
- **Read-through.** Crystal is the canonical example of the OSS-desktop-agent-manager lifecycle: MIT hobby project → traction → rewrite as a funded product → old repo archived. Note that Nimbalyst independently arrived at *kanban board of agent sessions*, i.e. reeve's ticketing axis, without being asked.

Sources: [stravu/crystal](https://github.com/stravu/crystal), [Nimbalyst pricing](https://nimbalyst.com/pricing/), [Crystal successor page](https://nimbalyst.com/crystal/)

### 3.5 claude-squad (smtg-ai)

- **What it does.** Go TUI that manages multiple Claude Code / Codex / Gemini / Aider sessions.
- **Task model.** Sessions with pause/resume/delete. Background completion and an auto-accept "yolo" mode.
- **Isolation.** **tmux** session per agent + **git worktree** per branch.
- **Review flow.** Diff view, checkout changes, commit/push to GitHub from the menu.
- **Stack / license.** Go, **AGPL-3.0**. Requires `tmux` and `gh`.
- **Pain points.** The `tmux` dependency means **no native Windows** (WSL only); weak visual diff review compared to GUI tools; no session persistence across restarts.

Sources: [repo](https://github.com/smtg-ai/claude-squad), [README](https://github.com/smtg-ai/claude-squad/blob/main/README.md), [competitor teardown](https://runpane.com/alternatives/claude-squad)

### 3.6 Emdash (Y Combinator W26) — **the closest thing to reeve that already ships**

- **What it does.** "The Open-Source Agentic Development Environment." Desktop app running multiple coding agents in parallel, no terminal juggling.
- **Task model.** **Imports issues and tickets from Linear, GitHub, Jira, GitLab, and Asana and hands them to agents.** This is reeve's "ticket → agent" loop, already built, against real trackers.
- **Agent integration.** Auto-detects installed CLIs: Claude Code, Codex, Cursor, OpenCode, Amp, Devin, Qwen Code, Droid, Copilot.
- **Isolation.** Git worktree + branch per task; also remote machines over SSH/SFTP with OS keychain integration.
- **Local-first.** State in **local SQLite**; code never leaves the machine (agent CLIs talk to their own providers).
- **Stack / license / platforms.** Electron/Node. **Apache-2.0.** **macOS, Windows, and Linux.** ~5.3k stars, 8,500+ commits, active.
- **Gap vs reeve.** No docs/knowledge layer, and it *consumes* external trackers rather than being one.

Source: [generalaction/emdash](https://github.com/generalaction/emdash)

### 3.7 Sculptor (Imbue)

- **What it does.** "The missing UI for coding agents." Runs parallel agents and previews their changes.
- **Isolation.** **Containers, not worktrees** — each agent gets its own container so it can install packages and run code safely. This explicitly targets the worktree weakness (shared runtime/dependency collisions).
- **Distinctive feature.** **Pairing Mode**: bidirectional sync of a container's work into your local repo, so you can hop into the agent's state in your own IDE.
- **Review flow.** Merge with automatic conflict detection; conflicts can be handed back to the agent. A beta "Suggestions" feature reviews before merge.
- **Session model.** Every session persists plans, chats, tool calls, and code changes; reopenable later.
- **Stack / license / model.** Closed source. **Free during beta**, BYO Anthropic access (API key or Pro/Max). **Mac (Apple Silicon) and Linux; Windows forthcoming.** Imbue has raised ~$232M, so monetisation pressure exists but terms are unpublished.

Sources: [imbue.com/sculptor](https://imbue.com/sculptor/), [announcement](https://imbue.com/blog/sculptor-announce)

### 3.8 Terragon Labs — **dead**

- **What it was.** A remote background-agent orchestrator: give it a task, it spins up a fresh sandbox, clones the repo, branches, runs Claude Code / Codex / Amp / Gemini, and produces a PR.
- **Isolation.** One isolated sandbox container per agent, each with its own repo copy.
- **Stack.** Node 20+, pnpm, Turbo monorepo, Drizzle ORM, PostgreSQL + Redis via Docker, Cloudflare R2, Stripe.
- **Status.** **Shut down 16 January 2026.** The repo is an as-is Apache-2.0 snapshot "with no guarantees of maintenance, support, or completeness."

Source: [terragon-labs/terragon-oss](https://github.com/terragon-labs/terragon-oss)

### 3.9 Devin (Cognition)

- **What it does.** Fully autonomous cloud software engineer: ticket in, PR out.
- **Task model.** Sessions billed in **ACUs** (~15 min of active work). Core plan $20 pay-as-you-go at $2.25/ACU; Team $500/mo including 250 ACUs at $2.00/ACU.
- **Isolation.** Fully managed remote cloud workspace with browser, shell, and editor.
- **Docs story.** Devin Wiki / knowledge features exist for repo context, but they are Devin's context, not the user's editable knowledge base.
- **Pain points.** Consistent 2026 criticism of **opaque, unpredictable ACU billing**; strong ROI only on "clearly specifiable" work (migrations, refactors) for teams of 5+; weak on nuanced requirements.

Sources: [Devin pricing analysis](https://brainroad.com/devin-pricing-in-2026-real-cost-hidden-spend-and-alternatives/), [2026 review](https://comparateur-ia.com/en/reviews/devin)

### 3.10 OpenHands (All Hands AI, formerly OpenDevin)

- **What it does.** Open-source autonomous SWE agent: writes code, runs tests, fixes bugs, opens PRs.
- **Isolation.** Sandboxed **Docker** runtime.
- **Task model.** Conversations, plus GitHub/GitLab/Bitbucket and **Jira/Slack** integrations for issue-driven work.
- **License / model.** **MIT** core. Free self-hosted; free BYOK cloud SaaS; Enterprise (custom pricing) adds VPC/Kubernetes self-hosting, an Agent Control Plane, SSO/SAML, RBAC, budget enforcement. Raised an **$18.8M Series A** (Nov 2025) led by Madrona.
- **Read-through.** This is the "open core + enterprise" answer to the business-model question that killed Bloop. It requires an enterprise buyer, which a single-user tool by definition does not have.

Sources: [pricing](https://www.openhands.dev/pricing), [Series A](https://www.businesswire.com/news/home/20251118768131/en/OpenHands-Raises-$18.8M-Series-A-to-Bring-Open-Source-Cloud-Coding-Agents-to-Enterprises)

### 3.11 Linear (the incumbent that moved)

- **What it does.** In 2026 Linear repositioned as **"the product development system for teams and agents."** Its agent directory lists **28+ integrations**: Codex, Cursor, GitHub Copilot, Devin, Charlie, Factory, Cyrus, Warp's Oz, plus QA, docs, and observability agents.
- **Delegation model.** You *assign an issue to an agent* the same way you assign it to a human. Assignment fires a webhook; the agent picks it up, works, and reports back into the issue as an agent session. Third parties build against Linear's **Agent SDK**.
- **Claude Code.** Notably still **no first-party Claude Code integration** — the community fills it with [Cyrus](https://hookdeck.com/webhooks/platforms/how-to-run-claude-code-as-a-linear-agent-with-cyrus-and-hookdeck-cli).
- **Read-through.** This is the direct refutation of "you have to leave your tool to go to Linear." Linear became agent-native. The escape-hatch problem reeve wants to solve is being solved from the tracker side, not just the workstation side.

Source: [Linear agent integrations](https://linear.app/integrations/agents)

### 3.12 Claude Code itself (Anthropic) — **the platform risk**

The agent vendor is absorbing the workstation layer:

- **April 2026:** complete Claude Code desktop redesign around **parallel sessions** — multi-session sidebar filterable by status/project/environment and groupable by project, drag-and-drop layout, integrated terminal, in-app file editor, a diff viewer rebuilt for large changesets, an expanded preview pane, side chat, and **Routines** (scheduled/automated agent work).
- **July 2026:** a **sandboxed in-app browser** (Cmd+Shift+B), configurable, running a clean profile, closing the "agents can code but can't see the web" gap without MCP glue.

That is: multi-session management, isolation, diff review, preview, and automation — first-party, free with the subscription, on the same platform reeve targets.

Sources: [MacRumors on the desktop rebuild](https://www.macrumors.com/2026/04/15/anthropic-rebuilds-claude-code-desktop-app/), [redesign guide](https://miraflow.ai/blog/claude-code-desktop-redesign-parallel-sessions-routines-workspace-guide), [in-app browser](https://www.digitalapplied.com/blog/claude-code-desktop-sandboxed-browser-agents-2026)

### 3.13 Also-rans worth knowing

| Tool | Notes |
|---|---|
| **[Pane](https://runpane.com/alternatives/claude-squad)** | AGPL-3.0, free, **Windows/macOS/Linux native**, keyboard-first, diff viewer, full git workflow, session persistence. Requires only git — no tmux. Explicitly positions against claude-squad's Windows gap. |
| **[mux](https://github.com/coder/mux)** (Coder) | AGPL-3.0, desktop + browser + VS Code extension. Three runtimes: **local / worktree / SSH**. Plan/Exec mode, cost tracking, Ollama + OpenRouter. macOS/Linux. |
| **[symphony](https://github.com/openai/symphony)** (OpenAI) | Apache-2.0 **specification** (with an Elixir reference impl). Watches project boards like Linear, spawns runs, and requires **evidence of completion** (CI status, review feedback, complexity analysis) before merging. Explicit framing: *"manage work instead of supervising coding agents."* Engineering preview. |
| **[sortie](https://github.com/sortie-ai/sortie)** | Apache-2.0, single **Go** binary + SQLite, no job queue. Turns **GitHub Issues / Gitea / Linear / Jira** tickets into autonomous agent sessions with configurable states (todo → in-progress → review → done), retry logic, state reconciliation, cost tracking. Claude Code, Copilot, OpenCode, Codex, Kiro. |
| **Superset** | YC-backed, macOS, source-available, free tier + **$20/seat/month** — a rare published price in this category. |
| **[Huly](https://openalternative.co/huly) / Plane** | The serious open-source Linear alternatives. Huly bundles issues + docs + chat + calendar self-hosted, but the stack (MongoDB + Elasticsearch + object storage) is heavy and it is not agent-native. |
| **Spec-driven tooling** | **AWS Kiro** generates `requirements.md` (EARS notation) + `design.md` + `tasks.md` before code; **GitHub Spec Kit** ships a CLI/templates/prompts pipeline (specify → plan → tasks → implement) that works across Copilot, Claude Code, Gemini CLI, Cursor. Also OpenSpec, BMAD, Tessl, Google Antigravity. |

---

## 4. Cross-cutting patterns

### 4.1 What the field has converged on (i.e. what is now table stakes, not innovation)

1. **Worktree-per-task isolation.** Universal. Git worktrees became load-bearing for AI coding around Q1 2026 — as soon as two agents edit one repo, they race on lockfiles and overwrite each other. Every tool in cluster B does this identically.
2. **Agent-agnostic CLI wrapping + BYO credentials.** Nobody resells inference. The wrapper is a thin process manager over `claude`, `codex`, etc.
3. **Diff-first review UI** with per-hunk or inline commenting that feeds back to the agent.
4. **Local SQLite state, local-first framing.** Emdash, sortie, and others already claim this exact positioning.
5. **Preview/dev-server per workspace.** vibe-kanban, Nimbalyst, Claude Code desktop.
6. **Session persistence and resume.**

### 4.2 What nobody has solved

- **Semantic conflicts.** Worktrees solve *file* collisions, not *semantic* ones. Two agents can produce individually-clean diffs that are jointly incoherent.
- **Shared runtime.** Ports, databases, env vars, and seeded state are still contended across worktrees. This is precisely why Sculptor moved to containers, at the cost of Pairing-Mode complexity.
- **The review bottleneck.** This is the actual ceiling. 4 parallel agents = 4× review burden; community guidance converges on ~4 concurrent agents max. Verification, not generation, is the constraint. Most open-source orchestrators "still leave task alignment, conflict resolution, and merge decisions on the user's plate."
- **Durable project memory.** Every tool treats a task as ephemeral: prompt → diff → merge → forget. The reasoning, the rejected approach, the constraint discovered on attempt three — all of it dies in a transcript nobody re-reads. `CLAUDE.md` and rules files are the industry's answer, and they are a flat, manually-curated text file.

Sources: [worktree tools comparison](https://nimbalyst.com/blog/best-git-worktree-tools-ai-coding-2026/), [orchestration tools survey](https://www.tembo.io/blog/ai-agent-orchestration-tools), [HN consensus 2026](https://www.developersdigest.tech/blog/what-hacker-news-gets-right-about-ai-coding-agents-2026)

### 4.3 The graveyard — read this twice

| Tool | Fate | Date |
|---|---|---|
| **Terragon** | Shut down; Apache-2.0 as-is snapshot | Jan 2026 |
| **Crystal** | Deprecated; superseded by Nimbalyst | Feb 2026 |
| **Bloop / vibe-kanban** | Company shut down; ~26k-star OSS project handed to the community | Apr 2026 |
| **humanlayer** | 11.2k stars, repo self-described as "pretty much all deprecated", rebuilt elsewhere | 2026 |
| **~15 tools** | The awesome-list's explicit "Resting (Inactive)" section, most with last commits in Feb–Apr 2026 | 2026 |

Bloop's exit line is the thesis statement of this entire category:

> "the vast majority are free users and we couldn't find a business model that we could get excited about."

The survivors are (a) funded companies with an enterprise motion (OpenHands, Devin, Cursor), (b) loss-leaders for a model vendor (Claude Code desktop, Codex), or (c) hobby projects with no cost base. reeve is explicitly (c), which is actually the *survivable* category for a solo dogfood project — but it means the strategic reasoning should be "this is a tool I maintain for myself and a portfolio artefact," not "this is an underserved market."

---

## 5. What reeve can learn or borrow

**Borrow directly:**

- **Worktree-per-task with branch + terminal + dev server**, the vibe-kanban model. Do not innovate here; copy the proven shape.
- **Inline diff comments that route back to the agent as a new turn** (vibe-kanban's best feature, and the highest-leverage review-loop primitive in the field).
- **Agent-agnostic CLI adapters + BYO credentials.** Never resell inference. Detect installed CLIs like Emdash does.
- **Single local SQLite state file, no server** (sortie's single-binary + SQLite architecture is the cleanest reference; Go, Apache-2.0, readable).
- **Evidence-of-completion gating before merge** — symphony's spec is the best articulation of this and it's Apache-2.0 and language-agnostic by design.
- **Container-or-worktree as a per-task choice** (mux offers local/worktree/SSH; Sculptor is container-only). Worktrees are cheaper; containers fix the shared-runtime problem.
- **Pairing Mode** (Sculptor): let the user drop into the agent's workspace in their own editor. This is the single most-praised UX idea in the field.

**Learn from the failures:**

- **The destructive-bug class is real and reputational.** vibe-kanban `#3406` wipes a git repository when a workspace is deleted. Worktree lifecycle management is where these tools hurt people. Treat worktree teardown as a safety-critical path from day one.
- **Auth/state reset must work.** vibe-kanban `#3434`: deleting the SQLite DB doesn't reset auth. Keep all state in one place, deletable.
- **Don't ship a remote/cloud tier.** Every product that did (Bloop, Terragon) died of it. Local-only is the right call and matches the survivable cost profile.
- **Windows is a genuine hole.** Conductor is Mac-only with a Windows waitlist. Sculptor is Mac AS + Linux. mux is Mac/Linux. claude-squad needs tmux, so WSL only. Only Emdash and Pane ship first-class Windows. The author develops on Windows 11 — build Windows-first and it's differentiated by accident.

---

## 6. Where the three axes actually stand

| reeve's axis | Already solved? | By whom |
|---|---|---|
| Parallel agents in isolated checkouts | **Fully solved, commoditised, free** | Conductor, Emdash, Pane, mux, claude-squad, Nimbalyst, Claude Code desktop |
| Diff review + feedback loop | **Fully solved** | vibe-kanban, Conductor, Pane, Cursor, Claude Code desktop |
| Kanban board over agent tasks | **Solved** | vibe-kanban, Nimbalyst, agent-kanban, multica, openkanban |
| Ticket → agent run, with state machine | **Solved** | sortie, symphony, Emdash, Linear Agent SDK + 28 integrations |
| Local-first, OSS, cross-platform, single-user | **Solved** | Emdash (Apache-2.0), Pane (AGPL-3.0) |
| "Never leave the tool" (tracker inside the workstation) | **Partially — and being attacked from the other side** | Linear became agent-native; GitHub/Copilot likewise |
| **Obsidian-style linked doc graph as a first-class peer of tickets and agent runs** | **Not solved** | Nobody in cluster B has any docs story at all |
| **Docs as the retrieval substrate and the output of agent runs** | **Not solved; only crude approximations** | Spec Kit / Kiro produce per-feature spec files; `CLAUDE.md` is a flat file; Devin Wiki is Devin's, not yours |

---

## 7. Verdict

### 7.1 Two-thirds of reeve is already built, free, and open source

Be blunt about this. **The agent-orchestration layer of reeve is a solved commodity.** Worktree isolation, parallel sessions, diff review, agent-agnostic CLI wrapping, local SQLite state, kanban-over-agents — all of it exists today in tools you can `git clone` under Apache-2.0 or AGPL-3.0. **Emdash** in particular is the uncomfortable one: Apache-2.0, local-first SQLite, Windows/macOS/Linux, worktree-per-task, nine agent CLIs auto-detected, *and it already imports tickets from Linear, GitHub, Jira, GitLab, and Asana*. Roughly 70% of reeve's stated scope is Emdash's current feature list, shipped, with a YC batch behind it and 8,500 commits.

The ticketing axis is also weaker than it looks. **Linear did not stand still.** It repositioned as a system "for teams and agents", shipped an Agent SDK, and now lists 28+ agents you can assign an issue to. The premise "a builder has to leave the tool to go to Linear" is being dissolved from Linear's side. And the ticket→agent bridge specifically is now a solved, boring problem with at least three credible implementations (sortie, symphony, Emdash) plus every native Linear integration.

And the platform risk is severe. **Anthropic is building the workstation.** The April 2026 Claude Code desktop redesign is a multi-session workspace with a project-grouped sidebar, integrated terminal, file editor, a diff viewer built for large changesets, a preview pane, and scheduled Routines. July 2026 added a sandboxed in-app browser. That is Conductor plus half of vibe-kanban, first-party, free with the subscription. Any feature reeve builds in cluster B should be assumed to be six months from being free in the client reeve depends on.

### 7.2 The one-third that is genuinely unclaimed

Here is the honest good news, and it is narrower and more interesting than "a builder workstation."

**Not a single tool in the agent-manager category has a documentation story at all.** Not vibe-kanban, not Conductor, not Nimbalyst, not Emdash, not Pane, not mux, not Sculptor, not claude-squad. Zero. The entire category treats a task as: prompt → worktree → diff → merge → *forget*. The reasoning behind a decision, the approach that was rejected and why, the constraint discovered on attempt three — none of it is captured as a durable, linkable, queryable artefact. The industry's answer is a flat, hand-maintained `CLAUDE.md`.

The nearest approximations all fall short in specific, exploitable ways:

- **Spec Kit / Kiro** produce spec files *per feature*, forward-only. They are inputs to a build, not a graph, and nothing links a spec to the run that implemented it or the ADR that later contradicted it. There is no backlink, no browse surface, no decay management.
- **Obsidian + agent plugins** give you the graph but no ticket model, no worktree, no run history — you're gluing it yourself.
- **Devin Wiki** is the vendor's internal repo context, not the user's editable second brain.
- **Huly** gives you docs + issues in one self-hosted app but is not agent-native at all and requires a MongoDB/Elasticsearch stack.

So the defensible thesis is **not** "Linear + Obsidian + Composer in one app." It is narrower:

> **A ticket, its docs, and its agent transcripts are one durable, linked, local Markdown artefact — and the doc graph is both the retrieval context for the next run and a deliverable of the last one.**

That closes a loop nobody closes: agent runs *write back* into the knowledge base (ADRs, domain notes, gotchas), and the knowledge base is what the next run retrieves from. Institutional memory for a team of one. If reeve builds that and it works, it is a real contribution, and it is the part worth putting on a portfolio.

### 7.3 Strategic implications, stated plainly

1. **Do not rebuild the orchestrator.** Every hour spent on worktree lifecycle, terminal multiplexing, and diff viewers is an hour spent reimplementing four Apache-2.0 projects, and it is the part most likely to be obsoleted by Claude Code desktop. Borrow the shape from vibe-kanban/sortie; keep it deliberately thin.
2. **Make the docs layer the load-bearing wall, not a tab.** If docs are a feature of the workstation, reeve is a worse Emdash. If the workstation is a feature of the docs graph, reeve is something new. Design the ticket as a node in the doc graph, not as a row in a table that happens to have a description field.
3. **Windows-first is a free, if minor, differentiator.** Conductor: Mac-only. Sculptor: Mac AS + Linux. mux: Mac + Linux. claude-squad: WSL. It is a real hole and the author lives in it.
4. **Never ship a hosted tier.** Bloop and Terragon both died there. Local-only, BYO credentials, no cost base — this is the only configuration in which a solo project in this category survives.
5. **Be honest in the README about what this is.** "A learning and dogfooding project that closes the docs↔tickets↔agents loop for one builder" is true, defensible, and interesting. "The builder workstation that replaces GitHub and Linear" is a claim four funded companies made and two of them are dead.

### 7.4 The uncomfortable summary

If the author's hope is *"nobody has built this yet"* — that hope is mostly wrong. The market is not underserved; it is **oversupplied and consolidating**, with ~150 catalogued entrants, a dedicated inactive section on the canonical list, and three notable shutdowns in the first four months of 2026 alone. The orchestration idea is not a gap; it is a commodity with a body count.

But the *docs* half is a real, verifiable, currently-empty hole — and it is empty for an understandable reason: it's the part that doesn't demo well, doesn't produce a screenshot of eight agents running at once, and doesn't get YC funding. Which is exactly why it is available to someone building for themselves rather than for a seed round.

Build reeve. Build it around the doc graph. Keep the orchestrator thin and borrowed. And drop the "replaces GitHub and Linear" framing — it invites a comparison reeve will lose, against tools that already ship what it hasn't built yet.

---

## Sources

**Primary (repos, official docs, announcements)**
- [BloopAI/vibe-kanban](https://github.com/BloopAI/vibe-kanban) · [issues](https://github.com/BloopAI/vibe-kanban/issues) · [shutdown announcement](https://www.vibekanban.com/blog/shutdown)
- [stravu/crystal](https://github.com/stravu/crystal) · [Nimbalyst pricing](https://nimbalyst.com/pricing/) · [Crystal successor](https://nimbalyst.com/crystal/)
- [smtg-ai/claude-squad](https://github.com/smtg-ai/claude-squad) · [README](https://github.com/smtg-ai/claude-squad/blob/main/README.md)
- [generalaction/emdash](https://github.com/generalaction/emdash)
- [coder/mux](https://github.com/coder/mux)
- [openai/symphony](https://github.com/openai/symphony)
- [sortie-ai/sortie](https://github.com/sortie-ai/sortie)
- [humanlayer/humanlayer](https://github.com/humanlayer/humanlayer)
- [terragon-labs/terragon-oss](https://github.com/terragon-labs/terragon-oss)
- [conductor.build](https://www.conductor.build/) · [changelog](https://www.conductor.build/changelog)
- [imbue.com/sculptor](https://imbue.com/sculptor/) · [Sculptor announcement](https://imbue.com/blog/sculptor-announce)
- [Linear agent integrations](https://linear.app/integrations/agents)
- [OpenHands pricing](https://www.openhands.dev/pricing) · [Series A](https://www.businesswire.com/news/home/20251118768131/en/OpenHands-Raises-$18.8M-Series-A-to-Bring-Open-Source-Cloud-Coding-Agents-to-Enterprises)
- [andyrewlee/awesome-agent-orchestrators](https://github.com/andyrewlee/awesome-agent-orchestrators)

**Secondary (analysis, comparisons, community)**
- [Best git worktree tools for AI coding 2026](https://nimbalyst.com/blog/best-git-worktree-tools-ai-coding-2026/)
- [AI agent orchestration tools (Tembo)](https://www.tembo.io/blog/ai-agent-orchestration-tools)
- [9 open-source agent orchestrators (Augment)](https://www.augmentcode.com/tools/open-source-agent-orchestrators)
- [What Hacker News gets right about AI coding agents in 2026](https://www.developersdigest.tech/blog/what-hacker-news-gets-right-about-ai-coding-agents-2026)
- [The Code Agent Orchestra — Addy Osmani](https://addyosmani.com/blog/code-agent-orchestra/)
- [Cursor 3 deep dive](https://www.digitalapplied.com/blog/cursor-3-deep-dive-agents-composer-review-2026) · [Cursor 3 agents window](https://www.digitalapplied.com/blog/cursor-3-agents-window-complete-guide)
- [Anthropic rebuilds Claude Code desktop around parallel sessions (MacRumors)](https://www.macrumors.com/2026/04/15/anthropic-rebuilds-claude-code-desktop-app/) · [redesign guide](https://miraflow.ai/blog/claude-code-desktop-redesign-parallel-sessions-routines-workspace-guide) · [sandboxed browser](https://www.digitalapplied.com/blog/claude-code-desktop-sandboxed-browser-agents-2026)
- [Devin pricing reality check](https://brainroad.com/devin-pricing-in-2026-real-cost-hidden-spend-and-alternatives/) · [Devin 2026 review](https://comparateur-ia.com/en/reviews/devin)
- [Pane vs claude-squad comparison](https://runpane.com/alternatives/claude-squad) · [Conductor Windows gap](https://runpane.com/alternatives/conductor-windows)
- [Running Claude Code as a Linear agent with Cyrus](https://hookdeck.com/webhooks/platforms/how-to-run-claude-code-as-a-linear-agent-with-cyrus-and-hookdeck-cli)
- [Best spec-driven development tools 2026](https://www.augmentcode.com/tools/best-spec-driven-development-tools) · [Kiro / Spec Kit / BMAD guide](https://medium.com/@visrow/comprehensive-guide-to-spec-driven-development-kiro-github-spec-kit-and-bmad-method-5d28ff61b9b1)
- [Huly (open-source Linear/Notion alternative)](https://openalternative.co/huly) · [Plane vs Huly vs Taiga](https://www.pistack.xyz/posts/plane-vs-huly-vs-taiga-self-hosted-project-management-guide-2026/)
