# 07 — LLD: Workspaces — git module and WorkspaceProvider

**Status:** Signed off (2026-07-27)
**Ticket:** [LLD: workspaces subsystem — git module and WorkspaceProvider](https://github.com/jorgesolerrr/reeve/issues/14)
**Grounded in:** [05-lld-skeleton.md](./05-lld-skeleton.md) · [04-hld.md](./04-hld.md) · [02-domain-model.md](./02-domain-model.md) · [03-api.md](./03-api.md) · [isolation.md](../research/isolation.md) · [reference-implementations.md](../research/reference-implementations.md) · [emdash-worktree-lifecycle.md](../research/emdash-worktree-lifecycle.md)
**Visual companion:** [lld-atlas.html](./lld-atlas.html) — View 4.

## Purpose & scope

This document fixes the internals of git orchestration: the typed `Git` seam and its CLI wrapper contract, the `WorkspaceProvider` trait, the create/adopt flow, the Windows-safe destroy sequence, `board_derivation`'s git queries, and the merge/push flows. Process spawning and the run registry belong to 08 (runs) — this document consumes them only through the "no live Run" precondition and the orphan-kill hook. The Review tab's rendering of diffs belongs to 10 (frontend).

## Module map

| Module | Crate | Owns |
|---|---|---|
| `seams/git` | `reeve-core` | The typed `Git` trait, its value types (`WorktreeInfo`, `WorkingTreeStatus`, `MergeOutcome`), and `GitError`. |
| `seams/workspace` | `reeve-core` | The `WorkspaceProvider` trait, `WorkspaceHandle`, `CommandSpec`. |
| `services/workspaces` | `reeve-core` | `create_workspace`, `get_workspace`, `discard_workspace` (03-api), composing the provider + the lock map. |
| `services/review` | `reeve-core` | `merge`, `push` — the two non-discard Workspace endings, plus the review-tab reads (diff, verify signal). |
| `domain/workspace_locks` | `reeve-core` | The per-ticket async mutex map shared by `workspaces` and `review`. Sequencing is domain law, so the lock lives in core, not in any provider. |
| `domain/board_derivation` | `reeve-core` | The derived-state predicate — a pure function over data fetched through the seams (extended below with the exact git inputs). |
| `git` | `reeve-infra` | The CLI wrapper (spawn, parse, classify) implementing `Git`, and `WorktreeProvider` implementing `WorkspaceProvider` — including the filesystem retry loop, which is I/O and therefore cannot live in core (skeleton, enforcement layer 2). |

## The `Git` seam: typed operations, CLI-free core

Per the grilling decision: the core sees domain-meaningful operations returning parsed types; argument construction, porcelain parsing, and exit-code classification live entirely in `reeve-infra::git`. A new git capability = a new trait method, visible in review as a design decision. The fixture (`fixtures::git`, in-memory branches + worktrees in a `HashMap`) fakes typed methods trivially — a raw `run(args)` seam would force the fixture to emulate the CLI.

```rust
pub trait Git: Send + Sync {
    // repo facts
    fn default_branch(&self, repo: &Path) -> Result<Option<String>, GitError>;   // origin/HEAD → fallback current branch
    fn current_branch(&self, repo: &Path) -> Result<Option<String>, GitError>;   // None when detached
    fn branch_exists(&self, repo: &Path, branch: &str) -> Result<bool, GitError>;
    // working-tree reads (all with --no-optional-locks)
    fn status(&self, dir: &Path) -> Result<WorkingTreeStatus, GitError>;         // porcelain v2 -z -uall
    fn ahead_count(&self, dir: &Path, base: &str) -> Result<u64, GitError>;      // rev-list --count base...HEAD
    fn diff_stats(&self, dir: &Path, base: &str) -> Result<DiffStats, GitError>; // numstat vs merge-base (review tab)
    fn diff_patch(&self, dir: &Path, base: &str) -> Result<String, GitError>;    // unified patch vs merge-base (review tab)
    // worktrees
    fn list_worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>, GitError>; // worktree list --porcelain -z
    fn add_worktree(&self, repo: &Path, path: &Path, branch: &str) -> Result<(), GitError>;
    fn remove_worktree(&self, repo: &Path, path: &Path) -> Result<(), GitError>;  // always --force; gating is the caller's law
    fn prune_worktrees(&self, repo: &Path) -> Result<(), GitError>;
    // branches + branch config
    fn create_branch(&self, repo: &Path, branch: &str, start: &str) -> Result<(), GitError>;
    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<(), GitError>;   // -D; provenance gating is the caller's law
    fn branch_config(&self, repo: &Path, branch: &str, key: &str) -> Result<Option<String>, GitError>;
    fn set_branch_config(&self, repo: &Path, branch: &str, key: &str, value: &str) -> Result<(), GitError>;
    // endings
    fn merge(&self, repo: &Path, branch: &str) -> Result<MergeOutcome, GitError>; // Ok | Conflict { files } (after abort)
    fn push(&self, repo: &Path, remote: &str, branch: &str) -> Result<(), GitError>; // push -u
}
```

`MergeOutcome::Conflict` is a *value*, not an error: the infra impl runs `git merge`, and on conflict runs `git merge --abort` itself before returning — the seam never exposes a repo in a half-merged state.

### The CLI wrapper contract (infra)

| Invariant | Rule |
|---|---|
| Spawning | argv vectors, never a shell — paths with spaces and non-ASCII (this repo's own path) are data, not strings to quote. `git` resolved from PATH once at preflight; minimum version **2.30** enforced there (everything used here — porcelain v2, `--no-optional-locks`, worktree porcelain — is comfortably older; 2.30 is the 2020 floor that predates every distro we care about). |
| Working directory | Every invocation passes `-C <dir>` explicitly. No inherited cwd, ever. |
| Environment | `GIT_TERMINAL_PROMPT=0` (fail, never hang — credential helpers still work for push), `LC_ALL=C` (stable stderr for the classification *fallback*; classification is exit-code-first, vibe-kanban's `gh` lesson applied to git). |
| Reads | All queries pass `--no-optional-locks` — reeve must never contend with the user's other git clients (the #3406 gc-concurrency lesson). |
| Parsing | `--porcelain=v2 -z` for status, `--porcelain -z` for worktree list. NUL separators mean paths never need unquoting. Output decoded as UTF-8 (lossy). |
| Timeouts | 30 s default, 120 s for `push` (the one network operation). On timeout: kill the process, return `GitError::Timeout`. |
| `.git` internals | **Never touched with filesystem calls.** Every mutation of git state is a git command. The one filesystem deletion this module performs is the worktree *directory* under reeve's managed root (destroy step 4) — never anything under `.git`. |

### `GitError` → `ApiError` mapping

| `GitError` | `ApiError` code | details |
|---|---|---|
| `Dirty { files }` | `git/dirty` | `{ files }` — UI lists them, offers the Terminal |
| `MergeConflict { files }` | `git/merge_conflict` | `{ files }` — merge was aborted; manual path stays open |
| `BaseNotCheckedOut { current }` | `git/base_not_checked_out` | `{ expected, current }` — actionable message, reeve never checks out for you |
| `PushRejected` | `git/push_rejected` | non-ff — fetch/rebase outside reeve |
| `Network` | `git/network` | offline-first: push is the only operation that can produce it |
| `Timeout` / `Failed { code, stderr }` | `git/failed` | the catch-all; stderr included verbatim for the human |
| (service-level, not from git) | `workspace/already_exists` · `workspace/run_active` · `workspace/not_found` · `workspace/destroy_incomplete` | see flows below |

## The `WorkspaceProvider` seam

```rust
pub trait WorkspaceProvider: Send + Sync {
    fn create(&self, project: &Path, ticket: &TicketId) -> Result<WorkspaceHandle, ApiError>;
    fn probe(&self, project: &Path, ticket: &TicketId) -> Result<Option<WorkspaceHandle>, ApiError>;
    fn destroy(&self, project: &Path, ticket: &TicketId, branch: BranchDisposition) -> Result<DestroyReport, ApiError>;
    fn spawn_spec(&self, handle: &WorkspaceHandle, req: CommandRequest) -> CommandSpec;
}

pub struct WorkspaceHandle { pub ticket: TicketId, pub path: PathBuf, pub branch: String, pub base: String }
pub enum BranchDisposition { Delete /* merge, discard */, Keep /* push */ }
pub struct DestroyReport { pub leftover: Option<PathBuf> }  // Some ⇒ retries exhausted, reported, never silent

pub struct CommandRequest { pub program: String, pub args: Vec<String>, pub env: Vec<(String, String)> }
pub struct CommandSpec    { pub program: String, pub args: Vec<String>, pub cwd: PathBuf, pub env: Vec<(String, String)> }
```

`spawn_spec` is the container seam, adapted to reeve's PTY model (the grilling's option c): the provider mediates *how* a process enters the workspace without owning execution. The worktree provider returns the command unchanged with `cwd = worktree path`; a future container provider returns a `docker exec -i …` wrapper. The `runs` service (08) hands the resulting `CommandSpec` to the `pty` seam without knowing which provider produced it. Every spec carries the v1 environment contract: `REEVE_TICKET_ID` and `REEVE_WORKSPACE_PATH` (port-deconfliction variables wait for real demand).

`probe` is derive-don't-store made mechanical: existence = `git worktree list` + branch config, path = the managed root, base = `branch.reeve/T-<n>.reeve-base`. There is no workspace row anywhere to desync.

**Deliberately absent:** `exec(handle, cmd)` batch execution (v1 has no batch caller; `spawn_spec` carries the intent), worktree locks and idle sweeps (reeve has no background reaper — nothing sweeps, so nothing needs locking against a sweep; the vibe-kanban orphan scanner is the design this absence rejects), and `--relative-paths` (it would flip `extensions.relativeWorktrees` on the *user's repo*, breaking libgit2-based tools today for a container feature we don't ship; the container provider decides later between relative paths and path-identical mounts).

## Git-config markers: provenance and base

Written at branch creation, read everywhere, stored *in git* so they survive index deletion, travel with the repo, and are inspectable with stock tooling (emdash's one great invention, minus its bug of not writing the marker on the live path):

```
branch.reeve/T-42.reeve-created = true      # provenance: reeve made this branch
branch.reeve/T-42.reeve-base    = main      # the base it was cut from
```

Two laws hang on the marker:

1. **Never delete what you can't prove you created.** `delete_branch` is only ever invoked on a branch whose `reeve-created` is set. A user's hand-made branch that happens to be named `reeve/T-42` is never deleted — creation fails with `workspace/already_exists` instead of adopting it.
2. **The managed root is its own proof.** Directories under `~/.reeve/worktrees/<slug>/` may be deleted by reeve (reeve owns that root; nothing else legitimately lives there). Outside it, reeve deletes nothing, ever. The containment check canonicalizes both paths and compares with `Path::starts_with` on components — never string prefixes (emdash's guard is broken on Windows precisely because `..\` ≠ `../`).

The base marker makes each live workspace's diff base a per-workspace fact: if the Project's `baseBranch` config changes mid-flight, live workspaces keep the base they were cut from — the diff you review is against what you branched from, and the merge dialog prefills from the marker.

## Base branch resolution

`baseBranch` is a Project config field (`.reeve/config.json` — 02 amended), resolved at Project registration: `git symbolic-ref refs/remotes/origin/HEAD` → fallback to the current branch when there is no remote. User-editable thereafter. `create_workspace` cuts `reeve/T-<n>` from the **local tip** of that branch — no implicit `git fetch`: offline-first, and reeve touches the network only when asked (push).

## Create flow

`create_workspace(ticketId)` — awaitable, budgeted < 10 s (03-api):

1. **Lock** the ticket (`workspace_locks`) — create/destroy of the same workspace serialize (vibe-kanban's per-path locks, keyed by ticket since path derives from it).
2. **Preconditions**: Ticket exists and is not `done`; no live worktree for it; base branch exists.
3. **Branch gate**: if `reeve/T-<n>` already exists —
   - with `reeve-created` set → **adopt**: skip to step 6, `git worktree add` on the existing branch. This is the legitimate resume-after-push case: reeve re-taking what reeve made, with the pushed work in place.
   - without the marker → `workspace/already_exists`. A foreign branch colliding with our namespace is never adopted or touched.
4. `git worktree prune` — clear stale registrations before creating (cheap, idempotent, emdash does it before every add).
5. `git branch reeve/T-<n> <base>` + write both config markers.
6. `git worktree add ~/.reeve/worktrees/<slug>/T-<n> reeve/T-<n>`.
7. **Verify**: the path exists and `git -C <wt> rev-parse --is-inside-work-tree` answers — "git reported success" is not proof (vibe-kanban: *"reported success but path does not exist"*).
8. **Retry once** if the add failed on a stale registration: `git worktree remove --force` on the conflicting path (git-mediated — never `remove_dir_all` inside `.git`), `prune`, retry the add exactly once.
9. **`.worktreeinclude` copy-in**: if the repo has one, copy the matching gitignored paths (`.env`, local configs) into the new worktree — Claude Code's filename and gitignore syntax, adopted not invented, matched with the `ignore` crate. No file, no copying. Dependency install is deliberately *not* here: it belongs to the user's Terminal in v1 and to the loop/hook model later ("on workspace created" is one of its events).
10. Emit `workspace_changed { ticketId }`.

## Destroy sequence (Windows-safe)

The shared destructive tail of discard, merge, and push. The failure mode it is built for, reproduced first-hand in [isolation.md](../research/isolation.md) §5.2: on Windows, `git worktree remove` can deregister the worktree *before* the directory deletion finishes — leaving a directory git can no longer see.

0. **Precondition (discard): no live Run** — `workspace/run_active`, and the UI offers kill. Discard never auto-kills: one operation, one destructive effect; kill-then-discard is two explicit user acts.
1. **Lock** the ticket (same map as create).
2. **Kill process-group orphans**: the run registry (08) tracks each Run's process group; even after the main process exited, children may survive (agent-started dev servers, MCP servers). Any survivor in the group is killed before touching disk. Reeve kills only what it spawned — a foreign handle (the user's editor open in the worktree) is never touched; it will exhaust the retries and be named in the final error.
3. **`git worktree remove --force <path>`** — the happy path, git-mediated, handles registration and directory together.
4. On failure (Windows file locks): **retry loop deleting the directory directly** — legal, it is inside the managed root (law 2 above) — with backoff **100 → 200 → 400 → 800 → 1600 ms** (5 attempts). Symlinks and junctions are removed as links, never recursed through (Rust std's `remove_dir_all` guarantees this; stated here as an invariant because the Claude Code worktree docs record the bug class). Then `git worktree prune` reconciles the registration — the right order for the deregister-before-delete failure.
5. **Branch disposition**: `Delete` (merge, discard) → `git branch -D reeve/T-<n>`, gated on `reeve-created`. `Keep` (push) → branch and markers stay.
6. `git worktree prune` (idempotent; also covers the step-3 success path).
7. **Verify**: path gone, worktree absent from `git worktree list`, branch per disposition. Leftover path ⇒ `DestroyReport { leftover: Some(path) }`.
8. Emit `workspace_changed { ticketId }`.

How a leftover surfaces depends on the caller: `discard_workspace` returns `workspace/destroy_incomplete { path }` (the operation's whole point failed); merge and push report it as a **warning in the result** (the merge/push *happened* — calling it an error would lie), and `discard` remains invocable as the cleanup retry. Nothing is ever swallowed (the anti-pattern is emdash's unconditional `stepOk()`).

## `board_derivation`: the git inputs

The In Review predicate (02): live Workspace ∧ last Run exited ∧ diff against base non-empty. This document fixes "diff non-empty" as:

> **commits above the merge-base** (`ahead_count(wt, base) > 0`, three-dot — the base advancing underneath never flips a ticket's state) **∨ dirty or untracked working tree** (`status(wt)` non-empty, `-uall`).

The second disjunct is deliberate: an agent that wrote files without committing has produced reviewable work — the board must say In Review, and the Review tab shows the full working-tree-vs-merge-base delta (`diff_stats`/`diff_patch`), not just commits. The guard that keeps uncommitted work from being *lost* lives in the merge flow, not here — deriving states imposes no policy.

Cost per board query: 1 × `list_worktrees` for all existence checks, then 2 cheap reads (`status`, `ahead_count`) per **live** workspace only — live workspaces are a handful (parallelism across tickets, not hundreds). All reads `--no-optional-locks`. **No cache**: derived at query time, every time (HLD law) — at this scale a cache is only a new way to lie.

## Merge flow

`merge(ticketId, baseBranch)` — the base must be checked out somewhere, and that somewhere can only be the user's main checkout (the ticket worktree holds `reeve/T-<n>`):

1. Lock; no live Run (`workspace/run_active`).
2. **Ticket worktree clean** — uncommitted work would die in the destroy tail; `git/dirty { files }`, the user commits in the Terminal and retries.
3. **Main checkout on `baseBranch`** — else `git/base_not_checked_out`. Reeve never checks out for you: moving the user's checkout is exactly the surprise "your git, your rules" forbids.
4. `git merge reeve/T-<n>` in the main repo, **no flags** — the user's git config decides ff vs merge commit; no generated messages (03-api).
5. **Conflict → abort + `git/merge_conflict { files }`.** Reeve never leaves the user's checkout mid-conflict. The escape hatch is git-native: merge by hand in the terminal with your own tools; the watcher sees the merge land, and a subsequent `discard` cleans branch and worktree with nothing lost (the work lives on base).
6. Merge landed → **mark `done`** (front-matter via vault; auto-committed under `.reeve/` when the flag is on). The domain act completes when the merge exists, not when cleanup finishes.
7. Destroy tail with `BranchDisposition::Delete`.
8. Cleanup failure after a landed merge → **warning in the result**, path named, `discard` as retry.
9. Link re-validation → warnings (FR-5.3 — warn, never block).

## Push flow

`push(ticketId, remote?)`:

1. Lock; no live Run; **worktree clean** (same guard as merge — push ships commits only, and the destroy tail would erase the rest).
2. `git push -u <remote> reeve/T-<n>` — `remote` defaults to `origin`; `-u` leaves tracking set for the post-PR work that happens outside reeve. The subsystem's only network operation: offline fails clean (`git/network`), non-ff fails as `git/push_rejected`.
3. Destroy tail with `BranchDisposition::Keep` — worktree removed, branch and markers stay.
4. No `done`; the ticket derives back to Backlog. `workspace_changed`.

The three endings, restated as this document implements them: merge and discard end with `Delete`; push ends with `Keep`; that disposition is their only difference in the destructive tail — exactly the asymmetry 03-api fixed.

## Amendments to earlier documents

- **02-domain-model.md** — the Project configuration list gains **`baseBranch`** (resolved at registration from `origin/HEAD`, fallback current branch; user-editable).
- **03-api.md** — `get_project_config` / `update_project_config` row: config contents now include `baseBranch`.

## Fixtures

`fixtures::git` — an in-memory `Git`: branches, worktrees, branch config, and working-tree status as plain maps; `merge` scripted per test (`Ok`/`Conflict`). `fixtures::workspace` — a `WorkspaceProvider` over a temp-dir-free in-memory model. Both compile only under `feature = "fixtures"` (skeleton) and make every flow above — including the destroy tail's disposition logic and the board predicate — testable hermetically in `reeve-core`.

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-27
