# CI selection and workers

The [component-map contract](../../../../ARCHITECTURE.md#ci-selection-and-worker-model)
keeps path ownership, CI selection and release attribution distinct. Read
`typescript/scripts/ci-scope.mjs`, `.github/components.json` and the affected
workflow before changing selection or a gate. Required checks remain `Code quality`,
`Unit tests`, `Docker E2E` and `Native package matrix`; selected missing, failed,
cancelled or unexpectedly skipped jobs fail closed. Never accept empty test discovery.

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

From `typescript/`, run the selection and workflow guards for a selection change:

```sh
corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts test/tooling/release-workflow.test.ts test/tooling/e2e-shards.test.ts
corepack pnpm check:tooling
```

Also run `actionlint` on every changed workflow and the selected layer's checks.
Review positive and negative path selections, cumulative queue diffs, shard
membership and exact aggregate results. A selection test never substitutes for
the runtime, archive or browser proof it selects.
