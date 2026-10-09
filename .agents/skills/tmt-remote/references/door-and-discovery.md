# Run the door, discovery and settings

The exact public documents, remembered-port and busy-port behavior, route-prefix
format, `stop` semantics and the browser-opening rules belong to the
[channel contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery);
this page holds how to run the door and what its implementation tests prove.

## Run the door

Build core and put `rust/target/debug` on `PATH`, then `tmt remote serve`. Human startup
returns after background handoff and links to the browser entry at `/`, not the protocol base. Use `tmt remote status --json` and `tmt remote stop` to inspect
and end that same owner. `serve --foreground` keeps terminal ownership; exact bare `serve --json`
also remains foreground for existing supervisors. Explicit `--background --json` detaches with
the unchanged protocol descriptor. Direct invocation needs an absolute `TMT_EXECUTABLE` and never
searches for another core. Stop preserves pairings/grants and cancels pending pairing/held work;
launcher departure after accepted handoff does not stop it. Limits are named in `src/limits.rs`.

`main` owns CLI grammar/dispatch; binary-private `serve` owns mode selection, exact native self-exec,
signals and one composition over existing core/state/Control/Approval/events/HTTP owners. The private
UnixStream is transferred through native Stdio safely, restored to close-on-exec and closed before
continued serving. Its bounded monitor ends/joins at acceptance or failure. Parent Ready -> Accept
-> Accepted handling uses successful Accept write as the no-kill cutoff; lost acknowledgment or
output is unconfirmed, with status/stop guidance, never automatic retry. Pre-accept cleanup requires
worker confirmation plus exit; forced kill and surviving inherited invocation leases remain uncertain.
The startup deadline does not bound disk calls or cleanup joins; the launcher has separate bounded
cleanup and reap, and never signals a successor or a reaped child.

Only the background worker clears/writes fixed `remote/serve-error.json`, admitted under Serving.
It is a bounded sanitized terminal failure record, not a log or health inventory. Duplicate starts
cannot clear it; writes/close precede lease release. Empty/missing/unwritable records are not proof
of healthy exit. Inspect the local file alongside `status`; use `serve --foreground` for terminal
troubleshooting. Do not reset state or automatically restart after uncertain startup.

Serve also owns the optional lease-bound object service: one attempt per static Local
declaration before door readiness, bounded by the same 250 ms absolute setup budget as
websocket demand, explicit shutdown after Site, and Drop on early exit.
An absent or refusing listener is silent at startup; unsafe, malformed and storage
failures still warn. Setup failure leaves the ordinary door running. Only Colab declares Local;
without its adapter's admission, operations are refused before ledger effects.
Validated Local websocket demand can reactivate an absent/ended channel;
failure still forwards without an origin. The [object-backend guide](object-backends.md)
owns the bounded single-flight lifecycle; ordinary discovery shapes are unchanged.

## Discovery and restart implementation

Serve remembers the bound port since Remote schema 5 (`door_port`). Schema 6
(`short_route_prefix`) regenerates an existing machine's non-credential route prefix
once, preserving its ID, key, origin-bound grants and port. The shared canonical
validator admits that format in store, routing and live status. The schema-5 fixture
proves migration persistence and rollback; stopped status still reads the schema-5 port
without migrating. Browser reopen adopts the current path.

Status resolves core's public `storage.root` once, then uses the running control
socket. With an absent or stale socket, it admits an existing private layout and takes
an existing serve lease before opening SQLite read-only. It never calls
`Store::open`, creates files, migrates old schemas or reads the machine key.
Pre-schema-5 state reports no remembered port. Unsafe files, malformed/silent
control replies and a held lease without reachable serve return errors. A serve
that answers the operation as unknown (alpha.1's `REMOTE_INPUT_INVALID` or the
current `REMOTE_CONTROL_UNSUPPORTED`) maps to `REMOTE_SERVE_OUTDATED` for both
ordinary status and stop, in `control::request_operation`. The optional
`status --machine --json` projection instead preserves the original unsupported code/message
without that restart advice, using the same control request/framing owner. Its exact four-key
live reply adds `Pairing::machine_id`, already captured by serve; it never opens Store or an offer.
The existing canonical UUIDv4 validator and origin/prefix rules validate that projection. Ordinary
live status keeps exactly three keys, and both projections keep the same two-key stopped result.
`status --objects --json` is a separate optional four-key running projection of Local channel
state, from a read-only view of the same service slots. It never triggers setup; stopped output
is unchanged. Door readiness is independent of channel readiness. `status --budget` adds the static,
dated Firestore free-plan limits table (no storage, no provider call). The optional projections
preserve unsupported peer errors. No HTTP descriptor or second status acquisition supplies a replacement hint; the
[owning contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery) defines its
best-effort observation and use-time authority limits.

