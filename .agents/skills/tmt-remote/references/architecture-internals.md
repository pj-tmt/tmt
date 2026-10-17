# Remote architecture internals

Module owners (put a change in the existing owner; `canonical`, `crypto`, `wire`,
`transport`, `sealed_sync` (including `sealed_sync::codec`), `declaration`, `deploy_plan`, `hosting`, `rules`, `firestore_budget`, `firestore_limits`, `readiness` and `deploy_run` have no I/O, clock, storage or `CoreClient` access):

| Module                                           | Owns                                                                                                                                                                                                                                                |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `main`, binary-private `serve`, `core`           | CLI dispatch, one foreground/background composition and the two startup calls; `CoreClient` runs only fixed public `api`, `list --json`, `identity list --json`, `check <name> --json` via `TMT_EXECUTABLE`                                         |
| `http`, `routes`, `site`, `limits`               | Loopback door framing and bounds, `/r/` binding routes, route dispatch; every bound is named in `limits`                                                                                                                                            |
| `wire`, `canonical`, `crypto`                    | Strict JSON admission with exact payload bytes, framing/fingerprint codecs, signature and HMAC verification                                                                                                                                         |
| `session`, `admission`                           | `session.open`, one normal message in flight per session, durable sequence consumption and admitted permits                                                                                                                                         |
| `message`                                        | Shared signed-message receive service: session admission, operation/journal hand-off and signed refusal/audit responses; foreground composition owns the service, not a backend                                                                     |
| `transport`                                      | Pure envelope/result driver port (`append`, finite `subscribe`, explicit `ack`); loopback `routes` forwards to the injected common service, never owns admission/effects                                                                            |
| `journal`, `budgets`, `audit`                    | Metadata streams and recovery ownership, persisted budgets, audit written in the owning transaction                                                                                                                                                 |
| `operations`, `approval`                         | Dispatch/read operations over the public core API; local held-operation confirmation on the control socket                                                                                                                                          |
| `authority`, `store`, `state`                    | Typed grants, `remote.db` and schema history, layout/machine key/serve lock                                                                                                                                                                         |
| `pairing`, `control`, `devices`                  | One pairing offer per run, owner-only control socket for discovery/stop and device list/revoke/rename and explicit sending-scope toggles                                                                                                            |
| `mount`, `pages`                                 | Extension mounts (allowlisted extensions only), static landing/pairing/error pages and embedded stylesheet/SDK assets                                                                                                                               |
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

`operations` projects permitted `agents.list` rows from one public Core list read. It forwards
only validated `session.activity` state/sinceMs and samples one owner `observedAtMs` when the Core
list read returns; viewers clamp negative elapsed values to zero.
The SDK tolerates absent or malformed optional evidence. The [channel contract](../../../../contracts/remote-channel-v1.md)
owns the shape and advisory semantics; no Core state, extra read, scope or bound is added.

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
  use the default cap of 8 sessions per device; `off` is unlimited. `session`
  evicts only that device's most idle session without a live transport. When no
  detached victim can make room, a verified fresh open receives signed
  `REMOTE_SESSION_LIMIT`; attached sessions survive. The transport-count lock
  linearizes detached end with attachment; no new lock order or cap is introduced.
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

Database, selected sign-in providers and indexes precede the Rules release. Index allocation
counts unrelated live field configurations against the Spark ceiling and preserves existing
index/TTL settings. Google OAuth and uninitialized Auth remain fixed owner steps. Ruleset
lookup is bounded and compares exact source before another create; exhausted lookup refuses.
Rules switching requires a single writer for the project: no provider atomic precondition
exists. The adapter re-reads immediately before the last switch and checks full source after
it; visible drift refuses, while ambiguity after a possible effect is unknown with no binding.
Fake process/HTTP tests use a vendored layout stub and canary credentials, never real login.
Real account/project provisioning remains separately authorized acceptance, not fixture proof.

