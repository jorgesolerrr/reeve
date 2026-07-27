# Emdash agents & architecture — code deep dive

> Companion note to [reference-implementations.md](./reference-implementations.md), feeding the **skeleton** and **runs** LLD tickets (monorepo layout, agent CLI invocation, PTY, hooks). Produced by code-reading `github.com/generalaction/emdash` (local clone). All paths are repo-relative. Overlaps with the other `emdash-*.md` notes are intentional; this note is the only one covering the agent-plugin system and monorepo layout.

## 1. Monorepo layout & architecture

**Workspace**: pnpm + nx monorepo (`nx.json`, `pnpm-workspace.yaml`, root `package.json` — Node >= 24, `type: module`). Layout:

- `apps/emdash-desktop` — the Electron app (electron-vite, electron-builder, oxlint/oxfmt, tsgo for typecheck).
- `apps/workspace-server` — a per-user Node daemon for remote (SSH) workspaces; exposes a "wire" contract over a Unix domain socket or stdio; desktop connects via SSH stream-local forwarding (`apps/workspace-server/docs/daemon.md`, file-lock + pidfile daemon lifecycle in `apps/workspace-server/src/daemon/lock.ts`).
- `packages/core` — domain logic shared main-process side: agents/plugin capability system, ACP client, git, files, pty helpers, host-dependency probing, exec contexts.
- `packages/plugins` — the big per-provider plugin registry: `src/agents/impl/*` (36 coding-agent CLIs), `src/integrations/impl/*` and `src/issues/impl/*` (12 trackers).
- `packages/wire` — "transport-agnostic live model and mutation primitives": typed contracts, LiveState/LiveLog/EventStream/LiveJob, transports (memory, MessagePort, electron, stream), utilityProcess supervision (`packages/wire/docs/README.md`, `packages/wire/src/process/utility-process-host.ts`).
- `packages/chat-ui` — notably **SolidJS**, not React: "Solid-based chat transcript renderer with pretext layout engine" (`packages/chat-ui/package.json`), embedded inside the React renderer (`apps/emdash-desktop/src/renderer/lib/chat/chat-transcript.tsx`).
- `packages/runtime` (acp-agents / tui-agents / agent-config runtimes), `packages/shared` (Result type, emitter, lifecycle), `packages/ui` (React components + theme).

**Electron split** (`apps/emdash-desktop/src/`): `main/` (424 non-test TS files, organized as `main/core/<feature>/{controller.ts,service.ts,...}`), `preload/index.ts` (tiny: generic `invoke`, generic event bridge, `getPathForFile`, `requestWirePort`), `renderer/` (React), `shared/` (contracts, zod schemas, pure logic used by both sides).

**IPC**: three mechanisms, all typed:

1. **RPC** — `apps/emdash-desktop/src/shared/lib/ipc/rpc.ts`: `createRPCRouter` of nested plain-object controllers; `registerRPCRouter` walks the tree and registers each leaf at its dot-joined path (`"workspace.gitWorktree.commit"`) with `ipcMain.handle`. The renderer client is a recursive `Proxy` that accumulates property accesses into the channel string (`createRPCClient`). Type safety comes purely from `export type RpcRouter = typeof rpcRouter` (`src/main/rpc.ts` — ~35 namespaces: `tasks`, `pty`, `issues`, `integrations`, `github`, `workspace.{gitWorktree,files,fileTree,editor}`, etc.). Dev builds wrap ipcMain with `withRpcLogging`.
2. **Events** — `shared/lib/ipc/events.ts`: `defineEvent<TData>('task:created')` tokens; main-side adapter broadcasts to all BrowserWindows (`main/lib/events.ts`), with optional per-topic suffix (`eventName.topic`, used for per-session PTY data).
3. **Wire** — MessagePort-based streaming for high-volume/live data (ACP chat, live models); preload exposes `requestWirePort`, and the ACP utility process's wire is exposed straight to renderer windows via `MessageChannelMain` (`main/core/acp/runtime-process/host.ts`), bypassing ipcMain round-trips.

