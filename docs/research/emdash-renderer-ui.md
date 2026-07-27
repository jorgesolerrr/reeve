# Emdash renderer UI — architecture deep dive

> Companion note to [reference-implementations.md](./reference-implementations.md), feeding the **frontend** LLD ticket (surfaces, query cache, diff viewer, terminal). Produced by code-reading `github.com/generalaction/emdash` (local clone, HEAD = `5ace464`). All paths are repo-relative.

## 0. Headline corrections to common assumptions

| Assumption | Reality |
|---|---|
| Zustand? | **No.** No Zustand, no Redux. State is **MobX 6 + `mobx-react-lite`** (171 renderer files import it) plus React Query for a thin outer ring. |
| `packages/chat-ui/` is React | **It is SolidJS** (`packages/chat-ui/package.json:8` — "Solid-based chat transcript renderer"; deps `solid-js`, `@vanilla-extract/*`, `shiki`). React only owns a mount-point div. |
| `allotment` | **Dead dependency.** Zero imports anywhere in `apps/` or `packages/`. Splits use `react-resizable-panels` (`apps/emdash-desktop/src/renderer/features/tasks/view/task-main-column.tsx:12`). |
| dnd-kit for a board | **There is no kanban board.** dnd-kit is used for sidebar reordering, tab dragging, and terminal-drawer→pane dragging. |
| `@tanstack/react-query` central | It handles ~38 files of "settings-ish" server state. The *hot* data paths (git, PTY, agent transcripts, file tree) bypass it entirely. |

## 1. State management

### 1.1 Four distinct state layers

Emdash deliberately runs **four** parallel mechanisms, each for a different data shape:

1. **MobX store graph** — the primary application model.
2. **React Query** — cold/settings-ish RPC data only.
3. **Live-model mirrors** (`LiveModel` in main ↔ `ModelMirror`+`bindMirror` in renderer) — git status/HEAD, file tree.
4. **Wire replicas** (`ReplicaState`/`ReplicaLog` over a `MessagePort`) — ACP agent sessions, out-of-process.

### 1.2 MobX root store

`apps/emdash-desktop/src/renderer/lib/stores/app-state.ts:10-43` constructs a singleton `AppState`:

```ts
class AppState {
  readonly update: UpdateStore;
  readonly projects: ProjectManagerStore;
  readonly sidebar: SidebarStore;
  readonly snapshots: SnapshotRegistry;
  readonly history: NavigationHistoryStore;
  readonly navigation: NavigationStore;
  readonly sshConnections: SshConnectionStore;
  readonly resourceMonitor: ResourceMonitorStore;
}
export const appState = new AppState();
```

Ownership chain: `ProjectManagerStore` → `ProjectStore.mountedProject` → `TaskManagerStore.tasks: observable.map<string, TaskStore>` (`apps/emdash-desktop/src/renderer/features/tasks/stores/task-manager.ts:140`). Components read via `getTaskStore(projectId, taskId)` / `getTaskManagerStore(projectId)` selector helpers (`features/tasks/stores/task-selectors.ts`) rather than through React context, so there is **no store Provider** — stores are module singletons and components are wrapped in `observer(...)`.

Notably there is **no `<MobXProvider>`**; the 21 `createContext` uses are for React-scoped concerns only: `PaneSizingContext`, `PaneDimensionProvider`, `TaskViewContext`, `ThemeProvider`, `TooltipProvider`, `IntegrationsProvider`, `GithubContextProvider`, `FeatureFlagProvider`, `RightSidebarProvider`, `TerminalPoolProvider`, `WorkspaceLayoutContextProvider` (see `apps/emdash-desktop/src/renderer/App.tsx:94-114`).

### 1.3 `Resource<T>` — the demand-gated fetch primitive

`apps/emdash-desktop/src/renderer/lib/stores/resource.ts:49` is a homegrown MobX SWR container with three strategies:

```ts
export type ResourceStrategy<T, TEventData = void> =
  | { kind: 'demand' }
  | { kind: 'poll'; intervalMs: number; pauseWhenHidden?: boolean; demandGated?: boolean }
  | { kind: 'event'; subscribe: (h: (e: TEventData) => void) => () => void;
      onEvent: 'reload' | ((event: TEventData, ctx: ResourceContext<T>) => void);
      debounceMs?: number };
```

Demand gating uses `onBecomeObserved(this, 'data', …)` / `onBecomeUnobserved` (lines 117, 224, 239) — a fetch only fires when a MobX observer actually reads `.data`, and polling stops when nothing is observing or `document.hidden`. `load()` dedupes concurrent calls via `_inFlight` + `_reloadQueued` (lines 127-159).

⚠️ **Latent footgun** at `resource.ts:87-111`: the `_ctx` object is built with a placeholder getter that returns `null` with the comment `// overridden below`, then patched via `Object.defineProperty`. Works, but a fragile construction.

### 1.4 React Query: setup, keys, invalidation

**Client** is bare-bones — no default options at all:

```ts
// apps/emdash-desktop/src/renderer/lib/query-client.ts:3
export const queryClient = new QueryClient();
```

Mounted at `App.tsx:119`. Every hook therefore has to spell out its own `staleTime`.

**Consumers** (38 files) are exclusively "cold" domains: automations, MCP servers, skills, prompt library, GitHub accounts/PRs, integrations/issues, agent install statuses, app settings, installed fonts, command-palette search, storage settings.

**Key conventions** are inconsistent — three coexisting styles:

- Inline array literals: `['automations', projectId]`, `['automations', 'runs', automationId, 'page', page, statusFilter]` (`features/automations/use-automations.ts:27,115`)
- Exported key-factory objects: `prQueryKeys.list / listFull / filterOptions` (`features/projects/components/pr-view/usePullRequests.ts:14-25`)
- Local `as const` helpers: `statusQueryKey(connectionId) => ['agents','status', connectionId ?? 'local']` and `opKey(op, connectionId)` (`lib/stores/use-agent-installation-statuses.ts:18-24`)

**Invalidation is driven by main-process IPC events**, wired once at bootstrap (`renderer/main.tsx:28-31`):

```ts
wireModelRegistryInvalidation(modelRegistry);
wirePrCacheInvalidation();
wireCommitHistoryInvalidation();
wireExternalLinkRequests();
```

Two flavours:

*Predicate-based invalidation* (`lib/pr-cache-invalidation.ts:6-12`):

```ts
events.on(prSyncProgressChannel, (progress) => {
  void queryClient.invalidateQueries({
    predicate: (query) => shouldInvalidatePrListQuery(query.queryKey, progress),
  });
});
```