Automation previews and saves one draft identity without provider effects; a terminal may
confirm one captured plan, or continue its unfinished saved digest without another question. `deploy_cli`
owns the short preview (`--details` shows the full plan) and existing inventory observations;
`deploy_command` retains the inputs/record across consent, and `deploy_run` keeps
the creation/release fences. Only a new terminal confirmation may abandon a known or unknown
unreleased Version only while its release is pending; it never supersedes unknown web-app/site/config creation. Preview keeps a changed unfinished intent visible without refusing it;
`deploy_run` gates replacement before calls and archives accepted unfinished runs as `superseded`
in the additive `supersededRuns` list, preserving checkpoints and step outcomes. Missing history
remains omitted, so existing records read unchanged. Explicit digest authorization names the whole envelope; foreign
Rules replacement is covered by the whole plan digest. Sign-in providers are explicit inputs. Command
registration wires factories into `deploy_cli`, the single declaration/setup/login/record
sequencing owner; fixtures drive that same owner and the shipped binary.
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
Discovery pairs each admitted raw declaration with its validated bundle; `hosting` does not parse
those bytes. It composes their bounded file set and deterministic gzip/ServingConfig, including the
public declaration file owned by the [contract](../../../../contracts/remote-channel-v1.md#extension-backend-declarations),
and names captured site/web-app creation and foreign replacement in the same frozen envelope.
The contract owns its schema, bounds, route allow-list and non-atomic publication order.
The real port inventories Hosting and executes the frozen steps through the same deploy runner.
`hosting` computes the [live-content identity](../../../../contracts/remote-channel-v1.md#extension-backend-declarations).
The adapter captures it with prepared content and passes it only in its private helper request;
the helper uses its prefix for labels/read-back without a second hash implementation. Full
file/config checks remain; no frozen plan/record field or old-label migration is added.
The helper owns the [user-file inventory projection](../../../../contracts/remote-channel-v1.md#extension-backend-declarations)
for every staged/live Version check, including the two reserved init paths and retained-fingerprint fence.
An omitted `uploadRequiredHashes` is empty. Optional cause file counts contain no identifiers
and do not enter the frozen envelope or its digest.
Unsupported lazy factories refuse declared Hosting before login or record setup. Concrete stage
substeps save resource handles and the frozen envelope in the versioned checkpoint; v1/v2 records
remain readable without conversion. Unknown creates are reconciled by bounded identity lookup,
never repeated on uncertain absence for web-app/Version creates; other absent idempotent work
can reapply. App creation observes its Operation within the apply budget; pending work blocks
later effects and needs the same command rerun. Expired unfinalized stages require a fresh explicit
plan. Joint read-back alone saves a verified public configuration
and same-project web.app entry with Complete; the remote-link producer consumes that type later.
Release POST/read-back compares validated site/release-ID identity across the two documented live
name forms, retaining the received name. The [contract](../../../../contracts/remote-channel-v1.md#extension-backend-declarations)
owns exact admission and the forward-only record boundary; no mutation retry or extra read is added.
Site.appId selects a validated public app config after any recorded app identity. Fresh/existing
unassociated sites use the same explicit appId-only configure step; nonempty associations are never
repointed. Joint verification binds that association.
An omitted WebApp displayName is empty only inside an otherwise admitted app; malformed names refuse.
The contract owns app selection, partial
availability and the single-writer limitation. Fake-provider evidence is not real project/browser acceptance.
Installed Colab supplies its release declaration through the fixed public command; production never substitutes Remote fixture data.
`remote_link` derives the strict public descriptor, canonical fragment and corruption checksum
only from the record's typed verified Complete publication. Deploy output and the lock-free
re-print use that pure owner; no second save, schema version, repair, provider or browser call.
The contract owns the descriptor schema; partial/unknown or unverified records yield no link.
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
No-declaration output touches neither provider nor deployment record. Plan-only inventory
checks live plus planned field configurations and composites separately; allocation rechecks each ceiling.
An admitted Field indexConfig with omitted indexes has an empty list; malformed configurations refuse.
Installed Colab acceptance and real-project proof still require separate owner authorization;
running `status --layers` now reads recorded deployment evidence without a provider call.
`DeployRecordEvidence` takes one bounded lock-free snapshot per request; missing/draft is
empty, damaged state is unknown and never repaired. The pure `readiness::from_record`
projection recognizes finished database/sign-in steps, usable verified Rules bindings and digest-derived
composite step outcomes from the authorized canonical index set. Index states, legacy unknown handling and
copy are owned by the [status-layer contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery). A failed
run before Rules preserves its old binding; an incomplete Rules attempt reports partial
Rules and withdraws it; an incomplete possible Hosting release does the same. This describes recorded outcomes, not live provider currentness.

Plan tier and quota remain unknown because the machine cannot observe either, and layer-1
traffic goes browser to Firestore. Sharing therefore stays unknown after a complete recorded
run. A later Core readiness decision must define tier treatment for layers requiring no paid
plan and a quota evidence source before sharing can read enabled.

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
Readers never take `deploy.lock`: an opened snapshot is a complete old or new record, so a
long deployment cannot block status. Malformed, oversized or unsafe state is refused, never
reset. Save failure stops effects; a directory-sync failure after rename does not mean the
visible publication rolled back. Staging cleanup only removes admitted deployment staging
names under the writer lock; the shared entry reserves `.deploy.json.tmt-staging`, while
admission still cleans only the old admitted `.deploy-<uuid>` names. Existing status/stop, settings, store and machine-key owners
keep the deployment identity and binding. Bound records persist the immutable project/region
with the first authorized run; plan-only drafts stay unbound v1 records. The writer checks it
before provider creation, login or inventory. Project-local v1 previews are unbound; reads never
infer a region or convert a file. The old
single path is refused without migration. `remote_selector` owns explicit/default/local resolution;
`remote_catalog` applies each edit once to Document's freshly validated current bytes;
reads stay observational and private mode remains 0600. It owns public references, never
grants or deployment authority. The contract owns
[native selection](../../../../contracts/remote-channel-v1.md#native-remote-selection). A conflicting
target leaves record and staging untouched. Foreign Rules replacement consent is the whole frozen plan digest, including its live
fingerprint; the run refuses a changed foreign fingerprint before the Rules effect. The contract
owns the one-deployer/tenant boundary and single-writer-project limitation.

The Firestore settings projection takes its record snapshot outside live/Store locks and
revalidates the current browser grant before disclosure. Only the explicit opt-in read adds
layer/budget members; ordinary settings and status shapes remain unchanged. Browser display
uses strict typed projections, not another provider or readiness engine. Optional failure
clears old evidence independently of management access and original-effect outcome ownership.
