# Emdash issue-tracker integrations — code deep dive

> Companion note to [reference-implementations.md](./reference-implementations.md), feeding the **sources** LLD ticket (TicketSource, gh_client, materialized regions). Produced by code-reading `github.com/generalaction/emdash` (local clone). All paths are repo-relative.

## Architecture in one paragraph

There are **two orthogonal plugin kinds**, defined in `packages/plugins/src/`:

- **Integration plugins** (`integrations/`) own *identity + auth* for an external service. `packages/plugins/src/integrations/plugin.ts:11-19` — capabilities `{ auth }`, assets `{ icon }`, metadata `{ id, name, description, websiteUrl }`.
- **Issues plugins** (`issues/`) own the *issue capability* and carry **no identity of their own** — they reference an integration by `integrationId` and the host resolves credentials through that integration's account scope. `packages/plugins/src/issues/plugin.ts:5-21`.

The Electron main process is the "host": it persists credentials, injects them into plugin calls, and short-circuits with a uniform not-connected error when none exist (`packages/plugins/src/integrations/host.ts:22-31`).

## 1. Which trackers are supported

**12 integrations**, registered in `packages/plugins/src/integrations/registry.ts:18-33` and mirrored 1:1 in `packages/plugins/src/issues/registry.ts:30-45`:

| id | Integration def | Issues plugin | `requiredInputs` | `getIssue` (rich context) |
|---|---|---|---|---|
| `github` | `integrations/impl/github/index.ts` | `issues/impl/github/index.ts` | `['repositoryUrl']` | ❌ |
| `linear` | `integrations/impl/linear/index.ts` | `issues/impl/linear/index.ts` | none | ✅ (`:57`) |
| `jira` | `integrations/impl/jira/index.ts` | `issues/impl/jira/index.ts` | none | ❌ |
| `gitlab` | `integrations/impl/gitlab/index.ts` | `issues/impl/gitlab/index.ts` | `['repositoryUrl']` | ❌ |
| `plane` | `integrations/impl/plane/index.ts` | `issues/impl/plane/index.ts` | none | ✅ (`:81`) |
| `forgejo` | `integrations/impl/forgejo/index.ts` | `issues/impl/forgejo/index.ts` | `['repositoryUrl']` | ❌ |
| `trello` | `integrations/impl/trello/index.ts` | `issues/impl/trello/index.ts` | none | ✅ (`:92`) |
| `asana` | `integrations/impl/asana/index.ts` | `issues/impl/asana/index.ts` | none | ❌ |
| `monday` | `integrations/impl/monday/index.ts` | `issues/impl/monday/index.ts` | none | ✅ (`:74`) |
| `notion` | `integrations/impl/notion/index.ts` | `issues/impl/notion/index.ts` | none | ✅ (`:87`) |
| `featurebase` | `integrations/impl/featurebase/index.ts` | `issues/impl/featurebase/index.ts` | none | ❌ |
| `plain` | `integrations/impl/plain/index.ts` | `issues/impl/plain/index.ts` | none | ✅ (`:67`) |

Each integration dir holds `index.ts` (plugin def + `auth.verify`), `client.ts` (SDK construction + `verify*Credentials`), `types.ts` (zod credential schema), `icon.ts`. Each issues dir holds `index.ts` (list/search/get) + `mapper.ts` (provider → canonical `IssueData`); Linear/Monday/Notion/Plain/Plane/Trello also have `context.ts` (+ `queries.ts`) for the markdown context blob.

SDKs are real vendor clients (`packages/plugins/package.json:42-62`): `@octokit/rest`, `@linear/sdk`, `jira.js`, `@gitbeaker/rest`, `@llamaduck/forgejo-ts`, `@makeplane/plane-node-sdk`, `trello.js`, `asana`, `@mondaydotcomorg/api`, `@notionhq/client`, `featurebase-node`, `@team-plain/graphql`.

The canonical issue shape is `IssueData` in `packages/plugins/src/issues/types.ts:6-18`; `IssueDetail = IssueData & { context?: string }` (`:47-49`), where `context` is explicitly "provider-specific enrichment (comments, activity, linked docs) formatted as a markdown context string for agent prompts".

Capability behavior contract, `packages/plugins/src/issues/capabilities/issues.ts:27-34`:

