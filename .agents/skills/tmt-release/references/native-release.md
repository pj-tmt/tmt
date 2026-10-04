# Native release verification

Commands and gotchas for archive, installer, upgrade and bootstrap work. Cut, version
injection, exact-tag pipeline selection, publication gates and readback are in
[main-cuts.md](main-cuts.md); fixture builds for installation and upgrade proofs are in
[installation-fixtures.md](installation-fixtures.md). Policy and authorization live in
[SKILL.md](../SKILL.md): verification is never publication authorization, and ordinary
changes use the focused checks in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md). Run from the repository root unless
stated. Raw runtime proof is in the
[smoke matrix](../../tmt-e2e/references/runtime-smoke-matrix.md); raw executables do not
prove archives or public installation.
Archive, installer, upgrade and public smoke verifiers share `native-runtime-proof.mjs`
for linkage, exact embedded skills and SQLite reopen behavior; each caller retains
its independent inventory/checksum/notices and failure/cleanup checks.

## Native pipeline

The matching-host pipeline is `.github/workflows/native-release-bundle.yml`; exact-tag
dispatch, failed-draft recovery, publication gates and manual readback are owned by
[main-cuts.md](main-cuts.md). Cached packaging tools are keyed by OS, architecture and
exact tool versions and are developer tools only; Rust dependency caches are per product
and target and written by `main` only. cargo-dist merges the downloaded manifests
(`dist build --artifacts global --output-format=json --no-local-paths`): never hand-merge
artifact JSON. For the CLI pass a complete `dist plan` as bootstrap `--plan` so a missing
matrix target cannot shrink the release. Intel candidate verification fails closed without
the Rosetta tooling. The wrapper reports whether `uname -m` or Node
`process.arch` admission failed, including the resolved Node path for a wrong
architecture. Check the caller's x64 `setup-tooling` selection; version injection
keeps that Node unchanged.

## Archive inventory and install facts

Every native product archive carries its executable, `LICENSE`, `NATIVE-INSTALL.md`
and `THIRD-PARTY-NOTICES.txt`. The CLI may also carry optional companion executables;
Squad additionally carries its skills tree. The installer enforces inventory in
`tmt-core`'s `native_install/product.rs`: adding, renaming or dropping an entry
changes the installer contract and needs upgrade proof.

`rust/archive/NATIVE-INSTALL.md` supplies the archive note through
`dist-workspace.toml` and extension includes. Keep it short, product-neutral and
version-free; the handbook owns user guidance, and onboarding tests execute its
PATH block in Bash and Zsh.

Targets are macOS x64/arm64 (deployment target 11.0) and static-musl Linux x64/arm64.
For macOS x64 follow the [runtime acceptance policy](../../tmt-e2e/references/runtime-smoke-matrix.md):
arm64 cross-build, complete Rosetta verifier process trees with exact installed-byte
architecture checks, plus weekly native Intel public installation/upgrade coverage.
A deployment target is not evidence of testing every macOS version; report tested hosts.

Manifest SHA-256 checksums detect corruption, not a compromised origin. Local
checksums are not signatures; only published immutable releases carry GitHub
attestations. The generated `install.sh` fixes the initial version/channel, checks
manifest/archive sizes and digests before temporary execution, then delegates
permanent writes to the native installer. It requires POSIX shell, curl, tar/gzip,
standard utilities and `sha256sum` or `shasum`, edits no shell profile and touches
no SQLite. Receipts record local archive verification, not independent attestation.
Human success names `<requested-prefix>/bin/tmt` even through a symlinked prefix
ancestor; validation, receipts and JSON retain canonical paths.

## Building and verifying archives

