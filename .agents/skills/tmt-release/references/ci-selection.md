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
- If every native-selecting path is Squad-owned, scope is `squad`: retain its
  Cargo checks, architecture guard, native tests and E2E file under the existing
  job names, and skip unchanged CLI runtime builds, packed installs and tooling
  unit tests. Any shared, CLI-owned or unknown native input requires full scope.
- Full workspace Rust checks still include Office and Remote members, independently
  of release eligibility. Reject empty Remote test discovery before execution.
  Office-specific browser, local-service and companion process checks require
  explicit Office selection; merge groups and main pushes do not run them.

## Worker gates

`Native Rust contracts` aggregates fmt/Clippy, workspace test/build, native process
and MSRV workers. MSRV reads `rust/Cargo.toml` and checks every workspace target.
Full scope requires all of those workers; the Office worker additionally requires
`native_office=true`. Squad scope requires its selected workers, and `none` skips
the aggregate. Invalid scope or worker results fail closed.

The selected Office build producer publishes one SHA-256-checked embedded-SPA
executable for the Office and native process workers. Without Office selection,
native process tests exclude Office-owned suites and require no companion
artifact; core discovery stays nonempty and other fixtures are built independently.

`Docker E2E` gates the two shard jobs selected by
`typescript/scripts/e2e-shards.mjs` and committed `test/e2e/shard-weights.json`.
Full native scope requires both shards (the first also runs adapter tests), scoped
component work requires the first, and no native selection requires neither.
The shard guard rejects any scenario in zero or two shards.

`Code quality` gates selected Office verification. `Native package matrix` gates
every selected native result and validates the event's `macos` classification:
false only for merge groups, where both macOS build/smoke results must be exactly
`skipped`; full-scope PRs require success. Linux rows remain required. Release
verification still includes macOS. Follow the
[runtime smoke matrix](../../tmt-e2e/references/runtime-smoke-matrix.md) for the
Rosetta process wrapper, exact installed-byte architecture admission and advisory
native Intel coverage.

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

Office's `office_browser` flag follows Office ownership plus Office-specific
workflow/emulator/Docker machinery. Shared dependencies, generic fixtures,
ordinary core dependencies and unknown paths do not select browser PR work while
Office is frozen. Weekly/manual runs cover all twelve partitions, including the
emulator. Infra's lead triages red scheduled runs and routes product failures to
their owner; workspace Rust coverage and required gate validation remain active.

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
