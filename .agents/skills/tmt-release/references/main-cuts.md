# Main release cuts

The [architecture](../../../../ARCHITECTURE.md#main-release-cuts) owns the cut and source
contract; the [release skill](../SKILL.md#main-cut-authorization)
owns authorization.

On the hourly schedule (minute 17 UTC) or a manual dispatch, `release.yml` captures main HEAD, complete
draft/tag allocation metadata and the component map/Cargo graph at that cut. Inspect
its summary and `release-cut-plan` artifact for proposed tag, notes, linked SHAs,
cut and skip reason. Missing draft visibility, pagination or history is a
blocked plan. Allocation is serialized; inspect each tag-specific native pipeline
independently. Allocate only when the component has releasable commits after its
newest allocated ancestor cut (draft or published). Failed drafts stay unpublished
without blocking new component work; unrelated or non-releasable main changes do
not repeat their content. Notes, migrations and breaking authorization still cover
the newest published ancestor through the captured main commit.

For an owner-authorized explicit version, dispatch `release.yml` on main with
`product=cli|squad`, `version=<canonical stable or alpha version>` and
`dry_run=true` first. Review the exact cut, notes, tag and native holds before the
owner chooses `dry_run=false`. Without a version, released alpha products advance
their current prerelease number. The draft starts tagless at the captured main
commit; native-release creates the immutable tag only after all publication gates.
A stable, breaking or held release still needs the owner's existing authorization.
Parked products and their first alphas remain #1418; this procedure activates none.

Keep the `release` Environment restricted to main. The cut job uses its scoped
workflow token for draft visibility, creation and native dispatch; it requires no
release-PR App token. Normal primary review, pinned-head checks and queue protection remain required.

The persistent `release-version-injection.yml` PR check proves CLI and Squad on
four native hosts. It reuses `.github/actions/inject-release-version`, fetches
locked dependencies, captures the source/version contract, proves full locked
metadata rejects a changed-version stale lock, updates only implied entries
offline, then verifies the source, dist plan/build and extracted binary. Tagless
preparation uses the shared synthetic version and retains the same gates.
Already-versioned reruns require zero source/lock changes.
`--no-deps` inheritance discovery cannot replace full offline locked verification.
Review `release-injection-<product>-<target>` artifacts and job summaries. No release
secrets or publication privileges enter PR jobs. The runtime producer transfers
the private TOML binary separately for ordinary tooling fixtures.

```bash
(cd rust && cargo build --locked -p tmt-release-tool --bin release-version && cargo test --locked -p tmt-release-tool --bin release-version)
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/release-cut.test.ts test/tooling/release-cut-live.test.ts test/tooling/release-version-injection.test.ts test/tooling/ci-scope.test.ts test/tooling/release-workflow.test.ts test/tooling/repository-layout.test.ts)
(cd typescript && corepack pnpm check:tooling)
actionlint .github/workflows/release.yml .github/workflows/release-version-injection.yml .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-upgrade.yml
```

Historical notes comparisons in `test/fixtures/release-cut-history.json` are
immutable inputs from public REST releases and Git objects. They retain historical
release PR parent cuts for CLI alpha.44→45/45→46 and Squad alpha.12→13; production
never maps tags through those parents.

For authorized local proof, follow the shared-host build/disk rules and use one
Cargo target. Build `tmt-release-tool` first; Node finds its `release-version`
binary at that target's `debug/` (or default `rust/target/debug/`). Run
`release-version-injection.mjs prepare <checkout> <snapshot-outside-checkout>
<product> <tag>`; an empty tag selects the shared non-publishing preparation version. When
versions differ, demonstrate stale-lock rejection with full `cargo metadata
--offline --locked`, then `cargo update --offline --workspace` and `verify
<checkout> <snapshot>`. Do not rewrite an already-versioned lock. After assembly,
`manifests <checkout> <snapshot> <plan.json> <build.json>` checks the metadata; after
archive extraction, `artifact <checkout> <snapshot> <plan.json> <build.json>
<binary>` additionally checks the binary. Recheck `verify` after each stage.
The checkout stays at its captured cut and may differ only in the exact version
field and implied local lock entries. Never broaden an allowed diff after failure.

## Native pipeline selection

`native-release.yml` selects one exact allocated tag with `prepare` off and
`tag=<tag>`. Its pipeline concurrency group includes that tag. A tagless `prepare`
run builds the selected main commit without a draft and publishes nothing.

The draft's `release-publication.json` marks a complete bundle: archives, the final
manifest and CLI installers are uploaded first, their digests compared with local
bytes, and the marker uploaded last after every final verifier passes. A draft
with `verification-failed.json` stays unpublished. After diagnosis, an owner may
dispatch that exact tag with `retry=<tag>` and `prepare` off. A cancelled run
records no failure marker; an owner-authorized dispatch of the same tag retries it.
A failed draft never holds later allocations or another tag's pipeline.

## Publication gates and recovery

Once a draft's bundle is attached, the run evaluates the publication gates in order:
`channel` (the version is an alpha, `X.Y.Z-alpha.N`; a stable version or any other pre-release
label such as `beta` or `rc` is held, and releasing that hold is refused: the owner publishes it
by hand), `commit` (the release's commit is on `main` and the pull request that produced it passed
`Code quality`, `Unit tests`, `Docker E2E` and `Native package matrix`), `immutability` (the
repository's newest published release is immutable, which shows that the setting was on; the
workflow token cannot read the setting itself), `monotonic` (an alpha has a unique unpublished tag and version; stable
publication remains newer than every published release of its product), `migration` (no commit of the release carries `!` or a
`BREAKING CHANGE:` footer; outside the alpha channel the component's migration list, named in
`.github/components.json`, also has no more entries than at the product's last published
release, while an alpha publishes new entries and the gate's summary only reports them) and
`upgrade` (four-host installation and real-archive acceptance, including CLI adapter
state migration under #575; the first release of a product has nothing to upgrade from). A failed gate does
not make the draft a failed build. The draft gets `publication-held.json` (`tag`, `sha`,
`gate`, `reason`, `runUrl`, `recordedAt`), and later runs list it as held and leave it alone.
The jobs that evaluate the gates hold the write token, so they run `main`'s code and only read
the release commit's data through git and the API. To release a hold once its cause is dealt
with, publish the draft by hand as below, or dispatch `native-release.yml` on `main` with the
product, `prepare` off and `hold` set to the tag: the run evaluates the gates again without
the one gate the marker names (never another, and never `channel`), removes the marker when
they pass and then publishes the draft as below.

An owner-authorized `rerun=<tag>` dispatch on main with `prepare` off instead re-proves
all gates, including the gate named in `publication-held.json`; it skips none. `retry`,
`hold` and `rerun` are mutually exclusive. The selected tag must be a bundled, held
product draft with a commit target. The gates validate the marker's tag, SHA and known
gate, and finish refuses a changed gate. Any failed gate leaves the original marker
unchanged; only after all gates pass is it removed, followed by normal publication,
attestation and public-install smoke checks. The
[release skill](../SKILL.md#automated-alpha-publication) owns rerun authorization.

Rerun uses the main commit selected by the dispatch for Node verifier scripts and
their locked dependencies. Archives, manifest, version and digests come from the
draft; CLI expected skill bytes, migration counts and applicable Rust adapter
acceptance code come from a separate checkout of its release SHA (`release-source`).
The read-only proof job compiles that source's adapter test with its own locked
dependencies and toolchain pin; release-source Node verifier scripts do not run.
Every checkout-dependent stage applies the reviewed version injection and verifies
the version-only source/lock contract before using candidate bytes. Ordinary
upgrade proof retains release-commit tooling. This separates repaired tooling
from the unchanged candidate under test without rebuilding or replacing its assets.

## Publication readback and public smoke

When every gate passes, the `publish` job publishes the draft. `release-publish.mjs publish`
reads the draft again and refuses unless its version is an alpha, its component is released
(`release: false` in `.github/components.json` parks a component: the cut planner creates nothing
for it and the native planner leaves its drafts alone and this command refuses them, so a draft that
predates the flag cannot publish), and it carries the bundle and neither
`publication-held.json` nor `verification-failed.json`; then publication applies the
[release skill's explicit flags and bounded latest convergence](../SKILL.md#main-cut-authorization). Both
flags are explicit because draft flags are not publication policy. The `published` job
then reads the release back: it is public and `immutable: true`, its flags are the policy's (a
CLI latest names the highest published CLI version, an extension release never is), its tag is on
the release commit, it carries `release-publication.json`, and GitHub's attestation verifies
(`gh release verify`, and `gh release verify-asset` for every asset downloaded from the
published release). GitHub finishes the attestation after publishing, so these checks are
retried for about two minutes. A failed check opens an issue and fails the run; nothing is
rolled back, because a published release is immutable and a repair needs a new reviewed
version. A `smoke` job then installs the published release as a user does
(`.github/workflows/native-release-smoke.yml`, also run by hand with `product` and `tag`, for
the newest published CLI/extension release or an exact published driver tag: CLI and
extension acquisition uses current public entry points, while a driver uses its versioned
archive URLs. A failed run reports on the issue like any other). On the four hosts of the upgrade proof, in an isolated home, state directory
and prefix, the highest CLI alpha goes through the public
`releases/latest/download/install.sh`; an older independent cut uses its versioned
installer: the installer names the tag's version, the installed `tmt`
is the one PATH selects and reports that version, the installed shared skills are the tag's
`skills/*` (same names, same `SKILL.md`), and `tmt upgrade --channel alpha --json` reads the live
metadata and reports the installation current (a newer alpha that appeared since passes with a
note). An extension alpha is installed by the newest published CLI's `tmt extension install
<extension>` into a separate prefix; `tmt extension list` must report the tag's version and no
CLI link may appear. A Herdr driver alpha downloads its exact tag’s
`dist-manifest.json` and matching standalone archive through public versioned URLs,
checks the bounded manifest, digest and inventory, then uses the current public CLI’s
supported `driver install <extracted-path> --yes --json` and `driver ls` surfaces
to verify capabilities and durable approval. Named released-driver acquisition belongs to #1084.
The tag is checked out only so its skills can be read; none of its code runs.
The shared `.github/actions/public-install-smoke` action supplies the workflow's
`contents: read` `GITHUB_TOKEN` only through the verifier's process environment.
The verifier forwards it to the shell bootstrap and native acquisition commands,
never argv, inspection commands or asset fetches. The native HTTPS client sends it
only to `api.github.com`, rebuilding authorization per redirect hop; public bootstrap
and archive downloads remain unauthenticated. Native bounded HTTPS retries are unchanged.

The CLI latest-installer read retries only an older alpha than the highest just-published
tag (three reads, two 20-second waits); unchanged lag fails, while malformed versions and download errors fail immediately. A newer valid
alpha selects the candidate's versioned installer for its exact installation proof. Install jobs have a 25-minute bound and
read-only contents permissions; a separate issue writer reports failures with all four
host artifacts. Historical anonymous rate-limit issues are not automatically closed.

## Manual publication readback

After an owner-authorized manual publication, use the pipeline's readback command
with an empty download directory outside the checkout:

```bash
node typescript/scripts/release-publish.mjs verify --product <product> --tag <tag> --directory <empty-directory>
```

It verifies public immutable state, the exact tag commit, product flags, latest
CLI selection, the completeness marker, release attestation and every downloaded
asset's attestation. A failed readback needs diagnosis; never republish immutable
assets or move a public tag.
