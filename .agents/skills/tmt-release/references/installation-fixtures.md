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