Install the pinned tools into a chosen directory: cargo-dist 0.32.0 (`cargo install --locked`)
and cargo-about 0.9.2 (`cargo install --locked --features cli`). Fetch locked dependencies
before the offline notice step. Crates whose archive omits a license file (taffy 0.7.7, yrs
0.28.0) would otherwise get cargo-about's SPDX template with placeholder attribution. Each has a
`[crate.clarify]` entry in `rust/about.toml` that resolves to a vendored file,
`rust/licenses/<crate>-<version>/<file>` (the exact upstream file at the crate's
`.cargo_vcs_info.json` revision). The builder's `vendored_licenses` table lists them; it fails if a
file's bytes or the locked crate version change (review the clarification on an upgrade). To add
a crate, vendor its file, add the clarification with its sha256 and a `__TMT_<CRATE>_LICENSE__`
path token, and add a row to the table. Generate the offline config
with `scripts/build-native-artifact.sh --notices-only <target> <product>` before calling
cargo-about directly: use its `rust/target/native-notices/about.toml`, retain `--fail`
and the archive verifier's placeholder rejection. Select the CLI explicitly with
`--tag v<CLI-version>` for direct `dist plan`/`dist build` calls. Build targets sequentially in
one checkout (distribution directory and notice input are per checkout), retaining each
manifest, archive and notices before the next build:

```sh
scripts/build-native-artifact.sh --notices-only aarch64-apple-darwin squad     # notices only; not archive proof
native_manifest=$(mktemp)
MACOSX_DEPLOYMENT_TARGET=11.0 scripts/build-native-artifact.sh aarch64-apple-darwin > "$native_manifest"
node typescript/scripts/verify-native-artifact.mjs --manifest "$native_manifest" \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin \
  --skill skills/tmux-team/SKILL.md --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

The verifier bounds inputs (64 MiB compressed, 128 MiB expanded), enforces the product
inventory above, runs the extracted executable with no Node/Rust/tmux on `PATH` and checks
SQLite persistence; macOS needs `otool` and `lipo` through `xcrun` (10 s bound), so every
workflow job that runs it on macOS first runs `.github/actions/warm-xcrun` (a guard test
fails otherwise); Linux needs `readelf`. A generated notice file is not legal certification.
`typescript/test/native/artifact.Dockerfile` builds a matching-architecture Linux musl image
(`TARGET_TRIPLE`, optional `--build-arg PRODUCT=<product>`, task-owned name, run with
`--rm --init --network none`); remove that image afterwards.

Negative archive tests use real tar fixtures: exercise checksum corruption, truncation,
missing executable/notices, links, unexpected paths, duplicates, bounds and cleanup, never
accepting an arbitrary process error as proof. The hard-link fixture uses synchronous tar
construction (the async packer's link queue can hang) and must assert a real `Link` entry.
Compare large executable buffers with `Buffer.equals`, not structural matchers (heap
exhaustion), and verify the comparator detects a changed byte. Installation cases use an
explicit 15 s subprocess budget for debug archive hashing.

### Squad archives

A Squad archive adds `skills/` copied from `extensions/tmt-squad/skills/` through the
package's cargo-dist `include` (a package list replaces the workspace list, so it repeats
the shared files); the installer inventories it from the checksum-verified archive.

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin squad > /absolute/squad-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product squad --manifest /absolute/squad-manifest.json \
  --archive target/distrib/tmt-squad-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin \
  --skills extensions/tmt-squad/skills \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

Runtime proof runs `tmt-squad --version` and checks `tmt-squad skill show` prints the archived
`SKILL.md`, leaving an empty HOME and config. Then `tmt extension install squad --archive
<archive> --manifest <manifest> --prefix <task-owned-prefix> --channel alpha --yes`, a repeat
install and `tmt extension uninstall squad`.

### Herdr driver archives

`driver-herdr` is `release: false` in the component map and `dist = false` in its Cargo package
until owner-authorized activation (#1418) changes both for its first standalone alpha; the CLI archive
keeps shipping its binary until then. Build independently (no `tmt` build):

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin driver-herdr > /absolute/driver-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product driver-herdr --manifest /absolute/driver-manifest.json \
  --archive target/distrib/tmt-driver-herdr-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
node typescript/scripts/verify-native-driver-upgrade.mjs --product driver-herdr \
  --archive /absolute/new/tmt-driver-herdr-aarch64-apple-darwin.tar.gz --manifest /absolute/new/dist-manifest.json \
  --previous-archive /absolute/old/tmt-driver-herdr-aarch64-apple-darwin.tar.gz --previous-manifest /absolute/old/dist-manifest.json \
  --driver-archive /absolute/cli/tmt-cli-aarch64-apple-darwin.tar.gz --driver-manifest /absolute/cli/dist-manifest.json --target aarch64-apple-darwin
```

