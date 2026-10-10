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

[Release-index v1](../../../../contracts/release-index-v1.md) owns record identity and
bounds. Draft assembly reads back the record before the completeness marker;
publication verifies it against release identity and downloaded bytes. The top-level
`native-release.yml` release-index job re-verifies in its own directory before
obtaining the release environment App token and rechecking those bytes for a
non-force pointer update. Smoke and the Project reconcile dispatch precede indexing;
an index failure keeps the release published and the run red. API-free client
consumption remains pending.

The packaging stages (build, assemble, final verification on the four matching hosts) live in the
read-only reusable `.github/workflows/native-release-prepare.yml`, called with an exact source SHA
by `native-release-bundle.yml` for a draft and by the rehearsal before merge, so a release is never
the first run of a packaging check. The rehearsal has no secrets, Environment, repository write
access, tags or draft access. Cache behavior: the Rust dependency cache is saved by main only; the
packaging-tools cache is saved on a miss in any run, including a pull request's own scope.
Manifest assembly retries its required exact-key tool-cache restore once; a final miss or restore failure still fails verification and retains the failed-draft policy.

**Release rehearsal.** `ci.yml` selects it on pull requests only, never in the merge group:
`typescript/scripts/release-rehearsal.mjs select <base> <head>` rehearses every active product
(released components with a package, from the component map and native release policy) when a
shared release input changes (lockfiles, workspace/toolchain/notice/dist configuration, the
component map and parity manifest, packaging and verification scripts, the prepare, bundle,
upgrade and rehearsal workflows and their local actions, and Cargo manifests under `rust/`), and
only the owning products for an extension Cargo manifest. Native-install and managed-skill
installation modules, plus the CLI native-upgrade command and its tests, select CLI only for the
existing real-archive upgrade proof. Other ordinary source and bundled-frontend edits never
select it; their own jobs cover them, and a frontend-only change that needs packaging proof uses
`release-rehearsal.yml` by dispatch. `Native package matrix`
requires a selected rehearsal to succeed and an unselected one to be skipped.
`release-rehearsal.yml` runs every active product nightly on main (and by dispatch); a red run is
the triage evidence. Publishing stays in `native-release-bundle.yml` (`check`, the `prepare` call,
`attach` and the jobs after it), whose `attach` and `record-failure` depend on the `prepare` call.
Verify with `test/tooling/release-rehearsal.test.ts` and `test/tooling/release-workflow.test.ts`.
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
A component declaring `skills: true` in `.github/components.json` additionally carries its
skills tree. The installer enforces inventory in
`tmt-core`'s `native_install/product.rs`: adding, renaming or dropping an entry
changes the installer contract and needs upgrade proof.
An extension archive may also carry `TMT-USES.json`; the installer validates it
(`native_install/uses.rs`, rules in `contracts/extension-api.md`) and a malformed file rejects
the release before publication. No script re-parses it: `extension-install.test.ts` proves the
rejection through the real CLI before merge, so the release verifiers need no separate check.

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

PR and merge-group CI checks dependency notices when the locked dependencies, clarification,
license inputs or notice machinery change. `Native package matrix` requires the selected notice job.
`node typescript/scripts/verify-native-notices.mjs` runs the builder's `--rust-notices-only` mode
for every active native product in `components.json` and every target in `dist-workspace.toml`,
then applies the archive verifier's empty/placeholder rejection. It needs Python 3.11+, pinned
cargo-about and fetched locked crates. This mode skips frontend installation/builds and checks
only cargo-about inventories; the ordinary notice/archive modes retain combined frontend notices.
Inventories, diagnostics and timings remain under `rust/target/native-notices/verified/` and
are uploaded as CI evidence. This performs no native compilation and proves notices only.

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
scripts/build-native-artifact.sh --notices-only aarch64-apple-darwin ops     # notices only; not archive proof
native_manifest=$(mktemp)
MACOSX_DEPLOYMENT_TARGET=11.0 scripts/build-native-artifact.sh aarch64-apple-darwin > "$native_manifest"
node typescript/scripts/verify-native-artifact.mjs --manifest "$native_manifest" \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin \
  --skill skills/tmt/SKILL.md --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
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

### Ops archives

