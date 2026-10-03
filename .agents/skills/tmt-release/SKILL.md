---
name: tmt-release
description: Maintain and promote tmux-team release lines, versions, and prerelease readiness without assuming publish authorization.
---

# tmux-team release management

Use this skill for release-line maintenance, v4 compatibility fixes, v5 promotion, version synchronization, or prerelease readiness in this repository.

## Long-lived branch policy

- `main` is the active v5 line after promotion.
- `v4` is the maintenance line rooted at commit `7056679dfa816a1acef8e7c978cf1733a578115b`.
- If remote `v4` does not exist, create it at that exact anchor only when the user has explicitly requested the maintenance line, then verify the remote ref before continuing.
- Before any branch mutation, verify the relevant remote refs and ancestry. Never force-push or repoint a long-lived line.
- A v4 maintenance fix requires a tracked issue, a dedicated branch and worktree, and a reviewable pull request. Keep the fix on the v4 line unless an explicitly scoped backport is requested.
- Use the checks available on the v4 line for maintenance pull requests; do not require contexts that the target branch cannot produce. Record any coverage gap in the issue.
- The CLI version is owned by `rust/Cargo.toml` and exposed through Cargo's package version; there is no TypeScript fallback. Keep any retained developer package version and public release instructions consistent when changing versions. The native skill ships with the CLI; there is no separately versioned plugin or marketplace.
- Follow `AGENTS.md` for GitHub issue state, branch and pull-request links, verification evidence, and safe worktree cleanup.

## Main cut authorization

