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

Code quality checks local relative-link targets in tracked Markdown outside `site/`,
excluding fences, inline code and HTML comments; it does not fetch URLs or validate anchors.

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
- Colab browser source and test/spec trees select Code quality and their existing
  browser lanes without the full native gate. Native chrome CSS, hosted-build
  inputs, entry/build/config files, public assets, vectors, native skill text and
  unknown inputs retain full verification. The renderer-style generator is checked
  by the unconditional app quality/build commands; browser selectors are independent.
- The exact Colab `contracts/colab-v1.md` prose path selects Code quality, whose
  app unit tests check shipped-skill error codes against it; native jobs do not
  read it. Contract vectors and unknown contract paths retain full native scope.
- Workspace Rust checks cover every retained member; no retired product exclusions
  or producers remain. Reject empty Remote test discovery before execution.

The Remote and Colab Firestore Rules emulator suites run as selected steps inside `Unit tests` (no new
required context). `remote_firestore` selects Remote's suite, fixtures, `rules.rs` or `ci.yml`;
`colab_firestore` selects Colab's `firestore/`, `deploy-declaration-*` vectors, emulator suite or `ci.yml`.
All selected inputs require full native scope so the job runs. Remote uses hosted Temurin 21
(`JAVA_HOME_21_X64`), Node 22.23.2 and firebase-tools 15.29.0. Colab builds its digest-pinned emulator
Dockerfile with the same pins, then runs `--rm --init --network none` with only its extension mounted
read-only and no credentials. Both suites fail closed within their step bounds, without retry or cache.

## Worker gates

`Native Rust contracts` aggregates fmt/Clippy, workspace test/build, native process
and MSRV workers. MSRV reads `rust/Cargo.toml` and checks every retained workspace
package target. Full and Ops scopes require every selected worker. Required-check
names stay stable; `none` skips the aggregate. Invalid scope, missing/extra worker
results and failed or unexpectedly skipped work fail closed.
Native process verification discovers every retained suite without a product blacklist.
The #3261 draft tests one nextest pool on the workspace runner, with independent Cargo/libtest inventory equality, separate doctests and the existing private-world wrapper. This is a CI-only experiment; local guidance remains `cargo test`, and rollout needs its own decision.

For current small packages, apt acquisition allows three attempts per phase (60 s for update, 120 s for install), with 10 s termination grace and 5/10 s backoff; native-process contracts have a 20 min job budget without changing test deadlines. After the first install timeout, the shared action removes only Azure from the verified runner-image mirror list for remaining attempts; unexpected source/list layouts are logged and left unchanged.

`Docker E2E` gates the two shard jobs selected by
`typescript/scripts/e2e-shards.mjs` and committed `test/e2e/shard-weights.json`.
Full native scope requires both shards (the first also runs adapter tests), scoped
component work requires the first, and no native selection requires neither.
The shard guard rejects any retained scenario in zero or two shards. Actual Vitest
and `e2e-shards.mjs` discovery cover the same retained files.

`Native package matrix` gates
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
all three native stages retain the actual Rust embedded-input guard.

## Trusted main verification

Every main push selects complete CI verification after a bounded read confirms
its event SHA is still current main. One admitted run finishes without cancellation;
one pending run is replaced by newer pushes. Superseded events skip verification
and seed work. All workers keep the admitted SHA, including both Rules suites,
native sync, notices, both Docker shards and all runtime/packed targets. PR/MG
selection and the four required names stay unchanged. PR title/Agent metadata
steps enforce only PR/MG events; main still runs the actual code and frontend checks.

`Complete trusted-main verification` requires the exact full worker inventory and
flags; missing, extra, failed, cancelled or skipped obligations refuse. It grants
no publication authorization and does not change release cut admission. A main-only
failure job reports bounded run/SHA/job evidence on #3054 with the workflow token;
only that job has issue-write permission. Repeated notification reads the bot's
run/attempt marker before one POST; uncertain writes never retry. Capacity or API
failures remain visible. Seeds cannot supply complete-verification evidence.

## Cache ownership

Docker E2E PR/merge-group shards restore only exact main-owned dependency archives; the purpose-specific main seed exports Cargo target/debug and registry, while misses build cold and real compiler failures never retry cold.

Rust dependency caches use the pinned `Swatinem/rust-cache` action with one
main-only writer per key: workspace tests for shared dev dependencies, process
contracts for their existing dev/release builds under `native-process-rust`, MSRV
for its toolchain, and each runtime target for its own cache. These CI writers use the
seed classification (`verify=false`) or admitted main-full event, and the main ref. PRs, merge groups
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

