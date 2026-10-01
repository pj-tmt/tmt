# Development

The Rust workspace is the shipped CLI runtime. The optional Office SPA foundation
lives in `extensions/tmt-office/typescript/apps/office` and is not required to use the CLI. The nested
`typescript` pnpm workspace owns private developer tooling for Vitest, fixtures
and release verification. Nx orchestrates repository tasks without making the
tooling workspace an npm product or a CLI fallback. Repository policy is
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
from `typescript/`; Cargo, Nx and Docker commands run from the repository root.

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
NX_DAEMON=false NX_INTERACTIVE=false ./nx show projects
```

The checked-in non-JavaScript Nx wrapper pins Nx 23.2.1 in `nx.json`. On first
use, the official wrapper uses npm only to populate ignored `.nx/installation`
runtime files; this isolated bootstrap is not a product workspace, a lockfile
owner, or an alternative to the nested pnpm commands above. Nx task caching is
disabled while this layout is established, and automated checks disable the Nx
daemon and interactive prompts.

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

**Each worktree keeps its own `rust/target`.** Native selectors, the Nx
targets, `scripts/tmt-dev.sh` and the Docker fixtures all read
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
2. either the PR is merged (`gh pr view <n> --json state` shows `MERGED`), or
   the branch has an upstream and `git log @{u}..` prints nothing.

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
NX_DAEMON=false NX_INTERACTIVE=false ./nx run native:tmt -- --version
(cd typescript && corepack pnpm tmt office --help)
(cd rust && cargo build --locked -p tmt-office)
./rust/target/debug/tmt-office --help
(cd rust && cargo build --locked -p tmt-squad)
./rust/target/debug/tmt-squad --help
```

The `native:tmt` Nx target and nested `pnpm tmt` script both launch only this
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

