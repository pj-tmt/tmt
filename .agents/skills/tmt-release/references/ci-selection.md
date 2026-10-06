# CI selection and workers

The [component-map contract](../../../../ARCHITECTURE.md#ci-selection-and-worker-model)
keeps path ownership, CI selection and release attribution distinct. Read
`typescript/scripts/ci-scope.mjs`, `.github/components.json` and the affected
workflow before changing selection or a gate. Required checks remain `Code quality`,
`Unit tests`, `Docker E2E` and `Native package matrix`; selected missing, failed,
cancelled or unexpectedly skipped jobs fail closed. Never accept empty test discovery.
The publication commit gate accepts scope-skipped `Unit tests` only with the latest
successful `Native package matrix` from the same GitHub Actions suite, completed
no earlier than that skip; the aggregate validates selection. Publication reuses this
proof with check-suite provenance, never recomputing historical selection or accepting bare skips.
Other required contexts must succeed. Held-draft recovery belongs to
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
- If every native-selecting path is Squad-owned, scope is `squad`: retain its
  Cargo checks, architecture guard, native tests and E2E file under the existing
  job names, and skip unchanged CLI runtime builds, packed installs and tooling
  unit tests. Any shared, CLI-owned or unknown native input requires full scope.
- Workspace Rust checks exclude only `tmt-office`, `tmt-office-storage`,
  `tmt-office-pairing` and `tmt-office-service`. The shipped CLI imports
  `tmt-office-command` and `tmt-office-model`, so their tests, Clippy and MSRV
  checks remain selected alongside CLI contracts, architecture and embedded inputs.
  Reject empty Remote test discovery before execution. Office product verification
  is retired for every event, independently of its parked release attribution.

## Worker gates

`Native Rust contracts` aggregates fmt/Clippy, workspace test/build, native process
and MSRV workers. MSRV reads `rust/Cargo.toml` and checks all retained workspace
package targets. Full scope requires the retained workers and exactly a skipped
Office worker (`native_office=false`). Required-check names stay stable; the
aggregate reports success only with those results. Squad scope requires its
selected workers, and `none` skips the aggregate. Invalid scope or worker results
fail closed.

Office SPA, local-service and companion producers never run. Native process
verification excludes Office-owned suites and requires no Office companion;
core discovery stays nonempty and other fixtures are built independently.

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

## Experimental workspace N=1 proof

The existing CI dispatch entry has an opt-in `workspace_proof=true` route with an
exact `proof_head`. It calls `workspace-proof.yml` and selects `none` for ordinary
workers; normal PR, merge-group, main and seed coverage is unchanged. Infra must
review the immutable-head launch plan before dispatch. Only original attempt 1 is
admitted. This experiment is not a required check, rollout or performance result and
never substitutes for required workers. `Native Rust workspace tests`, its four
exclusions, 20-minute cap and outer gate positions remain unchanged until separately
reviewed equivalence and rollout.

`workspace-proof.mjs` owns N=1 artifact/list/disposition admission;
`run-workspace-proof.mjs` owns bounded capture, observations and execution. The producer
runs the retained build before one locked workspace no-run JSON compilation, then
uses Cargo-owned metadata and `release-version parse` to admit targets. Remote
must have a nonempty list from those exact workspace executables; architecture,
Office command/model, ignored and zero-test harnesses remain explicit. Each doctest
target needs observed normal-list, ignored-list and execution completion, including
an explicit zero disposition. Ordered Cargo headers and Rustdoc source owners must
admit exactly one consecutive partition of the actual blocks; Rust 2024 multi-block
targets remain supported, while absent, duplicate-zero or ambiguous completion fails.
Cargo-emitted whole output sets plus executable qualify artifact identity; exact
feature vectors remain separate comparison payloads. Distinct variants stay separate,
while duplicate identities or shared output ownership refuse admission before Maps.
Harness assignment IDs keep their semantic target identity. Default/producer
inventory equality and each observed doc output identity/feature match remain exact;
unsupported output/profile differences refuse proof, without filename suffix parsing
or feature union. No source or Markdown parser infers missing evidence. Unknown
custom harnesses, required-feature target eligibility or output formats fail.

Cargo JSON stays a strict prefix through one successful `build-finished`. Mixed
execution can retain complete reasonless JSON objects only within an active nonzero
ordinary admitted-harness block before its summary. Direct restored harnesses use
the same explicit context; lists, zero-test and rustdoc blocks stay strict. Cargo
envelope keys, malformed or ambiguous objects and unclassified text refuse admission.
Auxiliary output is opaque: it never supplies a test terminal or sender/test identity,
and diagnostic values are not compared across runs. Original streams retain the bytes;
ordered command/harness-block, line, byte-count and hash references are reported separately
from exact coverage. The existing 4000-entry bound caps auxiliary references, and each
metadata report write counts toward the unchanged role evidence budget.

The manual workflow serializes producer, default Cargo baseline, restored whole
harness consumer and separate rustdoc obligations on Ubuntu 24.04 with Rust 1.97.0,
Node 22.23.2/pnpm 10.33.0. Baseline retains the original Remote listing and
workspace test-before-build order; its extra listing calls record obligations.
The producer's frozen regular-file target/Cargo-source archive is restored only
into absent roots at the same absolute paths. Source, toolchain/sysroot, GNU
loader dependencies, executable hashes, reviewed environment and default versus
separate-doc library features are bound to one source/run/attempt. Cargo cache
lock/access bookkeeping is mutable; dependency sources and acceptance binaries
are hash checked. Archive membership, missing/duplicate/extra names, changed
ignore disposition, filtering, process/listener leaks and selected failed or
cancelled jobs refuse aggregate success. Runtime closure, descendant cleanup and pinned output forms
remain unproved until a separately admitted original Linux experiment passes; fixture counts alone
cannot establish them. Every role keeps original streams/process reports on red.

Bounds live in `LIMITS`:150,000 regular files, 5 GiB payload, 512 MiB per file,
5 GiB plus bounded tar overhead, 32 MiB per command and 256 MiB total captured streams,
4000 commands and 18 minutes inside each 20-minute job. The final minute is
reserved for cleanup; command admission/capture settles before it, with at most five seconds for
exit/pipe settlement. Missing close, probe denial or overflow retains partial
streams with incomplete/unconfirmed evidence. Every role observes processes and
listeners before execution and in finally, including original red paths. Final process
observation follows the settled listener observer. Bounded stat/start/parent/group/session
facts and available executable/cwd links, direct-child observations and command-time
references are retained on red too; identity races, denied/vanished reads and limits are
explicit. Global new or unconfirmed identities remain red even with host-looking names.
No host command lines or environments are collected; own allowlisted argv is already
in command records. Snapshots allow at most 4000 identities, 32 MiB output and a
one-second observation window clipped to the existing phase deadline; stat fields are
at most 4096 bytes and link facts 1024 characters. Observations count toward the existing
256 MiB role evidence bound. Available facts diagnose a refusal, never infer custody. The runner
never sends effectful numeric PID/PGID signals: Node event state, ancestry, snapshots
and signal-zero probes cannot provide retained OS lifetime custody, even before
the exit event. Stops settle only our handles and remain termination-unconfirmed;
actual descendant termination is not claimed, and the outer job cap stays unchanged.
A numerically present stale group remains red, without adoption or signalling.
Sysroot and package-manager probes use the same captured-command/finally contract.
Unconfirmed cleanup stays red and leaves runtime roots intact. Confirmed
cleanup uses a bounded owned-root removal command before final report admission. Producer requires 12 GiB free before acquisition; baseline/doctest require 6 GiB.
Consumer requires 6 GiB after download, with at most 512 MiB for copied-input
sensitivity controls. Controls remove/corrupt private copies of the actual child,
library, Cargo and source inputs; immutable originals remain untouched. They prove
admission sensitivity, while real runtime necessity still requires the positive
harness proof. GNU tar packs
the verified files directly with hard-link dereferencing, avoiding a staging copy;
the uncompressed transport has the same membership/hash checks as the payload.
Only admitted cleanup permits bounded removal of owned target, Cargo home and runtime temp/home after retained reports;
existing roots are rejected, never overwritten. A bound or format failure keeps
the current worker and original red evidence. N=2/3 needs a later owner assignment.

## Cache ownership

Rust dependency caches use the pinned `Swatinem/rust-cache` action with one
main-only writer per key: workspace tests for shared dev dependencies, MSRV for
its toolchain, and each runtime target for its own cache. All writers use the
shared seed-event classification (`verify=false`) and main ref. PRs, merge groups
and other workers only restore. Dev debug information and incremental compilation
are disabled in CI; release profiles keep their manifest policy.

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
