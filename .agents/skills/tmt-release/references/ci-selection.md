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
  pending change; the event's `base_sha` is insufficient. Fetch full history:
  missing objects cannot yield a passing empty selection. Include deletions and
  both sides of renames.
- Ordered component-map rules select native work. Shared, unknown or unreadable
  inputs retain full verification; prose without build/runtime consumers selects
  only Code quality. Selection covers every path, recording owner, rule, selection
  and map digest; summaries detail the first 100 paths and group the rest by rule/count.
- Scope is `ops` only when every native-selecting path is Ops-owned: retain its
  Cargo checks, architecture guard, native tests and E2E under existing names;
  skip unchanged CLI builds, packed installs and tooling unit tests. Shared,
  CLI-owned or unknown native inputs require full scope.
- Colab browser source/test/spec trees select Code quality, browser lanes and the gating
  `colab-chrome` job without the full native gate.
  Native chrome CSS, hosted-build inputs, entry/build/config, public assets,
  vectors, native skills and unknown inputs retain full verification. Unconditional
  app quality/build checks the renderer-style generator; browser selectors are independent.
- Colab `contracts/colab-v1.md` prose selects Code quality, whose app unit tests
  check shipped-skill error codes; native jobs do not read it. Contract vectors
  and unknown contract paths retain full scope.
- Workspace Rust checks retain every member, without retired product exclusions
  or producers. Reject empty Remote discovery before execution.

Remote and Colab Rules emulators are selected steps in `Unit tests`, requiring
full scope. `remote_firestore` covers Remote's suite, fixtures, `rules.rs` or
`ci.yml`; `colab_firestore` covers Colab's `firestore/`, `deploy-declaration-*`
vectors, emulator suite or `ci.yml`. The [workflow](../../../../.github/workflows/ci.yml)
owns Java/Node/firebase-tools pins. Remote uses hosted Temurin; Colab's digest-pinned
image uses the same pins, `--rm --init --network none`, a read-only extension mount
and no credentials. Both fail closed within step bounds, without retries or cache.

## Worker gates

`Native Rust contracts` aggregates fmt/Clippy, workspace test/build, native process
and MSRV. MSRV reads `rust/Cargo.toml` and checks every retained package target;
process verification discovers every retained suite without a product blacklist.
Full/Ops scopes require every selected worker; `none` skips the aggregate. Invalid
scope, missing/extra results, failure or unexpected skips fail closed; names stay stable.

The [apt action](../../../../.github/actions/apt-install/action.yml) allows three
attempts per phase (60 s update, 120 s install), 10 s termination grace and 5/10 s
backoff for small packages. After the first install timeout, only Azure is removed
from the verified runner-image mirror list for remaining attempts; unexpected
layouts are logged and unchanged.
Native-process contracts retain a 20-minute job limit and unchanged test deadlines.

`Docker E2E` gates shards from `typescript/scripts/e2e-shards.mjs` and committed
`test/e2e/shard-weights.json`: full scope requires both (shard 1 also runs the pinned
adapter environment slice), component scope requires shard 1, and no native
selection requires neither.
Vitest and the splitter discover identical retained files; the guard rejects zero
or double membership. Pinned Node runs the built-in-only wrapper on the host;
pnpm activation/frozen install belong to the image. Only the hosted `ubuntu-latest`
mirror builder uses `cleanup: false` and job-VM disposal; fallback/persistent
runners retain default cleanup.

`Native package matrix` requires every selected native result. Its event-derived
`macos` flag is false only for merge groups, where both macOS build/smoke results
must be exactly skipped; full-scope PRs require success. Linux stays required;
release verification includes macOS. The [runtime smoke matrix](../../tmt-e2e/references/runtime-smoke-matrix.md)
owns Rosetta wrapping, installed-byte architecture admission and advisory Intel coverage.

`Code quality` verifies the private browser leaf with frozen, lifecycle-script-free
installation and nonempty filtered CSS-equality/type/lint/format/package checks.
Its import guard confines production to the leaf and excludes React from the static
entry. The component rule keeps full native scope; this is not product adoption
or broader advisory browser coverage. E2E/artifacts prepare checked CSS; all three native
stages retain the actual Rust embedded-input guard.

