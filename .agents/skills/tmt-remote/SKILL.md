---
name: tmt-remote
description: Build, run and verify the Remote door (`tmt-remote`, `tmt remote ...`), and its embedded browser client. Load when changing extensions/tmt-remote or running its checks. Owner - the tmt-remote squad.
---

# Remote development

Behavior lives in [`contracts/remote-channel-v1.md`](../../../contracts/remote-channel-v1.md)
and [ARCHITECTURE.md](../../../ARCHITECTURE.md); this skill holds module ownership
and how to build, run and verify. Shared Rust, native and Docker gates are in
[DEVELOPMENT.md](../../../DEVELOPMENT.md). The `tmt-extension-state` leaf is shared
with Colab; [tmt-colab](../tmt-colab/SKILL.md) links here for it.

## Read what you are changing

- [references/architecture-internals.md](references/architecture-internals.md): module
  owners, the rules that are easy to get wrong (one opener, authority in the
  transaction, revoke fence, serve lease, pre-auth refusals, mount trust) and the shared
  extension-state leaf checks. Read before touching any `tmt-remote` Rust module.
- [references/door-and-discovery.md](references/door-and-discovery.md): running the
  door, `status` / `stop` / remembered-port / route-prefix implementation checks,
  browser opening and `settings`.
- [references/browser-pages.md](references/browser-pages.md): the `/pair#CODE` page,
  the embedded stylesheet and header-token drift tests.
- [references/object-backends.md](references/object-backends.md): the object backend
  trait, the `LocalFs` ledger and private payload trees, effect order, charge formula
  and how to add an adapter and run its conformance suite.
- [references/sdk-operations.md](references/sdk-operations.md): the `remote-client`
  package gates, the embedded `remote-v1.js` asset (rebuild and commit it after any
  `remote-client/src` change; CI fails on a difference), crypto fixtures and the
  Chromium pairing smoke.
- Installer registration for `remote` and `colab` is verified with the
  [release reference](../tmt-release/references/native-release.md#remote-and-colab-installer-registration).

## Must-know rules

- Test with disposable HOME/XDG and short roots under `/tmp` (Unix socket paths are
  limited to about 100 bytes). No test uses a real model, account or core database.
- Limits are named in `src/limits.rs`, not in guides. A new workspace path also needs the
  [tmt-layout](../tmt-layout/SKILL.md) checks, generated release configuration and
  CI-scope checks.
- Keep run evidence in the issue or PR, not in this skill.

## Verify

From the repository root; use `CARGO_BUILD_JOBS=2` on shared machines.

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-remote --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts)
```

- The Firestore Rules emulator suite (`tests/emulator/suite.mjs`, no npm dependency) runs only under
  `firebase emulators:exec --only firestore` with firebase-tools 15.29.0 and Java 21, and has no skip path.
  CI runs it as selected steps of the Unit tests job (ci-scope `remote_firestore`, no Docker). Locally use the
  tiny `tests/emulator/Dockerfile` in the booked Docker slot only (reservation file RELEASED, 30 GiB free, one
  tag per worktree, remove the image and container after every run; see DEVELOPMENT's disk section):
  `docker run --rm --init --network none -v "$PWD/extensions/tmt-remote/rust/tmt-remote/tests:/t:ro" <tag> firebase emulators:exec --only firestore --project demo-tmt-remote --config /t/emulator/firebase.json --non-interactive 'node --test /t/emulator/suite.mjs'`.
- Native operation tests use signed requests, private real storage and
  deterministic public-process fixtures; they are not real-core acceptance. The
  SIGKILL probe checks serve-lease inheritance and release.
- `remote-operations` and `remote-recovery` Docker scenarios cover dispatch, hold,
  recovery and one permitted/refused read through `E2EFixture`. Run them with
  `CARGO_BUILD_JOBS=2 corepack pnpm test:e2e` in the booked isolated Docker heavy
  slot, twice. Their wrapper executes the selected real core; test-only grant
  seeding happens only in Remote storage while serve and owned children are stopped.
- Door tests use disposable HOME/XDG, count startup core calls separately, assert
  zero request-triggered core calls and run socket/process lifecycle twice.
- Per-topic commands (CLI/state, shared state leaf, SDK package, browser smoke,
  crypto fixtures) are in the references above.
