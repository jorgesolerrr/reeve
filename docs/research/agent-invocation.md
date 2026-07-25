# Agent invocation, completion detection, and emerging standards

**Research date:** 2026-07-25
**Question:** How are today's coding agents invoked non-interactively, how can a wrapper tell they finished (and whether they succeeded), and are there emerging standards reeve's "agent profile" layer should align with?
**Audience:** reeve design. Specifically the two integration layers — (1) the agnostic embedded terminal, (2) user-configured agent profiles driving automation loops.

---

## 1. Executive summary

**The headline finding is negative, and it's the most useful thing in this document: there is no standard for "run a coding agent non-interactively and find out what happened."** There is a rough *convergence of shape* (`<binary> <subcommand-or-flag> "<prompt>"`, plus a JSON output flag), but every layer below that shape — flag names, JSON schemas, event vocabularies, exit-code semantics, session identity — is per-vendor and unstable.

Four concrete conclusions:

1. **Exit codes are the only universal completion signal, and they are widely broken.** Multiple agents exit `0` after failing. This is documented, reproducible, and reported against aider, GitHub Copilot CLI, and OpenCode (§4.1). Any design that assumes "exit 0 means the agent did the work" is building on sand.
2. **Structured output exists everywhere but agrees nowhere.** Claude Code, Codex, Gemini CLI, Cursor and OpenCode all offer JSON or JSONL output. Not one pair of them shares a schema, an event vocabulary, or a completion event name (§3, §4.2). Parsing them is exactly the "deep per-agent integration" reeve has ruled out — and the ecosystem evidence says that ruling is correct.
3. **The Agent Client Protocol (ACP) is the only real candidate standard for this layer, and it is not ready to be a dependency.** It has genuine multi-vendor adoption (25+ agents, JetBrains/Google/GitHub, an official registry as of Jan 2026), but its schema line still ships breaking changes and carries a parallel `2.0.0-alpha` track (§5.1). It is worth designing *toward*, not building *on*, today.
4. **MCP is the wrong layer entirely** and should not appear in reeve's agent-profile design (§5.2). The one thing that *has* genuinely standardised is `AGENTS.md` (§5.3) — and it is a file convention, not an invocation protocol, so it costs reeve nothing and buys real interop.

The recommendation (§7) is a deliberately small agent profile: **command template + a declared completion contract + a declared workspace contract**, with the completion contract defaulting to the honest answer ("exit code, and don't trust it") and everything richer being opt-in per profile.

---

## 2. Scope and method

Sources are official CLI documentation, vendor repos, and public issue trackers, fetched July 2026. Where a claim is about a *failure*, I have cited a specific issue with a number, status and date rather than a blog post, because vendor docs systematically overstate reliability of headless modes.

Agents covered: Claude Code, OpenAI Codex CLI, Google Gemini CLI, Cursor CLI (`cursor-agent`), OpenCode, aider, GitHub Copilot CLI. Protocols covered: ACP, MCP, `AGENTS.md`, with a note on A2A.

**Caveat on freshness:** every CLI here ships weekly or faster. Claude Code's own docs annotate behaviour changes at patch-version granularity (e.g. stdin caps at v2.1.128, exit-drain behaviour at v2.1.208 and v2.1.214). Treat every specific flag below as a snapshot, and treat *that volatility itself* as a design input — it is the single strongest argument for reeve staying agnostic.

---

## 3. The invocation landscape

### 3.1 Shape comparison