```ts
export type IIssuesBehavior = {
  listIssues(host: ConnectedIntegrationHostContext, opts: IssueQueryOpts): Promise<IssueListResult>;
  searchIssues(host: ConnectedIntegrationHostContext, opts: IssueSearchOpts): Promise<IssueListResult>;
  getIssue?(host: ConnectedIntegrationHostContext, opts: IssueGetOpts): Promise<IssueGetResult>;
};
```

**Note — GitHub gets special-cased.** `apps/emdash-desktop/src/main/core/integrations/integration-payload-builder.ts:17-23` has a documented exception:

```ts
const tags = issuesPluginRegistry.get(integrationId) ? ['issues'] : [];
// GitHub's PR/repository support has no plugin type yet; keep the tags as a
// documented exception until those capabilities exist.
if (integrationId === 'github') tags.push('pullRequests', 'repositories');
```

## 2. Auth per tracker, and where tokens live

### Declared auth methods

The descriptor union is in `packages/plugins/src/integrations/capabilities/auth.ts:37-42`: `form` | `oauth` | `oauth-device` | `cli-import`.

**GitHub is the only non-`form` integration** (`integrations/impl/github/index.ts:14-24`):

```ts
auth: { methods: [
  { kind: 'oauth', providerId: 'github' },
  { kind: 'oauth-device', clientId: 'Ov23ligC35uHWopzCeWf', scopes: ['repo','read:user','read:org'] },
  { kind: 'cli-import', cli: 'gh' },
]}
```

The OAuth **client ID is hardcoded in source** (public client, expected for device flow, but worth flagging).

Everything else uses `kind: 'form'` with secret fields:

- **Linear** — `apiKey` (`linear/index.ts:16-29`)
- **Jira** — `siteUrl` + `email` + `apiToken` (basic auth) (`jira/index.ts:16-41`)
- **GitLab** — `instanceUrl` (default `https://gitlab.com`) + `apiToken` PAT, `read_api` scope (`gitlab/index.ts:16-35`)
- **Forgejo** — `instanceUrl` + `apiToken` (`forgejo/index.ts:16-34`)
- **Plane** — `apiBaseUrl` + `workspaceSlug` + `apiKey` (`plane/index.ts:16-42`)
- **Trello** — `apiKey` + `apiToken`; **`apiKey` is declared `secret: false`** so it renders as a plaintext input (`trello/index.ts:20-25` vs modal `type={field.secret ? 'password' : 'text'}` in `apps/emdash-desktop/src/renderer/features/integrations/integration-setup-modal.tsx:84`)
- **Asana** — `accessToken` PAT; Monday — `apiToken`; Notion — `apiToken`; Featurebase — `apiKey`; Plain — `apiKey`

### The three GitHub auth paths (all real, all wired)

1. **Device flow** (`@octokit/auth-oauth-device@^8.0.3`) — `apps/emdash-desktop/src/main/core/github/services/github-device-flow-service.ts`. `createOAuthDeviceAuth` is the default factory (`:119`); the clientId/scopes are read back **out of the plugin descriptor** at instance construction (`services/github-device-flow-service-instance.ts:11-20`, throws if the descriptor is missing). Verification code is pushed to the renderer over `githubAuthDeviceCodeChannel` (`:59-64`). Cancellation via `AbortController` (`:38`, `:111-116`). Stored with `credentialSource: 'device_flow'` (`:85-95`).

2. **Emdash-hosted OAuth** — `apps/emdash-desktop/src/main/core/account/services/account-oauth-client.ts` talks to `https://auth.emdash.sh` (`core/account/config.ts:1-6`), 5-minute default timeout. The exchanged provider token is dispatched through a registry (`core/account/provider-token-registry.ts:25-43`) to `GitHubAuthServerAdapter.storeOAuthToken` (`core/github/accounts/github-auth-server-adapter.ts:10-37`), stored with `credentialSource: 'emdash_oauth'`. `linkProviderAccount` **hard-rejects anything but github** (`core/account/services/emdash-account-service.ts:127-134`).

3. **`gh` CLI import** — `core/github/accounts/github-cli-account-import.ts:87-100`:

```ts
await this.ctx.exec('gh', ['auth', 'status', '--json', 'hosts', '--show-token'], { timeout: 5_000 })
```