Stop sends one control request to set serve's SIGTERM shutdown flag, then waits for
the lifecycle lease and verifies socket cleanup while holding that lease. It never
looks up or signals a PID.

Verify from `rust/` using disposable HOME/XDG and task-owned children:

```bash
CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test cli
CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test state
CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-extension-state
```

CLI cases assert exact status shapes, unchanged stopped-state bytes/mtimes/files,
same-origin restarts, occupied-port refusal and recovery, explicit port choices,
standalone human URLs, control stop and repeated idle stop, real paired-grant
survival across stop/serve, and listener/socket/child cleanup. State cases cover
non-creating reads, legacy schema without migration, damaged remembered ports,
and bounded lease-release confirmation. Optional machine cases compare the owned readiness
machine across isolated roots, preserve stopped bytes/mtimes/inventory, reject partial/invalid
IDs and distinguish original unsupported refusals from ordinary outdated-serve advice.
Readiness is the serve output event; no real account, model or core database is used.

## Browser opening and settings

Remote's `open` module supplies CLI flags and interaction to `tmt-invoke::open`,
the platform opener shared with Colab; Remote retains its own presentation. The
[contract](../../../../contracts/remote-channel-v1.md) owns which conditions suppress
an open. `settings` owns the private `settings.json` / `settings.lock` under Remote's
existing layout, independent of the database/serve lease. Missing settings use the default;
malformed settings use it with a human warning. Setters serialize through the bounded lock.

The [settings/device page authority](../../../../contracts/remote-channel-v1.md#remote-settings-browser-authority)
is separate from paired channel trust. Its native/SDK implementation reuses this settings owner and existing
device/session mutation owners; shared presentation supplies no authority. Settings semantics and
agent grants remain unchanged.

## Management implementation

The [fixed management protocol](../../../../contracts/remote-channel-v1.md#remote-management-protocol)
owns exact wire shapes, immutable outcome/deadline, first-touch uncertainty and recovery policy.
`management` composes the existing JSON settings writer and Store transaction-local device helpers.
Store owns local designation and bounded immutable management receipts. Device effects and receipts
commit in one SQLite transaction; `Devices` owns Session cleanup and events after releasing live/Store
locks. JSON settings and SQLite receipts are separate durability boundaries: uncertain writes are
never reapplied by receipt lookup. Live grant admission is required for all reads. Self-rename can reopen then read the original operation. Lost self-revoke acknowledgment
gets one fresh read-only admission attempt; refusal shows access loss plus unknown outcome and
`tmt remote devices`, never a resend or a committed-revoke inference. No old-key exception exists.
The shared-presentation `/settings` page composes the SDK for admitted forms, frozen outcomes and
original-ID reading; its separate page bundle imports the single served SDK. Shared CSS supplies presentation only. Management identity
limits are cumulative: 1000 per caller, 4000 installation-wide, including expired rows. The
30-day deadline bounds outcome availability, not row deletion. Capacity refuses new adoption
before effects; show the local settings/devices CLI path without automatic retries or storage
reset, and preserve any original unknown outcome. Compaction is deferred.

Focused native evidence includes `cargo test --offline --locked -p tmt-remote` (management storage,
signed admission, settings fault, deterministic management process interruption, CLI/state and existing lifecycle), plus SDK package check/test/build
and regenerated-byte equality. Use the existing isolated roots and the worktree-owned target.

The management interruption test runs the same effect owner in an owned child, reports actual
adoption/writer/transaction milestones, then the parent SIGKILLs and joins it before reopening
the root. Production uses a no-op observer; there is no runtime fault flag. Pending identity,
file uncertainty and device transaction rollback/commit are verified independently. The browser
fixture also kills/restarts its disposable serve after a signed management commit but before
acknowledgment, then verifies a fresh live Session's original receipt and designation. These are
process-interruption tests, not power-loss or complete product/release acceptance.