| Agent | Non-interactive entry | Structured output | Streaming | Resume | Documented exit codes |
|---|---|---|---|---|---|
| **Claude Code** | `claude -p "…"` | `--output-format json`, `--json-schema` | `--output-format stream-json` (+`--include-partial-messages`) | `--continue`, `--resume <id\|name>`, `--session-id`, `--fork-session` | `0` / `1`; `143` on SIGTERM |
| **Codex CLI** | `codex exec "…"` | `--output-schema`, `-o/--output-last-message` | `--json` (JSONL events) | `codex exec resume [--last\|<SESSION_ID>]` | non-zero on failure (unspecified set) |
| **Gemini CLI** | `gemini -p "…"` | `--output-format json` | JSONL variant documented | not prominent | `0`, `1`, `42` (input error), `53` (turn limit) |
| **Cursor CLI** | `cursor-agent -p "…"` | `--output-format json` | `--output-format stream-json`, `--stream-partial-output` | not documented on the headless page | **not documented** |
| **OpenCode** | `opencode run "…"` | `--format json` (raw events) | same flag | `--session <id>`, `--continue` | **not documented** |
| **aider** | `aider --message "…"` / `--message-file` | **none** | n/a | n/a | **not documented** |
| **Copilot CLI** | `-p` prompt mode | (n/a for this doc) | n/a | n/a | **`0` even on MCP startup failure** |

The shape convergence is real and it is what makes reeve's "command template" idea viable at all. The columns to the right are where it falls apart.

### 3.2 Claude Code — the most complete headless surface

`claude -p "<prompt>"` runs non-interactively and exits. It reads stdin, so `cat build-error.txt | claude -p 'explain'` works, with a 10 MB stdin cap since v2.1.128.

Output modes: `text` (default), `json` (result + session metadata, with `result`, `session_id`, `total_cost_usd` and a per-model cost breakdown), and `stream-json` (newline-delimited events, last line being a `result` message). `--json-schema` constrains the final answer and puts it in a `structured_output` field.

Notable for reeve:

- **`--bare`** skips auto-discovery of hooks, plugins, MCP servers and `CLAUDE.md`. Docs call it "the recommended mode for scripted and SDK calls" and say it will become the `-p` default. This is directly relevant: a reeve automation loop wants reproducibility, an embedded terminal wants the user's full environment. *Same binary, opposite flags.*
- **Session identity is first-class**: `session_id` comes back in JSON, and `claude -p "…" --resume "$session_id"` continues it. But session lookup is **scoped to the project directory and its git worktrees** — resume from elsewhere fails with `No conversation found with session ID`.
- **Transcripts on disk**: `~/.claude/projects/<project>/<session-id>.jsonl`, where `<project>` is the cwd with non-alphanumerics replaced by `-`. The docs explicitly warn: *"The entry format is internal to Claude Code and changes between versions, so scripts that parse these files directly can break on any release."* **reeve must not parse these.**
- **Signal handling is specified**: SIGTERM aborts the turn, kills the Bash process tree, runs `SessionEnd` hooks, and exits `143`. This is unusually good and reeve should not assume other agents do it.
- **Resume does not restore launch config**: `--mcp-config`, `--settings`, `--plugin-dir`, `--add-dir` must be passed again. So a profile's command template has to be *replayable*, not just fire-once.