Parses every host entry with `state === 'success'`, verifies each token via `getAuthenticatedUser`, stores with `credentialSource: 'cli'`. **This runs unattended at every app launch** — `apps/emdash-desktop/src/main/index.ts:209-216` calls `githubAccountReconciliationService.reconcileAtStartup()`, which is KV backfill → legacy-token backfill → **`importCliAccounts()`** (`core/github/accounts/github-account-reconciliation.ts:38-47`). So Emdash silently copies your `gh` tokens into its own DB on startup.

### Where tokens are stored

**Single store for everything: Electron `safeStorage` → base64 → SQLite `app_secrets` table.** There is **no keytar** (not in any `package.json`), no OS keychain item per credential.

`apps/emdash-desktop/src/main/core/secrets/encrypted-app-secrets-store.ts`:

```ts
async setSecret(key, secret) {
  this.assertSecureStorageAvailable();
  const encryptedSecret = this.safeStorageApi.encryptString(secret).toString('base64');
  await this.setEncryptedSecret(key, encryptedSecret);   // :29-34
}
```

`assertSecureStorageAvailable` (`:51-66`) throws when `isEncryptionAvailable()` is false, **and additionally throws on Linux if `getSelectedStorageBackend() === 'basic_text'`** — i.e. it refuses to write plaintext-equivalent secrets. Paired with `apps/emdash-desktop/src/main/app/linux-secret-storage.ts`, which forces Chromium's `gnome-libsecret` backend on Hyprland/sway/i3/dwm sessions (where Chromium would otherwise fall back to `basic_text` and break every secret feature — references issue #1875); KDE is excluded so kwallet auto-detection still works.

**Account metadata** lives in `provider_accounts` (`apps/emdash-desktop/drizzle/0019_eager_meteorite.sql`, schema `src/main/db/schema.ts:478-499`):

```sql
CREATE TABLE `provider_accounts` (
  `id` text PRIMARY KEY NOT NULL, `provider_id` text NOT NULL, `account_id` text NOT NULL,
  `credential_ref` text NOT NULL, `is_default` integer DEFAULT false NOT NULL,
  `meta` text, `created_at` integer NOT NULL, `updated_at` integer NOT NULL );
CREATE UNIQUE INDEX idx_provider_accounts_provider_account ON provider_accounts (provider_id, account_id);
CREATE UNIQUE INDEX idx_provider_accounts_default ON provider_accounts (provider_id) WHERE is_default = 1;
```

The row never holds secret material — only `credential_ref`, a key into `app_secrets` (`0001_skinny_robin_chapel.sql`). Default ref format: `` `provider-credential:${providerId}:${accountId}` `` (`core/provider-accounts/provider-account-registry.ts:49-51`). GitHub accounts use the released legacy key `` `github-account-token:<id>` `` via the `credentialRef` override (`core/github/accounts/github-kv-account-backfill.ts:10-12`), because "an account's credentialRef never changes" (`provider-account-registry.ts:32-35`).

Account-id conventions:

- generic integrations: `` `${host ?? integrationId}:${account.id}` `` when `verify` returned an account, else the literal `'default'` (`core/integrations/integration-connection-service.ts:29-35`, `DEFAULT_INTEGRATION_ACCOUNT_ID = 'default'` at `integration-credential-store.ts:21`)
- GitHub: `` `${host}:${providerAccountId}` `` (`github-accounts.ts:92`)

The **whole credential bag is JSON-stringified and stored as one secret** (`integration-credential-store.ts:119-126`), so non-secret fields (`siteUrl`, `instanceUrl`, `workspaceSlug`) are encrypted alongside the token. `verify` may return a normalized `credentials` record that replaces user input (`capabilities/auth.ts:69-74`) — Asana/Trello use this to bake resolved workspace/board ids in.

**Legacy migration (interesting security detail).** `IntegrationCredentialStore.migrateLegacyOnce` (`:142-169`) lazily migrates from pre-`provider_accounts` locations on first read: secrets from flat keys `emdash-<provider>-token` (`LEGACY_SECRET_KEYS`, `:7-18`) plus **non-secret connection config from a plaintext KV store** (`new KV('jira')`, `KV('gitlab')`, `KV('forgejo')`, `KV('plane')` — `integration-credential-store-instance.ts:11-23`). So historically `siteUrl`/`instanceUrl`/`workspaceSlug` sat unencrypted in KV; tokens were always in the encrypted store. Migration failures are deliberately **not** cached so a transient error can't mask recoverable credentials (`:161-168`) — nice touch.

