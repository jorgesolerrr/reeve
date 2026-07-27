# Emdash worktree lifecycle — code deep dive

> Companion note to [reference-implementations.md](./reference-implementations.md), feeding the **workspaces** LLD ticket (git module, WorkspaceProvider, worktree lifecycle & cleanup). Produced by code-reading `github.com/generalaction/emdash` (local clone). All paths are repo-relative.

## Overview: two parallel worktree stacks

The repo currently contains **two independent worktree lifecycles**:

| | **Legacy (live, used by the app)** | **New (built, exported, not wired)** |
|---|---|---|
| Entry | `apps/emdash-desktop/src/main/core/projects/worktrees/worktree-service.ts` | `packages/core/src/workspace-lifecycle/` + `workspace-coordinator/` |
| Spec compile | `apps/emdash-desktop/src/shared/core/workspaces/workspace-setup-spec.ts` (`compileSetupSpec`) | `packages/core/src/workspace-lifecycle/plan/planner.ts` (`compileBootstrapPlan`) |
| Executor | `apps/emdash-desktop/src/main/core/workspaces/local-workspace-setup-executor.ts` | `packages/core/src/workspace-lifecycle/runner/runner.ts` |
| Steps | `apps/emdash-desktop/src/main/core/workspaces/setup-steps/*` | `packages/core/src/workspace-lifecycle/steps/impl/*` |

`grep` for `@emdash/core/workspace-lifecycle` / `@emdash/core/workspace-coordinator` across `apps/` and `packages/` returns **zero** importers — the new stack is exported in `packages/core/package.json:100-110` and covered by its own tests only. `agents/workflows/worktrees.md:3-5` still names `worktree-service.ts` as the main file.

Also note: **`packages/core/src/git/git-worktree.ts` does not create or delete worktrees.** It is a live status/HEAD model over an *already existing* worktree (`GitWorktree` class, `git-worktree.ts:78-139`), wired by `GitRuntime.openWorktree` (`packages/core/src/git/git-runtime.ts:175-202`). Its only git mutations are `add`/`reset`/`checkout`/`clean`/`commit`/`push`/`pull` (lines 353-460).

## 1. Worktree CREATE

### Exact commands

**New stack** — `packages/core/src/workspace-lifecycle/steps/impl/add-worktree.ts:20-25`:

```ts
await runGit(['worktree', 'prune'], { cwd: ctx.repoPath, signal: ctx.signal });
await mkdir(path.dirname(args.path), { recursive: true });
const result = await runGit(['worktree', 'add', args.path, args.branchName], {
  cwd: ctx.repoPath, signal: ctx.signal,
});
```

No `--detach`, no `-b`, no `--force`, no `--no-checkout`. The branch must already exist (created by a preceding `create-local-branch` step).

**Legacy stack** — `worktree-service.ts:311-312` and `:413-414`, identical shape:

```ts
await this.ctx.exec('git', ['worktree', 'prune']).catch(() => {});
await this.ctx.exec('git', ['worktree', 'add', targetPath, branchName]);
```

Branch creation (new): `create-local-branch.ts:38-47`

```ts
const gitArgs = ['branch'];
if (args.noTrack) gitArgs.push('--no-track');
gitArgs.push(args.branchName, args.fromRef);
// on success:
await runGit(['config', `branch.${args.branchName}.emdash-created`, 'true'], …)
```

Legacy equivalent: `worktree-service.ts:303` → `git branch --no-track <branch> <sourceRef>`, or `:405-410` → `git branch --track <branch> <remote>/<branch>` when adopting an existing remote branch.

`runGit` itself (`packages/core/src/workspace-lifecycle/steps/run-git.ts:20-56`) uses `execFile('git', args, {…})` with `maxBuffer: 10MB`, `GIT_TERMINAL_PROMPT: '0'` and a merged `GIT_SSH_COMMAND` containing `-oBatchMode=yes`.

### Branch naming — `human-id` and `nbranch` are both used

