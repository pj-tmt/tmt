# Development

The Rust workspace is the shipped CLI runtime. Office is frozen and kept internal;
its retained SPA foundation lives in `extensions/tmt-office/typescript/apps/office` and is not required to use the CLI. The nested
`typescript` pnpm workspace owns private developer tooling for Vitest, fixtures
and release verification. This tooling workspace is not an npm product or a CLI
fallback. Repository policy is
in [AGENTS.md](AGENTS.md), architecture ownership in
[ARCHITECTURE.md](ARCHITECTURE.md), and style in [CONVENTIONS.md](CONVENTIONS.md).
Use this guide for reproducible commands and evidence.

## Setup

This section is for contributors, not end users. Rust/Cargo builds the product.
Node and pinned pnpm run Vitest, native-process and Docker orchestration, formatting,
type checks and release verification. `better-sqlite3` is an independent test
oracle; `tar` builds test archives. These are retained developer dependencies,
not reasons to install TMT through npm. npm is not a separate required workflow;
use the pinned pnpm lockfile rather than introducing another package manager.
Unless a command explicitly changes directories, pnpm commands in this guide run
from `typescript/`; Cargo and Docker commands run from the repository root.

Requirements are Node.js 22.12 or newer, the pinned pnpm toolchain, and the
Rust toolchain declared by `rust/rust-toolchain.toml`. The workspace MSRV is
Rust 1.95; CI also runs the current pinned release toolchain.
Remote-client tests require `python3` for the independent byte-fixture oracle.
Shell completion tests require Bash and Zsh. Runtime proof uses the selected
macOS developer tools or Linux `readelf` (binutils); these are verifier tools,
not product runtime dependencies.

```bash
(cd typescript && corepack pnpm install --frozen-lockfile)
(cd rust && rustup show)
```

Do not install the product globally while testing. Keep application state,
provider directories, temporary prefixes, sockets and child processes inside a
task-owned temporary root.

If `better-sqlite3` is used by retained tooling, it is a development-only
independent SQLite oracle. It is not the Rust runtime, a product dependency or
an excuse to reopen native schema state through Node.

## Keep local development from filling the disk

Local builds and browser runs are the largest source of waste on a developer
machine: Docker's build cache and image tags, and one Cargo `target` per
worktree. These rules bound them, and the other sections link here instead of
repeating them.

Before starting a Docker suite, run the read-only check. It prints free disk
space (a warning below `TMT_DISK_WARN_GB`, default 30) and `docker system df`,
and deletes nothing:

```sh
scripts/dev-disk-check.sh
```

Below the threshold, stop and tell the maintainer. Delete only what you
created, and never restart Docker Desktop; ask the maintainer instead.

**One image tag per worktree.** Name every local verification image after the
worktree, so a rerun replaces the image instead of adding another. Worktrees
are reused across tasks, so a tag never names a PR or an issue:

```sh
worktree=$(printf %s "$(basename "$(git rev-parse --show-toplevel)")" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9_.-' '-')
```

The commands in this guide use `tmt-office-browser:$worktree`, its
`-capacity` variant and `tmt-office-check:$worktree`. `pnpm test:e2e` already
uses a per-run tag and removes it when it exits. Do not create ad hoc tags such
as `pr423` or `c1b-<sha>`.

**Prune the build cache periodically.** Whoever runs the suites runs this
about once a day of use. It removes only build cache older than 24 hours, never
images or volumes, and no other `docker ... prune` is part of this workflow:

```sh
docker builder prune --filter until=24h -f
```

**Each worktree keeps its own `rust/target`.** Native selectors,
`scripts/tmt-dev.sh` and the Docker fixtures all read
`rust/target/debug/...`, and a directory shared between worktrees would let a
test run another worktree's binary. Do not point `CARGO_TARGET_DIR` at `/tmp`
or anywhere else. To avoid adding a worktree (and a fresh compile) for every
task, reuse a clean one: check that `git status --short` is empty and the
branch is pushed or merged, then `git fetch origin` and
`git switch -c <new-branch> origin/main`.

**Remove a worktree when its PR merges**, in the same turn, as AGENTS.md
requires, with the script that checks it is safe:

```sh
scripts/dev-worktree-remove.sh <worktree-path> <pr-number>
```

It removes the worktree and prunes only when both hold:

1. `git status --short` prints nothing (no uncommitted or untracked files);
2. either the PR is merged (REST `GET /repos/{owner}/{repo}/pulls/<n>` has
   a non-null `merged_at`), or
   the branch has an upstream and `git log @{u}..` prints nothing.

The script resolves the REST repository from the target worktree's remote. Open,
closed-unmerged and failed PR lookups require the upstream proof in (2).

`git log @{u}..` alone is not enough: when the maintainer updates a PR branch on
the server (a rebase), it lists local commits although nothing is lost. If
neither half of (2) holds, or the script refuses for any reason, stop and ask
the maintainer. A refusal is not permission to remove the worktree by hand,
with `--force`, or by deleting the branch. Only after the script succeeds,
remove that worktree's images:

```sh
worktree=<worktree-name>                       # as defined above, for that worktree
docker image rm "tmt-office-browser:$worktree" "tmt-office-browser:$worktree-capacity" \
  "tmt-office-check:$worktree"                 # images that were never built are reported and skipped
```

Removing the worktree deletes its `rust/target` with it.

## Run the workspace CLI

From this checkout's root:

```bash
(cd rust && cargo build --locked -p tmt-cli)
sh scripts/tmt-dev.sh --version
(cd typescript && corepack pnpm tmt office --help)
(cd rust && cargo build --locked -p tmt-office)
./rust/target/debug/tmt-office --help
(cd rust && cargo build --locked -p tmt-squad)
./rust/target/debug/tmt-squad --help
```

The `scripts/tmt-dev.sh` launcher and nested `pnpm tmt` script both launch only this
checkout's `rust/target/debug/tmt`. They do not
build automatically, install anything, or fall back to a global binary. Rebuild
after changing Rust sources. For clean JSON stdout use
`(cd typescript && corepack pnpm --silent tmt ...)`.
The launcher preserves arguments, exit status and environment; normal CLI commands
still use the usual application data unless you explicitly select isolated settings.
It does not replace the installed Office companion or update a running Office UI.
The reserved `tmt office` facade and direct `tmt-office` command share the Office
parser and handlers. Direct commands resolve core lookups through
`TMT_EXECUTABLE` (when invoked by TMT) or an executable `tmt` on PATH; use an
isolated application home when testing writes.

