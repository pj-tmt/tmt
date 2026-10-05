# Run the door, discovery and settings

The exact public documents, remembered-port and busy-port behavior, route-prefix
format, `stop` semantics and the browser-opening rules belong to the
[channel contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery);
this page holds how to run the door and what its implementation tests prove.

## Run the door

Build core, put `rust/target/debug` on `PATH`, then `tmt remote serve` (or `--json`
for the bound descriptor). Direct invocation requires an absolute `TMT_EXECUTABLE`;
it never searches for another core. Ctrl-C/SIGTERM or `tmt remote stop` closes
listeners, sockets and workers while keeping stored pairings and grants. Limits
are named in `src/limits.rs`.

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
status and stop, in `control::request_operation`.

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
and bounded lease-release confirmation.
Readiness is the serve output event; no real account, model or core database is used.

## Browser opening and settings

Remote's `open` module supplies CLI flags and interaction to `tmt-invoke::open`,
the platform opener shared with Colab; Remote retains its own presentation. The
[contract](../../../../contracts/remote-channel-v1.md) owns which conditions suppress
an open. `settings` owns the private `settings.json` / `settings.lock` under Remote's
existing layout, independent of the database/serve lease. Missing settings use the default;
malformed settings use it with a human warning. Setters serialize through the bounded lock.

The [settings/device page authority](../../../../contracts/remote-channel-v1.md#remote-settings-browser-authority)
is separate from paired channel trust. Its native/SDK draft reuses this settings owner and existing
device/session mutation owners; shared presentation supplies no authority. Settings semantics and
agent grants remain unchanged.

## Management implementation draft

The [fixed management protocol](../../../../contracts/remote-channel-v1.md#remote-management-protocol)
owns exact wire shapes, immutable outcome/deadline, first-touch uncertainty and recovery policy.
`management` composes the existing settings writer and Store transaction-local device helpers;
`Devices` still owns post-commit Session cleanup and events. Live grant admission is required for
all reads. Self-rename can reopen then read the original operation. Lost self-revoke acknowledgment
gets one fresh read-only admission attempt; refusal shows access loss plus unknown outcome and
`tmt remote devices`, never a resend or a committed-revoke inference. No old-key exception exists.
The actual settings page and complete feature acceptance remain pending.

Focused native evidence includes `cargo test --offline --locked -p tmt-remote` (management storage,
signed admission, settings fault, CLI/state and existing lifecycle), plus SDK package check/test/build
and regenerated-byte equality. Use the existing isolated roots and the worktree-owned target.