**SQLite**: `better-sqlite3` + **Drizzle ORM** (`main/db/client.ts`; WAL, `busy_timeout=5000`). Migrations are drizzle-kit generated SQL files in `apps/emdash-desktop/drizzle/` (0000–0019), **bundled into the binary via Vite `import.meta.glob(..., ?raw)`** and replayed with a hand-rolled runner keyed off `drizzle/meta/_journal.json` (`main/db/initialize.ts`). An FTS5 `search_index` virtual table is managed *outside* Drizzle with a version key in a `kv` table (drizzle can't emit FTS5 DDL). Tables (`main/db/schema.ts`): `projects`, `project_remotes`, `project_settings`, `app_settings`, `tasks`, `workspaces`, `conversations`, `terminals`, `messages`, `pull_requests` (+users/labels/assignees/checks), `automations`, `automation_runs`, `editor_buffers`, `kv`, `provider_accounts`, `app_secrets`, `ssh_connections`. JSON columns are wrapped in `versionedJsonColumn(zodSchema)` (`main/db/versioned-column.ts` + `shared/lib/versioned-schema`) — zod-validated, versioned blobs (e.g. `tasks.linked_issue`, `workspaces.config`). There is a whole `main/db/legacy-port/` subsystem for importing the previous app generation's DB.

## 2. Agent CLI detection & invocation

**Provider abstraction**: each agent is a *plugin* = `definePlugin(metadata, capabilities, assets)` + `registerPluginBehavior(plugin, behavior)` (`packages/core/src/agents/plugins/`). Capabilities are zod-described discriminated unions per axis: `acp`, `auth`, `models`, `hooks`, `hostDependency`, `mcp`, `prompt`, `sessions`, `trust`, `autoApprove`, `effort` (`packages/core/src/agents/plugins/capabilities/*`). All 36 providers are registered in `packages/plugins/src/agents/registry.ts` (codex, claude, grok, devin, qwen, droid, cursor, copilot, amp, opencode, goose, cline, …, with per-provider dirs under `packages/plugins/src/agents/impl/`).

**Detection**: each plugin declares `hostDependency.binaryNames` plus per-OS install/update commands. Discovery is `where <cmd>` on Windows / `which -a <cmd>` on POSIX with 5 s timeouts, then a version probe (`<bin> --version`, 10 s) and `realpath` for install-method classification — all in `packages/core/src/host-dependencies/runtime/probe.ts` and `host-dependency-manager.ts`. Multiple installations are enumerated and the user can pin one; the persisted selection lives in `hostDependencyStore` keyed by host (`local` or SSH connection id). Actual spawn-time resolution is `resolveAgentExecutable` (`main/core/conversations/impl/resolve-agent-executable.ts`) with explicit fallback order: pinned realpath → saved path → raw CLI command → cached probe path → live `which` → bare binary name. Because GUI-launched Electron has a stunted PATH, startup runs `$SHELL -ilc 'env'` (5 s timeout) and merges the captured PATH/SSH_AUTH_SOCK into `process.env` (`main/utils/userEnv.ts`) — with an AppImage guard against a fork-bomb regression (#1679) and `DISABLE_AUTO_UPDATE`/tmux guards. On Windows it instead just prepends `%APPDATA%\npm`.

**Two run modes per conversation** (`conversations.config.type: 'pty' | 'acp'`):

*PTY/TUI mode* (`main/core/conversations/impl/local-conversation.ts`): builds the argv via the plugin's `prompt.buildCommand`, which for most agents is `buildStandardCommand` (`packages/core/src/agents/plugins/helpers/standard-command.ts`) — a declarative flag spec. For Claude: `autoApproveFlag: '--dangerously-skip-permissions'`, `initialPromptFlag: ''` (positional prompt), `resumeFlag: '--resume'`, `sessionIdFlag: '--session-id'`, `modelFlag: '--model'`. Emdash generates its own conversation UUID and passes it as `--session-id` on fresh runs so it can `--resume <uuid>` later; providers that mint their own ids use `providerSessionId` + `sessionIdOnResumeOnly`. Prompt delivery kinds (`capabilities/prompt.ts`): `argv` (flag or positional), `stdin-pipe` (wrapped as `bash -c "printf '%s\n' <prompt> | <agent ...>"`), `keystroke` (6 agents — the prompt is *typed into the TUI*: wait for first output then 800 ms of quiet, max 15 s, then write payload + `\r`, multiline wrapped in bracketed-paste `\x1b[200~...\x1b[201~` — `impl/keystroke-injection.ts`, `shared/prompt-injection.ts`), or `none`. Prompts > 16 384 chars are **spilled to a temp markdown file** with a pointer message, because a full Linear issue once caused ENAMETOOLONG in Kilo Code (`spill-large-prompt.ts`, ENG-1546). The process is spawned with **node-pty** (`main/core/pty/local-pty.ts`, `xterm-256color`), through a platform resolver (`pty-spawn-platform.ts`, 441 lines) that on POSIX wraps everything in `$SHELL -c '<quoted line>'` (optionally inside tmux), and on Windows does its own PATH+PATHEXT resolution and re-routes `.cmd/.bat` through `cmd /d /s /c` (with the documented outer-double-quote quirk workaround), `.ps1` through `powershell -ExecutionPolicy Bypass -File`, and supports WSL profiles. A `ConversationSessionSupervisor` dedupes concurrent spawns and auto-respawns exited-but-desired sessions after 500 ms.

*ACP mode*: agents with `acp: supported` run through the **Agent Client Protocol** (`@agentclientprotocol/sdk`). For claude/codex, the spawn is not the CLI itself but an adapter: `command: process.execPath, args: [require.resolve('@agentclientprotocol/claude-agent-acp/dist/index.js')], env: { ELECTRON_RUN_AS_NODE: '1', CLAUDE_CODE_EXECUTABLE: <resolved cli path> }` (`packages/plugins/src/agents/impl/claude/index.ts`), spawned with plain `child_process.spawn` stdio-pipes (`main/core/acp/transport/local-acp-process-host.ts`). The whole ACP runtime lives in an Electron **utilityProcess** worker supervised by wire (`main/core/acp/runtime-process/host.ts`).

**Completion / needs-input detection** — three signals, no output-scraping heuristics for TUIs:

1. **Agent hooks**: emdash starts a localhost HTTP server on a random port with a UUID token (`main/core/agent-hooks/hook-server.ts`); for agents supporting lifecycle hooks it writes hook config into the worktree (Claude: `.claude/settings.local.json` with `UserPromptSubmit`→start, `Notification`, `Stop` posting to `/hook` with `x-emdash-token`/`x-emdash-pty-id` headers). Claude's Notification events carry no type, so it regex-classifies the message: `/permission|approval/i` → `permission_prompt`, else `idle_prompt` (`packages/plugins/src/agents/impl/claude/hooks.ts`).
2. **ACP session summaries**: `deriveAcpAgentStatusActions` (`main/core/acp/agent-status-transition.ts`) diffs `isGenerating`/`queuedPromptCount`/`pendingPermissionCount`/`lastStopReason` snapshots into start/stop/permission events.
3. **Process exit**: PTY `onExit` emits `agentSessionExitedChannel`.

Status lands in `conversations.agentStatus` (+`agentStatusSeen`) and drives sidebar badges/notifications.

## 3. Worktree lifecycle (summary — full detail in [emdash-worktree-lifecycle.md](./emdash-worktree-lifecycle.md))

**Placement**: default pool is `~/emdash/worktrees/<projectName>/<branchName>`; user-overridable; SSH projects use `<projectPath>/.emdash/worktrees` on the remote.

**Branch naming**: task names auto-generated from the title via `nbranch`'s `generateBranchName` or random `human-id`; final branch = optional `branchPrefix/`, optional 5-char base-36 random suffix; **Linear's suggested `branchName` wins verbatim**.

**Create** (`main/core/projects/worktrees/worktree-service.ts`, all git ops serialized through a promise-chain `enqueueGitOp` queue): reuse if `git worktree list --porcelain` shows the branch checked out anywhere valid; else validate/remove a stale dir at the target (with a **containment check — refuses to delete anything whose realpath is outside the pool**), then: verify `refs/heads/<branch>`; if absent resolve the base, `git branch --no-track <branch> <sourceRef>`, record base as git config `branch.<name>.base`, then `git worktree prune` + `git worktree add`. Afterwards, untracked files matching user "preserve patterns" (`.env` etc.) are copied from repo root into the worktree, with pattern-safety and realpath-containment checks. Constructor fires a fire-and-forget `git worktree prune` on every project open.

**Delete**: teardown live sessions → delete workspace row if unused → delete task row → only if no sibling task shares the workspace: containment-checked recursive rm + `git worktree prune`; fallback path **explicitly refuses when the workspace path resolves to the project root**. Branch deletion is opt-in (default `false`) and skipped when branch == base. Archive keeps the worktree.

**Windows handling**: no EBUSY retry loop; no MAX_PATH mitigation. The heavy Windows work went into PTY spawning and env instead (`main/utils/windows-env.ts` case-insensitive env-key lookup; PATHEXT resolution in `pty-spawn-platform.ts`). PTY kill: POSIX uses a `PosixPtyTerminator` escalation, win32 just `proc.kill()`.

**SQLite vs git**: DB stores intent + identity; live git state (status, diffs, validity) is always derived by shelling out. Reuse logic trusts `git worktree list --porcelain` over the DB path.

## 4. Tracker imports (summary — full detail in [emdash-tracker-integrations.md](./emdash-tracker-integrations.md))

Canonical plugin layer: `packages/plugins/src/integrations/` (auth) + `packages/plugins/src/issues/` (list/search/get); 12 providers. Issue plugin returns canonical `IssueData` + `IssueDetail.context` (comments/activity pre-rendered as a markdown context string for agent prompts). On task creation this is snapshotted into `tasks.linked_issue` as a flat versioned JSON blob; refreshed only by explicit re-link (no background issue sync — unlike PRs, which have a `pr-sync-scheduler`). The create-task modal composes the initial prompt as `issueContext + "\n\n" + userPrompt`; the large-prompt spill kicks in past 16 KB.

## 5. React UI (summary — full detail in [emdash-renderer-ui.md](./emdash-renderer-ui.md))

MobX class stores for domain state; TanStack Query for request/response RPC data; wire live models (MessagePort) for continuously-updating state. Task list is a virtualized sidebar, not a kanban grid. Terminal: xterm 6, main-side 16 ms batch flush + 64 KB per-session ring buffer, renderer terminal instance owned outside React in an off-screen host. Diff viewer: Monaco computes the diff from two whole file contents fetched over RPC (`git show :<path>` etc.).

## Cautionary findings

- Claude "needs input" detection is a **regex over notification message text** (`/permission|approval/i`) — brittle across CLI versions (`packages/plugins/src/agents/impl/claude/hooks.ts`).
- Keystroke prompt injection is timing-heuristic (800 ms quiet, 15 s max) and can lose the prompt if the TUI exits early — they log a warning for exactly that case.
- OS argv limits actually bit them (ENG-1546) — hence the 16 KB spill-to-file rule.
- cmd.exe `/S /C` quoting quirk needed an explicit documented workaround (`pty-spawn-platform.ts` `wrapCmdExeCommandLine`).
- Login-shell env capture fork-bombed AppImage builds until guarded (#1679, `userEnv.ts`).
- Schema carries lots of `@deprecated` columns — migration debt from moving task→workspace state.
- No Windows EBUSY/retry handling on worktree removal; a locked file (open terminal/agent still holding cwd) surfaces as a raw `removal-failed`.
- Secrets are safeStorage-encrypted but stored *in the SQLite file*, and the gh import copies the plaintext token out of `gh auth status`.

## Patterns worth adopting (for a Rust/Tauri app in this domain)

- **Declarative per-agent capability manifests** (binary names, install commands per OS, prompt-delivery kind, resume/session flags, auto-approve flag, hook events) + one generic `buildStandardCommand` interpreter — adding an agent is mostly data (`packages/plugins/src/agents/*`).
- Executable resolution as an explicit fallback chain with a user-pinnable selection per host, plus `which -a`/`where` enumeration of *all* installs (`probe.ts`, `resolve-agent-executable.ts`).
- **Localhost hook server with per-boot token** for agent lifecycle events — far more reliable than parsing TUI output; ACP for structured chat where supported, PTY as the universal fallback.
- Worktree pool outside the repo, branch-name = directory-name, `git worktree list --porcelain` as source of truth, `git worktree prune` sprinkled ambient, and **realpath-containment guards before any recursive delete** plus an explicit project-root refusal.
- Recording the base ref as `git config branch.<name>.base` — survives app reinstall, lives with the repo.
- Serializing all mutating git ops per repo through a queue (`enqueueGitOp`).
- Snapshotting the full issue context as markdown at link time and feeding it into the first prompt; spilling oversized prompts to a temp file with a pointer message.
- PTY output: batched flush (~16 ms) + per-session ring buffer for late subscribers; renderer terminal instance decoupled from view lifecycle.
- Versioned-JSON columns validated by schema on read (serde + versioned enums in Rust) and migrations bundled into the binary.
- Preserve-patterns copy of untracked config files (`.env`) into new worktrees, with pattern-safety checks.
- Login-shell PATH capture at startup (needed for any GUI app spawning user CLIs on macOS/Linux).

## Patterns to avoid

- Stringly-typed RPC held together by a `Proxy` and `typeof` — Tauri's command macros + generated TS types give real codegen; don't replicate the dot-path proxy.
- Regex-classifying agent notifications; prefer structured protocols and treat text parsing as last resort.
- Keystroke injection into TUIs as a first-class delivery mode — inherently racy; keep it only as a clearly-labeled fallback.
- Storing encrypted secrets inside the app DB; use the OS keychain directly.
- Deprecated-column sprawl: the tasks/workspaces split was retrofitted and the schema shows it — model "task vs workspace vs conversation" separately from day one.
- One-shot recursive delete on Windows without EBUSY/retry/backoff — in this domain (agents holding files open in worktrees) locked files are routine; plan retries and "worktree busy" UX.
- Three overlapping state systems in the renderer — powerful but a lot of conceptual surface; pick one reactive backbone.
- A 441-line hand-rolled Windows shell/PATHEXT resolver — in Rust, use maintained crates (`which`, `portable-pty`) instead of reimplementing cmd.exe semantics.