`colab-chrome` runs `e2e/chrome.spec.ts` against the native fallback chrome. `colab_chrome`
selects app-suite inputs, `ci.yml`, or `chrome.rs`, `socket.rs`, `assets.rs` and
`tests/socket.rs`, in PR, merge-group and main-full runs. The job builds the `tmt-colab`
`socket` test fixture, sets `COLAB_CHROME_REQUIRED=1` (a missing fixture fails, never skips)
and applies the complete-listed-case guard (`colab-native-spec-report.mjs`) with one worker
and no retries. `Native package matrix` requires it: selected means success, unselected
means skipped; no required name changes. It restores caches without saving, except
Chromium on main, and installs Chromium's system packages through the bounded apt action
rather than Playwright's own unbounded `apt-get`.

## Trusted main verification

Each main push selects full CI only after a bounded read admits its SHA as current
main. An admitted run finishes without cancellation; newer pushes replace one
pending run, while superseded events skip verification/seeding. All workers use
the admitted SHA, including both Rules suites, native sync, notices, the Colab chrome spec, both Docker
shards and all runtime/packed targets. PR/MG selection/required names are unchanged;
main runs code/frontend checks while PR title/Agent steps apply only to PR/MG.

`Complete trusted-main verification` requires the exact worker inventory/flags;
missing, extra, failed, cancelled or skipped work refuses. It grants no publication
or cut admission. Only the main-failure job has issue-write permission: it reports
bounded run/SHA/job evidence on #3054, checking the bot run/attempt marker before
one POST. Uncertain writes never retry; capacity/API failures stay visible.
Seeds cannot supply complete-verification evidence.

## Cache ownership