**Bug/rough edge:** `IntegrationCredentialStore.delete(integrationId)` with no `accountId` nukes **all** accounts (`:128-135`), and `IntegrationConnectionService.disconnect` always calls it that way (`integration-connection-service.ts:47`). The RPC surface (`core/integrations/controller.ts:6-14`) exposes no per-account connect/disconnect at all, and `createPluginIssueProvider` always reads the **default** account (`plugin-issue-provider.ts:32`: `integrationCredentialStore.get(provider)` with no accountId). Multi-account is fully modeled in the registry but only GitHub actually uses it.

## 3. What gets stored locally after import

**Nothing goes into a dedicated issues table.** There is no `issues` table anywhere in `src/main/db/schema.ts`. An imported issue is snapshotted as **JSON into a single column**: `tasks.linked_issue`.

`apps/emdash-desktop/src/main/db/schema.ts:130`:

```ts
linkedIssue: versionedJsonColumn(linkedIssue)('linked_issue'),
```

The stored shape — `apps/emdash-desktop/src/shared/core/linked-issue.ts:8-35` (v0, deliberately unversioned legacy: `defineVersionedSchema().unversioned(v0Schema).build()` at `:47`):

```
provider (enum of the 12 ids), url, title, identifier,
displayIdentifier?|null, description?, context?, branchName?,
status?, assignees?[], project?, updatedAt?, fetchedAt?
```

So **yes — the full issue body is persisted**: `description` holds the raw body (GitHub `issue.body`, GitLab `description`, Forgejo `body`, Jira ADF flattened to text by `flattenAdf` in `issues/impl/jira/mapper.ts:28-41`), and `context` holds the potentially very large enrichment blob (Linear comments + full history — see `issues/impl/linear/context.ts:48-71`). The doc comment is explicit: *"A task's linked issue captures the issue metadata at the time of linking"* (`linked-issue.ts:41-46`).

The main-process → renderer mapping that stamps the snapshot, `core/issues/plugin-issue-adapter.ts:24-40`:

```ts
export function toLinkedIssue(provider: IssueProviderType, issue: IssueDetail): LinkedIssue {
  return { provider, identifier, displayIdentifier, title, url: issue.url ?? '', description,
           context, branchName, status, assignees, project, updatedAt,
           fetchedAt: new Date().toISOString() };
}
```

Secondary storage touched by a linked issue:

- **`search_index`** — identifier + title are folded into the FTS keywords for the task (`core/search/search-service.ts:214-225`).
- **`tasks` write path** — `commitCreateTask` inserts `linkedIssue: params.taskConfig.linkedIssue ?? null` (`core/tasks/operations/createTask.ts:171`); later re-linking goes through `updateLinkedIssue(taskId, issue?)` (`core/tasks/operations/updateLinkedIssue.ts:9-37`), which also fires telemetry `issue_linked_to_task` with `{ provider, project_id, task_id }`.
- **`conversations.config`** — the constructed prompt/hidden context is persisted in the conversation config (`shared/core/conversations/conversation-config.ts:15`, `shared/core/tasks/task-config.ts:6-9`), so the issue text is effectively stored a second time.
- **PRs are entirely separate** — `pull_requests` (PK = `url`), `pull_request_users`, `pull_request_labels`, `pull_request_assignees`, `pull_request_checks`, `project_remotes` from `drizzle/0005_add_pull_requests.sql`. GitHub-only (`provider text DEFAULT 'github'`). Issues get none of this relational treatment.

Telemetry note: `core/telemetry/task-telemetry.ts:17` reads `initialConversation?.initialPrompt` — that prompt embeds the issue context. Worth confirming only lengths/booleans are captured, not the text.

## 4. Sync / refresh behavior

**There are no webhooks anywhere.** The only `webhook` hit in the whole main+plugins tree is a Linear docs URL (`integrations/impl/linear/index.ts:28`).

**Issues: pull-on-demand only, cached in react-query. No background polling, no scheduler.**

`apps/emdash-desktop/src/renderer/features/integrations/use-issues.ts`:

- initial list: `staleTime: 60_000`, limit 50 (`:76`, `:7`)
- search: debounced 300 ms, min length 2, limit 20, `staleTime: 30_000`, `keepPreviousData` (`:9-10`, `:45`, `:109-111`)
- Connection statuses: `rpc.issues.checkAllConnections()` with `staleTime: 30_000` + `refetchOnWindowFocus: true` (`integrations-provider.tsx:65-70`); the integration list itself is `staleTime: Infinity` (`:54-59`). Connect/disconnect invalidate the status query (`:113`).