Build each version in task-owned source copies; never rewrite the implementation checkout or
share a Rust target directory. The standalone proof executes protocol capabilities with no
application state.

## Upgrade and installer verification

Installation and upgrade tests build their fixtures as described in
[installation-fixtures.md](installation-fixtures.md). Every released product must
upgrade from its newest lower published version, preserving candidate > previous,
downgrade rejection and readable old receipts. Standalone drivers use the current
published CLI's path approval surface. Keep ownership in the installation prefix,
not application-state selectors; verify pin policy, old executable preservation,
partial command-link finalization and unchanged data. The internal installer is
not permission to replace a user or package-manager installation.

`tmt upgrade [--channel stable|alpha] [--to <version> | --unpin] [--json]` and `tmt update`
share one grammar. Use task-owned managed prefixes; an unmanaged checkout binary fails before
networking. Production has no test endpoint or TLS bypass: API fixtures inject only the
adapter's acquisition boundary and local TLS fixtures use test-only trust. Verify channel
discovery with injected responses only (more than 1,000 refs without a Link, complete
pagination, numeric alpha ordering, stable/alpha/beta/rc separation, tags without releases,
malformed or cross-endpoint pagination, page/byte/request exhaustion) and assert the
two-request common case and no release/asset request before complete ref discovery.
Incomplete discovery fails with `Release discovery exceeds its bound; select an exact version
with --to.` (`NATIVE_UPGRADE_FAILED`). Rate-limit fixtures: `cargo test --locked -p
tmt-adapters release_http`. Archive, handoff and typed grammar checks:
`cargo test --locked -p tmt-adapters native_install` and `... -p tmt-cli native_install`;
the [handoff contract](../../../../contracts/native-install-handoff-v1.md) owns the probe. A
timeout after installation starts is an uncertain outcome, never proof of rollback. Assert
active receipts and surviving executables, not only exit codes.

Actual-archive acceptance needs separately versioned matching-host artifacts built in
task-owned source copies (never alter the repository version or publish fixtures to test
updates). It is `#[ignore]` and selected explicitly by the CLI upgrade proof:

```sh
TMT_UPGRADE_OLD_ARCHIVE=/abs/old.tar.gz TMT_UPGRADE_OLD_MANIFEST=/abs/old.json \
TMT_UPGRADE_NEW_ARCHIVE=/abs/new.tar.gz TMT_UPGRADE_NEW_MANIFEST=/abs/new.json \
TMT_UPGRADE_TARGET=aarch64-apple-darwin CARGO_BUILD_JOBS=2 \
cargo +1.97.0 test --locked --manifest-path rust/Cargo.toml -p tmt-adapters --lib \
  native_install::upgrade::artifact_tests::cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts \
  -- --exact --ignored --nocapture
node typescript/scripts/verify-native-installation.mjs --previous-archive "$previous_archive" --previous-manifest "$previous_manifest" \
  --archive "$next_archive" --manifest "$next_manifest" --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

Require exactly one selected passing test. Identical embedded skill text prints a skipped
differential result; candidate-byte equality, conflict preservation and repair still run.
The installation verifier checks old/new versions with no runtime `PATH`, pinned rejection,
explicit `--unpin` advancement, retained executable, no-op, downgrade rejection, exact skill,
unchanged SQLite bytes and migration of state the previous release wrote. The
`native-release-upgrade.yml` proof (also `workflow_dispatch` from `main` for any draft or
published tag) does this on four hosts; extension and driver releases use
`verify-native-extension-upgrade.mjs` and `verify-native-driver-upgrade.mjs`. The first
release of a product has nothing to upgrade from and says so; a commit that predates the
scripts fails the proof with that message and is proven by hand.

Extension-upgrade proofs (`test/native/extension-upgrade-proof.test.ts`) use one native
recording driver on macOS and Linux; build it first and publish the built bytes through
`writeExecutable` (never a shell driver, a compile during a scenario, a relaxed Mach-O
inspection or a longer deadline). Both native-process CI scopes build it. The synthetic
driver archive's `NATIVE-INSTALL.md` holds fixture JSON with absolute `executable` and `log`
paths; it is fixture configuration, not shipped installation guidance.

```sh
cargo build --locked --manifest-path rust/Cargo.toml -p tmt-test-support --example recording-cli-fixture
(cd typescript && corepack pnpm exec vp test run --config test/native/vitest.config.ts test/native/extension-upgrade-proof.test.ts)
```

Offline composition (an internal entry point, not in help or completion), always with a
task-owned prefix and matching archive:

```sh
native_prefix=$(mktemp -d)
rust/target/debug/tmt __native-install --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz \
  --manifest "$native_manifest" --prefix "$native_prefix" --channel alpha --json
