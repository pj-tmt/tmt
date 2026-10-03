# Native checks for providers, Herdr and load

Shared native selection, isolation and fixture-build rules are in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md#native-process-and-shared-tests).

## Provider contracts (opt-in, developer only)

With the supported Claude Code and Codex CLI executables already installed:

```bash
cargo run --locked --manifest-path rust/Cargo.toml -p tmt-adapters --example runtime-contract -- /absolute/claude /absolute/codex
cargo run --locked --manifest-path rust/Cargo.toml -p tmt-adapters --example channel-contract -- /absolute/claude
```

They run only `--version` and the generated resume or `--help` arguments, fail outside
the pinned range and never start a model. Review provider behavior before changing a
pin. Help-parser acceptance does not prove a session can resume. Normal `tmt run` does
not run these checks.

## Provider setup and lifecycle

`test/native/setup.test.ts` owns CLI consent, stable-launcher repair/removal, the
always-zero bounded hook failure contract and custom `CODEX_HOME` without trust/config
mutation. Setup planning tests use disposable settings files and must preserve user
hook/permission bytes, exact reruns, recovery copies and changed-input refusal,
including symlinked settings (dangling or replaced after planning) and unset, empty,
absolute and relative `CLAUDE_CONFIG_DIR`. Backups are compared byte-for-byte with
private modes; publication continues with a warning above 32 backups in
`.tmt-setup-backups`. Unknown Claude `SessionEnd` reasons yield no lifecycle
observation (a known terminal reason is the positive control). Runtime adapter tests own
payload mapping and stale-end rejection, core/storage tests own session CAS, Docker
owns real pane/process integration. Manual provider acceptance uses a disposable
identity/window and isolated provider settings; no test runs setup against the user's
real provider directory.

## Herdr

`test/native/herdr.test.ts` is skipped unless `TMT_TEST_HERDR` names the pinned
`herdr` 0.9.1. Download it to a scratch directory, never an install path, and compare
its digest:

```bash
gh release download v0.9.1 -R herdrdev/herdr -p herdr-macos-aarch64 -D /tmp/hdrbin
gh api repos/herdrdev/herdr/releases/tags/v0.9.1 -q '.assets[]|select(.name=="herdr-macos-aarch64")|.digest'   # compare:
shasum -a 256 /tmp/hdrbin/herdr-macos-aarch64
mv /tmp/hdrbin/herdr-macos-aarch64 /tmp/hdrbin/herdr && chmod 755 /tmp/hdrbin/herdr
TMT_TEST_HERDR=/tmp/hdrbin/herdr pnpm exec vp test run --config test/native/vitest.config.ts test/native/herdr.test.ts
```

It starts a headless server on a short private socket and fails if any server process
remains. Its carry-over case also needs `TMT_TEST_PREVIOUS_TMT`: an absolute `tmt`
built from a revision before #1082 in its own worktree and target directory.

Driver checks (`tmt-driver-herdr`, independently versioned):

```bash
(cd rust && cargo test --locked -p tmt-driver-herdr)
(cd rust && TMT_TEST_HERDR=/tmp/hdrbin/herdr cargo test --locked -p tmt-driver-herdr --test herdr_driver)
(cd rust && cargo clippy --locked -p tmt-driver-herdr -p tmt-cli --all-targets -- -D warnings)
(cd rust && cargo test --locked -p tmt-adapters --lib host::external)
(cd rust && cargo test --locked -p tmt-cli --test architecture)
```

Without `TMT_TEST_HERDR` only the stand-in conformance case runs. A change to
`tmt-driver-protocol` (wire types, decoding, `serve`, either conformance harness) also
runs `cargo test --locked -p tmt-driver-protocol`. Runtime approval is checked with
`cargo test --locked -p tmt-cli --test driver_command`; `DriverProcess::locations` is the
shared `within(home)` admission boundary.

## Context and inbox

`test/native/context.test.ts` owns the unbound/missing-configuration no-side-effect
checks for `tmt whoami --context`; the `identity-context` and `identity-context-requests`
Docker scenarios own bound acceptance (independent SQLite snapshots must not change on a
context read). `test/native/inbox.test.ts` owns the no-tmux queue, bounded listen, receipt
and reply path; fake-clock unit tests own debounce timing.

## Load diagnostics

`pnpm test:stress:native` runs the maximum-body, 50-reply page as an explicit
diagnostic, not required acceptance.
