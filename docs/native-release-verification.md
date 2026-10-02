# Native release verification

Task-specific contributor reference extracted from DEVELOPMENT.md. Run commands
from the repository root unless stated otherwise. Read this entire guide for
archive, installer, upgrade, bootstrap or publication work; ordinary Office and
CLI changes use the focused checks in [Development](../DEVELOPMENT.md).
This is verification guidance, not publication authorization.

## Native Rust release archives

### Explicit multi-platform release preparation

`Native release artifacts` (`.github/workflows/native-release.yml`) is the per-product
release run, dispatched with an explicit `cli`, `office` or `squad` product, not part of
every PR. It has two modes. The default `prepare` builds and verifies one bundle from the
current main commit without a draft release and attaches nothing: dispatch each authorized
product on the release's reviewed, required-checks-green main commit and record the
product, run ID and exact SHA in its issue. With `prepare` off, the run plans the
product's draft releases that carry neither a verified bundle nor a recorded failure,
oldest first, and builds, verifies and attaches each one at its own commit
(`target_commitish`), one at a time. `.github/workflows/native-release-bundle.yml` is the
pipeline it calls once per draft. The pipeline builds on native macOS arm64/x64 and Linux
arm64/x64 hosts using the existing pinned tools and `build-native-artifact.sh`. A shared
matrix keeps build and final verification hosts aligned; dispatches outside main are
skipped. Cached packaging tools are keyed by OS, architecture and exact tool versions;
they are developer tools only. Rust dependency caches are per product and target and are
written by main only.

The draft release carries the state of its own build. A draft with
`release-publication.json` has a complete bundle: the archives, the final manifest and, for
the CLI, both installers are uploaded first, their digests compared with the local bytes,
and that file last, after every final verifier passed. A draft with
`verification-failed.json` (run URL, commit, failed jobs) is parked: later runs list it in
their summary and skip it. A cancelled run records nothing and is retried. Retry a parked
draft by dispatching the run with `prepare` off and `retry` set to its tag, or delete the
draft. Each product has one queued run group (`release-<product>`); GitHub keeps one pending
run per group and replaces it, which loses nothing because a run plans from the drafts
when it starts. The run asserts that the draft tag is the tag prefix and Cargo version of
its commit, and it never creates, edits or publishes a release.

cargo-dist itself merges the downloaded `*-dist-manifest.json` inputs through
`dist build --artifacts global --output-format=json --no-local-paths`. Do not
hand-merge artifact JSON or enable another installer. Generate a complete
`dist plan` on the same source. For CLI, pass it as bootstrap `--plan`: the
generator requires exact planned archive names/targets, preventing a missing
matrix target from silently shrinking the release. Bootstrap generation verifies
TMT ownership and archive inventory/digests before generating code; the final
CLI matrix executes both existing verifiers and compares the regenerated script
bytes. Office and Squad have no bootstrap and run their product-specific
archive/runtime verifiers against the same final-manifest ownership instead. All
jobs in the selected product run must pass before that product is published,
even if the assembled artifact can already be downloaded. CI artifacts expire
in seven days. Product-qualified artifact names prevent concurrent product runs
from being mistaken for one bundle. Notices alongside each bundle are
verification inputs; every archive also contains its own target-filtered notices.

