# Isolating agent work units: worktrees, containers, and hybrids

Research note for **reeve** — a free, open-source, single-user, local-first builder workstation that
runs coding agents on work units in isolated workspaces. Primary OS: **Windows 11 Home**.

Question: *what are the real-world patterns and trade-offs for isolating agent work units — git
worktrees, Docker/devcontainer sandboxes, or hybrids?*

This note is written to stress-test the design, not to decorate it. Every option below has costs that
land on either the user or the maintainer, and the Windows column is where most of them land.

**Bottom line (argued in full at the end):** ship **worktree-default + optional container sandbox**,
where "optional" means *designed for from day one, but not implemented in v1*. Worktrees are the only
mechanism that works on native Windows with no extra install; containers are the only mechanism that
provides an actual security boundary. reeve should be honest that v1 is an *organisational* boundary,
not a *security* boundary.

---

## 1. The three shapes, and what they actually buy

| | Worktree | Container / devcontainer | Hybrid (worktree + container) |
|---|---|---|---|
| Isolates file edits between units | Yes | Yes | Yes |
| Isolates dependencies / toolchain | No (shared host toolchain) | Yes | Yes |
| Isolates runtime (ports, DBs, daemons) | No | Yes (with per-unit compose project) | Yes |
| Security boundary vs. the host | **None** | Real, but leaky | Real, but leaky |
| Cost to create a unit | ~100 ms + dependency install | Image build (minutes first time) + start | Both |
| Works on native Windows, no extra install | **Yes** | No (needs Docker Desktop + WSL2) | No |
| Extra moving parts to maintain | Low | High | Highest |