with the predicate matching on positional key slots (`lib/should-invalidate-pr-list-query.ts:9-16` — `queryKey[0] === 'pull-requests'`, `queryKey[2] === progress.remoteUrl`). `lib/commit-history-invalidation.ts:9-11` is cruder still: `predicate: (query) => query.queryKey[1] === 'pr-commits'`.

*Live cache patching* (`lib/stores/use-agent-installation-statuses.ts:57-74`) — the IPC payload is a full DTO, so it's written straight in with `setQueryData` rather than triggering a refetch, then the parent list key is invalidated:

```ts
queryClient.setQueryData<AgentInstallationStatus[]>(key, (prev) => { … });
void queryClient.invalidateQueries({ queryKey: AGENTS_METADATA_QUERY_KEY });
```

### 1.5 How main-process state is bridged: `LiveModel` → `ModelMirror`

**Main side.** `packages/core/src/lib/live-model.ts:49` `class LiveModel<T, E>` — a cached, invalidation-driven, demand-gated model. Key semantics from its own docstring (lines 37-48): single-flight recomputes with exactly one trailing queued run; `invalidate()` only marks dirty when there are no subscribers; stale-while-revalidate (a failed recompute keeps the last-good value and pushes nothing). Every emitted value is a `LiveValue<T> = { value, generation, sequence }` (line 11) where `generation` is a process-unique monotonic id (`nextGeneration()` at line 124 uses `max(last+1, Date.now())`).

**Transport.** `apps/emdash-desktop/src/main/core/workspaces/workspace-factory.ts:199-207`:

```ts
unsubscribeGitUpdates = ws.gitWorktree.subscribe((update) =>
  handleGitWorktreeUpdate(workspaceId, update, (emitted) => {
    events.emit(gitWorktreeUpdateChannel, { projectId: context.projectId, workspaceId, update: emitted });
  })
);
```

**Renderer side.** `lib/stores/live/model-mirror.ts:5` `ModelMirror<T>` holds `current: LiveValue<T> | null` as `observable.ref`, and drops out-of-order updates via `MirrorVersion.shouldApply(generation, sequence)` (line 48).

`lib/stores/live/bind-mirror.ts:157` glues subscribe+snapshot together:

```ts
export function bindMirror<T, E = unknown>(opts: BindMirrorOptions<T, E>): MirrorBinding;
// BindMirrorOptions = { mirror, subscribe(push), snapshot(): Promise<Result<LiveValue<T>,E>>, onError?, onUnexpectedError? }
```

`MirrorBindingStatus = 'idle' | 'syncing' | 'live' | 'error'` (lines 6-14), with backoff `RETRY_DELAYS_MS = [1_000, 2_000, 5_000, 10_000, 30_000]` and `ERROR_AFTER_FAILURES = 3` (lines 23-24). A `runId` counter (line 57, 65-69) invalidates in-flight snapshots after `dispose()`.

**Concrete consumer** — `features/tasks/stores/git-worktree-store.ts:33-101`. Two mirrors (`status`, `head`) share one coalesced snapshot RPC and each filter the same broadcast channel:

```ts
bindMirror<GitStatusModel, GitWorktreeSnapshotError>({
  mirror: this.status,
  subscribe: (push) => events.on(gitWorktreeUpdateChannel, (payload) => {
    if (payload.workspaceId === this.workspaceId && payload.update.kind === 'status') {
      push({ value: payload.update.model, sequence: payload.update.sequence, generation: payload.update.generation });
    }
  }),
  snapshot: async () => { const r = await snapshot(); return r.success ? ok(r.data.status) : err(r.error); },
  onError,
})
```

`coalesce()` (`lib/stores/live/coalesce.ts:6`) shares one in-flight promise so both mirrors don't double-fetch. `OptimisticModel<GitStatusModel>` layers optimistic stage/unstage/discard/commit over the mirror (`stores/git-status-optimistic-updates.ts`).

### 1.6 View-state persistence

`lib/stores/snapshot-registry.ts:22-45` — a MobX `reaction` per registered key, `{ equals: comparer.structural, delay: 1000, fireImmediately: false }`, writing through `viewStateCache.set(key, snapshot)` then `rpc.viewState.save(key, snapshot)`. Registered for `'navigation'` and `'sidebar'` in `app-state.ts:34-35`; task views register `task:${taskId}` (read back in `task-manager.ts:483`). Restored in `main.tsx:54-61` before React mounts.

## 2. "Board" / task-list components

**There is no board.** Task lifecycle status exists in the data model (`shared/core/tasks/tasks.ts:47-56`):

```ts
export const taskLifecycleStatuses = z.enum([
  'todo','in_progress','review','done','cancelled','backlog','duplicate','triage',
]);
```

…but it is **never rendered as columns**. `updateTaskStatus` is only reachable via `rpc.tasks.updateTaskStatus` (`main/core/tasks/controller.ts:40`, called from `task-store.ts:226`), mirrored back through `taskStatusUpdatedChannel` (`task-manager.ts:172-183`). In the UI, tasks are grouped only by **`archivedAt`** (Active/Archived tabs) and sorted by four keys. The lifecycle enum is effectively issue-tracker-sync metadata.

### 2.1 Two task surfaces

**(a) Left sidebar tree** — `features/sidebar/sidebar-virtual-list.tsx`

- `@tanstack/react-virtual` `useVirtualizer` with fixed `ROW_HEIGHT = 32`, `overscan: 8` (lines 35, 57-62).
- Rows come from `sidebarStore.sidebarRows` — a MobX `computed` producing a flattened `project | task` list (`features/sidebar/sidebar-store.ts:92-114`):

```ts
export type SidebarRow =
  | { kind: 'project'; projectId: string }
  | { kind: 'task'; projectId: string; taskId: string };
```

  Filters out `type === 'automation-run'`, archived tasks, and pinned tasks (pinned get their own strip via `pinnedSidebarEntries`, line 124).

**(b) Project → Tasks page** — `features/projects/components/task-view/task-list.tsx`

- Also `useVirtualizer`, but dynamic: `estimateSize: () => 60` + `measureElement: (el) => el.getBoundingClientRect().height`.
- Tabs: `Active (n)` / `Archived (n)`; search filter; sorts `SORT_OPTIONS = ['updated-at','created-at','pr-status','unread']` (lines 38-43). `prStatusRank` maps merged→0, open-non-draft→1, closed→2, draft→3, no-PR→4 (lines 49-56). `isUnread` derives from `taskAgentStatus(task) ∈ {awaiting-input, error, completed}` (line 59).
- Multi-select with shift-range, bulk archive/restore/delete, hotkey `deleteSelectedTasks`.

### 2.2 dnd-kit wiring (sidebar reorder)

`sidebar-virtual-list.tsx:194-238`:

```ts
<DndContext
  sensors={useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }))}
  collisionDetection={sidebarCollision}
  measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
  autoScroll={{ threshold: { x: 0, y: 0.18 }, acceleration: 8, interval: 5 }}
  onDragStart onDragMove onDragEnd onDragCancel>
  <SortableContext items={allDndIds} strategy={verticalListSortingStrategy}>
```

- **ID encoding** (lines 242-261): `proj::<projectId>` and `task::<projectId>::<taskId>`, with `parseDndId` doing prefix/`split('::')` decode.
- **Custom collision detection** `sidebarCollision` (lines 266-282): task drags are restricted to droppables in the *same project*; project drags see every row. Tries `pointerWithin`, falls back to `closestCenter`.
- **Drop math** (lines 148-191): `isCursorAbove()` compares raw pointer Y against `over.rect` midpoint (falling back to `active.rect.current.translated`) rather than trusting dnd-kit's ordering, then `arrayMove` → `sidebarStore.setProjectOrder(...)` / `setTaskOrder(projectId, ...)`.
- **`InsertionIndicator`** (line 321) renders a 3px bar into `document.body` via `createPortal` with `position: fixed` and `zIndex: 9999`.

⚠️ **Bug**: the drag handle is spread onto the whole row — `<div ref={setNodeRef} style={combinedStyle} {...listeners}>` (`sidebar-virtual-list.tsx:376`) — but **`attributes` from `useSortable` is never spread** (line 363 destructures only `setNodeRef, transform, transition, isDragging, listeners`). That drops `role`, `aria-roledescription`, `aria-describedby`, and `tabIndex`, so keyboard drag and screen-reader announcements are unavailable. There is also no `KeyboardSensor` registered.

⚠️ **Interaction hazard**: virtualization + drag. `MeasuringStrategy.Always` plus a virtualizer that unmounts scrolled-away rows means `over` can vanish mid-drag; the code partially compensates by resolving indices from `rows` in `onDragEnd` rather than from dnd-kit's `over` index.

### 2.3 Other dnd-kit sites

- `features/tabs/tab-bar/draggable-tab.tsx:1` — `useDraggable` + `useDroppable` per tab.
- `features/tabs/pane-content.tsx:1` — `useDroppable` per pane (drop target for tabs/terminals).
- `features/tasks/terminals/terminal-drawer-sidebar.tsx:1` — `useDraggable` to drag a terminal out of the drawer into a pane.
- `features/tasks/view/task-main-column.tsx:1-9,71-77` — the `DndContext` that owns tab↔pane and terminal↔pane drags, `collisionDetection={pointerWithin}`, discriminating drag kinds via `isTerminalDrawerDragData(event.active.data.current)` (lines 43-68).

## 3. Embedded terminal (xterm.js)

### 3.1 Component/class stack

```
TerminalPtyContent (features/tasks/terminals/terminal-pty-content.tsx)
  └─ PaneDimensionProvider → PaneSizingContextProvider  (owns usePtyPaneResize controller)
       └─ PtyPane (lib/pty/pty-pane.tsx)  — paste/drop/image injection
            └─ usePty(...)  (lib/pty/use-pty.ts) — keybindings, listeners, resize reaction
                 └─ FrontendPty (lib/pty/pty.ts) — owns the xterm Terminal for the session's life
```

Ownership is *outside* React: `PtySession` (`lib/pty/pty-session.ts:14`) owns the `FrontendPty`, so the terminal survives React unmounts. `FrontendPty` docstring (`pty.ts:44-61`) spells this out. Global registries live on the class (`pty.ts:64-66`):

```ts
static readonly all = new Set<FrontendPty>();
static readonly bySession = new Map<string, FrontendPty>();
```

### 3.2 xterm configuration

`lib/pty/pty.ts:88-111`:

```ts
new Terminal({
  cols: 120, rows: 32,
  scrollback: SCROLLBACK_LINES,            // = 100_000  (line 13)
  // NOTE: convertEol must stay false … rewriting bare \n to \r\n corrupts raw-mode TUIs
  fontFamily: buildTerminalFontFamily(),
  fontSize: 13, lineHeight: 1.2, letterSpacing: 0,
  allowProposedApi: true,
  scrollOnUserInput: false,
  linkHandler: { activate: (_e, text) => { … confirmOpenExternalLink(text, …) } },
  theme: buildTheme(theme),
})
```

Addons: only `@xterm/addon-web-links` (line 116). **No `CanvasAddon`/`WebglAddon`** — deliberate, per the comment at line 113-114: *"Keep xterm on its DOM renderer: CanvasAddon repaints the full canvas on resize, which makes panel/sidebar transitions visibly flicker."* **No `@xterm/addon-search`** either — search is hand-rolled in `lib/pty/terminal-search.ts` (builds logical lines by stitching `isWrapped` rows, lines 28-40) with UI in `terminal-search-overlay.tsx`.

An OSC 52 handler forwards clipboard writes to the main process (`pty.ts:129-137`):

```ts
this.terminal.parser.registerOscHandler(52, (data) => {
  const text = decodeOsc52ClipboardData(data);
  if (text === null) return false;
  void rpc.app.clipboardWriteText(text)…; return true;
});
```

Padding is applied to `.xterm` (not the parent) with a comment explaining xterm v6's `getCoordsRelativeToElement` subtracts padding-left/top (`pty.ts:141-151`).

### 3.3 Off-screen host / mount-unmount instead of dispose

`lib/pty/xterm-host.ts:3` `ensureXtermHost()` creates a singleton `<div data-terminal-host>` at `position: fixed; left: -10000px; width:1px; height:1px; z-index:-1`. `FrontendPty.mount(target, targetDims)` (`pty.ts:198`) resizes **before** `appendChild` to avoid a flash, then `requestAnimationFrame(() => t.refresh(0, t.rows - 1))`. `unmount()` (line 221) reparents back to the host, preserving scrollback across tab switches.

⚠️ `mount()` reaches into private state — `(t as unknown as { _isDisposed?: boolean })._isDisposed` (line 210).

### 3.4 IPC channel names and data flow

Channels (`shared/core/pty/ptyEvents.ts`):

```ts
export const ptyDataChannel  = defineEvent<string>('pty:data');   // topic = sessionId
export const ptyExitChannel  = defineEvent<{ exitCode: number; signal?: number }>('pty:exit');
export const ptyInputChannel = defineEvent<string>('pty:input');
```

plus `ptyStartedChannel = defineEvent<{id:string}>('pty:started')` and `ptyKilledChannel = 'pty:killed'` in `shared/events/appEvents.ts:51,73`.

Topic-scoped channels are string-concatenated (`shared/lib/ipc/events.ts:57`): `const channel = topic ? `${event.name}.${topic}` : event.name;` — so the real Electron channel is **`pty:data.<projectId>:<scopeId>:<leafId>`**.

