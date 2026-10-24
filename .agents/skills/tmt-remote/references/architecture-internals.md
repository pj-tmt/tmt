# Remote architecture internals

Module owners (put a change in the existing owner; `canonical`, `crypto`, `wire`,
`cloud_hpke`, `transport`, `sealed_sync` (including `sealed_sync::codec`), `declaration`, `deploy_plan`, `hosting`, `rules`, `firestore_budget`, `firestore_limits`, `readiness` and `deploy_run` have no I/O, clock, storage or `CoreClient` access):

| Module                                           | Owns                                                                                                                                                                                                                                                |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main`, binary-private `serve`, `core`           | CLI dispatch, foreground/background composition and startup calls; `CoreClient` runs fixed public API, list/identity/check and receipt-bound reply via `TMT_EXECUTABLE`                                                                             |
| `http`, `routes`, `site`, `limits`               | Loopback door framing and bounds, `/r/` binding routes, route dispatch; every bound is named in `limits`                                                                                                                                            |
| `wire`, `canonical`, `crypto`                    | Strict JSON admission with exact payload bytes, framing/fingerprint codecs, signature and HMAC verification                                                                                                                                         |
| `cloud_hpke`, `cloud_keys`                       | Fixed native HPKE descriptors/carriers and consuming response contexts; private Document keyring renewed at serve startup and existing Site maintenance; no transport, peer authority or replay ledger                                              |
| `cloud_pairing`                                  | Native invite/join, proofs, mailbox and uid routing; Store confirmation/receipts under the serve lease; shared project Auth/runtime/budget                                                                                                          |
| `session`, `admission`                           | `session.open`, one normal message in flight per session, durable sequence consumption and admitted permits                                                                                                                                         |
| `message`                                        | Common signed-message receive service: session admission, operation/journal hand-off and signed refusal/audit responses; foreground composition owns the service, not a backend                                                                     |
| `transport`                                      | Pure envelope/result driver port (`append`, finite `subscribe`, explicit `ack`); loopback `routes` forwards to the injected common service, never owns admission/effects                                                                            |
| `journal`, `budgets`, `audit`                    | Metadata streams and recovery ownership, persisted budgets, audit written in the owning transaction                                                                                                                                                 |
| `operations`, `approval`                         | Dispatch/read operations over the public core API; local held-operation confirmation on the control socket                                                                                                                                          |
| `authority`, `store`, `state`                    | Typed grants, `remote.db` and schema history, layout/machine key/serve lock                                                                                                                                                                         |
| `pairing`, `control`, `devices`                  | One pairing offer per run, owner-only control socket for discovery/stop and device list/revoke/rename and explicit sending-scope toggles                                                                                                            |
| `mount`, `shell`, `pages`                        | Allowlisted mounts, bounded display-only describe composition, static pages and embedded assets                                                                                                                                                     |
| `reads`, `reads::ops`                            | Owner browser reads: fixed public `ops`/`digest` argv, single-flight 5 s reuse, Ops allowlist projection ([contract](../../../../contracts/remote-channel-v1.md#owner-browser-reads)); public CLI only, no state or worker                          |
| `objects`                                        | Object backend trait and `LocalFs`: the `objects.db` ledger and extension-private payload trees under the serve lease; the channel and config belong to `object_service`                                                                            |
| `declaration`, `deploy_plan`, `hosting`, `rules` | Strict backend declaration parse; digest-addressed `sharing` deploy plan; allow-listed Rules/indexes and frozen Hosting composition (bytes in; installed discovery belongs to `deploy_discovery`)                                                   |
| `deploy_command`, `deploy_record`, `remote_link` | Plan/authorization/output over captured inputs/provider; private deployment identity/run file, atomic replacement under its writer lock; lock-free per-request status evidence                                                                      |
| `remote_catalog`, `remote_selector`              | Public references and explicit/default/local targeting; imported configuration never grants access or ownership                                                                                                                                     |
| `sealed_sync`, `sealed_sync::codec`              | Bound sealed-document/Auth ports and pure context, restricted REST fields, query/interest/mutation and cloud-manifest validation; no provider, credential storage or I/O                                                                            |
| `firestore_budget`                               | Free-plan Firestore budget model and client guard; vectors in `tests/fixtures/firestore_budget`                                                                                                                                                     |
| `firebase_auth`, `sync_cli`                      | Remote-private refresh credential file, bounded ordinary-user ureq effects and monotonic lifecycle over injected ports; command/status composition, ID tokens in memory, no shared-state change                                                     |
| `native_sync`                                    | Injected ordinary-user REST effects, adaptive polling, project stop fence, serving/consumer caps and attempted-read persistence; no document decryption or Core authority                                                                           |
| `calendar`                                       | Pure Gregorian day conversion shared by Pacific quota resets and UTC sign-in status dates                                                                                                                                                           |
| `firestore_limits`                               | Dated free-plan Firestore limits table and its `status --budget` projection, validator and human lines; golden in `tests/fixtures/firestore_budget/limits-member.json`                                                                              |
| `readiness`                                      | Layered Firestore readiness: one table of items, reasons, sentences and human next steps; record projection, validator and human lines over the evidence source shared by status and opt-in signed settings                                         |
| `deploy_cli`, `deploy_discovery`, `deploy_tools` | CLI argv/composition; fixed public installed declaration replies; installed Firebase launcher/Node discovery (no receipt parser or mutable helper file)                                                                                             |
| `deploy_firestore`                               | Real provider port; binary-embedded Node helper, version-gated firebase-tools login, bounded no-retry mutations, timed/cancellable composite checks and exact Rules read-back                                                                       |
| `deploy_run`                                     | Authorized Firestore sharing deploy over an injected `DeployPort`: envelope digest, authorization, step order, record and binding rule (CLI composition in `deploy_cli`)                                                                            |
| `serve::native_factory`                          | Selected public config and enabled release declarations; one shared Auth/runtime/budget writer and minute checkpoint, project fence and joined final flush; v3 opt-in                                                                               |
| `activation_cli`                                 | Hidden host activation protocol through the existing CLI boundary; deadline-bound root lookup and one owner-control exchange, no state creation or provider construction                                                                            |
| `object_service`                                 | Lease-bound `ObjectService`: initial/demand single-flight Local setup, readiness, origins, admitted observation/upload; v3 bindings/awaiters share setup and dispatch; serve injects the shared native project factory; production Colab-only Local |
| `tmt-extension-objects` (leaf)                   | Remote-owned protocol leaf, consumed by Remote `object_service` and Colab `object_channel`: canonical IDs/encodings, object and v3 sync DTOs/bounds, strict JSON and the Unix carrier; no backend, policy or Remote/Colab types, and grants nothing |

`operations` forwards Core list evidence; the [operations contract](../../../../contracts/remote-channel-v1.md#operations)
owns activity validation, post-read observation time and advisory decoding.

Local device trust belongs to the [loopback driver](../../../../contracts/remote-channel-v1.md#transport-binding-loopback-http),
not a base authentication layer ahead of Firestore. The implementation below remains unchanged;
provider Auth/Rules and Colab content admission keep their separate owners.

Rules that are easy to get wrong:

- **One message receiver.** Foreground `serve` constructs one `message::MessageService`
  over the existing session and operation owners and injects it into the local `routes`
  driver. Ingress direction is driver → message → session/admission/operations, never
  a backend-specific verifier. The pure `transport` port only carries exact bytes and
  untrusted origin metadata; signed refusals and effect uncertainty stay in the common
  owner. A finite subscribe does not acknowledge; only the explicit signed ack does.
- **One opener.** `store::Store` opens only with the `state::Serving` proof of the
  serve lock. Pairing and device commands reach state through serve's control
  socket; `tmt remote devices` without serve takes the lock itself.
- **Authority lives in the transaction.** An effect runs in `Store::effect`: it
  rereads the persisted grant (revision, liveness, scope, recipients, expiry)
  inside an IMMEDIATE transaction and performs the core call under that fence. A
  new check outside the transaction is a race with revocation.
- **Revoke respects the fence.** SQLite authority writers wait at most
  `limits::AUTHORITY_WAIT` (5 s), then refuse contention without a mutation.
  Effects remain inside their IMMEDIATE transaction; a successful revoke follows
  its release. A timeout never means an in-flight effect was undone or may be resent.
- **Uncertainty keeps identity.** The `dispatching` audit row commits after the
  core call; recovery uses the adopted frozen intent and core's idempotent
  operation ID (`dispatch.show` before any `dispatch.create`) and never infers
  "no effect" from a missing row. Fixed read operations bypass journal adoption;
  they retain signed admission, sequence and call budgets. Recovery observation settles only a
  newly learned definitive original outcome. Stable polls never write ownership or entries,
  retry or send. A full stream drops its oldest entries (`make_room_for_entry`), and a full
  ownership table drops read leftovers and the oldest finished records
  (`make_room_for_operation`); held, dispatching and uncertain records are never evicted.
- **Serve lease.** Serving deliberately remains a specialized File-held guard: the shared
  Guard explicitly unlocks on drop and cannot retain invocation descriptors. Holder metadata
  is unavailable and means unknown; never infer authority from a PID. `Serving::retain_for_invocations` clears close-on-exec on the
  lock file so invocation children inherit it; closing never unlocks. A restart
  refuses while an orphaned child lives; unconfirmed cleanup disables writes until
  a fresh lease-owning run. The lease test is a real-process SIGKILL probe in
  `core_tests.rs`.
- **Pre-auth stays generic.** Refusals before a verified signature are one 404
  with no inventory. A verified fresh open may receive the contract's signed
  rejected-open envelope without allocating a Session or sequence row. Fresh-open stored-key
  proof checks retained valid authority before classifying disabled/expired, rechecks identity
  under live -> Store locks, reserves the nonce and charges Call. Ordinary admission still
  requires a live grant; unknown/deleted/corrupt keys remain opaque. The `tmt_door` cookie
  only identifies a device context on mounted paths; `/r/` refuses cookies. `session` retains
  live tokens plus one latest-token alias per device, rebinding that pointer to the deterministic
  current-revision survivor described in the
  [contract](../../../../contracts/remote-channel-v1.md#signed-envelopes).
  A new cookie retires the superseded alias; authority loss or no survivor deletes device mappings.
  A transport-only `tmt-session` identifier must belong to that cookie device; it is stripped before forwarding.
- **Mount trust.** Mounted extensions share one trust domain behind the door.
  The static Remote-owned short-entry table forwards public Colab entries without
  resolving sessions or adding device context; SDK mount lookup uses the same table.
  The device context header is added only for a live owner session and is never
  copied from a client. Session lifetime counts successful upgraded transports;
  last-close touches the session, starting the short inactivity grace for every session
  without a live transport. Reattach resumes that session; detached sessions count against
  the cap until expiry. Live-transport and no-transport idle limits belong to `limits`;
  the door maintenance loop persists expiry cleanup. `session` owns
  per-session replay and device-wide authority loss. Held work belongs to the grant and
  survives session end; only stop/revoke/expiry/revision change cancels it. Rename changes presentation
  without a revision bump or cleanup. Private `session::EndReason` owns wire end codes; revision
  changes and orderly shutdown have distinct bounded notices, never revocation proof. The journal/ack
  remain per device.
- **Session cap.** `session.open` rereads `settings` on each open. Unset settings
  are unlimited by default; explicit positive caps retain the same behavior. `session`
  evicts only that device's most idle session without a live transport. When no
  detached victim can make room, a verified fresh open receives signed
  `REMOTE_SESSION_LIMIT`; attached sessions survive. The transport-count lock
  linearizes detached end with attachment; no new lock order or cap is introduced.
- **Device cleanup.** Store owns durable last-use observations and atomic inventory-checked removal; public lists exclude revoked rows without removing event tombstones.
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
library-only and consumed by Remote, Colab and Ops:

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-extension-state -p tmt-extension-serve)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-colab --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-extension-state -p tmt-extension-serve -p tmt-remote -p tmt-colab --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
```