The honest framing: **a worktree is a concurrency mechanism that people misread as a sandbox.** Git's
own documentation describes worktrees as multiple working trees attached to the same repository,
sharing the object database and most refs ([git-worktree](https://git-scm.com/docs/git-worktree)) —
there is no claim of isolation from the user's machine, because there isn't any. Anthropic's own
sandbox comparison puts worktrees in a different chapter entirely from sandboxing, and lists
containers/VMs as the isolation options
([Choose a sandbox environment](https://code.claude.com/docs/en/sandbox-environments)).

---

## 2. Worktrees in practice

### 2.1 What git gives you for free

Verified first-hand on the author's machine (git 2.45.1.windows.1, NTFS, this repo):

```
$ git worktree add <tmp> -b probe        → elapsed 135 ms
$ cat <wt>/.git                          → gitdir: C:/…/reeve/.git/worktrees/reeve-wt-probe
$ git -C <wt> rev-parse --git-common-dir → C:/…/reeve/.git
$ git -C <wt> rev-parse --git-path hooks → C:/…/reeve/.git/hooks     ← shared with main checkout
$ git -C <wt> config --get core.autocrlf → true                      ← config shared
```

Creation is genuinely cheap for a small repo; for a large one the cost is a full checkout of tracked
files, because git decompresses every file from the object store into the new tree. The object
database is shared, so a second worktree of a 10 GB repo costs one working tree, not another 10 GB of
history.

Per-worktree state: `HEAD`, index, `refs/bisect`, `refs/worktree`. Shared state: objects, all of
`refs/`, **config** (unless `extensions.worktreeConfig` is enabled), and **hooks**
([git-worktree](https://git-scm.com/docs/git-worktree)). That hooks line is the load-bearing one for
the security section below.

### 2.2 Creation: the checkout is the easy 5%

A worktree contains only *tracked* files. Everything in `.gitignore` — `node_modules/`, `.env`,
`.venv`, `dist/`, build caches — does not exist. That is precisely the state a modern project cannot
run in. Practitioner reports converge on 3–10 minutes of setup per worktree for a real monorepo, and
on every orchestrator eventually shipping the same 50–100 line bootstrap script
([Every AI agent tool creates git worktrees…](https://dev.to/rohansx/every-ai-agent-tool-creates-git-worktrees-none-of-them-make-worktrees-actually-work-3ae9)).

The two mitigations that actually work:

- **Declarative copy-in of gitignored files.** Claude Code shipped `.worktreeinclude`, a
  `.gitignore`-syntax file listing gitignored paths to copy into each new worktree
  ([worktrees docs](https://code.claude.com/docs/en/worktrees)). This is the cheapest thing reeve can
  copy: it is a text file and a copy loop, and it removes the single most common "why is this broken"
  report (`.env` missing).
- **A content-addressed package store.** pnpm documents git worktrees + multi-agent development
  explicitly: with `enableGlobalVirtualStore: true`, each worktree's `node_modules` is symlinks into
  one shared store, giving near-zero per-worktree overhead ([pnpm: git worktrees](https://pnpm.io/git-worktrees)).
  Note pnpm's own caveat: a shared writable store assumes all worktrees share a **trust boundary** —
  which is exactly the assumption a security-conscious agent runner should not want to make.

Neither generalises. Python (`.venv`), Go build caches, Rust `target/`, Gradle, and Docker layer
caches each need their own answer. **reeve cannot solve dependency bootstrap in general; it can only
provide a per-project setup hook and get out of the way.** Pretending otherwise is how this feature
becomes a swamp.

### 2.3 Cleanup: where worktrees leak

`git worktree remove` refuses to remove a dirty tree without `--force`, and a locked tree without
`--force --force`. Stale administrative entries survive manual directory deletion until
`git worktree prune`, and automatic pruning is on a 3-month default (`gc.worktreePruneExpire`)
([git-worktree](https://git-scm.com/docs/git-worktree)). So the failure mode is not "loud error", it
is "silent disk accumulation".

Claude Code's design here is worth copying wholesale, because it is the product of hitting the
problem at scale ([worktrees docs](https://code.claude.com/docs/en/worktrees)):

- On exit, inspect the tree for changed files, untracked files, and unpushed commits; auto-remove only
  when clean; prompt otherwise.
- `git worktree lock` the tree **while an agent is running**, so a concurrent sweep can't delete it —
  and release the lock when the process dies, otherwise a killed session leaves a permanently locked
  tree (they shipped that fix in 2.1.210).
- A periodic sweep, bounded by a `cleanupPeriodDays` setting, that never touches user-created trees.
- Refuse to create a worktree on a symlinked path — before 2.1.212, following a committed symlink
  could create files *outside* the repository.

That last item is a warning about the whole category: worktree management is filesystem plumbing, and
filesystem plumbing has sharp edges that only show up under adversarial or messy input.

### 2.4 Ports, databases, and everything that isn't a file

Worktrees isolate files. They isolate nothing else. Multiple units running `pnpm dev` collide on
3000; multiple units running migrations hit the same local Postgres; multiple units running
`docker compose up` collide on the Compose project name (derived from the directory name), on
container names, and on published host ports
([worktree-compose](https://www.worktree-compose.com/), [Docktree](https://github.com/Bnjoroge1/Docktree)).

The known mitigations are all *conventions reeve would have to impose on the user's project*:

- Deterministic port offset per unit (`BASE + index`), injected as env vars the project must read.
- `COMPOSE_PROJECT_NAME` per unit.
- Per-unit database schema or database name.

Each of these requires the project to cooperate. This is the strongest argument *for* containers: a
container gives you runtime isolation without asking the user's `package.json` to change. It is also
the strongest argument for reeve exposing a small, well-documented env contract (`REEVE_UNIT_ID`,
`REEVE_PORT_BASE`) rather than trying to be clever.

### 2.5 Disk

Reported real numbers: five Node worktrees at ~2 GB of `node_modules` each ⇒ ~10 GB; one user
reported ~9.8 GB consumed in 20 minutes
([dev.to](https://dev.to/rohansx/every-ai-agent-tool-creates-git-worktrees-none-of-them-make-worktrees-actually-work-3ae9)).
On APFS, copy-on-write cloning makes duplicating a dependency tree near-free. On NTFS it is not:
NTFS has no CoW file clone. **Windows' equivalent is a Dev Drive (ReFS + block cloning + Defender
performance mode)**, which is opt-in, requires creating a separate volume, and is the only way a
Windows user gets Mac-like cheap duplication
([Dev Drive and copy-on-write](https://devblogs.microsoft.com/engineering-at-microsoft/dev-drive-and-copy-on-write-for-developer-performance/)).
reeve should surface disk usage per unit in the UI and make deletion a first-class action, because
the alternative is the user discovering the problem via a full disk.

---

## 3. Container sandboxes in practice

### 3.1 The spec and the headless path

The [Dev Containers spec](https://containers.dev/) is the interoperable format, and crucially it does
**not** require VS Code: `@devcontainers/cli` provides `devcontainer up` / `devcontainer exec` for
headless use ([Dev Container CLI](https://code.visualstudio.com/docs/devcontainers/devcontainer-cli)).
That matters for reeve, which is a workstation app, not an editor extension. Adopting the devcontainer
spec means reeve reuses configuration users may already have, instead of inventing a `reeve.yaml`.

### 3.2 How the reference implementations actually do it

Two distinct architectures show up, and the difference is the whole ballgame:

**(a) Mount the host repo into the container.** The Claude Code devcontainer bind-mounts the host
repository as the workspace; edits appear immediately in the local repo
([devcontainer docs](https://code.claude.com/docs/en/devcontainer)). Docker's own microVM sandboxes do
the same via filesystem passthrough, deliberately preserving the *same absolute path* inside and
outside so error messages and configs resolve
([Docker Sandboxes architecture](https://docs.docker.com/ai/sandboxes/architecture/)). Simple mental
model; but the workspace is writable from inside the sandbox, so file-level isolation is *not*
provided by the container — you still need a worktree or a branch underneath it.

**(b) Clone the repo into the container.** Sculptor gives each agent its own container with its own
copy of the repo and its own branch; the local repo is untouched until you pull from the container,
and a "pairing mode" checks the agent's branch out locally and mirrors state when you want to
collaborate ([Sculptor announce](https://imbue.com/blog/sculptor-announce)). OpenHands similarly
builds a runtime image and runs the agent in a container it controls, with the workspace supplied via
bind mount or named volume, plus an overlay mode for copy-on-write over a read-only mount
([OpenHands runtime architecture](https://docs.openhands.dev/openhands/usage/architecture/runtime)).
Dagger's `container-use` combines both: each agent gets a git branch *and* a Dagger container, and
results come back as ordinary branches ([container-use](https://github.com/dagger/container-use)).

Note the pattern: **the tools that clone-into-container still need a git-level mechanism to get work
back out.** Containers don't replace the branch/worktree model; they wrap it. Cursor's parallel agents
use worktrees locally and dedicated VMs remotely, and its self-hosted cloud agents exist precisely
because teams wanted the container boundary inside their own infrastructure
([Cursor: self-hosted cloud agents](https://cursor.com/blog/self-hosted-cloud-agents)). Conductor, by
contrast, is worktree-only and macOS-only ([conductor.build](https://www.conductor.build/)).

### 3.3 Startup cost and the caching answer

The naive container-per-task loses three minutes to `pip install` before the agent writes a line.
Imbue's fix is entirely conventional Docker practice — install dependencies in a layer above the
source copy, so agents start from a warm image — and they report going from minutes to seconds
([How we made sandboxed coding agents 10x faster to start](https://imbue.com/blog/containers)).

Their reported limitation is the one that will bite reeve: when an agent modifies the devcontainer
config, the change doesn't apply until a *new* agent starts from a branch containing it. Container
definitions are build-time; agents work at run-time. Any agent task of the form "add a dependency and
run the tests" needs either an in-container package install that is then lost, or a rebuild loop.
There is no clean answer here, only a choice about where the seam is.

### 3.4 Credentials and network

The credential question is where container isolation earns or loses its keep:

- Anthropic's guidance is explicit: don't mount host secrets such as `~/.ssh` or cloud credential
  files; prefer repository-scoped or short-lived tokens; pass cloud credentials as env vars rather
  than mounting files ([devcontainer docs](https://code.claude.com/docs/en/devcontainer)).
- Their reference container adds a default-deny iptables/ipset egress firewall, which requires
  `NET_ADMIN` and `NET_RAW` capabilities — i.e. you weaken the container's capability set to gain
  network control. They note the firewall is optional and you may prefer your own network controls.
- Docker's sandboxes route all egress through a host proxy that enforces policy and injects
  credentials, so the secret never lives in the sandbox
  ([architecture](https://docs.docker.com/ai/sandboxes/architecture/)). This is the better shape and
  the more expensive one to build.
- **Do not mount `/var/run/docker.sock`.** Any process that can talk to the daemon socket can start a
  container mounting the host root filesystem; read-only mounting does not help, because the API is
  the same ([Quarkslab](https://blog.quarkslab.com/why-is-exposing-the-docker-socket-a-really-bad-idea.html)).
  This is a live temptation for reeve, because agents want to run `docker compose up` for their
  project's services.

And the honest ceiling, stated by Anthropic themselves: with `--dangerously-skip-permissions`, a dev
container does not prevent a malicious project from exfiltrating anything reachable inside the
container, including the Claude credentials in `~/.claude`; they recommend using dev containers only
with trusted repositories ([devcontainer docs](https://code.claude.com/docs/en/devcontainer)).
A container is a blast-radius reducer, not a guarantee.

---

## 4. The security model, stated plainly

The realistic threat is not a malevolent model. It is **prompt injection plus supply chain**: the
agent reads attacker-controlled text (an issue body, a dependency's README, a web page, a
`postinstall` script) and takes actions on the host with the user's privileges. This is not
theoretical. In the August 2025 Nx compromise, malicious npm versions invoked locally installed AI
CLIs with `--dangerously-skip-permissions` / `--yolo` / `--trust-all-tools` to enumerate the
filesystem, then harvested GitHub and npm tokens, SSH keys and wallet files and exfiltrated them to a
repo created under the victim's own account
([Socket](https://socket.dev/blog/nx-packages-compromised), [Snyk](https://snyk.io/blog/weaponizing-ai-coding-agents-for-malware-in-the-nx-malicious-package/)).
An agent runner that runs unattended sessions with relaxed permissions is *exactly* the environment
that attack was written for.

### What each option actually contains

**Worktree — no boundary at all.** The agent process runs as the user. It can read `~/.ssh`,
`~/.aws`, the browser profile, and every other repo on the machine; it can `git push --force`; it can
reach any network endpoint. Three worktree-specific escalations deserve naming:

1. **Shared hooks.** Verified above: `rev-parse --git-path hooks` inside a worktree resolves to the
   *main* repo's `.git/hooks`. An agent confined to a worktree can write `post-checkout` or
   `pre-commit` there, and that script executes later in the user's main checkout, outside any
   review. Worktree "confinement" is therefore not even confinement to the worktree.
2. **Shared config and refs.** `.git/config` is shared by default; so is all of `refs/`. An agent can
   set `core.pager`, `core.fsmonitor`, or `alias.*` to arbitrary commands, or delete branches.
3. **Shared credential helper.** Whatever git credential helper the user has configured is available
   to every worktree, so "push to any repo the user can push to" is in scope.

Claude Code's mitigations are permission-layer, not isolation-layer: a working-directory write
boundary, prompt-on-network commands, fail-closed command matching
([Security](https://code.claude.com/docs/en/security)). Useful, bypassable by design once the user
turns prompts off — which is the entire point of an unattended runner.

**Container — a real boundary with named holes.** Kernel-shared namespaces plus whatever you mounted
and whatever egress you allowed. The holes are: mounted host paths (the workspace itself, any
credential mounts), the network allowlist, the docker socket if you mount it, added capabilities
(`NET_ADMIN`), and privileged/root containers. Anthropic's rule of thumb — use a container or the
sandbox runtime whenever running with permissions skipped — is the right shape
([sandbox environments](https://code.claude.com/docs/en/sandbox-environments)).

**VM/microVM — the strongest local option**, and the one Docker Desktop now ships as `sbx`
([Docker Sandboxes](https://docs.docker.com/ai/sandboxes/)). Out of scope for reeve v1 as a build
target, but relevant as an escape hatch: "point reeve at a Docker sandbox" is a cheap future story.

### Design consequence for reeve

If v1 is worktree-only, reeve **must not** market it as a sandbox, and should be structurally honest
in the UI: a work unit is isolated *from your other work*, not *from your machine*. The docs should
say, in the user's language: if you would not run this repo's `postinstall` script unattended, do not
run an unattended agent on it in a worktree.

---

## 5. Windows: the constraint that decides this

Measured on the author's actual machine (Windows 11 Home 26200, git 2.45.1.windows.1, NTFS,
Docker 27.0.3 present with WSL2 distros `docker-desktop` and `Ubuntu`, both stopped; Developer Mode
on; `LongPathsEnabled=1`; `core.longpaths=true`; `core.autocrlf=true`; `core.symlinks=false`).

### 5.1 Path length

`MAX_PATH` is 260 chars in the classic Win32 API; long-path support exists from Windows 10 1607 but is
**opt-in** via `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled`, and git needs
`core.longpaths=true` on top
([Microsoft](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation)).
Worktrees make this materially worse because they add a nesting level: Claude Code's default is
`.claude/worktrees/<name>/`, so every path in the checkout grows by ~25–30 chars before the project's
own `node_modules/.pnpm/@scope+pkg@1.2.3/node_modules/...` nesting starts. Reports of "Filename too
long" on worktree *deletion* exist even with short root paths
([GitWorktree issue #21](https://github.com/zielu/GitWorktree/issues/21)).

Both flags happen to be set on this machine — that is luck, not a baseline. **reeve must check both at
startup and tell the user how to fix them**, and should default to a short worktree root (e.g.
`C:\reeve\wt\<id>` or a sibling directory) rather than a deep in-repo path. Note also that this
repo's own path (`C:\00_Mis cosas\Proyectos\reeve`) contains spaces and non-ASCII characters — every
path reeve passes to a shell, a Docker `-v` flag, or a setup script must be quoted, and that is a
class of bug that will not appear on the maintainer's CI if CI is Linux.

### 5.2 File locking — the biggest practical Windows tax

Windows enforces mandatory file locking: an open handle blocks deletion. Node dev servers, TypeScript
watchers, language servers, IDE indexers, and Defender all hold handles inside a worktree. The
consequence is a real, filed bug — Claude Code on Windows failing to remove worktrees with
`Device or resource busy` / `Permission denied` after `npm install` and a dev server
([anthropics/claude-code#41740](https://github.com/anthropics/claude-code/issues/41740), reported
2026-04-01, since gone stale).

I reproduced the failure mode locally and found something worse than a failed delete:

```
# with one open handle inside the worktree
$ git worktree remove --force <wt>
error: failed to delete '<wt>': Invalid argument            (exit 255)

# retry after closing the handle
$ git worktree remove --force <wt>
fatal: '<wt>' is not a working tree                          (exit 128)

$ git worktree list        → only the main worktree
$ Test-Path <wt>           → True   (partially deleted directory, still on disk)
```

git removed the administrative registration *before* finishing the directory deletion, so after the
failure the tree is invisible to `git worktree list` while still occupying disk. Retrying the git
command cannot fix it; `prune` doesn't see it. **A worktree-based reeve on Windows must own deletion
itself**: track the child processes it spawned per unit, terminate the process tree before removal,
then delete the directory directly (with retries), and only then reconcile git's registration. If
reeve delegates cleanup to `git worktree remove`, it will leak directories on Windows, and the user
will find out via disk usage.

Related Windows-only wrinkle already handled by Claude Code: when deleting a worktree, delete NTFS
junctions and directory symlinks as *links*, not by recursing into them — earlier versions could
delete the pointed-to folder ([worktrees docs](https://code.claude.com/docs/en/worktrees)).

### 5.3 Symlinks, junctions, line endings

- `core.symlinks=false` is the Windows default (confirmed on this machine), so symlinks committed in a
  repo materialise as plain text files. Developer Mode being on (also confirmed) makes unprivileged
  symlink creation possible, but git won't use it unless configured.
- pnpm falls back to junctions when Developer Mode is off; junctions store absolute paths, so a
  `node_modules` tree built in one worktree cannot be moved or shared to another
  ([pnpm FAQ](https://pnpm.io/faq)). Copying a `node_modules` between worktrees on Windows is
  therefore not a safe shortcut.
- `core.autocrlf=true` (confirmed) means the working tree holds CRLF. Feed that working tree to a
  Linux container and shell scripts fail with `/bin/bash^M: bad interpreter`
  ([Red Hat](https://developers.redhat.com/blog/2021/05/06/why-windows-and-linux-line-endings-dont-line-up-and-how-to-fix-it)).
  **This is a direct hybrid tax:** the moment reeve bind-mounts a Windows-checked-out worktree into a
  Linux container, the CRLF question becomes reeve's problem. The fix is a project `.gitattributes`
  with `* text=auto eol=lf` — which reeve can detect and warn about but cannot impose.

### 5.4 Docker on Windows: what it really costs

- Docker Desktop on Windows 11 Home works, but **Home/Education editions can run Linux containers
  only**, and the WSL2 backend needs virtualisation enabled, 8 GB RAM, and WSL ≥ 2.1.5
  ([Docker install docs](https://docs.docker.com/desktop/setup/install/windows-install/)). Licensing
  is fine for reeve's audience (free for personal use, and for companies under 250 employees /
  $10M revenue), but it is still a proprietary dependency in an otherwise free/open-source tool.
- **Bind-mount performance across the Windows/Linux boundary is bad, by Docker's own documentation.**
  Docker tells you to store bind-mounted source in the Linux filesystem, not under `/mnt/c`, and warns
  that `inotify` events only fire for files stored in the Linux filesystem
  ([WSL2 best practices](https://docs.docker.com/desktop/features/wsl/best-practices/)). VS Code's
  remote docs say the same and recommend named volumes or cloning into a volume for heavy
  dependency trees ([improve disk performance](https://code.visualstudio.com/remote/advancedcontainers/improve-performance)).
  So the "obvious" hybrid — worktree on `C:\`, bind-mounted into a Linux container — is precisely the
  slow path, and it also breaks file watching, which agents rely on for test-watch loops.
- Resource overhead is a recurring complaint: `vmmem`/`VmmemWSL` growth, and the WSL2 VHDX growing
  without shrinking when images are removed
  ([docker/for-win#12518](https://github.com/docker/for-win/issues/12518),
  [microsoft/WSL#8725](https://github.com/microsoft/WSL/issues/8725)). For a single-user workstation
  running several units at once, this is not noise.
- On this machine, both WSL distros were *stopped* at probe time. Cold-starting the WSL VM plus the
  Docker engine is a multi-second-to-tens-of-seconds tax on the first unit of a session — an
  interaction cost that a "click ticket → agent starts" product feels directly.

### 5.5 What the ecosystem's Windows story looks like

This is the clearest signal in the whole research:

- **Conductor**: macOS only.
- **Sculptor** (container-based): macOS and Linux; Windows only via WSL2, by running the Linux
  AppImage inside WSL ([Imbue docs](https://docs.imbue.com/)).
- **OpenHands** (container-based): Windows path is "use WSL/Ubuntu"
  ([local setup](https://docs.openhands.dev/openhands/usage/run-openhands/local-setup)).
- **container-use**: macOS is the recommended install path; no Windows story in the README
  ([repo](https://github.com/dagger/container-use)).
- **Claude Code's own sandboxing**: the built-in Bash sandbox does not support native Windows; the
  documented Windows recommendation is a container, a VM, or running inside WSL2
  ([sandbox environments](https://code.claude.com/docs/en/sandbox-environments)).
- **Codex** went the other way and built a *native* Windows sandbox on Windows primitives (restricted
  tokens, filesystem ACLs, firewall rules, a dedicated low-privilege sandbox user), while still
  recommending WSL2 when you want Linux-native tooling
  ([Windows sandbox](https://learn.chatgpt.com/docs/windows/windows-sandbox)).

Read that list twice. **Container-based agent isolation on Windows is, in practice, "run the whole
tool inside WSL2".** If reeve chooses container-only, reeve is choosing to be a WSL2 application — and
then the user's Windows checkout, Windows editor, and Windows git config are on the wrong side of a
filesystem boundary that Docker itself warns you not to cross. That is a large, permanent tax on the
author's own primary platform.

---

## 6. The hybrid, and its specific trap

The attractive hybrid is: one worktree per unit on the host (branch isolation, easy diff review,
cheap), mounted into one container per unit (runtime isolation, security boundary). Both
`container-use` and several devcontainer-based workflows do exactly this.

The trap is mechanical and well documented: **a worktree's `.git` is a file containing an absolute
host path** (confirmed above: `gitdir: C:/…/reeve/.git/worktrees/reeve-wt-probe`). Mount only the
worktree into a container and every git command inside fails, because the common dir isn't there
([docker/for-win#7332](https://github.com/docker/for-win/issues/7332),
[GitWorktree.org devcontainer guide](https://www.gitworktree.org/guides/devcontainer)). Devcontainer
tooling added `--mount-git-worktree-common-dir` for this, and it is *silently ignored* when
`devcontainer.json` sets a custom `workspaceMount`
([devcontainers/cli#1243](https://github.com/devcontainers/cli/issues/1243),
[vscode-remote-release#11478](https://github.com/microsoft/vscode-remote-release/issues/11478),
[devcontainers/cli#796](https://github.com/devcontainers/cli/issues/796)).

The workarounds, in increasing order of soundness:

1. Mount the parent directory containing both the worktree and the main `.git` — leaks the whole
   repo, including every other unit's worktree, into the container. Destroys the isolation you wanted.
2. Mount the common dir at the exact path the `.git` file names — requires the container path to equal
   the host path, which is what Docker's sandboxes do deliberately, but is awkward with Windows paths.
3. Use **relative worktree paths**: git ≥ 2.46 supports `git worktree add --relative-paths` and
   `worktree.useRelativePaths`, which makes the pointer resolve identically on both sides of a mount.
   Caveat: it sets `extensions.relativeWorktrees`, so older git and libgit2-based tools (which still
   lack support, [libgit2#7210](https://github.com/libgit2/libgit2/issues/7210)) can't read the
   repo — and **the author's machine is on git 2.45.1, which predates the feature.**
4. Skip mounting entirely: clone into the container over the local repo (Sculptor's model) and move
   work back with git. Cleanest boundary, most machinery, and it needs a sync/pull UX.

None of these is free. Option 3 + option 4 are the two that a serious hybrid should choose between,
and both imply constraints (minimum git version; or a sync layer) that need to be decided
deliberately, not discovered.

---

## 7. Recommendation

**Ship worktree-default for v1, with the container sandbox as a designed-for, non-shipped second
backend.** Not container-only. Not worktree-forever.

### Why not container-only

1. **It makes reeve a WSL2 app on the author's own machine.** Every container-based tool surveyed
   (Sculptor, OpenHands, container-use) resolves Windows as "run it inside WSL". Anthropic's own
   guidance for native Windows hosts is container/VM/WSL2. Choosing container-only means the user's
   repo either lives in WSL2 (and their Windows editor, git config and file watchers are across a
   slow boundary) or lives on `C:\` and is bind-mounted (the path Docker explicitly warns against,
   with broken `inotify` on top). There is no third option.
2. **It imports a proprietary, heavyweight dependency** (Docker Desktop) into a free, local-first,
   single-user tool, along with VM memory overhead, an ever-growing VHDX, and cold-start latency on a
   product whose core interaction is "click a ticket, agent starts".
3. **The dependency-bootstrap problem doesn't disappear, it changes shape** — it becomes image
   authoring, and Imbue's own limitation (agents can't change the container spec they're running in)
   is unresolved.
4. Container-only still needs branches/worktrees underneath to get work back out, so it is strictly
   more machinery, never less.

### Why not worktree-forever

1. **A worktree is not a security boundary, and the threat is real** (Nx, 2025). Any user who runs
   reeve unattended on a repo with third-party dependencies is exposed at full host privilege. The
   shared-hooks finding above means an agent isn't even confined to its own worktree.
2. Worktrees don't isolate ports, databases, or daemons — the collisions that make parallel units
   actually painful — and the mitigations require the *user's project* to cooperate.
3. Reproducibility ("works the same for every unit") is a container property, not a worktree property.

### Why worktree-default is nevertheless right for v1

- It is the **only** option that works on native Windows with zero additional installation, and the
  author's primary platform should be the first-class one.
- It is fast (135 ms to create, measured), cheap to reason about, and produces artifacts the user
  already understands: a branch and a directory.
- Review UX is trivial: the diff is a local branch, `git diff` works, the editor opens the folder.
- It is what the mature single-user tools converged on (Conductor, Cursor's local parallel agents,
  Claude Code's `--worktree`), and their pain points are documented well enough to design around
  rather than rediscover.

### What v1 must actually contain (this is the cost of choosing worktrees)

Worktree-default is only cheap if reeve does the unglamorous parts. Non-negotiable:

1. **Own the process tree.** Track every process spawned per unit; kill the tree before cleanup.
   Without this, Windows cleanup fails (§5.2) and the failure leaves an orphaned directory that git
   can no longer see.
2. **Own the deletion.** Never trust `git worktree remove` alone on Windows: kill → delete directory
   with retry/backoff → `git worktree prune` → verify. Report leftovers in the UI.
3. **Lock while running, sweep when idle.** `git worktree lock` for the duration of a run; release on
   process exit; a bounded sweep for abandoned units that refuses to delete unpushed commits,
   modified files, or untracked files.
4. **Startup preflight for Windows:** `LongPathsEnabled`, `core.longpaths`, git version, and a warning
   when the repo path contains spaces or non-ASCII. Default the worktree root to a short path.
5. **`.worktreeinclude`-style copy-in of gitignored files** (`.env`, `.npmrc`, local configs). Adopt
   the existing filename and semantics rather than inventing one.
6. **A per-project setup hook** with clear "this is your job, not reeve's" framing, plus per-unit env
   (`REEVE_UNIT_ID`, `REEVE_PORT_BASE`, `COMPOSE_PROJECT_NAME`) so projects can self-deconflict ports
   and compose stacks.
7. **Disk accounting per unit** in the UI, and a documented pointer to Windows Dev Drive for users who
   run many units.
8. **Truth in labelling.** The UI and docs must say a work unit is isolated from other work, not from
   the machine; and must recommend the agent's own sandbox flags (Claude Code's `/sandbox`, Codex's
   Windows sandbox) as the per-command layer reeve does not provide.

### What to design for now so the container backend stays cheap later

- Model isolation as a **pluggable `WorkspaceProvider`** with one interface: `create(unit) → handle`,
  `exec(handle, cmd)`, `diff(handle)`, `destroy(handle)`. The worktree provider and a future container
  provider must both fit it. If `exec` isn't in the interface from day one, containers will never fit.
- Never assume the agent process runs on the host, and never assume host paths equal workspace paths.
- Prefer **`git worktree add --relative-paths`** wherever available (git ≥ 2.46; the author's 2.45.1
  needs an upgrade), so the same worktree can later be mounted into a container without the
  absolute-`gitdir` trap (§6). Feature-detect and fall back.
- When the container backend lands, prefer the **devcontainer spec + `@devcontainers/cli`** over a
  bespoke format, and prefer **egress-proxy-with-credential-injection** over mounting secrets. Never
  mount the docker socket.

### What would change this recommendation

- If reeve's target user turns out to run unattended agents on untrusted repos as the *normal* case,
  the security argument dominates and container-first becomes correct despite the Windows tax.
- If Docker Desktop's microVM sandboxes (`sbx`, Windows-supported via winget) prove to give
  same-path passthrough with acceptable Windows performance, "delegate isolation to `sbx`" becomes a
  cheaper container story than building one — worth a benchmark before writing any container code.
- If the author moves primary development into WSL2 anyway, the calculus flips entirely; that should
  be an explicit decision, not a drift.

---

## Sources

Primary documentation
- [git-worktree](https://git-scm.com/docs/git-worktree)
- [Claude Code — Run parallel sessions with worktrees](https://code.claude.com/docs/en/worktrees)
- [Claude Code — Choose a sandbox environment](https://code.claude.com/docs/en/sandbox-environments)
- [Claude Code — Development containers](https://code.claude.com/docs/en/devcontainer)
- [Claude Code — Security](https://code.claude.com/docs/en/security)
- [Dev Containers specification](https://containers.dev/) · [Dev Container CLI](https://code.visualstudio.com/docs/devcontainers/devcontainer-cli)
- [VS Code — Improve disk performance in containers](https://code.visualstudio.com/remote/advancedcontainers/improve-performance) · [Change the default source mount](https://code.visualstudio.com/remote/advancedcontainers/change-default-source-mount)
- [Docker — WSL2 best practices](https://docs.docker.com/desktop/features/wsl/best-practices/) · [Install Docker Desktop on Windows](https://docs.docker.com/desktop/setup/install/windows-install/) · [Docker Sandboxes](https://docs.docker.com/ai/sandboxes/) · [Sandboxes architecture](https://docs.docker.com/ai/sandboxes/architecture/)
- [Microsoft — Maximum path length limitation](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation) · [Dev Drive and copy-on-write](https://devblogs.microsoft.com/engineering-at-microsoft/dev-drive-and-copy-on-write-for-developer-performance/)
- [pnpm — Git worktrees](https://pnpm.io/git-worktrees) · [pnpm FAQ (junctions on Windows)](https://pnpm.io/faq)
- [OpenHands — Runtime architecture](https://docs.openhands.dev/openhands/usage/architecture/runtime) · [Local setup](https://docs.openhands.dev/openhands/usage/run-openhands/local-setup)
- [Codex — Windows sandbox](https://learn.chatgpt.com/docs/windows/windows-sandbox)

Implementations
- [Imbue — How we made sandboxed coding agents 10x faster to start](https://imbue.com/blog/containers) · [Sculptor announcement](https://imbue.com/blog/sculptor-announce) · [Imbue docs](https://docs.imbue.com/)
- [dagger/container-use](https://github.com/dagger/container-use)
- [Cursor — Run cloud agents in your own infrastructure](https://cursor.com/blog/self-hosted-cloud-agents)
- [Conductor](https://www.conductor.build/)
- [worktree-compose](https://www.worktree-compose.com/) · [Docktree](https://github.com/Bnjoroge1/Docktree)

Issues and incident reports
- [anthropics/claude-code#41740 — worktree removal fails on Windows (file locks)](https://github.com/anthropics/claude-code/issues/41740)
- [devcontainers/cli#1243](https://github.com/devcontainers/cli/issues/1243) · [devcontainers/cli#796](https://github.com/devcontainers/cli/issues/796) · [vscode-remote-release#11478](https://github.com/microsoft/vscode-remote-release/issues/11478)
- [docker/for-win#7332 — git worktree inside docker](https://github.com/docker/for-win/issues/7332) · [docker/for-win#12518 — vmmem memory](https://github.com/docker/for-win/issues/12518) · [microsoft/WSL#8725](https://github.com/microsoft/WSL/issues/8725)
- [libgit2#7210 — relative worktrees unsupported](https://github.com/libgit2/libgit2/issues/7210)
- [zielu/GitWorktree#21 — filename too long on worktree deletion](https://github.com/zielu/GitWorktree/issues/21)
- [Socket — Nx packages compromised](https://socket.dev/blog/nx-packages-compromised) · [Snyk — weaponizing AI coding agents](https://snyk.io/blog/weaponizing-ai-coding-agents-for-malware-in-the-nx-malicious-package/)
- [Quarkslab — why exposing the docker socket is a bad idea](https://blog.quarkslab.com/why-is-exposing-the-docker-socket-a-really-bad-idea.html)
- [Red Hat — why Windows and Linux line endings don't line up](https://developers.redhat.com/blog/2021/05/06/why-windows-and-linux-line-endings-dont-line-up-and-how-to-fix-it)

Practitioner reports
- [Every AI agent tool creates git worktrees; none make them work](https://dev.to/rohansx/every-ai-agent-tool-creates-git-worktrees-none-of-them-make-worktrees-actually-work-3ae9)
- [GitWorktree.org — worktrees with dev containers](https://www.gitworktree.org/guides/devcontainer)

First-hand measurements in §2.1, §5.1 and §5.2 were taken on the author's machine
(Windows 11 Home 26200, git 2.45.1.windows.1, NTFS, Docker 27.0.3 / WSL2) on 2026-07-25.