Main-side, `checkConnection` per provider is wrapped in an 8 s timeout and run in parallel across all providers (`core/issues/controller.ts:25`, `:56-74`, `:110-121`). `checkConnection` calls the plugin's `auth.verify` **on every check** — a live API round-trip per provider per refresh (`core/integrations/integration-connection-service.ts:74`), and it opportunistically re-persists normalized credentials if verify returns them (`:78-83`).

**Refresh of an already-linked issue** is explicit and lazy, only for providers with `getIssue`: `renderer/features/tasks/issue-context/refresh-linked-issue-context.ts:4-19` re-fetches at the moment the user inserts the issue into a prompt (`context-bar/resolve-context-action-text.ts:19`). It **silently falls back to the stale snapshot** on any failure (`:16`). Nothing ever rewrites `tasks.linked_issue` from a refresh — the DB snapshot goes stale forever unless the user re-links.

**Croner is used, but only for automations — not for issue sync.** `croner@^10.0.1` (`apps/emdash-desktop/package.json:71`) drives `core/automations/automation-scheduler.ts` (`ensureNextCronRun`/`markDueCronRunsQueued`). Automation triggers are cron-expression-only — `shared/core/automations/config.ts:6-9` is `{ expr, tz? }`. **There is no "issue created/assigned" trigger.**

**PRs do have real polling** (GitHub only): `core/pull-requests/pr-sync-scheduler.ts` — plain `setInterval` at `INCREMENTAL_SYNC_INTERVAL_MS = 5 * 60 * 1000` (`:20`, `:150-152`), one interval per GitHub remote per open project, started on `projectOpened` and torn down on `projectClosed` (`:34-45`, `:73-91`), plus event-driven single-PR syncs on `task:provisioned` (`:95-105`) and re-sync on remote/settings changes. Initialized at `src/main/index.ts:147`, disposed in `app/shutdown.ts:49`. Octokit clients are cached per `host:accountId` (`core/github/services/octokit-cache.ts:5-22`).

Minor smell: `pr-sync-scheduler.ts:278-279` does dynamic `await import('@main/db/schema')` / `await import('drizzle-orm')` inside `_findPrNumberForBranch` — presumably a circular-import workaround, everything else imports statically at `:11-12`.

## 5. Issue → task → workspace → agent: the exact path

### Path A — user-driven (create-task modal)

1. **Pick an issue.** `renderer/features/tasks/components/issue-selector/issue-selector.tsx` + `useIssueSearch.ts` → `use-issues.ts` → `rpc.issues.listIssues/searchIssues`.

2. **Main resolves the repo.** `core/issues/controller.ts:76-94` `withResolvedRemote` fills `repositoryUrl` from the project's base remote when the caller didn't supply one:

```ts
const remote = await project.gitRepository.getBaseRemote().catch(() => undefined);
const selectedRemote = opts.remote?.trim() || remote;
```

3. **Provider dispatch.** `core/issues/registry.ts:13-19` — GitHub gets `createGitHubPluginIssueProvider`, everyone else `createPluginIssueProvider`.
   - Generic: loads default-account credentials, injects `{ log, credentials }`, clamps limit (50 list / 20 search, hard cap 500 — `plugin-issue-adapter.ts:5-12`), maps to `LinkedIssue`.
   - GitHub: resolves repo ref, resolves the *project's* GitHub account (`resolveProjectGitHubAuthContext`), pulls a token via `githubApiAuthService.getToken(host, ctx)` and synthesizes `{ accessToken, apiBaseUrl }` as the plugin's credentials (`github-plugin-issue-provider.ts:70-77`) — i.e. GitHub never uses `integration_credential_store`.