Sources: [Run Claude Code programmatically](https://code.claude.com/docs/en/headless), [CLI reference](https://code.claude.com/docs/en/cli-reference), [Manage sessions](https://code.claude.com/docs/en/sessions).

### 3.3 Codex CLI — the cleanest event stream

`codex exec "<task>"` runs a single agent session to completion, streams progress to **stderr**, writes the final agent message to **stdout**, and exits. That stdout/stderr split is the cleanest separation of any agent here and is worth noting as a design ideal.

`--json` turns stdout into JSONL. Event types: `thread.started`, `turn.started`, `turn.completed`, `turn.failed`, `item.started`, `item.completed`, `error`. Items cover agent messages, reasoning, command executions, file changes, MCP tool calls, web searches, plan updates. Usage data (input/output/cached tokens) rides along.

Other flags: `-o/--output-last-message <path>` (final message to a file *and* stdout), `--output-schema <path>`, `--sandbox <workspace-write|danger-full-access>`, `--skip-git-repo-check`, `--ignore-user-config`, `--ignore-rules`, `--ephemeral` (don't persist rollout files). `--full-auto` is deprecated in favour of `--sandbox workspace-write`.

stdin: `cmd | codex exec "instruction"` appends piped context; `cat prompt.txt | codex exec -` uses stdin as the whole prompt.

Sessions persist as JSONL "rollout" files under `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, resumable via `codex exec resume --last` or `resume <SESSION_ID>`.

Exit codes: documented only as "non-zero on failure" with an instruction to check `$?` — no enumerated set. So `turn.failed` in the JSONL is *more* informative than the exit code, which is precisely the parsing burden reeve wants to avoid.

Sources: [Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode), [`docs/exec.md`](https://github.com/openai/codex/blob/main/docs/exec.md).

### 3.4 Gemini CLI — the only enumerated exit-code table

`gemini -p "<query>"`, with `--output-format text|json`. The JSON object has `response`, `stats` (token usage and latency), and an optional `error`. `--yolo`/`-y` auto-approves; `--approval-mode` (e.g. `auto_edit`) is finer-grained.

Gemini CLI is the **only** agent surveyed that publishes a real exit-code table:

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | General error or API failure |
| `42` | Input error (invalid prompt or arguments) |
| `53` | Turn limit exceeded |

That is genuinely useful — and it also demonstrates the fragmentation, because `42` and `53` are meaningful to nothing else on earth. A generic wrapper cannot interpret them without per-agent knowledge.

Sources: [Headless mode](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/headless.md).

### 3.5 Cursor CLI, OpenCode, aider — thinner and rougher

**Cursor CLI** (`cursor-agent`): `-p/--print`, `--output-format text|json|stream-json`, `--stream-partial-output`, `--force`/`--yolo`, `--api-key`. Its `stream-json` events are `system`(`init`), `assistant`, `tool_call`(`started`/`completed`), `result` — superficially similar to Claude Code's stream but *not* the same schema. **Exit codes are not documented at all** on the headless page. ([Using Headless CLI](https://cursor.com/docs/cli/headless))

**OpenCode**: `opencode run [message..]` with `--format default|json`, `--model provider/model`, `--agent`, `--session/-s <id>`, `--continue/-c`, plus `--file`, `--title`, `--attach`. Separately, `opencode serve` runs a headless HTTP server — an architecturally different and arguably better integration surface than CLI-scraping. ([CLI docs](https://opencode.ai/docs/cli/))

**aider**: `--message/-m` or `--message-file/-f` to send one message and exit; `--yes` to auto-confirm; `--commit`, `--dry-run`, `--auto-commits`. **No JSON output mode exists.** aider is the clearest case of an agent that is only usable via the agnostic-terminal layer and is essentially unusable as a loop-automation target beyond fire-and-hope. ([Scripting aider](https://aider.chat/docs/scripting.html))

---

## 4. How can a wrapper tell it finished — and succeeded?

### 4.1 Exit codes: universal, and broken

Exit codes are the only mechanism every agent has, because they come from the OS rather than the vendor. They are also demonstrably unreliable:

- **aider** [#3918](https://github.com/Aider-AI/aider/issues/3918) *(open, Apr 2025)* — aider prints "AI processing successful" and exits `0` after being **rate-limited and making no changes**. The reporter's script (`if aider --message "do foo bar" "$file"; then …`) silently records success. Still open.
- **GitHub Copilot CLI** [#3064](https://github.com/github/copilot-cli/issues/3064) *(open, May 2026)* — exits `0` when MCP servers fail to start, so the agent runs with an empty tool surface and reports success. A v1.0.22 regression went undetected in production; telemetry showed skipped detection jobs spike from 8–20/day to **2,569 across ~438 repositories**. This is the best available evidence that exit-code trust is a real, expensive production hazard.
- **OpenCode** [#28407](https://github.com/anomalyco/opencode/issues/28407) *(open, May 2026, v1.15.5, **Windows**)* — `opencode run` returns "Session not found", produces no output, and **exits `0`**. Daemon logs show `no_text (exit 0)`.

Meanwhile Gemini CLI has the opposite pathology: [#9281](https://github.com/google-gemini/gemini-cli/issues/9281) *(closed via PR #10671, opened Sep 2025)* — under `--output-format json` the CLI **exited on any tool error, even non-fatal ones** that the model could have self-corrected, while text mode recovered fine. Choosing structured output changed the failure semantics.

**Design consequence for reeve:** exit code is necessary but not sufficient. A profile must be able to declare a *stronger* success check — and the honest default is that reeve reports "the process ended with code N" and refuses to claim the work succeeded.

### 4.2 Structured output: five dialects, zero interop

Completion is signalled by, respectively: Claude Code's terminal `result` message; Codex's `turn.completed` / `turn.failed`; Gemini's single JSON object with an optional `error`; Cursor's `result` event; OpenCode's `step_finish`. Five names, five shapes, five sets of usage fields.

And the streams themselves are not reliably complete:

- **OpenCode** [#26855](https://github.com/anomalyco/opencode/issues/26855) *(closed via PR #31389, May 2026)* — `opencode run --format json` could exit **before emitting the final `step_finish` event**: the run loop observed `session.status=idle` and exited without draining the pending event. Downstream tooling lost all final token/cost accounting. One of the maintainer-suggested resolutions was literally *"document that JSON output is best-effort and may omit final accounting events."*
- **Claude Code** documents the same class of bug in its own changelog notes: before v2.1.208 piping a large response could truncate the final line and omit the `result` message; before v2.1.214 the exit-drain wait was ~2 s and could cut off the end of a large response. It now scales the drain wait up to 30 s.

So: *the final event of a stream is the thing most likely to be missing, and the final event is exactly the one a wrapper needs.* A completion detector that waits for a terminal JSON event, with no timeout and no process-exit fallback, will hang. This is not hypothetical — see also Cursor's [`-p` hangs indefinitely](https://forum.cursor.com/t/cursor-agent-p-print-headless-mode-hangs-indefinitely-and-never-returns/150246) *(CLI 2.4.21–2.4.22, silent for 10–15 s with zero stdout/stderr, reported fixed by Mar 2026)*.

**Design consequence:** completion detection must be `process exit OR terminal event, whichever first`, with a mandatory timeout. Never event-only.

### 4.3 Session artifacts: readable, and off-limits

Claude Code writes `~/.claude/projects/<project>/<session-id>.jsonl`; Codex writes `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Both are tempting sources of "what actually happened."

Both should be treated as off-limits for reeve. Anthropic's docs state the format is internal and changes between versions. Codex rollout files have their own known pathologies ([#24948](https://github.com/openai/codex/issues/24948): logs growing to 700 MB–2 GB from repeated compaction history and raw tool output). Reading these would be exactly the per-agent integration reeve has excluded, with the worst maintenance profile of any option.

The *paths*, however, are useful as an opaque artifact pointer — "the agent's transcript for this run is somewhere under here" — for a user to open, not for reeve to parse.

### 4.4 The signal reeve actually has that CI doesn't

reeve is local-first and workspace-isolated. That means it has a completion signal every CI-oriented wrapper lacks: **the git state of the isolated workspace.**

`git status --porcelain` / `git diff --stat` before and after a run answers "did the agent change anything?" without knowing anything about the agent. Combined with a user-supplied verification command (the project's test or build command), this gives a *tool-agnostic* success signal that is strictly more trustworthy than any vendor exit code. aider #3918 — success reported, nothing changed — is caught by this and by nothing else.

This deserves to be central to the design, not a fallback.

### 4.5 Windows-specific hazards

reeve is being developed on Windows 11, and headless agent modes are visibly less tested there:

- OpenCode #28407 (headless completely broken) is a Windows report.
- Claude Code docs note that **before v2.1.211, an unreadable stdin on Windows crashed the session or made it exit silently with no output**.
- Claude Code's own docs recommend escaping quotes in npm scripts specifically "to keep the script portable to Windows".

**Design consequence:** the command template must not be a naive string split on spaces, and reeve should prefer passing the prompt via stdin or a file over embedding it in a shell command line — quoting a multi-line prompt through `cmd.exe`/PowerShell is a reliable source of corruption.

---

## 5. Standards

### 5.1 Agent Client Protocol (ACP) — the real candidate, not yet a dependency

ACP is JSON-RPC 2.0 over stdio, created by Zed Industries (Aug 2025), explicitly modelled on LSP: *"Local agents run as sub-processes of the code editor, communicating via JSON-RPC over stdio."* It reuses MCP's JSON representations where possible but adds agentic-coding UX types like diffs.

Core methods: `initialize` (version + capability negotiation), `session/new` (cwd, MCP servers, extra dirs → session ID), `session/prompt`, `session/update` (notification: message chunks, tool calls, progress), `session/cancel`. A turn ends when `session/prompt` returns a **`StopReason`**: `end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`.

**That `StopReason` enum is the single most reeve-relevant artifact in this entire document.** It is the only vendor-neutral vocabulary anywhere for "the agent stopped, and here is why" — precisely the thing exit codes fail to express. Even if reeve never speaks ACP, **reeve should model its own internal run-outcome type on this enum**, plus a `failed` case, and map each agent's messy signals onto it.

Permissions are also modelled: agents send permission requests with options; clients return a `RequestPermissionOutcome` (accept / decline / cancel).

**Adoption is genuinely broad.** Clients: Zed, JetBrains, Neovim (CodeCompanion, avante.nvim), Emacs (agent-shell), marimo, Eclipse (prototype). Agents: 25+, including Claude Code, Codex CLI, Gemini CLI, OpenCode, GitHub Copilot CLI, Cursor, Goose, Cline, OpenHands, JetBrains Junie, Docker cagent, Qwen Code, Kimi CLI. An official **ACP Registry** launched Jan 28, 2026, so agents register once and appear in every ACP client. Zed shipped 1.0 in Apr 2026 with ACP as the headline feature.

**But it is not ready to be a hard dependency.** The releases track shows a schema line still in `1.x` with an active `2.0.0-alpha` pre-release track in parallel and changelog entries tagged `(unstable)` / `(unstable-v2)`. The protocol's own docs say *"Full support for remote agents is a work in progress."* There is no 1.0 stable declaration for the protocol itself.

Crucially for reeve: **ACP does not solve the invocation problem it looks like it solves.** How you *launch* an ACP agent is still a per-agent binary + args + env — Zed's own config is exactly that:

```json
{
  "agent_servers": {
    "my-agent": {
      "type": "custom",
      "command": "node",
      "args": ["~/projects/agent/index.js", "--acp"],
      "env": {}
    }
  }
}
```

That is *the same three fields* reeve's agent profile needs regardless of protocol. ACP standardises the conversation, not the launch. So aligning reeve's profile shape with `{command, args, env}` is free, correct today, and forward-compatible with ACP later.

Sources: [ACP overview](https://agentclientprotocol.com/overview/introduction), [protocol overview](https://agentclientprotocol.com/protocol/overview), [schema](https://agentclientprotocol.com/protocol/schema), [progress report](https://zed.dev/blog/acp-progress-report), [ACP Registry](https://zed.dev/blog/acp-registry), [Zed external agents](https://zed.dev/docs/ai/external-agents), [releases](https://github.com/agentclientprotocol/agent-client-protocol/releases).

### 5.2 MCP — the wrong layer

MCP standardises **agent → tool/data source**. It does not standardise **wrapper → agent**. The MCP maintainers are explicit about the scope boundary: MCP commits to exactly three primitives and is designed for the Agent↔Tool relationship, with Agent↔Agent explicitly delegated to A2A.

MCP is still relevant to reeve in exactly two indirect ways:

1. **As a failure source.** Copilot CLI #3064 shows MCP server startup failure silently producing a zero exit and an empty tool surface. If reeve's profiles pass `--mcp-config`, that failure mode is inherited.
2. **Possibly as reeve's own outward-facing surface** — reeve exposing its tickets/docs to whatever agent the user runs. That's a separate design question and not part of the agent profile.

The 2026-07-28 revision (largest since launch: stateless core, MCP Apps, Tasks extension) does not change the scope boundary.

Sources: [MCP 2026-07-28 release candidate](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/).

### 5.3 `AGENTS.md` — the one thing that actually converged

Not a protocol; a filename convention. A Markdown file at repo root carrying build commands, test commands, layout and conventions. It is read natively by Claude Code, Codex CLI, Cursor, aider, Gemini CLI, GitHub Copilot, Devin, Windsurf, Amazon Q and 30+ others, across 60,000+ repositories, with stewardship at the Linux Foundation.

**This is the highest interop-per-effort item available to reeve**, and it costs nothing: reeve generating/maintaining `AGENTS.md` in the isolated workspace steers *every* agent, with no per-agent code, no parsing, no protocol. It is the natural place for reeve to inject ticket context that survives across whatever agent the user picked.

The honest caveat: it is advisory. Nothing enforces that an agent reads or obeys it.

Sources: [AGENTS.md guide 2026](https://codersera.com/blog/agents-md-complete-guide-2026/), [AI agent standards 2026](https://blog.agentailor.com/posts/top-ai-agent-standards-2026).

### 5.4 A2A

Agent-to-Agent covers the Agent↔Agent relationship. Not relevant to reeve's single-user, single-agent-per-workspace model. Noted for completeness.

---

## 6. Interactive vs headless: two different products

The single most under-appreciated finding: **for most of these tools, headless is not "the interactive mode with the TUI turned off." It is a separate, less-tested code path with different failure semantics.**

Evidence:

- Cursor: interactive worked fine while `-p` hung indefinitely for weeks.
- Gemini CLI: `--output-format json` aborted on tool errors that text mode recovered from.
- OpenCode: `run` required a pre-existing session that it never created, breaking headless entirely while the TUI was fine.
- Claude Code: `--bare` exists *specifically* because `-p` otherwise inherits the interactive environment (hooks, plugins, MCP servers, `CLAUDE.md`) in ways that make scripted runs non-reproducible.

This validates reeve's two-layer split, but sharpens it: **the two layers are not the same agent invoked two ways. They are two different contracts.** A user's terminal profile and their automation profile for "the same agent" may legitimately need different commands, different flags, and different environments. The data model should permit that rather than assuming one command serves both.

It also implies reeve's terminal layer is the *robust* one — it inherits none of these bugs because it does nothing but allocate a PTY — while the automation layer is where all the risk lives. That asymmetry should be reflected in how each is presented to the user: the terminal is a stable capability; loops are best-effort and must surface raw output when they fail.

---

## 7. Recommendation: what an agent profile minimally needs

### 7.1 Design principles falling out of the evidence

1. **Never parse agent stdout for semantics.** Capture it, show it, log it, hash it if you like — do not branch on it. The five-dialect fragmentation plus documented dropped-final-event bugs make this a permanent maintenance tax for a permanently unreliable signal.
2. **Never trust exit `0` as "the work succeeded."** Treat it as "the process ended."
3. **Derive success from the workspace, not the agent.** Git diff + user-supplied verify command. This is agent-agnostic by construction and catches the exact failures exit codes miss.
4. **Every wait needs a timeout.** Hanging is a documented, shipped failure mode.
5. **Model outcomes on ACP's `StopReason`**, not on exit codes.
6. **Shape the launch fields as `{command, args, env, cwd}`** — matches Zed/ACP's registry shape, so an ACP transport can be added later without a schema migration.

### 7.2 Minimal agent profile

The profile is deliberately close to "a command", with three small additions that the evidence shows are non-negotiable.

```jsonc
{
  "id": "claude-code",
  "name": "Claude Code",

  // --- Launch (shared shape with ACP/Zed registry) ---
  "command": "claude",
  "args": ["-p", "{{prompt}}"],   // template; see placeholders below
  "env": {},                       // merged over inherited env
  "cwd": "{{workspace}}",          // reeve's isolated workspace

  // --- How the prompt is delivered (the Windows-quoting fix) ---
  "promptDelivery": "arg",         // "arg" | "stdin" | "file"

  // --- Which layer(s) this profile serves ---
  "modes": ["terminal", "automation"],

  // --- Completion contract (the honest part) ---
  "completion": {
    "signal": "exit",              // "exit" — the only universal one
    "successExitCodes": [0],
    "timeoutSeconds": 1800
  },

  // --- Verification: how reeve actually decides it worked ---
  "verify": {
    "requireWorkspaceChange": true,
    "command": null                // e.g. "npm test"; run in workspace
  }
}
```

**Placeholders** — minimum viable set: `{{prompt}}`, `{{workspace}}`, `{{ticket_id}}`, `{{ticket_title}}`. Substitution must be **argv-level, not shell-level**: substitute into an args array, never into a command string that gets re-parsed by a shell. On Windows this is the difference between working and silently corrupting multi-line prompts.

**Why each non-obvious field earns its place:**

| Field | Justified by |
|---|---|
| `promptDelivery` | Windows quoting hazards; Codex `-` stdin sentinel; Claude Code's 10 MB stdin cap and Windows stdin bugs. A long ticket body should never go through a shell command line. |
| `modes` | §6 — headless and interactive are different contracts. A user may want `claude` for the terminal and `claude -p --bare` for loops as two profiles, or one profile serving both. Don't force either. |
| `completion.timeoutSeconds` | Cursor `-p` hang; OpenCode `run` silent no-op. Mandatory, not optional. |
| `verify.requireWorkspaceChange` | aider #3918, Copilot #3064, OpenCode #28407 — all cases where the agent reported success having done nothing. This is the only check that catches all three. |
| `verify.command` | The user's own test/build command is a better success oracle than any vendor's JSON. |

**What is deliberately absent:**

- No output parser, no regex, no JSON path expressions. That is the per-agent integration reeve ruled out; §4.2 shows it would be both fragile and insufficient.
- No session-ID extraction. Requires parsing JSON, and Claude Code's session lookup is cwd-scoped in ways that interact badly with worktrees. **Defer.** If reeve later wants multi-turn loops, that is the moment to reconsider — and probably the moment to reconsider ACP.
- No per-agent capability flags (`supportsJson`, `supportsResume`, …). They would need maintaining against weekly releases and buy nothing while reeve doesn't parse output.

### 7.3 How this serves the two layers

**Agnostic terminal layer** uses `command`, `args` (with no `{{prompt}}`, or prompt omitted), `env`, `cwd`. reeve allocates a PTY in the isolated workspace and gets out of the way. It observes nothing but process lifetime. This layer inherits none of the headless bugs in §4 and should be presented as reliable.

**Loop-automation layer** uses the full profile. The run lifecycle is:

1. Snapshot workspace git state.
2. Write ticket context to `AGENTS.md` in the workspace (§5.3) — free, universal steering.
3. Launch with prompt delivered per `promptDelivery`; capture stdout/stderr verbatim to a run log.
4. Wait for process exit **or** timeout.
5. Compute outcome: exit code → workspace changed? → `verify.command` passes?
6. Map to a `StopReason`-shaped outcome: `completed` / `no_changes` / `verification_failed` / `timeout` / `cancelled` / `failed`.
7. Surface the raw log prominently on anything other than `completed`.

Note that step 5 uses the agent's own signal *only as the weakest of three inputs*. That inversion is the core recommendation.

### 7.4 Shipping defaults

Ship built-in profiles for `claude`, `codex`, `gemini`, `cursor-agent`, `opencode`, `aider` as **user-editable presets, not built-in integrations**. They are convenience seeds; the user owns them; reeve does not promise they stay correct. Given the observed release velocity, any promise of correctness would be broken within weeks. Say so in the UI.

---

## 8. Open questions and risks

1. **Multi-turn loops need session resumption, which needs output parsing.** The current recommendation punts. If reeve wants "agent runs, reeve reviews, reeve replies," that constraint must be revisited — and ACP becomes the better answer than screen-scraping `session_id` out of vendor JSON. Worth deciding *before* building loops, not after.
2. **When should reeve adopt ACP?** Suggested trigger: when the schema line declares stable with no parallel alpha track, *and* two of {Claude Code, Codex, Gemini CLI} ship first-party ACP (not community adapters). Until then, ACP shapes reeve's vocabulary (`StopReason`, `{command,args,env}`) without being a dependency.
3. **Permission/approval flags are per-agent and dangerous.** `--dangerously-skip-permissions`, `--yolo`, `--force`, `--sandbox danger-full-access` all live in `args`, which means reeve's UI will render "run this agent with all safety off" as an ordinary text field. Workspace isolation is the mitigation, but it should be a conscious, documented one.
4. **`verify.requireWorkspaceChange` has false positives** — a legitimate "no change needed" answer looks like failure. Needs to be per-loop overridable, not a global truth.
5. **Cost/token accounting is unobtainable agnostically.** Every agent reports it differently and §4.2 shows the final accounting event is the one most likely to be dropped. reeve should not promise cost tracking.
6. **Windows is under-tested upstream.** Since that is the development platform, headless profiles should be smoke-tested per agent on Windows before being shipped as presets.

---

## 9. Sources

**Claude Code**
- [Run Claude Code programmatically (headless)](https://code.claude.com/docs/en/headless)
- [CLI reference](https://code.claude.com/docs/en/cli-reference)
- [Manage sessions](https://code.claude.com/docs/en/sessions)

**OpenAI Codex CLI**
- [Non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- [`openai/codex` docs/exec.md](https://github.com/openai/codex/blob/main/docs/exec.md)
- [Issue #24948 — session logs grow to 700MB–2GB](https://github.com/openai/codex/issues/24948)

**Google Gemini CLI**
- [Headless mode docs](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/headless.md)
- [Issue #9281 — headless JSON exits on non-fatal tool errors](https://github.com/google-gemini/gemini-cli/issues/9281)

**Cursor CLI**
- [Using Headless CLI](https://cursor.com/docs/cli/headless)
- [Forum — `cursor-agent -p` hangs indefinitely](https://forum.cursor.com/t/cursor-agent-p-print-headless-mode-hangs-indefinitely-and-never-returns/150246)

**OpenCode**
- [CLI docs](https://opencode.ai/docs/cli/)
- [Issue #26855 — `run --format json` can exit before final `step_finish`](https://github.com/anomalyco/opencode/issues/26855)
- [Issue #28407 — `run` returns "Session not found" in headless mode (Windows)](https://github.com/anomalyco/opencode/issues/28407)

**aider**
- [Scripting aider](https://aider.chat/docs/scripting.html)
- [Issue #3918 — should return error when unable to realize changes](https://github.com/Aider-AI/aider/issues/3918)

**GitHub Copilot CLI**
- [Issue #3064 — stricter exit code when MCP servers fail to start](https://github.com/github/copilot-cli/issues/3064)

**Agent Client Protocol**
- [Introduction](https://agentclientprotocol.com/overview/introduction) · [Protocol overview](https://agentclientprotocol.com/protocol/overview) · [Schema](https://agentclientprotocol.com/protocol/schema)
- [How the Community is Driving ACP Forward](https://zed.dev/blog/acp-progress-report) · [The ACP Registry is Live](https://zed.dev/blog/acp-registry)
- [Zed — External Agents](https://zed.dev/docs/ai/external-agents) · [Releases](https://github.com/agentclientprotocol/agent-client-protocol/releases)

**MCP / AGENTS.md**
- [MCP 2026-07-28 specification release candidate](https://blog.modelcontextprotocol.io/posts/2026-07-28-release-candidate/)
- [AGENTS.md complete guide 2026](https://codersera.com/blog/agents-md-complete-guide-2026/) · [Top AI agent standards 2026](https://blog.agentailor.com/posts/top-ai-agent-standards-2026)