"$native_prefix/bin/tmt" --version
```

`--pin`/`--unpin` change pin state, omission preserves it, and neither authorizes a downgrade.
Receipts written by v5.0.0-alpha.2 through alpha.6 record `wkh237/tmux-team`, which reading
still accepts.

### Remote and Colab installer registration

Core recognizes `remote` and `colab` separately from archive publication. From `rust/`:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-core native_install
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters native_install
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli extension_install_command
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli parser::tests::native_install
```

Process fixtures build CLI, Squad, Remote and Colab independently in the worktree's `rust/target`,
then run `extension-install.test.ts` through the native test config; they use the built
`tmt-remote` and `tmt-colab`, never a substitute CLI. A registered product with no published
archive (inject empty refs or a tag without a release) must report `EXTENSION_RELEASE_UNAVAILABLE`
("No published remote release yet") with no asset acquisition or prefix creation. Synthetic
archives prove installer behavior, not published linkage or runtime versioning. Publish the
supporting CLI alpha before testing a public install or upgrade.

## CLI upgrade proof

`native-release-policy.mjs::upgradeSupportFloor` declares the exact published CLI
floor (alpha.36). For candidates above it, `release-upgrade.mjs` stages both the
floor and newest published source below the candidate, deduplicating identical sources, and the candidate
`install.sh`; every archive, manifest and bootstrap must retain its recorded
GitHub digest. Candidates at or below the floor keep their historical single-source proof.

Candidate `native_install/handoff.rs::VERSION` owns probe applicability. Protocol-1
sources require the exact successful candidate probe in
[the handoff contract](../../../../contracts/native-install-handoff-v1.md); malformed,
failed or unsupported probes cannot become legacy evidence. Each source creates
its own receipt and application state. An offline source installer that rejects
added inventory must emit its actual `NATIVE_INSTALL_FAILED` / `Unexpected native
archive asset inventory.` error and preserve the complete installation and SQLite.
The actual candidate bootstrap then recovers with only curl acquisition replaced
by the exact staged versioned assets, retaining old bytes and migrating state cleanly.
Offline installation remains strict and does not prove self-upgrade delegation.

Every distinct source also passes the existing managed lifecycle/migration checks
and actual-archive adapter acceptance. Compile the candidate's adapter lib tests
once per host; require exactly one discovered ignored test and one passing execution
per source. Acquisition is injected; production candidate delegation and real
candidate execution are exercised by the candidate adapter. Public downloads and
`tmt upgrade` remain the separate post-publication smoke.

Keep each prove job's ten-minute timeout, read-only dependency cache and per-source
bootstrap/adapter plus compile durations. For tooling changes, use a PR-only,
read-only/no-secrets four-host rehearsal against a pinned real main source and
actual cargo-dist archives; remove temporary rehearsal machinery before readiness.
Do not dispatch a publishing workflow to obtain proof.

Focused checks from the repository root:

```sh
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/release-upgrade.test.ts test/tooling/native-upgrade-proof.test.ts test/tooling/native-release-policy.test.ts test/tooling/native-bootstrap.test.ts test/tooling/intel-verification.test.ts test/tooling/xcrun-warmup.test.ts test/tooling/release-workflow.test.ts test/tooling/repository-layout.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/native-release-upgrade.yml
```

## Curl bootstrap

Generate the script only after final cargo-dist archives and their independent runtime verification; use
the existing native publisher, never a competing stock installer. The
manifest, not a hand-kept version table, owns the facts:

```sh
node typescript/scripts/generate-native-bootstrap.mjs --manifest /abs/dist-manifest.json --archive-dir /abs/artifacts > /abs/artifacts/tmt-installer.sh
sh -n /abs/artifacts/tmt-installer.sh
node typescript/scripts/verify-native-bootstrap.mjs --manifest /abs/dist-manifest.json \
  --archive /abs/artifacts/tmt-cli-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

The verifier needs real matching-host artifacts and replaces only curl acquisition; it is not
live GitHub or cross-target evidence. The Dockerfile above carries it too (`--entrypoint node`).
`test/tooling/native-bootstrap.test.ts` covers negative paths with a stub that is never
publication evidence. An authorized release uploads the exact verified manifest, archives and
script and verifies the public download before the README advertises it.
Replacing npm/pnpm is a fresh installation without data-transfer machinery; never
delete old state or silently uninstall another manager.

## Packed verifier cleanup

Packed verifiers use bounded synchronous subprocesses and own their isolated process
groups. A terminated `spawnSync` result establishes direct-child termination. Cleanup
signals the owned group before temporary-state deletion. A teardown `EPERM` is tolerable
only after direct termination and a subsequent group probe reports `ESRCH`; a live or
unknown group or another signal error preserves the original failure. Never relax status,
signal, stream or deadline assertions. Negative fixtures own and clean their descendants;
this policy remains separate from the native sandbox's asynchronous cleanup protocol.

For `typescript/scripts/packed-command.mjs` changes run
`test/tooling/packed-command.test.ts` and `test/tooling/release-cut-live.test.ts` through
`vp test run --config vitest.config.ts`, then `pnpm check:tooling`. Keep the negative
controls, confirmed-absence proof and original subprocess deadlines.

## Conventional PR-title rollout

`Code quality` runs `node typescript/scripts/pr-title-check.mjs` only for `merge_group`: it
checks every pending squash subject (after removing GitHub's final `(#PR)`) against
`type(scope)?: subject` with a lowercase type, optional nonempty scope and `!`. It is syntax
feedback only; the cut planner owns release attribution. The mode is **report-only**:
findings (PR, SHA, escaped title) go to job output and `GITHUB_STEP_SUMMARY`, and invalid
titles, unavailable queue evidence or summary I/O errors still exit zero (unavailable evidence
is never a clean result). A title edited after queue entry is not compared against REST, and
full CI has no `edited` trigger. Enforcement is a separate small PR whose cutover is the
report-only PR's actual `merged_at` plus 24 hours as an explicit UTC timestamp; no
clock-dependent flip or ruleset edit. Verify with
`pnpm exec vp test run --config vitest.config.ts test/tooling/pr-title-check.test.ts`,
`pnpm test:run`, `pnpm check` and `actionlint .github/workflows/ci.yml`.

## Project release tracking

`project-release.mjs` owns delivery evidence separately from publication. Epic trackers
retain their owning lead's acceptance/dogfood gate and appear as skipped in the summary.

Each sweep executes trusted main tooling, exports current main once and reads its map and
Cargo graph once. Full-history closing merges supply changed paths and containing-tag
ancestry, not historical attribution rules. Native release policy/version helpers own
product identities; notes, commit types and recency windows are not release evidence.
For each affected product choose the earliest publication whose tag contains every closing
merge. Only complete product coverage permits `Released`; otherwise retain available
publication evidence and `Merged`. Private components await their consumers' releases.
Only never-shipped work or waits confined to parked products reconcile to `Done`, with
`ships with the first <product> release` for each parked wait. An absent status marker
preserves activation waits; private consumers cannot name a never-shipped product.
`release:false` alone is never evidence that work needs no release (Herdr awaits
activation; activated Remote and Colab wait for their published containing tags); the
map's `releaseStatus` is valid only with `release:false` (`never`: test support contained
in no release; `parked`: Office). Style and invoke require CLI, Squad, Remote and Colab
release evidence; TUI requires only Squad evidence.