Extensions can use the public [local process API](docs/extension-api.md) for
structured dispatch, history, conditional room writes and bounded notebook reads.
Its contract is owned by [architecture](ARCHITECTURE.md#local-extension-api-v1).

## Office SPA

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
[Office architecture](docs/office/architecture.md) before changing its boundaries.
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
on empty discovery. Office uses Oxfmt; tooling and repository docs use the
`typescript/.prettierrc` Prettier configuration. Run
`pnpm --filter @tmt/office format` for app formatting, not the tooling formatter.
Root tooling, native, stress and Docker suites use exact Vitest 5.0.1 alongside the
extension packages; their separate configurations retain their own test discovery.
Each test configuration sets `clearMocks: false` to preserve mock history, and
ordered suites use `{ concurrent: false }`.
Vitest 5 changes generated `it.each` case labels: `$field` strings lose
quotes, and percent placeholders use the new value renderer (including signed
zero and object clipping). Migration parity records each changed label with its
source template and case index alongside both actual titles and equal statuses;
unchanged labels remain exact multiset matches. Preserve the raw reports rather
than silently normalizing these differences.
Office wire-schema conformance is a nested tooling test. From `typescript`, run
`corepack pnpm exec vitest run test/tooling/office-contracts.test.ts`. See
[`extensions/tmt-office/contracts`](extensions/tmt-office/contracts/README.md) for its single source of truth,
versioning and limits. Design vectors are not executable authorization or crash
recovery evidence; downstream suites must prove those behaviors separately.
Root tooling uses the threads pool with at most two suite workers. Native process, stress and tmux
configurations also select the threads pool and retain their own execution rules.

For an Office-only clean checkout, use `pnpm office:install`. It installs from
the app directory against the same workspace lockfile, without workspace recursion.
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
builds. A conservative
diff selector, driven by the component map `.github/components.json`, skips expensive
native jobs only for Office-only paths, and skips Office for native-source/skill-only
paths, Office's Rust crates, core-only test suites and E2E scenario files. Prose that
no job reads selects nothing beyond Code quality, and the run summary lists every
changed path with its owner, rule and selection. A change confined to the Squad extension
runs a Squad scope under the same job names (its Cargo checks and the architecture guard,
its native tests, its E2E file); the map's Squad `scopedChecks` name the tests, and
`Native package matrix` expects exactly the scoped results. Shared/unknown paths run both.
Remote Rust has an explicit rule retaining full native and Office coverage; the full
Rust checks require nonempty remote test discovery and run locked workspace tests,
Clippy and builds. A parallel `Native Rust MSRV` job runs
`cargo +"$MSRV" check --locked --workspace --all-targets` for both full and Squad
scopes, reading `MSRV` from `workspace.package.rust-version` in `rust/Cargo.toml`.
Rustup resolves the manifest's two-part minimum to its latest patch release,
rather than duplicating a patch pin in the workflow.
It replaces the MSRV executable builds and expands Squad MSRV coverage to the
whole workspace without changing the declared minimum. Its separate
`native-rust-msrv` cache has one writer, the MSRV job on main; PR and merge-group events only restore.
`Native Rust contracts` is the fail-closed aggregator of these two workers. It
requires both to succeed, rejects missing selection, and stays skipped for scope
`none`, preserving the outer native gate and required-check names.
The remote TypeScript and browser paths are outside that Rust rule. Code
quality includes the selector's own focused tests even when native unit jobs are
unselected, and requires the selected Office check. The native aggregator rejects
failed, cancelled or unexpectedly skipped selected jobs. The advisory Office browser
partitions run in their own workflow (below), so a red `CI` run means one of its own
jobs failed. CI changes need positive
and negative selection/gate evidence before pushing; do not change branch
protection merely to get a newly skipped job accepted.

Merge-group candidates use `node typescript/scripts/ci-scope.mjs merge-group
"$BASE_SHA" "$HEAD_SHA"` with the event's exact base/head SHAs. The two-dot diff
covers the cumulative group; docs-only groups skip native suites, Squad-only
groups run Squad checks, and shared changes select the full native scope. Empty
or unreadable diffs fail closed to full native/Office verification with both E2E
shards, and the selection summary reports the fallback. PR merge-base selection
is unchanged. The macOS exception is described in the runtime smoke matrix below.
Check event wiring with `pnpm exec vitest run test/tooling/ci-scope.test.ts`
from `typescript/` and `actionlint .github/workflows/ci.yml` from the root.
[Architecture](ARCHITECTURE.md) owns the event, gate and main-ref cache policy;
queue/ruleset changes remain a repository-owner operation.

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
type checks or release-config tests, run `pnpm install --frozen-lockfile --ignore-scripts` in
`.github/release-please/`; tests and the release wrapper load this single isolated pin.

Private leaves declare `releaseConsumers` in the component map; today only TUI names Squad.
The release workflow uses `release-please-run.mjs` with the pinned API to attribute these commits
before the ordinary splitter, excludes and product release cutoffs. No `additional-paths` option
exists in 17.11.2. An upgrade must re-verify the API shape and run
`pnpm exec vitest run test/tooling/release-please-config.test.ts` from `typescript/`:
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
(including its test fixtures) plus `docs/office/**` and browser-specific machinery
listed in `selectOfficeBrowser` (the browser workflow, emulator verifier and
Docker context policy). Shared dependency/selector/generic fixture changes rely
on the weekly/manual safety net to catch Office build breakage. Scheduled and manual runs cover all twelve
partitions, including the emulator, regardless of paths. Required CI selection
remains conservative and independent of this advisory cost policy.
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

A change affects Office when `typescript/scripts/ci-scope.mjs` selects
`native_office` for it: Office's own app, service, crates, contracts and skills;
every workspace crate under `rust/crates/` other than `tmt-cli`, except the modules
the script's verified denylist names; the CLI Office facade, API command and native install
commands; workspace build inputs; the shared test support and E2E harness the
image reads; and any path the script does not recognize. Squad, core skills, other
CLI code, prose outside Office, core-only test suites and E2E scenarios do not
affect it. `ci-scope.mjs` owns this mapping (the component map owns the rest of the
selection); its tests recompute what the Office crates and the API module reach across
the workspace crates so the denylist cannot go stale.

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
Colors, fonts and marks come from `site/src/design/tokens.json`, which the
stylesheet and the design page read. Anything not in a release is marked
planned.

```sh
cd site
pnpm install --frozen-lockfile
pnpm dev                        # local preview at http://127.0.0.1:5173/tmt/
pnpm check                      # types, oxlint and oxfmt
pnpm build                      # dist/ for GitHub Pages, one index.html per route
SITE_BASE=/ pnpm build          # for a root path, such as a custom domain
SITE_BASE=./ VITE_SITE_HISTORY=hash pnpm exec vite build   # a preview at an unknown path
```

`.github/workflows/site.yml` checks and builds the site on pull requests and
`main`. It deploys to GitHub Pages only from a manual run on `main` with
`deploy` set. The repository is public, so a deploy publishes the site; the
owner decides when.

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
[modular visual package](docs/office/references/rooms-and-walls/modular-v1/README.md).
`native-local-furniture-rotation.spec.ts` verifies retained static furniture's
directional successor through corner gestures, precision rotation, exact
Undo/Redo and restart. Its runtime gallery captures all four views of the
[directional furniture](docs/office/references/furniture-rotation/README.md);
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
NX_DAEMON=false NX_INTERACTIVE=false ./nx run office-spa:build-local
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

For the unregistered Codex queue transport (#736), focused deterministic checks
are `cargo test --locked -p tmt-adapters drivers::codex::queue` and
`cargo test --locked -p tmt-adapters drivers::codex::transport`. The transport
tests own local loopback peers and exercise receipt loss and absolute deadlines.
The refusal fixture holds a bound, non-listening socket through the connect attempt;
it never releases a port for a parallel test to claim. It uses the existing nix
Unix dev-dependency with `net`, without a new runtime dependency. These tests do
not start a model or inspect provider credentials. Dependency review
also records exact features/graph, Rust 1.95, licenses, current advisories and an
actual CLI release baseline/candidate under one toolchain/profile. Label a
zero delta from unused/dead-stripped groundwork honestly and repeat the size
measurement after the final consumer links it. See the
[owning contract](contracts/codex-channel-v1.md) for remaining integration gates.

Private Codex enrollment state (#737) is checked with
`cargo test --locked -p tmt-adapters drivers::codex::record`. These tests own
isolated temporary records and inject liveness evidence; they verify lock-scoped
state changes and replacement preservation, not real crashed-server recovery.

Owned Codex startup and attachment planning (#738) are covered by
`cargo test --locked -p tmt-adapters drivers::codex`. The server cases launch
isolated shell stand-ins and check observable process/file cleanup; cwd probes
compare relative and absolute `-C`. They do not start Codex or a model and do not
replace the final live foreground continuity gate.

The adapter `process::cleanup_policy_tests` must pass under both `cargo test` and
nextest: isolated re-exec cases prove timeout cleanup regardless of whether the
test runner makes its harness a process-group leader.

For cumulative completed-request counters (#872), run
`cargo test --locked -p tmt-adapters runtime::consumption` and
`cargo test --locked -p tmt-cli --bin tmt output::tests`. Redacted real provider
fixtures and source provenance live beside the runtime owner; failure and reset
variants are assembled. `usage-hooks.e2e.test.ts` verifies admitted hooks,
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
  test thread holds its descriptor (Squad's `test_support::write_executable`).
- The product writes the executable and then execs it, as an installer and its
  verifier do: the fixture waits out the window with a bounded retry of only
  that error (`tmt-office-command`'s `test_support::install_office`,
  `retry_on_text_file_busy`).

The opt-in stress test `cargo test -p tmt-office-command text_file_busy_stress
-- --ignored --nocapture` reproduces the race and reports failures with and
without the retry.

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

The CLI style guards ([enforcement](docs/cli-style.md#enforcement)) run in
`cargo test`. When a migrated command leaves its list, run them directly from
`rust/`: `cargo test --locked -p tmt-cli --bin tmt cli_style`, `cargo test
--locked -p tmt-squad cli_style` and the architecture test below. A failure
prints the exact list entry to add or remove.

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
tokens. Run the architecture test for dependency changes.
The crate has no executable or board consumer; these tests use in-memory XML,
not application configuration, SQLite or a terminal. Later admission/rendering
stages must not treat an admitted template as a fully validated scene.

The internal static subset is `flex`, `flex-row`, `flex-col`, `w-N`, `h-N`,
`w-full`, `h-full`, `gap-N`, `gap-x-N`, `gap-y-N`, `p-N`, `px-N`, `py-N`,
`grow`, `grow-N`, `shrink`, `shrink-N`, and `truncate`. `N` is ASCII decimal
0..4096, in cells (grow/shrink are integer weights). Overlapping properties,
even equal duplicates, fail rather than applying class order. No fractions,
variants, arbitrary values or CSS units are accepted. Padding is symmetric per
axis. View/col default to column direction; other elements default to row.
Sizes default to auto, gaps/padding/grow to zero, and shrink to one. Full means
the parent's available axis. Text defaults to clipping; leaf `wrap="true"` or
`wrap="false"` selects wrapping or clipping and conflicts with `truncate`.
`token` must name a shared `Role`; omission preserves inheritance. Binding
paths, dynamic tokens and row-track attributes await their owning later stages.

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
TMT_TEST_HERDR=/tmp/hdrbin/herdr pnpm exec vitest run --config test/native/vitest.config.ts test/native/herdr.test.ts
```

It starts a headless server on a short private socket with update checks off,
runs commands inside its panes, and fails if any server process remains.

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
(cd typescript && corepack pnpm exec vitest run test/tooling)
```

Native process tests must prove the missing-native negative control and selected
native positive control. Child processes are finite, are stopped and reaped
before fixture deletion, and receive signals only when they are task-owned.
Tests never use host tmux, global provider state, or process-wide environment
mutation as setup.

Use `withSandbox` for callback-owned native fixtures. Its descriptor clones
share active runs; disposal stops outstanding commands before deleting files
and rejects later launches. Each run has its execution deadline plus at most
one second to confirm direct close and process-group exit. Unconfirmed cleanup
fails and reports the retained fixture path instead of deleting potentially
live state. Focused lifecycle regressions live in `typescript/test/tooling/cli-process.test.ts`;
they use explicit Node fixtures, not a product-runtime fallback.

### Squad extension

`typescript/test/native/squad.test.ts` runs `rust/target/debug/tmt-squad`, or an
absolute path in `TMT_TEST_SQUAD`, through real `tmt` dispatch. It uses a
sandbox PATH holding the `tmt-squad` and `tmt-sq` links, with no installed-copy
fallback. It observes rooms and metadata through an independent SQLite reader.
A workspace `cargo build --locked` produces the default executable. Squad unit
tests run with `cargo test --locked -p tmt-squad`. For dependency changes,
compare `cargo tree -p tmt-cli -e normal,build -f '{p} {f}'` with `main` and the
package-scoped release `tmt` (see Rust checks) to prove the CLI is unchanged.

### Provider setup and lifecycle verification

Setup planning/publication tests use disposable settings files and preserve user
hook/permission bytes, exact reruns, recovery copies and changed-input refusal.
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

Codex channel foundation tests are provider-local (`drivers::codex` in the
adapter library): record foreground/takeover/withdraw, startup cleanup certainty
and permission/cwd planning. They use disposable state and owned stand-ins; they
do not establish live-provider or shared product-routing acceptance. See the
[contract](contracts/codex-channel-v1.md).

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
`check-routing.e2e.test.ts` and `session-routing.e2e.test.ts`), and `TMT_E2E_ADAPTER_TESTS=0` skips the Rust adapter tests. CI runs the suite as two
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

For archive, installer, upgrade, bootstrap or publication changes, read the
complete [native release verification guide](docs/native-release-verification.md)
before acting. It owns the exact commands, actual-archive evidence and authorized
publication gates. Ordinary changes do not need to load release-only procedures.
The runtime smoke matrix above remains part of native PR verification; raw
executables are not proof of release archives or public installation.

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
Follow `USER-GUIDE.md` and `skills/README.md` for provider/custom-root usage; do
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

## Project tracking

Progress is read from one place: the `pj-tmt` project
(<https://github.com/orgs/pj-tmt/projects/1>), filtered to `label:feature`.
Each product feature has one tracker issue titled `Feature: <name>` with the
`feature` label. The project's Sub-issues progress counts only direct
sub-issues, so the tracker is the only parent that matters for progress.

Every issue carries these Project fields:

- `Feature`: the tracker it serves. The issue is also a direct sub-issue of
  that tracker. Do not hang slices under an umbrella issue that is itself a
  tracker child; umbrella or findings-log issues stay outside the tracker.
- `Squad`: the squad whose lead owns the issue.
- `Status`, which moves forward only:
  - `Todo`: not started.
  - `In Progress`: implementation started, including draft or stacked PRs.
  - `In Review`: a PR is ready for review or queued. In a stacked chain, the
    issue stays here while any of its PRs is still queued.
  - `Merged`: the last required PR is on `main` and a release is pending. A
    `Fixes #N` merge moves the issue here through the Project workflow.
  - `Released`: shipped in a published release. Release automation sets it and
    fills `Released in`.
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
- New trackers are proposed to tmt-lead. A new product topic needs the
  maintainer's approval; agents never create a tracker on their own.
- Use one batched daily audit plus event-driven updates. Batch Project edits,
  never poll, and treat about 200 GraphQL calls per lead per day as a ceiling.
  The GraphQL limit is shared by every agent on the maintainer's account.

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
uses its own Prettier configuration; shared docs use the tooling formatter.

## Remote pilot development

The local-build-only remote crate is a foreground deny-all door. It performs
one public startup capabilities read, then refuses every remote application
request. Pairing, signing, grants, approval, sends and journal/SDK integration
are not implemented. The [client contract](contracts/remote-client-v1.md) is
proposed; [the separately owned browser shell](#browser-add-on-shell)
uses only a stub. No official remote installer/release exists.

```bash
(cd rust && cargo build --offline --locked -p tmt-remote)
(cd rust && cargo test --offline --locked -p tmt-remote)
(cd rust && cargo clippy --offline --locked -p tmt-remote --all-targets -- -D warnings)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
node typescript/scripts/release-please-config.mjs --check
(cd typescript && corepack pnpm exec vitest run test/tooling/release-please-config.test.ts test/tooling/ci-scope.test.ts)
```

Pure byte/crypto conformance runs with the remote Rust tests above, including
shared independent canonical vectors, strict Ed25519 refusals, full HMAC tags and
receipt domain separation. Regenerate/check only Rust-owned crypto fixtures with:

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
`TMT_EXECUTABLE`; it never searches for another core. Default hard window is
one hour (maximum 24 hours); denied traffic cannot reset the 15-minute idle
deadline, so this interim door closes after at most 15 minutes. Ctrl-C/SIGTERM
stops it; there is no autostart/LAN/daemon option. Tests use disposable HOME/XDG,
count startup separately, assert zero request-triggered core calls and run
socket/process lifecycle acceptance twice. No real model/account/DB is used.

## Colab pilot development

The private local-build Colab executable runs a foreground loopback placeholder
and lists local-space metadata. APIs and WebSocket upgrades are denied until
the authentication/sync slice. No installer exists.
Build and verify it from the repository root:

```bash
(cd rust && cargo build --offline --locked -p tmt-colab)
(cd rust && cargo test --offline --locked -p tmt-colab)
(cd rust && cargo clippy --offline --locked -p tmt-colab --all-targets -- -D warnings)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
node typescript/scripts/release-please-config.mjs --check
(cd typescript && corepack pnpm exec vitest run test/tooling/ci-scope.test.ts)
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
Signatures and role admission are required at the future request boundary;
these storage tests prove transaction rollback and reopening, not crash recovery.

```bash
(cd rust && cargo build --offline --locked -p tmt-cli -p tmt-colab)
PATH="$PWD/rust/target/debug:$PATH" tmt colab spaces --json
PATH="$PWD/rust/target/debug:$PATH" tmt colab serve --json
```

After building a core supporting `storage.root` (#860) and the extension, put
`rust/target/debug` on PATH and run `tmt colab serve` (default port 7341).
A busy port fails with a `--port` hint; `--port 0` selects a free port.
Direct invocation requires an absolute
`TMT_EXECUTABLE`. `tmt colab serve --json` prints one plain JSON descriptor with
space ID and working URL; Ctrl-C/SIGTERM closes sockets, joins workers and
releases the service lock. `tmt colab spaces --json` lists the local space and
running state without creating directories or keys; before first serve it
returns `{"spaces":[]}`. Use an isolated normal TMT data root for manual tests.

The printed `127.0.0.1:<port>` is the only accepted Host and Origin; no localhost,
forwarded-host or DNS-rebinding alias is admitted. The door bounds are named in
`src/limits.rs`: 16 active sockets, 8 KiB/32 header fields, 64 KiB HTTP bodies,
2-second total acquisition and 1-second total response. HTTP body capacity is
for later sign-in/management; page objects use the future sync path. Reserved
sync limits are 64 KiB frames and 8 queued frames with `RESYNC_REQUIRED` close
for slow subscribers; no WebSocket is accepted yet. Real socket and foreground
process cleanup tests run lifecycle scenarios twice, with no core calls from
denied traffic. Owner-key temporary cleanup is publication-locked; it preserves
foreign file names and refuses unsafe matching files.

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