Extensions can use the public [local process API](contracts/extension-api.md) for
structured dispatch, history, conditional room writes and bounded notebook reads.
Its contract is owned by [architecture](ARCHITECTURE.md#local-extension-api-v1).

Agents may launch `tmt mcp --identity <saved-name-or-uuid>` on redirected stdio.
The [local MCP contract](contracts/mcp-v1.md) owns the tool schemas and transport
qualification. Use an isolated application home for native MCP tests:
`CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-adapters mcp::tests` and
`CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test mcp` from
`rust/`. These tests cover protocol admission, command/resource parity, pinned
identity scoping and direct-child cleanup without provider credentials or tmux.
MCP runtime framing stays separate from private channel framing; provider setup,
writing tools and waiting talk are not part of this slice.

## Office SPA

Office is frozen for official installation and publication. Existing installations,
source, contracts and data remain; local contributor verification is still supported.
The [Office command reference](extensions/tmt-office/docs/commands.md) describes retained installations.

### Revive Office

Revival requires maintainer approval; a passing build is not permission to publish.
Keep this procedure as the single owner of the revival steps:

1. Review CLI/companion protocol and forward-only schema compatibility against the
   retained releases. Keep existing tags, receipts and user data intact; stop old
   writers before switching a development build. Any migration needs owner approval.
2. Restore Office release eligibility in `.github/components.json`, its binary's
   Cargo `package.metadata.dist` setting, the native-release product choices, and
   the CLI's `INSTALLABLE_EXTENSIONS` together. Regenerate with
   `node typescript/scripts/release-please-config.mjs` and verify with `--check`.
   Follow the [native release verification](#native-release-verification)
   for the generated config/manifest and compatible archive checks; do not reset
   a historical release version or recreate a published tag.
3. Run the release-policy, workflow and release-config tooling tests, and the
   native extension-install and Office lifecycle tests. Update the frozen-state
   expectations for the approved eligibility change; keep the installable-set
   comparison against the component map and retained/partial-removal coverage.
4. Review Office CI selection and worker/artifact dependencies under the approved
   revival scope. Verify positive and negative path selection, nonempty test
   discovery, required check names and aggregates; retain workspace Rust coverage.
   Run the [Office browser verification](#office-browser-verification) and affected
   local-service checks before enabling broader CI or proposing a release.
5. Update the user-facing frozen status and Office reference pages together.
   Coordinate any handbook changes with its single writer, tmt-design-lead. Obtain the separate release
   approval and complete the release guide's acceptance before publication.

### Local development

Office native data contracts and codecs are in
`extensions/tmt-office/rust/tmt-office-model`, a member of the `rust/` workspace.
Use `cargo test --manifest-path rust/Cargo.toml -p tmt-office-model` for pure
admission/vector checks, and the affected adapter tests for file/storage behavior.
After changing these boundaries, run `cargo test --manifest-path rust/Cargo.toml
-p tmt-cli --test architecture`; the guard checks actual workspace dependencies,
that only Office crates consume the Office model, and that core crates declare
no Office modules beyond the retained facade. Full isolated verification still
covers the runtime consumers; pure model tests do not replace it.

The optional app uses React, Vite, TanStack Router and Jotai. Read
[Office architecture](extensions/tmt-office/docs/architecture.md) before changing its boundaries.
From the repository root:

```sh
cd typescript
corepack pnpm install --frozen-lockfile
corepack pnpm office:dev
corepack pnpm office:check
corepack pnpm office:test
corepack pnpm office:build
```

The dev server binds loopback. Production output is `extensions/tmt-office/typescript/apps/office/dist`; preview
with `pnpm --filter @tmt/office preview`. A deployed SPA host must rewrite app
routes such as `/setup` to `index.html`; hosting and Firebase setup are not part
of the scaffold. Tests use jsdom and the real router, not a browser-layout proof.

`pnpm check` checks workspace tooling and Office. `pnpm check:tooling` retains the
native test/release tooling's independent quality gate. Tooling-workspace `test:run` still
selects only tooling tests; `office:test` explicitly selects app tests and fails
on empty discovery. Workspace and extension formatting uses `vp fmt` from exact
Vite+ 1.0.0 (bundled Oxfmt 0.70.0). Each package explicitly selects its existing
Vite or Vitest configuration's `fmt` block, preserving its options and file list.
Import and package-key sorting are disabled. Run `pnpm format` or
`pnpm format:check` for tooling and `pnpm --filter @tmt/office format` for the app.
Tooling code retains `typescript/.prettierignore`, including the Markdown ignore;
`pnpm docs:format:check` retains its explicit docs list and `../.gitignore`
override so those Markdown files are checked separately. The single target lists
live in `typescript/scripts/format-workspace.mjs`; both write and check modes expand
parent-relative globs to absolute paths there and fail if any pattern matches nothing.
Keep these script names and separate selections when changing formatting.
Root tooling, native, stress, Docker and extension suites run through exact
Vite+ 1.0.0 with bundled Vitest 5.0.1. Their separate configurations retain their
own test discovery and are selected explicitly with `--config`. Use
`pnpm test:watch` for watch mode; `vp test` runs once by default. The workspace
Vite override makes Office/addon build, dev and preview commands and their plugins
share the aliased core. The alias exposes no `vite` executable; those commands use
`vp build`, `vp dev` and `vp preview` with their existing compiler steps and flags.
Each test configuration sets `clearMocks: false` to preserve mock history, and
ordered suites use `{ concurrent: false }`.
Vitest 5 changes generated `it.each` case labels: `$field` strings lose
quotes, and percent placeholders use the new value renderer (including signed
zero and object clipping). Migration parity records each changed label with its
source template and case index alongside both actual titles and equal statuses;
unchanged labels remain exact multiset matches. Preserve the raw reports rather
than silently normalizing these differences.
Office wire-schema conformance is a nested tooling test. From `typescript`, run
`corepack pnpm exec vp test run --config vitest.config.ts test/tooling/office-contracts.test.ts`. See
[`extensions/tmt-office/contracts`](extensions/tmt-office/contracts/README.md) for its single source of truth,
versioning and limits. Design vectors are not executable authorization or crash
recovery evidence; downstream suites must prove those behaviors separately.
Root tooling uses the threads pool with at most two suite workers. Native process, stress and tmux
configurations also select the threads pool and retain their own execution rules.

For an Office-only clean checkout, use `pnpm office:install`. It installs from
the app directory against the same workspace lockfile, without workspace recursion.
The Vite/Vitest overrides have one owner in `typescript/package.json`
(`pnpm.overrides`). Pinned pnpm also reads that manifest at the lockfile directory
for isolated installs; `--ignore-workspace` skips the workspace YAML.
`pnpm-workspace.yaml` retains package membership and the lifecycle script allowlist.
With pinned pnpm 10.33, a plain Office `--filter` install still builds root
SQLite; the explicit isolated install avoids that unrelated dependency. Do not
create an app lockfile or remove `--frozen-lockfile` to work around a mismatch.
The script resolves the lockfile directory through an absolute path: pinned pnpm
can apply a relative path twice and place modules outside the checkout. The
non-root browser image verifies dependencies stay in its workspace. Firebase's
optional auto-configuration postinstall and protobufjs's version warning script
remain unapproved; explicit app configuration and bundled SDK code need neither.

The app's verification-only Dockerfile proves the isolated install has neither
the root SQLite oracle nor a native TMT executable. It runs quality, DOM tests
and a production build with networking disabled:

```sh
docker build -f extensions/tmt-office/typescript/apps/office/Dockerfile -t "tmt-office-check:$worktree" .
docker run --rm --init --network none "tmt-office-check:$worktree"
docker image rm "tmt-office-check:$worktree"
```

Use the worktree tag from [Keep local development from filling the
disk](#keep-local-development-from-filling-the-disk); remove only that
verification image. This is not a deployment image or a browser/Firebase E2E claim.

For native-only fixtures, `pnpm --filter tmux-team install --frozen-lockfile`
installs only tooling dependencies; Docker copies workspace/package metadata before
this step. Do not make native fixture containers install or execute Office.
The workspace explicitly permits lifecycle scripts only for the existing
`better-sqlite3` oracle and esbuild tooling. Fresh oracle builds need Python,
make and a C++ compiler; the tmux fixture image supplies these build tools.

CI always reports `Code quality` and `Native package matrix`. Selected Office
changes also run the required `Office SPA` job, which builds the existing
`browser-tests-base` target without executing Playwright. That target owns the
Office service check/test/build, Office type/lint/format/unit checks, browser-test
type checking and partition inventory, and local, preview, emulator and cloud SPA
builds. The component-map selector keeps native coverage conservative for shared and unknown
inputs and scopes Squad-only changes to its declared tests. Frozen Office web,
local-service and companion-dependent native verification runs only for Office-owned
PR paths (including Office-specific verification machinery), weekly or manually.
Merge groups and main cache-seeding pushes never select that work. Workspace Rust
checks still compile and test Office; unselected native process runs need no Office
artifact and exclude only the Office-owned suites. The run summary records each
changed path's owner, rule and selection, and aggregate gates require exact selected
or skipped results.
The [CI selection and worker model](ARCHITECTURE.md#ci-selection-and-worker-model)
owns worker responsibilities, scope expectations, fixture handoff and cache policy.
To reproduce the MSRV check, read `workspace.package.rust-version` from
`rust/Cargo.toml` into `MSRV`, then run
`cargo +"$MSRV" check --locked --workspace --all-targets` from `rust/`.
Rustup resolves the manifest's two-part minimum to its latest patch release,
rather than duplicating a patch pin in the workflow.
Remote Rust retains full native coverage; remote TypeScript and browser
paths are outside that Rust rule. Code
quality includes the selector's own focused tests even when native unit jobs are
unselected, and requires the selected Office check. The native aggregator rejects
failed, cancelled or unexpectedly skipped selected jobs. The advisory Office browser
partitions run in their own workflow (below), so a red `CI` run means one of its own
jobs failed. CI changes need positive
and negative selection/gate evidence before pushing; do not change branch
protection merely to get a newly skipped job accepted.

Merge-group candidates use `node typescript/scripts/ci-scope.mjs merge-group
"$HEAD_SHA"` with the event's exact head SHA, after fetching full history. The
selector diffs `merge-base(refs/remotes/origin/main, head)..head`: under HEADGREEN,
the event base can be a preceding pending queue commit, so it must not be used
as the cumulative baseline. A site-only tip still selects checks for earlier
pending release version/lock changes. Docs-only groups skip native suites,
Squad-only groups run Squad checks, and shared changes select the full native
scope. Missing, unreadable, ambiguous or empty range evidence fails closed to
full native verification with both E2E shards and Office unselected, and the selection summary
reports the fallback. PR merge-base selection is unchanged. The macOS exception is described in the runtime smoke matrix below.
Check event wiring with `pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts`
from `typescript/` and `actionlint .github/workflows/ci.yml` from the root.
[Architecture](ARCHITECTURE.md) owns the event, gate and main-ref cache policy;
queue/ruleset changes remain a repository-owner operation.

### Merge queue metrics

Collect read-only REST evidence with authenticated `gh` (no GraphQL):

```sh
node typescript/scripts/merge-queue-metrics.mjs --repo pj-tmt/tmt \
  --since 2026-10-02T00:00:00Z --until 2026-10-02T08:08:40Z \
  --boundary 2026-10-02T06:08:40Z --cache /tmp/tmt-queue-metrics-cache \
  --output /tmp/tmt-queue-metrics.md --json /tmp/tmt-queue-metrics.json
```

Bounds are UTC, inclusive start/exclusive end; `--boundary` compares cohorts.
Runs/merge and merges/hour measure queue throughput. Group duration measures
start to last completed job; job creation-to-start delay includes runner and dependency wait.
Enqueue latency measures the last recorded queue entry to merge.
Per-job/event/runner rows measure executions, duration and sole worker failures.
Tree tags separate confounders; fail/pass pairs identify flake candidates.
Methodology limits and unknown evidence are described in the script and report.
`--details` prints all cost rows; repeated `--tag-pr N` overrides #961/#963.
`--repo OWNER/REPO` defaults to `pj-tmt/tmt`; `--workflow FILE` defaults to `ci.yml`.
The local cache holds REST responses; `--offline` requires cached evidence.
`--max-requests N` overrides the 500-request budget; split capped searches into
smaller windows. Posting is separate. Verify with
`pnpm exec vp test run --config vitest.config.ts test/tooling/merge-queue-metrics.test.ts` from `typescript/`.

Linux CI package installation uses `.github/actions/apt-install`: each apt update
or install attempt has a 120-second timeout with a 10-second forced-kill grace.
The existing retry helper makes at most three attempts, with 5- and 10-second
backoffs. Apt also uses 30-second HTTP/HTTPS network timeouts and two acquisition
retries. The action removes the unused Chrome source before updating; package
selection stays with each caller. Revisit the attempt bound before adding large
packages to the current small dependency sets.

`release-please-config.json` is generated, not hand-edited. After changing the component
map, a crate's version declaration, the workspace's crates or its dependencies between
them (including a new file or directory under an extension root, because the CLI's exclude
list is written out from the tracked files), run
`node typescript/scripts/release-please-config.mjs --write`; the
`release-please-config` tooling test (and `--check`) fails while the file is stale; `Code quality`
runs that test on every pull request, including Squad-only ones whose Unit tests are skipped,
because Squad's manifest is one of its inputs. The
generator needs `cargo` and reads no network. Update the pinned release-please CLI in
`.github/release-please/` with `pnpm install` there and commit its lockfile; the test
requires an exact version and an integrity hash for every locked package. Before local tooling
checks or release-config tests, run this explicit prerequisite from the repository root:

```bash
pnpm --dir .github/release-please install --frozen-lockfile --ignore-scripts
```

Tests and the release wrapper load this single isolated pin. `pnpm check:tooling`
(and therefore `pnpm check`) first checks its runtime and type entry files through
`release-please-run.mjs check-install`. A missing install stops before TypeScript
checking with the command above, rather than cascading implicit-any diagnostics.
The guard only reads files; it never installs packages. Direct `pnpm type:check` and
release-config test runs also require the explicit install.

Private leaves declare `releaseConsumers` in the component map; today only TUI names Squad.
The release workflow uses `release-please-run.mjs` with the pinned API to attribute these commits
before the ordinary splitter, excludes and product release cutoffs. No `additional-paths` option
exists in 17.11.2. An upgrade must re-verify the API shape and run
`pnpm exec vp test run --config vitest.config.ts test/tooling/release-please-config.test.ts` from `typescript/`:
the suite exercises real release candidates, TUI-only and unrelated/private controls, mixed commits
and independent release cutoffs. Ownership, CI selection and version/lock updates remain separate.

For the separate Office Auth/Firestore environment, follow
[`extensions/tmt-office/typescript/services/office/README.md`](extensions/tmt-office/typescript/services/office/README.md). It uses Docker-contained
Java and Firebase tooling with a demo project; no host Firebase login is required
for emulator tests. Local real-project mappings and credentials must remain
ignored by both Git and Docker. Never substitute this bootstrap smoke proof for
future membership rules, invitation or browser tests.

### Local browser sign-in

Start the emulators as described above, then explicitly opt in:

```sh
pnpm office:dev --mode emulator
```

Open the loopback URL printed by Vite. Choose **Sign in with Google (emulator)**,
add a local test account in the popup and observe its UID. No real Google login,
Firebase owner alias or credentials are needed. Reload signs out; another tab
does not inherit the session. Default `office:dev` / `office:build` stays a
disconnected preview. Login does not create a world or grant access.
Console-managed tester admission gates direct Firestore world creation/read.
For explicit real Google sign-in and owner-local configuration, see
[the limited cloud pilot](extensions/tmt-office/typescript/services/office/README.md#limited-cloud-pilot).
Do not point automated tests at a real project.

Run the real-browser suite with the same emulator owner:

```sh
docker build --target browser-tests -f extensions/tmt-office/typescript/services/office/Dockerfile -t "tmt-office-browser:$worktree" .
docker run --rm --init --shm-size=256m "tmt-office-browser:$worktree"
```

The tag is per worktree (see [Keep local development from filling the
disk](#keep-local-development-from-filling-the-disk)), so a rerun replaces the
image. Remove it with `docker image rm "tmt-office-browser:$worktree"` when the
worktree is done.

The image installs pinned Chromium, checks the app, runs DOM/session tests and
builds preview, emulator and unconfigured cloud variants. The container starts
disposable Auth/Firestore/Functions emulators and three strict-port preview servers, then
Playwright. The cloud build must fail closed without operator configuration;
it never contacts a real project during automated tests.
It uses one worker, no retries, bounded waits and independent browser contexts.
The complete local command above remains the acceptance entry point. The
`Office browser verification` workflow (`.github/workflows/office-browser.yml`) runs
the same standard browser identities as advisory diagnostics in twelve isolated
partitions to reduce the chance
that serial scenarios exhaust a per-job deadline:
emulator-backed contracts, three local Vite shards, and eight native-local shards.
The eight native-local shards (#424) and the three local Vite shards (#574) are paused on
pull requests until they are fixed and run weekly and by manual dispatch instead; a pull
request runs only the emulator partition, for Office-owned paths or its own
verification machinery.
`office_browser` selects paths owned by Office in `.github/components.json`
(including its documentation and test fixtures) plus browser-specific machinery
listed in `selectOfficeBrowser` (the browser workflow, emulator verifier and
Docker context policy). Shared dependency/selector/generic fixture changes rely
on the weekly/manual safety net to catch Office build breakage. Scheduled and manual runs cover all twelve
partitions, including the emulator, regardless of paths. Required CI selection
uses the same frozen Office ownership policy while retaining conservative native coverage.
For red weekly Office runs, follow the triage ownership in the
[CI selection and worker model](ARCHITECTURE.md#ci-selection-and-worker-model).
One `image` job builds the `browser-tests` target once and shares it as a one-day
artifact; every partition loads that image and never builds it.
Local partitions do not start Firebase, while native-local shards use the container
Secret Service and embedded companion without Vite or Firebase.
`test:browser:partitions` compares the exact Playwright identities from all twelve
partitions with the standard suite, rejects overlaps or omissions, keeps
`native-decoration.spec.ts` in the emulator partition, and separately proves that
the four opt-in capacity scenarios retain the full original inventory without
overlapping the standard browser inventory. Every partition keeps one worker, zero retries and the
existing scenario limits. Browser results do not gate merge aggregates and are not
required checks; failed jobs remain visible in that workflow, whose status is the only
place advisory failures show, and retain their logs and artifacts. Native Rust CI still owns
formatting, linting, locked builds, embedded SPA service tests and process/parser
contracts. The container-native shards retain installed-browser diagnostics without
being rerun after compilation.
Browser matrices and Playwright commands continue after individual failures while the
runner remains active. A failed command uploads available Playwright error contexts
before a final fail-closed step records the advisory job failure. Native browser jobs have a
25-minute deadline for each retained 4–9-test partition. That remains a best-effort
diagnostic budget: several tests can still consume their 120-second scenario limits. A
deadline or runner termination can truncate a partition and prevent later artifact
steps despite their failure/cancellation predicate. Report completed and expected
counts together, including missing artifacts; do not claim a complete inventory from
the partition count alone.

### Office browser verification

`ci-scope.mjs` owns the frozen Office selection described by the
[CI selection and worker model](ARCHITECTURE.md#ci-selection-and-worker-model).
Office-owned scenarios require the companion fixture; shared and unknown changes
rely on the weekly/manual Office safety net. Local verification of an Office-owned
change still uses the full browser and native acceptance suites.

- Run the local browser suite above before opening a PR for an Office-affecting
  change, and record the result in the PR.
- On a pull request, the `Office browser verification` workflow runs only the emulator
  partition, and only when `office_browser` is selected (Office ownership or verification machinery).
  The native Office shards and the local Vite shards do not run on pull requests until #424 and #574 are fixed, because they
  fail on most runs: weekly/manual runs include them and the emulator, all twelve together, and
  `ci-scope.mjs` still computes `native_office` for the change that re-enables the native
  shards. Their results are advisory: they are not required checks and never gate merge,
  and a red run of that workflow is a browser diagnostic, not a `CI` failure.
- If a remote browser job fails but the same tests pass reliably in the local
  suite, treat the failure as flaky: record the local pass in the PR and move on.
  Only a failure that also reproduces locally needs a fix.

Capacity diagnostics are preserved as explicit opt-in runs and are not required CI:

```bash
docker build --target browser-tests -f extensions/tmt-office/typescript/services/office/Dockerfile -t "tmt-office-browser:$worktree-capacity" .
docker run --rm --init --shm-size=256m --network none \
  --env TMT_TEST_BROWSER_CHANNEL=chromium \
  "tmt-office-browser:$worktree-capacity" \
  sh /workspace/extensions/tmt-office/typescript/services/office/with-test-keyring.sh \
  pnpm --filter @tmt/office test:browser:capacity
```

Only failed-transaction assertions use the Firestore fixture's 20-second budget:
the pinned SDK's five attempts can spend 12.1875 seconds in jittered backoff
alone. Keep the ordinary 10-second UI expectation and 30-second scenario limits.
Those scenarios assert intercepted transaction reads and independent durable
state before and after explicit retry; a timeout alone is not transport evidence.
Auth responses are real and local. The suite also exercises the real Firestore
SDK against Rules: immutable creation/retry, cross-user denial, self-grant denial,
shape validation, and browser grant/create/revocation. The separate agent-grant
Rules suite adds custom-principal tokens, assigned UUID blocks, claim/capability
isolation, malformed/expired grants and one-way owner revocation with unchanged
cached tokens. `pairing-service.spec.ts` adds actual HTTP approval/custom-token
issuance, concurrent claims and live revocation. Direct service composition with
an injected signer adds failure/recovery evidence against the same real database.
Those service scenarios alone are not evidence of protected native storage or
CLI-to-browser pairing. Service-only scenarios can run with `--network none`
by overriding the image command to select that spec. Operator fixtures
also independently inspect saved block layouts. Home-block scenarios cover
revision races, exact retries, Rules/client conformance vectors, bounded list
validation, owner/device isolation and browser placement/save/reopen/revocation.
Desktop and narrow editor screenshots are written to Playwright test results
for primary visual review, not treated as automatic visual acceptance.
Operator fixture writes
use a hard-wired loopback demo-project bypass, never production credentials.
Google's official popup transport still loads
public JavaScript from `apis.google.com`, even in emulator mode. The browser
allowlist permits only those script GETs and loopback, blocks optional upstream
CDN styling, and fails for any other destination. This suite requires Internet
access for that library; it is not a fully offline OAuth test. No host
credentials/volumes or published ports are used. A failed browser test must
propagate through `emulators:exec`; do not count a skipped or empty suite as proof.
The selected Office CI job runs this same target. Native tmux E2E remains
separate. Remove the task-owned verification image when no longer needed.

`pairing-browser.spec.ts` proves real browser consent -> issuer approval ->
original-proof claim -> scoped block write -> cached-token denial after owner
revocation, plus same-request reopening, non-owner denial, lost-response retry
and logout during a pending confirmation. Inspect the actual approval POST body
as well as durable state; URL-only capture is not proof of a secret-free body.
The originating proof is a fixture, not a native CLI. DOM/state tests separately
cover admission/route fencing and explicit consent. Browser and service decoders
consume the same literal pairing corpus; browser encoding tests use an independent
Node encoder. Review desktop/narrow screenshots rather than accepting their
existence as visual proof.

`native-pairing.spec.ts` installs the real compiled CLI/companion through a
synthetic verified archive, using the existing `typescript/test/support` process and artifact
owners. It resumes a protected proof across CLI processes after Chromium consent,
then independently checks the issued grant, OS-store record, scoped Firestore
read, token refresh, expired server/native lease renewal, simulated lost-response readback,
unchanged resources, corruption, revocation and same-name replacement.
The browser target therefore builds Rust and installs the root test-helper
dependencies with scripts disabled; native helpers never enter the SPA. The
standalone app image stays independent.
`with-test-keyring.sh` owns a disposable D-Bus session and real Linux Secret Service
with a fixture-only password and private container directories. The scenario
deletes and verifies absence of its protected entry; container teardown removes
the disposable store. No host Keychain, credentials or installed CLI is used.
This is Linux credential-store evidence, not macOS runtime or real release-archive
acceptance. Reuse the separate native artifact verifier for published artifacts.

`native-cancellation.spec.ts` verifies expired protected state, unknown-request
preservation, browser cancellation, secret-free receipts and same-identity fresh
pairing using the real CLI and isolated vault. `pairing-cancellation.spec.ts`
checks cancellation/approval races and denied unknown-request writes through the
real issuer. Neither replaces cached-token denial or retirement coverage.

`native-hooks.spec.ts` adds the causal retirement path using a private real tmux
server and the container's `sqlite3` tool as an independent state observer.
Verify local retirement plus pending notification before any remote cleanup,
then disabled pairing/grant, retained block and denial of a previously usable
cached token. A missing session bus or vault entry must retain pending work.
An unknown pending proof stays retryable and later cancels a known approval
without claiming credentials or creating a grant. A test-only SQLite
acknowledgment trigger separates remote/vault success from local completion;
retry must consume the protected revoked receipt without restoring credentials.
Neither hook registration unit tests nor service-only revocation proves this
chain. Run the Office browser target and tmux lifecycle suite twice after changes
to these cleanup boundaries, with verified private-server/vault cleanup.

For focused service checks use `pnpm office:service:check`,
`pnpm office:service:test` and `pnpm office:service:build`. `pnpm check` also runs
the combined `@tmt/office type:check:e2e`; standalone `office:check` deliberately
does not load service dependencies through E2E fixtures. The Docker browser
target runs both package checks and this explicit E2E type check on Node 22.

For in-progress native deployment discovery, run
`cargo test --locked -p tmt-office-pairing office_deployment` from `rust/`.
These tests verify literal
browser/native descriptor conformance, strict URL/JSON validation and bounded
unauthenticated HTTP; they do not prove native pairing or protected credentials.
`deployment.spec.ts` checks the actual built emulator descriptor and unavailable
preview/cloud variants. Issuer scenarios independently compare claim
`grantExpiresAt` with Firestore, including retries and a shortened grant; approval
and token expiry must not stand in for that value.

## Handbook website

The handbook is a static site in `site/` (Vite, React, TanStack Router, Jotai,
Tailwind and MDX), with its own lockfile outside the TypeScript workspace.
Chapters are `site/src/chapters/*.mdx`, registered in `site/src/chapters/index.ts`.
The sticky status bar at the top is the chapter navigation: one tmux-style window per
chapter group (`windows` in the same file), with the `[tmt]` button opening the full page tree.
Animated scenes are declarative frames in `site/src/scenes/` (`Window`, `TmuxBar`, `useFrames`): they
advance only while on screen and rest on one complete frame under `prefers-reduced-motion`.
A chapter opens with one such scene from `site/src/chapter-scenes/` (the working chapter's message travel, the squad
chapter's mark legend, the sketches on the in-progress colab and planned meet pages); its words are in `site/src/lang/strings.ts` like the home page's.
`index.html` also asks Google Fonts for the token mono family's glyphs of the marks (`●○◌◆✗✓↻▸`), because the
latin subset has none; the family lacks `○✗✓↻`, which fall back to the system monospace font.
Every page exists in English at its path and under `/ja/` and `/zh-hant/`. A translation is
`site/src/i18n/<lang>/<chapter file>.mdx`, named like the English chapter in `site/src/chapters/` and
exporting its front matter as `frontmatter` (`title` is the page title). A page without a file shows the
English page with a "not yet translated" note. The language dropdown in the status bar (and in the `[tmt]`
menu on a narrow screen) keeps the page, remembers the choice in the browser and sets `<html lang>`
(`zh-hant` is `zh-Hant`). Published `/zh/` routes redirect to `/zh-hant/`, preserving the
page, query and anchor; the earlier browser preference `zh` also selects `zh-hant`.
`scripts/spa-routes.mjs` writes each language's route files with their `<html lang>`
and `hreflang` alternates; `SITE_ORIGIN` makes the alternates fully qualified, and
`.github/workflows/site.yml` sets it to the Pages origin next to the default `/tmt/` base path.
The words of the site's own components (home page, status bar, notes around a page) are typed data in
`site/src/lang/strings.ts`; a language overrides any of them, key by key, in
`site/src/i18n/<lang>/strings.json`, and what it leaves out stays English. The reserved `$source` key of that
file is for the staleness check and is never merged.
Colors, fonts and marks come from `design/tokens/tokens.json`, which the
stylesheet and the design page read. Anything not in a release is marked
planned.

```sh
cd site
pnpm install --frozen-lockfile
pnpm dev                        # local preview at http://127.0.0.1:5173/tmt/
pnpm check                      # types, Vite+ lint/format, MDX imports, translation sync and its tests
pnpm build                      # dist/ for GitHub Pages, one index.html per route
SITE_BASE=/ pnpm build          # for a root path, such as a custom domain
SITE_BASE=./ VITE_SITE_HISTORY=hash pnpm exec vp build   # a preview at an unknown path
```

### Translations

English is the source. A translation of `site/src/chapters/<page>.mdx` is
`site/src/i18n/<lang>/<page>.mdx` (`ja`, `zh-hant`) and starts with front matter:

```yaml
---
source: site/src/chapters/<page>.mdx
sourceRevision: <git hash-object site/src/chapters/<page>.mdx>
title: <the translated page title>
---
```

`sourceRevision` is the git blob SHA of the English page the translation was written
from, so the check needs no git history. `pnpm check` runs `scripts/i18n-sync.mjs`:
a page whose English source has changed since is reported as stale, as a warning
(a GitHub annotation in CI) that does not fail the check; a missing front matter,
a `source` that is not the same-named English chapter, or a malformed
`sourceRevision` fails it. After updating a translation, set `sourceRevision` to the
new blob SHA. A chapter without a translation falls back to English. The directory
list comes from `languageExceptions` in `.github/repository-layout.json`
([AGENTS](AGENTS.md#repository-content-language) owns the language exception), and
`scripts/mdx-imports.mjs` checks translated pages too. The block is kept out of the
page and exported as `frontmatter`.

A language's UI and home strings are `site/src/i18n/<lang>/strings.json`, overriding
`site/src/lang/strings.ts` key by key. The file carries a reserved top-level
`"$source"` object, which the loader never merges, and the same check applies:

```json
{
  "$source": {
    "source": "site/src/lang/strings.ts",
    "sourceRevision": "<git hash-object site/src/lang/strings.ts>"
  }
}
```

A changed `strings.ts` makes the file stale (a warning); invalid JSON, a missing or
malformed `$source`, or another `source` fails the check.

Translators keep these in English everywhere: command names, flags and ids, the
words `talk`, `reply` and `receipt`, the board marks (● ○ ◌ ◆ ✗ ✓ ↻ ▸), sample
terminal output and code. A translated heading keeps the English slug as an explicit
id (`<h3 id="install">安裝</h3>`), because links, the home page and the on-this-page
list use it and `slug()` drops non-Latin text. Chinese (`zh-hant`) is Traditional Chinese
with Taiwan usage; it also keeps `agent`, `driver`, `harness`, `board`, `colab` and
`meet`, and uses 窗格 for pane, 終端機 for terminal, 擴充套件 for extension, 卡住 for blocked and
恢復 for resume.

`.github/workflows/site.yml` checks and builds the site on pull requests and
`main`. Every push to `main` that changes `site/**`, `design/tokens/**` or the
workflow deploys the built site to GitHub Pages, one deployment at a time; a manual
run on `main` with `deploy` set redeploys. Pull requests never deploy. The
repository is public, so merging a site change publishes it.

## Personal-office milestone acceptance

`retained-spaces.spec.ts` proves owner discovery of a revoked grant, opening and
editing its retained UUID block through the shared editor, independent stored
state, unchanged home layout, reopening and admission loss. The grant Rules
suite independently checks bounded owner pagination and denies unbounded,
oversized, foreign-owner, agent and revoked-admission queries. Owner editing
does not by itself prove native block mutation, reassignment or credential recovery.

Retained-block reassignment adds actual competing approval transactions in
`pairing-reassignment.spec.ts`: one source reservation, exact retries, abandoned
unclaimed recovery and denial of claimed ancestors. Browser approval selects
the source explicitly and preserves assignment across uncertain responses.
The native acceptance must resume the original protected proof, inspect the
same retained block and corroborate it in the browser and independent database;
seeded grants or a browser-only claim do not substitute for that chain. Keep
the former cached credential's denial and unchanged layout as separate assertions.

M1 acceptance is the offline local flow through the actual runtime owners: native
CLI and companion -> authenticated loopback browser -> shared SQLite -> service
restart. Browser approval, cloud credentials and Firebase emulators are not M1
prerequisites. Retained remote regression coverage separately uses the native CLI
and Office companion -> browser owner approval -> scoped credential use -> durable
resource change -> visible browser result, with demo-project Auth/Firestore
emulators; it is not an M1 prerequisite. Use deterministic mock agents, isolated
real tmux where relevant and Playwright Chromium. Extend the existing fixture owners
as each feature ships, not a parallel mock implementation of TMT. Independent
database observations must corroborate UI and command results. Separate green layer
suites are not proof of an integrated flow. The native pairing browser scenario
covers acquisition and scoped reads. `native-decoration.spec.ts` adds real remote
block show/apply, independent stored tokens/revisions, visible scene geometry, exact
retry without timestamp changes, invalid input, conflicting local callers and
read-only/revoked authority.
Local callers share fail-fast locks: pre-execution contention reports `OFFICE_BUSY`;
retrying the original intent after the winner must conflict without another write.
This is not proof of a Firestore precondition race. The existing browser transaction and Rules suites retain that
independent server-side coverage. Neither replaces the other's acceptance.

Automated acceptance must not call a paid model, use provider/host credentials,
contact production Firebase or require paid runners. Keep the native-only tmux
suite network-disabled. Office integration keeps service traffic inside its
isolated fixture; the browser popup's existing allowlisted Google script is an
explicit network exception, not permission for cloud data or model calls.
Use bounded readiness and polling, fail on empty/skipped acceptance, and verify
child/socket/buffer/state cleanup. Run lifecycle-sensitive local suites twice.

Cover approval, denial, expiry, duplicate/concurrent claims, wrong scope,
revocation with cached credentials, uncertain writes, reconnect, temporary
retirement and same-name replacement. Resource slices add revision conflicts,
profile/layout/notebook independence, board disclosure and installed-guidance
checks. Preserve real request/response and storage owners; no terminal-output
completion fallback or second memory store.

For local presentation-profile changes, run the shared profile vectors in Rust and
Office, SQLite create/no-op/retry/conflict plus retirement tests, companion/CLI and
protected loopback route tests, and the real local CLI→SQLite→browser→restart flow.
Capture desktop and narrow screenshots and inspect name, hair, clothing, mark and
offline-presence legibility. No cloud account or remote publication is part of this gate.

Direct-manipulation UI changes are covered by `local-office-direct-manipulation.spec.ts`
(click/drag/cancel, auto-apply, persisted Undo and room properties),
`native-local-catalog-drag.spec.ts` (catalog pointer previews, rejected/cancelled
drops, one-change auto-apply, exact history and native restart persistence),
`local-office-editor-hud.spec.ts` (desktop/narrow context controls) and
`native-local-skybridges.spec.ts` (native auto-apply and canvas meeting creation).
`world-yjs.test.ts` covers selective history, entity ordering, observation exclusion,
atomic gestures and lifecycle; `use-world-editor.test.tsx` covers the JSON/CAS queue,
acknowledgement races, explicit conflict recovery and refreshed native observations.
These are local history checks, not evidence of a deployed Yjs synchronization provider.
The remaining legacy browser scenarios below still contain explicit layout-mode
scripts and require migration before a release gate can claim full current-UI coverage.

`native-local-world.spec.ts` owns the installation-wide layout lifecycle: a lazy
furnished 2×2 Lobby plus four unassigned offices, explicit browser Save, one SQLite world revision, empty placements
remaining empty after restart, and no new per-identity block rows.
`typescript/test/support/office-world.ts` owns explicit legacy fixtures for older topology
scenarios; do not translate the evolving new-world preset to simulate old inputs.
`native-local-topology.spec.ts` owns personal-area removal, replacement Lobby,
retained identity content and real external-write conflicts using explicit legacy
inputs rather than retired terrain-authoring controls. Its error-state
desktop/short/narrow viewport checks require non-overlapping, operable HUD controls
without resizing the canvas. `native-local-walls.spec.ts` covers browser-authored
wall art, native exterior/support admission, retained rejected drafts, explicit
repair and restart. Shared read-only SQL and pointer helpers live in
`native-world-state.ts` and `world-editor-gesture.ts`; keep expected fixture extents
independent of production geometry and assertions inside their scenarios.
`native-local-composition.spec.ts` combines private-tmux saved/temporary identities,
furnished personal areas, a meeting set, editable wall objects and floating
Chat/Info/room-audience panels. Inspect its desktop/narrow renders after readiness;
it supplements, rather than replaces, the focused lifecycle and delivery tests.
`native-local-profile.spec.ts` independently verifies that profile edits, retries
and conflicts do not mutate the saved world or role definition, and retirement
retains profile and world bytes.
`native-local-meeting-modules.spec.ts` exercises the V4 in-world name entry,
independent canonical-room creation, layout Undo/Redo/Cancel, existing-room
reattachment, furnished Save, targeted membership and blocked spatial removal.
Independent SQLite reads distinguish room writes from layout writes.
`native-local-unified-areas.spec.ts` covers explicit legacy-to-v8 alignment,
acknowledged Undo/Redo, use changes at a stable floor pointer position, spatial
removal, canonical-room retention, a shared creation ghost and service restart.
Inspect its desktop and narrow screenshots for lamp/icon differentiation and
unchanged platform materials. Shared unified-area vectors prove Rust/TypeScript
floor and opening parity; projection tests cover occupancy-independent spacing
and inverse picking. Retained island vectors and strict freeform connectivity
remain separate compatibility tests.
The retained V4 scenario supplies 1536×1024 DPR-1 and narrow screenshots;
`native-local-central-grid.spec.ts` adds
sparse circulation, wall mounts and DPR-2 idle evidence. These fixtures do not
prove default-world conversion or final visual fidelity.
`native-local-agent-meeting.spec.ts` verifies agent Info → room selection → member
review → Save using the installed companion and independent SQLite/CLI reads.
It covers retained members, discard, draft protection across agent switching,
restart, unchanged world bytes and no dispatch. DOM dialog visibility is emulated
once in `src/test-setup.ts`; native browser tests own real modal/focus behavior.
Native browser navigation selects agents in the whole-world Directory; identities
without assigned areas do not acquire implicit rooms. Avatar reinstall/fallback
and inbox/reply scenarios use the same installed companion and isolated fixture.

`native-local-module-keyboard.spec.ts` verifies keyboard name entry and button
activation, Undo/Redo/Save, restart and narrow-screen object admission/repair,
with independent SQLite observations and no terrain-painting interface. Native
select values use Playwright selection; this does not prove OS-level popup
keyboard navigation, which requires a separate native-input accessibility check.

`native-local-prop.spec.ts` verifies whole-world placement, directional artwork,
per-object customization, restart and missing/corrupt-pack recovery without an
identity-owned room. Observe layout bytes independently of catalog mutations;
compare rendered pixels, not only object selectors. `native-local-prop-capacity.spec.ts`
covers full catalog admission, overflow immutability and actual visible artwork
from every installed pack. Keep pixel probes clear of architectural occlusion.
`native-local-workstation.spec.ts` verifies source-derived bundled furniture and wall props through
real catalog placement, authored chair views and native Save/reopen. Offline art
encoding and its source-review gates are documented in the
[modular visual package](extensions/tmt-office/docs/references/rooms-and-walls/modular-v1/README.md).
`native-local-furniture-rotation.spec.ts` verifies retained static furniture's
directional successor through corner gestures, precision rotation, exact
Undo/Redo and restart. Its runtime gallery captures all four views of the
[directional furniture](extensions/tmt-office/docs/references/furniture-rotation/README.md);
review those images as well as the state assertions when changing this art.
`native-local-furniture-base.spec.ts` owns full-art upper hit testing and frontmost
selection independently of shallow physical support. Its real drags distinguish
supported overhang from unsupported bases, retain atomic Undo/Redo and restart,
and exercise all directions plus the narrow-screen rotation control.
`native-local-room-materials.spec.ts` checks exact world-pixel Undo/Redo, retained
content and bounded finish textures. `captureWorldScene` excludes HUD presentation
for pixel comparisons; separate unmodified screenshots verify the visible HUD.
For the accepted platform style, review the same seeded scene at a fixed viewport,
DPR and browser version: a Lobby with four offices, both bridge axes, meeting
branches, selected objects, and valid/invalid drag previews. Geometry assertions
lock the connector constant and inverse picking; behavior assertions lock no-write
invalid/cancelled drops and one completed gesture per Undo/Redo step. Current
skybridge/direct-manipulation tests retain native lifecycle evidence. The separate
opt-in `playwright.visual.config.ts` owns two focused visual scenarios under
`e2e/visual/*.visual.ts`, using the existing local HTTP fixture and real scene/HUD.
They protect platform lighting, supported overhang, held rotation/selection, and
desktop/narrow inspector and creation spacing. They do not prove native admission
or persistence, and do not join the standard browser CI partitions.

From `typescript`, run `pnpm --filter @tmt/office test:visual`. The config starts
only the offline Vite server, uses the lockfile-pinned Playwright Chromium at DPR 1,
fixed viewport/locale/color scheme and reduced decorative motion, and refuses to
create missing baselines by default. Install the matching browser with
`pnpm --filter @tmt/office exec playwright install chromium` when necessary.
PNG names include the host platform; a missing platform baseline is not permission
to copy another platform's pixels or claim cross-platform equivalence. Failed
comparisons preserve expected/actual/diff images in Playwright's test-results.

After an intentional design change, explicitly run
`pnpm --filter @tmt/office test:visual --update-snapshots=all`, inspect every changed
PNG against the previous baseline and the approved design, then rerun without the
update flag. Never regenerate a baseline merely to make a failing comparison pass.
Keep screenshot tolerances strict and verify representative defect sensitivity;
portable geometry/behavior assertions remain with their existing unit/native owners.
`native-local-world-capacity.spec.ts` exercises dense tile-budget and connected
sparse worlds through native admission and the browser. `scene-observation.ts`
observes actual WebGL submissions and texture lifetimes across zoom, pan, revisit
and replacement; the idle window must submit no new draws. Keep measured startup,
CPU and heap evidence separate from portable assertions: texture counts are not
GPU bytes, and JS heap is not total browser memory.

For local discussion-board changes, extend `native-local-discussion-admin.spec.ts`: use
one-shot CLI mutations while the service is stopped, the rendered browser through
the real loopback API, a fresh CLI read and independent SQLite observations. Cover
category isolation, inert text and attribution, board-specific stale revisions with
unchanged storage, owner moderation tombstones, CLI replies visible after browser
refresh, restart durability, loopback-only traffic and an
asserted stopped service. No browser route interception or cloud approval is part of
this local gate.

`native-local-service.spec.ts` owns installed-service faults and cleanup: separate
browser/control authority, live-session reuse, bounded incomplete-write shutdown,
rotated credentials, version drift, dead-child recovery, occupied ports, early
companion exit and uncertain receipts. Use the shared native Office fixture;
do not recreate its installer, process sandbox or executable selection in scripts.

`extensions/tmt-office/typescript/apps/office/e2e/native-local-discussion.spec.ts` owns spatial presentation checks:
opening and closing over the same panned canvas, unsent draft retention, explicit
post/reply persistence without task dispatch, mobile navigation and focus return.
It uses the shared native Office fixture, not a second service harness.

Whiteboard drawing and snapshot/reference scenarios share that fixture. The
snapshot scenario paints in a real browser, checks frozen JSON/PNG against SQLite,
copies a token-free reference, stops the web service, then verifies native reads
and no-clobber PNG export. It saves the exported image for visual inspection;
structured text alone is not image-access evidence. The Send scenario additionally
checks no enqueue before confirmation, frozen recipient UUIDs across retirement
and same-name recreation, exact-envelope replay, native inbox/reply/result and PNG
access. Mounted-state tests cover uncertain transport and exact Retry send input;
the native scenario does not intercept routes to fabricate a successful send.
After the sequential SPA and
native builds below, run these without Vite servers or Firebase:

```bash
pnpm --filter @tmt/office test:browser:native native-local-whiteboard
```

The native browser config reuses the normal one-worker/no-retry policy. Fixtures
own and stop their isolated services. `TMT_TEST_BROWSER_CHANNEL` selects the local
browser (default `chrome`).

`native-local-conversation.spec.ts` verifies direct chat against that same native
fixture: draft retention, target-switch confirmation, inbox delivery, real CLI
reply, browser display, and reload recovery after the host accepts a send but its
HTTP response is dropped. Independent SQLite counts prove recovery creates no
duplicate requests and reading the chat does not acknowledge incoming work.
`native-local-direct-wake.spec.ts` uses an isolated real tmux server and installed
companion to prove a new direct request sends only a request-ID and explicit
recipient-UUID instruction to
the verified pane, replay sends no second notification even after uncertain
paste, and an offline recipient keeps durable inbox acceptance without pane input.
The transport-fault case forwards to the real host before aborting the browser
response; it never fabricates acceptance. It captures desktop/narrow screenshots
and asserts service shutdown. State tests separately cover bounded page/body
reads, observation deadlines, hidden-tab cancellation and failed-read recovery.

`native-local-status.spec.ts` adds private-tmux actors and real CLI self-reports:
one canonical record reaches the directory and Info, browser-only time advancement
expires the rendered cue without another read or stored mutation, and a real
request/reply takes visual priority without changing status. It checks saved and
Contractor presentation, restart persistence and desktop/narrow rendering. Unit
tests own exact deadline/rollback scheduling and appearance/status read races.

Build the local SPA from the repository root:

```bash
(cd typescript && corepack pnpm office:build:local)
```

After the SPA build exits successfully, run the pinned toolchain from `rust/`,
where `rust-toolchain.toml` applies. Never overlap these producer/consumer steps:
Cargo can otherwise reuse the old embedded assets before Vite replaces them.

```bash
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo clippy --locked -p tmt-office --features local-service -- -D warnings)
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-office --features local-service)
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo test --locked -p tmt-office --features local-service)
(cd rust && CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-cli)
```

Finally, return to the repository root and exercise the real installed-style flow:

```bash
(cd typescript && corepack pnpm office:local:e2e)
```

Run the complete applicable local gates before pushing the reviewed commit.
Record commands, results, exact commit, limitations and cleanup in its PR/issue.
Do not use repeated CI pushes for local debugging. Existing required CI gates
remain authoritative until a reviewed cost-policy change provides replacement
evidence and merge requirements; unselected jobs are not passing selected tests.
Do not activate paid runners, model APIs, billing or production deployment.

Real Google login, IAM, deployment and live-agent usability are separate owner-
authorized pilot checks. Request owner assistance when those checks are actually
ready. Emulator success does not prove production IAM or device credential
protection, and a live-agent demo does not replace deterministic regression tests.

## Rust checks

The companion package lives at `extensions/tmt-office/rust/tmt-office`, but remains
in the `rust/Cargo.toml` workspace. Run the same package commands from `rust/`;
the shared lockfile, toolchain and `rust/target` artifact paths are unchanged.
Every Cargo build stage must copy all workspace member directories at their
workspace-relative paths, including private extensions; `docker-workspace.test.ts`
checks the E2E, artifact and Office native contexts.

Office storage migration tests live in `tmt-office-storage`
(`cargo test --locked -p tmt-office-storage`) and build their source databases
under temporary roots through the public core storage entry point. For manual
diagnostics, the companion's hidden
`tmt-office __tmt-office-storage 1 <status|prepare|copy|verify|switch|recover> --global-dir <absolute directory>`
requires an explicit disposable root and never uses configuration discovery.
Never point it at a real installation. Real migration requires the separately
consented user path.

Run from `rust/` for a normal native change:

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
cargo build --locked --example storage-probe
cargo build --locked --example tmux-probe
cargo +1.95.0 build --locked
```

For Codex queue transport, focused deterministic checks are
`cargo test --locked -p tmt-adapters drivers::codex::queue` and
`cargo test --locked -p tmt-adapters drivers::codex::transport`. The transport
tests own local loopback peers and exercise receipt loss and absolute deadlines.
`cargo test --locked -p tmt-adapters drivers::codex::delivery` also checks peer
readiness and a channel-gated final process observation: preparation cannot spend
the queue/receipt budget, expired or unverifiable rechecks send no queue frame,
and the old shared deadline fails the delayed-receipt positive control. Preparation
and delivery each retain a three-second absolute bound; a send can spend six
seconds across these stages. Talk's observer deadline still starts before the
synchronous send and includes its elapsed time; it is not a transport cancellation
boundary. The [Codex contract](contracts/codex-channel-v1.md#transport-and-qualification)
owns these budgets and terminal uncertainty.
Reply-notice waiter timing is checked with
`cargo test --locked -p tmt-cli --bin tmt reply_notice_command::tests`.
These deterministic tests use an injected monotonic clock and sender gate over
real isolated SQLite claims: the competitor waits through the longest declared
single send, dispatches its untouched notice once after settlement, and preserves
the live claim and queued notice when grace expires. The old three-second grace
must fail the positive control. Registry maximum selection is checked with
`cargo test --locked -p tmt-adapters runtime::tests::maximum_send_duration`.
Existing `storage::requests::service_tests::notification` tests retain ownership
of dead-sender recovery and attempted-frame no-replay assertions.
The refusal fixture holds a bound, non-listening socket through the connect attempt;
it never releases a port for a parallel test to claim. It uses the existing nix
Unix dev-dependency with `net`, without a new runtime dependency. These tests do
not start a model or inspect provider credentials. The
[owning contract](contracts/codex-channel-v1.md#transport-and-qualification)
defines the dependency boundary, supported builds and integrated evidence gates.

Private Codex enrollment state (#737) is checked with
`cargo test --locked -p tmt-adapters drivers::codex::record`. These tests own
isolated temporary records and inject liveness evidence; they verify lock-scoped
state changes and replacement preservation, not real crashed-server recovery.

Owned Codex startup and attachment planning (#738) are covered by
`cargo test --locked -p tmt-adapters drivers::codex`. The server cases launch
isolated shell stand-ins and check observable process/file cleanup; cwd probes
compare relative and absolute `-C`. They do not start Codex or a model and do not
replace the final live foreground continuity gate.

For fresh Codex bootstrap (#1198), run focused model-free checks:
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters drivers::codex::supervisor`,
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters drivers::codex::channel_hooks`,
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters drivers::codex::record`,
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters drivers::codex::server`, and
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli --bin tmt run_command::channel`.
They cover empty fresh baseline, one-time thread discovery, deferred publication,
eager same-generation hooks, immediate unrelated-generation/auxiliary paths,
immutable binding, callback failure and evidence-preserving group cleanup.
Activity mapping tests cover exact fresh/resume foreground attribution, prompt
Working and Stop Idle, rejection of missing/malformed/replaced/Unknown evidence,
and the byte-identical default observed incarnation for Claude/plain hooks.
The TERM-ignoring child test checks group SIGKILL and verified reaping without
orphan zombies. Run `CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli --test architecture`
for the shared admission and hook-composition boundaries. Also run
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli private_hook_worker_budget` and
`CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli budget_tests` for typed relative
budget bounds, malformed arguments and the unchanged default two-second worker.
These checks do not
invoke a provider/model or replace the built-driver eager-TUI and Idle proof.

The adapter `process::cleanup_policy_tests` must pass under both `cargo test` and
nextest: isolated re-exec cases prove timeout cleanup regardless of whether the
test runner makes its harness a process-group leader.

For cumulative completed-request counters (#872), run
`cargo test --locked -p tmt-adapters runtime::consumption` and
`cargo test --locked -p tmt-cli --bin tmt output::tests`. Redacted real provider
fixtures and source provenance live beside the runtime owner; failure and reset
variants are assembled. Streaming scan tests also cover multi-MiB appends,
buffer-boundary deduplication, escaped keys, captured EOF and injected deadline
exhaustion. For manual release-mode measurements, run the ignored
`runtime::consumption::tests::streaming_scan_measurement` test with `TMT_SCAN_MIB`
set to `1`, `10` or `100` and `TMT_SCAN_MIX` set to `foreign`, `usage` or `long`.
Fixture generation precedes the reported scan time; run the built test binary
under `/usr/bin/time -l` on macOS for process peak RSS. These measurements are
evidence, never scan-budget calibration. `usage-hooks.e2e.test.ts` verifies admitted hooks,
unchanged context usage, public consumption, silent failures and compaction.

Before native installation/process tests, build the two product fixtures
independently, after workspace checks:

```sh
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-office
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-cli
```

Workspace builds unify adapter features, so their debug CLI can include Office
dependency debug information. Package-scoped builds exercise the ordinary CLI
product without Office features. These fixtures omit debug symbols, not debug
assertions or runtime checks; installation tests should hash/package executable
behavior rather than large DWARF payloads. Keep the existing process deadlines
and assertions. Both binaries remain in `target/debug`, using the established
selectors. This does not replace optimized release-archive verification.

The same unification applies to any combined build: `cargo build -p tmt-cli -p
tmt-squad` (or a workspace build) may compile shared dependencies with features
that only another package enables, so its `tmt` can differ from the product.
Product identity proofs are package-scoped, matching per-product release builds:
compare `cargo build --locked --release -p tmt-cli` alone, at the same checkout
path, before and after a change.

A fixture that writes an executable and then runs it can be refused with
ETXTBSY ("Text file busy") in a multi-test binary: another test thread's `fork`
holds a copy of the write descriptor until the child's `exec`, and nothing the
writer does closes that window. Pick the rule by who writes the file and who
runs it. Production code never retries ETXTBSY.

- The test writes a script and controls how it runs: run `/bin/sh <script>`
  so nothing execs the written inode (the `tmt-invoke` and `tmt-adapters` tests).
- The test writes a stand-in that something else must exec by path, such as a
  fake `tmt` or `tmux` on `PATH`: let a short-lived `sh` write the file, so no
  test thread holds its descriptor. The dev-only
  `tmt-test-support::write_executable(path, bytes, mode)` owns this publication;
  preserve the caller's 0700 or 0755 mode. It uses a cleared writer environment,
  a generous thirty-second hung-writer bound and `tmt-invoke` process-group
  cleanup, with no retries. Readiness probes and their payload-free branches stay
  owner-local.
- The product writes the executable and then execs it, as an installer and its
  verifier do: the fixture waits out the window with a bounded retry of only
  that error (`tmt-office-command`'s `test_support::install_office`,
  `retry_on_text_file_busy`).

In `typescript/test`, publish every written executable or interpreted fixture
through `test/support/executable-fixture.mjs` (`writeExecutable`). Its isolated
Node writer stages on the destination filesystem, fsyncs, closes, chmods and
renames before returning. Closing in the test worker, even with `O_CLOEXEC`, does
not remove the concurrent fork-to-exec window. Pass the existing mode explicitly
for non-executable or negative fixtures; never replace malformed fixture bytes,
add ETXTBSY retries or extend test deadlines. Synthetic installers use the same
module's `--write FILE MODE` entry point with bytes on stdin and an absolute
fixture Node path; this is test infrastructure, not a product runtime dependency.

The opt-in stress test `cargo test -p tmt-office-command text_file_busy_stress
-- --ignored --nocapture` reproduces the race and reports the in-process writer's failures with and
without the case-3 retry, plus shared case-2 publication without retries.
The unretried control remains probabilistic; zero observed failures do not prove
that the race is absent. Non-ETXTBSY errors fail rather than count as race evidence.
For publication and dependency changes, also run
`cargo test --locked -p tmt-test-support` and
`cargo test --locked -p tmt-cli --test architecture`. The latter checks the exact
six dev consumers and rejects production/build imports, aliases, unreviewed target
edges and publishable/distributable support metadata.

For explicit tmux target-resolution errors (#949), run
`cargo test --locked -p tmt-adapters tmux::io_tests` and
`cargo test --locked -p tmt-cli --test target_resolution` from `rust/`.
The adapter tests inject execution faults; the CLI fixture uses a slow stand-in
under an isolated HOME, verifies check/add error codes and confirms timeout cleanup.
It does not contact a real tmux server or replace Docker routing evidence.

The external-host shell fixtures use a test-local runner with a thirty-second
execution budget for success cases. This does not change the driver's wire
`deadlineMs` or output limit. Conformance timing uses scripted elapsed values;
the late-answer case retains the production runner and deadline. Run the focused
suite with `cargo test --locked -p tmt-adapters host::external::tests`.

Human output and help snapshots (`insta`, a dev-dependency) live beside the
tests that assert them, such as `rust/crates/tmt-cli-style/tests/snapshots/`.
After an intended change, regenerate with `INSTA_UPDATE=always cargo test -p
<crate>`, delete any leftover `*.snap.new` files, and review the snapshot diff as
part of the change. CI never updates snapshots.

The CLI style guards ([enforcement](design/cli-style.md#enforcement)) run in
`cargo test`. When a migrated command leaves its list, run them directly from
`rust/`: `cargo test --locked -p tmt-cli --bin tmt cli_style`, `cargo test
--locked -p tmt-squad cli_style` and the architecture test below. A failure
prints the exact list entry to add or remove.

Core's `cli_style` tests also require a nonblank, single-line description for every
option in the public core grammar, recursively excluding the extension-owned Office
tree. The guard's negative controls cover missing, blank and multiline descriptions;
hidden commands/options and positional arguments are outside that help-option rule.
Run `cargo test --locked -p tmt-cli --bin tmt skill_reminder` for named identity hints
and their JSON/opt-out suppression. Native `discovery.test.ts` and `identity.test.ts`
check saved-identity creation and no-op output; Docker `discovery.e2e.test.ts` checks
temporary and saved bindings through the real CLI and private tmux server.

The same core command runs the [printed command guard](design/cli-style.md#enforcement).
When adding or changing a printed command template, update its presentation site's
test-only `HintSpec` list. Supply explicit command boundaries, representative
numeric or enum operands, and a reason for any external-command skip. Source
coverage fails on missing and stale samples. The guard parses without executing;
it also checks editable config keys through the pure settings policy. Its negative
controls retain the original invalid context and config commands from #1014/#1015,
plus unknown flags, subcommands and keys.

Request-ID prefix selection is checked with
`cargo test --locked -p tmt-core --lib request::service::responses`,
`cargo test --locked -p tmt-adapters storage::requests::service_tests::response`,
`cargo test --locked -p tmt-adapters retained_request_id_sample_and_overflow_count`,
and `cargo test --locked -p tmt-cli --test result_prefix` from `rust/`.
The service cases use real isolated SQLite and an injected clock for logical
expiry, late-final horizons and unchanged attention; the storage query-plan case
requires indexed request-ID ranges for both the sample and overflow count.
The CLI cases compare full-ID and prefix human/JSON output and assert bounded
ambiguity errors, unknown/short prefixes, announcements and help. They start no
tmux server or model and use the shared isolated CLI process harness.

The architecture guard is included in `cargo test`. A focused offline run is:

```bash
cargo test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
cargo +1.95.0 test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
```

When changing a guard, exercise a real positive and negative source/dependency
fixture and restore the checkout exactly. A stale lockfile or an unexecuted
test is not evidence that the guard worked. Core must remain free of concrete
I/O; adapters own SQLite/files/processes; CLI owns grammar and composition.

### Internal TUI markup admission

From `rust/`, run `cargo test --locked -p tmt-tui` and
`cargo +1.95.0 test --locked -p tmt-tui` for structural XML admission and its
byte/depth/node limits, integer utilities, property conflicts and literal theme
tokens, schema binding, lexical repeats, scoped IDs and exact expansion limits.
Geometry tests cover flex/grid, native percentages, fr/minmax/span, gaps/padding,
fractional boundaries, shared text budgets, resize restoration and cut clipping.
Paint tests cover grapheme-safe cuts/wrap/clamp, inherited Theme/Depth roles,
caller-owned selection, clipped identity precedence, wide edge blanks and
recorded-width/fractional measure–paint agreement. Run the architecture test for
dependency changes, and
`cargo test --locked -p tmt-squad` for its in-memory source adapter and frozen
board/list parity fixture, projected and retained-view loading identities,
coverage, priority and CSS clamp/default-min mapping. Board fitting uses the shared grapheme owner;
CLI lists retain their scalar fitter. `text::measure` width is a capped upper
bound, not the widest wrapped line: derive intrinsic demand from unwrapped
escaped content; measurement and fitting share the recorded text width.
The parity harness captures the three explicit presets and the team default at
120×30, 80×30 and 120×30 again,
including every cell's style/state, hits, row starts and list text/JSON. Its source
revision is recorded in the fixture. Once it is on main, every later board PR
that intentionally changes captured output must regenerate the baseline in that
PR, in a separate commit, using
`cargo test --locked -p tmt-squad regenerate_markup_parity_fixture -- --ignored`.
Decode the cell/style diff and attribute every change to the PR's approved behavior;
review hit identities and list bytes too. Unexplained changes block handoff.
Normal tests never write the fixture; it contains no host paths or clocks.

Composition changes also run `cargo test --locked -p tmt-squad board::composition`:
literal nested/folded/tiny rectangles, tabs reservation, cache transitions and
non-overlap accompany a 6,006-case preset/fold fingerprint captured from the
pre-cutover solver at `40ad78c8` with Squad's approved nested fractional-parent
rounding rule (issue #775, request `req_8fd1910e`). The rule-derived CSS expectation
and literal cycle cases remain independent of that fingerprint. Keep frozen
board/list parity unchanged; any other rectangle difference stops review.
Header/footer, rich painters and controller tests remain their existing owners.

The internal utilities use one spelling per value kind:

- Cells/weights/counts: `w-N`, `h-N`, `basis-N`, `min-w-N`, `max-w-N`,
  `grow-N`, `shrink-N`, `gap-N`, `gap-x-N`, `gap-y-N`, `p-N`, `px-N`, `py-N`,
  `col-span-N`, `line-clamp-N`; also `grow`, `shrink`, `w-full`, `h-full`.
- Percentages/tracks: `w-[N%]`, `h-[N%]`, `basis-[N%]`, `grid-cols-[tracks]`.
  Grid tracks are underscore-separated cells, percentages, integer `Nfr`, `auto`,
  or `minmax(a,b)` (no fr minimum). `auto` is also admitted as the minimum in
  `minmax(auto,N)`; it is not admitted as a minmax maximum.

`N` is ASCII decimal 0..4096, in terminal cells or integer weights/counts;
spans/clamps must be positive. Percentages are integers 0..100. Unlike Tailwind,
`w-4` means four terminal cells, not a rem spacing scale. Bracket integers fail
with a located error and a bare-form hint. Layout modes are `flex`, `flex-row`,
`flex-col` and `grid`; grid conflicts with explicit flex direction. Text uses
`truncate`, `truncate-middle` (terminal extension) and `line-clamp-N`; the former
`text-ellipsis-middle` proposal and unbracketed percentages are rejected.
Unknown/malformed utilities, duplicate/overlapping properties, variants,
fractional numbers and arbitrary CSS values/units fail admission.
Padding is symmetric. View/col default to column; other elements to row.
Sizes/basis default to auto, gaps/padding/grow to zero, and shrink to one.
Text defaults to clipping; leaf `wrap="true"`/`"false"` conflicts with all
text-flow utilities. `token` names a shared `Role`; omission preserves inheritance.

Geometry consumes already selected tracks: Squad owns priority hiding. Optional
tracks must fit their full mapped minimums; only non-priority overflow can cut.
CSS percentages use the parent's content box; gaps can cause grid overflow.
Earlier tracks retain sizes; the cut cell needs four visible cells or hides whole.
One injected scalar measurer owns intrinsic metrics and wrapping/clamp; paint
reuses the recorded integer text width, then ellipsizes cut visual lines without
reflow. See [architecture](ARCHITECTURE.md) for the inactive production seam.

## Native process and shared tests

### Selecting the CLI under test

The maintained JavaScript suites live under `typescript/test/native/`, `typescript/test/e2e/`,
`typescript/test/tooling/` and `typescript/test/support/`. Rust tests stay beside the owner in
`rust/crates/*` or the companion package under `extensions/tmt-office/rust/`.
`tmt-cli/tests/support` owns the environment and direct-child lifetime shared by
`stdin_flags` and `request_observer`. Their commands clear the parent environment;
HOME, XDG config/data/state/cache, CODEX_HOME, temporary files and the tmux socket
directory stay under each fixture's owned root. State uses the canonical
XDG config `tmux-team` directory. Native TypeScript sandboxes use the same isolation
contract with an explicit system/Node PATH and UTF-8 locale. Their named runtime
connection allowlist retains only `DBUS_SESSION_BUS_ADDRESS`, so Office browser
fixtures can reach the container-owned Secret Service without inheriting HOME,
XDG runtime paths or unrelated parent variables. Executable selectors are resolved
separately; scenario-local environment changes remain explicit.
These fixtures do not inherit caller/provider markers, driver recursion flags or
color settings. Environment isolation does not remove process ancestry.

The native process selector resolves the repository build at
`rust/target/debug/tmt` by default and fails if it is absent. An explicit
descriptor may select another absolute native executable; it must be
executable, and neither an installed host command nor Node is an allowed
fallback. Paths and argv are passed as data, never through shell fragments.

Suites that start the local Office service (such as `office-storage`) need a
companion built with the embedded SPA, as CI does:
`(cd typescript && corepack pnpm office:build:local)`, then
`(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" cargo build --locked -p tmt-office --features local-service)`.
A later plain workspace `cargo build` or `cargo test` replaces
`rust/target/debug/tmt-office` without the service, and those suites then fail
with `OFFICE_SERVICE_UNAVAILABLE`; rebuild the companion before rerunning them.

Native Rust CI explicitly selects the same-checkout release CLI for process
contracts, so installed-companion integrity checks run with production compiler
optimization rather than debug hashing cost. The independent Office companion
and storage probe remain debug fixtures. Rust debug tests, Clippy, MSRV builds
and embedded service tests remain separate required checks; process deadlines
and assertions are unchanged. Local selection still defaults to the debug CLI.

CLI version expectations and Office installation/hook fixtures use the shared workspace reader
once per suite, selecting the relevant crate by name and running bounded
`cargo metadata --no-deps --offline --locked`. The reader also reads `rust/Cargo.lock` and
lists tracked files with `git ls-files -z`, so the suite needs a Git checkout. Cargo, the
lockfile and workspace resolution inputs must remain available even when selecting an explicit
CLI executable. The documented build below supplies the resolution inputs; the expectation
has no alternate version reader.

Build first, then explicitly select the test-only storage probe. The product CLI
uses its repository-native default; the probe is never an installed SQL command:

```bash
cargo build --locked --manifest-path rust/Cargo.toml
cargo build --locked --manifest-path rust/Cargo.toml --example storage-probe
TMT_TEST_STORAGE_PROBE='{"executable":"/absolute/checkout/rust/target/debug/examples/storage-probe","args":[]}' \
  pnpm test:native
```

Squad context fixtures separate successful core-invocation evidence from deadline
termination. Cold/fresh reads and a promptly returning stale-context sentinel
assert the cache-only gate independently. Timeout scenarios establish a gated
child in the hook's owned process group before exec, so descendant cleanup never
requires core to start within the 300 ms local or 200 ms outer deadline. Assert
SIGKILL, empty output, the sub-second bound, unchanged claims and child/group
absence. Before timed sections, freshly written context, provider, clipboard and
reply-notice host fixtures execute one reserved readiness branch with a 30 s
failure ceiling; it returns before every payload effect and the file is never rewritten afterward.
This keeps macOS first-exec assessment outside production deadlines. Do not add
readiness sleeps, retries or larger production budgets.

The maximum-body, 50-reply installed-companion page is an explicit load diagnostic,
not required process acceptance:

```bash
pnpm test:stress:native
```

For a deliberate explicit selection or a task-owned moved binary:

```bash
TMT_TEST_CLI='{"executable":"/absolute/checkout/rust/target/debug/tmt","args":[]}' \
TMT_TEST_STORAGE_PROBE='{"executable":"/absolute/checkout/rust/target/debug/examples/storage-probe","args":[]}' \
  pnpm test:native
```

The real-Herdr host test (`test/native/herdr.test.ts`) is skipped unless
`TMT_TEST_HERDR` names a pinned `herdr` binary (0.9.1). Download the release
asset into a scratch directory, never an install path, and check it against its
GitHub digest before use:

```bash
gh release download v0.9.1 -R herdrdev/herdr -p herdr-macos-aarch64 -D /tmp/hdrbin
gh api repos/herdrdev/herdr/releases/tags/v0.9.1 \
  -q '.assets[]|select(.name=="herdr-macos-aarch64")|.digest'   # compare:
shasum -a 256 /tmp/hdrbin/herdr-macos-aarch64
mv /tmp/hdrbin/herdr-macos-aarch64 /tmp/hdrbin/herdr && chmod 755 /tmp/hdrbin/herdr
TMT_TEST_HERDR=/tmp/hdrbin/herdr pnpm exec vp test run --config test/native/vitest.config.ts test/native/herdr.test.ts
```

It starts a headless server on a short private socket with update checks off,
approves the `tmt-driver-herdr` built beside the tested `tmt`, runs commands
inside its panes, and fails if any server process remains. Its carry-over case
also needs `TMT_TEST_PREVIOUS_TMT`, an absolute `tmt` built from a revision
before #1082 (when Herdr was built in) in its own worktree and target
directory; it binds with that build, then proves the binding waits for the
driver and carries over with its server ID once approved:

```bash
git worktree add --detach ../tmt-previous <revision before #1082>
(cd ../tmt-previous/rust && cargo build --locked -p tmt-cli --bin tmt)
TMT_TEST_HERDR=/tmp/hdrbin/herdr TMT_TEST_PREVIOUS_TMT="$(cd ../tmt-previous && pwd)/rust/target/debug/tmt" \
  pnpm exec vp test run --config test/native/vitest.config.ts test/native/herdr.test.ts
```

The Herdr host driver (the `tmt-driver-herdr` library, whose executable is a
`tmt-cli` bin shipped in the CLI archive) has its own executable test, which
uses the same pinned binary. It runs the protocol
conformance harness and every declared operation against a private server and
HOME, and fails if a server process remains. Its prompt case runs a
shell-script stand-in named `claude` in a pane, never a real agent. Without
`TMT_TEST_HERDR`, only the stand-in `herdr` conformance case runs. Its checks:

```bash
(cd rust && cargo test --locked -p tmt-driver-herdr)
(cd rust && TMT_TEST_HERDR=/tmp/hdrbin/herdr cargo test --locked -p tmt-cli --test herdr_driver)
(cd rust && cargo clippy --locked -p tmt-driver-herdr -p tmt-cli --all-targets -- -D warnings)
(cd rust && cargo test --locked -p tmt-adapters --lib host::external)
(cd rust && cargo test --locked -p tmt-cli --test architecture)
node typescript/scripts/release-please-config.mjs --check
```

A change to the protocol crate itself (wire types, decoding, `serve` or either
conformance harness) runs the crate's tests, which hold the conforming and
broken in-process fixtures for both driver kinds, and the host consumers
above, since the host harness also asks every runtime operation:

```bash
(cd rust && cargo test --locked -p tmt-driver-protocol)
(cd rust && cargo test --locked -p tmt-cli --test herdr_driver)
(cd rust && cargo test --locked -p tmt-adapters --lib host::external)
```

Runtime approval (#1266 PR A) is checked with
`cargo test --locked -p tmt-cli --test driver_command`. The isolated runtime
fixture covers disclosure, no-terminal refusal, both kinds in one registry,
claim/name conflicts, write-target containment, malformed locations and explicit
re-approval, timeout rollback and registry capacity. `DriverProcess::locations` is the shared `within(home)` admission
boundary for approval and future setup consumers. Runtime launch/hooks and the
kill/restart delivery case belong to PR B2, not approval evidence.

The suite covers grammar, configuration-before-effects, identity metadata and
binding lifecycle, role/preamble, response/receipts, exchanges/attention, inbox listening, talk,
local Office board grammar/persistence, managed skills and native installation. It uses bounded process budgets,
task-owned files and independent SQL/schema oracles. Frozen migration fixtures
and provenance under `typescript/test/fixtures/storage-history/` are immutable evidence;
do not generate expected data with the implementation under test.
For schema changes, update the independent native schema expectation in
`typescript/test/native/storage-fixture.ts` and the explicit migration/table assertions,
then run this complete process suite before pushing. Rust storage tests do not
replace process-level migration and future-version rejection tests.

Runtime resume mappings have deterministic adapter tests and a separate opt-in
provider parser contract. With the explicitly selected Claude Code 2.1.283 and
Codex CLI 0.157.1 executables already installed, run:

```bash
cargo run --locked --manifest-path rust/Cargo.toml -p tmt-adapters \
  --example runtime-contract -- /absolute/claude /absolute/codex
```

This invokes only `--version` and the driver's generated resume arguments with
`--help`, under bounded process ownership. It neither installs providers nor
starts a model conversation. A different version fails rather than silently
expanding support; review provider behavior before updating the pinned contract.
Help-parser acceptance does not prove that a session can actually resume: the
manual lifecycle evidence in issue #321 owns that distinction, including the
Codex cross-mode limitation. Normal `tmt run` does not execute this developer
check or enforce these version pins on user commands.

The Claude channel provider has its own opt-in check against the supported range
and the builds with recorded channel evidence (see the
[channel contract](contracts/claude-channel-v1.md)):

```bash
cargo run --locked --manifest-path rust/Cargo.toml -p tmt-adapters \
  --example channel-contract -- /absolute/claude
```

It runs only `--version` and `--help`, fails outside the range and says when the build
is accepted but untested. `tmt run --channel` applies the same range rule to the user's
command before it binds or spawns anything.

`tmt whoami --context [--json]` is the read-only rehydration entry point. It reports
the verified caller identity and lifetime, up to 500 characters of role text,
an existing saved notebook path (not its contents), and unacknowledged originated
and incoming X counts with explicit-identity inspect commands. The complete output
is limited to 4 KiB, with a truncation marker when role/path content is shortened.
Only a verified empty pane receives the binding hint. Unavailable or ambiguous
evidence returns empty human output or JSON `status: "unavailable"`, successfully,
without initializing configuration, storage or tmux metadata.
Ordinary `whoami` keeps its existing human output and
adds `interfaceKind` and `sessionState` to its JSON projection.
An already admitted provider's verified prompt-submit context adds one incoming
unacknowledged-attention line with an inbox pull command only when the count is
nonzero. It reuses the same read-only snapshot; attention is not an unsent count.
Zero attention and zero extension contributions emit no prompt context, and
foreign, unadmitted or ended sessions receive none. The
`identity-context-requests` mock-runtime scenarios verify these gates, unchanged
request/attention state and cleanup; CLI formatter tests retain the 4 KiB bound.

The `identity-context` and `identity-context-requests` Docker scenarios own bound
context acceptance. Their independent SQLite snapshots include verification
timestamps, retention and both participants' attention state: context reads must
not change them. `typescript/test/native/context.test.ts` owns unbound/missing
configuration no-side-effect checks; Rust storage/formatter tests own retained
counts, role limits, escaping and the complete serialized byte bound.

`typescript/test/native/inbox.test.ts` owns the real no-tmux queue -> bounded listen ->
detail/receipt -> reply -> result path. It uses isolated SQLite, verifies compact
listen output excludes receipts and bodies, preserves participant-scoped
acknowledgment, and starts no Office process. Rust request/storage tests own route
validity, revision CAS, migration and indexed observation. Fake-clock unit tests
own debounce/deadline timing without a real 15-minute wait.

The tooling unit suite is independent developer-tool coverage. Its denominator
must contain no deleted TypeScript source and must not present Rust as a
cross-language percentage. Run focused tooling tests with:

```bash
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling)
```

Native process tests must prove the missing-native negative control and selected
native positive control. Child processes are finite, are stopped and reaped
before fixture deletion, and receive signals only when they are task-owned.
Tests never use host tmux, global provider state, or process-wide environment
mutation as setup.

Office companion scenarios are grouped under `test/native/office-*.test.ts`; retained-install
fixtures use `__native-install` without acquiring or publishing a product release.
Frozen public Office acquisition refusal is asserted separately in `office-freeze.test.ts`.

Use `withSandbox` for callback-owned native fixtures. Its descriptor clones
share active runs; disposal stops outstanding commands before deleting files
and rejects later launches. Each run has its execution deadline plus at most
one second to confirm direct close and process-group exit. Signal or initial
group-probe EPERM is tolerated only after child close and a subsequent ESRCH
group probe; live or unknown groups and other initial probe or signal errors
still fail cleanup. An unconfirmed group is never signalled. Unconfirmed cleanup
fails and reports the retained fixture path instead of deleting potentially
live state. Focused lifecycle regressions live in `typescript/test/tooling/cli-process.test.ts`;
they use explicit Node fixtures, not a product-runtime fallback.
On Linux, after registered runs stop, the post-callback cwd guard rejects live
inspectable same-user processes inside the sandbox, re-verifies ownership before terminating them,
and confirms absence before deleting files using `/proc`. The guard and its
detached-service regression are skipped on macOS and other platforms; no
user-wide process scan runs there. Linux discovery skips permission-denied entries
(such as nondumpable system processes), so it cannot detect residents whose cwd
is unreadable. A detected leak fails even when termination succeeds; failed
re-inspection of a verified resident or termination retains the fixture. Detached
Office service coverage is in `test/native/office-uninstall.test.ts`. Scenarios must still stop services
they start through their existing control/receipt path on every callback exit.

### Optional performance probes

Performance probes are developer tools, not timing gates in ordinary CI. They use
the executable selectors above and the existing isolated fixtures. Never measure an
installed host command by accident, and never fall back to another runtime.

**Startup resources (macOS).** From the repository root, build a release CLI,
select it, and run the probe at least twice:

```sh
cargo build --locked --release --manifest-path rust/Cargo.toml
export TMT_TEST_CLI="{\"executable\":\"$PWD/rust/target/release/tmt\",\"args\":[]}"
node typescript/scripts/benchmark-startup.mjs > "$TMPDIR/tmt-startup-run-1.json"
node typescript/scripts/benchmark-startup.mjs > "$TMPDIR/tmt-startup-run-2.json"
```

The probe uses macOS `/usr/bin/time -lp` and the bounded packed-command runner. It
creates and removes private home/config/workspace state, omits caller context,
blocks PATH-based tmux execution and checks the output. Each create uses fresh
storage; show reopens that identity. It never installs globally. CPU includes
waited descendants; maximum RSS is in bytes. Linux resource accounting is not
implemented, and denied kernel statistics are a measurement failure, not zero.

**Private-tmux latency (Docker).** Use a task-owned image tagged for the worktree
as described in [the disk section](#keep-local-development-from-filling-the-disk)
(it defines `$worktree`), run `scripts/dev-disk-check.sh` first, and never run these tmux scenarios on the
host:

```sh
docker build --build-arg TMT_NATIVE_PROFILE=release -f typescript/test/e2e/Dockerfile -t "tmt-performance:$worktree" .
docker run --rm --init --network none \
  -e TMT_PERFORMANCE_BASELINE=1 "tmt-performance:$worktree" \
  sh -c 'cd /workspace/typescript && pnpm exec vp test run --config test/e2e/vitest.config.ts test/e2e/performance-baseline.e2e.test.ts'
docker image rm "tmt-performance:$worktree"
```

Repeat the run before removing the image. Add `-e TMT_PERFORMANCE_SLOW_REPLY=1`
for the controlled 1,500 ms mock reply delay. The image places the selected
profile at the shared selector's default path, and mock replies inherit it;
regression builds stay debug. The scenario emits a `TMT_PERFORMANCE_BASELINE` JSON
report and is otherwise skipped. Setup of 200 extra unbound panes is outside the
measurements. Trace counts include fixture caller-session discovery, not all OS
subprocesses. Assertions gate causal behavior, scoped subprocess bounds and
cleanup, never latency; a report followed by failed teardown is invalid evidence.
Talk timings include the 500 ms paste/Enter delay and observer policy, and a
mock reply can move completion by a whole one-second poll interval, so do not
subtract the injected peer delay and call the remainder processing time.

**Comparison rules.**

- Keep all raw samples, the first observation, the median and the range. A fresh
  process is not a cold machine or cache, and seven samples are not a tail-latency
  study.
- Record the source revision and dirty diff, selected executable and peer,
  toolchain, locked dependencies, profile, target/linkage and binary size. Use
  the same machine, resources, fixture sizes and command defaults, and interleave
  at least two runs with seven samples per repeated scenario.
- Correctness, compatibility and cleanup gates stay mandatory. A repeated scoped
  median regression above 15% is a review trigger, not a timing assertion.
  Report the talk delay and poll policy separately.
- Preserve the deterministic scoped subprocess bounds and inspect output size and
  scope. Do not reduce production delays to advertise speed, or claim unmeasured
  throughput, memory, platform or release behavior.

### Squad extension

`typescript/test/native/squad.test.ts` runs `rust/target/debug/tmt-squad`, or an
absolute path in `TMT_TEST_SQUAD`, through real `tmt` dispatch. It uses a
sandbox PATH holding the `tmt-squad` and `tmt-sq` links, with no installed-copy
fallback. It observes rooms and metadata through an independent SQLite reader.
A workspace `cargo build --locked` produces the default executable. Squad unit
tests run with `cargo test --locked -p tmt-squad`. For dependency changes,
compare `cargo tree -p tmt-cli -e normal,build -f '{p} {f}'` with `main` and the
package-scoped release `tmt` (see Rust checks) to prove the CLI is unchanged. Notebook link checks cover wrapped Unicode
hits, target previews, configured overrides, inert unknown schemes, argv isolation
and sender/member/open-request revalidation. Parsing and paint must perform no actions.

Native Squad tests verify leadership selection and clearing without membership
or role loss, repeated additions without overwriting state, and explicit recovery
from a squad without a lead. The Docker Squad lifecycle test kills temporary and
saved panes before the first list read, then checks both the returned roster and
SQLite retirement/binding state. Run lifecycle coverage twice to check cleanup.

Tab parity is checked by `built_in_board_documents_equal_ls_tab_documents` and
`user_board_and_ls_share_members_sections_bindings_and_failed_reads`: board views
and `ls --tab` must have identical projected documents and row-grid metadata,
including hidden squads/tabs, source section deduplication, repeated user section
matches, cross-squad memberships, and partial-read failure/recovery evidence.
`board::home::tests` verifies the board-only retained model against that same
aggregate document and text, shared section matches, per-squad membership
counts, request/observed age provenance, bounded acquisition calls and partial
failure recovery. The model does not change painting; frozen board parity stays
unchanged. Use an isolated `XDG_CACHE_HOME` when testing board observation.

User tab validation happens during Config reading, including hidden definitions. `--tab` conflicts
with `--squad` and `--refresh-fields`; aggregate reads do not run providers.

Notes cursor tests cover source-line mapping through Markdown wrapping and
unsupported constructs, wrapped continuation paging/click placement, resize,
content insertion/deletion, duplicate anchors and per-line annotation composer
cancellation/empty submission. Request projection tests cover bounded escaped
quotes, sender/current-lead checks and marker removal after an answer. A
painted-cell regression proves markers cannot clip wrapped content; selection
covers each wrapped continuation to the pane edge. Display-only annotation
quotes strip list prefixes and truncate without changing durable source tags. Check the
focused cursor and sent marker in `tmt`, `tmt-light` and `NO_COLOR`; notes keep
the shared `Scrolls` viewport owner.

For the completed-request meter, `board::rate`, `board::meter` and the view's
backend recorder cover four-window bucket boundaries, bounded tab retention,
no-data/zero/gap aging, key overrides/text inputs, easing and
retargeting, meter-only emitted coordinates, and disabled buffer/ANSI equality.
Verify both enabled/reduced-motion policies with the normal renderer. Measure
matched 60-second idle off/on/reduced-motion process CPU time with isolated
public-command fixtures; incremental usage must stay below 0.5 percentage point
of one core. Record sampling child counts, emitted-cell coordinates and
terminal dimensions with the PR's dark/light/NO_COLOR/narrow captures. Timing
measurements are local evidence, not a flaky CI threshold.

### Provider setup and lifecycle verification

Setup planning/publication tests use disposable settings files and preserve user
hook/permission bytes, exact reruns, recovery copies and changed-input refusal.
Claude cases cover unset/empty, absolute and relative `CLAUDE_CONFIG_DIR` roots
across settings, skills, detection and transcript admission. Symlinked settings
(including dangling links and replacement after planning) are preserved along
with their target, with actionable human/JSON refusal. Backup tests independently
compare exact bytes and private directory/file modes, preserve legacy copies,
and prove publication continues with a directory/manual-cleanup warning above
32 backups in `.tmt-setup-backups`. Unknown Claude `SessionEnd` reasons yield
no lifecycle observation, with a known terminal reason as the positive control;
rejection is not process-exit evidence.
`test/native/setup.test.ts` owns CLI consent, stable-launcher repair/removal and
the always-zero bounded hook failure contract, including custom `CODEX_HOME`
without trust/config mutation. Runtime adapter tests own Claude/Codex payload
mapping, pane-ancestor evidence, continuation transitions and stale-end rejection;
core/storage tests retain session CAS, unique exact-thread lookup and transaction
ownership. Docker E2E owns real pane/process integration, including Codex's
independent-to-shared transition and unmapped shared rejection. Fixture hook
execution proves TMT integration, not provider-version compatibility or trust UI.
Manual provider acceptance must use a disposable identity/window and isolated
provider settings; installing hooks into the user's real global settings needs explicit
consent. No test invokes setup against the user's actual provider directory.

Codex channel product scenarios live in `typescript/test/e2e/codex-channel.e2e.test.ts`.
They use the existing private E2E fixture and a model-free Rust
`codex-channel-fixture` example, built into the E2E image only. For a local
focused run, build both `tmt-cli` and that example; no installed provider or
credentials are used. The tests independently check native queue receipts,
durable replies, the shared enrollment/pane gates, and per-pane terminal writes
with Default and explicit `--no-channel` plain-session positive controls.
Explicit `--channel` launch and exact resume cover qualified enrollment and
strict refusal; Default bypasses channel setup without a fallback notice.
The scenarios also cover unchanged original resume argv,
thread-ID mismatch rejection, retained-evidence refusal, and Ctrl-C cleanup.
Launcher unit tests toggle the channel port's advertised default independently
of provider names. Provider live continuity evidence is
separate; see [the contract](contracts/codex-channel-v1.md#verification-boundaries).

`reply-batching` owns fixed pane windows, disabled grouping, uncertainty and
binding-replacement fencing, real attached PTY key debounce, the 30 s usability
bound, and worker/log cleanup. Its Python fixture owns and reaps a normal tmux
client so actual terminal key bytes exercise `client_activity`; control-mode
`send-keys` is not user key activity. `claude-channel` retains individual channel
notice routing. Storage adapter tests own durable batch reopen and claim CAS;
native configuration tests own global editing and bounds.

## Docker E2E

Run the full private tmux/caller lifecycle harness twice for lifecycle,
transport, identity, talk, or cleanup changes:

```bash
(cd typescript && corepack pnpm test:e2e)
(cd typescript && corepack pnpm test:e2e)
```

`TMT_E2E_FILES="squad.e2e.test.ts"` (space-separated plain file names) limits the run to those
E2E files (the image passes them to vitest as anchored `test/e2e/<name>` paths, because vitest
matches a filter by substring and a bare `routing.e2e.test.ts` would also run
`check-routing.e2e.test.ts` and `session-routing.e2e.test.ts`), and `TMT_E2E_ADAPTER_TESTS=0` skips the Rust adapter tests. A host `CARGO_BUILD_JOBS` (a
positive count or `default`) limits the image's cargo builds; unset, they use every CPU of the
Docker VM. CI runs the suite as two
shard jobs behind the required `Docker E2E` gate, each with its own file list from
`typescript/scripts/e2e-shards.mjs`, balanced by the seconds in
`typescript/test/e2e/shard-weights.json` (refresh them from a full run when the shards drift
apart; a missing or stale weight only costs balance, and a guard fails if any scenario file is
in no shard or two).

The harness builds its pinned image, uses `--network none`, private tmux
sockets and deterministic mock agents, and selects the Docker-built native
executable. It must not connect to a host tmux server or a real agent. The
wrapper's `--init` behavior is part of orphan-child cleanup evidence.
The image puts its selected dev/release binary at `rust/target/debug/tmt` inside
the fixture and leaves the shared selector unset. This exercises the same
default path as a developer checkout, including nested replies; explicit test
descriptors still override it. There is no second default executable owner.

### Scenario ownership

Name Docker scenarios for the behavior or adapter they verify, not a retired
implementation language. All public-command scenarios already use Rust.
Keep these boundaries when choosing where a regression belongs:

| E2E file(s)                                                    | Distinct evidence                                                                                                                     |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `binding-reconciliation`                                       | Publication races, uncertain endpoint evidence, scoped/global discovery, binding transitions and live listing presentation            |
| `marked-binding`                                               | Explicit marked-pane selection, frozen target evidence, server isolation, mark preservation and binding lifecycle reuse               |
| `durable-identity`, `identity-lifecycle`, `identity-retention` | Stored identity/profile continuity, pane/server lifecycle and retirement consequences                                                 |
| `check-routing` versus `capture-limits`                        | Real target/server resolution and capture failures versus numeric/configuration bounds                                                |
| `profile-binding` versus `role-lifecycle`                      | Verified caller/retired-name ownership versus restart, file input, validation and concurrent writes                                   |
| `talk-completion` versus `durable-talk`                        | Observer interruption, output/attribution and uncertain delivery versus inline input, size bounds, concurrency and submission retries |
| `exchange-watermarks` versus `exchange-attention`              | Gated revision/watermark state and exact late finals versus config isolation, redaction and rebind access                             |
| `tmux-adapter`, `transport-adapter`                            | Explicit adapter-probe evidence: caller/inventory/markers and delivery/capture stages; not public CLI success                         |
| `pane-badge`                                                   | Default-off behavior, opt-in updates, theme preservation, rendering, conflicts and cleanup                                            |
| `executable-selection`, `smoke`                                | Harness selection, causal nested replies, startup and cleanup controls                                                                |
| `claude-channel`                                               | Claude channel delivery against a mock `claude`: no paste to an opted-in pane, crash cleanup, plain paste kept                        |

Similar commands do not imply duplicate evidence: native-process tests inspect
the executable's public contracts and independent stored state, while Docker
adds real tmux/process ordering. Consolidate only after mapping setup, causal
action, assertions and failure/cleanup observations. The redundant basic badge
case formerly in the binding suite is owned by the stronger `pane-badge` cases;
do not add another copy there.

`typescript/test/e2e/cli-assertions.ts` owns `expectJsonResult` for the E2E result shape.
It checks the success envelope and returns the same parsed value; it does not
validate domain fields. Scenarios retain their exact/partial payload assertions
and independent SQL oracles. Helpers with a different stderr or parse contract
remain local. Do not combine partial identity views into a permissive shared
schema or import product types to manufacture expected results.

Docker scenario imports use the `typescript/test/e2e/harness.ts` facade; the
fixture, readiness, cleanup and type modules live under
`typescript/test/e2e/harness/`. The
[architecture map](ARCHITECTURE.md#testing-and-evidence-boundaries) defines their ownership.
Synchronous fixture tmux calls fail after five seconds and kill the client;
scenario timeouts remain required. Install a fixture's tmux trace once, then
reuse its `clear()` method between actions; a second installation is refused.

Shared cross-suite utilities belong in `typescript/test/support/`; suite-only harness,
assertions and observers stay with their suite. Focused helper tests belong in
`typescript/test/tooling/` and must prove rejection as well as positive behavior.
The tooling [import-direction guard](ARCHITECTURE.md#testing-and-evidence-boundaries)
checks these suite/support and harness boundaries without starting Docker.

For ordinary developer checks, run:

```bash
(cd typescript && corepack pnpm check)
(cd typescript && corepack pnpm test:run)
```

The nested `pnpm check` script is the common quality entrypoint for all retained
tooling, including E2E. For focused work use `pnpm type:check`, `pnpm lint` or
`pnpm format:check` from `typescript/`; the old duplicate `e2e:*` quality aliases
are removed. `pnpm test:e2e` remains the actual Docker scenario runner.

The first command checks TypeScript types and lint/format for retained tooling
and docs; the second runs tooling behavior and source-boundary tests. Neither
replace the Rust commands, native process suite or Docker runs.

## Runtime smoke matrix

The six native smoke environments required on full-scope PRs are:

- macOS x64 and macOS arm64;
- Linux glibc x64 and Linux glibc arm64;
- Linux musl x64 and Linux musl arm64.

CI builds four raw targets once (the two macOS targets and two static Linux musl
targets) and reuses the matching static musl executable for both Linux smoke
environments. Preserve the `Packed install (<environment>)` check names and
`Native package matrix` final aggregator; those names retain historical CI
compatibility, while their
steps are real native runtime checks rather than npm package checks. Each smoke
runs outside the checkout with isolated HOME/state, no Node or Rust on the
product PATH, and checks version/help, exact embedded skill bytes, managed skill installation and
SQLite reopen/persistence. Cross-compilation alone is never claimed as runtime
evidence.

Merge-group runs retain the two Linux builds and four Linux smoke rows. They
skip the separate macOS build and install jobs, whose `skipped` results the
aggregate requires explicitly only on that event. PRs still run both macOS
architectures, and native release workflows still build and verify macOS before
publication. The queue tests each cumulative group head (HEADGREEN).

The same distinction applies to release artifacts: raw PR executables prove
source-runtime behavior only. They do not prove archive inventory, notices,
checksums or bootstrap behavior. Linkage is checked in both raw and archive proof.

## Native release verification

For archive, installer, upgrade, bootstrap or publication work, read this entire
section before acting. Run commands from the repository root unless stated otherwise.
It owns the exact commands and actual-archive evidence; verification guidance is
not publication authorization. Ordinary changes use the focused checks above.
The runtime smoke matrix remains part of native PR verification; raw executables
are not proof of release archives or public installation.

### Native Rust release archives

#### Explicit multi-platform release preparation

`Native release artifacts` (`.github/workflows/native-release.yml`) is the per-product
release run, dispatched with an explicit `cli` or `squad` product, not part of
every PR. Office is frozen: its component is parked and neither preparation nor draft
publication accepts it; existing releases remain untouched. It has two modes. The default `prepare` builds and verifies one bundle from the
current main commit without a draft release and attaches nothing: dispatch each authorized
product on the release's reviewed, required-checks-green main commit and record the
product, run ID and exact SHA in its issue. With `prepare` off, the run plans the
product's draft releases that carry neither a verified bundle nor a recorded failure,
oldest first, and builds, verifies and attaches each one at its own commit
(`target_commitish`), one at a time. `.github/workflows/native-release-bundle.yml` is the
pipeline it calls once per draft. The pipeline builds on native macOS arm64/x64 and Linux
arm64/x64 hosts using the existing pinned tools and `build-native-artifact.sh`. A shared
matrix keeps build and final verification hosts aligned; dispatches outside main are
skipped. Cached packaging tools are keyed by OS, architecture and exact tool versions;
they are developer tools only. Rust dependency caches are per product and target and are
written by main only.

The draft release carries the state of its own build. A draft with
`release-publication.json` has a complete bundle: the archives, the final manifest and, for
the CLI, both installers are uploaded first, their digests compared with the local bytes,
and that file last, after every final verifier passed. A draft with
`verification-failed.json` (run URL, commit, failed jobs) is parked: later runs list it in
their summary and skip it. A cancelled run records nothing and is retried. Retry a parked
draft by dispatching the run with `prepare` off and `retry` set to its tag, or delete the
draft. Each product has one queued run group (`release-<product>`); GitHub keeps one pending
run per group and replaces it, which loses nothing because a run plans from the drafts
when it starts. The run asserts that the draft tag is the tag prefix and Cargo version of
its commit, and it never creates, edits or publishes a release.

cargo-dist itself merges the downloaded `*-dist-manifest.json` inputs through
`dist build --artifacts global --output-format=json --no-local-paths`. Do not
hand-merge artifact JSON or enable another installer. Generate a complete
`dist plan` on the same source. For CLI, pass it as bootstrap `--plan`: the
generator requires exact planned archive names/targets, preventing a missing
matrix target from silently shrinking the release. Bootstrap generation verifies
TMT ownership and archive inventory/digests before generating code; the final
CLI matrix executes both existing verifiers and compares the regenerated script
bytes. Office and Squad have no bootstrap and run their product-specific
archive/runtime verifiers against the same final-manifest ownership instead. All
jobs in the selected product run must pass before that product is published,
even if the assembled artifact can already be downloaded. CI artifacts expire
in seven days. Product-qualified artifact names prevent concurrent product runs
from being mistaken for one bundle. Notices alongside each bundle are
verification inputs; every archive also contains its own target-filtered notices.

`Release` (`.github/workflows/release.yml`) runs on every push to `main`, documentation
included (a merge of any kind moves `main` under the open release pull requests), and on a
manual dispatch with `dry_run` (default on). Its `release-please` job
runs the pinned release-please CLI (`.github/release-please`, exact version and lockfile
integrity) against the generated `release-please-config.json` and
`.release-please-manifest.json`: it opens one release pull request per released component, and when
one is merged it creates the draft release (release-please's drafts, so a published release
never has to receive assets). A live run, which is only allowed on `main`, creates a GitHub
App token in that job alone, enables auto-merge for one open release PR at a time (the merge queue sets the strategy),
through the normal required checks and merge queue. An enabled or queued release PR
blocks enabling another component until it merges. The workflow does not refresh a
BEHIND branch: the queue tests the combined result on current main, including required
checks. release-please retains `always-update` for conflict recovery, subject to the
[queued-PR pre-check](#queued-release-pull-requests), but the pinned wrapper suppresses
an update when the title, complete inline notes, generated release-file bytes and modes
already match the observed immutable head and GitHub does not report a conflict. This
preserves running CI across main pushes that do not change the release content. Changed
release content, missing files, confirmed conflicts and overflow notes use the original
updater. Unknown mergeability preserves an otherwise unchanged head and returns normally,
so `github-release` still reconciles merged release PRs in the same invocation; acquisition
errors fail visibly and can be retried on the next main push. New releasable commits
may still restart CI; this policy does not promise bounded latency under continuously
changing release content. A `dispatch` job
then starts the per-product run above for every
product that has a draft without a bundle. The job runs in the `release` Environment and the
App credentials, `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, are secrets of that
Environment, not repository secrets, so only a run its deployment branch rule admits can read
them; the step that decides the mode is told whether they exist, never their values. Until
both secrets exist a push is a dry run: `release-please` runs with `--dry-run` and the run
summary shows what it would open, tag and start; nothing is created. A manual run with
`dry_run` off and no secrets fails instead of falling back, and so does one on any ref but
`main`. Neither job publishes.

Owner setup, once, when the release App exists: create the Environment `release` and limit
its deployment branches to `main`; add `RELEASE_APP_ID` (the numeric App ID) and
`RELEASE_APP_PRIVATE_KEY` as secrets of that Environment; install the App on this repository
only, with Contents and Pull requests read/write and no webhook. GitHub creates the
Environment without a rule the first time the workflow names it, and the secrets are added
only after the rule exists. Once the rule exists a dispatch from any other ref, even a dry
one, is refused by the Environment. Do not add repository secrets of the same names: those
are readable from every ref.

Once a draft's bundle is attached, the run evaluates the publication gates in order:
`channel` (the version is an alpha, `X.Y.Z-alpha.N`; a stable version or any other pre-release
label such as `beta` or `rc` is held, and releasing that hold is refused: the owner publishes it
by hand), `commit` (the release's commit is on `main` and the pull request that produced it passed
`Code quality`, `Unit tests`, `Docker E2E` and `Native package matrix`), `immutability` (the
repository's newest published release is immutable, which shows that the setting was on; the
workflow token cannot read the setting itself), `monotonic` (the release is newer than every
published release of its product), `migration` (no commit of the release carries `!` or a
`BREAKING CHANGE:` footer; outside the alpha channel the component's migration list, named in
`.github/components.json`, also has no more entries than at the product's last published
release, while an alpha publishes new entries and the gate's summary only reports them) and
`upgrade` (the proof above, which for the CLI includes migrating state the previous release
wrote; the first release of a product has nothing to upgrade from). A failed gate does
not make the draft a failed build. The draft gets `publication-held.json` (`tag`, `sha`,
`gate`, `reason`, `runUrl`, `recordedAt`), and later runs list it as held and leave it alone.
The jobs that evaluate the gates hold the write token, so they run `main`'s code and only read
the release commit's data through git and the API. To release a hold once its cause is dealt
with, publish the draft by hand as below, or dispatch `native-release.yml` on `main` with the
product, `prepare` off and `hold` set to the tag: the run evaluates the gates again without
the one gate the marker names (never another, and never `channel`), removes the marker when
they pass and then publishes the draft as below.

An owner-authorized `rerun=<tag>` dispatch on main with `prepare` off instead re-proves
all gates, including the gate named in `publication-held.json`; it skips none. `retry`,
`hold` and `rerun` are mutually exclusive. The selected tag must be a bundled, held
product draft with a commit target. The gates validate the marker's tag, SHA and known
gate, and finish refuses a changed gate. Any failed gate leaves the original marker
unchanged; only after all gates pass is it removed, followed by normal publication,
attestation and public-install smoke checks. The
[release skill](.agents/skills/tmt-release/SKILL.md#automated-alpha-publication) owns rerun authorization.

Rerun uses the main commit selected by the dispatch for Node verifier scripts and
their locked dependencies. Archives, manifest, version and digests come from the
draft; CLI expected skill bytes, migration counts and applicable Rust adapter
acceptance code come from a separate checkout of its release SHA (`release-source`).
The read-only proof job compiles that source's adapter test with its own locked
dependencies and toolchain pin; release-source Node verifier scripts do not run.
Ordinary upgrade proof retains release-commit tooling. This separates repaired tooling
from the unchanged candidate under test without rebuilding or replacing its assets.

Fixture-only verification (no dispatch or Docker):

```bash
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/plan-release-builds.test.ts test/tooling/publication-gates-script.test.ts test/tooling/release-upgrade.test.ts test/tooling/release-workflow.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-upgrade.yml
```

Publication is authorized by the owner. The owner chose a trunk-based alpha channel, and that
choice is the standing authorization, recorded in the release skill, for the release pipeline
to publish an alpha draft that passes every gate above; everything a gate holds, every stable
release and every publication by hand needs the owner's explicit authorization.

When every gate passes, the `publish` job publishes the draft. `release-publish.mjs publish`
reads the draft again and refuses unless its version is an alpha, its component is released
(`release: false` in `.github/components.json` parks a component: release-please opens nothing
for it, the planner leaves its drafts alone and this command refuses them, so a draft that
predates the flag cannot publish), and it carries the bundle and neither
`publication-held.json` nor `verification-failed.json`; then one `gh release edit <tag>
--draft=false --prerelease=<bool> --latest=<bool>` applies the product's policy below. Both
flags are explicit because release-please makes every draft a prerelease. The `published` job
then reads the release back: it is public and `immutable: true`, its flags are the policy's (a
CLI release is the repository's latest release, an extension release never is), its tag is on
the release commit, it carries `release-publication.json`, and GitHub's attestation verifies
(`gh release verify`, and `gh release verify-asset` for every asset downloaded from the
published release). GitHub finishes the attestation after publishing, so these checks are
retried for about two minutes. A failed check opens an issue and fails the run; nothing is
rolled back, because a published release is immutable and a repair needs a new reviewed
version. A `smoke` job then installs the published release as a user does
(`.github/workflows/native-release-smoke.yml`, also run by hand with `product` and `tag`, for
the newest published release of the product only: it installs what the public entry points serve
now, so any other tag fails its first check and a failed run reports on the issue like any
other). On the four hosts of the upgrade proof, in an isolated home, state directory
and prefix and with no token, a CLI alpha goes through the public
`releases/latest/download/install.sh`: the installer names the tag's version, the installed `tmt`
is the one PATH selects and reports that version, the installed shared skills are the tag's
`skills/*` (same names, same `SKILL.md`), and `tmt upgrade --channel alpha --json` reads the live
metadata and reports the installation current (a newer alpha that appeared since passes with a
note). An extension alpha is installed by the newest published CLI's `tmt extension install
<extension>` into a separate prefix; `tmt extension list` must report the tag's version and no
CLI link may appear. The tag is
checked out only so its skills can be read; none of its code runs. All smoke acquisition stays
unauthenticated. Only a native JSON failure with the classified `GitHub API rate limit:
reset/earliest retry time ...` cause may retry its failed acquisition step: at most two
attempts, waiting until the diagnostic's UTC epoch plus one second, with a five-minute
wait limit. Missing timing, a reset beyond the limit or a repeated limit fails clearly with
`github-api-rate-limit` infrastructure data. Any other failure (including a bare 403/429,
mixed diagnostics or a real error after retry) fails immediately. Separately, the CLI
latest-installer read may retry only when it embeds an older alpha than the just-published
tag (three reads, two 20-second waits); unchanged lag still fails, while a newer or
malformed version and download errors fail immediately. A tooling contract test pins the
Rust diagnostic format, UTC epoch representation and reasons consumed by smoke.
The smoke job has a 25-minute bound. Infrastructure-only failures open a separate
`Release <tag> public install blocked by GitHub API rate limit` issue; they do not claim a
broken release. Mixed or real failures retain the post-publication failure issue. Both
conclusions fail the job and retain artifacts from all four hosts. Missing or malformed
host evidence cannot establish an infrastructure-only failure; nothing is silently accepted.
The host artifacts are qualified by product, tag and target and retain the classified
diagnostic and its parsed UTC reset epoch. The smoke report's shared `smokeFailureOutcome`
output schedules only an infrastructure-only failure through a dedicated `actions: write`
dispatch job. It explicitly POSTs `native-release-smoke-retry.yml` on main with the source
run ID/attempt, product, tag and affected targets: `workflow_dispatch` is exempt from
`GITHUB_TOKEN` event suppression, while completion-triggered `workflow_run` is unreliable
for automatically dispatched native runs. Reusable callers propagate this capability;
install and issue-report jobs do not request it.
`public-install-retry.mjs` requires four distinct matching-host results per tag, validates
the exact diagnostic through the smoke owner's parser, and selects only the classified
failed acquisition targets. Unclassified hosts are never retried. Missing timing,
inconsistent evidence or a reset more than 60 minutes away keeps the original failure.
The retry planner verifies the source through REST: the current attempt of a same-repository
main `workflow_dispatch` run of `native-release.yml` or standalone `native-release-smoke.yml`,
with four matching concluded target jobs and product/tag-qualified artifact conclusions.
Requested targets must each be classified and reset-eligible; only those targets are retried,
while unrequested host failures retain their original conclusions. Dispatch inputs cannot
fabricate a recovered host, select healthy targets or recursively retry a retry run.
Host artifacts record their run attempt; artifacts from an earlier attempt cannot
prove the current one. A partial manual rerun without four current-attempt results
retains the issue and does not schedule recovery.

One Linux job waits until the last selected reset plus one second, bounded at 60 minutes;
each selected target then repeats the public install on its matching host with `--retry`,
which allows one acquisition attempt without another rate-limit retry. Source and retry
install jobs share `.github/actions/public-install-smoke`: it owns the tag data
checkout, Node setup and verifier invocation, including any host architecture wrapper.
Fixtures require the source matrix's target/runner pairs to equal the retry planner's `TARGETS`.
The source dispatch finishes promptly; the long wait runs solely in the independent workflow, whose
source-run/attempt/product/tag `public-install-retry-...` group never holds `release-<product>`
and cannot block the next release. Only the planner has `actions: read` to verify the source
run/jobs and download its artifacts;
install legs have `contents: read`, no token on the acquisition path, and tag checkouts
read solely as data. A separate `issues: write` reporter reconciles retry evidence with
all original hosts. Full recovery comments on and closes the infrastructure issue,
naming both runs; repeated classified failures, real failures and missing retry evidence
retain failure reporting. Real post-publication issues are never closed by this retry.
Recovery also leaves an infrastructure issue open when its latest reporter-owned
failure names a different run. Shared hosted-runner quota can be exhausted again
after the reset; that result stays failed.
Original failed jobs remain failed, and a failed retry also fails its run. A newer release
appearing during the wait can still fail the existing latest-installer/version checks;
the retry does not relax them or rerun publication.

Fixture-only checks for smoke and deferred reset recovery (no live install or dispatch):

```bash
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/verify-public-install.test.ts test/tooling/public-install-retry.test.ts test/tooling/release-publish.test.ts test/tooling/release-workflow.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/native-release-smoke.yml .github/workflows/native-release-smoke-retry.yml .github/workflows/native-release-bundle.yml
```

A failed leg keeps its failed checks as data, and a final job
with `issues: write` comments on, or opens, the issue of the checks above; nothing is rolled
back. Both `publish` and `published` run `main`'s code and never the release commit's: `publish`
holds the write token, `published` only read access and `issues: write`; the install legs have
neither. A run that stopped before it published is completed by the
next run of the product: it plans every complete draft without a hold again and evaluates its
gates again. A bundle prepared without a draft (`prepare`) never publishes.

For a manual publication, verify the selected product run's exact commit and all required PR
checks, and enable GitHub release immutability before creating a draft release.

Each bundle carries `release-publication.json` from
`typescript/scripts/native-release-policy.mjs`; create the draft with its
`flags` (`gh release create <tag> --draft <flags> …`). The CLI release is
published as a normal release with `--latest=true`, so
`releases/latest/download/install.sh` reaches its installer; alpha status stays
in the version and title. Office and Squad releases keep `--prerelease` and
`--latest=false` and can never become latest. `tmt upgrade` accepts a CLI
pre-release published either way (earlier alphas were flagged prereleases) but
never a stable CLI flagged prerelease, and accepts an extension release only when
its flag matches whether its version is a pre-release
(`Product::accepts_prerelease_flag`). After a manual publication, check
`node typescript/scripts/release-policy.mjs --check-latest "$(gh api repos/pj-tmt/tmt/releases/latest --jq .tag_name)"`. A CLI
release attaches its four tar.gz archives, final `dist-manifest.json`,
`tmt-installer.sh` and the byte-identical `install.sh` (the name the one-line
install uses); an Office release uses the independent `tmt-office-v<version>`
tag and attaches its four archives and final manifest without a CLI bootstrap, and
a Squad release does the same under `tmt-squad-v<version>`. Every release also carries
`release-publication.json`, the completeness marker uploaded last: it stays on the published
release (about 100 bytes, and `tmt upgrade` selects assets by exact name and ignores it).
Verify uploaded SHA-256 digests before publishing each draft. Verify
`immutable: true`, tag commit and GitHub release attestation (`gh release verify`
and `gh release verify-asset`); the pipeline does this for its own publications. Never combine product manifests, replace an
immutable release's assets or move its tag. A repair needs a new reviewed version.

Also verify **upgrading from the last published release**, not only fresh installs.
`Native release upgrade proof` (`.github/workflows/native-release-upgrade.yml`) does this
on the four matching hosts with real bytes, and runs by hand (`workflow_dispatch`) for any
draft or published tag, from `main` only. Its `fetch` job, which holds the write token that
can see draft assets and runs `main`'s code, downloads the release's archive and manifest and
those of the newest published release of the same product below it, each checked against the
digest GitHub recorded, and hands them over as a run artifact; the read-only `prove` jobs
re-check the digests and run the scripts of the release's own commit on them. A CLI release
goes through
`verify-native-installation.mjs`: the previous archive is installed pinned, the candidate
is refused while pinned and installed with `--unpin`, the exact skills are served, SQLite is
unchanged by the installation, the old executable is preserved, a repeat is a no-op and a
downgrade is refused. It also proves the migration of state the previous release wrote
(`migrated-state.mjs`): before the upgrade the previous release writes identities (one with a
preamble, role, metadata and status) and a room they joined with a message queued to each
member, all through commands that need no tmux; once the candidate has opened that state, the
proof reads the database file and requires exactly the migrations the candidate's source lists
(the publication gate's `countMigrations`), clean `integrity_check` and `foreign_key_check`,
every id the previous release wrote still held by some table, and no table that held rows
short of them. Only ids, counts and these SQLite checks are compared, never command output.
Binding a pane needs tmux, which the proof does not use, so `bindings` and `host_servers` stay
empty and a migration that rebuilds them over rows is not exercised.
An Office or Squad release is installed over the previous one by the newest published CLI
with `tmt extension install <extension>` and read back with `tmt extension ls`
(`verify-native-extension-upgrade.mjs`): the version changes, the previous release stays on
disk, a repeat is a no-op, a downgrade is refused and no CLI link is created. Extensions
have no install command of their own under `tmt <extension>`; the proof must use the surface
a user's install runs. The first release of a product has nothing to upgrade from and says
so. A commit that predates these scripts fails the proof with that message; prove it by
hand as below. A verifier command that fails says, on one line, which command failed, how it
ended and the first thing it said. A failed host keeps its log as an artifact, and the
`conclude` job outputs, as the workflow's `reason`, the first error of each failing host on one
bounded line (it reads the logs as data, because the release commit's own scripts wrote them);
the `upgrade` hold marker carries that reason with the run URL.

For CLI, each `prove` host also runs the real-archive adapter acceptance test after
the installer/migration proof, using the same digest-checked previous and candidate
archives. `release-upgrade.mjs acceptance` compiles only `tmt-adapters`' lib tests
from `rust/` to select its pinned toolchain, with no debug information or
incremental compilation, then
requires exactly one discovered and executed passing ignored test. The existing
read-only `native-rust` dependency cache is restored without another writer. The
job retains its ten-minute timeout; compile duration and the test's output appear
in the run summary and proof log. A compiler, discovery or test failure fails the
upgrade gate. CI uses Cargo's available workers; local developers export
`CARGO_BUILD_JOBS=2`, which the verifier preserves. Extension proofs do not compile
or run this CLI test.

The adapter proof injects canonical acquisition responses into the release's
upgrade adapter and executes real old/new binaries in isolated state. It verifies
receipt provenance, download cleanup, retained old bytes, exact candidate skill
bytes, conflict preservation, partial refresh failure and repair. If the two
embedded skill texts match, only differential content-transition coverage is
reported as skipped; the other assertions still run. An owner-authorized rerun
uses current main's repaired tooling to compile and execute the release's own
adapter from `release-source/rust`, selecting that source's toolchain pin. If the
release-source checkout predates the post-#575 release-gate test form (including
the #563 variant that requires differing text), this sub-proof reports
`predates; not applicable` without blocking the rerun or claiming a pass. The
installer/migration proof remains required; a missing release-source checkout is
an error.

Neither pre-publication proof runs public `tmt upgrade` downloads or a public
installer: the candidate has no published release to find, and production has no
test endpoint. The post-publication smoke below retains that separate live proof.
For manual installer coverage, use an isolated HOME and prefix to install the
previous published version with its public installer, then the candidate's
installer. Preserve the superseded receipt. Receipts written by older releases
stay readable: v5.0.0-alpha.2 through
alpha.6 and Office 0.1.0-alpha.1 through alpha.3 record the pre-rename repository
`wkh237/tmux-team`, which receipt reading accepts as the official one (#492).

The pipeline's `smoke` job (see Publication above) runs the actual public script with an
isolated HOME, state directory and prefix and checks the version, the managed skills, PATH
selection and `tmt upgrade --json` against live immutable metadata after every automatic
publication; run it by hand for a tag published another way. Promoting README installation
instructions stays the owner's decision, and a host installation is never mutated. Record the
smoke separately from controlled-curl fixture evidence. npm publication is not part of native
GitHub release publication.

The PR smoke matrix verifies raw native runtimes, not release archives.
#135 introduced archive generation; later slices delivered installation.
The target inventory and artifact metadata live in `dist-workspace.toml` and
the generated cargo-dist manifest, not another TMT release catalog.
Select the CLI release explicitly with `--tag v<CLI-version>` for direct
`dist plan`/`dist build` calls: Office is independently versioned. The maintained
build script resolves the selected package version from Cargo automatically.

Install the pinned developer tools into a chosen tool directory (not needed by
end users): cargo-dist 0.32.0 with `cargo install --locked`, and cargo-about
0.9.2 with `cargo install --locked --features cli`. Put their binaries on PATH.
Fetch the locked workspace dependencies before the offline notice step.
Build targets sequentially in one checkout, or use separate worktrees: the
generator's distribution directory and generated notice input are per-checkout.

```sh
# Choose a target from dist-workspace.toml that matches the verification host.
native_manifest=$(mktemp)
MACOSX_DEPLOYMENT_TARGET=11.0 scripts/build-native-artifact.sh aarch64-apple-darwin > "$native_manifest"
node typescript/scripts/verify-native-artifact.mjs \
  --manifest "$native_manifest" \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

For Office, Vite generates the bundled frontend license inventory as
`target/office-spa/THIRD-PARTY-NOTICES.txt`; the artifact builder appends it to
the target-filtered Rust notices. CLI notices remain Rust-only. A missing or
empty frontend notice file fails Office packaging.

Review the generated `rust/target/native-notices/THIRD-PARTY-NOTICES.txt` against
the locked, archive-target-filtered runtime graph, including Unicode copyrights;
the verifier compares the archived notices and license with these selected
inputs and rejects placeholder attribution. A successful generator
is not legal certification. Keep the complete notice text with redistributed
binaries. The verifier bounds inputs (64 MiB compressed, 128 MiB expanded),
requires exactly the four runtime files, and removes its private staging after
success or failure. It runs the extracted executable with no Node/Rust/tmux on
PATH and verifies native SQLite persistence through public commands. macOS
requires system `otool`, which it finds once through `xcrun` under a 10 s bound;
the first `xcrun` call on a fresh hosted runner can exceed that, so every workflow
job that runs the verifier on macOS first runs `.github/actions/warm-xcrun`
(bounded retry, logs the duration). A new macOS verifier job must do the same, and
a guard test fails when one does not. Linux requires `readelf` for static-musl
linkage checks.

`typescript/test/native/artifact.Dockerfile` provides a local matching-architecture Linux
musl build and verifier. Set `TARGET_TRIPLE` from the selected generator target,
give the image a task-owned name, then run it with `--rm --init --network none`
and `--archive artifacts/<manifest archive name> --target <target>`. Remove that
owned image after verification. Emulated execution and cross-compilation alone
do not satisfy native target acceptance. This optional image is not the tmux
E2E harness or a publication workflow.

Negative archive tests use real tar fixtures and causal guard assertions.
The native-artifact-policy hard-link fixture uses synchronous tar construction:
the asynchronous packer's pending-link queue can hang under out-of-order filesystem
callbacks. Keep this tiny fixture outside that queue, and verify the resulting
archive contains a real `Link` entry before asserting the unchanged policy rejection.
Exercise checksum corruption, truncation, missing executable/notices, links,
unexpected paths, duplicates, bounds and cleanup; never accept any arbitrary
process error as proof of the intended check. Inspect exact manifest and archive
bytes from the final source before running the reviewed CI candidate.

### Offline native installer verification

#### Native update verification

`tmt upgrade [--channel stable|alpha] [--to <version> | --unpin] [--json]`
and `tmt update` share one grammar and implementation. Use task-owned managed
prefixes, never a user's installed command or app data. The invoking executable
must be the active release; an unmanaged checkout binary fails before networking.
Production has no test endpoint or TLS bypass. API fixtures inject only the
adapter's acquisition boundary; actual local TLS fixtures use test-only trust.
Test old/new real release archives separately from synthetic tar fixtures. Use
different embedded skills for differential proof that the newly active executable
supplies refresh; identical text still permits the remaining upgrade assertions.

Channel discovery uses GitHub's product-prefixed `git/matching-refs/tags/<prefix>`
API and exact-tag release metadata, independent of the repository's total release
count. Matching refs currently return a complete array without pagination, including
more than 1,000 refs; the adapter also follows a supplied Link `next` relation, admitting
only consecutive pages of the same product endpoint. Ref discovery has at most ten
pages and a total 2 MiB response budget. Only complete discovery permits semantic-version
selection; ref order and release creation dates do not select a version. The ordinary
metadata path makes exactly two requests (refs and the highest candidate's release).
Confirmed missing releases and explicit drafts are the exceptions that require another
tag lookup, capped at 32 lookups per invocation. Distinct published versions with equal
precedence fail. Shared deadlines stay at ten seconds for metadata-only checks and
60 seconds for acquisition, with existing bounded HTTPS redirects. The shared
HTTPS client permits only one rate-limit wait/retry across its requests, inside
that same deadline. API 403/429 responses require primary evidence (zero
`x-ratelimit-remaining` plus `x-ratelimit-reset`) or secondary `retry-after`.
The wait honors applicable timing constraints plus positive jitter; an invalid
header, a wait beyond the remaining deadline or another rate limit fails clearly
with the reset/earliest-retry time when available and an optional `GITHUB_TOKEN`
hint. This adds at most one retry request without expanding discovery bounds.
A provided token is sent only to `api.github.com`, rebuilt per redirect hop;
missing/empty tokens keep acquisition unauthenticated. Post-publication smoke
remains token-free. The fixed-version shell bootstrap makes no API discovery
requests and does not send tokens to its asset downloads.

Deterministic local HTTPS rate-limit fixtures run with
`cargo test --locked -p tmt-adapters release_http`: primary reset and secondary
Retry-After followed by success, deadline refusal, malformed timing, ordinary
permission 403, repeated limits, a retry allowance shared across calls, and
API-only token scope through an asset-host redirect. Test-only wait injection
avoids real rate-limit sleeps; real TLS, headers, response bodies and request
counts remain observable. Existing deadline, redirect and size controls still
run. Production has no test endpoint or TLS bypass.

Incomplete ref discovery or an exhausted candidate scan fails with the unchanged
`Release discovery exceeds its bound; select an exact version with --to.` error
(`InvalidData`, surfaced as `NATIVE_UPGRADE_FAILED` by CLI upgrade); an extension check
reports `unknown` on discovery failure. Oversized HTTP responses retain the transport
size-limit error. Neither case selects a release from partial evidence.

Verify discovery with injected responses, never live API tests: more than 1,000
product refs without Link; complete pagination; a winner before and after 300
interleaved product/channel releases; numeric alpha ordering; stable/alpha/beta/rc
separation; tagged drafts and tags without releases; malformed refs/links,
duplicate refs, cross-endpoint pagination, page/byte/request exhaustion and HTTP
failure without older-version fallback. Assert the two-request common case and
no release/asset request before complete ref discovery. Retain exact-tag and
pinned no-network controls and the installer's prior-file/no-staging assertions.
The generated shell bootstrap fixes a manifest version and uses versioned download
URLs; it does not perform channel discovery. CLI alpha publication flags make
`releases/latest` unsuitable for stable-channel selection.

Check pinned no-network behavior, explicit pin/unpin, unchanged release identity,
preserved old bytes, missing/mutable release rejection, dual digest checks,
same-version integrity and concurrent pin fencing. Cancellation and finalization
tests must inspect active receipts and surviving executables, not only exit codes.
After activation, skill failures retain a partial report and nonzero status;
malformed/nonzero/oversized/timed-out child output is never a success. The existing
process runner retains bounded output only for completed nonzero exits and never
prints it implicitly. Verify both byte preservation and task-owned cleanup.

The synchronous HTTPS dependency is pinned ureq 3.4.0 (MIT/Apache-2.0, upstream
MSRV 1.85), selected without an async runtime or curl fallback. The lockfile and
workspace MSRV 1.95 remain authoritative for the complete graph. rustls and
platform-verifier use native trust and library proxy environment behavior.
Runtime attribution includes ISC crypto and CDLA-Permissive-2.0 certificate data;
generate target-filtered notices through cargo-about and retain complete texts.
rcgen/rustls local-server fixtures are dev-only, not production endpoint options.

The explicit actual-archive acceptance test requires separately versioned,
matching-host cargo-dist artifacts. It is ignored by ordinary tests and explicitly
selected by the CLI release upgrade proof. A manual invocation is:

```sh
TMT_UPGRADE_OLD_ARCHIVE=/absolute/old/archive.tar.gz \
TMT_UPGRADE_OLD_MANIFEST=/absolute/old/manifest.json \
TMT_UPGRADE_NEW_ARCHIVE=/absolute/new/archive.tar.gz \
TMT_UPGRADE_NEW_MANIFEST=/absolute/new/manifest.json \
TMT_UPGRADE_TARGET=aarch64-apple-darwin \
CARGO_BUILD_JOBS=2 cargo +1.97.0 test --locked --manifest-path rust/Cargo.toml -p tmt-adapters --lib \
  native_install::upgrade::artifact_tests::cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts \
  -- --exact --ignored --nocapture
```

Require one selected passing test, not an empty filtered run. This test injects
canonical acquisition responses but executes real old/new binaries, managed
skill installation, partial hidden refresh and repair in scrubbed task-owned
state. It does not claim to contact a public release or exercise the public CLI
over a fake production endpoint. Run the independent artifact verifier too.
Identical embedded text prints a skipped differential content-transition result;
candidate-byte equality, conflict preservation and repair are still required.

#### Offline composition

The internal entrypoint consumes a local cargo-dist archive and manifest. It is
not advertised by help/completion and does not change `tmt install` skill syntax.
Use a task-owned prefix and matching host archive, never an existing user install:

```sh
native_prefix=$(mktemp -d)
rust/target/debug/tmt __native-install \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz \
  --manifest "$native_manifest" --prefix "$native_prefix" --channel alpha --json
"$native_prefix/bin/tmt" --version
```

`--pin` pins the selected candidate, `--unpin` clears an existing pin, and omission
preserves its state. Neither authorizes a downgrade. Repeated exact artifacts are
no-ops after ownership validation; metadata-only changes activate a new receipt
with the same payload. Receipts record local-archive provenance, not authenticated
public release provenance. Keep application state isolated separately when running
identity/profile commands; installing the executable must not open a database.

The internal `--product office` selector uses the same offline verifier/publisher
for a `tmt-office` package and executable. Omission selects the CLI unchanged.
Office installs under `lib/tmt-office` with only `bin/tmt-office`; it must not
modify CLI links, receipts, application state or managed skills. Verify coexistence,
cross-product rejection and interruption in isolated prefixes. Copied CLI binaries
in synthetic Office test archives prove installation behavior only, not an actual
Office companion, protocol compatibility or public distribution. The native
process suite now separately copies the compiled `tmt-office` into synthetic
archives and exercises its exact versioned probe. Build the workspace first;
a missing companion is an error, never a fallback to the CLI. This additional
evidence does not replace real cargo-dist archive and distribution acceptance.

#### Office archives

The local Linux artifact Dockerfile accepts `--build-arg PRODUCT=office` with
the matching `TARGET_TRIPLE`. Pass `--product office` and the corresponding
Office archive to its verifier entrypoint. The default remains CLI; both use
the same generator, notice owner and independent verification path.

Generate an actual matching-host Office archive with the same toolchain and
target policy, selecting Office's runtime dependency notices:

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin office > /absolute/office-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product office \
  --manifest /absolute/office-manifest.json \
  --archive target/distrib/tmt-office-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

Build products sequentially and retain their manifests, archives and generated
notices separately before another build overwrites distribution output. The
same bounded independent verifier checks inventory, hashes, notices and linkage;
Office runtime proof requires its exact probe without creating application state,
not CLI-only skill/SQLite commands. Follow with `office install --yes --archive
<archive> --manifest <manifest> --prefix <task-owned-prefix>`, status, repeat
installation and explicit uninstall. Inspect surviving bytes after rejected
candidates and deactivation. Public availability is a separate authorized gate;
local cargo-dist's package selection tag does not publish a Git tag. Historical Office
releases use `tmt-office-v<version>`. Office is frozen and is
not accepted by the shared release workflow; local archive verification does not
authorize publication or a CLI tag.

#### Squad archives

A Squad archive adds one directory to the runtime files: `skills/`, copied from
`extensions/tmt-squad/skills/` by the package's cargo-dist `include` (a package
list replaces the workspace list, so it repeats the shared files). cargo-dist
declares that directory as the single manifest asset `skills`; the installer
inventories its files from the checksum-verified archive. Build and verify it
like Office, passing the skill sources for a byte-for-byte comparison:

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin squad > /absolute/squad-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product squad \
  --manifest /absolute/squad-manifest.json \
  --archive target/distrib/tmt-squad-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skills extensions/tmt-squad/skills \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

The runtime proof checks `tmt-squad --version` and that `tmt-squad skill show`
prints the archived `SKILL.md`, with an empty HOME, config and working directory
afterwards. The local Linux Dockerfile takes `--build-arg PRODUCT=squad`; pass
`--product squad --skills expected-squad-skills` to its entrypoint. Follow with
`tmt extension install squad --archive <archive> --manifest <manifest> --prefix
<task-owned-prefix> --channel alpha --yes`, a repeat install and `tmt extension
uninstall squad`. Squad's closure adds the Zlib license (`foldhash`), accepted in
`rust/about.toml`; the CLI and Office notices do not change.

Native adapter tests cover bounded archive acquisition and publication failures;
native process contracts use the existing executable selector and sandbox. Test
current/receipt tampering, manager collisions, pin changes, interrupted staging,
lock contention, missing command-link repair and retained previous releases.
Assert surviving bytes and cleanup, not merely a failed exit. Tests comparing
large executable buffers must use exact `Buffer.equals`
checks rather than structural object matchers: enumerating every byte can exhaust
the test runner's heap on debug binaries. Verify the comparator detects a changed
byte; do not replace byte equality with a size-only assertion or raise CI memory
limits to hide assertion overhead. Installation cases also use an explicit
15-second subprocess budget for debug
archive hashing/decompression and durable publication, distinct from the ordinary
CLI's five-second test budget and each scenario's 60-second cap. Keep ordinary
command and production timeout policies unchanged; validate installation budgets
under constrained local resources rather than treating CI as a timing probe.
Synthetic filesystem
fixtures are not proof of runnable release artifacts: retain separate actual
cargo-dist archive execution and target/linkage/notice evidence. Run the shared
skill-installation regressions when changing shared file locks or content digests.

For actual old-to-new acceptance, build two separately versioned cargo-dist
archives in task-owned source copies. Do not alter the repository release version
or publish fixtures merely to test updates. The newer archive must contain the
offline installer; the older artifact must run its real reported version. Verify
each archive's notices/linkage with the artifact verifier above, then run:

```sh
node typescript/scripts/verify-native-installation.mjs \
  --previous-archive "$previous_archive" --previous-manifest "$previous_manifest" \
  --archive "$next_archive" --manifest "$next_manifest" \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

This reuses the bounded packed-command runner and independent archive verifier.
It checks exact old/new versions with no runtime PATH, pinned rejection, explicit
unpin advancement, a retained executable, no-op and downgrade rejection, exact
embedded skill, unchanged SQLite bytes during installation and the migration of state the
previous release wrote. Its temporary
prefix/application state is always invocation-owned and removed afterward.

### Native curl bootstrap verification

Generate the release-specific script only after final cargo-dist archives and
their independent runtime verification. All selected archives must be present;
the manifest, not a hand-maintained version table, owns the generated facts:

```sh
node typescript/scripts/generate-native-bootstrap.mjs \
  --manifest /absolute/dist-manifest.json --archive-dir /absolute/artifacts \
  > /absolute/artifacts/tmt-installer.sh
sh -n /absolute/artifacts/tmt-installer.sh
node typescript/scripts/verify-native-bootstrap.mjs \
  --manifest /absolute/dist-manifest.json \
  --archive /absolute/artifacts/tmt-cli-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

The last command requires actual matching-host release artifacts, uses isolated
HOME/state/PATH, real shell utilities and the new native executable. Only curl
acquisition is replaced with task-owned fixture copies; production has no test
endpoint. It checks repeat/no-op, explicit pin followed by no-network upgrade,
exact installed skill-bundle bytes, old npm command preservation, PATH warning and
temporary cleanup. It is not live GitHub download or cross-target evidence.

The existing `typescript/test/native/artifact.Dockerfile` also carries this verifier. After
building its task-owned matching-architecture image, run the normal artifact
entrypoint, then repeat with `--entrypoint node` and
`typescript/scripts/verify-native-bootstrap.mjs --manifest native-manifest.json --archive
artifacts/<archive-name> --target <target> --skill expected-skill.md`. Keep
`--rm --init --network none` and remove only the task-owned image afterwards.

`typescript/test/tooling/native-bootstrap.test.ts` covers the generated shell's negative paths with
the existing bounded CLI sandbox and a synthetic executable for orchestration.
Do not count that stub as native publication evidence; pair it with the actual
artifact verifier. Run `(cd typescript && corepack pnpm check)`, unit tests and
two Docker lifecycle passes before the reviewed CI candidate. Keep generated
outputs out of source control.
No new runtime dependency, data migration or public publication is authorized
by generation. An authorized release must upload the exact verified manifest,
archives and generated script, establish immutable release/provenance evidence,
and verify the public download before the README advertises it as available.

## Installed guidance source ownership

`skills/tmux-team/SKILL.md`, `skills/tmt-inbox/SKILL.md`, and the optional
`extensions/tmt-office/skills/tmt-office/SKILL.md`, `extensions/tmt-office/skills/tmt-prop-create/SKILL.md`, and
`extensions/tmt-office/skills/tmt-avatar-create/SKILL.md` are the five
canonical guidance sources in one versioned bundle. Core install exposes only
the first two; explicit Office install or upgrade manages all three Office siblings
in detected and already-managed custom roots. Verify exact embedded bytes,
core-only preservation, sibling
managed links, repeat no-op, backup/conflict and partial-failure behavior, lock
ownership, refresh without resurrection, and no effects on SQLite or tmux.
`test/native/legacy-extension-skills.test.ts` owns the old five-skill bundle
regressions: twelve provider links, dangling generations, owner adoption,
half-removed listing/removal, retired refresh intent, preservation of user
content, executable conflict recovery commands and native upgrade causes. Every
fixture uses the existing isolated HOME/config/process sandbox. Existing-source
integrity and canonical-store/name rejection controls stay with the Rust skill
owner tests.
Follow the [Settings handbook chapter](site/src/chapters/settings.mdx) and
`skills/README.md` for provider/custom-root usage; do
not add provider-specific skill copies. The squad lead skill
(`extensions/tmt-squad/skills/tmt-squad/SKILL.md`) is deliberately outside this
bundle: the squad executable embeds it, and its native test checks the
documented status row shape against real output. The squad playbooks
(`extensions/tmt-squad/playbooks/<name>/SKILL.md`) are embedded the same way but
kept out of the release skills tree; `playbook.rs` tests pin the catalog to the
source files and that separation, and `squad.test.ts` covers install and removal
against isolated provider roots. Every command a playbook tells an agent to run
is executed once in a disposable tmux server and git repository before it is
written down. Extension-owned skills
(`skills.install`, owned by an extension rather than this bundle) are covered by
`skill_installation::owned_tests` with isolated provider roots: publish, repeat
no-op, core and cross-owner claims, force backup, Office adoption, removal by
owner (all skills or a named subset) and drift. Runtime/linkage proof shared by archive
and raw verification lives in `typescript/scripts/native-runtime-proof.mjs`.

## Release PR safety gates

`Code quality` runs `node typescript/scripts/release-pr-safety.mjs notes` for PRs
(opened, synchronized or reopened) and merge groups. Body/title edits do not
restart full CI; the merge-group step re-reads the current PR body over REST
and gates it at queue time. A release branch under
`release-please--branches--main--` must resolve to a released component. Its notes
must compare from that component's newest published tag; each linked commit SHA
must be a descendant of that tag and an ancestor of the candidate base, excluding
the tag itself. COVERAGE also requires every commit that release-please would list
for this component in `(published tag, candidate base]` to have a commit link.
The gate projects links with the isolated pinned release-please parser, path
splitter, exclusions and default notes renderer, including the shared private-leaf
attribution. Package paths, `exclude-paths` and visible `changelog-sections` come
from `release-please-config.json` at the candidate base; omitted sections use the
pinned renderer's defaults. Breaking changes, nested messages and revert suppression
retain that renderer's behavior. There is no separate type list or entry-count
policy. Local coverage history is capped at 500 commits with 30-second command
bounds; missing config, unsupported changelog renderers or incomplete evidence fail
closed. Missing tags and malformed notes also fail.
Existing locked Cargo workers verify the cumulative queue result through the
[CI selector](ARCHITECTURE.md#ci-selection-and-worker-model), so a later prose-only HEADGREEN tip retains earlier
release version/lock changes.

The notes job fetches full history and tags. For a PR it uses the event's base SHA;
for a merge group it reads all pending squash commits from the common ancestor of
fetched `origin/main` and the queue head. GitHub appends `(#PR)` to each squash
subject; REST reads resolve those PRs and identify release branches first. Only
release candidates must have matching title, repository and main base metadata;
ordinary PR title/base mismatches are skipped by this gate. A candidate's parent is
its notes range endpoint. Unavailable, stale or oversized queue evidence fails
rather than dropping a candidate. Rerun after publication metadata settles; an
outdated compare anchor, out-of-range note or missing COVERAGE link requires
release-please regeneration on the next main push. A late merged commit therefore
holds the merge group until refreshed notes cover it, preventing silent omissions.

After `github-release` reconciles merged release PRs and before `release-pr`,
`node typescript/scripts/release-pr-safety.mjs draft`
checks all manifest versions with fresh App-token REST reads, including drafts
created in this invocation. A matching draft with no exact git tag holds only
its manifest path. The step writes JSON `held_paths`, sanitized matching `drafts` evidence and a
summary naming held paths;
`skip=true` only when every released manifest path is held. The workflow passes
`TAGLESS_DRAFT_PATHS` to the pinned wrapper. Its ManifestPlugin candidate hook
filters held paths before separate PR updates, allowing unheld components to
regenerate; malformed/unknown hold paths fail closed. Both held means a full
`release-pr` skip; either held alone does not stall the other component.
`github-release` and draft build/publication dispatch continue. The guard needs a
token that can see draft releases: live mode uses the Release App token. Dry runs
fall back to `github.token` with `contents: read`, which sees no drafts, so the
tagless-draft guard does nothing in those runs. Once its tag exists, the next main
push resumes that component’s release PR creation. This pre-check is an observation, not an atomic
fence with later publication.

Both gates use workflow tokens and REST only, with 30-second command bounds,
at most ten 100-item pages per list and 60 requests per invocation; queue discovery
is capped at 40 commits. Errors and caps fail visibly. Test with fixtures only:

```bash
cd typescript
pnpm exec vp test run --config vitest.config.ts test/tooling/release-pr-safety.test.ts
pnpm test:run
pnpm check
cd ..
actionlint .github/workflows/ci.yml .github/workflows/release.yml
```

## Release stall monitoring

`release.yml` runs `node typescript/scripts/release-stall.mjs` in its own advisory
job after release-please. The release job retains read-only workflow-token
permissions; only the monitor job has `contents: read` plus `issues: write`.
It consumes mode, held-path, queue-skip and sanitized matching-draft outputs;
no credentials cross job outputs.
A tagless component hold or queue skip warns when its matching manifest draft is
strictly older than 30 minutes. An open component release PR warns when its head
commit timestamp is more than one hour older than that component’s newest
releasable commit, and a REST ancestry comparison proves the commit is absent
from the head. The timestamp difference is between commits, not time since the
workflow ran. Candidates come from real pinned release-please planning, including
private-leaf attribution, excludes and component cutoffs; unrelated component or
nonreleasable pushes do not make a preserved head stale.

The full checkout supplies bounded local history/files/tags for this read-only
plan. Discovery uses REST only, at most 60 requests and ten 100-row pages per
list, with a 90-second total deadline and ten-second individual command bound.
Planning consumes at most 500 commits; incomplete history, missing candidate
links or unavailable metadata warn instead of declaring healthy. Proven draft
stalls still open/update the issue when independent PR planning is unavailable. No GraphQL or
live API tests are used. The separate job’s five-minute bound and
`continue-on-error`, plus the monitor step’s two-minute bound, isolate setup,
startup, summary and API failures from release gating and dispatch.

The monitor uses `github.token` for all its REST reads and issue writes. The
push-capable App token stays in the release job, where the existing draft guard
needs it to see unpublished releases. That one discovery now also emits matching
draft path/ID/tag/creation-time metadata, excluding bodies, assets and credentials.
Read-only monitor credentials cannot list drafts: the transported snapshot supplies
their evidence. A current published-release read supersedes any draft published
since the guard ran; absent or inconsistent transport warns without closing the
issue. No second App token is minted or passed to monitoring.
A complete fixed-title search
finds the single `Release stalled` issue, reopening it for later stalls. A recent
REST issue page also checks for a just-created issue not yet indexed by search. Each new
draft ID or stale PR head/commit pair receives a comment marker; retries do not
repeat the occurrence. Complete healthy discovery closes it automatically.
Ambiguous discovery or mutation errors leave a visible summary warning; resolve
multiple matching issues before retrying. Dry runs only summarize and do not
edit issues. If the App token is unavailable, dry reads cannot observe drafts,
as described in the safety gate above. For current published manifest tags, the monitor also
reads open reporter issues and distinguishes public-smoke rate-limit infrastructure
from real post-publication check failures. It recommends retrying smoke after reset for the former, never publication.
Old release issues and pull requests are ignored; unavailable issue discovery cannot
declare healthy. The detector never edits release PRs, tags, drafts or publication state. Fixtures verify both thresholds, component
isolation, real pinned planning, occurrence lifecycle and nonblocking failure.

## Conventional PR-title rollout

`Code quality` runs `node typescript/scripts/release-pr-safety.mjs titles-report`
only for `merge_group`. It reuses the notes gate's bounded cumulative queue reader
and checks every pending squash subject after removing GitHub's final `(#PR)`
suffix. It checks `type(scope)?: subject`: a lowercase type, optional nonempty
scope, optional `!` breaking-change marker, colon/space and a nonempty subject.
This is syntax feedback; release-please remains the owner of release attribution
and changelog parsing. A title edited after queue entry is not compared against
REST: the squash subject is the release input being checked.

The current mode is **report-only**. Findings include PR number, commit SHA and
escaped title in job output and `GITHUB_STEP_SUMMARY`. Invalid titles, unavailable
queue evidence and summary I/O errors leave the command's exit code at zero, so
this step cannot fail a group before enforcement. Missing evidence is visible as
an unavailable report; it is not a clean title result. Existing notes and other
required gates retain their failure behavior. Full CI has no `edited` trigger
and there is no separate feedback workflow.

The observation day starts when the report-only PR for #1125 merges. Enforcement
must be a separate small PR, with the cutover set to that actual `merged_at` plus
24 hours and written as an explicit UTC timestamp in the PR and this section.
The concrete timestamp is pending the report-only merge; record it in the
separate enforcement PR immediately afterward, and do not merge the flip before
that time. Neither a clock-dependent automatic flip nor protection/ruleset edits
are part of this rollout.

Fixture and full verification commands:

```bash
cd typescript
pnpm exec vp test run --config vitest.config.ts test/tooling/pr-title-check.test.ts test/tooling/release-pr-safety.test.ts
pnpm test:run
pnpm check
cd ..
actionlint .github/workflows/ci.yml
```

## Queued release pull requests

`Release` runs `github-release` first, then the fresh tagless-draft check, then
`typescript/scripts/release-please-queue.mjs` before `release-pr`. The pinned
release-please refuses PR updates while a merged release PR is still untagged;
reconciling it first lets the same run refresh remaining PRs against updated main.
A `github-release` failure stops the job and explicitly reports that `release-pr`
was not attempted. Both commands still only plan in a dry run.
Explicit-token GraphQL discovery follows up to 20 cursor pages of 100 open PRs, ordered
by creation time, and considers only same-repository heads starting with
`release-please--branches--main--` targeting `main`. Complete discovery is required
before deciding. A queued PR skips `release-pr` only when the shared
`checkReleaseNotes` gate accepts its current notes against fetched `origin/main`, even when the push-event
checkout lags that main HEAD.
The release job uses the same `fetch-depth: 0` checkout as Code quality, including tags.
A queued candidate held by a tagless draft keeps its queue entry; release-please's
existing manifest-path filter preserves that candidate while unheld components regenerate.
Invalid compare anchors, out-of-range links or missing COVERAGE links require a
live dequeue before `release-pr`: GitHub locks a queued PR's head branch against
updates. The pre-check re-reads the selected PR's node ID, number, open state,
repository, head SHA/ref, base, draft state and queue entry; they must match the
observed same-repository main release PR. It then makes one `dequeuePullRequest`
mutation with the same release App token used for auto-merge enabling and verifies
the returned PR and queue entry IDs. Each request has the existing 30-second bound;
there are no retries or polling. Multiple queued release PRs with stale notes
require reconciliation before any mutation. Successful dequeue permits refresh in
this invocation; the existing enable step can re-enqueue the refreshed candidate.
A failed recheck or failed/unverified dequeue emits `blocked`, writes a clear
recovery notice in the step summary, and skips both `release-pr` and auto-merge
enabling. `github-release` has already run, allowing downstream draft processing to
continue; its own failures still fail the job. Verify the PR and queue state before
retrying the Release run through the normal authorized workflow. Dry runs only
report the planned dequeue and run release-please in dry-run mode. Covered queued
notes retain their head and a summary notice;
`github-release` precedes either decision. REST notes must match the discovered PR head.
Acquisition failures, malformed PR metadata and incomplete history fail the run
visibly rather than silently skipping. Malformed responses, repeated
cursors, duplicate PRs, API errors and exhausted discovery fail visibly.

The same script's live-only `enable` command finishes discovery before choosing one
non-draft release PR by ascending PR number. An existing enabled or queued release
retains the slot; multiple active releases fail with instructions to reconcile them.
Enabling uses `--auto --match-head-commit`, with no strategy flag: under a merge queue `gh`
warns on stderr when one is passed, which the packed-command contract treats as a
failure. It propagates failures without
trying a second PR. It never updates a BEHIND branch or jumps the queue. The next main
push after the active PR merges permits the remaining component to be regenerated and
enabled against the updated manifest.

A single active release PR with permanently failing checks holds the slot and blocks
the other component. `tmt-infra-lead` coordinates diagnosis and a fix with the owning
squad; if the release PR is to be abandoned, the lead asks tmt-lead or Ben to close it.
The next main push can then select the remaining component. Automation does not bypass
failed checks or abandon an active release on its own.

Discovery, dequeue and enabling use the release App token in live runs (dry-run
discovery uses the workflow token), never an agent's token. The existing workflow concurrency group
serializes automatic enabling; external/manual enabling and enqueues are not atomic
with discovery. A PR queued between the pre-check and a branch update can still fail
once and recover on the next main push. The recheck and dequeue are not atomic with
external enqueues or head updates; a changed or unverified entry blocks refresh
rather than authorizing another mutation. Required checks and publication
authorization are unchanged.

Fixture-only verification (no dispatch or Docker):

```bash
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/release-queue.test.ts test/tooling/release-pr-safety.test.ts test/tooling/release-please-config.test.ts test/tooling/release-workflow.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/release.yml
```

Fixtures cover stale queued notes → recheck/dequeue/refresh, failed dequeue → skip
with summary and prior `github-release`, covered queued notes → skip, and
non-queued PRs → refresh. Identity/head/queue races, dry runs and failure responses
must prove no retry or unintended mutation; unchanged generated heads remain preserved.

## Project tracking

Progress is read from one place: the `pj-tmt` project
(<https://github.com/orgs/pj-tmt/projects/1>), filtered to `label:epic`.
Each epic has one tracker issue titled `Epic: <name>` with the `epic`
label. The project's Sub-issues progress counts only direct
sub-issues, so the tracker is the only parent that matters for progress.

Every issue carries these Project fields:

- `Epic`: the tracker it serves. The issue is also a direct sub-issue of
  that tracker. Do not hang slices under an umbrella issue that is itself a
  tracker child; umbrella or findings-log issues stay outside the tracker.
- `Squad`: the squad whose lead owns the issue.
- `Status` records delivery state; the release sweep corrects stale closed-issue states:
  - `Todo`: not started.
  - `In Progress`: implementation started, including draft or stacked PRs.
  - `In Review`: a PR is ready for review or queued. In a stacked chain, the
    issue stays here while any of its PRs is still queued.
  - `Merged`: the last required PR is on `main` and a release is pending. A
    `Fixes #N` merge moves the issue here through the Project workflow.
  - `Released`: every affected product has a published tag containing the closing
    merge commit(s). Release automation sets it and fills `Released in`.
  - `Done`: closed without a delivering merged PR (not planned, duplicate, or resolved elsewhere); release automation sets it.
- `Agents`: comma-separated names of agents actively building or coordinating
  it now, including assigned members waiting on a named dependency. List the
  lead first. Reviewers who build nothing are not listed. Removing a member
  from `Agents` is part of its retirement checklist.

Tracker rules:

- Each tracker has one owning lead, recorded in `Squad`. On a tracker shared by
  squads, the owner writes the tracker's Status and body, and each child keeps
  the Status and Agents of the lead whose member works on it.
- The tracker body keeps a short `Now / Next / Blocked` section of three to six
  plain lines. Describe the outcome first, with issue numbers in parentheses.
  When something is runnable, add one `Try it` line with the command. Update
  the section when a PR merges, a member starts or retires, or something
  blocks. Keep logs and evidence in the child issues and PRs.
- Tracker Status is `In Progress` while any child is active, `Todo` when
  nothing has started or the feature is parked (say "parked" in `Now`),
  `Merged` when all required delivery is on `main`, and `Released` only after
  publication and any feature acceptance or dogfood gate. Keep pending gates
  visible under `Blocked`. Optional future children must not reopen a
  delivered milestone; state the delivered scope in `Now` and label deferred
  scope.
- A tracker (`Epic: <name>`) is a product item the maintainer set. A
  squad lead may propose one through tmt-lead, but it is opened only after
  the maintainer approves it; no agent creates a tracker on its own.
- Below a tracker, leads and the project manager may open child issues
  freely. Each child is one outcome with its own acceptance criteria and,
  normally, one reviewable PR. Split a child that hides progress across
  several PRs or squads.
- The project manager runs one batched pass per hour
  ([PM procedure](.agents/skills/tmt-pm/SKILL.md)); leads add event-driven
  updates. Batch Project edits, never poll, and treat about 200 GraphQL calls
  per lead per day as a ceiling. The GraphQL limit is shared by every agent
  on the maintainer's account.

## Review and evidence

Before handoff, report the changed owner and run the smallest relevant focused
tests, then the complete gates required by the issue:

1. `pnpm check` and retained tooling tests;
2. Rust fmt, clippy, locked tests, build and MSRV build;
3. the complete native process suite, plus the tooling
   selector's missing-native negative and selected-native positive controls;
4. two complete Docker E2E runs for lifecycle/transport changes;
5. available real-host smoke environments, with remaining architecture coverage
   left to the required CI matrix;
6. matching-host artifact/bootstrap proofs for release changes.

Do not turn a filtered, skipped, cross-compiled, or failed subprocess into a
success claim. Preserve exact bytes and independent oracles. Keep returned
errors distinct from crash recovery, and retain uncertainty, transaction,
retention, acknowledgment, lifetime and cleanup evidence when changing those
boundaries.

Test design and causal positive/negative controls belong to
[CONVENTIONS](CONVENTIONS.md#tests-and-review); isolation and evidence ownership
belong to [ARCHITECTURE](ARCHITECTURE.md#testing-and-evidence-boundaries).
Compare exact file bytes, not symlink-directory snapshots or enumerated binary
objects. Use structured output or a focused formatter test, not mocked
`console.log`. Apply the [architecture maintenance contract](ARCHITECTURE.md#maintenance-contract)
when changing an owner, boundary or verification procedure.

## Browser add-on shell

The private MV3 demo shell lives in
`extensions/tmt-remote/typescript/browser-addon`. It uses the existing pnpm
workspace and lockfile. It does not connect to TMT or implement pairing/crypto.
From `typescript/`:

```sh
corepack pnpm --filter @tmt/browser-addon --fail-if-no-match check
corepack pnpm --filter @tmt/browser-addon --fail-if-no-match test
corepack pnpm --filter @tmt/browser-addon --fail-if-no-match build
corepack pnpm --filter @tmt/browser-addon exec playwright install chromium
corepack pnpm --filter @tmt/browser-addon --fail-if-no-match test:browser
```

Browser tests use Playwright's Chromium, a disposable profile and a task-owned
loopback selection page. They never load the host Chrome profile or team data.
Load this package's `dist/` as an unpacked add-on in a separate development
profile to inspect it manually; Chrome 137 or later is required. Both right-click
Send to agent and the popup capture only after a gesture. All displayed agents
and replies are demo fixtures; Send does not deliver to an agent. Package code
uses the `fmt` block in its Vite configuration; shared docs use the tooling formatter.

## Remote pilot development

The local-build-only remote crate is a foreground deny-all door. It performs
two public startup reads (capabilities and `storage.root`), creates or reopens
its private `<dataRoot>/remote/` state (0700 directory; 0600 machine key,
SQLite database and locks), then refuses every remote application request. The
`/r/` route prefix and machine ID persist across restarts, and a second serve
on the same data root fails with `REMOTE_ALREADY_SERVING`. With serve running,
`tmt remote pair` prints a pairing link and code, shows the device's kind,
origin, name and four words, and asks once on the terminal; `tmt remote pair
--json` streams one event per line and reads `confirm` or `refuse` from stdin,
which is how the process tests drive it. `tmt remote devices [--json]` lists
paired devices with their four words, and `tmt remote devices revoke
<client-id>` ends one device's access, whether or not serve is running.
`tmt remote devices rename <client-id> <name>` changes its display name without
changing authority; a changed name ends the old session for silent reopening.
Mounted extensions receive current names and revoked tombstones through the
owner-only socket's reserved device-event callback. Native mount tests cover
delivery, retry/replay and both browser-spoofing guards; the
[channel contract](contracts/remote-channel-v1.md#extension-channel-api) owns
callback fields and consumer revision handling. Opening
the pairing link in a browser serves the pairing page, which shows the same four
words; after the owner confirms, the browser opens a door session with a signed
`session.open`, and its cookie then carries the device context to mounted pages. Pairing and state tests use short
roots under `/tmp`, because Unix socket paths are limited to about 100 bytes. It mounts colab under `/r/<prefix>/x/colab/` (`serve --json` prints the route prefix `/r/<prefix>`) while
`<dataRoot>/colab/door.sock` exists as an owner-only socket in a 0700
directory; mounted requests carry a device context only under a live door
session. Pages learn their principal from their own extension backend, using
the per-request `tmt-device-context`; the browser SDK has no principal accessor.
`certifyKey` signs a new mount-scoped `tmt-ext-cert-v1` certificate on each call
with the current `issuedAtMs`; the extension verifier enforces freshness. Existing
paired records with an extra `certificates` field load without migration.
Colab gets at most 16 live WebSocket tunnels, each closed after 120 seconds
without traffic; a full pool answers 503 with `retry-after`, so colab should
keep one socket per tab and reconnect after idle close.
Signed operations, hold, sends, the relay and journal integration are not
implemented. The [channel contract](contracts/remote-channel-v1.md) is
proposed; [the separately owned browser shell](#browser-add-on-shell)
uses only a stub. No official remote installer/release exists.

```bash
(cd rust && cargo build --offline --locked -p tmt-remote)
(cd rust && cargo test --offline --locked -p tmt-remote)
(cd rust && cargo clippy --offline --locked -p tmt-remote --all-targets -- -D warnings)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
node typescript/scripts/release-please-config.mjs --check
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/release-please-config.test.ts test/tooling/ci-scope.test.ts)
```

The door embeds `extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js`,
built from `remote-client`. After changing `remote-client/src`, rebuild and
commit it (Code quality rebuilds it and fails on a difference), then run the
Chromium pairing smoke against a debug build of the door:

```bash
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match build)
(cd rust && cargo build --offline --locked -p tmt-remote)
(cd typescript && pnpm --filter @tmt/remote-client exec playwright install chromium)
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match test:browser)
```

Pure byte/crypto conformance runs with the remote Rust tests above, including
shared independent canonical, pairing-code and fingerprint vectors, strict Ed25519
refusals, full HMAC tags and `serverProof` domain separation. The shared vectors
come from `extensions/tmt-remote/typescript/remote-client/test/reference.py`,
which also checks the pinned BIP-39 list digest. Regenerate/check only Rust-owned crypto fixtures with:

```bash
python3 extensions/tmt-remote/rust/tmt-remote/tests/fixtures/mac-reference.py --check
node extensions/tmt-remote/rust/tmt-remote/tests/fixtures/webcrypto.mjs
```

Use the repository Node 22 version and repeat the WebCrypto command locally on
Node 24. No extra required-CI Node setup is needed. The script verifies deterministic
signatures that Rust independently reproduces and verifies; `--write` regenerates
the public-test-key fixture. The Python oracle does not import product code.
These checks do not prove real Chrome key persistence/non-extractability across
MV3 worker restarts. Pairing, authority and browser integration remain separate.

After building core, put `rust/target/debug` on PATH and run `tmt remote serve`
(or `--json` for its bound descriptor). Direct invocation requires an absolute
`TMT_EXECUTABLE`; it never searches for another core. The door has no default
deadline; Ctrl-C/SIGTERM closes the listener, retained sockets and workers.
There is no autostart/LAN/daemon option. Door bounds are named in
`src/limits.rs`: 32 concurrent connections, 20 unauthenticated `/r/` attempts
per minute, 8 KiB/32 header fields, a 16 KiB pairing body, a door-wide 32 MiB
in-flight body budget (excess concurrent bodies get 429) and five-second
acquisition. The printed `127.0.0.1:<port>` is the only accepted Host.
Tests use disposable HOME/XDG,
count startup separately, assert zero request-triggered core calls and run
socket/process lifecycle acceptance twice. No real model/account/DB is used.

## Colab pilot development

The private local-build Colab executable serves an owner-only mounted socket,
owner-browser registration and stream sync, and lists local-space metadata.
Owner requests load the built browser app when its local output is available.
No installer exists. Rust builds and tests do not require a browser build.
Build and verify it from the repository root:

```bash
(cd rust && cargo build --offline --locked -p tmt-colab)
(cd rust && cargo test --offline --locked -p tmt-colab)
(cd rust && cargo clippy --offline --locked -p tmt-colab --all-targets -- -D warnings)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
node typescript/scripts/release-please-config.mjs --check
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts)
```

Tests inject temporary data roots; never point them at the real TMT directory.
The extension owns `<dataRoot>/colab/` (0700) and regular secret/state files
(0600). Production startup obtains the absolute root through `tmt api storage.root`
via `tmt-invoke`; no path guess or Colab root environment variable
is supported. Store bounds are named in `src/limits.rs`: 16 MiB plus 2 KiB per
opaque envelope, 64 MiB retained ciphertext and 100,000 durable update receipts
per page across epochs. Capacity rejects writes without eviction. Checkpoint
pruning keeps receipts and preserves the other namespace and concurrent tails;
it also reclaims superseded unpinned checkpoint payloads. `pin_checkpoint` is
the future verified authority-cut caller's preservation seam.
Object signatures and role admission are required at the future request boundary;
these storage tests prove transaction rollback and reopening, not crash recovery.

The owner-state foundation uses the same private database and the append-only
migration history in `src/store/schema.rs`. Schema 2 adds `owner_state`,
`membership_log`, `recipients`, `devices`, `epoch_secrets`, `wraps` and
`owner_operations`, without altering schema-1 tables. Epoch secrets are sensitive
local key material protected by the existing directory/file permissions.
Run its focused tests with:

```bash
(cd rust && cargo test --offline --locked -p tmt-colab --test owner_state)
```

The tests cover signed log/head and wrap round trips, operation replay/conflict,
rollback at each write stage, concurrent expected-head checks, preservation of
schema-1 rows and byte-for-byte database preservation on newer-schema refusal.
Exact mutation outcomes are capped at 32 MiB; exceeding the cap rolls back.
Callers must propagate transaction-method errors and perform live request,
certificate, policy and baseline admission before committing authority changes.
No management mutation route or CLI transition is enabled here; baseline production
is available separately through the isolated decoder library.

```bash
(cd rust && cargo build --offline --locked -p tmt-cli -p tmt-colab)
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app build
PATH="$PWD/rust/target/debug:$PATH" tmt colab spaces --json
PATH="$PWD/rust/target/debug:$PATH" tmt colab serve --json
```

After building a core supporting `storage.root` (#860) and the extension, put
`rust/target/debug` on PATH and run `tmt colab serve`. It listens only on its
owner-only socket `<dataRoot>/colab/door.sock`; a stale socket from an earlier
serve is replaced, any other file at that path refuses with
`COLAB_STATE_UNSAFE`, and a data root too deep for a Unix socket path refuses
with `COLAB_SOCKET_PATH_TOO_LONG`. Direct invocation requires an absolute
`TMT_EXECUTABLE`. `tmt colab serve --json` prints one plain JSON descriptor with
the space ID and socket path; Ctrl-C/SIGTERM closes sockets and tunnels, joins
workers, removes the socket and releases the service lock. `tmt colab spaces
--json` lists the local space and running state without creating directories or
keys; before first serve it returns `{"spaces":[]}`. Use an isolated normal TMT
data root for manual tests. Browsers reach colab through `tmt remote serve` at
`/r/<prefix>/x/colab/`; remote owns Host and Origin admission and forwards the
paired owner's device context.

By default, `serve` loads `extensions/tmt-colab/typescript/app/dist` relative to
its compile-time crate directory, canonicalized at startup. This is a local-build
binary: moving the checkout requires rebuilding it or passing
`serve --app-dir /absolute/path/to/dist`. The override is optional; an invalid
explicit directory fails with `COLAB_APP_UNAVAILABLE` before creating Colab state.
A missing, unsafe or incomplete default instead starts the socket and shows owners
`build the app: corepack pnpm --dir typescript --filter @tmt/colab-app build`.
Build and restart serve to adopt new assets. Files are snapshotted in memory, with
128-file/16-MiB total bounds, no symlinks and no request-time filesystem access.
Asset access needs remote's owner context, not prior Colab registration. Anonymous
root requests retain private-space guidance; other asset requests are denied.
App resources use same-origin relative URLs. The current font stacks fall back to
installed/system fonts without third-party font requests; the server also supports
bundled WOFF/WOFF2/TTF/OTF output. The exact app CSP and static route behavior are
owned by [colab-v1](extensions/tmt-colab/contracts/colab-v1.md#implemented-mounted-browser-assets-1253).

The socket bounds are named in `src/limits.rs`: 16 request workers, 8 KiB/32
header fields, 64 KiB HTTP bodies, 2-second total acquisition and 1-second total
response, and 16 WebSocket tunnels closed after 120 seconds without inbound
bytes. Bounded HTTP bodies carry registration requests; page objects use the
stream sync path. The stream sync library enforces 64 KiB frames and 8 queued frames with
`RESYNC_REQUIRED` close for slow subscribers; serve drives the sync library
over registered-owner upgrades. Real socket
and foreground process cleanup tests run lifecycle scenarios twice, with no core calls from
socket traffic. Owner-key temporary cleanup is publication-locked; it preserves
foreign file names and refuses unsafe matching files.

### Colab native export verification

After local page state exists, export its admitted source without running HTML:

```bash
PATH="$PWD/rust/target/debug:$PATH" tmt colab export 10000000-0000-4000-8000-000000000001 --json
PATH="$PWD/rust/target/debug:$PATH" tmt colab export 10000000-0000-4000-8000-000000000001 --dir /existing/export-parent
```

Use the actual page UUID. The parent must exist; the command creates a new UUID
subdirectory containing `page.html` and `manifest.json`. Existing output is never
replaced. Parent aliases resolve once and output reports the canonical path;
created entries never follow symlinks, and `..` is refused. Files are 0600 and
the output and staging directories are 0700. JSON returns `directory`, both file sizes and
SHA-256 values, and `disclosure`; human output shows the same plaintext disclosure
before publication. The manifest explicitly excludes discussions. Missing state
is not initialized or migrated. Archived/deleted pages currently return
`COLAB_EXPORT_INACTIVE`; archived reads wait for the #1348 policy split.
On publication failure, inspect any reported `error.partialDirectory`; partial
output is preserved, and cleanup touches only checked staging from that invocation.
If a parent was moved during publication, the partial path names its original
canonical location.

From `rust/`, run the focused behavior checks:

```bash
CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-colab --test export
CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-colab export::tests
```

Real SQLite, signed/encrypted objects and decoder children prove exact
CRLF/Unicode/NUL/empty source and title, baseline rotation, frozen/reopened
bundles, verified manifest hashes and admission/capacity denial. Real subprocesses
verify CLI help/human/JSON/defaults and read-only state handling. Descriptor-level
publication tests prove permissions, collisions, symlink refusal, manifest-last
partial output and preservation of replaced/foreign staging entries. Run the
normal Colab Rust gates and docs formatting before handoff. Browser download
verification is added with the subsequent #1309 browser slice.

### Colab owner transition verification

Run `(cd rust && cargo test --offline --locked -p tmt-colab --test transitions)`
for real SQLite/keyring and isolated-child fold/rotation tests. They cover exact
baseline materialization, remaining-recipient wrap decryption, checkpoint pins,
reopen replay, stale writes, tampered signatures/chains/descriptors, role/epoch/
root denial, revoked/expired-device exclusion and rollback on baseline/wrap
write failure. FIFO barriers pause the real child while a second SQLite writer
appends: one moving snapshot retries successfully; three return `STALE_HEAD`
without authority changes. No Docker or fixed sleep is involved.

Schema 4 adds `baselines(page, epoch, descriptor, envelope)` with a composite
primary key and an epoch-secret foreign key. Earlier tables are unchanged;
baseline ciphertext counts toward the existing per-page quota. Re-run
`--test owner_state --test registration` for legacy schema preservation and
registration/tombstone behavior. Newer schemas still refuse without mutation.

The local `transitions::Engine::advance_epoch` seam requires root authorization,
an operation ID and expected owner revision. It accepts no source, cuts or wraps
from a request. It verifies and decrypts stored envelopes, validates namespace
roots through the isolated decoder, and produces the baseline before its short
writer transaction. The exact signed response is replayed without regeneration.
A fold exceeding the existing decoder batch/update/baseline caps fails closed;
there is no truncation or fallback. The same test target covers shared/current
member joins, immutable owner membership, member removal, role reduction pins,
known-device revocation and rollback across every authority effect. A nine-page
join seeds signed/encrypted retained history with fresh Y.Doc identities and
proves the 64-epoch cap, numeric ordering, 512/64 wrap lists and all-or-none
rollback on the final page. Fresh operation IDs cannot rotate a revoked device
again. Browser/CLI device-signed management composition is tracked by #1111;
sharing/history/retention/archive/delete transitions are tracked by #1160.

Link transition fixtures use real model-derived signing/encryption keys and
certified device projections. They cover numeric ordering across 512-entry wrap
lists at the 64-epoch cap, current-only joins without rotation, Reset statement
order, old-seed/device exclusion, rollback on a late receipt failure and replay
after reopening. Byte searches over exact committed statements, decoded payloads,
receipts, projections, wraps and returned outcomes reject raw, base64url and hex
seed leakage. Link seeds are borrowed owner-local inputs; tests never claim this
library seam admits unsigned browser management requests.

Runner verification uses the same `--test transitions` suite. Real-store cases
cover transport-bound and root-local retries, unchanged legacy digests, scope
match/mismatch, conflicting replay, revoked-target replay and the original signed
head after subsequent writes and reopening. Existing membership/link/epoch cases
continue through their thin `Engine::apply` wrappers, including late-write rollback
and moving-snapshot retries. `OwnerRequest.scope = None` preserves root-local
composition; a browser management caller must admit its live signature/session
and supply its transport digest and scope. No management route is added here.

### Colab owner registration verification

Run `(cd rust && cargo test --offline --locked -p tmt-colab --test registration)`
for real SQLite/keyring persistence, strict certificate admission, exact retry,
one-year certificate validity/renewal, transaction rollback, revision-ordered
revocation and the mounted HTTP endpoint exercised twice with socket cleanup.
The independent management-key/remote-certificate oracle is
`python3 extensions/tmt-colab/contracts/vectors/authority-reference.py` (requires
the same Python cryptography tooling as the model foundation). Rust consumes
frozen vectors without Python.

The endpoint is `POST /api/devices/register` beneath the remote mount. Its strict
request, response, key derivation and failure codes are owned by
[colab-v1](extensions/tmt-colab/contracts/colab-v1.md#implemented-owner-browser-registration-1162).
Use the remote SDK's `certifyKey` for each purpose, and its authenticated owner
session; a cookie-only context cannot register. The response is committed before
HTTP success. Schema 3 adds device registration bindings/tombstones while retaining
all prior authority/ciphertext rows; newer schemas refuse without mutation.
An existing incompatible management-member key binding fails closed.

The trusted `Registration::revoke(deviceId, grantRevision) -> Result<bool>` callback
atomically signs and rotates for a known device, with a durable tombstone. Unknown
IDs only tombstone; equal/older events and already-revoked devices return false
without writes, including before genesis. Inject the decoder executable when
constructing Registration (`current_exe` in the executable, the product binary
in tests). The reserved socket consumer delivers remote events and closes matching
tunnels under the sync lock before acknowledgment. The registration suite checks
the callback; the socket suite checks owner-signed rotation and live tunnel cleanup.
The callback is not a browser management capability.

### Colab stream sync verification

Run `(cd rust && cargo test --offline --locked -p tmt-colab --test sync)` for
real duplex-socket tests of append/broadcast, exact retries, durable conflicts,
gaps, role/signature/epoch denial, capacity, strict frames, revocation and
slow-subscriber cleanup. Tests drive server turns explicitly without sleeps,
worker threads, real identities or Docker. An injected write gate over a real
socket proves that a buffered frame counts toward the queue cap and that
revocation never flushes blocked ciphertext.

The transport accepts already-upgraded nonblocking streams and a caller-supplied
`Admission` implementation; `serve` now supplies `registration::OwnerAdmission`
and socket workers. The caller must drive
readiness and the one-second blocked-write deadline, drop connections on shutdown,
and supply current verified membership/device policy. Model statement/certificate
verification remains that caller's responsibility; a successful upgrade is not
page authority. The admission implementation supplies the verified retained owner
head through `Store::owner_head` and the exact persisted baseline descriptor
through `Store::baseline`.
A non-null first-page descriptor includes `baselineObject`
from the matching Store record; tests cover inline and twelve-chunk delivery,
exact hashes/bytes, missing records, descriptor/scope mismatches and length caps.
Strict hello includes the last verified `membershipRevision`; bootstrap emits
bounded exact membership pages, author chains once per connection and
retained device/member wraps before stream objects. After hello every outbound
frame costs one of eight credits; a scoped valid ack returns one credit, even
with empty cursors for metadata/partial chunks. Test full-window silence,
missing/unsolicited acks and a twelve-chunk stored object across credit releases.
Reference/chunks stay consecutive and live overflow still resyncs.
One hello starts lazy catchup;
its final page enables live delivery under the server lock. Unknown/pruned cursors
return `RESYNC_REQUIRED`. An empty-cursor subscription remains live-only. Large
updates use one bounded, deadline-limited inbound transfer before append verification;
large broadcasts/catchup objects stream chunks lazily. Tests cover more pages/chunks
than queue slots, concurrent appends during catchup, paired namespace checkpoints,
exact reassembly/replay, partial-byte isolation and transfer failure/cleanup.
Shared sequence tests verify signed checkpoint heads and every subsequent hash
across interleaved content/own tails. An unpaired checkpoint leaves full history
available and bootstrap uses the previous pair, or the complete update chain.
Run the Store read cases with `cargo test --offline --locked -p tmt-colab --test
state`. Store cases prove same-head pairing, unpaired/mismatched-head retention,
failed partner rollback, pinned checkpoint retention and durable receipts after
paired pruning and reopen. The caller drives acquisition and blocked-write deadlines even without
socket input; `Connection::poll_at` accepts a monotonic instant for deterministic
verification. Exact wire shapes and budgets are owned by colab-v1. Owner
registration is implemented under #1162. Run
`(cd rust && cargo test --offline --locked -p tmt-colab --test socket)` for two
registered owner tabs through the real mounted socket: append/broadcast, durable
retry/catchup, read-only `/api/session` and `/api/pages` owner discovery,
130-revision exact-byte membership paging and unknown-revision resync,
large signed statements through exact chunks across the eight-frame credit window,
resumed first-page references and SQL-side statement-size refusal,
inline/chunked baseline-first ordering before statements for fresh/resumed clients,
strict event bodies/header/path, failed-revoke rollback, replay
without writes, active/pre-hello tunnel closure, cap/idle bounds and shutdown.
Tests inject private temporary roots and verify socket removal; they use no
Docker or remote identities. Full two-browser application acceptance is later.

The transport uses the existing workspace tungstenite 0.30.0 edge in `tmt-colab`
(default features disabled, handshake enabled). Mounted composition adds no
dependencies or lockfile resolutions.

### Colab decoder verification

The decoder takes caller-admitted `decoder::UpdateBatch` values through a
caller-owned library runner and private child entry, with no
server integration or public decode command. Build the native executable and
run its real-child tests with an isolated test environment:

```bash
(cd rust && cargo test --offline --locked -p tmt-colab --test decoder -- --nocapture)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
```

The decoder test retains #830 archive provenance and runs 261 seeded cases twice,
using one second per case and 45 seconds per suite. The six saved timeout/panic
dumps must match the original generator. Successful child PIDs must be gone;
timeout/output-limit cleanup must be confirmed before owner reuse. These fixture
budgets are separate from the two-second production budget. macOS must report
`memory limit unavailable`; only Linux enforces the child address-space limit.
Tests cover cleared environment, input/output backpressure, role/namespace and
projection rejection, dependency/delete-set preservation and writer attribution.

`Decoder::produce_baseline` accepts exact UTF-8 source bytes, title and source
digest from the owner's authenticated fold. It returns one fresh update-v1,
source digest and commitment after child materialization verification. Persist
and distribute these exact bytes; calling the producer again creates a new
struct identity. The caller still owns log/page/epoch admission and signing.
`verify_baseline` checks a supplied update/commitment against that view in the
same child. Source is capped at 2 MiB, title at 256 KiB, and update-v1 at source
plus title caps plus 1 KiB framing. Both modes retain 4 MiB serialized streams,
the two-second deadline and cleanup fencing. Verification sends the update,
authenticated source digest and title; the child reconstructs the exact source
and checks its digest and commitment without sending a second source copy.
Tests produce and verify at exactly the 2 MiB source cap and reject 2 MiB + 1.
Check the independent minimal update-v1/commitment oracle with:

```bash
python3 extensions/tmt-colab/contracts/vectors/baseline-reference.py
(cd rust && cargo test --offline --locked -p tmt-colab --test decoder baseline_)
```

Frozen vectors include empty, CRLF and Unicode/NUL fixture text; those code
points are intentional exact-byte test data. The oracle imports no product code
or third-party libraries. The Rust producer matches it at a test-only fixed
client ID; real-child tests verify those bytes and concurrent clients applying
one production baseline. This does not prove browser/epoch-transition wiring.

The exact yrs 0.28.0 dependency brings smallstr 0.3.1.
[RUSTSEC-2026-0215](https://rustsec.org/advisories/RUSTSEC-2026-0215.html) is an
INFO unmaintained advisory with no patched version. Its disposition is retained
as maintenance debt for this pinned integration, with decoder containment and
hostile-corpus gates; it is not a claim that the decoder is safe or sandboxed.

## Colab model foundation

The private Rust model has no server or CLI. From `rust/`, run
`cargo test --offline --locked -p tmt-colab-model` and
`cargo clippy --offline --locked -p tmt-colab-model --all-targets -- -D warnings`;
workspace boundary changes also require the architecture guard above.
Tests consume frozen contract vectors without Python. To check/regenerate the
independent namespace/sign-in oracle, use Python with `cryptography` installed:
`python3 extensions/tmt-colab/contracts/vectors/model-reference.py` and
`python3 extensions/tmt-colab/contracts/vectors/authority-reference.py` from the
repository root; add `--write` only after reviewing changed bytes. Fixture keys
are public test data. This foundation does not satisfy the complete L1 gates.

### Colab browser verification

The private local page app has its own package and Chromium isolation suite.
Build `tmt-colab` before `test:browser` so the real-socket asset scenario can start
`rust/target/debug/tmt-colab`; `COLAB_SERVE_EXECUTABLE` optionally selects an
absolute test binary built from this checkout. The scenario runs twice with a
temporary data root and a test HTTP-to-Unix-socket adapter, comparing actual built HTML/JS/CSS bytes, MIME
types, nested mount loading, default/override selection, owner asset denial and
process/socket cleanup. It also exercises the opaque sample renderer under the served app CSP, and proves
cross-origin requests are blocked with a same-browser capture-server positive
control. The adapter supplies owner context and a fixed core storage-root response; it does not mock
static assets or add product API routes. The mounted shell reaches its existing
blocked state because the test door does not serve Remote's SDK. This is native asset-serving evidence, not Remote pairing
or complete native co-editing acceptance.
Its test/build/dev entry points use workspace-pinned Vite+; `vitest.config.ts`
keeps app unit discovery separate. TypeScript and Oxlint retain their existing
check responsibilities; the existing `check` script selects the Vite configuration's
`fmt` block, preserving the app's single quotes, all trailing commas and 100-column
width with import and package-key sorting disabled:

```sh
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match check
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match build
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app exec playwright install chromium
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test:browser
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match dev
```

The Ask preview foundation (#1312) has no production selection/threads entry
point or live remote operation adapter. `test/ask.test.ts` verifies independent
canonical/signature vectors, immutable async inputs, composed byte bounds,
persist-before-send, metadata-only storage, conflicts, double-click coalescing,
held/uncertain outcomes and no retry through a deterministic RemoteClient double. `e2e/ask.spec.ts`
mounts the parent component through the test-only `test/ask-browser.tsx` entry
and uses real WebCrypto/non-extractable keys, IndexedDB and Web Locks. It checks
inert exact/control-byte previews, disabled absent runtime, trusted-click-only
signing, frozen live-source inputs, stored-record exclusion of message bytes and
durable reload without another send.
These tests prove the browser primitive, not remote delivery or the L5 real-TMT
acceptance gate. No real TMT home or provider is used by this fixture. Screenshot
evidence is written under `/private/tmp/colab-1110-design/`.

The frozen send-byte oracle is checked with
`python3 extensions/tmt-colab/contracts/vectors/send-preview-reference.py` from
the repository root, using the existing Python `cryptography` tooling described
below. Add `--write` only after reviewing changed bytes. App tests read the
frozen JSON without Python. The public RFC 8032 seed and exact Unicode/control
characters are intentional fixture data. No browser/SQLite version migration,
new dependency or lockfile resolution is required by this foundation.

The dev server binds loopback and serves in-process sample pages. The paired mount
client path is tested with Vite plus signed protocol fixtures: first-use key
persistence/non-extractability, registration failure, root pin mismatch, strict
owner-log/author-chain admission and missing-wrap blocking. The live fixture
exercises two same-device tabs, a single durable writer, large chunked updates,
reload reconstruction and retry of exact accepted bytes after receipt interruption.
The fixture paces server delivery with the existing ACK frame; native delivery
uses the eight-frame ACK window described in the sync section. It covers unpruned content updates
from sequence one, plus signed reset-baseline fixtures. The baseline suite covers
chunked retrieval, descriptor/log binding, exact baseline struct identity, subsequent
edits/reload and rejection of commitment/source/descriptor/old-epoch substitution
without partial rendering. Paired-checkpoint fixtures cover raw merged update-v1
bytes above 256 KiB, chunked retrieval, interleaved content/own tails, reload/edit
continuity and
rejection of prefix sequence/hash, namespace, body and gap substitutions. Signed
revocation tests require exact pinned checkpoints and cut endpoints. Checkpoints
use single-item Worker steps bounded by its 4 MiB state cap (also the aggregate
checkpoint catchup cap); tails remain bounded to 200 updates/256 KiB. Dependency
resolution is required at the final tail step, before any renderer publication. Own ciphertext
is authenticated for the chain only, with an explicit parent notice; it never
enters the decoder. The shared `checkpoint-v1.json` fixture also runs through the
bounded native decoder child (`cargo test --manifest-path rust/Cargo.toml --locked
-p tmt-colab --test checkpoint_vectors`). Native bootstrap supplies paired checkpoints, their full
cross-namespace tail and the matching stored baseline object. Statement
fixtures cover signed envelopes above 64 KiB across ACK-paced frames, baseline-first
ordering, exact durable log/reload and rejection before renderer publication.
Admission units cover hash/signature/payload/target/chain substitutions, write failure
and durable-prefix conflicts without head advancement; framing units cover shared
assembly identity/order/size limits, interleaving, the absolute deadline and cleanup.
This is signed
protocol-fixture evidence, not native mounted browser E2E: #1250 owns refresh;
the separate real-socket scenario verifies native assets.
The content Worker suite proves concurrent writer convergence and reload
reconstruction, rejects malformed/mixed roots, checks termination/cleanup and
proves prepared edits cannot leak through committed projections.
The unchanged renderer suite proves opaque origin isolation, CSP request blocking,
source-digest/window binding and teardown, including the permitted self-navigation
request before teardown. These app fixtures do not establish real mounted
co-editing or replace the primitive library's three-engine conformance gate below. Code quality runs
filtered frozen install, check, unit tests and build; renderer tests run locally.

The private browser primitives use the existing frozen pnpm workspace. From the
repository root:

```sh
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match check
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match test
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client exec playwright install chromium firefox webkit
(cd rust && cargo build --locked -p tmt-colab-model --example browser_conformance --example browser_authority)
COLAB_REPORT=/tmp/colab-browser-results.json corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match test:browser
```

On Linux, add `--with-deps` to the Playwright install command to install system
libraries too. Browser binaries are explicit test dependencies; the harness
requires Chromium, Firefox and WebKit, all 148 strict Ed25519 rows and nine
accepted controls in each. Missing/skipped/error engines, missing controls or
wrong row counts fail nonzero; unit tests exercise that refusal. It records
engine versions and raw-verifier bypass results and checks fresh ciphertext
interoperability both ways with the Rust model. This is not a product server.

`COLAB_RUST_TOOLCHAIN` selects an installed Rust toolchain (default `+1.97.0`).
Optional `COLAB_CHROMIUM_EXECUTABLE`, `COLAB_FIREFOX_EXECUTABLE` and
`COLAB_WEBKIT_EXECUTABLE` select explicit local binaries; launch failures never
skip an engine. Reports default to ignored `differential-results.json`; use
`COLAB_REPORT` to retain evidence elsewhere. Browser/server cleanup runs even on
engine failure. Build the two examples first so cold compilation does not consume
the harness's bounded native-call deadline. Python is not needed at test time.

The default always requires all three engines. For a scoped diagnostic, append
`--engines chromium` to `test:browser`; comma-separated known engine names are
also accepted. Pass the flag directly, without an extra `--` separator:

```sh
COLAB_REPORT=/tmp/colab-chromium-results.json corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match test:browser --engines chromium
```

Empty, unknown or duplicate sets reject before browser/Rust work.
The gate requires exactly the requested set with full rows/controls and fails
on any unavailable requested engine. Reports contain `{engines, results}` and
logs name the selected set; Chromium-only evidence is not a three-engine pass.
Developers run the default complete three-engine harness locally before handoff.
The separate advisory `Colab browser verification` workflow runs Chromium on
scoped PRs, outside required CI aggregates. It has no main-push trigger. Weekly
and manual runs use all three engines as the safety net for other shared inputs
and engine drift. The
[CI selection and worker model](ARCHITECTURE.md#ci-selection-and-worker-model)
owns scope and cache policy. `COLAB_HARNESS_ROOTS` in `ci-scope.mjs` owns the
narrow client/model/vector inputs within Colab component ownership;
`COLAB_HARNESS_INPUTS` also selects the workflow, `rust/Cargo.toml`,
`rust/Cargo.lock` and `typescript/pnpm-lock.yaml` on PRs.
The client's `package.json` is already within the client root. Runs retain the available JSON report and command
log for seven days, including failure evidence. `tmt-lead` confirms a green run
at the reviewed head when accepting later L1 PRs; a Chromium-only advisory run
is not full three-engine L1 acceptance.

## Project release tracking

`project-release.yml` sweeps all existing closed issue items from
[pj-tmt organization project 1](https://github.com/orgs/pj-tmt/projects/1), regardless
of their current Status, except issues labeled `epic`: the owning lead retains
both tracker fields under [Tracker rules](#project-tracking). Each skipped epic
appears in the per-item summary as `skipped: epic tracker`. The release App token uses the existing
`RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY` in the main-only `release`
environment, with **Organization projects: read and write**, plus issue and PR
read permissions. The pinned token action requests only those permissions and
revokes the token on cleanup. Only reconciliation receives it. The separate
`GITHUB_TOKEN` reads published releases; no PAT or `PROJECT_TOKEN` is used.
Missing credentials or incomplete permissions fail visibly before writes.

The daily cron remains 04:23 UTC. After publication read-back succeeds and all
smoke jobs conclude, `native-release-bundle.yml` explicitly dispatches a full
sweep with `GITHUB_TOKEN`. It accepts successful smoke or a complete failure set
classified by the existing reporter as GitHub API rate-limit infrastructure.
Missing/mixed failure evidence and actual release failures do not qualify for
this immediate dispatch; the scheduled sweep still reconciles repository state.
No failed smoke job is made successful. The calling native workflow grants
`actions: write` to the reusable-workflow boundary; only its dedicated dispatch
job requests that permission, with no content or issue access and no checkout.
`workflow_dispatch` works with `GITHUB_TOKEN`, avoiding the suppression that
made `workflow_run` unreliable. The redundant completion trigger is removed.
The updater neither publishes nor blocks publication and always executes main's
trusted code, with full git history and tags, never release-tag code or artifacts.

Each run discovers all published supported releases and all Project items within
explicit bounds. GitHub's paginated `closedByPullRequestsReferences`, including
closed PRs, supplies merged closing PRs. Local git reads the first-parent merge
delta (including deleted paths and both sides of renames) and tag containment.
The existing component owner map assigns products; its private-leaf consumers
are reused. Existing historical Office tags remain evidence even while Office
publication is parked. Components without a native publication policy stay
Merged with an explicit waiting reason. For each affected product, the first
publication containing all relevant closing merge commits becomes the sole
canonical entry. All products must be present for Released. Closed issues with
no merged closing PR use the closed-issue state defined in [Project tracking](#project-tracking),
with empty release evidence. Open issues, epic trackers, PR items and foreign-repository
issues are untouched.

The sweep owns Status and Released in for these closed issues: it replaces stale
text and corrects incorrect terminal states instead of trusting previous field
values. It does not preserve manual text in the generated Released in field.
The per-item summary shows current → planned Status and Released in, including
unchanged items and waiting reasons. This is also the built-in-workflow strategy:
full sweeps are authoritative on every run, repairing late close/merge workflow
writes to Merged. The fixture explicitly resets Released and Done to Merged and
proves convergence; no Project settings are changed or claimed to have been
inspected. Concurrent external changes can cause readback failure; the next full
sweep repairs them. Avoid manual edits to these two fields during a live run.

For initial activation, run a full dry run first and review the per-item step
summary against the first write-enabled run. There is no tag replay or latest-ten
window. After merge, an authorized operator can dispatch the non-publishing
updater through REST:

```bash
gh api repos/pj-tmt/tmt/actions/workflows/project-release.yml/dispatches \
  --method POST --input - <<'JSON'
{"ref":"main","inputs":{"dry_run":"true"}}
JSON
# After reviewing the table, repeat with dry_run=false.
```

A run allows at most 200 GraphQL requests, 20 REST requests, 20 pages per connection and 2,000 distinct merged closing PRs. Release and Project
item pages contain 100 rows; closing-PR pages contain 10. Project field schemas
must fit a complete 100-row page and each issue’s labels a complete 20-row page,
or discovery fails before writes. Smaller nested connections limit point cost
on the shared release App token. Both issue reads and field
writes batch 25 aliases. Ordinary discovery costs R REST release pages plus P
GraphQL Project pages and ceil(I/25) closing-PR queries for I closed issues,
plus any additional closing-PR pages. Each mutation phase costs ceil(F/25) for
its changed fields; a live write run adds P readback pages. No-op runs write
nothing. Mutation reservation includes all planned field batches plus 20 possible
readback pages before the first write. A ceiling can be reached before a nominal
row cap: incomplete discovery or insufficient reserve aborts with no writes.
Requests have a 30-second timeout with no retries or polling; the job deadline is
15 minutes. Each GraphQL read includes `rateLimit { cost remaining }`. The
summary and failure message report exact REST/GraphQL attempt counts, summed
reported read points and the last reported remaining quota. Mutation costs and
requests without returned telemetry are not included in that point sum; missing
valid telemetry on an otherwise successful read fails before proceeding.
Local git requires complete history and every published tag; missing evidence
fails visibly. False Released states are demoted before text changes, and new
Released promotions follow evidence. Partial failures remain safely retryable.

Run fixture-only checks without live API calls or Docker:

```bash
cd typescript
corepack pnpm exec vp test run --config vitest.config.ts test/tooling/project-release.test.ts test/tooling/release-workflow.test.ts test/tooling/release-publish.test.ts
corepack pnpm check:tooling
cd ..
actionlint .github/workflows/project-release.yml .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-smoke.yml
```