4. **Modal state.** Selecting an issue drives three things:
   - **task name** ← `getIssueTaskName` from `issue.branchName` (`create-task-modal/issue-task-name.ts:4-19`)
   - **branch name** ← `resolveTaskBranchName` (`shared/resolveTaskBranchName.ts:12-31`). **Linear is special-cased twice**: its `branchName` is used verbatim, and the random suffix is suppressed for Linear even when the raw branch is used:

     ```ts
     const linearBranchName = linkedIssue?.provider === 'linear' ? linkedIssue.branchName?.trim() : undefined;
     if (linearBranchName) return linearBranchName;
     const shouldAppendSuffix = appendRandomSuffix && !disableRandomSuffix && linkedIssue?.provider !== 'linear';
     ```

     (Provider-specific logic leaking into shared code — the `branchName` field is generic but only Linear populates it.)
   - **prompt context** ← auto-injected. `task-config/initial-conversation-section.tsx:210-219`:

     ```ts
     const defaultIssueContext = useMemo(() => (linkedIssue ? buildIssueContextText(linkedIssue) : null), [linkedIssue]);
     useEffect(() => { state.setIssueContext(includeIssueContextByDefault ? defaultIssueContext : null); }, [...]);
     ```

     A mention chip `(issue:<provider>:<identifier>)` is prepended to the composer (`:249-261`); **deleting the chip clears the injected context** (`:293-303`).

5. **Prompt construction** — `shared/core/issues/issue-context.ts:53-75`:

```ts
export function buildIssueContextText(issue: LinkedIssue): string {
  const parts = [`Provider: ${formatIssueProviderId(issue.provider)}`, `Identifier: ${issue.identifier}`,
                 `Title: ${issue.title}`, `URL: ${issue.url}`];
  if (issue.description) parts.push(`Description: ${normalize(issue.description)}`);
  ...
  let text = parts.join('. ');
  if (issue.context) text += `\nContext:\n${issue.context}`;
  return text;
}
```

⚠️ **`normalize` collapses all newlines in the description to spaces** (`:54`) — a markdown issue body loses every code block, list and paragraph break before it reaches the agent. `issue.context` is appended verbatim, so the enrichment blob keeps its formatting while the body doesn't. This looks like a genuine bug.

6. **Two prompt shapes**, `create-task-modal/build-create-task-params.ts:34-52`:

```ts
...(type === 'acp'
  ? { initialQueue: buildInitialQueue(state) }
  : { initialPrompt: buildFinalPrompt(state.issueContext, state.prompt) }),
```

   - **PTY**: `buildFinalPrompt` = `[issueContext, userPrompt].join('\n\n')` (`initial-conversation-text.ts:7-18`) — the issue text is **visible in the terminal prompt**.
   - **ACP**: `buildInitialQueue` (`:10-32`) keeps the user text as `text` and stuffs issue context + every `@`-mention context into a separate `hiddenContext` field.

7. **Create.** `use-create-task-callback.ts:32-44` → `taskManager.createTask({ taskConfig: { name, linkedIssue, initialStatus, initialConversation }, workspaceConfig })`.

8. **Persist.** `core/tasks/operations/createTask.ts` — `prepareCreateTask` (`:59-149`) resolves the workspace (new worktree / byoi / existing) and builds the conversation insert; `commitCreateTask` (`:157-191`) inserts `tasks` (with `linked_issue`), `workspaces`, `conversations` in one transaction; `finalizeCreateTask` (`:197-212`) emits `conversationCreatedChannel` and, for PTY conversations with a non-empty `initialPrompt`, an agent `start` event (`:26-44`).

9. **Agent start.**
   - **PTY**: `hydrateConversation.ts:48-53` passes `config?.initialPrompt` only on first spawn → `local-conversation.ts:149-158` → **`spillLargePrompt`** (`core/conversations/spill-large-prompt.ts`). Prompts over `MAX_INLINE_PROMPT_CHARS = 16_384` (`:17`) are written to a temp `task-context.md` and replaced with a pointer message (`:23-29`). The doc comment names the exact motivating incident:

     > *"Large prompts (e.g. a Linear issue description plus its full comment/activity context) can blow past OS argument limits and crash the underlying CLI — see ENG-1546, where Kilo Code interpreted the prompt as a path and threw ENAMETOOLONG."*

     Delivery is by **keystroke injection into the TUI** for keystroke-mode agents — `core/conversations/impl/keystroke-injection.ts`: wait for first output, then 800 ms quiet period, hard cap 15 s, then write payload + submit sequence (`:9-10`, `:37-65`).
   - **ACP**: the queue prompt is sent as two separate text blocks, `packages/runtime/src/acp-agents/session/cell.ts:530-538`:

     ```ts
     prompt: [ ...images,
       ...(prompt.text ? [{ type: 'text', text: prompt.text }] : []),
       ...(prompt.hiddenContext ? [{ type: 'text', text: prompt.hiddenContext }] : []) ]
     ```

     Note there is **no delimiter or system framing** — the issue body (attacker-controllable text from a public tracker) is appended as an ordinary user-role text block. Only the `@`-mention path wraps it in `<issue_context provider=… identifier=…>` tags with attribute escaping (`issue-context.ts:77-88`, `escapeXmlAttr` at `:109-115`); the auto-injected linked-issue path does **not**. Prompt-injection surface worth flagging.

