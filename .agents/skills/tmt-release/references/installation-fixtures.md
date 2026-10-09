# Installation and upgrade fixtures

Installation and upgrade sandboxes use the one `withReleaseSandbox` helper in
`test/support/native-installation.ts`. They require a real debug CLI injected to the shared non-publishing preparation
version, at the selected Cargo target's
`debug/native-release-fixture/tmt`, with its Herdr companion beside it. They never
label a development executable as an alpha or relax channel checks. CI builds
this fixture only when the existing native test selection includes installation
or upgrade tests; it reuses the production injection action, verifies the source
gate after the debug build, reports build seconds, then restores the reviewed
workspace version and lock before running tests. Other process tests keep the
development CLI.

For local installation-fixture preparation, start with a clean, committed task
checkout and follow the [version-injection procedure](main-cuts.md) using
`product=cli`, an empty tag and a snapshot outside the checkout. Use the
[synthetic preparation version](#synthetic-preparation-version) below. Build
only `tmt-cli --bin tmt` in debug mode, copy it and the independently built Herdr
companion into `debug/native-release-fixture/`, and verify the source gate again.
After that succeeds, restore only `rust/Cargo.toml` and `rust/Cargo.lock` from the
captured commit and confirm `git diff --exit-code HEAD --` before running tests.
Retain the fixture, build time and source-gate evidence in the delivery record;
never publish this synthetic version. Shared-host Cargo limits still apply.

## Process fixture builds

**Fixture builds belong in the process job itself.** Process and archive fixtures
need the CLI and Herdr together (`cargo build --locked -p tmt-cli -p
tmt-driver-herdr --bins`); extension archive scenarios also need `tmt-ops`,
`tmt-remote` and `tmt-colab`, and Colab's verifier needs `cargo build --locked -p
tmt-test-support --example colab-runtime-fixture`. Another job's build or a warm
local target does not supply them.

## Synthetic preparation version

Non-publishing `prepare` rehearsals and installation fixtures use
`<committed major.minor.patch>-alpha.999999`, derived only through
`release-versions.syntheticAlphaVersion`. This synthetic version is never a real
cut or publication target. Preparation keeps the draft tag empty, creates no Git
tag and runs all source, archive and installation gates against the injected
version. It adds no dispatch input and activates no product.

## Development version comparisons

The infra tooling comparer, `typescript/scripts/release-versions.mjs::compareVersions`,
orders exact `X.Y.Z-dev` below every non-dev version with the same major, minor and
patch. Numeric core-version precedence and ordering between non-dev versions retain
Semantic Versioning rules. Other prerelease identifiers, including `dev.1`, keep
their ordinary semver ordering.

This comparison rule does not make dev builds eligible for installation or
publication. Native release channels and managed receipts accept only their
release versions; installation fixtures still use the injected synthetic alpha
above. Skill drift compares content and owned links, and companion probes compare
exact versions; neither uses release precedence.

## Native recording driver

Extension-upgrade proofs use one native recording driver on macOS and Linux.
Build it before running `test/native/extension-upgrade-proof.test.ts`:

```sh
cargo build --locked --manifest-path rust/Cargo.toml -p tmt-test-support --example recording-cli-fixture
(cd typescript && corepack pnpm exec vp test run --config test/native/vitest.config.ts test/native/extension-upgrade-proof.test.ts)
```

The example is selected from `rust/target/debug/examples/recording-cli-fixture`,
matching the host architecture (including an x64 Node/Rust pair under Rosetta).
Both native-process CI scopes build it. The synthetic driver archive's
`NATIVE-INSTALL.md` contains JSON with absolute `executable` and `log` fixture
paths; the driver records the first two arguments and replaces itself with the
selected CLI, preserving argv, stdio, cwd, environment and exit behavior.
The note is fixture configuration, not shipped installation guidance.
Publish the built bytes through `writeExecutable`; do not package a shell driver,
compile during a scenario, relax exact Mach-O inspection or extend its deadline.

## Prepared Squad to Ops qualification

The explicit case in `extension-upgrade-proof.test.ts` consumes one task-owned
`TMT_EXTENSION_UPGRADE_PROOF_DIRECTORY`: `plan.json` and the target subdirectories
from `release-upgrade.mjs` staging, plus `component-map.json` containing the reviewed
registration/activation copy. Never change the committed activation map for a rehearsal.
The old driver and Squad alpha.50 archive are digest-checked published assets; the candidate
CLI is built with source/version injection from the captured Ops dependency commit, and the
Ops archive carries genuine independently built fixture bytes. No shell/mock installer or
scenario-time compilation substitutes for either CLI. Default PR CI has no prepared input and
excludes this explicit qualification; supplying an empty or malformed input fails.

After the separate heavy-slot, load and disk admission, run only this prepared case:

```sh
TMT_EXTENSION_UPGRADE_PROOF_DIRECTORY=/absolute/task-owned/staging corepack pnpm exec vp test run --config test/native/vitest.config.ts test/native/extension-upgrade-proof.test.ts -t 'prepared two-real-CLI'
```

It rechecks staged digests and provenance through `proveStaged`, invokes the existing verifier,
retains real command stdout/stderr and the result beside the staged inputs, and checks the
private proof home was removed. The verifier preserves every sentinel byte and old immutable
skill generation through replacement, repeat and old-archive refusal. This local qualification
is not a published-release proof; the first Ops release uses the normal published driver pair.
Reported removal order and final state do not prove internal lock/syscall timing.
