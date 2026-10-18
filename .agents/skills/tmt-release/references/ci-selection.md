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

For current small packages, apt acquisition allows three attempts per phase (60 s for update, 120 s for install), with 10 s termination grace and 5/10 s backoff; native-process contracts have a 20 min job budget without changing test deadlines. After the first install timeout, the shared action removes only Azure from the verified runner-image mirror list for remaining attempts; unexpected source/list layouts are logged and left unchanged.

`Docker E2E` gates the two shard jobs selected by
`typescript/scripts/e2e-shards.mjs` and committed `test/e2e/shard-weights.json`.
Full native scope requires both shards (the first also runs adapter tests), scoped
component work requires the first, and no native selection requires neither.
The shard guard rejects any retained scenario in zero or two shards. Actual Vitest
and `e2e-shards.mjs` discovery cover the same retained files.
The Docker jobs use pinned Node and the built-in-only wrapper on the host;
pnpm activation and the frozen dependency install belong to the image, not the host.
Only the hosted `ubuntu-latest` E2E mirror builder uses `cleanup: false`, delegating its build-resource lifetime to job-VM disposal; the fallback and any persistent runner retain default cleanup.

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
advisory like the rest of the workflow. Component shards use whole-file duration weights in
`e2e/shard-weights.json`, through the shared `typescript/scripts/colab-acceptance-shards.mjs`
helper. The helper and workflow matrix share a guarded shard count; every spec runs exactly
once, including new files with the default weight. The 20-minute job limit, one worker,
no retries and 30-second test limit remain. Dot and JSON reporters share the existing results
artifact (`results.json` holds per-test durations) for refitting weights; record source runs,
method and variance, and verify full test-ID equality before changing the split.

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

## Workspace compilation and partitions

Full native verification calls `workspace-partitions.yml` under the existing
workspace worker ID. Its producer retains independent Cargo/libtest discovery,
separate doctests, the dev build and selected native-sync proofs; Ops scope keeps
its original Cargo/architecture path. `partitions` is the K parameter; required CI
uses 2. Each consumer runs on its own hosted Ubuntu VM, admits the producer's
source/run/toolchain and archive digest separately from its execution attempt,
proves its exact slice before execution, then reports actual PASS identities. The always-run collector rejects
missing, skipped, failed, overlapping or foreign results before the caller can
satisfy the native aggregate. It never replaces identity proof with counts.

The pinned nextest installer is shared by producer and consumers. Consumers also
install the same pinned Rust toolchain for the native tests that run Cargo metadata.
The isolated nextest run receives the real Cargo executable in `CARGO`; producer
and consumer identities record actual Cargo/rustc versions. Consumers only extract
and run the archive; they restore no Cargo cache and issue no build.
Producer-only cache writes retain main admission. Phase evidence records compile,
listing, archive, extraction and execution; Actions step times retain setup,
transfer, selected emulators and collection costs. Separate VM ownership avoids a
shared-host cleanup domain; any local heavy proof still needs Infra's run grant.
Failed commands drain captured output into the job log before throwing, so failing
test names, details and summary remain visible there as well as in the full phase artifact.
The producer receipt is available only after the entire job and both artifact
uploads succeed. It binds the original attempt, exact manifest bytes, archive
hash/size and immutable inventory/closure artifact IDs and digests. Manifest and
result schema 2 distinguish that producer from each actual execution attempt;
result artifact names carry both. Cargo-built runtime examples remain explicit
archive inputs; consumers verify restored executable paths before tests.

The collector selects the latest actual successful execution for each partition
within at most three attempts of the same producer generation. A newer failed,
cancelled or incomplete execution refuses collection; it never falls back to an
older success. GitHub can copy retained successful jobs under new IDs, so attempt
start times plus an exact earlier-success fingerprint distinguish those copies
from new executions. Unavailable/ambiguous metadata, changed source/toolchain,
expired/missing/duplicate artifacts, digest mismatch or different producer refuses
collection. The recovery metadata reader allows GET only, at most 20 requests in
90 seconds; the three-attempt K=3 case needs at most 14. A single attempt makes
no recovery REST calls and adds no job or step. Only the collector receives an
`actions: read` token; its reusable caller sets the corresponding ceiling.

Workspace downloads use `actions/download-artifact@v8` with
`digest-mismatch: error` (v4 only warns). Uploads stay on v4. Selection uses original
artifact IDs and retains `selection.json` with the exact job/artifact identities
and result hashes alongside the unchanged aggregate counts. The collector still
proves every non-ignored case ran exactly once; reuse never replaces test proof.