Leave open issues, PR items, other repositories and project membership unchanged.
Recompute both owned fields, correcting stale terminal states and historical text. Complete
discovery and the dry-run plan precede bounded batched mutations and one Project readback.
Correct false terminal status before replacing evidence; write valid release evidence before
promoting to `Released`. Partial writes converge on the next authoritative full sweep,
including recovery from built-in close/merge workflow writes. Runs serialize project-wide
but do not claim atomic exclusion of external writers. Discovery caps fail before writes,
never silently truncate. The daily sweep recovers missed dispatches and genuine smoke
failures without authorizing publication or a publishing-workflow replay.

`project-release.yml` sweeps closed issue items (except `epic`) of
[pj-tmt project 1](https://github.com/orgs/pj-tmt/projects/1) and owns `Status` and
`Released in` for them; it needs **Organization projects: read and write** on the release App
token (main-only `release` Environment) and no PAT. The daily cron is 04:23 UTC, and
`native-release-bundle.yml` dispatches a full sweep after publication read-back and
successful smoke. After changing component eligibility or Rust production dependencies run the cut, component-scope and workflow tests (`test/tooling/release-cut.test.ts`, `ci-scope.test.ts`, `release-workflow.test.ts`); keep CI scope separate from release attribution (normal/build workspace dependencies follow the product binary through Cargo-resolved metadata). Avoid manual edits to those two fields during a live run. Product attribution
uses `ci-scope.releasedComponentsForPath` (`owns`/`excludes` plus each released package's
transitive Cargo normal/build workspace dependency directories; dev-only edges never
attribute) over `cargo-workspace.mjs::readCargoWorkspace(root, {runner})` (`cargo metadata
--offline --locked`, no Git logic); workflows run `cargo fetch --locked` from `rust/` first.
Bounds per run:
200 GraphQL and 20 REST requests, 20 pages per connection, 2,000 merged closing PRs; every
GraphQL read reports `rateLimit`, and insufficient reserve aborts before any write. For
activation or a change, dry-run first and review the per-item table against the first live run:

```bash
gh api repos/pj-tmt/tmt/actions/workflows/project-release.yml/dispatches --method POST --input - <<'JSON'
{"ref":"main","inputs":{"dry_run":"true"}}
JSON
# After reviewing the table, repeat with dry_run=false.
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/project-release.test.ts test/tooling/cargo-workspace.test.ts test/tooling/release-attribution.test.ts test/tooling/release-cut.test.ts test/tooling/ci-scope.test.ts test/tooling/release-workflow.test.ts test/tooling/release-publish.test.ts)
actionlint .github/workflows/project-release.yml .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-smoke.yml
```

## Merge queue metrics

Read-only REST evidence (no GraphQL) on queue throughput, group duration, enqueue latency and
per-job failures: `node typescript/scripts/merge-queue-metrics.mjs --repo pj-tmt/tmt --since <UTC>
--until <UTC> [--boundary <UTC>] --cache <dir> --output <md> --json <json>`. Bounds are UTC, start
inclusive and end exclusive; `--boundary` compares cohorts, `--details` prints all cost rows,
`--offline` requires cached evidence, `--max-requests N` overrides the 500-request budget (split
capped windows), `--workflow FILE` defaults to `ci.yml` and `--tag-pr N` (repeatable) marks
confounder PRs. The script and report state their methodology limits. Verify with
`pnpm exec vp test run --config vitest.config.ts test/tooling/merge-queue-metrics.test.ts` from
`typescript/`.