`apps/emdash-desktop/package.json:76,79` declares `"human-id": "^4.1.2"` and `"nbranch": "^0.1.1"`. Used only in `apps/emdash-desktop/src/main/core/tasks/name-generation/generateTaskName.ts`:

```ts
export function generateRandom(): string {
  return sanitize(humanId({ separator: '-', capitalize: false }));
}
function generateFromInput(title, description) {
  const raw = generateBranchName(input, { addRandomSuffix: false, separator: '-', maxLength: 64 });
  return sanitize(raw);   // /[^a-z0-9-]/g → '-', collapse, trim, slice(0,64)
}
```

These produce the **task name**, not the branch directly. The branch is assembled in `apps/emdash-desktop/src/shared/resolveTaskBranchName.ts:19-31`:

```ts
const branch = shouldAppendSuffix ? `${rawBranch}-${suffix}` : rawBranch;
return branchPrefix ? `${branchPrefix}/${branch}` : branch;
```

- `branchPrefix` default `'emdash'`, `appendRandomBranchSuffix` default `true` — `apps/emdash-desktop/src/main/core/settings/settings-registry.ts:19-20`.
- The random suffix is **not** from `human-id`; it's `Math.random().toString(36).slice(2, 7)` computed once per modal session (`renderer/features/tasks/create-task-modal/use-branch-name.ts:27`).
- A Linear-linked issue's `branchName` **overrides everything**, prefix included (`resolveTaskBranchName.ts:20-25`).

Typical result: `emdash/fix-the-widget-a3k9d`.

### Base branch resolution

`compileSetupSpec` (`workspace-setup-spec.ts:57-88`) and `compileBootstrapPlan` (`planner.ts:30-53`) agree:

- `fromBranch.type === 'remote'` → `git fetch <remote>`, then branch from `` `${remote}/${branch}` `` with `--no-track`, then record `branch.<name>.base = <remote>/<branch>`.
- `fromBranch.type === 'local'` → branch from the local name, `--no-track`, base = the local name.
- PR intents fetch `refs/pull/<n>/head:refs/heads/<headBranch>` from `baseRemote` (or add a fork remote named after the owner) and optionally stack a `taskBranch` on top.

`baseRemote`/`pushRemote` default to `'origin'` (`db-project-settings-provider.ts:273-281`). Base is persisted as git config `branch.<name>.base` — `set-branch-base.ts` (core) and `worktree-service.ts:200-220` (legacy, non-overwriting, warns on failure).

### On-disk path construction

Pool root:

```
getDefaultLocalWorktreeDirectory() = path.join(homedir(), 'emdash', 'worktrees')   // worktree-defaults.ts:8-10
getDefaultSshWorktreeDirectory(p)  = path.posix.join(p, '.emdash', 'worktrees')    // :12-14
```

User-overridable via project setting `worktreeDirectory`; must be natively absolute after `~` expansion (`projects/settings/worktree-directory.ts:29-66`) and is `mkdir -p`+`realpath`'d (`:68-78`).

Per-project pool (local): `create-project-provider.ts:66-72`

```ts
path.join(directory, safePathSegment(project.name, project.id))
```