Failed-jobs recovery is qualified for the unchanged-source K=2 lane within these
bounds ([run 39128389438, attempt 2](https://github.com/pj-tmt/tmt/actions/runs/39128389438/attempts/2)).
It retained the attempt-1 producer and partition-1 result, executed partition 2
on attempt 2, and proved the exact case union. Fresh CI job-wall time totaled
4.90 runner-minutes versus 71.47 for the initial full-scope attempt, excluding
queue time and retained copies. A production planner replay reproduced the
selection with 8 metadata GETs; the hosted collector emits no request counter.
The [qualification record](https://github.com/pj-tmt/tmt/issues/3571#issuecomment-6250385665)
includes transfer and aggregate timings. These measurements do not establish a
PR median improvement. Missing, expired or ambiguous evidence, or a changed
producer generation, requires re-run all jobs. This qualification grants no
publication or workflow-dispatch authority.
PR median and child p90 budgets use the five-run K2 series; the PR gate p90 stays
at 1095 s, below the observed 1190 s macOS-queue outlier (n=5, p90=max).
The four new job rows inherit the old 861 s MG limit; other MG limits stay unchanged
until the weekly report has ten natural MG runs. Infra reverts if those runs exceed
the pre-rollout 886 s MG median wall or 13.9 workspace runner-minutes, or show an
unexplained equivalence failure.

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

## Prepared Infra tick summary

`typescript/scripts/infra-tick-summary.mjs` prepares read-only evidence for the
hourly Infra lead tick. Run it manually beside the existing tick before changing
its cron prompt; the lead compares the two reports after the script lands.
The script does not judge reviews, attribute causes, arm PRs or publish releases.
A new output directory first records a bootstrap boundary with **zero API calls**:
"baseline established; details from the next tick." This is not a health report.
Run the same command again to collect evidence after that boundary:

```sh
node typescript/scripts/infra-tick-summary.mjs --output /tmp/tmt-infra-tick --review-map /tmp/infra-review-map.json
node typescript/scripts/infra-tick-summary.mjs --output /tmp/tmt-infra-tick --review-map /tmp/infra-review-map.json
```

`--bootstrap` explicitly establishes that boundary in an output directory whose
historical collection refused before creating a cursor. It cannot replace an
existing cursor or combine with `--since`. Earlier failure details and review
history are intentionally omitted; old runs that change after the boundary are
still eligible, and observed active runs remain tracked through completion.
An older active run that completes before the first observation is outside
coverage. Historical recurrence starts from observed post-boundary evidence.

The caller supplies a reviewed path-to-lead map with a #606 source line, for
example `{"rules":[{"prefix":".github/","leads":["tmt-lead","tmt-infra-lead"],"source":"#606 shared paths, lines 17-19"}]}`.
This is an example, not a complete ownership map: include every applicable
shared path and affected lead. Unmapped shared paths and missing exact-head lead
statements are anomalies. The JSON keeps each exact comment, head and stated lead;
presence of a statement never means scripted acceptance. Guide-index changes
are conservatively flagged at file level for the lead to inspect.

Remote calls are GET-only REST, with **one fixed read-only GraphQL queue query**
as the sole exception. All calls, catalogue reads, logs and the query share
80 API calls / 120 seconds; the existing consistent catalogue reader keeps its
stricter inner budget. Queue query failure, errors or more than 100 entries print
`queue coverage unavailable`; positions are never inferred from timestamps.
Draft visibility requires repository push permission; missing evidence, changing
counts/identities or exhaustion refuses collection rather than printing healthy
partial results. Failed workers exclude propagated aggregate-only gates. Named
test recurrence uses observed log tokens and distinct PR/merge-group contexts;
missing logs remain unavailable evidence, not a cause or a successful test.
Raw job logs use the packed command owner's existing 16 MiB capture ceiling;
other reads keep its default. Oversize logs remain explicitly unavailable.
Public-install failures join the workflow's exact tag-bearing job label to a
unique published catalogue record; missing, draft or ambiguous evidence stays
unavailable. Multiple failed hosts count as one affected release.

Run health uses `per_page=1` count reads for the 24-hour and tick windows,
including totals above GitHub's 1,000-result search cap. Details enumerate only
failure, timed-out, action-required and stale status slices in the created
window; every enumerated slice refuses explicitly at 1,000 matches, before
walking beyond GitHub's searchable evidence. Queued and in-progress discovery
has no created filter, so older active reruns are tracked; other active states
are included inside the window. An unobserved older rerun that starts and
finishes between ticks remains outside coverage, which every report states.
Workflow `startup_failure` conclusions are not searchable by status and are outside
detail coverage; the report states this gap. Only failed attempts absent from the previous cursor incur job/log analysis.
Previously observed test evidence stays in the cursor for 24-hour recurrence.
Review evidence covers **open PRs updated since `since`, plus PRs merged in that
interval**; both updated-desc walks stop on the first older row. The listed head
and the final exact PR read bind complete file counts and comment statements.

A quiet invocation with one page per listing costs `17 + P` calls, where `P`
is the release catalogue's page count: two health counts, nine status searches,
one queue query, repository visibility, `P+2` catalogue reads and two PR walks.
Add tracked-run reads, new failed-attempt job pages/logs, extra list pages, and
two reads per recent PR (files and final identity/count) plus comment pages for
shared paths. Bootstrap avoids all historical detail reads, independent of the
number of old failures and reviews. An explicit `--since` without a cursor opts
into historical collection within 24 hours; qualified terminal job/log evidence
can be reused after refusal, but **convergence is not guaranteed**: mutable status
and PR reads repeat and can alone exhaust 80 calls. Use bootstrap to start a new
observer, rather than repeatedly retrying a busy historical baseline.

`calls.jsonl` appends each invocation's network endpoint, method, outcome and
elapsed milliseconds (including the fixed queue query); cache hits do not appear
as new network calls. It records no credentials, response bodies or raw logs.
Each invocation stays inside 80 calls / 120 seconds; inspect repeated endpoints
when diagnosing exhaustion. A transient "Truncated REST evidence" refusal can
occur when a listing changes; retry the same output directory with the cursor
unchanged. No incomplete or bootstrap report claims healthy coverage.

`summary.md` lists anomalies first (up to 20, with an explicit pointer to the full
JSON), coverage and host disk/load. `summary.json` keeps the complete evidence and
local cache paths. `cursor.json` advances atomically only after a complete report;
refusal or unavailable coverage returns nonzero and leaves the cursor intact.
An empty anomaly list is stated explicitly only after complete collection. Known
GitHub credential values are redacted from output. No secrets or raw logs belong
in a posted summary. Shared guide line/byte limits have one owner,
`typescript/scripts/guide-budgets.json`, used by both the summary and guide guard;
this extraction changes no limit. The prepared report is input for the lead's
judgement, never a replacement for it.
