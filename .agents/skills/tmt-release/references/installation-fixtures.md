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
