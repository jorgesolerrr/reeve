# 08 — LLD: Runs — run registry, PTY, logs

**Status:** Signed off (2026-07-28)
**Ticket:** [LLD: runs subsystem — run registry, PTY, logs](https://github.com/jorgesolerrr/reeve/issues/15)
**Grounded in:** [05-lld-skeleton.md](./05-lld-skeleton.md) · [04-hld.md](./04-hld.md) · [02-domain-model.md](./02-domain-model.md) · [03-api.md](./03-api.md) · [07-lld-workspaces.md](./07-lld-workspaces.md) · [agent-invocation.md](../research/agent-invocation.md) · [reference-implementations.md](../research/reference-implementations.md)
**Visual companion:** [lld-atlas.html](./lld-atlas.html) — View 5.

## Purpose & scope

This document fixes the internals of process management: the run registry's concurrency design, the `Pty` seam and its `portable-pty` implementation, the reader pipeline behind `pty_output`, the `start_run` choreography, the single exit path, the run-history schema, raw log files and their retention, crash reconciliation, and the app-close kill sequence. It consumes the `CommandSpec` produced by 07's `spawn_spec` without knowing which provider built it. Worktree lifecycle belongs to 07; the terminal and run-history UI belong to 10 (frontend).

## Module map

| Module | Crate | Owns |
|---|---|---|
| `seams/pty` | `reeve-core` | The `Pty` trait, `RunHandle`, `PtySize`, the output sink and exit callback types. |
| `seams/run_history` | `reeve-core` | The `RunHistory` trait over the run tables: insert, finalize, mark-interrupted, list, log-file lookup. |
| `services/runs` | `reeve-core` | The six operations (03-api: `preview_context`, `start_run`, `kill_run`, `write_stdin`, `resize_pty`, `list_runs` — plus `read_run_log`, amended below), the in-memory run registry, and the exit finalization. |
| `pty` | `reeve-infra` | `portable-pty` spawn, process-group control (setsid / Job Objects), the reader thread, the waiter, raw log writing. |
| `index` (run tables) | `reeve-infra` | The `RunHistory` implementation over the same per-Project SQLite file 06 owns. |

## The run registry

A map with one global lock, tasks per Run — no actor layer:

```rust
pub struct RunRegistry(Mutex<HashMap<(ProjectId, TicketId), RunEntry>>);

enum RunEntry {
    Starting,                     // reservation: spawn in flight
    Live(LiveRun),
}

struct LiveRun {
    run_id: i64,                  // the SQLite rowid
    kind: RunKind,                // Agent | Terminal | Verify
    handle: Arc<dyn RunHandle>,   // write / resize / kill
}
```

- **One mutex for the whole registry.** Single-user scale: a handful of live Runs, contention is zero in practice. The lock is **never held across an `await`** — a discipline, greppable in review; commands clone the `Arc<dyn RunHandle>` out under the lock and operate on it after releasing.
- **Sequentiality is checked where the entry is inserted.** Any entry present (`Starting` *or* `Live`) ⇒ `workspace/run_active`. The `Starting` reservation closes the race two concurrent `start_run`s would otherwise win together — the check and the claim are atomic under the same lock.
- **The registry is the only truth about liveness.** No `status` column, no liveness flag anywhere on disk (see schema below): "is a Run live?" is answered by this map and nothing else — derive-don't-store applied to processes.

## The `Pty` seam

```rust
pub trait Pty: Send + Sync {
    /// Spawns spec in a fresh PTY, owning the whole output lifecycle:
    /// raw bytes → log file at log_path, UTF-8 chunks → sink, exit → on_exit
    /// (called exactly once, after the log is flushed and closed).
    fn spawn(
        &self,
        spec: CommandSpec,
        size: PtySize,                       // initial 80×24; resized on attach
        log_path: PathBuf,
        sink: Arc<dyn PtyOutputSink>,        // pre-scoped to (project, ticket)
        on_exit: Box<dyn FnOnce(ExitReport) + Send>,
    ) -> Result<Arc<dyn RunHandle>, ApiError>;
}

pub trait RunHandle: Send + Sync {
    fn write_stdin(&self, data: &[u8]) -> Result<(), ApiError>;
    fn resize(&self, size: PtySize) -> Result<(), ApiError>;
    fn kill(&self);                          // hard group kill; idempotent
}

pub trait PtyOutputSink: Send + Sync {
    fn output(&self, utf8: &str);            // called from the reader thread
}

pub struct PtySize { pub cols: u16, pub rows: u16 }
pub struct ExitReport { pub exit_code: i32 } // what wait() reported; signal deaths per portable-pty
```

**The hot path is a direct call, not a channel.** 06's watcher pushes `GraphChanged` values on a broadcast channel that the composition root forwards to ring 1's emitter; `pty_output` deliberately does not follow that pattern. The sink is a trait object the composition root builds over the ring-1 emitter (Tauri's `emit` is thread-safe, callable from the reader thread directly), pre-scoped to `(project, ticketId)`. Zero hops for the one high-frequency stream in the system — `pty_output` is 03's declared exception, and its plumbing is allowed to look like one.

## Spawn and the process group

`portable-pty`: `openpty(size)` → `CommandBuilder` built from the `CommandSpec` (program, args, cwd, env — argv vectors, never a shell) → `slave.spawn_command`. Group control is won *around* that spawn, per platform:

- **Unix — free.** `portable-pty` gives the child its own session (`setsid`) as the PTY's controlling process: it is session and group leader, so **pgid = child pid**. Kill = `kill(-pgid, SIGKILL)` — every descendant that hasn't escaped the session falls.
- **Windows — one Job Object per Run.** Created before spawn with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; the child's PID is assigned (`AssignProcessToJobObject`) immediately after spawn; grandchildren inherit the Job automatically. Kill = `TerminateJobObject` — one syscall, the whole tree. Two properties ride along: the spawn-to-assignment window (milliseconds; grandchildren created inside it escape the Job) is documented and accepted; and `KILL_ON_JOB_CLOSE` is a free crash safety net — if reeve dies, the OS closes the Job handle and reaps the tree unaided. The Tier 1 platform gets the hard guarantee.

**Kill is hard, always.** No SIGTERM-then-grace phase: `kill_run` is an explicit user abort, not a cooperative shutdown. An agent killed mid-write leaves the worktree in a state Review and discard already handle (the diff is inspectable; discard erases everything); a grace phase would add a timer and two intermediate registry states no requirement asks for. vibe-kanban's group-SIGKILL is the precedent.

## The reader pipeline

One dedicated OS thread per Run — `portable-pty` reads are blocking, so a thread exists by physical necessity; the design gives it *everything* and nothing else gets a piece:

```
loop: read up to 8 KiB from the PTY master
  1. append raw bytes to the log   (BufWriter, flushed per chunk)
  2. incremental UTF-8 decode      (carry buffer for split multibyte sequences)
  3. sink.output(chunk)            (→ pty_output event)
until read == 0 (EOF) → final flush, close the file, thread ends
```

- **Log before emit, per chunk.** The file is always at least as complete as what the UI saw: if reeve dies mid-Run, the log does not trail the screen. Flush-per-chunk keeps the forensic record live (a log seconds behind the terminal would falsify crash forensics).
- **Bytes vs text.** The log stores **raw bytes** — the session as it happened, ANSI escapes included. The event carries **UTF-8**: it crosses into JSON/xterm.js. The decode is incremental with a carry buffer, never `from_utf8_lossy` per chunk — a naive per-chunk conversion corrupts multibyte characters (accents, box-drawing) exactly at chunk boundaries.
- **Per-chunk emission, no coalescing machinery.** The kernel's PTY buffer already coalesces under load: each `read` returns whatever accumulated while the previous chunk was being logged and emitted, so the event rate self-limits to the pipeline's own pace. xterm.js buffers writes internally and renders by frames regardless of call frequency. Time-window coalescing over a blocking `Read` means timeout-reads or a cross-thread timer — speculative machinery of exactly the kind the reference research warns against. **Extension point, should dogfooding show real flooding:** accumulate inside this loop before `sink.output` — a local change, touching no contract.

## The exit sequence — one path out

Anchoring "the Run exited" on the reader's EOF would be a domain deadlock: a grandchild holding the inherited slave (an agent-started dev server) keeps EOF from ever arriving. **The main process's exit is the domain event, and its death takes its tree with it:**

A *waiter* task per Run (`spawn_blocking` over `child.wait()`):

1. `wait()` returns → capture the exit code.
2. **Kill the rest of the group immediately** (`TerminateJobObject` / `kill(-pgid)`). Domain law: **nothing a Run spawned survives it** — as a real terminal hangs up your jobs on close. ADR-0005 deferred preview servers, so no requirement wants long-lived grandchildren; and killing at exit means no pgid retention after death (the Unix PID-reuse window never opens) and no zombie Job handles in the registry.
3. The group kill closes the last slave handles → the reader sees EOF, flushes, closes the log. The waiter joins the reader with a short timeout (~2 s) — the log is complete before anything is announced.
4. Finalize the SQLite row: `exit_code`, `ended_at`.
5. **Remove the registry entry — before emitting.** When the UI re-queries on the event, the state is already coherent; there is no "event says dead, registry says live" window.
6. Emit `run_exited { runKind, exitCode }` and `workspace_changed`.

`kill_run` is the same path, only provoked: `RunHandle::kill` group-kills (main process included), the waiter — already blocked in `wait` — wakes and runs 1–6 identically. Steps 4–6 never fork on *why* the process died.

**Refinement of 07.** The destroy sequence's step 2 ("kill process-group orphans") now finds the group already dead — it died with its Run. The step remains as belt-and-braces over whatever the registry still knows, and destroy's precondition (no live Run) is unchanged.

## `start_run` choreography

1. **Reserve under the lock:** entry present → `workspace/run_active`; else insert `Starting`, release the lock.
2. **Resolve the command** (outside the lock):

   | kind | program | prompt/context |
   |---|---|---|
   | `agent` | the Agent Profile's command template | regenerate `AGENTS.md` via `context_assembler` (temp file + rename — never half a file), adjustments applied whole |
   | `terminal` | Windows: `powershell.exe` · Unix: `$SHELL`, fallback `/bin/sh` — fixed in v1, configurable if dogfooding asks | none |
   | `verify` | the Project's Verify Command (02: `.reeve/config.json`) | none |

   The resolved request goes through the provider's `spawn_spec` (07) — cwd, `REEVE_TICKET_ID`, `REEVE_WORKSPACE_PATH` arrive from there.
3. **Create the log file** (path per the naming rule below). Failure ⇒ the Run does not start — better no Run than a Run without its forensic record.
4. **Spawn** via the `Pty` seam — initial size **80×24**; the frontend sends `resize_pty` as soon as xterm mounts (documented handshake). Job Object / setsid per above.
5. **Insert the SQLite row** — *after* the spawn: a row exists only if its process existed. The crash window between 4 and 5 (process without row) is covered on Windows by `KILL_ON_JOB_CLOSE` and is the same accepted Unix gap as reconciliation's.
6. **Swap the reservation** for `Live` (handles, reader, waiter already running) and return the run row.

**Unwind on failure in 2–4:** remove the reservation, delete the log file if it was created, return the error. No row — failed attempts to start are not Runs and leave no residue in history.

## Run history schema

Run tables live in the same per-Project `index.sqlite` as the graph (06), same `user_version`, same policy: **rebuild, never migrate** — history is operational metadata, losable by decree of 02.

```sql
CREATE TABLE runs (
  id          INTEGER PRIMARY KEY,          -- rowid; the runId in the API
  ticket_id   TEXT    NOT NULL,             -- 'T-42'
  kind        TEXT    NOT NULL,             -- 'agent' | 'terminal' | 'verify'
  profile     TEXT,                         -- Agent Profile name; kind='agent' only
  started_at  INTEGER NOT NULL,             -- unix ms
  ended_at    INTEGER,                      -- unix ms; NULL while live
  exit_code   INTEGER,                      -- NULL while live or when interrupted
  interrupted INTEGER NOT NULL DEFAULT 0,   -- set by startup reconciliation
  log_file    TEXT    NOT NULL              -- filename, relative to logs/<ticket_id>/
);
CREATE INDEX runs_by_ticket ON runs (ticket_id, started_at);
```

The decisions buried in those columns, explicit:

1. **No `status` column.** "Live" is never read from the database — the registry is the truth of liveness. A row's state derives: `exit_code NOT NULL` = exited; `interrupted = 1` = interrupted; neither, and absent from the registry = reconciliation candidate. A persisted `running` status is exactly the lie vibe-kanban paid for.
2. **`log_file` is stored** — a deliberate exception to derive-don't-store. Deriving it would freeze the timestamp format forever; the stored name is an immutable fact created together with the file. The *root* (`~/.reeve/projects/<slug>/logs/`) stays derived — the database survives `~/.reeve` moving.
3. **The board's "last Run"** (In Review predicate) = max `started_at` per ticket — hence the composite index.
4. **`profile` is a name, not a snapshot.** History says *what* you ran, not the config it ran with; exact reproducibility is not a requirement, and the raw log is the forensic record.
5. Any `user_version` mismatch ⇒ delete the file and rebuild — history included, by decree.

## Log files

- **Naming:** `logs/<ticket_id>/<YYYYMMDD-HHMMSS-mmm>.log` — human-readable, lexicographic = chronological, no `:` (Windows Tier 1). The filename is the row's `log_file`.
- **Creation:** in `start_run`, before the spawn (choreography step 3).
- **Retention rides the Workspace's fate — no reaper:**
  - **`discard` deletes** the ticket's `runs` rows and its `logs/<ticket_id>/` directory. 03's law, extended to its full residue: a failed experiment leaves none.
  - **`merge` and `push` keep both.** Landed work keeps its forensic record; the Resolution Note is the distilled memory (ADR-0001), the raw log its cheap backing. Plain text, megabytes at most.
  - **No size/age policy in v1.** An age reaper is machinery without a requirement; the layout (everything under `logs/`) keeps that decision trivial if dogfooding ever demands it.
- **Reading:** `read_run_log` (amended into 03 below) returns the log as lossy UTF-8, defaulting to a **bounded tail (2 MiB)** — a giant log must not freeze the UI, and xterm.js needs no more scrollback than that. Serves both history inspection and scrollback restore after an app restart.

## Crash reconciliation

Runs **at Project open** (projects are lazy; each owns its SQLite):

```sql
UPDATE runs SET interrupted = 1 WHERE exit_code IS NULL AND interrupted = 0;
```

- The in-memory registry starts empty, so board derivation is correct by construction. No events: the UI is querying fresh anyway.
- **Interrupted counts as exited** for the board: the process is gone, so "last Run exited" holds — with a non-empty diff the ticket wakes up **In Review**, the agent's work inspectable. Exactly what you want after a crash: see what it left, don't lose it.
- **No cross-lifetime orphan hunt.** Windows needs none — `KILL_ON_JOB_CLOSE` already reaped the tree when reeve died. On Unix, hunting would mean persisting pgids and speculatively `kill(-pgid)`-ing at startup, where PID reuse can murder an innocent process; verifying identity (process start-time via `/proc`, different again on macOS) is fragile cross-platform machinery for a rare case. The Unix crash-orphan is a **documented gap**: it may keep writing to the worktree for a while; the watcher and query-time derivation absorb whatever it writes, and the worst case is a diff that grows after the ticket surfaced In Review. The asymmetry is honest: the Tier 1 platform holds the hard guarantee; on Unix a rare orphan beats a speculative kill.

## App-close kill sequence

The HLD's law — reeve owns its children; closing kills live Runs, with a warning — split into two layers with different owners:

1. **UX layer (frontend).** Intercepts the close (`onCloseRequested`). It knows the live set from its own state — it started every Run and receives every `run_exited` — so no new operation and no fifth event. Non-empty ⇒ dialog: *"N runs are live; quitting will kill them"* — confirm proceeds, cancel aborts. Empty ⇒ frictionless close.
2. **Guarantee layer (backend, unconditional).** On Tauri's exit event, the registry runs its final sequence, synchronous and bounded: group-kill every live entry, then wait briefly (~2 s total) for waiters to finalize rows and readers to flush logs. This layer consults nobody — it runs even if the dialog had a bug. Its failure net already exists: an unfinalized row is marked `interrupted` at the next open — coherent degradation, not corruption.

Runs killed at exit finalize like any kill: the same single exit path, violent-death exit code recorded. On Windows, if even layer 2 dies mid-close, `KILL_ON_JOB_CLOSE` finishes the job.

The dialog is the frontend's, not the core's — the core knows no windows (ring rule); "warn before killing" is presentation, "nothing survives reeve" is registry law.

## Amendments to earlier documents

- **03-api.md** — `list_runs` is the **ticket's** history, not the Workspace's: rows and logs survive merge/push (a `done` ticket's history stays readable); discard erases them. And the runs area gains one operation: **`read_run_log(ticketId, runId, tailBytes?)`** → lossy UTF-8 log content, bounded tail (2 MiB) by default. Rationale: the frontend reading log files directly would open a second data path around the API — exactly what ring discipline and future server extraction forbid.
- **07-lld-workspaces.md** — destroy step 2 refined: by the exit sequence above, a Run's group dies with it; the step remains as belt-and-braces over registry leftovers.

## Fixtures

`fixtures::pty` — a scripted `Pty`: each spawn plays a `PtyScript { chunks, exit_code }` through the real sink/callback contract (chunks to the sink, then `on_exit`), with `kill()` cutting the script short. `fixtures::run_history` — the `RunHistory` trait over an in-memory `Vec`. Together they make the whole surface above — choreography and unwind, sequentiality under concurrent `start_run`, the exit ordering (registry-before-event), reconciliation, retention — testable hermetically in `reeve-core` (skeleton, `feature = "fixtures"`).

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-28