Weekly/manual seeding has no diff and selects full native scope; the Rust aggregate
validates its workers while outer verification gates remain skipped. Main-full
verification retains the same cache owners and keys. Feature-branch dispatches
only restore.

## Advisory browser selection

Colab's separate advisory workflow selects `colab_harness` for its client, model
and contract-vector paths, its workflow, shared Cargo manifest/lockfile and pnpm
lockfile. Empty/unknown PR diffs select nothing. PRs run Chromium; weekly/manual
runs keep Chromium/Firefox strict and prove Linux WebKitGTK refusal, while a
separate macOS job runs strict WebKit corpus and interop; macOS never runs on PRs
or merge groups. There is no main-push trigger.
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

The advisory `colab-hosted` PR job runs the hosted bundle spec when `colab_hosted` or
`colab_hosted_emulator` selects it. `selectHostedReaderEmulator` in `ci-scope.mjs` owns the
latter: hosted-reader inputs (including Objects, Catchup and reader keys), Colab's signed-proof
client source, native producer/composer inputs and Remote's production Rust, assets and SDK
source; client tests, Remote test/docs trees and the test-only Colab source pin stay out.
Its selected qualifier uses `.github/actions/hosted-reader-emulator`: runner Java 21,
`firebase-tools@15.29.0`, a populated Firestore emulator cache and the unchanged owner wrapper,
with private state, loopback demo Auth/Firestore and Chromium. It runs after the ordinary spec
because it rebuilds Remote with `test-emulator`. Full label/weekly/manual acceptance runs the
same action once, on shard 1 after ordinary acceptance, whatever its result, unless the run is
cancelled; build-hash recording stays before it. It stays advisory,
never runs on main pushes or merge groups, and adds no permission or required aggregate.
On failure the action retains proof test-results and the two demo-emulator logs for three days; the wrapper copies only those logs before removing its private world.

The same workflow runs advisory real-binary Colab acceptance weekly, manually,
or on a PR carrying `colab-acceptance`. Only adding that exact label can start
acceptance from a label event; removing a label never starts it. Label edits skip
the existing selector and interop jobs. Job-level concurrency keeps those edits
from cancelling in-flight checks; acceptance never cancels an active run.
The acceptance job is a matrix of two shards (`fail-fast: false`, one concurrency
group and one artifact per shard). Each shard runs whole spec files, balanced by
measured time (`typescript/scripts/colab-acceptance-shards.mjs` with the weights in
`acceptance/shard-weights.json`; every file lands in exactly one shard). Each shard
checks the default PR merge ref and records both the PR head
and tested merge SHA in `tested-head.txt`. It builds the app before one build of
all three executables, and runs Chromium with one worker on Ubuntu x64 within
30 minutes. It restores Rust dependencies without saving and retains only source,
build hashes and test outcomes with bounded assertion locations/timeouts for seven
days, never raw error values, private traces or profiles. Unexpected outcomes also
print the bounded summary from the private JSON report for triage.
It remains outside required aggregates and does not authorize publication.

## Gate-time budgets

`node typescript/scripts/ci-gate-time.mjs` (read-only REST, no CI run, about 85 requests) measures the
last 40 successful `ci.yml` PR and merge-group runs, ordered by creation then run ID: gate wall (run
creation to the last selected required context), per-job wall and queue, the controlling job and
runner-minutes, with the median and nearest-rank p90. Skipped jobs and negative spans (reused job copies in a retried run) have no sample. The controlling job
is the last non-aggregate job to complete before the gate ends. It reads the budgets and lists
every value over its limit; it also lists budgeted jobs with no sample (a renamed job would otherwise stop being checked). It never blocks a PR (timing is too noisy for that).

[`typescript/scripts/ci-gate-budgets.json`](../../../../typescript/scripts/ci-gate-budgets.json) is the one owner
of the budgets: the gate median and p90 for PRs and merge groups, and a p90 per required job. They start at
#3230's frozen baseline (2026-10-15) and are tightened as #3047 slices land. A gate or job is over budget when
its measured median (gate only) or p90 exceeds the figure. Raising a budget needs a written note from the Infra
lead on #3283.

The Monday hygiene pass runs the script and compares it with the budgets. A job over budget for two
consecutive weeks gets an issue for the squad whose suite dominates it, asking for a shard, a consumer
or a trim. A PR expected to add more than about 20 s to a required job says so in its body and updates
the shard weights or assignment (`typescript/test/e2e/shard-weights.json`, or
`extensions/tmt-colab/typescript/app/acceptance/shard-weights.json`) in the same PR.

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
packaging incidents map to `release-rehearsal`. The manifest's `incidents` keys list the recorded
release failures; each row names the release step that caught it and a counterpart or a concrete
release-only reason. Adding an incident also extends the guard's incident list and tests.

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
