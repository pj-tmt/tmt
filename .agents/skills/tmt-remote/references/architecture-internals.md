# Remote architecture internals

Module owners (put a change in the existing owner; `canonical`, `crypto`, `wire`,
`transport`, `declaration`, `deploy_plan`, `rules`, `firestore_budget`, `firestore_limits`, `readiness` and `deploy_run` have no I/O, clock, storage or `CoreClient` access):

| Module                                           | Owns                                                                                                                                                                                                                  |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main`, binary-private `serve`, `core`           | CLI dispatch, one foreground/background composition and the two startup calls; `CoreClient` runs only fixed public `api`, `list --json`, `identity list --json`, `check <name> --json` via `TMT_EXECUTABLE`           |
| `http`, `routes`, `site`, `limits`               | Loopback door framing and bounds, `/r/` binding routes, route dispatch; every bound is named in `limits`                                                                                                              |
| `wire`, `canonical`, `crypto`                    | Strict JSON admission with exact payload bytes, framing/fingerprint codecs, signature and HMAC verification                                                                                                           |
| `session`, `admission`, `transport`              | `session.open`, one normal message in flight per session, durable sequence consumption, envelope hand-off                                                                                                             |
| `journal`, `budgets`, `audit`                    | Metadata streams and recovery ownership, persisted budgets, audit written in the owning transaction                                                                                                                   |
| `operations`, `approval`                         | Dispatch/read operations over the public core API; local held-operation confirmation on the control socket                                                                                                            |
| `authority`, `store`, `state`                    | Typed grants, `remote.db` and schema history, layout/machine key/serve lock                                                                                                                                           |
| `pairing`, `control`, `devices`                  | One pairing offer per run, owner-only control socket for discovery/stop and device list/revoke/rename and explicit sending-scope toggles                                                                              |
| `mount`, `pages`                                 | Extension mounts (allowlisted extensions only), static landing/pairing/error pages and embedded stylesheet/SDK assets                                                                                                 |
| `objects`                                        | Object backend trait and `LocalFs`: the `objects.db` ledger and extension-private payload trees under the serve lease; the channel and config belong to `object_service`                                              |
| `declaration`, `deploy_plan`, `rules`            | Strict backend declaration parse; digest-addressed `sharing` deploy plan; allow-listed Rules/indexes composition (bytes in; installed discovery belongs to `deploy_discovery`)                                        |
| `deploy_command`, `deploy_record`                | Plan/authorization/output over captured inputs/provider; private deployment identity/run file, atomic replacement under its writer lock; lock-free per-request status evidence                                        |
| `firestore_budget`                               | Free-plan Firestore budget model and client guard; vectors in `tests/fixtures/firestore_budget`                                                                                                                       |
| `firestore_limits`                               | Dated free-plan Firestore limits table and its `status --budget` projection, validator and human lines; golden in `tests/fixtures/firestore_budget/limits-member.json`                                                |
| `readiness`                                      | Layered Firestore readiness: one table of items, reasons and sentences; record projection, validator and human lines over an injected evidence source                                                                 |
| `deploy_cli`, `deploy_discovery`, `deploy_tools` | CLI argv/composition; fixed public installed declaration replies; installed Firebase launcher/Node discovery (no receipt parser or mutable helper file)                                                               |
| `deploy_firestore`                               | Real provider port; binary-embedded Node helper, version-gated firebase-tools login, bounded no-retry mutations and exact Rules read-back                                                                             |
| `deploy_run`                                     | Authorized Firestore sharing deploy over an injected `DeployPort`: envelope digest, authorization, step order, record and binding rule (CLI composition in `deploy_cli`)                                              |
| `object_service`                                 | Lease-bound `ObjectService`: initial/demand single-flight Local setup, readiness, origins, admitted observation/upload; production Colab-only Local                                                                   |
| `tmt-extension-objects` (leaf)                   | Remote-owned protocol leaf, consumed only by `object_service`: canonical IDs/encodings, protocol bounds, strict JSON, typed frames and the Unix carrier; no backend, policy or Remote/Colab types, and grants nothing |

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
  "no effect" from a missing row. Fixed read operations bypass journal adoption;
  they retain signed admission, sequence and call budgets. Recovery observation settles only a
  newly learned definitive original outcome. Stable polls never write ownership or entries,
  retry or send. A full stream drops its oldest entries (`make_room_for_entry`), and a full
  ownership table drops read leftovers and the oldest finished records
  (`make_room_for_operation`); held, dispatching and uncertain records are never evicted.
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
  evicts that device's most idle session without a live transport first; when all
  are attached, it evicts the most idle attached session.
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

## Deployment record

`deploy_firestore` implements the existing `DeployPort`, not a second deployment algorithm.
Its helper is compiled into the binary and passed to Node with fixed argv and no shell or
runtime helper path. The package path selects an installed firebase-tools release; only
version 15.29.0 and its checked auth/layout are supported. Compatibility is checked before
auth loads or any provider call. The child inherits only HOME, PATH and XDG_CONFIG_HOME;
every helper child starts at `/`, before auth loads, so project-local Firebase configuration
cannot apply; tokens stay in that child. Each effect re-resolves the login and verifies its live account.
Only bounded, fixed-schema JSON crosses stdout; stderr and exception text are discarded.
The absolute invocation deadline includes process I/O and exit; owned-group cleanup/reaping
has its existing separate bound. Mutations are never automatically retried. An ambiguous
reply is unknown: inspect the retained run and read back before authorizing another attempt.

Database, selected sign-in providers and indexes precede the Rules release. Index allocation
counts unrelated live field configurations against the Spark ceiling and preserves existing
index/TTL settings. Google OAuth and uninitialized Auth remain fixed owner steps. Ruleset
lookup is bounded and compares exact source before another create; exhausted lookup refuses.
Rules switching requires a single writer for the project: no provider atomic precondition
exists. The adapter re-reads immediately before the last switch and checks full source after
it; visible drift refuses, while ambiguity after a possible effect is unknown with no binding.
Fake process/HTTP tests use a vendored layout stub and canary credentials, never real login.
Real account/project provisioning remains separately authorized acceptance, not fixture proof.

The deployment owner defaults to a plan and saves one local draft identity,
without a provider effect. Explicit digest authorization names the whole envelope; foreign
Rules replacement needs its own digest. Sign-in providers are explicit inputs. Command
registration composes `deploy_cli`, `deploy_discovery`, `deploy_tools` and the real port.
`deploy_discovery` enumerates trusted enabled mounts and invokes their fixed public
`deploy-declaration --json` commands through supplied `TMT_EXECUTABLE`, with a neutral cwd,
explicit environment allow-list, capped strict JSON and existing waited cleanup. It captures
and validates exact UTF-8 declaration/artifact bytes and both digests once, then uses the pure
plan/Rules owners. No installed-root/receipt parser or artifact-path read exists in Remote.
An old unsupported command means no declaration; malformed/failed replies are unavailable.
Colab has no shipped declaration yet; production never substitutes Remote fixture data.
`deploy_tools` resolves the installed Firebase launcher's realpath and exact supported package
layout, then resolves Node from that launcher's absolute or env-node shebang, excluding
relative PATH entries. The embedded helper's compatibility gate still precedes credentials.
No-declaration output touches neither provider nor deployment record. Plan-only inventory
checks live plus planned field configurations; effect-time allocation rechecks the ceiling.
Installed Colab acceptance and separately authorized real-project proof wait for its command;
running `status --layers` now reads recorded deployment evidence without a provider call.
`DeployRecordEvidence` takes one bounded lock-free snapshot per request; missing/draft is
empty, damaged state is unknown and never repaired. The pure `readiness::from_record`
projection recognizes finished database/sign-in steps and usable verified bindings. A failed
run before Rules preserves its old binding; an incomplete Rules attempt reports partial
Rules and withdraws it. This describes recorded outcomes, not live provider currentness.

Plan tier and quota remain unknown because the machine cannot observe either, and layer-1
traffic goes browser to Firestore. Sharing therefore stays unknown after a complete recorded
run. A later Core readiness decision must define tier treatment for layers requiring no paid
plan and a quota evidence source before sharing can read enabled.

The native `emulator_artifact_is_the_exact_verified_deployment_output` test runs the existing
algorithm through the fake provider and real record store, binding its captured Rules bytes
and independent marker/body digest to `tests/fixtures/rules/deployed.rules`. The emulator
loads that exact artifact for member admission, expiry, isolation and create-only refusal;
it does not prove project/database/Auth provisioning, login, billing or quotas.

`deploy.json` and `deploy.lock` belong to Remote's private layout, separately from the
serving lease and database. The writer retains the nonblocking lock for a plan/run, writes a
create-new private staging file, syncs it, atomically renames it and syncs the directory.
Readers never take `deploy.lock`: an opened snapshot is a complete old or new record, so a
long deployment cannot block status. Malformed, oversized or unsafe state is refused, never
reset. Save failure stops effects; a directory-sync failure after rename does not mean the
visible publication rolled back. Staging cleanup only removes admitted deployment staging
names under the writer lock. Existing status/stop, settings, store and machine-key owners
keep the deployment identity and binding.