### Path B — automation-driven (fully headless)

`core/automations/actions/taskCreate.ts:104+`:
`executeTaskCreate(automation, run, onStepCompleted)` → `buildAutomationInitialQueue` (`:66-102`) builds `hiddenContext` from the stored `automation.taskConfig.taskConfig.linkedIssue` (via `buildIssueMentionContextBlock`) **plus a live `issueController.getIssueContext` fetch for every `@`-mention in the prompt** (`:86-92`) → `prepareCreateTask` → `commitCreateTask` in a tx → `taskService.notifyTaskCreated` → **`taskService.launch(taskId)`** (provisions the worktree, `task-service.ts:129-131`) → `createConversation` → for ACP, `acpClient.startSession({ …, initialQueue })` (`:256-269`). This is the only path where an issue goes to a running agent with zero human input.

### Path C — `@`-mention in a live ACP chat

`renderer/features/conversations/acp/acp-chat-panel.tsx:268-284` — on submit, `buildIssueMentionHiddenContext` resolves each `(issue:provider:id)` token through `rpc.issues.getIssueContext` and attaches the result as `hiddenContext` (`:293-294`, `:305-306`). Failures log a warning and drop the block (`:275-281`).

## Bugs, hacks and TODOs found

The codebase is unusually clean — a repo-wide `TODO|FIXME|HACK` grep over `packages/plugins/src/{integrations,issues}` and `main/core/{integrations,issues,github}` returned exactly **one** hit, and it's a false positive (`plain/index.ts:20`, a Plain thread status literal `'TODO'`). The real issues are behavioral:

1. **Newline destruction in issue descriptions** — `issue-context.ts:54` `normalize` flattens `\r\n` → space for `description` only. Markdown bodies reach the agent as one line.
2. **No injection boundary on auto-injected issue context** — the ACP hidden-context block (`cell.ts:538`) and the PTY `buildFinalPrompt` concatenation ship untrusted tracker text with no delimiter. The mention path *does* wrap in `<issue_context>`; the linked-issue path doesn't. Inconsistent.
3. **`gh` tokens auto-imported at every startup** without user action (`index.ts:209` → `reconcileAtStartup` → `importCliAccounts`, `--show-token`).
4. **Linked-issue snapshots never refresh in the DB** — `refreshLinkedIssueContext` fetches fresh data for the prompt but nothing writes back to `tasks.linked_issue`; and it silently returns the stale snapshot on error (`refresh-linked-issue-context.ts:16`).
5. **Disconnect is all-or-nothing per provider** — `integration-connection-service.ts:47` calls `delete(integrationId)` with no accountId, which hits the `removeAllAccounts` branch (`integration-credential-store.ts:128-135`). Multi-account is modeled but unreachable for non-GitHub providers, which always read the default account (`plugin-issue-provider.ts:32`).
6. **Declarative auth methods that nothing generic consumes** — `kind: 'oauth'` and `kind: 'cli-import'` appear in the schema (`capabilities/auth.ts:21-35`) and in GitHub's descriptor, but a repo-wide grep shows the only reader of any non-`form` method is `github-device-flow-service-instance.ts:13-14` (`oauth-device`). The setup modal only renders `form` methods (`integration-setup-modal.tsx:38-40`, `if (!method) return null` at `:63`) — so a hypothetical second OAuth integration would render an empty connect dialog.
7. **Trello API key marked non-secret** — `secret: false` (default) means it renders as `type="text"` (`trello/index.ts:20-25`).
8. **GitHub can't supply issue context** — `github/index.ts:79-81` registers only `{ listIssues, searchIssues }`, so `supportsIssueContext` is false and no GitHub issue comments ever reach an agent, despite GitHub being the flagship integration. Also `listForRepo` filters PRs client-side after fetching (`:36`), so a repo with many PRs returns fewer than `limit` issues.
9. **Hardcoded OAuth client ID** in `github/index.ts:19`.
10. **Dynamic imports as a cycle workaround** — `pr-sync-scheduler.ts:278-279`.
