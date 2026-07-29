# 05 — LLD: Skeleton — monorepo layout and ring-rule enforcement

**Status:** Signed off (2026-07-26)
**Ticket:** [LLD: skeleton — monorepo layout and ring-rule enforcement](https://github.com/jorgesolerrr/reeve/issues/12)
**Grounded in:** [04-hld.md](./04-hld.md) · [ADR-0006](../adr/0006-app-shell-and-tech-stack.md) · [reference-implementations.md](../research/reference-implementations.md)
**Visual companion:** [lld-atlas.html](./lld-atlas.html) — the living LLD atlas this document seeds; tickets 06–10 extend it with per-subsystem classes and state.

## Purpose & scope

This document fixes the physical shape of the codebase: the Cargo workspace and its crates, the frontend project layout, the type contract between them, and the machinery that makes the HLD's ring rule verifiable on every commit. Subsystem internals (schemas, trait signatures, component classes) belong to the five LLD documents that follow (06–10).

## The Cargo workspace: three crates, one per ring

The three HLD rings map 1:1 onto three crates. The crate graph *is* the first enforcement layer: a crate physically cannot use a dependency its `Cargo.toml` does not list.

| Crate | Folder | Ring | Kind | Contains |
|---|---|---|---|---|
| `reeve-app` | `src-tauri/` | 1 — adapters | binary | `commands` (one `#[tauri::command]` per operation), `events` (the four-event emitter), and the **composition root**: the only place concrete infra implementations are wired into core services. |
| `reeve-core` | `crates/reeve-core/` | 2 — core | library | The 9 services (`projects`, `graph`, `tickets`, `epics`, `sources`, `workspaces`, `runs`, `review`, `system`), the shared domain components (`context_assembler`, `board_derivation`), the seam traits (`TicketSource`, `WorkspaceProvider`, and the infra traits for git/index/vault/pty/gh), the DTOs, and the single `ApiError` envelope (03-api). |
| `reeve-infra` | `crates/reeve-infra/` | 3 — infrastructure | library | `git`, `index`, `vault`, `pty`, `gh_client` — the modules of the HLD inventory, each implementing the core trait it serves. |

Dependency edges, and nothing else:

- `reeve-infra` → `reeve-core` (dependency inversion: infra *implements* the traits core defines).
- `reeve-app` → `reeve-core` + `reeve-infra` (wiring at startup only; anything beyond delegation in a command is a design bug per 03-api).
- `reeve-core` → nothing of ours, no `tauri`, no `rusqlite`, no `portable-pty`.

vibe-kanban's ~30-crate workspace is the cautionary ceiling (11 crates were the dead cloud tier); three crates is the floor that keeps the rule physical. New crates require a design decision, not a habit.

### Workspace conventions

- Root `Cargo.toml` declares the workspace, centralizes versions in `[workspace.dependencies]`, and hosts a shared `[workspace.lints]` table (`clippy::all` as warnings, CI runs `-D warnings`).
- Folder name = package name (`crates/reeve-core` ⇒ `package.name = "reeve-core"`).
- `rust-toolchain.toml` pins stable — identical builds on the dev machine and both CI runners.
- Seam-trait **fixtures** (in-memory, deterministic implementations for hermetic domain tests — sortie's pattern) live in `reeve-core` under a `fixtures` cargo feature: they sit next to the traits they implement, any crate pulls them as a dev-dependency with the feature on, and they never reach a production binary.

## Frontend layout: fixed frame, prototype interior

`app/` is a self-contained Vite + React + TypeScript project managed with **pnpm**. Only the frame is design-fixed; everything inside the feature folders is prototype territory, free to churn until the frontend LLD ticket (#17) settles it against prototypes.

```
app/
├── package.json
├── tsconfig.json            # strict: true — non-negotiable
└── src/
    ├── generated/types.ts   # ts-rs output — never hand-edited
    ├── lib/                 # the curated boundary: Tauri invoke/listen bridge + TanStack Query cache
    ├── board/               # five feature folders mirroring the HLD surfaces
    ├── node/
    ├── workspace/
    ├── settings/
    └── preflight/
```

Fixed by this document: `generated/types.ts` (CI-guarded, layer 4 below), `lib/` as the only module that talks to the backend, the five surface folders, and `strict` TypeScript. Not fixed: UI-state library, component structure, styling — deliberately open for prototyping.

## The type contract: ts-rs with an explicit registry

DTOs and error types in `reeve-core` derive `TS` (ts-rs). A dedicated binary — `src-tauri/src/bin/generate_types.rs` — lists every exported type **explicitly** (vibe-kanban's pattern: the registration list makes the frontend contract reviewable as a diff; nothing leaks in by accident) and writes `app/src/generated/types.ts`.

- `cargo run --bin generate_types` — regenerate.
- `cargo run --bin generate_types -- --check` — regenerate to a temp file, diff against the committed one, non-zero exit on drift. This is CI layer 4.

## Ring-rule enforcement: four layers

| # | Layer | Mechanism | Catches |
|---|---|---|---|
| 1 | Crate graph | `Cargo.toml` dependency lists | `tauri::` in core/infra, `rusqlite`/`portable-pty` in core — they don't compile. |
| 2 | Clippy bans | `crates/reeve-core/clippy.toml`: `disallowed-types` / `disallowed-methods` for `std::process::Command`, `std::process::Child`, `tokio::process::Command`, and the `std::fs` entry points | What the graph cannot block because it ships with std: process spawning and file I/O inside core ("core does no I/O" made lintable). |
| 3 | Ring test | `src-tauri/tests/ring_rule.rs` — a plain `#[test]` that walks the workspace sources and fails on `use tauri::` outside `src-tauri/` or `rusqlite`/`portable_pty` outside `reeve-infra` | Dependency drift in a PR (e.g. adding a crate to the wrong `Cargo.toml`); it is the HLD's greppable rule, executable via `cargo test` on any machine — no CI-only shell script. |
| 4 | Type-contract check | `generate_types -- --check` in CI | Frontend types drifting from the Rust source of truth. |

The ring test lives in `reeve-app` because layer 2 bans `std::fs` in core — the enforcement tool must not violate the rule it enforces.

## CI

GitHub Actions, matrix `windows-latest` (Tier 1, NFR-4 — mandatory, never optional) + `ubuntu-latest` (fast feedback). Jobs per matrix leg:

1. `cargo fmt --check`
2. `cargo clippy --workspace -- -D warnings` (picks up the layer-2 bans)
3. `cargo test --workspace` (includes the layer-3 ring test; core tests run with `--features fixtures`)
4. `cargo run --bin generate_types -- --check`
5. `pnpm install && pnpm -C app test && pnpm -C app build` (Vitest on `lib/` — amended by 10-lld-frontend, which pays this line's original debt)

## The full tree

```
reeve/
├── Cargo.toml                 # workspace root
├── rust-toolchain.toml
├── CONTEXT.md
├── app/                       # frontend (see layout above)
├── src-tauri/                 # reeve-app — ring 1
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── main.rs            # composition root
│   │   ├── commands.rs
│   │   ├── events.rs
│   │   └── bin/generate_types.rs
│   └── tests/ring_rule.rs     # enforcement layer 3
├── crates/
│   ├── reeve-core/            # ring 2
│   │   ├── Cargo.toml         # feature: fixtures
│   │   ├── clippy.toml        # enforcement layer 2
│   │   └── src/
│   │       ├── services/      # 9 modules, 1:1 with API areas
│   │       ├── domain/        # context_assembler, board_derivation
│   │       ├── seams/         # traits: TicketSource, WorkspaceProvider, git/index/vault/pty/gh
│   │       ├── dto/           # API DTOs + ApiError (derive TS)
│   │       └── fixtures/      # cfg(feature = "fixtures")
│   └── reeve-infra/           # ring 3
│       └── src/{git, index, vault, pty, gh_client}.rs
├── docs/
└── .github/workflows/ci.yml
```

Module *internals* shown here (file names under `services/`, `seams/`, etc.) are the frame the subsystem LLDs fill in — those documents may reorganize within their crate, never across crates.

## Handovers to the subsystem LLDs

- **06 graph** (#13): `vault`, `index`, `context_assembler` internals; SQLite schema; watcher.
- **07 workspaces** (#14): `git` module, `WorkspaceProvider`, branch-provenance marker, Windows destroy sequence.
- **08 runs** (#15): run registry, `pty`, process groups, log files.
- **09 sources** (#16): `TicketSource` trait signature, `gh_client`, materialized regions.
- **10 frontend** (#17): everything inside the `app/` feature folders, diff viewer, terminal registry — prototype-driven.

Each extends [lld-atlas.html](./lld-atlas.html) with its classes and state machines.

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-26
