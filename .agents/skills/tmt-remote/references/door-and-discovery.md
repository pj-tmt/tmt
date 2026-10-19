# Run the door, discovery and settings

The exact public documents, remembered-port and busy-port behavior, route-prefix
format, `stop` semantics and the browser-opening rules belong to the
[channel contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery);
this page covers the local driver's door and implementation checks; its device trust is not a Firestore gate.

## Run the door

Build core and put `rust/target/debug` on `PATH`, then `tmt remote serve`. Human startup
returns after background handoff and links to the browser entry at `/`, not the protocol base. Use `tmt remote status --json` and `tmt remote stop` to inspect
and end that same owner. `serve --foreground` keeps terminal ownership; exact bare `serve --json`
also remains foreground for existing supervisors. Explicit `--background --json` detaches with
the unchanged protocol descriptor. Direct invocation needs an absolute `TMT_EXECUTABLE` and never
searches for another core. Stop preserves pairings/grants and cancels pending pairing/held work;
launcher departure after accepted handoff does not stop it. Limits are named in `src/limits.rs`.

`main` owns CLI grammar/dispatch; binary-private `serve` owns mode selection and one composition
over existing core/state/Control/Approval/events/HTTP owners. The exact native self-exec worker, the
private handoff (Ready -> Accept -> Accepted, the no-kill cutoff, bounded cleanup and reap, lost
acknowledgment) and the failure-record writer are the shared `tmt-extension-serve` leaf; the
[extension-serve contract](../../../../contracts/extension-serve-v1.md) owns those semantics for
Remote and Colab, so read it there. Remote supplies the Ready shape
and validation, its error codes and hints, the limits and the serve-error phase mapping.

Only the background worker clears/writes fixed `remote/serve-error.json`, admitted under Serving.
It is a bounded sanitized terminal failure record, not a log or health inventory. Duplicate starts
cannot clear it; writes/close precede lease release. Empty/missing/unwritable records are not proof
of healthy exit. Inspect the local file alongside `status`; use `serve --foreground` for terminal
troubleshooting. Do not reset state or automatically restart after uncertain startup.

