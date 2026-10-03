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
- The native version is owned by `rust/Cargo.toml` and exposed through Cargo's package version; there is no TypeScript fallback. Keep any retained developer package version and public release instructions consistent when changing versions. The native skill ships with the CLI; there is no separately versioned plugin or marketplace.
- Follow `AGENTS.md` for GitHub issue state, branch and pull-request links, verification evidence, and safe worktree cleanup.

## Private-leaf attribution

The component map's `releaseConsumers` currently attributes private TUI changes to Squad.
The release workflow's small `release-please-run.mjs` wrapper adds only in-memory consumer paths
before release-please's splitter and cutoffs; 17.11.2 has no `additional-paths` config option.
Keep its pinned API shape verified by tooling tests loading the release job's isolated install.
Follow DEVELOPMENT's generator and real-candidate checks before changing this consumption rule
or upgrading release-please. This does not change publication authorization or private-leaf version ownership.

## Release PR safety

Release PRs must pass `Code quality`'s notes gate before merge: compare from the
component's newest published tag, with every linked commit inside its ancestry
range through the candidate base. COVERAGE requires a link for every commit the
pinned release-please renderer lists for the component in that range. Use its
parser, path splitter, exclusions and private-leaf attribution with the
candidate-base config's changelog sections or pinned defaults; do not introduce
a second visible-type list or entry-count policy. Missing links hold the merge
group until release-please refreshes the notes on a main push.
The cumulative merge-group selector
keeps existing locked Cargo workers selected for earlier pending release changes.
A visible matching manifest draft without its git tag holds only that component’s
release PR candidate; unheld components regenerate normally. Only all-held
manifest paths skip `release-pr`; `github-release` and draft processing continue. Missing or inconsistent evidence
fails closed. [DEVELOPMENT's safety gates](../../../DEVELOPMENT.md#release-pr-safety-gates)
own draft-token visibility, bounded REST discovery, fixtures and recovery procedures. Neither gate
authorizes manual tagging, release editing or publication.

## Release queue robustness

The release workflow enables one same-repository main release PR at a time. Keep the
queue pre-check's `skip`/`run`/`blocked` interface and its live-only `enable` command in
the single queue owner; complete paginated discovery and head-pinned enabling must fail
visibly on uncertainty. Skip a queued release PR only when `checkReleaseNotes` accepts
its notes against main HEAD; keep coverage in that safety owner. Invalid compare anchors, out-of-range links or missing COVERAGE links require one
bounded live dequeue with the release App token, after rechecking the PR identity,
head and queue entry, before release-pr refreshes the notes. Dry runs never dequeue.
Tagless-draft-held candidates retain their queue entry and existing generation filter.
Failed or unverified dequeue writes a recovery summary and blocks release-pr and
queue enabling while github-release and downstream draft processing continue;
initial acquisition errors still fail visibly. Fetch full history and tags with Code quality's
checkout pattern. Do not enable another component while a release PR is enabled
or queued. Existing workflow concurrency serializes this policy, not external enqueues.

Keep `always-update` for conflict recovery and the pinned update wrapper's unchanged
release-content comparison for CI continuity. BEHIND alone does not require a branch
refresh: the merge queue runs required checks against current main's merged result.
Changed release content still needs fresh checks; no queue priority jump is used.
[DEVELOPMENT's queue section](../../../DEVELOPMENT.md#queued-release-pull-requests)
owns request bounds, failure and recovery details. Tooling tests must cover pagination,
single-active selection, queued covered/stale notes against main HEAD, dequeue-before-refresh ordering,
failed dequeue with continued github-release and suppressed queue enabling, identity/head races, dry-run non-mutation, unchanged generated files and original conflict/update behavior.

## Release stall monitoring

Keep advisory stall detection separate from required release gates. The pinned
manifest remains the releasability owner; a monitor must not close its issue on
incomplete evidence or mutate held release PRs. Keep `issues: write` for monitoring
in its separate job; its REST uses `github.token`. Only the existing
release-job App reader sees drafts, passing metadata rather than credentials.
Distinguish current published-release smoke infrastructure issues from real check failures;
a rate-limit issue recommends retrying smoke, never publication. Preserve zero-failure behavior,
visible summary warnings and fixture-only REST tests. [DEVELOPMENT’s monitor
section](../../../DEVELOPMENT.md#release-stall-monitoring) owns thresholds,
credentials, bounded discovery and the single-issue recovery lifecycle.

## Conventional PR titles

Merge groups report conventional squash-title syntax through the shared safety
owner. The report-only phase writes findings and unavailable evidence to job
output/summary and always exits zero; it does not enforce titles yet. Do not add
an `edited` trigger to full CI or compare ordinary queued subjects against mutable
REST titles. [DEVELOPMENT's rollout](../../../DEVELOPMENT.md#conventional-pr-title-rollout)
owns the observation day and the separate explicit UTC cutover, 24 hours after
the report-only PR merges. Keep release-please as the release attribution and
changelog owner.

## Promotion and prerelease checks

Read the complete [native release verification section](../../../DEVELOPMENT.md#native-release-verification)
before archive, installer, upgrade, bootstrap or publication work. It owns the
procedures referenced below; DEVELOPMENT owns ordinary native checks.

- For Rust archives, follow the guide's native Rust release archive procedure.
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
  marked latest (the README's `releases/latest/download/install.sh` depends on
  it); Office and Squad releases stay prereleases with `--latest=false`. After
  a manual publication, run the guide's `--check-latest` check (the pipeline checks
  its own publications). Do not equate a
  downloadable CI bundle with a published or accepted release.
- Every CLI or extension release also passes the guide's upgrade from the last
  published release, not only a fresh install. Old receipts must stay readable.
  The pre-publication CLI proof requires installation, migration and real-archive
  acceptance of the release's own adapter on all four hosts. Follow the guide's
  distinction between injected acquisition, skipped differential skill coverage for identical text,
  older-source rerun applicability and separate public installer/upgrade smoke.
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
  x64/arm64 with a static musl runtime. A deployment target is not testing on every
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
  normal release marked latest; Office and Squad alphas as prereleases with `--latest=false`,
  as the bundle's `release-publication.json` says.
- Still the owner's explicit authorization: stable releases and anything outside the alpha
  channel; a release from a branch line; a draft that any gate holds, and in particular a
  breaking change (a `!` or `BREAKING CHANGE:` commit), which always pauses for the owner's
  explicit OK;
  README installer promotion; creating or rotating the release App credentials and the
  `release` Environment (the owner's setup is in the guide's release-please section);
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
  real failure there is reported on the same issue. Smoke remains unauthenticated. Retry only
  the native classified rate-limit diagnostic, within DEVELOPMENT's attempt/reset-wait
  bounds; exhausted rate limits keep a failed job with a separate infrastructure issue,
  while any real or mixed failure keeps the release-failure conclusion. Keep the separate
  bounded latest-installer lag retry for an older alpha, and pin the consumed native
  diagnostic format in fixture tests. Never dispatch
  publication to recover a public smoke rate limit.
