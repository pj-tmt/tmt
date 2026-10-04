---
name: tmt-remote
description: Build, run and verify the Remote door (`tmt-remote`, `tmt remote ...`), and its embedded browser client. Load when changing extensions/tmt-remote or running its checks. Owner - the tmt-remote squad.
---

# Remote development

Behavior lives in [`contracts/remote-channel-v1.md`](../../../contracts/remote-channel-v1.md)
and [ARCHITECTURE.md](../../../ARCHITECTURE.md); this skill holds only how to build,
run and verify. Shared Rust, native and Docker gates are in
[DEVELOPMENT.md](../../../DEVELOPMENT.md).

## Rust crate

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-remote --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts)
```

- Pairing and state tests use short roots under `/tmp`: Unix socket paths are
  limited to about 100 bytes.
- Native operation tests use signed requests, private real storage and
  deterministic public-process fixtures; they are not real-core acceptance. The
  SIGKILL probe checks serve-lease inheritance and release.
- `remote-operations` and `remote-recovery` Docker scenarios cover dispatch, hold,
  recovery and one permitted/refused read through `E2EFixture`. Run them with
  `CARGO_BUILD_JOBS=2 corepack pnpm test:e2e` in the booked isolated Docker heavy
  slot, twice. Their wrapper executes the selected real core; test-only grant
  seeding happens only in Remote storage while serve and owned children are stopped.
- Door tests use disposable HOME/XDG, count startup core calls separately, assert
  zero request-triggered core calls and run socket/process lifecycle twice. No real
  model, account or database is used.

## Run the door

Build core, put `rust/target/debug` on `PATH`, then `tmt remote serve` (or `--json`
for the bound descriptor). Direct invocation requires an absolute `TMT_EXECUTABLE`;
it never searches for another core. Ctrl-C/SIGTERM closes listeners, sockets and
workers. Limits are named in `src/limits.rs`.

## Shared extension state

[The state leaf](../../../ARCHITECTURE.md#shared-extension-state-layout) is
library-only and shared with Colab:

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-extension-state)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-colab --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-extension-state -p tmt-remote -p tmt-colab --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
```

Synced publication tests prove filesystem behavior, not power-loss recovery. A new
workspace path also needs the tracked-file layout, generated release configuration
and CI-scope checks.

## Embedded client and crypto fixtures

The door embeds `extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js`, built
from `remote-client`. After changing `remote-client/src`, rebuild and commit it
(Code quality rebuilds it and fails on a difference), then run the Chromium pairing
smoke against a debug door:

```bash
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match build)
(cd rust && CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote)
(cd typescript && pnpm --filter @tmt/remote-client exec playwright install chromium)
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match test:browser)
```

Byte and crypto conformance runs with the Rust tests. The shared vectors come from
`extensions/tmt-remote/typescript/remote-client/test/reference.py` (which also checks the
pinned BIP-39 list digest). Check the Rust-owned fixtures with:

```bash
python3 extensions/tmt-remote/rust/tmt-remote/tests/fixtures/mac-reference.py --check
node extensions/tmt-remote/rust/tmt-remote/tests/fixtures/webcrypto.mjs
```

Use the repository Node 22 and repeat the WebCrypto command on Node 24; `--write`
regenerates the public-test-key fixture. The Python oracle imports no product code.
None of this proves real Chrome key persistence across MV3 worker restarts.

## Installer registration