Session IDs are deterministic (`shared/core/pty/ptySessionId.ts:12`):

```ts
export function makePtySessionId(projectId, scopeId, leafId) { return `${projectId}:${scopeId}:${leafId}`; }
```

The docstring explains why (lines 6-9): the renderer can subscribe to `pty:data` **before** calling `startSession`/`createTerminal`, avoiding a round-trip to learn the id.

**RPC surface** (`main/core/pty/controller.ts:24`): `pty.sendInput(sessionId, data)`, `pty.resize(sessionId, cols, rows)`, `pty.subscribe(sessionId)`, `pty.unsubscribe(sessionId)`, `pty.kill(sessionId)`, `pty.stopSession(sessionId)`, `pty.uploadFiles({sessionId, localPaths})`, `pty.persistDroppedBlob`, `pty.persistClipboardImage`.

**Output path.** Main batches at ~60fps and keeps a ring buffer (`main/core/pty/pty-session-registry.ts:13-14, 46-70`):

```ts
const FLUSH_INTERVAL_MS = 16;              // ~60 fps
const RING_BUFFER_CAP = 64 * 1024;         // 64 KB per session
…
pty.onData((data) => {
  buffer += data;
  if (!flushTimer) flushTimer = setTimeout(flush, FLUSH_INTERVAL_MS);
  let rb = (this.ringBuffers.get(sessionId) ?? '') + data;
  if (rb.length > RING_BUFFER_CAP) rb = rb.slice(-RING_BUFFER_CAP);
  this.ringBuffers.set(sessionId, rb);
});
```

`flush()` emits `events.emit(ptyDataChannel, buffer, sessionId)`.

**Scrollback restore** is atomic-by-construction (`controller.ts:54-61` + `registry.subscribe` at line 132):

```ts
subscribe: (sessionId: string) => ok({ buffer: ptySessionRegistry.subscribe(sessionId) }),
// registry.subscribe: snapshot ring buffer + register consumer in ONE synchronous tick
```

Consumed by `FrontendPty.connect()` (`pty.ts:180-191`):

```ts
async connect(): Promise<void> {
  const result = await rpc.pty.subscribe(this.sessionId);
  const historical = result.success ? result.data.buffer : '';
  if (historical) this.terminal.write(historical);
  this.offData = events.on(ptyDataChannel, (data: string) => this.terminal.write(data), this.sessionId);
}
```

Note the restored scrollback ceiling is the **64 KB ring buffer**, not the 100 000-line xterm `scrollback` — the latter only accumulates while the renderer stays attached.

**Input path** is RPC, not events: `use-pty.ts:249` `void rpc.pty.sendInput(sessionId, data)`.

🐞 **Dead code**: `ptyInputChannel` (`pty:input`) has a listener registered in main (`pty-session-registry.ts:94-102`) but **zero emitters in the entire codebase** (verified by grep). It's a vestigial path.

🐞 **Dead state**: `PtySessionRegistry.activeConsumers` is added/deleted in `subscribe`/`unsubscribe`/`register`/`unregister` but **never read** — `flush()` broadcasts unconditionally. Combined with `main/lib/events.ts:6-11` (which sends to *every* `BrowserWindow`), PTY output is broadcast to all windows regardless of whether anyone subscribed.

**Lazy connect.** `PtySession` (`lib/pty/pty-session.ts:41-44`) uses MobX to defer connection until first render:

```ts
onBecomeObserved(this, 'status', () => { if (this.status === 'disconnected') void this.connect(); });
```

### 3.5 Resize architecture (the most intricate part)

Three layers, documented in `lib/pty/pane-sizing-context.tsx:1-13` and `lib/pty/use-pty-pane-resize.ts:1-15`:

1. `PaneDimensionProvider` measures the pane's content region (TabBar excluded) into an observable `PaneDimensionSink`.
2. `usePtyPaneResize(sessionIds, sink, bottomPadding)` converts px→cols/rows and **broadcasts `rpc.pty.resize` to ALL sessions in the pane** (active + background), exposing `controllerDims: IObservableValue<{cols,rows}|null>`.
3. `use-pty.ts:643-653` runs a MobX `reaction` on `paneAtMount.controllerDims.get()` and calls `term.resize(cols, rows)` on the visible grid only.

Cell metrics are seeded by standalone canvas measurement so background PTYs resize before any terminal mounts, then refined by `calibrateCell(width, height)` from a live terminal. A `hasCalibratedRef` guard (`use-pty-pane-resize.ts:92, 144-146`) suppresses backend broadcasts until real calibration lands.

Debouncing is **leading+trailing**, and the reason is documented at length (`lib/pty/resize-scheduler.ts:1-18`):

> *"Firing on the LEADING edge keeps the SIGWINCH the child receives in lockstep with the xterm grid"* — a pure trailing debounce left TUIs drawing against stale dims (**ENG-1577: "Claude Code output overlaps input field, fixed by resizing"**).

`PTY_RESIZE_DEBOUNCE_MS = 60` (`use-pty-pane-resize.ts:37`).

⚠️ `measureAndResize` has a cold-path retry loop capped at 5 × 100 ms because the terminal is opened off-DOM and xterm's font measurement isn't populated (`use-pty.ts:184-189`).

### 3.6 Other terminal quirks

