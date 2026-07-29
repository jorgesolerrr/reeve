# 09 — LLD: Sources — TicketSource, gh_client, materialized regions

**Status:** Signed off (2026-07-28)
**Ticket:** [LLD: sources subsystem — TicketSource, gh_client, materialized regions](https://github.com/jorgesolerrr/reeve/issues/16)
**Grounded in:** [05-lld-skeleton.md](./05-lld-skeleton.md) · [04-hld.md](./04-hld.md) · [02-domain-model.md](./02-domain-model.md) · [03-api.md](./03-api.md) · [06-lld-graph.md](./06-lld-graph.md) · [reference-implementations.md](../research/reference-implementations.md)
**Visual companion:** [lld-atlas.html](./lld-atlas.html) — View 6.

## Purpose & scope

This document fixes the internals of the source seam: the `TicketSource` trait and its DTOs, the `gh_client` invocation contract, the Materialized Region format and the `rewrite_region` semantics promised by 06, ticket id allocation, the import and curated-refresh flows, and the offer-to-close flow. The offer-to-close *dialog* (when it appears, what it looks like) belongs to 10 (frontend); this document fixes only the operation it invokes and the trigger contract the UI implements.

## Module map

| Module | Crate | Owns |
|---|---|---|
| `seams/source` | `reeve-core` | The `TicketSource` trait, its DTOs (`SourceItemSummary`, `SourceItemDetail`, `SourceComment`, `CloseOutcome`), `SourceError`, and the strategy-factory type. |
| `services/sources` | `reeve-core` | The six operations (03-api): imported-mapping, materialization (id allocation, region rendering, file scaffolding), refresh with compare-and-skip, close orchestration. |
| `gh_client` | `reeve-infra` | The GitHub `TicketSource` implementation over the `gh` CLI: argv construction, exit-code classification, serde parsing. |
| `fixtures/source` | `reeve-core` (`feature = "fixtures"`) | The sortie-style fixture strategy: items from an in-memory JSON document, implementing the full trait — the hermetic test double for the service. |

## The seam cut: remote I/O only

The trait is **pure remote I/O**. Materialization — rendering the region, allocating `T-<n>`, writing the file, auto-committing — lives in `services/sources`, written once for every strategy. This is emdash's lesson at scale (12 providers behind one neutral `IssueData` shape, with the host normalizing before the plugin sees anything): the strategy returns agnostic data; everything that touches the filesystem, ids, or the region format is service code. Adding Linear is two functions and a mapper — zero API operations change, exactly as 03 promises. The trait stays pure with respect to the clock and the disk, so the fixture adapter slots in without friction.

Per the skeleton's seam convention (07, 08), the trait is synchronous (`fn` + `Send + Sync`); the service invokes it on a blocking task.

```rust
pub trait TicketSource: Send + Sync {
    fn list_items(&self) -> Result<Vec<SourceItemSummary>, SourceError>;
    fn fetch_item(&self, external_id: &str) -> Result<SourceItemDetail, SourceError>;
    fn close_item(&self, external_id: &str, comment: Option<&str>) -> Result<CloseOutcome, SourceError>;
}

pub struct SourceItemSummary {
    pub external_id: String,       // GitHub: the issue number as a string
    pub title: String,
    pub state: RemoteState,        // Open | Closed
    pub url: String,
    pub updated_at: String,        // ISO 8601, verbatim from the source
}

pub struct SourceItemDetail {
    pub summary: SourceItemSummary,
    pub body: String,              // verbatim remote Markdown
    pub comments: Vec<SourceComment>,
}

pub struct SourceComment {
    pub author: String,
    pub created_at: String,
    pub body: String,              // verbatim
}

pub enum CloseOutcome { Closed, AlreadyClosed }

pub enum SourceError {
    Auth,            // gh exit code 4, or stderr match on older versions
    NotFound,        // issue or repo gone
    Network,         // connectivity, and the per-call timeout
    BinaryMissing,   // gh not on PATH
    Malformed(String), // JSON that does not parse into the DTO
}
```

Deliberate absences:

- **No `kind()`** — the service knows which strategy it constructed from the config; an introspection method would be redundant.
- **No batch fetch.** `refresh_source` without a `ticketId` makes **N `fetch_item` calls**, one per imported ticket. Accepted: refresh is curated and user-initiated (never background), N is small at single-user scale, and `gh` offers no cheap batch API to wrap anyway.
- **The `imported: ticketId?` field of the API DTO is not here.** The service computes it by crossing `external_id` against the index's source columns (06: `source_kind`, `source_key`, `source_url` on `nodes`) — the strategy never knows what is imported, which keeps the DTO neutral.

## Manual is absence, not a strategy

No `ManualSource` type exists. The trait we cut is pure remote I/O — a manual implementation would have nothing meaningful to do in any of its three methods, and an implementation whose every method is absurd is the classic sign of a forced abstraction. Manual tickets are created by the `tickets` service (`create_ticket`) without touching the `sources` service; "manual" is simply a Ticket with no source reference. "Degenerate strategy" (03) remains domain-model language — it explains why the board treats all tickets identically — not a type in the code.

The "always-present strategy for tests" slot is filled by the **fixture adapter** instead: `fixtures/source`, behind the `fixtures` feature (same pattern as `fixtures::git` from 07), serving items from a JSON document and implementing the full trait.

## Construction and registration

The **match arm lives in `reeve-app`** (composition root). Core defines the factory shape but never sees `gh_client` (ring rule: core does not import infra); the app, which already wires every seam (05), adds one arm per kind:

```rust
// reeve-core::seams/source
pub type SourceFactory = dyn Fn(&SourceConfig) -> Box<dyn TicketSource> + Send + Sync;

// reeve-core::dto — the typed config union (03: discriminated, never opaque JSON)
pub enum SourceConfig {
    Github { repo: String },       // "owner/name"
}
```

"Adding Linear" = one variant in the union (core), one implementation (infra), one arm (app) — zero new operations.

**Config shape** in `.reeve/config.json`: an object keyed by kind, not an array — 03 fixes "at most one GitHub source" in v1, and the **`sourceId` is the kind** (`"github"`):

```json
{ "sources": { "github": { "repo": "jorgesolerrr/reeve" } } }
```

If N-per-kind ever becomes real, ids grow to `github:2` without breaking any operation signature (they take `sourceId: string`). The `origin`-remote auto-detection (02) happens **once, at `configure_source` time**, via the `Git` seam — never inside `gh_client`.

## The `gh_client` contract

- **Binary and auth.** `gh` resolved from PATH on every invocation (no cached path); auth belongs entirely to `gh` — reeve never sees, requests, or stores tokens (NFR-2). `run_preflight` (FR-4.4) checks presence and minimum version: **`gh` ≥ 2.40**, which guarantees `--json` on `issue list`/`view` and the documented exit-code contract.
- **Explicit repo, always.** Every call carries `--repo <owner>/<name>` from the Source config — never the cwd, never `gh`'s own remote detection. Stateless, like every operation (03).
- **The three commands:**

| Trait method | Invocation |
|---|---|
| `list_items` | `gh issue list --repo … --state open --json number,title,state,url,updatedAt --limit 200` |
| `fetch_item` | `gh issue view <n> --repo … --json number,title,state,url,updatedAt,body,comments` |
| `close_item` | `gh issue close <n> --repo … --comment <text>` (comment travels in the close; omitted when empty) |

  `list_items` returns **open issues only**: the list is the catalog of *importables*, and importing a closed issue is not a v1 case. `fetch_item` is where `state: closed` arrives — that is how refresh detects a remotely closed issue. `close_item` is the **only write** this module ever performs (FR-1.2).
- **Error classification** (vibe-kanban's pattern): exit-code first — **4 = `Auth`** — with a stderr string-match fallback for older versions; then `NotFound`, `Network`, `BinaryMissing`, `Malformed`. The enum lives in core; every argv/exit-code mapping lives in `reeve-infra::gh_client`. An `already closed` response to `close_item` is classified as `CloseOutcome::AlreadyClosed` — success, not an error (see offer-to-close).
- **Per-call timeout: 30 s**, surfaced as `Network` — no sources operation may hang the UI (all are awaitable request/response).
- **serde with unknown fields ignored** — `gh` output drift across versions does not break the parser.

## The Materialized Region

### Format

```markdown
<!-- reeve:source:begin -->
> Imported from [jorgesolerrr/reeve#123](https://github.com/jorgesolerrr/reeve/issues/123) · snapshot 2026-07-28 · state: open

<issue title and body, verbatim>

---
**@alice** · 2026-07-20

<comment body, verbatim>

---
**@bob** · 2026-07-21

<comment body, verbatim>
<!-- reeve:source:end -->
```

- **Minimal, fixed markers**: `<!-- reeve:source:begin -->` / `<!-- reeve:source:end -->`, always on their own line, no metadata inside the marker. Coordinates (kind, repo, number, URL) already live in the front-matter (02); duplicating them in the marker would create two sources of truth.
- **The header line** (blockquote) carries the remote link, the **snapshot date**, and the **remote state** seen at the last fetch. This is emdash's snapshot-with-`fetchedAt` rule, placed *inside* the region: it is Source-owned content, rewritten wholesale, and the freshness datum is read identically by the user and the agent (the region travels verbatim into the Context Package). No `AGENTS.md`-style determinism concern applies: the region is a file of truth, not a per-Run derivative.
- **Comments as flat sections** separated by `---`, author + date + verbatim body. No invented headings: the issue body brings its own `#` levels and we do not collide with them.
- **Escape rule — the only transformation on "verbatim" content**: the renderer prefixes with a single space any content line that is byte-equal to a marker line. A body that happens to contain `<!-- reeve:source:end -->` cannot truncate the region. Documented as the sole exception to verbatim copying.
- **Placement**: import creates the file as front-matter → region → nothing. Everything the user adds afterwards lives outside the markers and is local, untouchable by refresh.
- **All comments, no cap**: the assembler's greedy fill (06) already manages the token budget; truncating discussion here would break 02's "the agent reads the full issue, discussion included".

### `rewrite_region` semantics (fixing 06's promised primitive)

1. Scan lines for the **first** line byte-equal to the begin marker, then from there the **first** byte-equal to the end marker. Replace what lies between (exclusive); every other byte of the file is untouched. The write is atomic via the vault's write path; the watcher re-indexes it like any other edit — reeve's own writes re-enter through the same single emission path (06).
2. **Zero markers, begin without end, or end before begin** → `RegionMalformed`: that ticket is **skipped with a warning** in the refresh result (03's warnings-in-result envelope). Reeve never guesses and never re-appends a region: if the user deleted or broke it, reeve reports and does not touch the file. Repair is manual in v1.
3. `rewrite_region` remains a generic primitive on the `Vault` seam (06 owns the trait); the *source* region markers and rendering are `services/sources` policy — a future region owner brings its own markers.

## Ticket id allocation

**A stored counter, not a derived maximum.** Derive-don't-store applies to the *derivable*, and 02's "never reused" is not derivable once a file is deleted — history cannot be reconstructed from the filesystem's present. If ids were `max(existing) + 1`, deleting `T-12.md` (the highest) would let T-12 be reborn: old `[[T-12]]` links in notes and docs would silently point at the impostor, and an orphaned `reeve/T-12` branch could be adopted by it.

- **`.reeve/counters.json`** — `{ "ticket": 12, "epic": 3 }` — committed, bumped on every allocation under the auto-commit policy. It is truth, like `done` in front-matter — not cache (it never enters SQLite, which stays disposable).
- **Allocation rule**: next id = `max(counter, highest id observed in the index) + 1`. The counter cannot desync in the direction that matters — any drift (e.g. a future merge from another machine bringing a higher `T-n`) self-corrects forward. This keeps ADR-0006's law: nothing stored that *can* desync.
- Separate counters for `T-` and `E-`; `import_items` allocates in the order of the array received; allocation is serialized by the service (single process, no race). Counter bump is written before the ticket file — a crash between the two wastes a number, which is harmless by design (never-reuse is the invariant, density is not).

## The import flow (`import_items`)

For each `external_id`, in array order:

1. **Idempotency check**: query the index for `source_key = external_id` (scoped to the source's kind). Already imported → **skip**, reported in warnings with the existing `T-<n>`. No duplicates, ever.
2. `fetch_item` — full detail including comments. A per-item fetch failure is a per-item warning; the batch proceeds.
3. **Allocate** the next ticket id (counter rule above).
4. **Render** the file: front-matter `{ title: <remote title>, source: { kind, key, url } }` + the Materialized Region. No `done`, no `epic`.
5. Write `.reeve/tickets/T-<n>.md` through the vault; the batch auto-commits **once** (`reeve: import T-43 T-44`) under the policy (02).

Result: `{ imported: [{ externalId, ticketId }], skipped: [{ externalId, ticketId }], warnings }`.

**`title` is fixed at import and local-owned thereafter.** Refresh never touches it: if the remote title changes, the change is visible inside the region (Source-owned), but the front-matter is the user's — they may have retitled locally. This is the strict application of "everything outside the markers is local". Consequence: 06's mention of "region refresh bookkeeping" via `patch_front_matter` is **empty in v1** — refresh rewrites only the region (06 amended accordingly).

## The refresh flow (`refresh_source(sourceId, ticketId?)`)

1. Resolve the imported tickets of the source via the index (`source_kind` columns); the optional `ticketId` scopes to one.
2. Per ticket: `fetch_item` → render the region → **byte-compare** against the current region content → `rewrite_region` only if changed. The compare-and-skip keeps auto-commit quiet: a 10-ticket refresh where one changed produces one touched file, not ten.
3. Result on the promise (no events — 03): `{ updated: [ids], unchanged: n, remoteClosed: [ids], warnings }`. Warnings distinguish fetch failures (deleted issue, network) from broken regions.
4. **A remotely closed issue** shows up in the region header (`state: closed`) and in `remoteClosed` for the UI to signal — and triggers **no structural act**: no `done`, no suggestion machinery in the core. The local file is truth; remote state is a datum, not an order (the anti-sortie lesson: remote-as-truth reconciliation is correct for a daemon whose board *is* the tracker, and wrong for reeve).

## The offer-to-close flow

- **Trigger**: an **imported** ticket transitions to `done` — after `merge` (which sets done, 03) *and* after a manual `mark_done` (the push → PR merged outside → done case). This is the full reading of FR-1.2's "offer-to-close on Done", not just the merge case. The chaining is the **UI's**: the core only exposes `close_source_item`; no core path ever closes on its own.
- **Default comment, editable** in the dialog (English, like every artifact): `Resolved via reeve — merged to <base>.` for merge, `Resolved via reeve.` for mark_done. The user may edit or clear it (close without comment).
- **`AlreadyClosed` is success with a warning**, not an error: the remote issue may have been closed by someone else or auto-closed by a PR (`Fixes #n`) — *frequent* in the push flow. `gh_client` classifies it; the result reports it quietly.
- **Real failure** (auth, network): error in the envelope; the local `done` is untouched (it is already truth). **No stored "closed successfully" flag** — derive-don't-store: instead, a "Close remote issue" action remains permanently available in the UI of any imported ticket, so retry is trivial and there is no state to desync.
- **Declining is not remembered**: no persistence of the "no". The ticket is already done; the automatic dialog does not fire again because the transition does not repeat.

## `list_source_items` and the imported-mapping

The service takes the strategy's `SourceItemSummary` list and joins it against the index (`source_key`) to fill `imported: ticketId?` — the single door for initial import and later discovery (03). New remote items simply appear unimported; nothing is ever auto-imported (curated refresh).

## Amendments

- **[06-lld-graph.md](./06-lld-graph.md)**: the front-matter write path listed "region refresh bookkeeping" among `patch_front_matter` callers — empty in v1; refresh rewrites only the region. Annotated in 06.
- **[02-domain-model.md](./02-domain-model.md)**: `.reeve/counters.json` joins the committed repository layout — the id counters are truth (never-reuse is not derivable), not cache.

## Sign-off

- [x] Signed off by Jorge Soler — 2026-07-28