Docker PR/MG shards restore exact main-owned dependency archives; the purpose-specific
main seed exports Cargo target/debug and registry. Misses build cold; real compiler
failures never retry cold. Pinned `Swatinem/rust-cache` has one main-ref writer per
key: workspace for shared dev dependencies, process for its dev/release builds
(`native-process-rust`), MSRV for its toolchain, each runtime target for its cache.
Only admitted main-full or `verify=false` seed events write; PRs, merge groups and
other workers restore only. The process cache is cold until its first main save.
CI disables dev debug info/incremental compilation; release profiles follow manifests.
[Native release verification](native-release.md#cli-upgrade-proof) owns release/adapter caches.

CI consumers and `colab-browser.yml` use [install-ci-rust.sh](../../../../scripts/install-ci-rust.sh)
to retain the requested host toolchain before restore, preserving components,
targets and manifest MSRV; runner-image extras are removed only on disposable Actions VMs.
`native-intel.yml` retains Intel `runtime-${TARGET}` setup; the upgrade proof keeps
its `RUSTUP_TOOLCHAIN` and source-scoped `CARGO_TARGET_DIR`. Pairing/project-release,
bundle/prepare metadata and `release.yml` keep their own environment families,
without CI dev-debug settings; release prepare/version injection use separate
product/target keys. [Workflow definitions](../../../../.github/workflows) own these
settings. Keys bind compiler/platform/environment/manifests; exact hits do not prove
complete feature/profile population.

Weekly/manual seeds have no diff and select full scope: the Rust aggregate validates
workers, outer verification gates skip. Main-full keeps the same owners/keys;
feature-branch dispatches restore only.

## Advisory browser selection

[Colab browser verification](../../../../.github/workflows/colab-browser.yml) is
advisory (the gating chrome spec lives in `ci.yml`), outside required aggregates, with no main-push or merge-group trigger.
No lane authorizes publication.
`colab_harness` selects client/model/contract vectors, its workflow, shared Cargo
manifest/lockfile and pnpm lockfile; empty/unknown PR diffs select nothing. PRs run
Chromium; weekly/manual runs require strict Chromium/Firefox and Linux WebKitGTK
refusal, plus separate strict macOS WebKit corpus/interop. macOS never runs on PR/MG.
Rust restores never save; browser keys bind OS/architecture/pinned Playwright, and only
successful main-ref runs save. Reports prove primitives, not product acceptance.

`colab-app` uses `colab_app` for app/colab-client/browser-ui paths, workflow and pnpm lockfile,
plus weekly/manual events. `@tmt/colab-app test:browser` runs Chromium with one
worker/no retries/no Rust; native-executable specs skip with a named reason, so
the run proves the browser components only. Bounded logs/failure traces last seven days. Whole-file weights in
`e2e/shard-weights.json` use `typescript/scripts/colab-acceptance-shards.mjs`;
helper/matrix share a guarded count, assigning each spec exactly once, including
new default-weight files. Keep the 20-minute job/30-second test limits, one worker
and no retries. Dot/JSON reporters share the results artifact (`results.json`
per-test durations); refits record source runs/method/variance and verify full
test-ID equality before changing the split.

`colab-hosted` PRs run the bundle spec for `colab_hosted` or `colab_hosted_emulator`. The
same job's final step runs `e2e/served.spec.ts` against the embedded binary and a private
expected-byte copy of `dist` (checkout `dist` is hidden and restored by an exit trap) when
`selectColabHosted` selects the exact serve/door/assets/reader-entry inputs. There
`COLAB_SERVE_REQUIRED=1` fails a missing binary, and the JSON report guard requires every
listed case to pass exactly once with zero skips; component-only runs keep the named
missing-native skip. Record the first three real timings on #3126; stop and report if the
served step exceeds 90 seconds or the job 15 minutes.
`selectHostedReaderEmulator` in `ci-scope.mjs` selects hosted-reader inputs (Objects,
Catchup/reader keys), signed-proof client, native producer/composer and Remote
production Rust/assets/SDK; it excludes client tests, Remote tests/docs and the
test-only Colab pin. The [shared action](../../../../.github/actions/hosted-reader-emulator/action.yml)
owns Java/firebase-tools pins and populated cache, using the unchanged owner wrapper,
private state, loopback demo Auth/Firestore and Chromium. The qualifier follows the
ordinary spec because it rebuilds Remote with `test-emulator`. Full label/weekly/manual
acceptance runs it once on shard 1 after ordinary acceptance, even on failure,
unless cancelled; build hashes are recorded first. It adds no permission/aggregate;
on failure, the action retains proof test-results and two demo-emulator logs for
three days. The wrapper copies only those logs before removing its private world.

Real-binary acceptance runs weekly/manually or on `colab-acceptance` PRs. Only adding
that exact label triggers acceptance; removing it never does. Label edits skip
selector/interop; job concurrency prevents cancelling in-flight checks. Two shards
use `fail-fast: false`, separate concurrency/artifacts and whole-file duration
weights (`colab-acceptance-shards.mjs`, `acceptance/shard-weights.json`), exactly once
per file. Each tests the default merge ref and records head/merge SHAs in
`tested-head.txt`, builds the app then all three executables once, and runs Ubuntu
x64 Chromium with one worker in 30 minutes. Rust restores never save. Seven-day
evidence retains only source/build hashes/outcomes with bounded assertion
locations/timeouts, never raw errors/private traces/profiles. Unexpected outcomes
print the bounded private-JSON summary; acceptance never cancels active runs.

## Workspace compilation and partitions

Full scope calls `workspace-partitions.yml` under the workspace worker ID; Ops
keeps Cargo/architecture verification. The producer retains independent Cargo/libtest
discovery, separate doctests, dev build and selected native sync. Required `partitions`
(K) is 2. Each Ubuntu consumer admits producer source/run/toolchain/archive digest
separately from execution attempt, proves its slice and records actual PASS identities.
The always-run collector rejects missing/skipped/failed/overlapping/foreign results
before native aggregation; counts never replace exact non-ignored case coverage.

Producer/consumers share pinned nextest and record actual Cargo/rustc versions.
Consumers install pinned Rust for tests using Cargo metadata, pass real Cargo in
`CARGO`, extract/run without cache restore or build. Only the admitted main producer
writes caches. Phase evidence covers compile/list/archive/extract/execute; Actions
steps retain setup/transfer/emulator/collection cost. Separate VMs own cleanup;
local heavy proofs require Infra's run grant. Failed commands drain captured output
before throwing, keeping names/details/summary in job logs and phase artifacts.

Only a fully successful producer job and both uploads yield its receipt, binding
original attempt, exact manifest bytes, archive hash/size and immutable inventory/
closure IDs/digests. Schema 2 separates producer from actual execution attempt;
result names contain both. Cargo-built runtime examples are archive inputs and
consumers verify restored executable paths before testing.

Recovery admits K=2 or 3; collection selects each partition's latest actual success
within three attempts of one producer generation. Newer failed/cancelled/incomplete executions refuse,
never falling back. Attempt start time plus exact earlier-success fingerprint
identifies copied retained jobs with new IDs. Ambiguous/unavailable metadata,
changed source/toolchain, expired/missing/duplicate artifacts, digest mismatch or
different producer refuses. Recovery reads GET only, <=20 requests/90 seconds;
single attempts make no recovery calls or added job/step. Only the collector has
`actions: read`, with the reusable caller's matching ceiling.

Downloads use `actions/download-artifact@v8`, `digest-mismatch: error`; uploads use
v4. Original artifact IDs and `selection.json` retain job/artifact identities/result
hashes with unchanged aggregate counts and exact-once proof. Failed-jobs recovery
is [qualified for unchanged-source K=2](https://github.com/pj-tmt/tmt/issues/3571#issuecomment-6250385665)
within these bounds; missing/expired/ambiguous evidence or changed producer requires
re-run all jobs. This grants no publication/workflow-dispatch authority.
The four workspace job p90 budgets use the weekly report's 31–35 samples for both
events; other MG limits stay unchanged.
Infra reverts if those runs exceed the pre-rollout 886 s MG median wall or 13.9
workspace runner-minutes, or show an unexplained equivalence failure.
Budget provenance stays in the JSON below.

## Gate-time budgets

`node typescript/scripts/ci-gate-time.mjs` uses read-only REST (~85 requests) for the
last 40 successful `ci.yml` PR/MG runs, ordered by creation then ID. It reports gate
wall (creation to last selected required context), per-job wall/queue, controlling
job and runner-minutes with median/nearest-rank p90. Skips and negative reused-copy
spans have no sample; the controlling worker is the last non-aggregate before gate
end. Over-budget values and budgeted jobs without samples stay visible; timing
never blocks PRs. [ci-gate-budgets.json](../../../../typescript/scripts/ci-gate-budgets.json)
owns figures/provenance: gate median/p90 and job p90. Exceeding a figure is over
budget; increases need Infra's written note on #3283.

Monday hygiene compares budgets: two consecutive over-budget weeks require an
issue to the dominating suite's squad for sharding/consumers/trimming. A PR expected
to add more than about 20 s to a required job must say so in its body and update
weights or assignment (`typescript/test/e2e/shard-weights.json` or Colab
`acceptance/shard-weights.json`) there.

## Verification

### Release gate parity

Infra's `.github/release-parity.json` inventories executable steps in `native-release.yml`,
`release.yml` and local reusables. Publication-policy/incident entries are narrower;
`coverage: policy` means regression tests, not publication or actual archives.
`releaseOnly` requires a concrete live-state reason no PR can reach and cannot
carry `followUp`; each gap needs a counterpart or that reason, never an open marker.

A `preMerge` counterpart names a `ci.yml` job selected by a ci-scope output. Separate
selectors (such as
`release-rehearsal.mjs`) name `selection.source`, which must emit the output.
Always-selected verification uses `selection.kind: "always"` and the actual incident
step; the guard rejects path-dependent conditions or absent steps. The manifest
maps rehearsed prepare/packaging stages to `release-rehearsal`; each incident names
the release failure step and counterpart/release-only reason. Extend guard incident
lists/tests when adding an incident.

For workflow changes review coverage and update executable-step inventory/admitted
job SHA-256. `node typescript/scripts/release-parity.mjs inventory` prints definitions
without updating the manifest. Changed commands/conditions/matrices/called workflows
need review even with unchanged names. The guard rejects stale/unmapped workflows,
jobs, steps, publication policies, invalid counterparts/selectors or unsupported
structure. Local reusables are followed; external reusables or alias/flow job/step
mappings need admission tests before use. Fingerprints require review and never
establish semantic equivalence.

Run `node typescript/scripts/release-parity.mjs check` and, from `typescript/`,
`corepack pnpm exec vp test run --config vitest.config.ts
test/tooling/release-parity.test.ts test/tooling/release-workflow.test.ts`.
Code quality runs parity for PR/MG, including prose-only changes; the manifest records existing coverage.
Selection/workflow changes also run:

```sh
corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts test/tooling/release-workflow.test.ts test/tooling/e2e-shards.test.ts
corepack pnpm check:tooling
```

Run `actionlint` on changed workflows and the selected layer's checks. Review positive/
negative selections, cumulative queue diffs, shard membership and exact aggregates;
selection never substitutes for runtime/archive/browser proof.

## Prepared Infra tick summary

`typescript/scripts/infra-tick-summary.mjs` prepares read-only evidence for the
hourly Infra lead tick. Run it manually beside the existing tick before changing
its cron prompt; the lead compares the two reports after the script lands.
The script does not judge reviews, attribute causes, arm PRs or publish releases.
A new output directory establishes a zero-call bootstrap boundary, not a health
report. Run the command again for evidence after that boundary:

```sh
node typescript/scripts/infra-tick-summary.mjs --output /tmp/tmt-infra-tick --review-map /tmp/infra-review-map.json
```

`--bootstrap` can establish a boundary after historical refusal only without an
existing cursor or `--since`. It omits earlier failure/review history; changed old
runs remain eligible, observed active runs stay tracked, and recurrence begins at
observed post-boundary evidence. Older active runs completed before first observation
are outside coverage. The caller supplies a reviewed path-to-lead map with every
applicable shared path/lead and its #606 source line, for example
`{"rules":[{"prefix":".github/","leads":["tmt-lead","tmt-infra-lead"],"source":"#606 shared paths, lines 17-19"}]}`.
This is an example, not a complete ownership map. Unmapped shared paths/missing
exact-head statements are anomalies. JSON retains
exact comment/head/stated lead; statements (including NO-GO) never imply acceptance.
Guide-index changes are conservatively flagged at file level for lead inspection.

All calls share 80 requests/120 seconds, including failures; the catalogue has its
stricter inner bound. REST is GET-only, with two fixed read-only GraphQL exceptions:
queue positions and one <=100-PR review batch. Queue errors/failure/>100 entries print
`queue coverage unavailable`, never timestamp-derived positions. Review cost is
logged in `calls.jsonl`; missing/invalid cost, errors or >20 points refuses. The
batch binds listed heads to file counts/paths and comment bodies/URLs, reading
the first 100 files and last 100 comments. Pagination uses full REST/final
exact-head/file-count verification for that PR only.
Truncated/foreign/duplicate evidence or changed heads refuse, never empty healthy sweeps.
Draft visibility needs push permission; changing counts/identities, missing required
metadata or exhaustion refuses. Discovery, queue, releases, PR reviews and complete
terminal-job metadata precede optional logs. Select the first failed non-aggregate
worker per attempt, newest runs first (ties by run ID, jobs by ID), at most three
logs per tick. Each read has a 20 s maximum within a 60 s phase, reduced to retain
10 s and two API calls. Caps include cache reads; hits are not network calls.
Reports begin `logs read: k of n failed workers` (including retained samples),
and name every unread worker/reason. Unread logs are not retried automatically or
asserted healthy. Recurrence uses observed tokens/distinct PR/MG contexts only.
Missing/oversize/timed-out logs remain unavailable, not causes or successes. Full
logs retain their SHA-256/cache and the packed owner's 16 MiB ceiling; other reads
keep its default. Public-install failures join exact tag-bearing job
labels to unique published records; missing/draft/ambiguous joins are unavailable,
and multiple failed hosts count as one release.

Health counts use `per_page=1` for 24-hour/tick windows, including >1,000 totals.
Details enumerate created-window failure/timed-out/action-required/stale slices,
refusing at 1,000 matches before exceeding searchable evidence. Queued/in-progress
searches track older reruns without a created filter; count/row mismatch gets up to two
same-page re-reads in the shared budget, then refuses with status/count/rows; created-window
queries stay strict and other active states stay within the window. Unobserved older reruns
starting/finishing between ticks and
workflow `startup_failure` are outside detail coverage, explicitly stated in reports.
Only unseen failed attempts incur job/log reads; observed tokens remain for 24-hour recurrence.
Reviews cover open PRs updated since `since` plus PRs merged in the interval.
Exhaust open PR pages; discover closed candidates through `issues?state=closed&since=`
with a one-second overlap before the rounded-down boundary, filtered locally to inclusive
`since`, then hydrate each PR for its head and merge time. Neither walk stops on an old row.
Two fresh passes must agree on page count and identity membership, allowing reordered
members; duplicates, additions, drops or shrinking pages refuse. This checks observed
membership, not a transactional snapshot. Reserve all hydration calls plus the review
batch before reading any candidate; insufficient budget refuses without a partial set.

Quiet one-page acquisition costs `19 + P + H` calls (`P` catalogue pages, `H` recently
updated closed PR candidates, including unmerged closures), plus tracked/new failure
details, extra list pages, one nonempty review query and per-PR REST fallbacks.
`H` grows with merges/closed-PR updates; split a long window with `--until`.
The timed fixture (P=7, H=0, one review batch, 0.5 s other calls, 1.8 s per
single-page failed attempt) supports 53 uncached failed attempts: 80 calls/108.9 s;
the 54th refuses. Extra pages, hydration, tracked runs or slow calls lower this
ceiling. Twenty attempts plus the full 60 s log phase cost at most 109.5 s in
that fixture; 10/30 workers across ten attempts cost at most 91.5 s. The shared
80/120 limits always apply; mandatory exhaustion refuses with the cursor unchanged.
Bootstrap has no historical detail reads. Explicit `--since` without a cursor opts
into <=24-hour historical collection; terminal job/log cache can survive refusal,
but unbounded/heavily paginated windows may not converge. Bootstrap starts a new
observer, not a repair of an oversized incremental window. Split backlog with
`--since UTC --until UTC`, without resetting the cursor, then collect the remainder.
There is no pending-window checkpoint/automatic retry. Use at most two manual
invocations per window (<=160 calls); after two refusals stop, split or escalate.
A transient `Truncated REST evidence` may allow one retry in the same directory;
`--until` fixes its end and the cursor stays unchanged.

`calls.jsonl` records network endpoint/method/outcome/elapsed time and review query
cost; cache hits are not calls. It stores no credentials, response bodies or raw logs.
Refusals name the acquisition stage and cohort PR count. `summary.md` ranks up to 20 anomaly lines:
failed drafts/publication failures and unavailable queue/public-install evidence, failed runs/repeated tests, guide budgets, stuck queue entries,
then PR statements (one count with three example PRs). `summary.json` retains every anomaly and
evidence/cache path; coverage and host disk/load follow the printed anomalies. `cursor.json`
advances atomically when all required metadata is complete under the stated log
sample, including omitted/unavailable diagnostics. Only failed attempted downloads
add log anomalies; omissions remain in worker records and the tail list.
Required metadata unavailability/refusal returns nonzero with unchanged cursor; the first line
says `INCOMPLETE: <kinds>; cursor not advanced`. Queue failures retain a credential-redacted
reason. Entries enqueued after the fixed report boundary have age zero.
Core's #2268 tick prompt must say: "Logs are sampled; log not read is unknown
diagnostic detail, not a healthy worker." The prompt itself is not changed here.
Only complete reports can explicitly state no anomalies. Known GitHub
credentials are redacted; posted summaries contain no secrets/raw logs.
[guide-budgets.json](../../../../typescript/scripts/guide-budgets.json) owns shared
guide line/byte limits used by the summary and guard. Reports remain input for lead judgement.