`Release` (`.github/workflows/release.yml`) runs on every push to `main`, documentation
included (a merge of any kind moves `main` under the open release pull requests), and on a
manual dispatch with `dry_run` (default on). Its `release-please` job
runs the pinned release-please CLI (`.github/release-please`, exact version and lockfile
integrity) against the generated `release-please-config.json` and
`.release-please-manifest.json`: it opens one release pull request per released component, and when
one is merged it creates the draft release (release-please's drafts, so a published release
never has to receive assets). A live run, which is only allowed on `main`, creates a GitHub
App token in that job alone, enables auto-merge (squash) on the open release pull requests,
which merge through the normal required checks, and updates the ones that fell behind `main`
(`strict` requires an up-to-date branch; a busy `main` can keep a release pull request behind
until a quiet moment). release-please runs with `always-update`, so every run also rebuilds each
open release pull request from `main`'s current files and force-pushes its branch, subject to the
[queued-PR pre-check](../DEVELOPMENT.md#queued-release-pull-requests); that, not
`gh pr update-branch`, is what clears a conflict (every release pull request edits the shared
manifest, and adjacent lines conflict). A `dispatch` job then starts the per-product run above for every
product that has a draft without a bundle. The job runs in the `release` Environment and the
App credentials, `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, are secrets of that
Environment, not repository secrets, so only a run its deployment branch rule admits can read
them; the step that decides the mode is told whether they exist, never their values. Until
both secrets exist a push is a dry run: `release-please` runs with `--dry-run` and the run
summary shows what it would open, tag and start; nothing is created. A manual run with
`dry_run` off and no secrets fails instead of falling back, and so does one on any ref but
`main`. Neither job publishes.

Owner setup, once, when the release App exists: create the Environment `release` and limit
its deployment branches to `main`; add `RELEASE_APP_ID` (the numeric App ID) and
`RELEASE_APP_PRIVATE_KEY` as secrets of that Environment; install the App on this repository
only, with Contents and Pull requests read/write and no webhook. GitHub creates the
Environment without a rule the first time the workflow names it, and the secrets are added
only after the rule exists. Once the rule exists a dispatch from any other ref, even a dry
one, is refused by the Environment. Do not add repository secrets of the same names: those
are readable from every ref.

Once a draft's bundle is attached, the run evaluates the publication gates in order:
`channel` (the version is an alpha, `X.Y.Z-alpha.N`; a stable version or any other pre-release
label such as `beta` or `rc` is held, and releasing that hold is refused: the owner publishes it
by hand), `commit` (the release's commit is on `main` and the pull request that produced it passed
`Code quality`, `Unit tests`, `Docker E2E` and `Native package matrix`), `immutability` (the
repository's newest published release is immutable, which shows that the setting was on; the
workflow token cannot read the setting itself), `monotonic` (the release is newer than every
published release of its product), `migration` (no commit of the release carries `!` or a
`BREAKING CHANGE:` footer; outside the alpha channel the component's migration list, named in
`.github/components.json`, also has no more entries than at the product's last published
release, while an alpha publishes new entries and the gate's summary only reports them) and
`upgrade` (the proof above, which for the CLI includes migrating state the previous release
wrote; the first release of a product has nothing to upgrade from). A failed gate does
not make the draft a failed build. The draft gets `publication-held.json` (`tag`, `sha`,
`gate`, `reason`, `runUrl`, `recordedAt`), and later runs list it as held and leave it alone.
The jobs that evaluate the gates hold the write token, so they run `main`'s code and only read
the release commit's data through git and the API. To release a hold once its cause is dealt
with, publish the draft by hand as below, or dispatch `native-release.yml` on `main` with the
product, `prepare` off and `hold` set to the tag: the run evaluates the gates again without
the one gate the marker names (never another, and never `channel`), removes the marker when
they pass and then publishes the draft as below.

Publication is authorized by the owner. The owner chose a trunk-based alpha channel, and that
choice is the standing authorization, recorded in the release skill, for the release pipeline
to publish an alpha draft that passes every gate above; everything a gate holds, every stable
release and every publication by hand needs the owner's explicit authorization.

When every gate passes, the `publish` job publishes the draft. `release-publish.mjs publish`
reads the draft again and refuses unless its version is an alpha, its component is released
(`release: false` in `.github/components.json` parks a component: release-please opens nothing
for it, the planner leaves its drafts alone and this command refuses them, so a draft that
predates the flag cannot publish), and it carries the bundle and neither
`publication-held.json` nor `verification-failed.json`; then one `gh release edit <tag>
--draft=false --prerelease=<bool> --latest=<bool>` applies the product's policy below. Both
flags are explicit because release-please makes every draft a prerelease. The `published` job
then reads the release back: it is public and `immutable: true`, its flags are the policy's (a
CLI release is the repository's latest release, an extension release never is), its tag is on
the release commit, it carries `release-publication.json`, and GitHub's attestation verifies
(`gh release verify`, and `gh release verify-asset` for every asset downloaded from the
published release). GitHub finishes the attestation after publishing, so these checks are
retried for about two minutes. A failed check opens an issue and fails the run; nothing is
rolled back, because a published release is immutable and a repair needs a new reviewed
version. A `smoke` job then installs the published release as a user does
(`.github/workflows/native-release-smoke.yml`, also run by hand with `product` and `tag`, for
the newest published release of the product only: it installs what the public entry points serve
now, so any other tag fails its first check and a failed run reports on the issue like any
other). On the four hosts of the upgrade proof, in an isolated home, state directory
and prefix and with no token, a CLI alpha goes through the public
`releases/latest/download/install.sh`: the installer names the tag's version, the installed `tmt`
is the one PATH selects and reports that version, the installed shared skills are the tag's
`skills/*` (same names, same `SKILL.md`), and `tmt upgrade --channel alpha --json` reads the live
metadata and reports the installation current (a newer alpha that appeared since passes with a
note). An extension alpha is installed by the newest published CLI's `tmt extension install
<extension>` into a separate prefix; `tmt extension list` must report the tag's version and no
CLI link may appear. The tag is
checked out only so its skills can be read; none of its code runs. The network steps get three
attempts, and a GitHub API rate limit that persists is reported as one (the installed CLI reads
the release list unauthenticated). A failed leg keeps its failed checks as data, and a final job
with `issues: write` comments on, or opens, the issue of the checks above; nothing is rolled
back. Both `publish` and `published` run `main`'s code and never the release commit's: `publish`
holds the write token, `published` only read access and `issues: write`; the install legs have
neither. A run that stopped before it published is completed by the
next run of the product: it plans every complete draft without a hold again and evaluates its
gates again. A bundle prepared without a draft (`prepare`) never publishes.

For a manual publication, verify the selected product run's exact commit and all required PR
checks, and enable GitHub release immutability before creating a draft release.

Each bundle carries `release-publication.json` from
`typescript/scripts/native-release-policy.mjs`; create the draft with its
`flags` (`gh release create <tag> --draft <flags> …`). The CLI release is
published as a normal release with `--latest=true`, so
`releases/latest/download/install.sh` reaches its installer; alpha status stays
in the version and title. Office and Squad releases keep `--prerelease` and
`--latest=false` and can never become latest. `tmt upgrade` accepts a CLI
pre-release published either way (earlier alphas were flagged prereleases) but
never a stable CLI flagged prerelease, and accepts an extension release only when
its flag matches whether its version is a pre-release
(`Product::accepts_prerelease_flag`). After a manual publication, check
`node typescript/scripts/release-policy.mjs --check-latest "$(gh api repos/wkh237/tmt/releases/latest --jq .tag_name)"`. A CLI
release attaches its four tar.gz archives, final `dist-manifest.json`,
`tmt-installer.sh` and the byte-identical `install.sh` (the name the one-line
install uses); an Office release uses the independent `tmt-office-v<version>`
tag and attaches its four archives and final manifest without a CLI bootstrap, and
a Squad release does the same under `tmt-squad-v<version>`. Every release also carries
`release-publication.json`, the completeness marker uploaded last: it stays on the published
release (about 100 bytes, and `tmt upgrade` selects assets by exact name and ignores it).
Verify uploaded SHA-256 digests before publishing each draft. Verify
`immutable: true`, tag commit and GitHub release attestation (`gh release verify`
and `gh release verify-asset`); the pipeline does this for its own publications. Never combine product manifests, replace an
immutable release's assets or move its tag. A repair needs a new reviewed version.

Also verify **upgrading from the last published release**, not only fresh installs.
`Native release upgrade proof` (`.github/workflows/native-release-upgrade.yml`) does this
on the four matching hosts with real bytes, and runs by hand (`workflow_dispatch`) for any
draft or published tag, from `main` only. Its `fetch` job, which holds the write token that
can see draft assets and runs `main`'s code, downloads the release's archive and manifest and
those of the newest published release of the same product below it, each checked against the
digest GitHub recorded, and hands them over as a run artifact; the read-only `prove` jobs
re-check the digests and run the scripts of the release's own commit on them. A CLI release
goes through
`verify-native-installation.mjs`: the previous archive is installed pinned, the candidate
is refused while pinned and installed with `--unpin`, the exact skills are served, SQLite is
unchanged by the installation, the old executable is preserved, a repeat is a no-op and a
downgrade is refused. It also proves the migration of state the previous release wrote
(`migrated-state.mjs`): before the upgrade the previous release writes identities (one with a
preamble, role, metadata and status) and a room they joined with a message queued to each
member, all through commands that need no tmux; once the candidate has opened that state, the
proof reads the database file and requires exactly the migrations the candidate's source lists
(the publication gate's `countMigrations`), clean `integrity_check` and `foreign_key_check`,
every id the previous release wrote still held by some table, and no table that held rows
short of them. Only ids, counts and these SQLite checks are compared, never command output.
Binding a pane needs tmux, which the proof does not use, so `bindings` and `host_servers` stay
empty and a migration that rebuilds them over rows is not exercised.
An Office or Squad release is installed over the previous one by the newest published CLI
with `tmt extension install <extension>` and read back with `tmt extension ls`
(`verify-native-extension-upgrade.mjs`): the version changes, the previous release stays on
disk, a repeat is a no-op, a downgrade is refused and no CLI link is created. Extensions
have no install command of their own under `tmt <extension>`; the proof must use the surface
a user's install runs. The first release of a product has nothing to upgrade from and says
so. A commit that predates these scripts fails the proof with that message; prove it by
hand as below. A verifier command that fails says, on one line, which command failed, how it
ended and the first thing it said. A failed host keeps its log as an artifact, and the
`conclude` job outputs, as the workflow's `reason`, the first error of each failing host on one
bounded line (it reads the logs as data, because the release commit's own scripts wrote them);
the `upgrade` hold marker carries that reason with the run URL.

The automated proof does not run `tmt upgrade` or a public installer: the candidate has no
published release for `tmt upgrade` to find, and production has no test endpoint. By hand,
in an isolated HOME and prefix, install the previous published version with its own public
installer. Then run the candidate installer's `__native-install` over that prefix, and run
the candidate's `tmt upgrade` against a receipt with that version's online
(`github-release`) provenance. Both must succeed and leave the superseded receipt
unchanged. Receipts written by older releases stay readable: v5.0.0-alpha.2 through
alpha.6 and Office 0.1.0-alpha.1 through alpha.3 record the pre-rename repository
`wkh237/tmux-team`, which receipt reading accepts as the official one (#492).

The pipeline's `smoke` job (see Publication above) runs the actual public script with an
isolated HOME, state directory and prefix and checks the version, the managed skills, PATH
selection and `tmt upgrade --json` against live immutable metadata after every automatic
publication; run it by hand for a tag published another way. Promoting README installation
instructions stays the owner's decision, and a host installation is never mutated. Record the
smoke separately from controlled-curl fixture evidence. npm publication is not part of native
GitHub release publication.

The PR smoke matrix verifies raw native runtimes, not release archives.
#135 introduced archive generation; later slices delivered installation.
The target inventory and artifact metadata live in `dist-workspace.toml` and
the generated cargo-dist manifest, not another TMT release catalog.
Select the CLI release explicitly with `--tag v<CLI-version>` for direct
`dist plan`/`dist build` calls: Office is independently versioned. The maintained
build script resolves the selected package version from Cargo automatically.

Install the pinned developer tools into a chosen tool directory (not needed by
end users): cargo-dist 0.32.0 with `cargo install --locked`, and cargo-about
0.9.2 with `cargo install --locked --features cli`. Put their binaries on PATH.
Fetch the locked workspace dependencies before the offline notice step.
Build targets sequentially in one checkout, or use separate worktrees: the
generator's distribution directory and generated notice input are per-checkout.

```sh
# Choose a target from dist-workspace.toml that matches the verification host.
native_manifest=$(mktemp)
MACOSX_DEPLOYMENT_TARGET=11.0 scripts/build-native-artifact.sh aarch64-apple-darwin > "$native_manifest"
node typescript/scripts/verify-native-artifact.mjs \
  --manifest "$native_manifest" \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

For Office, Vite generates the bundled frontend license inventory as
`target/office-spa/THIRD-PARTY-NOTICES.txt`; the artifact builder appends it to
the target-filtered Rust notices. CLI notices remain Rust-only. A missing or
empty frontend notice file fails Office packaging.

Review the generated `rust/target/native-notices/THIRD-PARTY-NOTICES.txt` against
the locked, archive-target-filtered runtime graph, including Unicode copyrights;
the verifier compares the archived notices and license with these selected
inputs and rejects placeholder attribution. A successful generator
is not legal certification. Keep the complete notice text with redistributed
binaries. The verifier bounds inputs (64 MiB compressed, 128 MiB expanded),
requires exactly the four runtime files, and removes its private staging after
success or failure. It runs the extracted executable with no Node/Rust/tmux on
PATH and verifies native SQLite persistence through public commands. macOS
requires system `otool`, which it finds once through `xcrun` under a 10 s bound;
the first `xcrun` call on a fresh hosted runner can exceed that, so every workflow
job that runs the verifier on macOS first runs `.github/actions/warm-xcrun`
(bounded retry, logs the duration). A new macOS verifier job must do the same, and
a guard test fails when one does not. Linux requires `readelf` for static-musl
linkage checks.

`typescript/test/native/artifact.Dockerfile` provides a local matching-architecture Linux
musl build and verifier. Set `TARGET_TRIPLE` from the selected generator target,
give the image a task-owned name, then run it with `--rm --init --network none`
and `--archive artifacts/<manifest archive name> --target <target>`. Remove that
owned image after verification. Emulated execution and cross-compilation alone
do not satisfy native target acceptance. This optional image is not the tmux
E2E harness or a publication workflow.

Negative archive tests use real tar fixtures and causal guard assertions.
Exercise checksum corruption, truncation, missing executable/notices, links,
unexpected paths, duplicates, bounds and cleanup; never accept any arbitrary
process error as proof of the intended check. Inspect exact manifest and archive
bytes from the final source before running the reviewed CI candidate.

## Offline native installer verification

### Native update verification

`tmt upgrade [--channel stable|alpha] [--to <version> | --unpin] [--json]`
and `tmt update` share one grammar and implementation. Use task-owned managed
prefixes, never a user's installed command or app data. The invoking executable
must be the active release; an unmanaged checkout binary fails before networking.
Production has no test endpoint or TLS bypass. API fixtures inject only the
adapter's acquisition boundary; actual local TLS fixtures use test-only trust.
Test old/new real release archives separately from synthetic tar fixtures, with
different embedded skills, to prove the newly active executable supplies refresh.

Check pinned no-network behavior, explicit pin/unpin, unchanged release identity,
preserved old bytes, missing/mutable release rejection, dual digest checks,
same-version integrity and concurrent pin fencing. Cancellation and finalization
tests must inspect active receipts and surviving executables, not only exit codes.
After activation, skill failures retain a partial report and nonzero status;
malformed/nonzero/oversized/timed-out child output is never a success. The existing
process runner retains bounded output only for completed nonzero exits and never
prints it implicitly. Verify both byte preservation and task-owned cleanup.

The synchronous HTTPS dependency is pinned ureq 3.4.0 (MIT/Apache-2.0, upstream
MSRV 1.85), selected without an async runtime or curl fallback. The lockfile and
workspace MSRV 1.95 remain authoritative for the complete graph. rustls and
platform-verifier use native trust and library proxy environment behavior.
Runtime attribution includes ISC crypto and CDLA-Permissive-2.0 certificate data;
generate target-filtered notices through cargo-about and retain complete texts.
rcgen/rustls local-server fixtures are dev-only, not production endpoint options.

The explicit actual-archive acceptance test requires separately versioned,
matching-host cargo-dist artifacts with different embedded skills. It is ignored
by ordinary tests, not counted as release proof until selected and passed:

```sh
TMT_UPGRADE_OLD_ARCHIVE=/absolute/old/archive.tar.gz \
TMT_UPGRADE_OLD_MANIFEST=/absolute/old/manifest.json \
TMT_UPGRADE_NEW_ARCHIVE=/absolute/new/archive.tar.gz \
TMT_UPGRADE_NEW_MANIFEST=/absolute/new/manifest.json \
TMT_UPGRADE_TARGET=aarch64-apple-darwin \
cargo test --locked --manifest-path rust/Cargo.toml -p tmt-adapters \
  cargo_dist_upgrade_refreshes_real_artifacts_and_preserves_conflicts -- --ignored
```

Require one selected passing test, not an empty filtered run. This test injects
canonical acquisition responses but executes real old/new binaries, managed
skill installation, partial hidden refresh and repair in scrubbed task-owned
state. It does not claim to contact a public release or exercise the public CLI
over a fake production endpoint. Run the independent artifact verifier too.

### Offline composition

The internal entrypoint consumes a local cargo-dist archive and manifest. It is
not advertised by help/completion and does not change `tmt install` skill syntax.
Use a task-owned prefix and matching host archive, never an existing user install:

```sh
native_prefix=$(mktemp -d)
rust/target/debug/tmt __native-install \
  --archive target/distrib/tmt-cli-aarch64-apple-darwin.tar.gz \
  --manifest "$native_manifest" --prefix "$native_prefix" --channel alpha --json
"$native_prefix/bin/tmt" --version
```

`--pin` pins the selected candidate, `--unpin` clears an existing pin, and omission
preserves its state. Neither authorizes a downgrade. Repeated exact artifacts are
no-ops after ownership validation; metadata-only changes activate a new receipt
with the same payload. Receipts record local-archive provenance, not authenticated
public release provenance. Keep application state isolated separately when running
identity/profile commands; installing the executable must not open a database.

The internal `--product office` selector uses the same offline verifier/publisher
for a `tmt-office` package and executable. Omission selects the CLI unchanged.
Office installs under `lib/tmt-office` with only `bin/tmt-office`; it must not
modify CLI links, receipts, application state or managed skills. Verify coexistence,
cross-product rejection and interruption in isolated prefixes. Copied CLI binaries
in synthetic Office test archives prove installation behavior only, not an actual
Office companion, protocol compatibility or public distribution. The native
process suite now separately copies the compiled `tmt-office` into synthetic
archives and exercises its exact versioned probe. Build the workspace first;
a missing companion is an error, never a fallback to the CLI. This additional
evidence does not replace real cargo-dist archive and distribution acceptance.

### Office archives

The local Linux artifact Dockerfile accepts `--build-arg PRODUCT=office` with
the matching `TARGET_TRIPLE`. Pass `--product office` and the corresponding
Office archive to its verifier entrypoint. The default remains CLI; both use
the same generator, notice owner and independent verification path.

Generate an actual matching-host Office archive with the same toolchain and
target policy, selecting Office's runtime dependency notices:

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin office > /absolute/office-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product office \
  --manifest /absolute/office-manifest.json \
  --archive target/distrib/tmt-office-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

Build products sequentially and retain their manifests, archives and generated
notices separately before another build overwrites distribution output. The
same bounded independent verifier checks inventory, hashes, notices and linkage;
Office runtime proof requires its exact probe without creating application state,
not CLI-only skill/SQLite commands. Follow with `office install --yes --archive
<archive> --manifest <manifest> --prefix <task-owned-prefix>`, status, repeat
installation and explicit uninstall. Inspect surviving bytes after rejected
candidates and deactivation. Public availability is a separate authorized gate;
local cargo-dist's package selection tag does not publish a Git tag. Public Office
discovery uses `tmt-office-v<version>`; select `office` explicitly when dispatching
the shared release workflow and never publish its bundle under a CLI tag.

### Squad archives

A Squad archive adds one directory to the runtime files: `skills/`, copied from
`extensions/tmt-squad/skills/` by the package's cargo-dist `include` (a package
list replaces the workspace list, so it repeats the shared files). cargo-dist
declares that directory as the single manifest asset `skills`; the installer
inventories its files from the checksum-verified archive. Build and verify it
like Office, passing the skill sources for a byte-for-byte comparison:

```sh
scripts/build-native-artifact.sh aarch64-apple-darwin squad > /absolute/squad-manifest.json
node typescript/scripts/verify-native-artifact.mjs --product squad \
  --manifest /absolute/squad-manifest.json \
  --archive target/distrib/tmt-squad-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skills extensions/tmt-squad/skills \
  --notices rust/target/native-notices/THIRD-PARTY-NOTICES.txt --license LICENSE
```

The runtime proof checks `tmt-squad --version` and that `tmt-squad skill show`
prints the archived `SKILL.md`, with an empty HOME, config and working directory
afterwards. The local Linux Dockerfile takes `--build-arg PRODUCT=squad`; pass
`--product squad --skills expected-squad-skills` to its entrypoint. Follow with
`tmt extension install squad --archive <archive> --manifest <manifest> --prefix
<task-owned-prefix> --channel alpha --yes`, a repeat install and `tmt extension
uninstall squad`. Squad's closure adds the Zlib license (`foldhash`), accepted in
`rust/about.toml`; the CLI and Office notices do not change.

Native adapter tests cover bounded archive acquisition and publication failures;
native process contracts use the existing executable selector and sandbox. Test
current/receipt tampering, manager collisions, pin changes, interrupted staging,
lock contention, missing command-link repair and retained previous releases.
Assert surviving bytes and cleanup, not merely a failed exit. Tests comparing
large executable buffers must use exact `Buffer.equals`
checks rather than structural object matchers: enumerating every byte can exhaust
the test runner's heap on debug binaries. Verify the comparator detects a changed
byte; do not replace byte equality with a size-only assertion or raise CI memory
limits to hide assertion overhead. Installation cases also use an explicit
15-second subprocess budget for debug
archive hashing/decompression and durable publication, distinct from the ordinary
CLI's five-second test budget and each scenario's 60-second cap. Keep ordinary
command and production timeout policies unchanged; validate installation budgets
under constrained local resources rather than treating CI as a timing probe.
Synthetic filesystem
fixtures are not proof of runnable release artifacts: retain separate actual
cargo-dist archive execution and target/linkage/notice evidence. Run the shared
skill-installation regressions when changing shared file locks or content digests.

For actual old-to-new acceptance, build two separately versioned cargo-dist
archives in task-owned source copies. Do not alter the repository release version
or publish fixtures merely to test updates. The newer archive must contain the
offline installer; the older artifact must run its real reported version. Verify
each archive's notices/linkage with the artifact verifier above, then run:

```sh
node typescript/scripts/verify-native-installation.mjs \
  --previous-archive "$previous_archive" --previous-manifest "$previous_manifest" \
  --archive "$next_archive" --manifest "$next_manifest" \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

This reuses the bounded packed-command runner and independent archive verifier.
It checks exact old/new versions with no runtime PATH, pinned rejection, explicit
unpin advancement, a retained executable, no-op and downgrade rejection, exact
embedded skill, unchanged SQLite bytes during installation and the migration of state the
previous release wrote. Its temporary
prefix/application state is always invocation-owned and removed afterward.

## Native curl bootstrap verification

Generate the release-specific script only after final cargo-dist archives and
their independent runtime verification. All selected archives must be present;
the manifest, not a hand-maintained version table, owns the generated facts:

```sh
node typescript/scripts/generate-native-bootstrap.mjs \
  --manifest /absolute/dist-manifest.json --archive-dir /absolute/artifacts \
  > /absolute/artifacts/tmt-installer.sh
sh -n /absolute/artifacts/tmt-installer.sh
node typescript/scripts/verify-native-bootstrap.mjs \
  --manifest /absolute/dist-manifest.json \
  --archive /absolute/artifacts/tmt-cli-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --skill skills/tmux-team/SKILL.md
```

The last command requires actual matching-host release artifacts, uses isolated
HOME/state/PATH, real shell utilities and the new native executable. Only curl
acquisition is replaced with task-owned fixture copies; production has no test
endpoint. It checks repeat/no-op, explicit pin followed by no-network upgrade,
exact installed skill-bundle bytes, old npm command preservation, PATH warning and
temporary cleanup. It is not live GitHub download or cross-target evidence.

The existing `typescript/test/native/artifact.Dockerfile` also carries this verifier. After
building its task-owned matching-architecture image, run the normal artifact
entrypoint, then repeat with `--entrypoint node` and
`typescript/scripts/verify-native-bootstrap.mjs --manifest native-manifest.json --archive
artifacts/<archive-name> --target <target> --skill expected-skill.md`. Keep
`--rm --init --network none` and remove only the task-owned image afterwards.

`typescript/test/tooling/native-bootstrap.test.ts` covers the generated shell's negative paths with
the existing bounded CLI sandbox and a synthetic executable for orchestration.
Do not count that stub as native publication evidence; pair it with the actual
artifact verifier. Run `(cd typescript && corepack pnpm check)`, unit tests and
two Docker lifecycle passes before the reviewed CI candidate. Keep generated
outputs out of source control.
No new runtime dependency, data migration or public publication is authorized
by generation. An authorized release must upload the exact verified manifest,
archives and generated script, establish immutable release/provenance evidence,
and verify the public download before the README advertises it as available.