The [architecture](../../../ARCHITECTURE.md#main-release-cuts) owns the release
model; the [main-cut reference](references/main-cuts.md) owns its procedure.
Automatic cuts and publication cover only released products on their existing
alpha core version. Stable releases, major/minor changes, breaking releases and
any manual publication require the owner. An explicit version on a cut requests a
draft and native verification; it never overrides a publication gate.

A first alpha also needs the product owner's authorization and reviewed activation
under #1418. The CLI/Squad switch grants no parked-product activation. Keep hold,
retry and rerun authorization below, required merge checks, every publication
proof and immutable releases intact. Missing draft visibility or history requires
investigation, never an incomplete allocation catalog.

Independent tags never wait for a prior component draft or pipeline. Failed drafts
stay unpublished without holding later cuts; retries, hold releases and reruns
remain owner-authorized operations on one exact tag. Alpha publication requires a
unique unpublished tag/version; stable publication retains the existing monotonic
gate. CLI alphas keep `prerelease=false`: every CLI publication initially uses
`latest=false`, then bounded read/correct/readback rounds converge latest to the
highest published CLI version. A stale publisher corrects again to the current
maximum; disagreement after the bound fails visibly. Extensions remain prereleases
with `latest=false`. Public smoke proves `releases/latest/download/install.sh` for
the highest CLI, or the versioned installer for an older out-of-order cut. Managed
alpha discovery must still choose the highest eligible version. Upgrade proofs use the newest published version
**below** the candidate, preserve candidate > previous and downgrade rejection,
and retain every four-host artifact/installation/adapter proof. Notes, migration comparison and breaking authorization share one boundary: the
newest **published** ancestor release of the component. Failed or running drafts
reserve numbers but never advance that boundary. Concurrent cuts can therefore
repeat note items; inherited unpublished migrations remain counted and inherited
unapproved breaking changes stay held by the existing owner gate.

Cut allocation runs hourly (minute 17 UTC) or by owner dispatch, never on every
main push. The hourly schedule replaces the daily allocation safety net. New work
eligibility uses the newest allocated ancestor cut, whether draft or published:
no releasable component commit after it means no new cut. Failed drafts reserve
their content and number without blocking subsequent new component work. Notes,
migration comparison and breaking authorization retain the published boundary
above. Allocation remains a short serialized critical section; each admitted run
captures current main and preserves unallocated work. Once allocated, tag-keyed
pipelines never displace another tag's pending pipeline.

Non-publishing `prepare` rehearsals and installation fixtures derive
`<committed major.minor.patch>-alpha.999999` through the single
`release-versions.syntheticAlphaVersion` helper. This deliberately synthetic
version is never a real cut or publication target. Preparation keeps the draft
tag empty, creates no Git tag, and runs every source, archive and installation
gate against the injected version. It adds no dispatch input and authorizes no
product activation. Follow the [installation-fixture procedure](references/installation-fixtures.md).

The private release tool's mechanical source/lock proof grants no publishing
permission. Do not commit injected versions back to main, create tags early,
replace public assets or replay publication to recover a smoke failure.

## Conventional PR titles

Merge groups report conventional squash-title syntax through `pr-title-check.mjs`. The report-only phase writes findings and unavailable evidence to job
output/summary and always exits zero; it does not enforce titles yet. Do not add
an `edited` trigger to full CI or compare ordinary queued subjects against mutable
REST titles. [DEVELOPMENT's rollout](../../../DEVELOPMENT.md#conventional-pr-title-rollout)
owns the observation day and the separate explicit UTC cutover, 24 hours after
the report-only PR merges. The cut planner owns release notes and attribution.

## Project release reconciliation

Keep delivery evidence separate from publication: the Project updater derives
closed-issue fields from merged closing PRs, changed-path product ownership and
the earliest published containing tag for every affected product. Use the existing
component map and release policy/version helpers, never notes or a recency window.
Every sweep is authoritative for eligible issues, including recovery from built-in
status workflow writes. Exclude epic trackers from both fields; their owning lead
retains the acceptance/dogfood gate, and the summary lists them as skipped. The post-publication dispatch waits for read-back and smoke completion;
authenticated acquisition errors remain failures, with the daily sweep retaining publication reconciliation.
Retain the daily safety net. Follow [DEVELOPMENT's Project release tracking
procedure](../../../DEVELOPMENT.md#project-release-tracking) for full dry-run table
review, request and GraphQL point-cost reporting, exact verification commands and
activation evidence.
A tracking dispatch never authorizes publication or a publishing-workflow replay.

## Promotion and prerelease checks

Read the complete [native release verification section](../../../DEVELOPMENT.md#native-release-verification)
before archive, installer, upgrade, bootstrap or publication work. It owns the
procedures referenced below; DEVELOPMENT owns ordinary native checks.

For packed verifier process-runner changes, follow DEVELOPMENT's
[packed cleanup checks](../../../DEVELOPMENT.md#packed-verifier-process-cleanup).
Preserve its real absence and surviving-group controls; synthetic fixture success
does not authorize publication or replace artifact acceptance.

- For Rust archives, follow the guide's native Rust release archive procedure.
  Generate the offline clarification config before calling cargo-about directly,
  as documented there; keep its vendored-license checksum and version checks,
  `--fail` and the archive verifier's placeholder rejection intact.
  Keep cargo-dist's manifest as the artifact metadata owner; independently verify
  bounded extraction, notices, linkage, skill installation and persisted state.
  Raw PR runtime checks do not establish release archive correctness. Do not enable a
  generated installer or publication workflow merely to obtain local archives.
- Follow DEVELOPMENT's native runtime checks and the guide's archive verification for artifact changes.
  For fixture-only archive-policy fixes, follow DEVELOPMENT's negative archive checks
  for hard-link construction and rejection controls.
  Reuse the shared runtime proof for linkage, exact embedded skills and SQLite
  reopen behavior. Keep the independent archive inventory/checksum/notices and
  installer failure/cleanup evidence; raw binaries are not release artifacts.
- For native binary publication changes, also follow the guide's offline
  installer lifecycle procedure using actual separately versioned archives.
  Keep ownership anchored in the installation prefix, not application-state
  selectors; verify old executable preservation, pin policy, partial command-link
  finalization and unchanged data. The internal preview entrypoint is not a
  public bootstrap or permission to replace a user/package-manager installation.
- Promotion requires passing Code quality, Unit tests, and Docker E2E checks.
- For a public native alpha, follow the guide's explicit multi-platform
  release preparation procedure. A `prepare` run of the manual artifact workflow never
  publishes;
  all four final native verifiers must pass on the recorded reviewed commit.
  Keep cargo-dist as the merged manifest owner. Authorized publication uses an
  immutable draft-to-published GitHub release and verifies its attestation and
  public installer before promoting README instructions. Publish with the
  bundle's `release-publication.json` flags: the CLI release is a normal release
  whose latest selection follows the main cut authorization contract above;
  Office, Squad and Herdr releases retain their product prerelease policy and never
  become latest. After
  a manual publication, run the [publication readback](references/main-cuts.md#manual-publication-readback) (the pipeline checks
  its own publications). Do not equate a
  downloadable CI bundle with a published or accepted release.
- Every CLI, extension or driver release also passes the guide's upgrade from the last
  published release using the product-specific proof in DEVELOPMENT, not only a
  fresh install. Old CLI/extension receipts must stay readable.
  Synthetic extension-upgrade tests use the private native recording driver on
  every platform. Follow the [fixture build contract](references/installation-fixtures.md#native-recording-driver)
  before running them; preserve exact architecture admission and public-command
  assertions rather than substituting a shell executable.
  The pre-publication CLI proof requires installation, migration and real-archive
  acceptance of the release's own adapter on all four hosts. Follow the guide's
  distinction between injected acquisition, skipped differential skill coverage for identical text,
  older-source rerun applicability and separate public installer/upgrade smoke.
  A standalone driver uses previous/candidate archives and the current published
  CLI's path approval surface. Herdr remains parked; product activation and its first alpha belong to #1418
  and require owner authorization.
- For curl bootstrap, follow the guide's native curl bootstrap verification.
  Generate from final verified cargo-dist artifacts and invoke the existing
  native publisher; do not enable a competing stock installer. Test an actual
  matching-host archive without Node/Rust on runtime PATH, and distinguish
  controlled-download evidence from an authorized public release smoke test.
  npm/pnpm replacement is a fresh installation without data-transfer machinery,
  not permission to delete old state or silently uninstall another manager.
- Tags, GitHub Releases, npm publishing, and npm dist-tags are separate operations that require explicit authorization; this skill never assumes permission for them. The one standing authorization is the release pipeline's alpha publication below.
- Update user-facing installation or channel documentation whenever a version change would make it inaccurate.
- The v5 root npm package is private developer tooling, not a product distribution.
  Do not restore npm publishing or a download wrapper without a separately scoped
  distribution decision. Historical v4 publishing uses that branch's own rules.

## Archive contents and install facts

Colab's single-executable packaging route is prepared, with its app embedded by
the Colab-owned `TMT_COLAB_APP_DIR` build boundary and frontend notices appended
to Rust notices. Embedding (#1421) and core registration (#1423) are implemented.
It remains parked: a published supporting CLI alpha and actual-archive acceptance
precede separately authorized activation. Follow [Colab packaging verification](../../../DEVELOPMENT.md#colab-packaging-wiring-parked)
for fixture-only proof versus real archive/public-install evidence; do not treat
a tiny embedded-app fixture as delivery of the Colab product.
For proof/fixture changes, run `colab-runtime-proof.test.ts` and the complete
`verify-public-install.test.ts` through the tooling Vitest config, with the native
CLI/Herdr prerequisites and built `tmt-test-support` `colab-runtime-fixture` example.
Use absolute `TMT_TEST_COLAB_FIXTURE` for a separate Cargo target. Startup errors
wait for child `close` and drained stdio under the existing deadline/stream limits;
the regression holds the immediate native diagnostic until `exit` and checks
classification and state cleanup. Accepted HTTP fixture sockets use blocking I/O
with read/write timeouts, including on Darwin, which inherits the listener's
nonblocking flag. The native-accept/header barrier covers premature peer close/EPIPE.

- Every product archive (CLI, Office, Squad) carries its executable, `LICENSE`,
  `NATIVE-INSTALL.md` and `THIRD-PARTY-NOTICES.txt`. The installer enforces this
  inventory (`tmt-core`'s `native_install/product.rs`), so adding, renaming or
  dropping an entry is an installer-contract change with an upgrade proof, not a
  documentation edit. The CLI release may also carry optional companion executables.
- The archive's `NATIVE-INSTALL.md` is sourced from `rust/archive/NATIVE-INSTALL.md`
  through `dist-workspace.toml` and the Squad package include. Keep it a short,
  product-neutral offline note without version numbers: user guidance belongs to the
  handbook, and the onboarding test runs the note's PATH block in Bash and Zsh.
- Release targets are macOS x64/arm64 (build deployment target 11.0) and Linux
  x64/arm64 with a static musl runtime. macOS x64 follows DEVELOPMENT's
  [runtime acceptance policy](../../../DEVELOPMENT.md#runtime-smoke-matrix):
  cross-build on arm64, complete Rosetta verifier process trees with exact
  installed-byte architecture checks, plus weekly native Intel public
  installation and upgrade coverage. A deployment target is not testing on every
  macOS version; cite the release's verification evidence for tested hosts.
- The manifest's SHA-256 checksums detect corruption, not a compromised download
  origin. Locally generated checksums are not signatures, and no local test artifact
  carries a GitHub attestation. Only a published immutable release does
  (`gh release verify`, `gh release verify-asset`).
- The generated `install.sh` fixes the initial version and channel (never a mutable
  tag). It verifies the manifest and archive sizes and digests before it runs the
  temporary binary, then delegates permanent writes to the native installer. It does
  not edit shell profiles or touch SQLite, and it needs only a POSIX shell, curl,
  tar/gzip, standard utilities and `sha256sum` or `shasum`. Its receipts record
  local-archive verification, not independent attestation provenance.
- Successful human installer output names `<requested-prefix>/bin/tmt`, matching the
  bootstrap summary even when a prefix ancestor is a symlink. Installation
  validation, receipts and JSON reports keep canonical paths (#1098).

## Automated alpha publication

The owner chose a trunk-based alpha channel: there is no separate edge channel, and a merge to
`main` publishes an alpha release through the release pipeline once its publication gates pass
(the owner's decisions on #497). That choice is the owner's standing authorization for **the
pipeline** to publish alpha releases from `main`; it is recorded here so that the written rule
matches practice. The
[native release verification section](../../../DEVELOPMENT.md#native-release-verification) owns the
gates, the markers and the procedures; this section owns who may publish what.

- Covered: an alpha draft of the CLI, Office or Squad (a version `X.Y.Z-alpha.N`, enforced by
  the `channel` gate and again by the publish command) that the pipeline built from `main`,
  verified and attached, of a component that is released (`release: false` in the component map
  parks one), and that passes every publication gate. A new SQLite migration does not hold an
  alpha: migrations are forward-only, and the `migration` gate only reports the new entries in
  its summary. The CLI alpha is published as a
  normal release; its latest convergence follows the main cut contract above.
  Office and Squad retain the bundle's prerelease policy and never become latest.
- Still the owner's explicit authorization: stable releases and anything outside the alpha
  channel; a release from a branch line; a draft that any gate holds, and in particular a
  breaking change (a `!` or `BREAKING CHANGE:` commit), which always pauses for the owner's
  explicit OK;
  README installer promotion; creating or rotating Project release App credentials,
  changing the main-only `release` Environment (the owner's setup is in the guide's main-cut section);
  enabling or changing release immutability; and this authorization itself.
- The authorization belongs to the pipeline, not to an agent. An agent still never tags,
  creates, edits or publishes a release by hand, and never dispatches a run that publishes,
  without the owner's explicit authorization for that release. A run of `native-release.yml`
  with `prepare` off publishes every draft of the product that passes its gates, and so does
  its `hold` input for the released draft; `prepare` on (one bundle, no draft), `release.yml`
  with `dry_run` on and the upgrade proof are not publication.
- A held draft carries `publication-held.json` with the gate, the reason and the run. Read it,
  then follow the guide: the owner publishes by hand, or releases the hold by dispatch, which
  skips only the gate the marker names. An owner-authorized `rerun` instead re-proves
  every gate with current main tooling against the draft's existing assets and its
  release-source expectations and adapter code, preserving the marker on failure
  and removing it only after all pass.
  `rerun` requires the owner's explicit authorization, like `hold`.
- After it publishes, the pipeline reads the release back (public, immutable, the policy's
  flags, the tag on the release commit, GitHub's attestation for the release and every asset).
  A failed check opens an issue and fails the run; nothing is rolled back, and a repair is a new
  reviewed version. A read-only smoke then installs the published release through the public
  installer (and `tmt upgrade` for the CLI) in an isolated environment on the four hosts, and a
  real failure there is reported on the same issue. The shared public-install-smoke
  action uses the workflow's `contents: read` `GITHUB_TOKEN` for native API acquisition.
  Pass it through env only, never argv or disk; redact output and verify isolated
  installed state contains no credential. Asset downloads remain public and token-free.
  Preserve the native bounded HTTPS retry and the bounded older-alpha latest-installer
  lag retry. Exhausted rate limits are ordinary failed smoke checks; there is no
  deferred rate-limit retry workflow. Use the PR-only four-host proof against existing
  published releases for tooling changes, never a publishing dispatch. Keep the
  post-publication Project dispatch gated on successful smoke and historical failure issues
  visible to the release monitor. Never dispatch publication to recover a smoke failure.

## CLI upgrade proof

`native-release-policy.mjs::upgradeSupportFloor` declares the exact published CLI
floor (alpha.36). For candidates above it, `release-upgrade.mjs` stages both the
floor and last published source, deduplicating identical sources, and the candidate
`install.sh`; every archive, manifest and bootstrap must retain its recorded
GitHub digest. Candidates at or below the floor keep their historical single-source proof.

Candidate `native_install/handoff.rs::VERSION` owns probe applicability. Protocol-1
sources require the exact successful candidate probe in
[the handoff contract](../../../contracts/native-install-handoff-v1.md); malformed,
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
