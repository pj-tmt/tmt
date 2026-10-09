# Remote architecture internals

Module owners (put a change in the existing owner; `canonical`, `crypto`, `wire`
and `transport` have no I/O, clock, storage or `CoreClient` access):

| Module                                 | Owns                                                                                                                                                                                                                  |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main`, binary-private `serve`, `core` | CLI dispatch, one foreground/background composition and the two startup calls; `CoreClient` runs only fixed public `api`, `list --json`, `identity list --json`, `check <name> --json` via `TMT_EXECUTABLE`           |
| `http`, `routes`, `site`, `limits`     | Loopback door framing and bounds, `/r/` binding routes, route dispatch; every bound is named in `limits`                                                                                                              |
| `wire`, `canonical`, `crypto`          | Strict JSON admission with exact payload bytes, framing/fingerprint codecs, signature and HMAC verification                                                                                                           |
| `session`, `admission`, `transport`    | `session.open`, one normal message in flight per session, durable sequence consumption, envelope hand-off                                                                                                             |
| `journal`, `budgets`, `audit`          | Metadata streams and recovery ownership, persisted budgets, audit written in the owning transaction                                                                                                                   |
| `operations`, `approval`               | Dispatch/read operations over the public core API; local held-operation confirmation on the control socket                                                                                                            |
| `authority`, `store`, `state`          | Typed grants, `remote.db` and schema history, layout/machine key/serve lock                                                                                                                                           |
| `pairing`, `control`, `devices`        | One pairing offer per run, owner-only control socket for discovery/stop and device list/revoke/rename                                                                                                                 |
| `mount`, `pages`                       | Extension mounts (allowlisted extensions only), static landing/pairing/error pages and embedded stylesheet/SDK assets                                                                                                 |
| `objects`                              | Object backend trait and `LocalFs`: the `objects.db` ledger and extension-private payload trees under the serve lease; the channel and config belong to `object_service`                                              |
| `object_service`                       | Lease-bound `ObjectService`: initial/demand single-flight Local setup, readiness, origins, admitted observation/upload; production Colab-only Local                                                                   |
| `tmt-extension-objects` (leaf)         | Remote-owned protocol leaf, consumed only by `object_service`: canonical IDs/encodings, protocol bounds, strict JSON, typed frames and the Unix carrier; no backend, policy or Remote/Colab types, and grants nothing |

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
  device context on mounted paths; `/r/` refuses cookies. A transport-only
  `tmt-session` identifier must belong to that cookie device; it is stripped before forwarding.
- **Mount trust.** Mounted extensions share one trust domain behind the door.
  The device context header is added only for a live owner session and is never
  copied from a client. Session lifetime counts successful upgraded transports;
  last-close touches the session, starting the short inactivity grace for every session
  without a live transport. Reattach resumes that session; detached sessions count against
  the cap until expiry. Live-transport and no-transport idle limits belong to `limits`;
  the door maintenance loop persists expiry cleanup. `session` owns
  per-session replay and device-wide authority loss. Held work belongs to the grant and
  survives session end; only stop/revoke/expiry/revision change cancels it. The journal/ack remain per device.
- **Session cap.** `session.open` rereads `settings` on each open. Unset settings
  use the default cap of 8 sessions per device; `off` is unlimited. `session`
  enforces an active cap by evicting that device's least recently used session.
- **Multi-session migration.** The `multi_session` store migration preserves grants
  and copies existing client/server sequence counters into session-ID-keyed rows.
- **Embedded SDK asset.** The door embeds `assets/remote-v1.js` built from
  `remote-client/src`; rebuild and commit it as described in
  [sdk-operations.md](sdk-operations.md#embedded-client-and-crypto-fixtures).
- **Object backend.** `objects` has one metadata and accounting owner (`objects.db`),
  opened with the `Serving` proof; bodies live under `<dataRoot>/<extension>/objects/`
  and are reached only through no-follow directory handles. Read
  [object-backends.md](object-backends.md) before changing it.
- **Parked add-on.** `typescript/browser-addon` is a demo shell with no crypto,
  pairing or network; it is not a working channel (#1056).

## Shared extension state

[The state leaf](../../../../ARCHITECTURE.md#shared-extension-state-layout) is
library-only and shared with Colab:

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-extension-state)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-colab --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-extension-state -p tmt-remote -p tmt-colab --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
```

Synced publication tests prove filesystem behavior, not power-loss recovery.

The wire leaf `tmt-extension-objects` is library-only and has one consumer: Remote's
`object_service` (policy-guarded; no other Remote file, no other package). Its Unix-only
`carrier` module (handshake, bounded frame I/O, correlation ledger, `Bus` driver) is the only part that names `httparse` or `nix`
(`cfg(unix)` dependencies, guarded); the protocol modules stay free of OS dependencies. It owns the wire
bounds (`limits`: chunk, policy input, payload); the `tmt-remote` limits of the same value do not
depend on it, so consumer slices must reuse the leaf bounds or equality-test them against the
backend limits. Its tests are in-crate, so run it alone and keep it in the architecture guard:

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-extension-objects)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
```