Core's installer registration for `remote` and `colab` is verified with the commands in the
[release reference](../tmt-release/references/native-release.md#remote-and-colab-installer-registration).

## SDK operations

Follow [tmt-dev](../tmt-dev/SKILL.md), the
[Remote architecture](../../../ARCHITECTURE.md#remote-extension-pilot) and
[channel contract](../../../contracts/remote-channel-v1.md). The
[SDK README](../../../extensions/tmt-remote/typescript/remote-client/README.md)
owns caller-facing usage. Inspect the SDK and Rust operation shapes together.

From `typescript/`, run the package gates with pinned pnpm:

```sh
pnpm --filter @tmt/remote-client --fail-if-no-match check
pnpm --filter @tmt/remote-client --fail-if-no-match test
pnpm --filter @tmt/remote-client --fail-if-no-match build
```

`test` checks the independent Python oracle before unit tests; repeat on Node 24
for WebCrypto conformance. Cover signed states/refusals, response correlation,
sequence serialization and both consumed/unconsumed lost-request recovery branches
on the existing session through `operation.show`, with the original ID and no
recovery dispatch or reopen. Verify the two-guess limit and typed refusal/unknown
outcome codes.

Commit the regenerated `extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js`.
Repeat the build and compare exact artifact bytes; after staging, check the asset's
`git diff --exit-code` and `git status --porcelain` for regeneration drift.
Rebuild the worktree's debug door from `rust/` with
`CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote`, then run
`pnpm --filter @tmt/remote-client --fail-if-no-match test:browser` from `typescript/`.
Install Chromium through the package's Playwright command if absent.
The smoke uses real Remote/Chromium and a deterministic public-core fixture;
assert direct acceptance, read-only observation, the retained final and one core
dispatch. Preserve pairing/certification/revocation and joined fixture cleanup;
this does not prove real-core agent delivery. Use [tmt-layout](../tmt-layout/SKILL.md)
for added modules/fixtures. Keep run evidence in the issue/PR.

## Architecture internals

Module owners (put a change in the existing owner; `canonical`, `crypto`, `wire`
and `transport` have no I/O, clock, storage or `CoreClient` access):

| Module                              | Owns                                                                                                                                                                           |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `main`, `core`                      | Foreground composition and the two startup calls; `CoreClient` runs only fixed public `api`, `list --json`, `identity list --json`, `check <name> --json` via `TMT_EXECUTABLE` |
| `http`, `routes`, `site`, `limits`  | Loopback door framing and bounds, `/r/` binding routes, route dispatch; every bound is named in `limits`                                                                       |
| `wire`, `canonical`, `crypto`       | Strict JSON admission with exact payload bytes, framing/fingerprint codecs, signature and HMAC verification                                                                    |
| `session`, `admission`, `transport` | `session.open`, one normal message in flight per session, durable sequence consumption, envelope hand-off                                                                      |
| `journal`, `budgets`, `audit`       | Metadata streams and recovery ownership, persisted budgets, audit written in the owning transaction                                                                            |
| `operations`, `approval`            | Dispatch/read operations over the public core API; local held-operation confirmation on the control socket                                                                     |
| `authority`, `store`, `state`       | Typed grants, `remote.db` and schema history, layout/machine key/serve lock                                                                                                    |
| `pairing`, `control`, `devices`     | One pairing offer per run, owner-only control socket, device list/revoke/rename                                                                                                |
| `mount`, `pages`                    | Extension mounts (allowlisted extensions only) and the pairing page/SDK assets                                                                                                 |

Rules that are easy to get wrong:

- **One opener.** `store::Store` opens only with the `state::Serving` proof of the
  serve lock. Pairing and device commands reach state through serve's control
  socket; `tmt remote devices` without serve takes the lock itself.
- **Authority lives in the transaction.** An effect runs in `Store::effect`: it
  rereads the persisted grant (revision, liveness, scope, recipients, expiry)
  inside an IMMEDIATE transaction and performs the core call under that fence. A
  new check outside the transaction is a race with revocation.
- **Revoke waits for the fence.** SQLite authority writers wait
  `limits::AUTHORITY_WAIT` (40 s), longer than two `CORE_CALL` runs (15 s each)
  plus cleanup, so a revoke ordered after an in-flight effect succeeds once the
  call releases. Keep that ordering when changing either constant.
- **Uncertainty keeps identity.** The `dispatching` audit row commits after the
  core call; recovery uses the adopted frozen intent and core's idempotent
  operation ID (`dispatch.show` before any `dispatch.create`) and never infers
  "no effect" from a missing row. Reads never retry or send.
- **Serve lease.** `Serving::retain_for_invocations` clears close-on-exec on the
  lock file so invocation children inherit it; closing never unlocks. A restart
  refuses while an orphaned child lives; unconfirmed cleanup disables writes until
  a fresh lease-owning run. The lease test is a real-process SIGKILL probe in
  `core_tests.rs`.
- **Pre-auth stays generic.** Refusals before a verified signature or live
  session are one 404 with no inventory. The `tmt_door` cookie only identifies a
  session on mounted paths; `/r/` refuses cookies.
- **Mount trust.** Mounted extensions share one trust domain behind the door.
  The device context header is added only for a live owner session and is never
  copied from a client.
- **Embedded SDK asset.** The door embeds `assets/remote-v1.js` built from
  `remote-client/src`. After changing the SDK, rebuild and commit the asset (CI
  rebuilds it and fails on a difference); see Embedded client and crypto fixtures above.
- **Parked add-on.** `typescript/browser-addon` is a demo shell with no crypto,
  pairing or network; it is not a working channel (#1056).