- **DECRQM workaround** for an xterm.js 6.0 bug (`use-pty.ts:347-379`): registers CSI handlers for `$p` / `?$p` and replies `\x1b[<mode>;0$y` manually.
- Font/setting changes are propagated by **DOM `CustomEvent`s on `window`** — `terminal-font-changed`, `terminal-auto-copy-changed`, `terminal-mac-option-is-meta-changed` (`use-pty.ts:621-623`, `use-pty-pane-resize.ts:198`). An escape hatch outside both MobX and IPC.
- Right-click opens a **native** menu via `rpc.app.showTerminalContextMenu({ requestId, selectionText, linkText, x, y })` with a `requestId` correlation token echoed back on `terminalContextMenuActionChannel` (`use-pty.ts:536-571`).
- Duplicate-paste guards `lastDomImagePasteAtRef` / `lastSystemPasteAtRef` with `isNearDuplicatePaste()` (`pty-pane.tsx:143-144, 150, 223`) — two paste paths (DOM `onPasteCapture` and xterm's key handler) racing each other.
- SSH image/file drops upload over SFTP into `.emdash/uploads` rather than the worktree root, with an explicit issue reference: *"Writing to the root left every attached image behind as an untracked file that dirtied `git status` … (#2680)"* (`main/core/pty/controller.ts:161-164`).

## 4. Diff viewer

### 4.1 Library: Monaco — and diffs are computed **client-side**

The workspace diff viewer is Monaco's `IStandaloneDiffEditor`. Crucially, **the main process never produces a unified diff/patch for the editor**. It ships two *whole file contents*; Monaco's built-in diff algorithm does the rest.

`lib/monaco/sticky-diff-editor.tsx:56-60`:

```ts
const editor = m.editor.createDiffEditor(mountRef.current, {
  ...DIFF_EDITOR_BASE_OPTIONS,
  readOnly: !modifiedUriRef.current.startsWith('file://'),
  renderSideBySide: diffStyle === 'split',
});
```

`diffStyle: 'unified' | 'split'` (props at line 15) maps directly to `renderSideBySide`.

### 4.2 The three-model registry

`lib/monaco/monaco-model-registry.ts:73` `class MonacoModelRegistry` maintains up to three `ITextModel`s per file, keyed by URI scheme (docstring lines 52-72):

```
buffer (file://) — writable; user edits + undo stack
disk   (disk://) — read-only mirror of on-disk content; updated by FS watcher
git    (git://)  — read-only snapshot of a git ref
```

URI construction (line 194):

```ts
toGitUri(bufferUri: string, ref: GitRef): string
// file://workspace:abc/src/index.ts + HEAD_REF → git://workspace:abc/HEAD/src/index.ts  (ref percent-encoded)
```

`DiffFileRenderer` picks the pair per diff group (`features/tasks/diff-view/main-panel/diff-file-renderer.tsx:119-140`):

| `diffGroup` | original (left) | modified (right) |
|---|---|---|
| `disk` | `git://…/STAGED` | `file://…` (live buffer) |
| `staged` | `git://…/HEAD` | `git://…/STAGED` |
| `git` | `git://…/<originalRef>` | `git://…/<modifiedRef ?? HEAD>` |
| `pr` | `git://…/<originalRef>` | `git://…/<modifiedRef ?? HEAD>` |

### 4.3 What the main process actually runs (git commands)

**File content fetch** (`monaco-model-registry.ts:356-373`):

```ts
if (ref.kind === 'staged') rpc.workspace.gitWorktree.getFileAtIndex(projectId, workspaceId, filePath);
else                       rpc.workspace.gitWorktree.getFileAtRef(projectId, workspaceId, filePath, gitRefToString(ref));
```

Backed by `packages/core/src/git/git-worktree.ts`:

- `getFileAtIndex` (line 197) → `git show :<relativePath>`
- `getFileAtRef` (line 193) → `repository.readBlobAtRef(ref, path)` (via `cat-file --batch`, see `packages/core/src/git/cat-file-batch.ts`)
- Disk content → `rpc.workspace.files.readFile` (line 284), which returns `{ content, truncated, totalSize }`; when `truncated`, no model is created and status becomes `'too-large'` (lines 296-303).

**Changed-file lists / line counts** — `git-worktree.ts:217-247`:

```ts
const diffArgs = resolved.cached ? ['diff','--numstat','--cached'] : ['diff','--numstat', resolved.ref];
const nameArgs = resolved.cached ? ['diff','--name-status','--cached'] : ['diff','--name-status', resolved.ref];
```

and for commits (`getCommitFiles`, lines 300-318):

```ts
['diff-tree','--root','--no-commit-id','--numstat','-r', hash]
['diff-tree','--root','--no-commit-id','--name-status','-r', hash]
```

Working-tree status uses porcelain v2 with a fingerprint hash (lines 162-175):

```ts
['--no-optional-locks','status','--porcelain=v2','-z', untracked === 'normal' ? '--untracked-files=normal' : '-uno']
```

**Delivery is neither streamed nor chunked** — file contents come back whole over `ipcRenderer.invoke`, with a size cutoff producing `'too-large'`. The only *live* part is invalidation.

### 4.4 Invalidation bridges

The registry is explicitly a pure SWR cache — *"it does not subscribe to any events. Callers must wire external invalidation bridges"* (`monaco-model-registry.ts:63-66`). That's `lib/monaco/invalidation-bridges.ts:28`:

```ts
export function wireModelRegistryInvalidation(registry: MonacoModelRegistry): () => void
```

- `fileChangesChannel` (`files:changes`) → `registry.findDiskUris({workspaceId, filePath})` → `invalidateModel(uri)`; a `'resync'` update invalidates every disk URI for the workspace (lines 30-44). `.git`-path changes are skipped (line 11).
- `gitWorktreeUpdateChannel` → `update.kind === 'status' ? STAGED_REF : HEAD_REF` → invalidate matching git URIs (lines 47-52).
- `gitRepoUpdateChannel` with `update.kind === 'refs'` → invalidate all `refKind: 'branch'` git URIs (lines 54-60).

`applyDiskUpdate` (lines 895-919) implements external-edit conflict handling: if the buffer was dirty and the new disk content differs, the URI is added to `pendingConflicts` (line 917) — deferred until the user tries to save.

### 4.5 Lifecycle, view-state, performance

- **Ref-counted models with 60 s TTL eviction**; re-registering cancels the timer (`unregisterModel`, lines 530-592).
- **Fetch dedup** by `dedupFetch(key, fn)` with key `${projectId}:${workspaceId}:${filePath}:disk|git:<ref>` (lines 136-138, 208-214).
- **Diff view-state preservation** across tab switches, keyed `${originalUri}::${modifiedUri}`, swept when either model evicts (lines 88-90, 573-577, 637-658).
- **Hover prefetch**: `usePrefetchDiffModels(projectId, workspaceId, group, originalRef, modifiedRef?)` pre-warms both sides on hover so clicking is instant (`diff-view/changes-panel/hooks/use-prefetch-diff-models.ts`).
- **Crash-recovery autosave**: `BUFFER_DEBOUNCE_MS = 2000` (line 8) → `rpc.workspace.editor.saveBuffer(...)` on every content change (lines 411-437, 467-497). This block is **duplicated verbatim** between the re-register and first-register paths — a clear refactor candidate.

`StickyDiffEditor` swaps models in-place with a MobX `autorun` that waits for both `modelStatus` entries to be `'ready'` (`sticky-diff-editor.tsx:127-155`) rather than remounting the editor, and disposes stray `inmemory:` models to avoid leaks (lines 122-123, 145-146).

### 4.6 Syntax highlighting — three different highlighters

1. **Monaco** (editor + diff): themes defined in `lib/monaco/monaco-themes.ts`; bootstrap via `@monaco-editor/react`'s `loader.init()` (`lib/monaco/monaco-bootstrap.ts:25`), awaited in `main.tsx:40` before React renders. Leaks a global: `(globalThis as any).__monaco = m` (line 28).
2. **Shiki** (chat transcript): `packages/chat-ui/src/core/highlight/highlighter.ts` uses `createHighlighterCoreSync` + `createJavaScriptRegexEngine`. Only **5 bundled languages**: `SUPPORTED_LANGS = new Set(['typescript','javascript','python','json','bash'])` (line 62), with aliases ts/tsx/js/jsx/mjs. Dual-theme contract documented at lines 9-13 — tokens must carry `--shiki-light`/`--shiki-dark`.
3. **Prism** (markdown renderer): `react-syntax-highlighter` with `oneDark`/`oneLight` (`lib/ui/markdown-renderer.tsx:4-5`) — plus `apps/emdash-desktop/src/renderer/globals.d.ts:1-2` declares it as an untyped module.

### 4.7 A second, unrelated diff renderer inside the chat

Agent edit tool-calls render their own inline diff in Solid: `packages/chat-ui/src/components/rows/tools/diff/diff-lines.ts` implements **Myers shortest-edit-script from scratch, no dependencies** (docstring lines 1-13). API: `computeDiffRows(oldText, newText)`, `countChanges(rows)`, `selectPreview(rows)` (≤12-row window around the first change). It operates on ACP's `old_string`/`new_string` region, not whole files.

## 5. Preload surface, IPC naming, and how agent output reaches React

### 5.1 The preload is 23 lines — five generic functions

`apps/emdash-desktop/src/preload/index.ts` in full:

```ts
contextBridge.exposeInMainWorld('electronAPI', {
  invoke: (channel: string, ...args: unknown[]) => ipcRenderer.invoke(channel, ...args),
  eventSend: (channel: string, data: unknown) => ipcRenderer.send(channel, data),
  eventOn: (channel: string, cb: (data: unknown) => void) => { … returns removeListener closure },
  getPathForFile: (file: File) => webUtils.getPathForFile(file),
  requestWirePort: (channel: string) => requestWirePort({ ipcRenderer, window }, { channel }),
});
```

⚠️ **Security note**: `invoke` and `eventOn` are **unallowlisted** — the renderer can call *any* `ipcMain.handle` channel and listen on *any* channel. Type safety is compile-time only; there is no runtime channel validation at the bridge. (The ACP wire path *does* validate — see 5.4.)

### 5.2 Typed RPC via a recursive Proxy

`shared/lib/ipc/rpc.ts:96` builds a client with no codegen:

```ts
export function createRPCClient<Router extends RouterMap>(
  invoke: (channel: string, ...args: unknown[]) => Promise<unknown>
): IpcClient<Router>
```

`makeChannelProxy` (line 79) accumulates dotted path segments on every property access and calls `invoke(parts.join('.'), ...args)` when invoked. Registration mirrors it — `registerHandlers` (line 45) walks the handler tree and `ipcMain.handle(prefix, …)` at each leaf. So channel names are literally the object path: **`tasks.createTask`, `pty.resize`, `workspace.gitWorktree.getFileAtRef`, `workspace.files.readFile`, `appSettings.get`, `viewState.save`**. Arbitrary nesting depth is supported by one recursive type (`IpcClient<R>`, line 70).

There's a `withSender(handler)` decorator (line 12) that stashes a sender-aware handler behind `Symbol.for('emdash.rpc.senderHandler')`; `registerHandlers` prefers it and passes `event.sender.id`. The public function throws if called directly.

Router namespaces (`main/rpc.ts:38-76`, 36 controllers): `account, agents, legacyPort, app, automations, appSettings, providerSettings, browser, gitRepository, update, pty, resourceMonitor, files, github, integrations, issues, promptLibrary, skills, ssh, storage, projectSetup, projects, previewServers, tasks, conversations, terminals, mcp, telemetry, pullRequests, viewState, search, projectSettings`, plus nested `workspace: { gitWorktree, files, fileTree, editor }`.

The client is instantiated once (`renderer/lib/ipc.ts:19,34`):

```ts
export const rpc = createRPCClient<RpcRouter>(electronAPI.invoke);
export const events = createEventEmitter(createRendererAdapter());
```

### 5.3 Event emitter, topics, and fan-out

`shared/lib/ipc/events.ts` — `defineEvent<TData>(name)` is a phantom-typed marker (`_data?: TData` at line 3). `createEventEmitter(adapter)` (line 17) keeps **one `ipcRenderer.on` listener per channel** and fans out to a JS `Set` of subscribers, pruning the adapter listener when the last subscriber leaves (`maybePrune`, line 40). Subscriber callbacks are wrapped in `try/catch {}` (line 31) — **errors in one handler are silently swallowed**.

Topic = channel suffix (line 57): `${event.name}.${topic}`.

Main-side adapter broadcasts to every window (`main/lib/events.ts:6-11`):

```ts
for (const win of BrowserWindow.getAllWindows()) { if (win.isDestroyed()) continue; win.webContents.send(channel, data); }
```

**Complete channel catalogue** (from `defineEvent` call sites):

| Domain | Channels |
|---|---|
| PTY | `pty:data` (topic=sessionId), `pty:exit` (topic=sessionId), `pty:input` *(dead)*, `pty:started`, `pty:killed` |
| Tasks | `task:created`, `task:deleted`, `task:status-updated`, `task:pr-updated`, `task:provision-progress`, `task:lifecycle-script-status`, `task:provisioned` |
| Conversations | `conversation:changed`, `conversation:created`, `conversation:agent-status-changed` |
| Agents | `agent:session-exited`, `agent-installation-status-updated` |
| Git | `git:repo-update`, `git:worktree-update` |
| FS | `files:changes`, `fs:file-tree-projection` |
| PRs | `pr:sync-progress`, `pr:updated` |
| Automations | `automation:changed`, `automation:run-changed` |
| SSH / preview / projects | `ssh:connection-event`, `preview-server:event`, `project-settings-changed` |
| App/menu | `app:undo`, `app:redo`, `app:paste`, `menu:open-settings`, `menu:check-for-updates`, `menu:undo`, `menu:redo`, `menu:close-tab`, `menu:quit-requested`, `menu:give-feedback`, `window:maximize-changed`, `external-link-open-requested`, `plan:event`, terminal context-menu action, tab-navigation shortcut, browser app shortcut, notification focus-task |
| GitHub | `github:auth-device-code`, `github:auth-success`, `github:auth-error`, `github:accounts-changed` |
| Updates | `update:checking / available / not-available / downloading / progress / downloaded / installing / error` |
| Resource monitor | `resource-monitor:snapshot` |

**Batching/throttling inventory** (there is no generic batching layer; each site rolls its own):

| Site | Mechanism |
|---|---|
| `main/core/pty/pty-session-registry.ts:13,63` | 16 ms output coalescing per session + 64 KB ring buffer |
| `lib/pty/resize-scheduler.ts` | leading+trailing debounce, 60 ms |
| `packages/core/src/lib/live-model.ts:194-201` | `scheduleDebounced()` (`debounceMs`, default 0/next-tick) + `revalidateIntervalMs` |
| `lib/stores/snapshot-registry.ts:35` | MobX reaction `delay: 1000` + structural equality |
| `lib/monaco/monaco-model-registry.ts:8` | 2 s buffer autosave debounce |
| `packages/wire/src/live/state/batched-live-state.ts:22,28` | `microtaskScheduler` (default) / `timerScheduler(ms)`, Immer-coalesced patches |
| `features/conversations/acp/acp-chat-store.ts:591-605` | 300 ms prompt-draft write debounce |
| `lib/stores/resource.ts` | `debounceMs` on `event` strategy; `pauseWhenHidden` polling |
| `use-pty.ts:519` | 150 ms auto-copy-on-selection debounce |

### 5.4 Agent (ACP) output — a second, *non-IPC* transport

The most interesting design decision: **agent transcript state does not travel over `ipcRenderer`.** It runs over a `MessagePort` to a **separate utility process**, using the `@emdash/wire` protocol.

**Handshake.** The only preload involvement is `requestWirePort(channel)`. Renderer (`renderer/lib/acp/runtime-client.ts:4,20-30`):

```ts
const ACP_WIRE_CHANNEL = 'acp-wire';
async function createAcpRuntimeClient(): Promise<AcpRuntimeRpcClient> {
  const portPromise = awaitWirePort(window, { channel: ACP_WIRE_CHANNEL });
  await window.electronAPI.requestWirePort(ACP_WIRE_CHANNEL);
  const port = (await portPromise) as DomPortLike;
  return client(acpApiContract, connect(domPortTransport(port)));
}
```

Main (`main/core/acp/runtime-process/host.ts:112-130`) spawns a lazy worker (`lazyWorker` from `@emdash/wire/worker`, entry `desktopWorkerPath('acp')`) and exposes it:

```ts
rendererWireDispose = exposeWireToWindows(
  { ipcMain, createMessageChannel: () => { const c = new MessageChannelMain(); return { port1: c.port1, port2: c.port2 }; } },
  controller, { channel: ACP_WIRE_CHANNEL });
```

The forwarded controller is wrapped in `withValidation(acpApiContract, …, runtimeWireValidationPolicy())` where the policy is `import.meta.env.DEV ? 'full' : 'inputs'` (line 133) — full schema validation in dev, inputs-only in prod.

Two security shims sit in the main-process choke point:

- `withProviderEnv` (line 67) — *"Spawn env must originate solely from the trusted main-process settings. Overwrite (never merge) any env supplied by the renderer-facing caller so the renderer cannot inject variables such as PATH/HOME/proxy vars"*.
- `withSessionIdPersistence` (line 82) — persists returned `sessionId` to the DB.

There's an analogous `getAgentConfigRuntimeClient()` for a second worker (`renderer/lib/agent-config/runtime-client.ts`).

**Wire protocol** (`packages/wire/src/api/protocol.ts:119-133`) — 14 message kinds:
`call | snapshot | attach | detach | cancel | result | update | topic-gap | topic-error | blob-pull | blob-chunk | blob-end | blob-error | blob-close`. Error codes (line 4): `CANCELLED, DISCONNECTED, UNKNOWN_PROCEDURE, UNKNOWN_TOPIC, NOT_FOUND, MISSING_HANDLER, CONTRACT_MISMATCH, ALREADY_EXISTS, HANDLER_ERROR`. Blob channels implement **credit-based flow control** (`blob-pull` carries `credit: number`) for attachment upload/download.

**Live replicas.** `renderer/lib/acp/acp-live-session.ts:52` `class AcpLiveSession` holds seven independent topic subscriptions:

```ts
readonly sessionState: ReplicaState<SessionState>;
readonly config:       ReplicaState<sessionConfigState>;
readonly usage:        ReplicaState<sessionUsage | null>;
readonly plan:         ReplicaState<planState | null>;
readonly activeTurn:   ReplicaState<transcriptTurn | null>;
readonly draft:        ReplicaState<promptDraft | null>;
readonly terminals:    ReplicaState<TerminalState[]>;
private readonly terminalLogs = new Map<string, ReplicaLog>();
```

each created via `createReplicaState(client.session.state({conversationId}, 'activeTurn'), schema)` with `store: createImmutableMobxStore()` (line 265) — so wire patches land as MobX-observable immutable snapshots.

`ReplicaState` (`packages/wire/src/live/replica/state.ts:25`) is a `LiveFollower` over `snapshot()` + `attach(update)`, with `onReattach: () => void this.refresh()` (line 58), Zod validation, `LiveCursor {generation, sequence}` ordering, and `waitForMutation(mutationId)` for read-your-writes (line 115).

`ReplicaLog` (`packages/wire/src/live/replica/log.ts:25`) is the **append-only streaming** primitive used for agent terminal output — `onReset(data)` / `onAppend(chunk)` with a byte `writtenOffset`, backed by `createMobxLogStore()` (`acp-live-session.ts:232`).

Server-side coalescing is `BatchedLiveState` (`packages/wire/src/live/state/batched-live-state.ts:46`): N mutations in a scheduler window are folded into **one** `server.produce()` so Immer emits a single minimal patch. The docstring (lines 32-45) gives the examples — rename-then-delete-parent collapses to one remove op. `snapshot()` flushes first so a client seeding from it can never receive a patch with a stale `baseSequence` (lines 97-106).

**Into React.** `AcpChatStore` (`features/conversations/acp/acp-chat-store.ts:61`) is a MobX store bridging replicas → `ChatState`:

```ts
// line 561
const disconnectChatSession = connectSession(this.chatState,
  { activeTurn: asValueSource(session.activeTurn),
    plan:       asValueSource(session.plan),
    sessionState: asValueSource(session.sessionState) },
  { onTurnCommitted: () => void this._refreshHistory() });
```

`asValueSource(replica)` (`acp-live-session.ts:34`) adapts `ReplicaState` to `{ getSnapshot, subscribe }`. `connectSession` (`packages/chat-ui/src/state/chat-state.ts:314-356`) pushes each source into Solid signals: `state.session.setPermissions`, `setPlan`, `state.transcript.activeTurn.set(turn)`, `setTerminalOutputs`.

History is **paged, not streamed**: `clientSession.getHistory(undefined, 100)` at bootstrap (line 426) and again on every `onTurnCommitted` (`_refreshHistory`, line 639) — i.e. the *active* turn streams via replica patches, and once committed the whole 100-turn history is re-fetched and re-seeded.

Agent terminal output binding: `bindSessionTerminalOutputs(session, setTerminalOutput)` (`acp-terminal-output-binding.ts:14`) reconciles the terminal id set on every change and calls `binding.onAppend(syncOutput)` where `syncOutput = () => setTerminalOutput(id, binding.text())` — note this re-reads the **entire log text** on each append rather than appending incrementally (line 49-51). Fine for short tool output, O(n²) for long-running commands.

**Solid↔React seam.** `renderer/lib/chat/chat-transcript.tsx` — the docstring (lines 1-23) explains the design: *"Uses React.createElement (no JSX) to avoid dual-JSX-runtime conflicts."* React renders a bare `<div ref>`, then a dynamic `import('@emdash/chat-ui')` calls `createChatView({ context, state, parent: ref.current, … })` (lines 81-108). Props that would otherwise go stale are pushed through `propsRef` wrappers and imperative setters (`setModel`, `setContentPadding`, `setCommands`, lines 120-134).

### 5.5 PTY-based (non-ACP) conversations

Older/TUI agents run as plain PTYs. Their status is derived in main (from agent hooks) and pushed as `conversation:agent-status-changed` with `AgentStatus = 'idle' | 'working' | 'awaiting-input' | 'error' | 'completed'` (`shared/core/agents/agentEvents.ts:5`). `ConversationManager` (`features/conversations/conversation-manager.ts:110-157`) subscribes to four channels and manually filters by `taskId`, then rolls them up into `taskStatus` (line 159) with unread precedence: unseen `awaiting-input` > `error` > `completed` > `working`.

## 6. Bugs, hacks and TODOs worth flagging

**Confirmed dead code**

1. `pty:input` channel — listener in `main/core/pty/pty-session-registry.ts:94-102`, **zero emitters** repo-wide.
2. `PtySessionRegistry.activeConsumers` — written in 4 places, **never read**; `flush()` broadcasts unconditionally.
3. `allotment@^1.20.5` in `apps/emdash-desktop/package.json` — **zero imports**. So are `mermaid`, `react-zoom-pan-pinch`, `s3mini` at least in the renderer paths traversed (allotment verified conclusively).

**Doc/impl drift**

4. `shared/core/agents/agentEvents.ts:47` says *"Emitted when an agent PTY session exits. **Topic = taskId**."* — but neither the emitters (`main/core/conversations/impl/local-conversation.ts:221`, `ssh-conversation.ts:198…253`) nor the subscriber (`conversation-manager.ts:131`) pass a topic. Every window receives every agent exit and filters in JS.
5. `renderer/lib/chat/shared-chat-context.ts:9-13` says *"Call once from the renderer bootstrap (main.tsx) so the context's font-load hook fires at startup rather than on first conversation open."* — **`initSharedChatContext` is never called from `main.tsx`** (verified by grep: only its own definition and the lazy fallback at line 33). The startup font preload never happens; the cost is paid on first conversation open.
6. `use-pty-pane-resize.ts:165` uses `// eslint-disable-next-line react/exhaustive-deps` while the rest of the file (and repo) uses `// oxlint-disable-next-line` — the linter is oxlint, so that suppression is inert.

**Accessibility**

7. `sidebar-virtual-list.tsx:363,376` — `useSortable`'s `attributes` are never spread onto the node, and no `KeyboardSensor` is registered. Drag-and-drop reordering of projects/tasks is mouse-only and unannounced.

**Duplication / maintenance**

8. `monaco-model-registry.ts:411-437` and `467-497` — the `onDidChangeContent` handler (dirty tracking + 2 s autosave) is duplicated verbatim across the re-register and first-register paths.
9. Three separate syntax highlighters (Monaco, Shiki, Prism) with three theme systems.
10. Shiki bundle covers only 5 languages (`highlighter.ts:62`); everything else renders unhighlighted in the chat transcript.

**Documented workarounds (all with rationale in-code — good practice, but worth knowing)**

11. `use-pty.ts:347-379` — DECRQM handler for an xterm.js 6.0 bug.
12. `pty.ts:91-95` — `convertEol` must stay `false` or raw-mode TUIs corrupt.
13. `pty.ts:113-114` — Canvas renderer deliberately disabled (flicker on pane transitions).
14. `resize-scheduler.ts:1-18` + `use-pty.ts:634-641` + `use-pty-pane-resize.ts:152-154` — the leading-edge resize design exists solely to fix **ENG-1577**.
15. `App.tsx:36-50` — onboarding step list is *frozen* after first resolution because query refetches mid-onboarding would unmount active step components.
16. `sidebar-virtual-list.tsx:64-65, 76-82` — two carefully-worded effects preventing "collapse immediately re-expands" and "unrelated task deletion yanks scroll position".

**Open TODOs** (11 total in the renderer; the dominant cluster is one refactor)

- `TODO(conversations-extraction)` × 8 — `acp-chat-panel.tsx:26,31`, `acp-chat-store.ts:20`, `conversation-manager.ts:3`, `conversations-panel.tsx:5,7`, `create-conversation-modal.tsx:5`, `sidebar-conversations-list.tsx:7,9`. All say the same thing: the conversations feature illegally imports task stores instead of receiving injected scope, blocking its extraction into a standalone package.
- `features/tasks/editor/pane-selectors.ts:46` — `// TODO: remove once all callers are updated`.

## 7. One-paragraph summary

Emdash's renderer is a **MobX-first** Electron UI where React is essentially a rendering shell: a module-singleton store graph (`appState`) holds all domain state, components are `observer()`-wrapped, and React Context is reserved for layout/theme plumbing. Three data-freshness strategies coexist by data shape — React Query for cold RPC state (invalidated by IPC-event bridges wired once in `main.tsx`), a bespoke `LiveModel`→`ModelMirror` mirror protocol with generation/sequence ordering for git and FS state, and a full `MessagePort`-based wire protocol (`@emdash/wire`) with Immer-patch replicas for agent sessions running in a separate utility process. The preload is a deliberately minimal 5-function generic bridge; all typing is achieved through a recursive `Proxy` RPC client whose channel names are literally the router's object paths. The terminal keeps xterm instances alive outside React in an off-screen host div with a 64 KB main-side ring buffer for atomic scrollback restore, and a three-layer resize controller that broadcasts to background PTYs on a leading-edge debounce. The diff viewer never receives a patch — the main process ships two whole file blobs (`git show :path` / `cat-file`) into a ref-counted three-scheme Monaco model registry and lets Monaco compute the diff, while the chat transcript renders its own dependency-free Myers diff in SolidJS. There is no kanban board; the task lifecycle enum exists but is never surfaced as columns.
