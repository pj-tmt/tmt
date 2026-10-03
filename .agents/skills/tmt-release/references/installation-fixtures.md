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
`product=cli`, an empty tag and a snapshot outside the checkout. The
[release skill](../SKILL.md#main-cut-authorization) owns the
synthetic-version rule. Build
only `tmt-cli --bin tmt` in debug mode, copy it and the independently built Herdr
companion into `debug/native-release-fixture/`, and verify the source gate again.
After that succeeds, restore only `rust/Cargo.toml` and `rust/Cargo.lock` from the
captured commit and confirm `git diff --exit-code HEAD --` before running tests.
Retain the fixture, build time and source-gate evidence in the delivery record;
never publish this synthetic version. Shared-host Cargo limits still apply.

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