`safePathSegment` (`src/shared/path-name.ts:14-27`) strips `<>:"/\|?*` + control chars and rejects Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`). SSH pool skips this: `path.join(worktreeDirectory, project.name)` (`create-project-provider.ts:137`).

Final worktree path — **and here the two stacks disagree**:

- New: `planner.ts:135-141` → `path.join(pool, branchName.replace(/[^a-zA-Z0-9._-]/g, '-'))` ⇒ `…/worktrees/myproj/emdash-fix-widget-a3k9d`
- Legacy: `worktree-service.ts:224` and `:277` → `this.files.path.join(worktreePoolPath, branchName)` with **no sanitisation** ⇒ `…/worktrees/myproj/emdash/fix-widget-a3k9d` (nested, because of the `/` in the prefix)

So a migration to the new planner will orphan every existing worktree directory.

## 2. DELETE / cleanup

### New stack — `steps/impl/remove-worktree.ts` (entire file, 15 lines)

```ts
export const removeWorktreeImpl = implement(removeWorktreeStep, async (args, ctx) => {
  const result = await runGit(['worktree', 'remove', '--force', args.path], {
    cwd: ctx.repoPath, signal: ctx.signal,
  });
  if (result.success) return stepOk();
  await rm(args.path, { recursive: true, force: true }).catch(() => {});
  return stepOk();
});
```

Notes: `--force` is unconditional (discards dirty worktrees silently). On failure it falls back to a raw `rm -rf` of whatever path it was handed, **with no containment check at all**, then swallows the error and returns success. It never calls `git worktree prune` afterwards, so the `.git/worktrees/<id>` administrative entry is leaked when the fallback path is taken.

Branch deletion — `steps/impl/delete-branch.ts:6-17`:

```ts
const exists = await runGit(['rev-parse','--verify',`refs/heads/${args.branchName}`], …);
if (!exists.success) return stepOk();
await runGit(['branch', '-D', args.branchName], …);   // force delete, result ignored
```

Gated by `observed.branchCreatedByEmdash` in `plan/teardown.ts:58-63`, which reads git config `branch.<name>.emdash-created` (`probe.ts:124-137`).

**Bug:** the *legacy* `create-local-branch` step (`setup-steps/create-local-branch.ts`) never writes that marker — only the core version does (`impl/create-local-branch.ts:44`). Every branch created by the currently-shipping code path is therefore invisible to `branchCreatedByEmdash`, so the new teardown planner would never delete it.

`removeWorktreeStep` and `deleteBranchStep` are both `fatal: false` (`steps/catalog.ts:96-121`), so failures downgrade to warnings and the plan continues.

### Legacy stack

`WorktreeService.removeWorktree` (`worktree-service.ts:435-439`) never calls `git worktree remove` at all:

```ts
async removeWorktree(worktreePath: string): Promise<void> {
  await this.removePathForReuse(worktreePath).finally(() => {
    this.ctx.exec('git', ['worktree', 'prune']).catch(() => {});
  });
}
```

`removePathForReuse` (`:91-112`) is the safety valve:

```ts
const contained = await isRealPathContained(this.files, poolPath, targetPath, { candidateMustExist: true });
if (!contained.success || !contained.data) {
  throw new Error(`Refusing to remove worktree path outside pool: "${targetPath}"`);
}
```

then `rm -r`, then re-`exists()` and throws `path still exists` if the removal was a no-op. See §3 for why this guard is broken on Windows.

Task-delete fallback — `tasks/operations/task-lifecycle-utils.ts:94-134`, `removeOwnedLocalWorktreeDirectory`:

- refuses when `workspacePath === projectRootPath` and kind is `worktree` → `{ type: 'project-root-refused' }` (`:102-111`)
- requires `isWorktreeWorkspace(workspace)` (`:113`)
- `localFileSystem.remove(recursive: true)`, tolerates ENOENT, re-checks existence, then `pruneGitWorktrees()` → `git worktree prune` with `timeoutMs: 5_000` (`:66-80`)

Orchestration in `deleteTask.ts:69-112`: teardown sessions → `removeWorktreeIfUnused` (git path) → if that returned false, `removeOwnedLocalWorktreeDirectoryIfUnused` (fs path) → only then, if `deleteBranch` and the worktree actually went away and `config.git.kind === 'create-branch'` and the branch ≠ its own base, `project.gitRepository.deleteBranch(branch)` → `git branch -D -- <branch>` (`packages/core/src/git/git-repository.ts:269-279`). Default is `deleteBranch = false` (`deleteTask.ts:24`).

`archiveTask.ts` deliberately does **not** remove the worktree — it tears down with mode `'archive'` only (`:21-27`).

### Retries

Only one retry mechanism exists, and it's in the unwired stack: `runner/runner.ts:31` `DEFAULT_RETRY_DELAYS_MS = [1_000, 4_000]` (3 attempts max), applied only when `result.class === 'transient'` (`:114-118`). "Transient" is a *network* regex — `helpers.ts:4-5`:

```
/could not resolve host|connection (reset|refused|timed out)|early eof|failed to connect|network is unreachable|operation timed out|the remote end hung up unexpectedly/i
```

No filesystem error (EBUSY/EPERM/ENOTEMPTY) is ever classified transient, so removal is never retried.

### Concurrency guard

`runner/repo-lock.ts` — a per-`repoPath` promise-chain mutex; `RepoLock.withLock` serialises all plans for one repo. The legacy service has its own single-lane queue: `worktree-service.ts:31,54-58` (`gitOpQueue` / `enqueueGitOp`), applied only to `checkoutBranchWorktree` and `checkoutExistingBranch` — **not** to `removeWorktree` or `moveWorktree`.

### Busy-workspace guard

`packages/core/src/workspace-coordinator/guard.ts:9-27` — `assertWorkspaceIdle` returns `workspace-busy` with `holders` and `resolutions: ['force']` when `activity.sessionsFor(path)` is non-empty. Also `manager.ts:117-135` calls `hooks.beforeTeardown` for non-forced teardowns. Again, unwired.

## 3. Windows-specific handling

**There is essentially none in the worktree lifecycle.** `grep -rn "win32"` across `packages/core/src/workspace-lifecycle`, `workspace-coordinator`, `git/`, and `core/projects/worktrees` returns nothing. All the `process.platform === 'win32'` gates live in PTY spawning, terminal-shell resolution, and app-utils.

Specific gaps:

**a) No EBUSY / locked-file retry.** Every removal path bottoms out in `fs.rm(p, { recursive: true, force: true })` with **`maxRetries` left at its default of 0**:

- `steps/impl/remove-worktree.ts:13`
- `steps/impl/remove-directory.ts:7`
- `packages/core/src/files/fs/file-system.ts:173`

Node's `fs.rm` explicitly supports `maxRetries` + `retryDelay` for exactly the EBUSY/EMFILE/ENFILE/ENOTEMPTY/EPERM classes that Windows throws when an editor, watcher, or agent process still holds a handle in the worktree. The only EPERM handling anywhere is for *single files*, and it is a chmod-then-retry, not a loop — `file-system.ts:243-253`:

```ts
const code = (error as NodeJS.ErrnoException).code;
if (code !== 'EACCES' && code !== 'EPERM') throw error;
await fs.chmod(absPath, 0o666);
await fs.unlink(absPath);
```

This path is unreachable for directories (`remove()` branches to `fs.rm` at `:164-175`).

The app also runs a native FS watcher over each worktree (`git-worktree.ts:120-138`) — a classic source of Windows delete-locks. `dispose()` releases the watch (`:462-467`), but `removeOwnedLocalWorktreeDirectory` in `task-lifecycle-utils.ts` does not coordinate with the registry.

**b) `contains()` is broken on Windows — the pool containment guard does not hold.** `packages/core/src/files/paths.ts:17-20`:

```ts
export function contains(parent: string, child: string): boolean {
  const rel = path.relative(parent, child);
  return rel === '' || (rel !== '..' && !rel.startsWith('../') && !path.isAbsolute(rel));
}
```

On Windows `path.relative` emits backslashes, so `'..\\b'.startsWith('../')` is `false`. Verified:

| parent | child | `rel` | `contains` |
|---|---|---|---|
| `C:\pool\a` | `C:\pool\b` | `"..\\b"` | **true** |
| `C:\pool` | `C:\other` | `"..\\other"` | **true** |

Any sibling or ancestor-escaping path is reported as contained. This is the guard behind `WorktreeService.removePathForReuse`'s *"Refusing to remove worktree path outside pool"* (`worktree-service.ts:93-98`), behind `getWorktree`'s pool check (`:243`), and behind the preserved-file destination check (`:493-499`). The fix is `rel !== '..' && !rel.startsWith(`..${path.sep}`)` (or `path.win32.sep` handling).

**c) Workspace keys are case-sensitive over paths.** `apps/emdash-desktop/src/main/core/workspaces/workspace-key.ts:3-13` hashes `` `local:${absolutePath}` `` with SHA-256 and no case folding. On NTFS, `C:\Users\…` and `c:\users\…` are the same directory but yield different keys, so `persistPath`'s dedupe (`workspace-bootstrap-service.ts:277-284`) and the `idx_workspaces_key` unique index (`db/schema.ts:183`) can both be defeated.

**d) No `MAX_PATH` / long-path awareness.** `grep` for `MAX_PATH`, `\\?\`, or `260` across `packages/core/src` and `apps/emdash-desktop/src/main` returns nothing. Given the path shape `%USERPROFILE%\emdash\worktrees\<project>\emdash\<64-char-task-name>-<suffix>\…`, plus deep `node_modules`, this is a realistic failure mode with no mitigation.

**e) `runGit` bypasses the app's git resolution.** `steps/run-git.ts:22` calls `execFile('git', …)` literally. The desktop app resolves git through `resolveGitBin()` with PATHEXT-aware lookup and a `GIT_PATH` override (`apps/emdash-desktop/src/main/core/utils/exec.ts:1-50`), used by `LocalExecutionContext.resolveCommand` (`local-execution-context.ts:36-38`). The new stack ignores all of that, so a `GIT_PATH` override or a `git.cmd` shim won't apply.

**f) Path comparison is otherwise handled well.** `probe.ts:208-214` compares via `realpath` + `path.resolve`, and `git-worktree.ts:696-703` correctly tests both `path.isAbsolute` and `path.win32.isAbsolute`.

## 4. Orphan detection / reconciliation

There is **no reconciliation between `git worktree list` and the `workspaces` table**. Nothing enumerates git worktrees and cross-checks DB rows (or vice versa) to garbage-collect either side. What exists is narrower:

**Git-side self-healing (legacy):** `git worktree prune` is fired opportunistically and always `.catch(() => {})`'d — at `WorktreeService` construction (`worktree-service.ts:51`), whenever a `worktree list` entry fails `isValidWorktree` (`:162`, `:246`), before every `worktree add` (`:285`, `:311`, `:369`, `:413`), and after every removal (`:437`, plus `task-lifecycle-utils.ts:73`).

`isValidWorktree` (`worktree-service.ts:60-77`) is the orphan test: a linked worktree must have a `.git` **file** and `git -C <path> rev-parse --is-inside-work-tree` must print `true`.

**Adoption of pre-existing checkouts:** `findBranchAnywhere` (`:148-166`) parses `git worktree list --porcelain`, matches `branch refs/heads/<name>`, validates, and returns the path — the branch is adopted wherever git already has it. `getWorktree` (`:222-251`) does the same but additionally requires containment in the realpath'd pool.

**Stale-directory recovery:** `recovery-strategy.ts:17-56` handles exactly two error shapes — `branch-already-checked-out` (adopt the found path) and `stale-directory` (remove + prune, then retry once). Driven from `create-project-provider.ts:273-283`, which runs the executor, applies recovery, and re-runs at most once.

**New stack:** `add-worktree.ts:10-18` pre-checks `getWorktreeForBranch` and returns `stepOk({ created: false })` when the branch is already at the target path, or a structured `conflict` with `resolutions: ['use-existing','remove-existing']` otherwise — but nothing consumes those resolutions yet. `probe.ts:73-96` computes `worktree.registered` by realpath-matching the ref against `git worktree list --porcelain`, and `listRepoWorkspaces` (`:43-71`) enumerates all git worktrees with `isMain: index === 0`, `hasSetupStamp`, and `branchCreatedByEmdash`. This is the raw material for reconciliation; nobody diffs it against the DB.

**DB-side orphan handling** is limited to key collisions: `ensure-repository-workspace.ts:49-58` looks for a pre-existing row with the same key ("orphan from a previous partial failure") and adopts it; `persistPath` (`workspace-bootstrap-service.ts:277-284`) returns the *other* row's id when a key collides — note this early-returns **without** writing `path`/`branchName`, and callers ignore the returned id.

`deleteWorkspaceIfUnused` (`task-lifecycle-utils.ts:191-220`) is the counterweight: never deletes `kind === 'project-root'`, and only deletes when no sibling task references the row — with a comment documenting the regression it fixes (siblings previously orphaned onto a missing row, surfacing as `Workspace not found`).

## 5. Persisted state vs. derived-from-git

### SQLite: `workspaces` table — `apps/emdash-desktop/src/main/db/schema.ts:155-185`

| Column | Type | Notes |
|---|---|---|
| `id` | text PK | UUID minted in `createTask.ts:77` |
| `key` | text | SHA-256 of `local:<path>` / `ssh:<connId>:<path>`; unique index `idx_workspaces_key` where non-null (`:183`) |
| `type` | text NOT NULL | `'local' \| 'project-ssh' \| 'byoi'` — marked **@deprecated**, "use kind + location" |
| `kind` | text | `'worktree' \| 'project-root' \| 'byoi'` |
| `location` | text | `'local' \| 'remote'` |
| `sshConnectionId` | text FK | `→ ssh_connections.id`, `onDelete: 'set null'` |
| `data` | versioned JSON | BYOI provider data only (`workspace-provider-data.ts`) |
| `path` | text | **the resolved worktree directory** |
| `config` | versioned JSON | `{version:'2', git: GitSetup, workspace: WorkspaceTarget}` — `workspace-config.ts:62-104` |
| `branchName` | text | **cache** of the currently checked-out branch |
| `linesAdded` / `linesDeleted` | integer | **cache** of git diff totals |
| `createdAt` / `updatedAt` | text | |

`workspaces.config.git` is the authoritative *intent*: `none` / `use-branch{branchName}` / `create-branch{branchName, fromBranch, pushBranch}` / `pr-branch{prNumber, headBranch, headRepositoryUrl, isFork, taskBranch, pushBranch}` (`workspace-config.ts:22-40`).

Related, on `tasks` (`schema.ts:119-153`): `workspaceId`, plus four **@deprecated** columns kept for legacy reads — `sourceBranch`, `taskBranch`, `workspaceProvider`, `workspaceProviderData`, `workspaceIntent`. On `projects`: `repositoryWorkspaceId` (`:62`).

Note: **no `worktreeDirectory` column on `workspaces`** — the pool root is a project setting, and no per-worktree row records which pool it came from.

### Written when

- `createTask.ts:103-110` — insert with `kind:'worktree'`, `location`, `type`, `config`; **`path` and `branchName` are null** at this point.
- `WorkspaceBootstrapService.persistPath` (`workspace-bootstrap-service.ts:286-290`) — sets `path`, `key`, `branchName`, `updatedAt` after provisioning resolves a path.
- `refreshWorkspaceCurrentBranchCache` (`workspace-current-branch-cache.ts:44`) — updates `branchName` from live HEAD, driven by `handleGitWorktreeUpdate` on every `kind === 'head'` update (`workspace-worktree-update.ts:17-28`).
- `cacheWorkspaceLineStats` (`workspace-worktree-update.ts:35-56`) — updates `linesAdded`/`linesDeleted` on every `status` update.

### Derived from git at runtime (never persisted)

- **Registration / existence**: `git worktree list --porcelain` + realpath match — `probe.ts:73-96`, `worktree-service.ts:148-166`
- **HEAD / branch / detached-ness**: `symbolic-ref --short HEAD` → `rev-parse --verify HEAD`, falling back to `rev-parse --short HEAD` — `git-worktree.ts:497-514`
- **Status**: `git --no-optional-locks status --porcelain=v2 -z -uall` streamed into `StatusParser`, plus two `diff --numstat` passes — `git-worktree.ts:470-495, 530-539`
- **Branch base**: git config `branch.<name>.base` — `set-branch-base.ts`, `worktree-service.ts:200-220`
- **Branch ownership**: git config `branch.<name>.emdash-created` — `probe.ts:124-137`
- **Setup freshness**: a JSON stamp at `<gitDir>/emdash/setup-stamp` (or `<dir>/.emdash/setup-stamp` for plain directories), compared by `configHash` → `ready` / `setup-stale` / `setup-needed` — `write-setup-stamp.ts:12-13`, `probe.ts:151-184`
- **Lifecycle phase**: `derivePhase(observed, inFlight)` — pure function of directory existence + setup state + in-flight map, never stored (`probe.ts:32-41`)
- **Liveness**: `isLive: !!workspaceRegistry.get(id)` — in-memory refcount only (`getProjectWorkspaces.ts:80,98`; registry at `workspace-registry.ts:33-159`)

## Bugs / TODOs / hacks worth flagging

1. **`getDeletePreflight.ts:64-67` — dead assignment kills the dirty-worktree warning.**

```ts
if (status.kind === 'ok') {
  hasUncommittedChanges = status.staged.length > 0 || status.unstaged.length > 0;
}
hasUncommittedChanges = status.kind === 'too-many-files';   // ← unconditionally overwrites
```

For a normal (`kind === 'ok'`) dirty worktree this always lands on `false`, so the "you have uncommitted changes" confirmation before destructive task deletion never fires. The `if` block above it is dead code.

2. **`packages/core/src/files/paths.ts:17-20` — `contains()` does not detect escapes on Windows** (verified above). Directly weakens `removePathForReuse`'s "refusing to remove outside pool" guard, `getWorktree`'s pool containment, and preserved-file destination validation.

3. **`remove-worktree.ts:13` — unguarded `rm -rf` fallback.** No pool containment check, no `worktree prune` afterwards, error swallowed, `stepOk()` returned unconditionally. It will happily delete the main repo if handed the repo path, and leaks `.git/worktrees/<id>` on the fallback path.

4. **`remove-worktree.ts:7` — `--force` is not conditional.** Uncommitted work in the worktree is discarded with no `--force`-vs-plain attempt and no caller opt-in. The busy/idle guard that would gate this (`workspace-coordinator/guard.ts`) is in the unwired stack.

5. **Ownership marker missing on the live code path.** `setup-steps/create-local-branch.ts` never sets `branch.<name>.emdash-created`; `probe.ts:124-137` and `teardown.ts:58-63` depend on it. Every branch created by shipping code is invisible to the new teardown planner.

6. **Worktree path schemes disagree between the two stacks** (`planner.ts:139-141` sanitises `/` → `-`; `worktree-service.ts:224,277` does not). Cutover without a migration orphans existing directories.

7. **`workspace-key.ts` is case-sensitive over Windows paths** — breaks dedupe and the unique index.

8. **`persistPath` collision path silently no-ops** (`workspace-bootstrap-service.ts:280-283`): returns the colliding row's id without writing `path`/`branchName` on the current row, and both call sites (`:125`, `:213`) discard the return value.

9. **No filesystem errors are ever retryable** — `helpers.ts:4-5`'s transient regex is network-only, so the runner's `[1s, 4s]` backoff can never help a Windows EBUSY delete.

10. **`removeWorktree` / `moveWorktree` bypass the `gitOpQueue`** (`worktree-service.ts:431-439`), so a delete can interleave with a concurrent `worktree add` on the same repo.

11. **Fire-and-forget git in a constructor**: `worktree-service.ts:51` runs `git worktree prune` from the `WorktreeService` constructor with an unawaited `.catch(() => {})` — it can race the first `worktree add`.

12. **`workspace-config.ts:98-101` documented incompleteness**: the v1→v2 upgrade returns `null` ("needs-context") when `git.kind === 'none'` and the host is local/ssh, because `repositoryWorkspaceId` isn't reachable from the schema. Callers must resolve it out-of-band.