A candidate skills product's archive (`skills: true` in `.github/components.json`; the archive policy,
verifier and `component-skills.test.ts` read that one field) adds `skills/` copied from
`extensions/<name>/skills/` through the
package's cargo-dist `include` (a package list replaces the workspace list, so it repeats
the shared files); the installer inventories it from the checksum-verified archive.

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin ops > /absolute/ops-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product ops --manifest /absolute/ops-manifest.json \
  --archive target/distrib/tmt-ops-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin \
  --skills extensions/tmt-ops/skills \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

Runtime proof runs `tmt-ops --version` and checks `tmt-ops skill show` prints the archived
`SKILL.md`, leaving an empty HOME and config. Then `tmt extension install ops --archive
<archive> --manifest <manifest> --prefix <task-owned-prefix> --channel alpha --yes`, a repeat
install and `tmt extension uninstall ops`. Published historical archives declare skills through
their own verified manifest inventory; retired Squad needs no current component record and
is refused as a release candidate.

### Herdr driver archives

`driver-herdr` is activated for independent native alpha cuts (#1418), starting at `0.1.0-alpha.1`.
The CLI archive retains its companion until released-package acquisition (#1084) is delivered;
standalone archives use the existing explicit executable-path approval. Build independently (no `tmt` build):

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
downgrade rejection and readable old receipts. With no own prior publication, predecessor
selection and digest-checked staging retain the old product tag/archive identity; cross-product
installer and migration acceptance requires its separately reviewed Core/product contract. Standalone drivers use the current
published CLI's path approval surface. Keep ownership in the installation prefix,
not application-state selectors; verify pin policy, old executable preservation,
partial command-link finalization and unchanged data. The internal installer is
not permission to replace a user or package-manager installation.

For the one-release Core name cutover (#2270), the owner runs two back-to-back
unpinned `tmt upgrade` commands: the first activates rename A (the first release
that carries the tmt names) and refreshes managed skills with the new executable;
the second, from newly active A, converts verified pane options. The prior
executable owns the first finish and cannot run the new pane converter. Do not
release rename B (the later release that drops the former names) until this
second step completes; no install/setup or skill-refresh pane effects are added.
Remove this paragraph once rename B has shipped.

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
  --archive "$next_archive" --manifest "$next_manifest" --target aarch64-apple-darwin --skill skills/tmt/SKILL.md
```

Require exactly one selected passing test. Identical embedded skill text prints a skipped
differential result; candidate-byte equality, conflict preservation and repair still run.
The installation verifier checks old/new versions with no runtime `PATH`, pinned rejection,
explicit `--unpin` advancement, retained executable, no-op, downgrade rejection, exact skill,
unchanged SQLite bytes and migration of state the previous release wrote. The
`native-release-upgrade.yml` proof (also `workflow_dispatch` from `main` for any draft or
published tag) does this on four hosts through the shared proof stages in
`native-release-upgrade-prove.yml`; extension and driver releases use
`verify-native-extension-upgrade.mjs` and `verify-native-driver-upgrade.mjs`. A product with
neither own nor predecessor published history has nothing to upgrade from and says so; a commit
that predates the scripts fails the proof with that message and is proven by hand.
Retries of pre-rename tags use the tag's own tooling; current tooling expects `skills/tmt/SKILL.md` in the release source.
Asset acquisition by immutable GitHub id allows three attempts with 1/2 s backoff and the unchanged
300 s per-call bound, logging earlier failures; staged-byte verification and local file errors are never retried.
Only x64-Apple verification dependency installs allow one extra attempt after the exact Node async-hook abort,
with a 5 s wait, owned partial-module cleanup and visible attempt logs; other failures stay immediate,
and the existing 600/780 s verification/proof job caps and failed-draft publication protections remain unchanged.

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
Receipt provenance accepts `pj-tmt/tmt` and the former organization name `wkh237/tmt`.

### Remote and Colab installer registration

Core recognizes `remote` and `colab` separately from archive publication. From `rust/`:

```sh
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-core native_install
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-adapters native_install
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli extension_install_command
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli parser::tests::native_install
```

Process fixtures build CLI, Ops, Remote and Colab independently in the worktree's `rust/target`,
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

Keep each prove job's thirteen-minute timeout and per-source bootstrap/adapter plus compile durations.
Successful main-ref Darwin CLI proofs may seed the purpose-specific adapter dependency cache, reusing the existing compilation when applicable and its environment hash; PRs, Linux and non-CLI proofs only restore. Adapter stderr separately records compile, discovery and each source run's start plus returned/failed elapsed seconds; a returned command is not proof acceptance, and a start without a terminal line has no measured completion. Do not dispatch a publishing workflow to obtain proof.

**Rehearsal upgrade proof.** The release rehearsal (`ci.yml` on selected pull requests and the
nightly `release-rehearsal.yml`, never the merge group) passes `upgrade: true` to
`native-release-prepare.yml`. Its read-only `upgrade-fetch` job stages the candidate from the
verified bundle of the same run (`release-upgrade.mjs fetch --candidate-directory`): the manifest's
announcement tag is the synthetic tag, and every archive must match both its cargo-dist `.sha256`
file and the staged digest, so the candidate never vouches for itself. The previous release, declared
floor and driving CLI still come from published releases only, and a synthetic candidate must be
newer than the newest published release or the fetch fails. The same proof stages
(`native-release-upgrade-prove.yml`) then run on the four hosts, including the CLI adapter
acceptance. The write-token `fetch` of `native-release-upgrade.yml` is called only by the publication
bundle on main, so a pull request can only reach the local fetch; the publication path's upgrade
job keeps its jobs and outputs (`release-workflow.test.ts` evaluates the conditions).

When the selected previous release is a declared predecessor, fetch requires the successor's
`requiresCliSha`, resolves published CLI tags by REST, and checks ancestry against that registration.
The newest published CLI must contain it; the previous driver is the newest strict ancestor before
it (drafts and divergent commits cannot substitute). Both drivers use the existing digest-checked
staging path, with captured tag/commit provenance rechecked before emitting the verifier's second
driver arguments. Same-product staging keeps one driver. The one extension verifier accepts the
complete former-product/previous-driver pair only for Squad -> Ops: the old CLI establishes Squad
and skill consent; the new CLI replaces it, checks reported removal order, owner/target migration,
list/repeat/old-archive refusal, unchanged application bytes and absence of CLI links.
Default PR CI does not run the historical two-real-CLI scenario. Explicit matching-host qualification
uses the digest-recorded staging directory described in [installation fixtures](installation-fixtures.md#prepared-squad-to-ops-qualification),
with separately admitted builds; the first published Ops release still requires its ordinary token-free proof.
Reported JSON removal order and final durable state do not independently prove internal lock,
syscall or failure timing.

**Rehearsal publication gates.** The same rehearsal runs `gates-dry` in the prepare workflow:
`publication-gates.mjs dry --product P --tag <synthetic tag> --sha <candidate> [--on-main]` evaluates
`channel`, `immutability`, `monotonic` and `migration` for the candidate from published releases and
git history, plus `commit` on main (the nightly run; a pull request's merge commit is not on main
yet). It records no hold and writes nothing, and a gate that would hold fails the job with the gate
named. Only the draft-bound evidence (the allocated draft, its hold marker and the proof's result in
`finish`) stays release-only. The live `release.yml` `cut` job also stays release-only: it needs a
token that sees drafts.

Focused checks from the repository root:

```sh
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/release-upgrade.test.ts test/tooling/native-upgrade-proof.test.ts test/tooling/native-release-policy.test.ts test/tooling/native-bootstrap.test.ts test/tooling/intel-verification.test.ts test/tooling/xcrun-warmup.test.ts test/tooling/release-workflow.test.ts test/tooling/repository-layout.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/native-release-upgrade.yml .github/workflows/native-release-upgrade-prove.yml .github/workflows/native-release-prepare.yml
```

## Curl bootstrap

Generate the script only after final cargo-dist archives and their independent runtime verification; use
the existing native publisher, never a competing stock installer. The
manifest, not a hand-kept version table, owns the facts:

```sh
node typescript/scripts/generate-native-bootstrap.mjs --manifest /abs/dist-manifest.json --archive-dir /abs/artifacts > /abs/artifacts/tmt-installer.sh
sh -n /abs/artifacts/tmt-installer.sh
node typescript/scripts/verify-native-bootstrap.mjs --manifest /abs/dist-manifest.json \
  --archive /abs/artifacts/tmt-cli-aarch64-apple-darwin.tar.gz --target aarch64-apple-darwin --skill skills/tmt/SKILL.md
```

The verifier needs real matching-host artifacts and replaces only curl acquisition; it is not
live GitHub or cross-target evidence. The Dockerfile above carries it too (`--entrypoint node`).
`test/tooling/native-bootstrap.test.ts` covers negative paths with a stub that is never
publication evidence. An authorized release uploads the exact verified manifest, archives and
script and verifies the public download before the README advertises it.
Replacing npm/pnpm is a fresh installation without data-transfer machinery; never
delete old state or silently uninstall another manager.

## Compiled CLI schema preparation

New CLI preparation requires Core's accepted `__native-schema --source-sha <cut> --json`
exporter. A source predating it refuses explicitly; existing published schema-less archive
readers retain their compatibility. The schema carrier supplies no producer trust
tuple, RC catalog, cleanup qualification or publishing authorization. Exporter integration and
separately authorized four-host proof are delivery gates; tooling fixtures prove no native archive.

The existing target jobs export from their verified archive's CLI with matching-host/Rosetta
execution and no extra build. `native-application-schema.mjs` checks the strict compact v1 record
against the exact-cut version snapshot: the complete direct `storage/schema/*.sql` inventory,
`storage/migrations.rs` and `storage/migrations/host_names.rs`, including the separate indexes input.
Unknown/omitted/extra closure inputs refuse; changing that declared closure requires Core review.
Core owns domain/version semantics; SQL counts and the descriptive caller SHA prove neither.
Version-only manifest/lock edits remain with the existing injection owner. Export runs with an
empty PATH and isolated HOME/cwd/config, no credentials, and refuses observed state writes;
this observation is not an OS sandbox or proof of no external effects. Red roots are retained.

Target evidence binds captured source, binary/output and unchanged archive hashes. After the
ordinary global cargo-dist merge, all four records must agree before the carrier adds
`tmt_application_schema`, still within the 4 MiB final manifest bound. Bootstrap generation and
final upload follow; final matching-host archive verification independently exports the extracted
CLI and compares the whole field and target evidence. It refuses a changed manifest through the
end of runtime verification. Sidecars remain preparation evidence, not a new release asset policy.
No extension schema is inferred, published manifest rewritten or PR binary executed with writer
credentials. Source/tooling proof and actual integrated native preparation remain separate gates.

## Opt-in PR release candidates

`pr-rc.yml` runs reviewed main tooling for an open same-repository PR carrying
`rc-build`; dispatch supplies the exact head and its first twelve hex characters.
The label opts into CLI preparation on all four native targets and its runner cost;
per-PR concurrency cancels superseded runs. Existing `native-release-prepare.yml`
receives the guarded PR head as source data, uses its ordinary verification gates,
and captures a report after each matching-host final archive/source check.
No PR executable runs with artifact-deletion or repository-write authority.

`pr-rc-coordinator.mjs` reads authenticated current pull, label timeline and main
workflow-run identity before staging unchanged archive/manifest bytes. Four final
reports bind the same run/attempt, source, version, schema, archive and notices.
Each three-day payload contains only the root manifest and target archive. Returned
Actions IDs, ZIP sizes/digests and bounded regular-root member bytes are read back
before the revision-2 catalog is uploaded last as the sole `catalog.json` member;
its returned ID and bytes are checked again with fresh current-head/opt-in evidence.
Missing, ambiguous, incomplete or changed observations fail visibly without retry.
Core owns catalog admission, compiled schema semantics and installer trust; the
producer's workflow/tooling hashes remain diagnostics, not a rotation allow-list.

`pr-rc-cleanup.yml` uses one no-checkout implementation for close, daily reconciliation,
manual dispatch and post-publication retirement. It has only `actions:write` and
identifies its artifacts through the authenticated `pr-rc.yml` main run and exact
`pr-rc #<PR> <head12>` run-name, then records exact artifact IDs before deletion.
At most two current successful catalog-bearing generations remain live per PR.
Close, missing label, changed head or three-day expiry retires catalog discovery
first; an owned pending run is cancelled and observed settled before payload cleanup.
A fresh inventory catches late uploads, and each deletion requires authenticated
absence. Unknown ownership, incomplete inventories, unsettled cancellation or late
uploads fail visibly; retention is the backstop, not proof of physical reclamation.
Ordinary preparation artifacts, caches, releases and local installations are untouched.

Focused controls in `test/tooling/pr-rc-coordinator.test.ts` cover catalog bytes,
final verification binding, bounded ZIP readback and exact-ID cleanup; workflow
controls pin trusted preparation and catalog-last ordering. Native preparation,
representative installs and the Core reader's protected-main trust seam remain
separate qualification evidence; source controls do not claim a live acceptance run.

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

## PR Agent attribution

Every PR body needs visible `Agent: <seat> <preset>` lines for its contributors,
or `Agent: ben` for human-only work. Seat is `[a-z0-9][a-z0-9-]*`; the parser and
approved presets are owned by `typescript/scripts/pr-agent-check.mjs`.

| Preset              | Model and effort   |
| ------------------- | ------------------ |
| codex-sol-high      | GPT-6.1-Sol high   |
| codex-sol-med       | GPT-6.1-Sol medium |
| codex-luna-med      | GPT-6-Luna medium  |
| codex-luna-low      | GPT-6-Luna low     |
| claude-opus-med     | Opus 5.5 medium    |
| claude-sonnet-high  | Sonnet 5.5 high    |
| claude-sonnet-xhigh | Sonnet 5.5 xhigh   |

New models may use `Agent: <seat> <provider> <model> <effort>` with three
`[A-Za-z0-9._-]+` value tokens. An unknown single preset or malformed claimed line
fails even beside a valid line. Fenced examples and HTML comments do not count;
replace the visible template placeholder. The provider co-author trailer stays.
Only the exact REST author `tmt-ci-bot[bot]` together with a `chore(main):` title
is exempt. The maintainer's account and other bots are not exempt.

Code quality reads the current PR body through REST, including on reruns. The
small Conventional PR title job also checks attribution on body edits; correcting
that feedback does not replace an earlier failed Code quality result, which needs
an authorized rerun. Merge-group checks initially report findings and unavailable
evidence without failing; queue enforcement is a separate rollout after open PRs
have been attributed. No required-check name or workflow permission changes.

## Conventional PR titles

`typescript/scripts/pr-title-check.mjs` owns the exported `CONVENTIONAL_PR_TYPES` policy and
`type(scope)?: subject` syntax (optional nonempty scope and `!`). It is syntax feedback;
the cut planner owns release attribution.

- **Pull request: enforcing.** Both `Code quality` and the small `pr-title.yml` workflow read
  the current title through `gh api` (a rerun's event title may be stale) and changed paths
  from the event's `base...head`. A title outside the approved policy fails when a path
  changes a released component, using `releasedComponentNamesOfPath` (owned roots plus
  declared consumers, without Cargo closure). `cli` owns `.` except other components' roots;
  only PRs confined to unreleased roots keep any title. Missing evidence fails closed.
  The separate read-only Node job runs on opened/edited/reopened/synchronize with independent
  concurrency, no dependency install or product build. Editing a title triggers that check;
  rerun `Code quality` if its previous title step failed. Full CI has no `edited` trigger.
- **Merge group: enforcing.** `Code quality` checks every pending cumulative squash subject
  after removing GitHub's final `(#PR)`, not only the queue tip or fresh REST title. Invalid
  titles and unavailable queue evidence fail the required job; PR/SHA/exact escaped title
  findings and expected syntax/types are retained in stdout and `GITHUB_STEP_SUMMARY`.
  Summary I/O cannot change the decision. Explicit `--report-only` retains the former
  observation command's zero exit on findings/unavailable evidence; required CI never uses it.

The planner is the fail-safe behind the gate: a non-conventional or capitalized top-level
subject on a released component's paths is releasable and listed under `Other changes`, never
dropped (#1643). Conventional `docs`, `chore`, `test` and similar types stay hidden.

Verify with
`pnpm exec vp test run --config vitest.config.ts test/tooling/pr-title-check.test.ts test/tooling/release-cut.test.ts`,
`pnpm check:tooling` and `actionlint .github/workflows/ci.yml .github/workflows/pr-title.yml`.

## Project release tracking

`project-release.mjs` owns delivery evidence separately from publication. Epic trackers
retain their owning lead's acceptance/dogfood gate and appear as skipped in the summary.

Each sweep executes trusted main tooling, exports current main once and reads its map and
Cargo graph once. Full-history closing merges supply changed paths and containing-tag
ancestry, not historical attribution rules. Native release policy/version helpers own
product identities; notes, commit types and recency windows are not release evidence.
A sibling product can publish between the checkout's tag fetch and the REST release listing,
so `validateTags` fetches exactly the missing published tags once and revalidates; tags still
unresolved fail the sweep closed and are named in the error.
For each affected product choose the earliest publication whose tag contains every closing
merge. Only complete product coverage permits `Released`; otherwise retain available
publication evidence and `Merged`. Private components await their consumers' releases.
Only never-shipped work or waits confined to parked products reconcile to `Done`, with
`ships with the first <product> release` for each parked wait. `release:false` alone never
proves that work needs no release; an absent status marker retains activation waits.
The map's `releaseStatus` is valid only with `release:false` (`never`: test support contained
in no release; `parked`: Office and the private browser-addon demo). Private consumers cannot
name a never-shipped product. Browser-addon publication remains deferred by the v1 freeze
(#1056 / v1-later), with no release consumer. Released products require published containing tags;
Style and invoke require their active consumers' containing tags; TUI is contained by Ops through its Cargo closure.

For reviewed leaves inside a component, `neverShippedPaths` in the component map is a
Project-only list of `{root, reason, testOnlyReferences?}`. Roots are normalized literal
repository-relative paths owned by that component; undeclared and mixed shipping inputs
retain containing-tag requirements. Each optional test-only reference names `{file, reason}`;
the architecture guard requires its parent module declaration immediately after literal
`#[cfg(test)]`. This field changes neither ownership, cuts, versions nor ordinary CI selection.
CI runs the separate all-file, macro-token-aware Rust architecture guard for map changes,
every declaring crate input and the release build script. It rejects shipping source,
package/dist/skill-tree overlap, production references, missing/ambiguous inputs and unproved
dynamic includes. References originating within a declared root are ignored; no cfg or
module reachability is inferred. Admission is CI-only; the Node sweep executes no source.

The optional single `generatedInputs` entry records `includeSite`, exact `expression`,
`generator`, reviewed Git `generatorBlob`, `buildScript`, `variable`, `inputDirectory`, `packageRoot`, `releaseScript` and
`reason`. Colab's canonical OUT_DIR include is admitted only with its reviewed build-script
forwarding, generator and exact release app-directory pin; other dynamic inputs fail closed.
Never-shipped roots cannot overlap its app package or generated directory. Local environment
overrides are outside canonical release attribution. Update declarations and their proofs in
the same reviewed change when packaging or embedding changes; do not add naming exemptions.

Leave open issues, PR items, other repositories and project membership unchanged.
Recompute both owned fields, correcting stale terminal states and historical text. Complete
discovery and the dry-run plan precede bounded batched mutations and a Project readback.
Each mismatching issue reports expected and observed `Status` and `Released in` fields in
the log and step summary. A mismatch triggers one correction planned from that fresh
readback against the sweep's frozen delivery/publication evidence, with request budget
reserved before correction writes. A second mismatching readback fails the gate; there
is no polling or transport retry. Reopened, missing or newly epic issues fail before
correction writes. Dry runs do not write, read back or retry.
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
actionlint .github/workflows/project-release.yml .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-prepare.yml .github/workflows/release-rehearsal.yml .github/workflows/native-release-smoke.yml
```

## Merge queue metrics

Read-only REST evidence (no GraphQL) on queue throughput, group duration, enqueue latency and
per-job failures: `node typescript/scripts/merge-queue-metrics.mjs --repo pj-tmt/tmt --since <UTC>
--until <UTC> [--boundary <UTC>] --cache <dir> --output <md> --json <json>`. Bounds are UTC, start
inclusive and end exclusive; `--boundary` compares cohorts, `--details` prints all cost rows,
`--offline` requires cached evidence. The default 500-request budget can be exhausted on windows
near 12 hours; narrow the window or pass `--max-requests N`. `--workflow FILE` defaults to `ci.yml`
and `--tag-pr N` (repeatable) marks
confounder PRs. Terminal job pages refresh once through complete acquisition before cache reuse;
unqualified offline pages cannot prove completion. API source SHA/tree stay separate from each
worker's checkout-log and immutable Git-tree proof; missing or ambiguous checkout evidence and
legacy source-only snapshots are excluded from same-tested-tree candidates. Only relevant failed
workers and subsequent successes acquire bounded logs within the REST budget. The script and
report state their methodology limits. Verify with
`pnpm exec vp test run --config vitest.config.ts test/tooling/merge-queue-metrics.test.ts` from
`typescript/`.