Serve owns the lease-bound `ObjectService`; [object-backends.md](object-backends.md#modules)
owns startup/demand setup, degradation, single-flight budgets and shutdown. Only an admitted
Colab Local declaration enables it; ordinary door discovery promises no object readiness.
The [v3 carrier contract](../../../../contracts/remote-channel-v1.md#native-sealed-document-carrier) owns explicit
internal activation, verified factory inputs and restart/re-activation. Native refusal preserves
the existing Local channel and ordinary door; status/read/channel loss never activate it.

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

The private pairing offer carries `ownerPolicyVersion: 1`; every pairing flow requires it
before exposing the offer or confirming, otherwise the CLI reports `REMOTE_SERVE_OUTDATED`.

Stop sends one control request to set serve's SIGTERM shutdown flag, then waits for
the lifecycle lease and verifies socket cleanup while holding that lease. It never
looks up or signals a PID.

`restart --json --no-open` uses a read-only candidate check and the current owner's private
snapshot to prove background mode and window identity; home, lease, route and port reuse their
existing records. `restart-stop` checks the existing offer -> live -> Store owners and object
channel/transfer accounting, then sets the ordinary stop flag while the serve lease is held. Busy work
defers untouched; approval cancellation is unchanged. No new admission guard closes the narrow
check-to-stop race: newly arriving work may be interrupted as by ordinary stop. The
[channel contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery) owns outcomes,
exact-endpoint cutover, bounds and uncertainty. Foreground/legacy owners stay running. The
replacement gets one start after lease/socket cleanup, with no port fallback or successor stop.

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
device/session mutation owners; shared presentation supplies no authority. The contract owns settings
semantics and the explicit talk-scope toggle.

## Management implementation

The [fixed management protocol](../../../../contracts/remote-channel-v1.md#remote-management-protocol)
owns exact wire shapes, immutable outcome/deadline, first-touch uncertainty and recovery policy.
`management` composes the existing JSON settings writer and Store transaction-local device helpers.
Store owns local designation and bounded immutable management receipts. Device effects and receipts
commit in one SQLite transaction; `Devices` owns Session cleanup and events after releasing live/Store
locks. JSON settings and SQLite receipts are separate durability boundaries: uncertain writes are
never reapplied by receipt lookup. Live grant admission is required for all reads. Self-rename
preserves its Session, cookie and held work, then reads the original operation. Lost self-revoke
acknowledgment gets one fresh read-only admission attempt; refusal shows access loss plus unknown
outcome and
`tmt remote devices`, never a resend or a committed-revoke inference. No old-key exception exists.
The shared-presentation `/settings` page uses the served SDK and this management owner;
[browser-pages.md](browser-pages.md#settings-page) owns controls and captures. The
[management contract](../../../../contracts/remote-channel-v1.md#remote-management-protocol) owns retained-identity
caps, expiry, capacity recovery and original-outcome rules; no automatic retry or storage reset.

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

New pairings are read only. After the word-match confirmation, the terminal asks separately whether
to allow sending, defaulting to no. Owner-side `--talk` preselects sending and skips that second
prompt; bare JSON confirmation remains read only. `--agents <uuid,...>` and `--hold` require `--talk`
and narrow the issued policy, not enrollment. Existing grants are unchanged; no migration ships.
`tmt remote devices talk <clientId> on|off` changes only sending, preserving every other grant
field, and works through the current serve or under the stopped serve lease. It commits sanitized
audit metadata with the scope change; designated settings browsers use the existing immutable
management receipt/effect owner. Changed revisions end previous Sessions; revoked grants stay revoked.
A fresh signed open under the retained valid device key can prove revoked or expired authority;
unknown, deleted/corrupt keys and opaque replies remain unconfirmed. The SDK's bounded reopen
ends only on verified proof and offers re-pair, never resending an uncertain original effect.
Restart requires a new current-window open; it does not revive sessions or grant authority.

## Native Firestore sign-in and transport

The native `firebase_auth` library is separate from deployment login and local device pairing.
The [host-driver contract](../../../../contracts/remote-channel-v1.md#host-driver-authentication) owns that boundary. `firebase_auth` uses injected
HTTP, clock, jitter, cancellation and credential-store ports. `tmt remote sync sign-in`
and `sync revoke` compose them with the verified public selector; nothing opens. An active
sign-in is reused without a network request. The space owner admits a new identity in Colab.
Files are `remote/firebase-auth/<project>.json` plus
the project lock, with refresh credentials only. The Remote writer uses stage/file-sync/
rename/directory-sync under the lock and compares the captured credential before replacing
it. The shared state leaf's directory/lock admission stays strict; only a readable owned
credential file with loose mode is tightened to 0600, returning one notice for CLI composition.
Owned abandoned staging is removed and directory-synced under the lock on load/purge;
other projects stay untouched. Corrupt, unreadable, unsafe or project-disagreeing files
never trigger another sign-up.
Fixed limits live in `limits.rs`: 365-day access lifetime, idle expiry off, refresh 300–360 s
early, four 5 s attempts within 30 s and 30–900 s series backoff. Token timing is monotonic;
absolute credential expiry is checked before use and purges the secret on the next invocation.
Stopped clients clear memory tokens and run no refresh work. Ambiguous signup is not retried
automatically; an explicit later sign-in can create another identity. Revoke durably marks
revoking before one delete attempt; offline/unknown keeps that state for explicit revoke
rerun, with no normal token access. Delete/already-absent purges the local secret. No settings
keys, Keychain integration or claims of immediate invalidation of previously issued tokens.

Ordinary status adds `signIn` with `state` and nullable `expiresAtMs`. Unreadable/unsafe
credentials report `unreadable`, and a transient writer lock reports `busy`, with the existing
credential diagnostic line; both preserve door status and a normal exit. Explicit/default
selection errors still refuse. Expiring human copy
names the ISO UTC end date and says to sign in after that date, then admit the new identity.
Command JSON uses the full uid; human output abbreviates it to eight characters. Paired
device revocation remains `tmt remote devices revoke`, with cross-pointing help. A host
without a published Firestore configuration still reports its ordinary door status; an
explicit or configured-default selection failure never silently chooses another remote.
The production `UreqHttp` adapter uses fixed Google endpoints, one attempt with a global
deadline including body reads, no redirects or hidden retries, and each request's response
cap. Errors never retain provider bodies, bearer tokens or refresh credentials. System time
and random jitter are invocation-owned; cancellation fences bounded waits and late replies.

Focused proof: `CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test firebase_auth`
from `rust/`; it uses fake HTTP/time and real private files, not a project or Auth emulator.

The serving `AuthOwner` implements the frozen `AuthPort`, fixes one uid, coalesces token
requests and clips refresh attempts/waits to the caller's cancellation/deadline budget.
Native REST requests use the same injected HTTP boundary and canonical fields codec;
ordinary-user bearer tokens grant no owner-device or Admin authority. Provider pages are
bounded to one 397,312-byte carrier result. The native default is one document per query,
charged as three reads including Rules lookups; admitted fields fit 393,216 bytes. Smaller
document packing requires the leaf's actual encoded-size validator. Finite gaps or scope
mismatches expose no partial page. Create is immutable; put/delete require declared modes.
Create conflicts read back the same frozen target, never resend or perform a preparation read.

`native_sync` shares one attempted-read counter per Remote home/project/Pacific day under
`remote/native-budget/<project>.json` and one serving writer lock. It charges the bounded
maximum before each request, never refunds, and uses the existing model's 70/90-percent
warning/refusal and Pacific midnight reset. The serving owner schedules one shared minute
`NativeRuntime.checkpoint` tick even during provider backoff; request-time checks use that
same flush cadence. Persistence flushes dirty state on the tick and
when the serving owner calls `NativeRuntime.finish` at stop; abrupt owner loss can undercount only the last flush window. Unsafe,
corrupt or externally changed state is refused, never replaced with a fresh allowance.

Polling has a 5 s floor for 120 s after local publication or a received count change,
decays to 60 s idle, adds positive jitter and bounds transient backoff to 900 s. Probe
requests do not overlap within an interest. One `runAggregationQuery` COUNT over the exact
regular equality scope minus stream is precharged as 1 + the two shared Rules lookups.
There is no stream/seq filter, so counts cover initially empty pages and late seq-1 writers.
The first probe emits Changed without recording activity; a later count difference emits
Changed and records activity. Equal counts
suppress Changed until a ten-minute heartbeat bounds a delete/create swap; the heartbeat
does not reset activity. Only the last count and notice time are kept in memory; no persisted
fingerprint, stream cursor or delivery progress. Colab alone owns catch-up positions.
An empty finite query preserves its input cursor (including None). Provider quota exhaustion
retains unknown mutation outcomes and stops subsequent effects until the same reset.

The subscription runs no discovery scan. Consumers issue ordinary declaration-named
Discovery queries on Changed. Their signed stateless positions bind the last document,
scope, shape, client generation and original deadline; every continuation clips Auth/provider
work to that deadline. A consumer run admits at most 256 records and 257 one-document pages
including empty completion. Closing fences late results and releases the 3-consumer/12-serving
page-interest cap. One interest counts once regardless of writers; no document event queue.

Pessimistic reads per page interest/hour, before mutation traffic and backoff. Q is the number
of one-document query pages a consumer requests per wake, including empty completion and
consumer-issued Discovery. At the active floor every probe may observe a count change; idle
COUNT probes cost 180/hour and heartbeat catch-up adds 18/hour per Q. The heartbeat shares an
already due probe, not another COUNT request. The existing model gives active
`modeled_poll_reads_per_hour(1, 5000) * (1 + Q)` and idle
`modeled_poll_reads_per_hour(1, 60000) + modeled_poll_reads_per_hour(1, 600000) * Q`.

| Q per wake | Active probe + wake queries (5 s) | Idle probes + heartbeat queries (10 min) |
| ---------- | --------------------------------: | ---------------------------------------: |
| 0          |                             2,160 |                                      180 |
| 1          |                             4,320 |                                      198 |
| 8          |                            19,440 |                                      324 |
| 256        |                           555,120 |                                    4,788 |

Initial catch-up and extra consumer queries add their actual precharged maxima. No request
or wake refunds reads, and no periodic provider-owned Discovery cost is added.

The shared guard refuses before these worst cases can spend the modeled allowance; this
table grants no extra quota or claim about Google's measured usage. Native vectors pin
these costs, COUNT wakes and empty completion reads. A delete has no stream fields and conservatively
reserves its whole resource; other mutations reserve the declared stream, with 4 serving
writes total. Opaque document IDs are never parsed into a stream. No preparation read.

Ordinary-user `remote_selector::client` resolves an explicit reference, then the configured
default, then this host. Failed explicit/default targeting is a typed refusal. Imported
Firestore links need no local deploy record: `remote_link` returns only verified project
and public config, optional --project must agree, and admission still belongs to the space
owner. This resolver never grants deployment authority; deploy-only commands retain
`owned`/`existing`. Serve captures exact enabled release declarations via InstalledDeclarations,
validates their declaration/admission digests and injects the factory without provider I/O.
One selected project shares Auth/runtime, budget writer and a cancellation-wakeable minute tick;
factory calls only bind SyncContext and construct NativeClient. The transport library does not
activate the carrier. Checkpoint failure fences all project work. Stop fences activation/work
and refresh, closes/joins carrier/provider jobs, joins the tick, then calls `NativeRuntime.finish`
before releasing the budget writer and Serving lease; early setup failure uses owned cleanup. `budget_state` exposes the shared warning/refusal
without charging or making provider requests.

## Firestore deployment

The [owner runbook](firestore-owner-runbook.md) owns setup, terminal confirmation,
non-interactive authorization, recovery, completion checks and the stop-and-fix table.
The [deployment contract](../../../../contracts/remote-channel-v1.md#authorized-deploy)
owns captured inputs, provider diagnostics, partial outcomes and exact-plan recovery.
The [native selection contract](../../../../contracts/remote-channel-v1.md#native-remote-selection)
owns project/catalog/default resolution and legacy-state refusal; catalog commands change
references, not cloud resources or deployment history.

Implementation checks use fixed public declaration commands of enabled installed extensions;
missing/refusing declarations never select fixture bytes. Real-project acceptance requires
separate owner authorization. The [status-layer contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery)
owns recorded readiness; status is not a provider probe.

The [record evidence owner](architecture-internals.md#deployment-record) is shared by status,
Operations and opt-in signed settings. The snapshot is outside live/Store locks; management
revalidates the current grant before disclosure. Ordinary shapes stay unchanged.
