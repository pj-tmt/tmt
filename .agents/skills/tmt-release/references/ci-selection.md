# CI selection and workers

The [component-map contract](../../../../ARCHITECTURE.md#ci-selection-and-worker-model)
keeps path ownership, CI selection and release attribution distinct. Read
`typescript/scripts/ci-scope.mjs`, `.github/components.json` and the affected
workflow before changing selection or a gate. Required checks remain `Code quality`,
`Unit tests`, `Docker E2E` and `Native package matrix`; selected missing, failed,
cancelled or unexpectedly skipped jobs fail closed. Never accept empty test discovery.
The publication commit gate accepts scope-skipped `Unit tests` only with the latest
successful `Native package matrix` from the same GitHub Actions suite, completed
no earlier than that skip; the aggregate validates its selection. The other required
contexts must succeed. Held-draft recovery belongs to
[main cuts](main-cuts.md#publication-gates-and-recovery).

## Diff and scope selection

- PRs use the merge-base diff. Merge groups use the common ancestor of fetched
  `origin/main` and `merge_group.head_sha` through the queue head, covering every
  pending change. The event's `base_sha` can be an earlier queued commit and is
  insufficient. Fetch full history; missing objects cannot produce a passing
  empty selection. Include deletions and both sides of renames.
- Ordered component-map rules select native work. Shared, unknown and unreadable
  inputs conservatively retain full native verification. Prose with no build or
  runtime consumers selects only Code quality. The selector records owner, rule, selection and map digest
  for each path in the run summary.
- If every native-selecting path is Ops-owned, scope is `ops`: retain its
  Cargo checks, architecture guard, native tests and E2E file under the existing
  job names, and skip unchanged CLI runtime builds, packed installs and tooling
  unit tests. Any shared, CLI-owned or unknown native input requires full scope.
- Workspace Rust checks exclude only `tmt-office`, `tmt-office-storage`,
  `tmt-office-pairing` and `tmt-office-service`. The shipped CLI imports
  `tmt-office-command` and `tmt-office-model`, so their tests, Clippy and MSRV
  checks remain selected alongside CLI contracts, architecture and embedded inputs.
  Reject empty Remote test discovery before execution. Office product verification
  is retired for every event, independently of its parked release attribution.

The Remote Firestore Rules emulator suite runs as selected steps inside the `Unit tests` job (no new
required context, no Docker): `remote_firestore` is true only for the suite, its fixtures, `rules.rs` or
`ci.yml`, all full native scope, so the job always runs when it is selected. The steps use the hosted Temurin
21 (`JAVA_HOME_21_X64`) and the pinned firebase-tools, with a step timeout and no retry or cache.

## Worker gates

`Native Rust contracts` aggregates fmt/Clippy, workspace test/build, native process
and MSRV workers. MSRV reads `rust/Cargo.toml` and checks all retained workspace
package targets. Full scope requires the retained workers and exactly a skipped
Office worker (`native_office=false`). Required-check names stay stable; the
aggregate reports success only with those results. Ops scope requires its
selected workers, and `none` skips the aggregate. Invalid scope or worker results
fail closed.

Office SPA, local-service and companion producers never run. Native process
verification excludes Office-owned suites and requires no Office companion;
core discovery stays nonempty and other fixtures are built independently.

For current small packages, apt acquisition allows three attempts per phase (60 s for update, 120 s for install), with 10 s termination grace and 5/10 s backoff; native-process contracts have a 20 min job budget without changing test deadlines. After the first install timeout, the shared action removes only Azure from the verified runner-image mirror list for remaining attempts; unexpected source/list layouts are logged and left unchanged.

`Docker E2E` gates the two shard jobs selected by
`typescript/scripts/e2e-shards.mjs` and committed `test/e2e/shard-weights.json`.
Full native scope requires both shards (the first also runs adapter tests), scoped
component work requires the first, and no native selection requires neither.
The shard guard rejects any retained scenario in zero or two shards. The exact
retired Office file list is owned by `e2e-shards.mjs` and also drives real Vitest
discovery; Docker no longer builds or copies the Office companion.

`Code quality` gates selected Office verification. `Native package matrix` gates
every selected native result and validates the event's `macos` classification:
false only for merge groups, where both macOS build/smoke results must be exactly
`skipped`; full-scope PRs require success. Linux rows remain required. Release
verification still includes macOS. Follow the
[runtime smoke matrix](../../tmt-e2e/references/runtime-smoke-matrix.md) for the
Rosetta process wrapper, exact installed-byte architecture admission and advisory
native Intel coverage.

The private browser presentation leaf is verified inside `Code quality`: its
frozen-lock tool install ignores lifecycle scripts, and nonempty filtered check/test
commands cover generated CSS equality, type/lint/format and package tests. The tooling
import guard keeps production inside the leaf and the static entry free of React.
Its component rule retains full native verification; this is not product adoption
or broader advisory browser coverage. E2E/artifact stages prepare checked CSS, and
all three native stages retain the actual Rust embedded-input guard. Office receives
only the manifest required by the existing filtered root install until a real
native reader needs CSS; Office product execution remains disabled.

## Cache ownership

Docker E2E PR/merge-group shards restore only exact main-owned dependency archives; the purpose-specific main seed exports Cargo target/debug and registry, while misses build cold and real compiler failures never retry cold.

Rust dependency caches use the pinned `Swatinem/rust-cache` action with one
main-only writer per key: workspace tests for shared dev dependencies, process
contracts for their existing dev/release builds under `native-process-rust`, MSRV
for its toolchain, and each runtime target for its own cache. These CI writers use the
shared seed-event classification (`verify=false`) and main ref. PRs, merge groups
and other workers only restore; the process cache is cold until its first main save.
Dev debug information and incremental compilation
are disabled in CI; release profiles keep their manifest policy.
Release build and adapter cache ownership is described in [native release verification](native-release.md#cli-upgrade-proof).

CI cache consumers and `colab-browser.yml` use `scripts/install-ci-rust.sh` to
retain only the requested host toolchain before restore, preserving components,
targets and manifest-driven MSRV. The helper removes runner-image extras only in
disposable Actions jobs. `native-intel.yml` keeps its Intel-host
`runtime-${TARGET}` setup; `native-release-upgrade-prove.yml` keeps its distinct
`RUSTUP_TOOLCHAIN` and source-scoped `CARGO_TARGET_DIR` environment.
`remote-pairing.yml`, `project-release.yml`, `native-release-bundle.yml`, metadata
gates in `native-release-prepare.yml` and `release.yml` retain their environment
families without CI's dev-debug setting. Release builds in
`native-release-prepare.yml` and `release-version-injection.yml` keep separate
product/target keys. Cache keys retain compiler, platform, environment and
manifest identities; an exact hit does not certify complete feature/profile
population.

Main seeding runs on selected Cargo/workflow changes, weekly and manually. It has
no diff and selects full native scope; the Rust aggregate validates its workers
while outer verification gates remain skipped. Feature-branch dispatches only
restore.

## Advisory browser selection

Office browser verification is disabled, with no automatic trigger and manual
selection that admits no product job. The workflow and historical failure artifacts remain available;
retirement is not a repaired-product claim. Office browser/SPA/local-service/native
selection stays false for PRs, merge groups, main pushes, schedules and dispatches.
The product remains parked for publication. Actual shipped CLI inputs and required
gate validation remain active.

Colab's separate advisory workflow selects `colab_harness` for its client, model
and contract-vector paths, its workflow, shared Cargo manifest/lockfile and pnpm
lockfile. Empty/unknown PR diffs select nothing. PRs run Chromium; weekly/manual
runs cover three engines and other shared drift, with no main-push trigger.
Rust cache restores never save. Browser cache keys include OS, architecture and
pinned Playwright version; only successful main-ref runs save. Reports are
primitive-library evidence, not product or publication acceptance.

The same workflow's `colab-app` job runs the app's Playwright component specs
(`@tmt/colab-app test:browser`: Chromium, one worker, no retries, no Rust). It
selects `colab_app` for the app, colab-client and browser-ui paths, the workflow and
the pnpm lockfile, and also runs weekly and manually. The specs that need the native
Colab executables skip with a named reason there, so the run proves the browser
components only. It keeps a bounded run log and failure traces for seven days and is
advisory like the rest of the workflow.

The same workflow runs advisory real-binary Colab acceptance weekly, manually,
or on a PR carrying `colab-acceptance`. Only adding that exact label can start
acceptance from a label event; removing a label never starts it. Label edits skip
the existing selector and interop jobs. Job-level concurrency keeps those edits
from cancelling in-flight checks; acceptance never cancels an active run.
The acceptance job checks the default PR merge ref and records both the PR head
and tested merge SHA in `tested-head.txt`. It builds the app before one build of
all three executables, and runs Chromium with one worker on Ubuntu x64 within
30 minutes. It restores Rust dependencies without saving and retains only source,
build hashes and test outcomes with bounded assertion locations/timeouts for seven
days, never raw error values, private traces or profiles. Unexpected outcomes also
print the bounded summary from the private JSON report for triage.
It remains outside required aggregates and does not authorize publication.

## Verification

### Release gate parity

`.github/release-parity.json` records current release verification coverage, owned by
infra. Its whole-job entries inventory every executable step in `native-release.yml`,
`release.yml` and their local reusable workflows. Publication-policy and incident
entries name narrower checks; `coverage: policy` means regression tests, not live
publication or actual-archive proof. `releaseOnly` gives the reason a stage has no
equivalent pre-merge execution: live draft, publication or release state that no pull
request can reach. A `releaseOnly` entry cannot carry a `followUp`; a gap is closed with a
counterpart or a concrete reason, never marked open.

A `preMerge` counterpart names a job in `ci.yml` selected by a ci-scope output; a selector
with its own module (the release rehearsal's `release-rehearsal.mjs`) names it as `selection.source`,
and the guard requires that file to emit the output. A job that runs on every verification
event uses `selection.kind: "always"` with the `step` that proves the incident (for example
`code-quality`'s PR title check); the guard rejects an `if` that depends on a path selection and a
step the job does not have. The rehearsed prepare stages (`build`,
`assemble`, `verify`, the upgrade proof stages, `gates-dry` and the bundle's `prepare` call) and the
packaging incidents map to `release-rehearsal`. Incident rows (#1534, #1541, #1542, #1550, #1593,
#1604, #1616, #1643, #1646, #1661, #1680) name the release step that caught the failure and a counterpart or a concrete
release-only reason; adding an incident also extends the guard's incident list and tests.

For a release workflow change, review its coverage and update the recorded step
inventory and SHA-256 of the admitted job definition. `node
typescript/scripts/release-parity.mjs inventory` prints the current definitions;
it does not update the manifest. A changed command, condition, target matrix or
called workflow needs the same review even if the step name is unchanged.
The guard rejects unmapped/stale workflows, jobs, steps and publication policies,
invalid counterpart jobs/selectors and unsupported workflow structure. It follows
local reusable workflow calls; external reusable release workflows and alias/flow
job or step mappings require extending admission with tests before use. Manifest
fingerprints require review, rather than establishing semantic equivalence by hash.

Run `node typescript/scripts/release-parity.mjs check` and, from `typescript/`,
`corepack pnpm exec vp test run --config vitest.config.ts
test/tooling/release-parity.test.ts test/tooling/release-workflow.test.ts`.
`Code quality` runs the cheap parity guard for PRs and merge groups, including
prose-only changes. The manifest records existing coverage only.

From `typescript/`, run the selection and workflow guards for a selection change:

```sh
corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts test/tooling/release-workflow.test.ts test/tooling/e2e-shards.test.ts
corepack pnpm check:tooling
```

Also run `actionlint` on every changed workflow and the selected layer's checks.
Review positive and negative path selections, cumulative queue diffs, shard
membership and exact aggregate results. A selection test never substitutes for
the runtime, archive or browser proof it selects.