Synced publication tests prove filesystem behavior, not power-loss recovery.

The wire leaf `tmt-extension-objects` is library-only, consumed by Remote's
`object_service` and Colab's `object_channel` (policy-guarded; no other Remote file).
Colab consumes neutral wire types, never Remote implementation types. Its Unix-only
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
has its existing separate bound. Hosting has separate 30-second preparation, mutation-reply and
read-only reconciliation budgets under a 90-second apply bound. An ambiguous effect gets one
exact read-back, never another mutation. The pure runner persists the adapter's closed cause/phase
and optional HTTP class on unknown steps; absence keeps old record bytes and version unchanged.
Human/JSON project that metadata; neither diagnostics nor helper bytes enter the plan digest.

The [shipped deployment contract](../../../../contracts/remote-channel-v1.md#native-remote-selection) owns step ordering,
index/Rules fences, owner-only console actions and single-writer limitations. Fake process/HTTP
tests use a vendored layout stub and canary credentials, never real login or project acceptance.

`deploy_input` owns backend-neutral manifest selection, unresolved planning defaults,
registered option/help support and fixed missing-input details, with no I/O or persistence.
`error::RemoteError` publishes that detail through the ordinary error envelope and exit-1
boundary. The [input contract](../../../../contracts/remote-channel-v1.md#deployment-input-manifest)
owns semantics; backend integration remains separate from this shared model.
`deploy_cli` owns argv, short/full preview and terminal-only `progress`; `deploy_command`
retains captured inputs across consent; pure `deploy_run` owns transitions and persistence.
Progress notifications do not change effects or records; the joined heartbeat clears before
final output/errors. The [authorization contract](../../../../contracts/remote-channel-v1.md#authorized-deploy)
owns interactive consent, digest authorization, supersession and creation/release fences;
the [owner runbook](firestore-owner-runbook.md) owns commands and recovery steps.

`deploy_discovery` enumerates trusted enabled mounts and invokes their fixed public
`deploy-declaration --json` commands through supplied `TMT_EXECUTABLE`, with a neutral cwd,
explicit environment allow-list, a 10 s cold-launch bound and existing waited cleanup.
The [Remote contract](../../../../contracts/remote-channel-v1.md) owns the strict reply
schema, byte bounds, digest rules and no-declaration codes. Discovery captures
and validates exact UTF-8 declaration/artifact bytes and both digests once, then uses the pure
plan/Rules owners. No installed-root/receipt parser or artifact-path read exists in Remote.
An old unsupported command means no declaration; malformed/failed replies are unavailable.
Optional Hosting capture uses the fixed `hosting-bundle --json` command after declaration
validation and before provider setup, with a separate bounded stream and waited child cleanup.
Discovery pairs each admitted raw declaration with its validated bundle; pure `hosting`
composes captured bytes, deterministic gzip and ServingConfig. The [composition contract](../../../../contracts/remote-channel-v1.md#extension-backend-declarations)
owns the public declaration file, content identity, app/site selection, staged handles, optional
diagnostics, user-file projection (including reserved init paths) and forward-only recovery.
Rust `hosting` alone hashes content identity; the adapter supplies it privately to the helper,
which uses it for labels/read-back without a second hash implementation. The same deploy runner
executes Hosting and Rules, and joint verification alone records Complete.
`remote_link` derives descriptors only from that typed verified publication; the
[link contract](../../../../contracts/remote-channel-v1.md#extension-backend-declarations) owns read-only reprinting and refusal.

For offline declaration-vector regeneration, run from `rust/`:

```sh
CARGO_BUILD_JOBS=2 cargo run --offline --locked -p tmt-remote --example compose_firestore -- /absolute/envelope.json /absolute/staging
```

The input is the contract's six-field reply containing exact declaration/artifact strings.
Staging receives `firestore.rules`, `firestore.indexes.json` and `plan.json`; stdout is a stable
JSON digest summary. No provider, credentials or product CLI is involved. Colab reviews and copies
these files beside its vector as `deploy-declaration-v1.firestore.rules`,
`deploy-declaration-v1.firestore.indexes.json` and `deploy-declaration-v1.plan.json` in its own
`extensions/tmt-colab/contracts/vectors/` directory. Remote's `deploy_vectors` test reads that
home only; an absent vector reports pending, a present vector needs the complete golden trio.
Remote-private fixtures under `tests/fixtures/deploy_vectors/` qualify the entry without implying
Colab provenance. The contract owns the parser boundary and input/output identity rules.

The offline Hosting view includes the declaration file without another input. For that view only: `CARGO_BUILD_JOBS=2 cargo run --offline --locked -p tmt-remote --example compose_hosting -- /absolute/declaration-envelope.json /absolute/hosting-bundle-reply.json`; refusals emit one stderr code and no stdout.

`deploy_tools` resolves the installed Firebase launcher's realpath and exact supported package
layout, then resolves Node from that launcher's absolute or env-node shebang, excluding
relative PATH entries. The embedded helper's compatibility gate still precedes credentials.
No-declaration output touches neither provider nor deployment record. The
[composition contract](../../../../contracts/remote-channel-v1.md#shipped-layer-1-composition) owns bounded
index inventory/allocation; live evidence is not quota usage. Only an admitted Field indexConfig
may omit its indexes as empty; malformed configurations refuse.
`DeployRecordEvidence` takes one bounded lock-free snapshot per request. Pure
`readiness::from_record` and the SDK share the [status-layer contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery)
for factual outcomes, legacy/missing/damaged evidence, binding withdrawal and guard-based
tier/quota satisfaction. Status never probes, repairs, measures headroom or checks a runtime guard.

The default-off `test-emulator` Cargo feature is private proof support: from the isolated checkout, build `CARGO_BUILD_JOBS=2 cargo build --offline --locked --manifest-path rust/Cargo.toml -p tmt-remote --bin tmt-remote --features test-emulator` into that checkout's `rust/target`. The shared `firebase_auth` HTTP constructor routes only with both `FIREBASE_AUTH_EMULATOR_HOST` and `FIRESTORE_EMULATOR_HOST` set to loopback `host:port` and a `demo-` project; partial/unsafe inputs or unknown URLs refuse typed, with no redirects, ambient proxy or official-host fallback. Serve prints one fixed activation line on stderr. Default builds ignore the variables; feature builds without them stay production. The never-ships guard pins default/dependency/release-command exclusion; Colab owns the integrated emulator proof. Its sign-in command is `tmt-remote --remote <name> sync sign-in --project demo-<id> --json`; sign-in/revoke reject `--no-open` by design because neither opens a browser.

The native `emulator_artifact_is_the_exact_verified_deployment_output` test runs the existing
algorithm through the fake provider and real record store, binding its captured Rules bytes
and independent marker/body digest to `tests/fixtures/rules/deployed.rules`. The emulator
loads that exact artifact for member admission, expiry, isolation and create-only refusal;
it does not prove project/database/Auth provisioning, login, billing or quotas.

Project-local `deployments/<project>/deploy.json` and the shared home `deploy.lock` belong to
Remote's private layout, separately from the
serving lease and database. The writer retains a fail-fast `Guard` for a plan/run; contention names its recorded holder
(or explicitly unknown);
`Document::update` re-admits the private current bytes and deployment identity under its
bounded write lock, then owns staging, fsync, rename and directory sync. The CLI supplies
its exact selected retry command, including authorization/output flags, without storing it.
Readers never take `deploy.lock`: snapshots see a complete old or new record, never block on
the deployment, and refuse malformed/oversized/unsafe state without reset. Save failure stops
effects; a post-rename directory-sync failure does not prove rollback. Staging cleanup removes
only admitted deployment names under the writer lock: `.deploy.json.tmt-staging` is reserved
by Document; legacy cleanup admits only `.deploy-<uuid>`. The [selection contract](../../../../contracts/remote-channel-v1.md#native-remote-selection)
owns deployment identity/region, legacy-path refusal and catalog targeting; `remote_selector`
resolves targets and `remote_catalog` applies each edit once to Document's freshly validated
bytes. References remain private 0600 data, never grants or deployment authority. Foreign
replacement consent/fingerprint and the single-deployer boundary stay in the [deployment contract](../../../../contracts/remote-channel-v1.md#authorized-deploy).

The Firestore settings projection takes its record snapshot outside live/Store locks and
revalidates the current browser grant before disclosure. Only the explicit opt-in read adds
layer/budget members; ordinary settings and status shapes remain unchanged. Browser display
uses strict typed projections, not another provider or readiness engine. Optional failure
clears old evidence independently of management access and original-effect outcome ownership.

## Native cloud crypto

The [fixed profile](../../../../contracts/remote-channel-v1.md#end-to-end-operation-encryption)
owns bytes and lifecycle. `cloud_hpke` reuses `canonical::envelope_header`, strict `wire` parsing
and `crypto` verification; only its carrier uses `hpke` and `aes-gcm`. Request and response contexts
are single-use values. `cloud_keys` publishes admitted state only after Document succeeds, preserving
its prior admitted keyring on uncertainty. Its named bounds live in `limits`; the existing Site
maintenance callback renews without adding a worker and a failure never stops local pairing.
`cloud_inbox` is the native carrier driver: `document` admits provider fields/createTime;
`provider` reuses `NativeRuntime` Auth/HTTP/budget; `ingress` alone constructs a verified hand-off into
`MessageService`; `driver` reconciles one machine query with a durable machine continuation; `sender` freezes before
HTTP and recovers a lost context with a fresh signed ID lookup. `store` uses the authoritative Store
connection/lease and existing journal effect owner, never a second replay ledger. Permanent carrier
refusals share that row and the existing audit transaction. Attempted response uncertainty cannot
block later recovery; pre-claim transient failures keep the affected carrier pending. The journal owns typed
owned-read provenance; the message layer preserves it internally without changing signed wire bytes.
The existing checkpoint owner also saves one machine-wide query continuation in `cloud_transport`
(`kind=checkpoint`, reserved non-UUID `peer_id=inbox-query`, empty `id`). Its bounded private bytes retain the query lower
bound, earliest provider snapshot time, exact indexed timestamp and document-ID tie-breaker after
each durable outcome. A capacity stop, transient failure or restart resumes that prefix without
changing operation authority or replaying an effect. A fully drained query clears the continuation
and advances the machine timestamp; the next query uses the normal overlap. Legacy peer-only
checkpoints seed the first query except the machine UUID, which belongs to enrollment. Inbox completion never updates those peer keys; a quiet peer cannot hold subsequent machine progress back.
The first query always probes one document. A full page can use bounded follow-up pages, sized by
`store::cloud_inbox_page_limit` from existing retained rows, without skipping any overlap timestamp
or ID. `provider` bounds retained response bytes by the admitted page count; `limits` owns the cap.
Empty idle polls retain their one-minute cadence and one-document reservation. `driver` observes
an already-attempted response by exact ID after its current grant fence, avoiding another unsent
commit reservation; ambiguous observations preserve the same frozen uncertainty and progress rules.

`cloud_inbox::finals` binds pushed finals to the original claimed outbox destination and local
originator under the current grant fence. Core's reply receipt is private binding data, never
envelope or audit data. `CoreClient` submits exact stdin to ordinary receipt-bound `tmt reply`;
Core keeps immutable acceptance and retention. The joined native worker observes mapped local
Core finals and uses the reverse inbox with one deterministic-ID, frozen/claimed carrier.
Each reconciliation makes one local Core request read per pending accepted dispatch, bounded by
`journal::OPERATIONS` per peer. Reads can repeat on each reconciliation until the final's
publication claim is persisted. These are local Core reads, not Firestore polls.
Permanent per-operation refusals are reported without starving later mapped finals; shared
transient/budget failures retain their stop behavior.
Core's message contract (#4831, `contracts/remote-message-v1.md`) owns the versioned schema.
`cloud_command` admits the local request and preserves the target echo. Its optional reply capability
is retained with the destination and originator in `cloud_inbox::store`'s existing private outbox
before publication can be claimed. The association is immutable across repeats and restarts;
the receipt never enters dispatch payloads, envelopes, answers or audit. Core retains final authority.
A target may carry only a name;
`Operations` resolves it with Core's public `identity show --json -- <name>` before adoption, freezes
the returned UUID/current name, and applies the same recipient grant and UUID-pinned effect check.
Missing/ambiguous names are durable per-document refusals. Explicit UUID targets never perform a
name lookup, and adopted targets never resolve again. The UUID-pinned check uses public
`identity show --json --expect-name=<expected-name> -- <uuid>`; Core owns normalization and the
atomic name/lifetime check. Changed identities map to `REMOTE_AGENT_STALE`, absent UUIDs to
`REMOTE_AGENT_NOT_FOUND`; offline/unknown presence is not a refusal. Native pairing composes it in
`serve`, sharing `native_sync::projects::Projects` with extension sync. `message_cli` admits bounded
prepared bytes and reaches `Service::send` through the existing owner-only `control::cloud_message`
exchange. The serving owner alone opens state and composes transport; ordinary control caps remain
smaller. The Sender distinguishes fresh-write refusals from original-request recovery; the CLI keeps
missing/invalid owner answers unconfirmed. Core preparation, receipts and replay ownership do not move.
The [contract activation section](../../../../contracts/remote-channel-v1.md#public-native-agent-activation)
owns the handoff and uncertainty boundary.
No enrolled peer means no provider poll. Legacy reciprocal discovery keeps one negative timestamp
per project/peer in existing `cloud_transport` checkpoint storage. A missing machine row is read
at most once per `limits::CLOUD_PEER_DISCOVERY_RETRY_MS` (24 hours), including after reopen; a later
successful observation still verifies the native pin before saving a connection. This cache grants
no authority and changes no inbox/enrollment cursor. The contract owns all bounds.

`cloud_pairing::wire` owns domain-separated invite/reroute capabilities. The Store persists the
owner's hash with grant/receipt/route adoption; the private joiner connection retains the separate
reroute token. Provider publication commits route changes, capability changes and an optional
child uid-only update together. The reciprocal token uses the same response key and existing
framing with an explicit reverse direction part, retaining forward bytes. Confirmation stores
the inviter reverse connection/token and joiner grant/hash in their respective existing transactions
before clearing the key. No second capability ledger or pairing ceremony is introduced. The native Rules asset owns invitation/owner gates and immutable
inviter provenance; common native possession, generation and effect fences remain unchanged.
`deploy_firestore::owner` reuses ordinary Auth and control/serve-lease identity access for the
explicit digest-bound owner-access step. The helper uses the authorized deploy credential for
conditional registration/recovery; Verify never writes. The contract owns predicates and bounds.

`tests/cloud_hpke.rs` pins RFC 9180 A.3.1; `tests/fixtures/cloud-hpke-reference.mjs` independently
checks application framing/signatures/AAD/response bytes only, without implementing HPKE.
Run it with Node and the focused Rust `cloud_` tests. `cloud-hpke-v1.json` contains public test keys
and frozen pre-change v1 descriptor/keyring bytes, not a historical release fixture.

The explicit actual-Core receipt/result proof is
`node extensions/tmt-remote/rust/tmt-remote/tests/fixtures/cloud-final-core-acceptance.mjs`
from the repository root, with Node 26 and offline builds of `tmt-cli`, `tmt-remote` and the
existing `runtime-caller-fixture` example. It reuses the native CLI sandbox and verified archive
fixtures to prepare a real Core remote request, then selects one signed/HPKE final-ingress test
with that private reply handle. The public Core reply/result commands verify exact empty and
Unicode/NUL bodies, duplicate/reopen behavior and immutable conflict. The original outbound
extension call is captured; this proves the receipt/result seam, while the two-machine emulator
qualifies carrier publication and reciprocal permissions. Neither fixture changes Core storage
directly, links Core into Remote or supplies a production activation shortcut.

The explicit native carrier qualification is `cargo test -p tmt-remote --features test-emulator --test
cloud_inbox_emulator -- --ignored --nocapture` from `rust/`, in an Infra-approved emulator slot with
both emulator hosts set to loopback and the fixture's `demo-cloud-inbox` project. It has no real-provider
fallback; without the explicit feature the test contains no executable case. Fixture setup alone seeds
registrations. The maintained ordinary-user driver uses `ProductionHttp`, common admission and the journal.

## Native pairing

`cloud_pairing` separates byte proofs from local-owner confirmation. Its distinct transcript binds
project, owner UUID/key, offer/challenge/generation/expiry, peer UUID/key, authenticated uid, name
and candidate nonce. One-shot reroutes sign the target machine UUID/key, peer, new uid, generation
and timestamp; the provider createTime supplies the 60-second freshness reference. It reuses local pairing MAC, possession, fingerprint and receipt-proof primitives.
A verified proof alone grants nothing. `pairing_cli` adapts the owner-only control socket to explicit
terminal consent; `cloud_pairing::service` owns policy and reconciliation, and its `provider` extends
the ordinary-user carrier provider. Byte proofs have no I/O; none of these modules prepares Core work.

The additive `cloud_pairing` migration uses the existing Store/lease. Explicit owner confirmation
atomically saves the default read-only grant (or the same ceremony's explicit talk choice), original receipt/proof and desired route. Exact lost-join
observation needs possession plus the saved candidate digest before the original expiry; it retains
no raw code or response key. The sender reserves its counter durably before signing. A native-key proof with a strictly greater
generation replaces a uid only under the current grant fence, without changing permissions. Revoke
marks routes for cleanup inside the existing revoker transaction.
Provider publication must apply each peer's desired route swap atomically; acknowledgments compare
project/uid/peer/generation/revocation, so stale completion cannot hide newer changes. Offline work
stays pending. This state is not a second operation ledger.

`native_sync::Projects` owns one cached Auth/runtime/budget per project, shared with extension sync.
Only explicit pairing can sign in. A replaced persisted uid changes the native Auth owner while
keeping the runtime/budget; old extension consumers stop without inheriting membership. The joined
pairing worker reuses finite inbox reconciliation and stops/flushes before the serve lease is released.
Machine registration pins the native key and inviter provenance; authorized uid recovery preserves both.
The same hourly enrollment maintenance also collects this machine's expired invite gates through
one-result expiry queries and the existing operation cap/deadline. Strict gate admission and the
shared Auth/budget precede deletion; failed cleanup cannot replace a ceremony outcome or grant
expired authority. A next explicit invite reuses gate-only cleanup best-effort for a first interrupted
offer; peerless projects still start no provider polling. The contract owns mailbox paths, bounds,
recovery window and route proof.

Pairing qualification is `cargo test --offline --locked -p tmt-remote --features test-emulator --lib
ordinary_user_pairing_rules_restart_reroute_and_revoke -- --ignored --nocapture` from `rust/`.
Both emulator hosts must be numeric loopback addresses, in an Infra-approved slot. It uses ordinary
Firebase Auth plus `ProductionHttp`; Admin installs Rules only. The injected fake Core query keeps
the receiver side independent of an installed recipient Core. The qualifier also runs built public Core
talk through the real Remote CLI/control sender; composed public final delivery is #4540 and real
accounts remain #4539 acceptance. Default tests
keep this qualifier ignored and cover local ceremony/reopen/revoke with injected provider ports.
The library and binary test roots share `test_isolation::isolated_case` for native serve,
factory and two-owner pairing fixtures. It executes before project files or locks open, so sibling
fixture-publication forks cannot inherit their credential/budget locks; `tmt_invoke` bounds the
child, captures failures and reaps its process group. The ceremony keeps its assertions and deadlines.
