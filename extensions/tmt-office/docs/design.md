# Office v1 design

Status: local-first M1 semantics plus a separately scoped remote pilot proposal;
delivered SPA, Rules and pairing boundaries are described in
[architecture](architecture.md).
This document owns policy and user-visible semantics; the
[wire schema](../contracts/v1.schema.json) owns message shapes.

## Local-first M1

The local slice is installation-owned. Direct `tmt office layout show/apply`
commands use the existing local SQLite repository and need no Firebase
account, remote world, pairing, or running browser service. `tmt office start` is
an optional loopback UI for the same state, not a prerequisite for local content
commands. The local path does not publish work, dispatch tasks, call models or
join a remote world.

Local identity selection chooses the owner in the invoking user's database; it is
not a security boundary between processes sharing that user's OS account. Local
file/SQLite permissions and the user's account remain the trust boundary. Remote
pairing, leases, grant renewal and revocation retain their existing contracts and
remain separately scoped to the later remote slices.

## Remote pilot boundary

The remote pilot proposal is two people in one private world, each with an invited block
and optionally selected local TMT agents. They can visit, share bounded board
objects, opt into a finite social session and request revision-bound review.
Connecting blocks is not connecting databases or automatically trusting another
computer. Independently deployed worlds do not federate in v1.

Visitors need only a browser. One-shot agent decoration requires an installed
extension and valid scoped pairing/assignment, not a resident connector.
Continuous local event/work reception requires a foreground-running connector.
Credential renewal requires a defined scoped authorization contract. Native TMT remains usable without
Office, Node, Firebase or a background process. No agent is a human account.

The [data-only sandbox refinement](sandbox.md) owns community props, structured
exploration, temporary/saved identity assignments and trusted contextual notices.
Community content provides neither runtime code nor prompts. This refines the
decoration path; the remote-work protocol below remains separately authorized.

## Ownership and identifiers

| Entity          | Authority and scope                                                                                                        |
| --------------- | -------------------------------------------------------------------------------------------------------------------------- |
| Deployment      | One configured HTTPS origin and Firebase project; the operator controls the backend and can access stored plaintext        |
| World           | Opaque ID inside that deployment; private and undiscoverable without membership                                            |
| Human           | Firebase user authenticated in that deployment; Google sign-in is the initial provider choice, anonymous login is disabled |
| Membership      | Current world-specific grants and revision; checked server-side, not inferred from a cached token or invitation            |
| Block           | Belongs to one human in one world; its owner controls visitors' actions within world policy                                |
| Device          | Separate scoped Firebase device principal, paired to a human and world; never the human's own refresh token                |
| Published agent | Opaque world-scoped ID mapped locally to a selected database identity UUID and device; names and avatars are presentation  |
| Exchange        | Opaque world-scoped shared request ID; maps to one durable local dispatch record and native request/attempt                |

Local names remain unique within their selected TMT database. Two owners can
both display an agent named `alice`; routing uses the published agent ID, not
that string, a pane number, a folder, or a socket path. A retired temporary
identity must not be rebound by name to a new agent. Unpublishing prevents new
work and retires that publication ID permanently. It rejects queued work that
has no dispatch ID; work already assigned is reconciled without claiming it was
cancelled. Republish creates a new publication ID, never revives the old queue.
Membership/device revocation follows the same no-revival rule. Retained exchange
history remains subject to current access rules.

Google sign-in is a scoped PoC default, not a requirement to replace existing
TMT provider support. Additional identity providers and enterprise federation
require a later authentication slice. A self-deployed operator supplies their
own auth configuration; no project IDs or credentials ship in source.

## Authorization, not proximity

### Implemented private-world boundary

World creation and reads use the authenticated Firestore client directly, not
an HTTP/Admin world service. `worlds/{worldId}` owns immutable world metadata
and `ownerUid`; there is no redundant owner membership record. Rules require
verified Google authentication, current operator-managed `testers/{uid}` admission
and resource ownership. Client writes to tester grants are always denied.
Operators edit `{ enabled: true }` in Firebase Console; missing/disabled/malformed
grants deny world access. Authentication itself is not blocked by this gate.

See [private world document v1](../contracts/private-world.md) for
the exact create/read and retry contract. Future blocks and messages live in
world subcollections, not unbounded arrays on the root. Those paths currently
deny all client access except the owner-only `blocks/home` decoration slice
specified in [block v1](../contracts/block-v1.md) and scoped UUID blocks
under [agent grant v1](../contracts/agent-grant-v1.md). Grant enforcement
is complemented by the local [pairing issuer and browser approval](../contracts/pairing-v1.md)
and the implemented [native pairing contract](../contracts/native-pairing.md).
Invitations and device/work operations below remain
future design; they do not justify a generic backend for ordinary world storage.
The initial owner-only world does not implement visitor memberships or presence.

| Action             | Minimum authorization                                                                        | Explicitly does not grant                                                      |
| ------------------ | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Visit/read a block | Active membership and block visibility grant                                                 | Chat, board edits or work                                                      |
| Chat               | Visit plus chat grant and recipient/session consent                                          | Tool use, workspace context or infinite replies                                |
| Edit board         | Visit plus board-edit grant for the object/block                                             | Other owners' objects or local file changes                                    |
| Request work       | Visit plus work-request grant, recipient policy and queue admission                          | Immediate execution or a GitHub approval                                       |
| Execute locally    | Fresh server dispatch permit plus paired device and current local agent/capability allowlist | Arbitrary shell, paths, host tools or another agent                            |
| Manage world       | Human world administrator                                                                    | Another owner's device credentials, private local data or execution permission |

All grants are deny-by-default. A membership role is administrative convenience,
not a replacement for resource-specific checks. The guard/receptionist is an
optional routing aid with the same grants as any agent; it cannot mint grants.
Visitors' chat, board text, review text and decorations are untrusted content.
They cannot change policy, invoke hidden commands or authorize disclosure.

This is a protocol permission boundary, not a claim that a prompt sandboxes an
already-running coding agent. Publishing a work capability exposes the selected
agent's existing local tool policy; the owner must see and accept that risk.
Use an actually isolated workspace/provider permission profile when narrower
execution is required, and refuse publication if the required isolation cannot
be established. Never implement a tool restriction using prompt wording alone.
Tool-free social chat needs a separate context with no workspace tools; reusing
a full-privilege tmux conversation cannot satisfy that requirement.

Cloud Functions/HTTPS handlers are needed for invitation redemption, scoped
device pairing, admission and dispatch permits. These are trusted operations,
not a generic backend wrapping every Firestore read. Privileged SDK calls bypass
Firestore rules, so these handlers must perform the same current membership,
resource and device checks themselves and use restricted IAM. Browser direct
reads/board writes remain constrained by rules and bounded queries. No client
may directly issue or enlarge membership, device grants, dispatch permits or
final status. The implemented agent-grant contract permits only one-way owner
revocation through client Rules, not client issuance or renewal.

World operators are trusted with cloud-visible data; v1 is not end-to-end
encrypted. Do not upload private repository content by default. Request previews
show the exact outgoing text and recipient; publishing an agent is not consent
to upload its whole conversation. Results are locally export-gated. Logs omit
tokens, request/final bodies and sensitive local paths.

## Invitations and pairing

Invitations are targeted to an existing authenticated UID, scoped to a world
and explicit grants, valid for 24 hours and redeemable once in a transaction.
No public directory or bearer link that grants membership by itself. Expired or
revoked invitations reject without revealing world details. Renewing creates a
new invitation; it cannot revive a redeemed/revoked token. Supporting email-only
invitations later must handle verified email/account changes explicitly.

Pairing is a separate authorization from installation and login:

1. The connector creates a cryptographically random 256-bit secret locally and
   carries only its SHA-256 challenge to the browser approval page. There is no
   unauthenticated durable begin write. An approved pairing expires in five minutes.
2. It shows a short comparison code and opens the configured deployment's pairing
   page. The code is an identifier, not sufficient to claim the device.
3. The logged-in human chooses a world and sees the device label/fingerprint,
   selected local identity and exact requested resources/capabilities. They approve explicitly. No auto-approval from
   an agent message, a URL parameter or a claimed owner UID.
4. The originating connector proves possession of its secret and claims the
   approval as one logical assignment. Original-proof retries within the same
   window reuse its principal/grant and never renew its deadline; see
   [pairing v1](../contracts/pairing-v1.md). The service creates a distinct principal for the selected
   installation/identity/world binding and returns a
   short-lived Firebase custom token; only that device exchanges it for its own
   credentials. Neither the pairing URL nor ordinary output contains bearer or
   refresh tokens. Custom-token issuance stays server-side.
5. This scoped resource approval does not publish work capabilities. The local
   owner separately opts into work publication; its default is empty.
   Credentials use an adapter-owned protected store, never
   the app config, source tree, process arguments or logs. The credential adapter must verify the
   selected platform store and secure fallback before use.

Pairing polling is bounded (at least five seconds between polls), idempotent and
stops on expiry/revocation. Incorrect proof cannot consume another pending
pairing. Device principals cannot create invitations, approve pairings or act as
humans. Membership and device revocation are live database checks; Firebase token
expiry alone is insufficient. Access policy tests must distinguish human and
device principals even when both have a valid Firebase token.

## Presence and admission

Browser presence, device connectivity, agent availability and work state are
separate fields. A 30-second renewal with a 90-second lease is the initial
low-frequency budget; timestamps come from the service. A stale observation is
`unknown/offline`, not proof that a local pane died. Animation interpolates
movement intentions without per-frame writes or model calls.

Receiving policy is per block: `closed`, `receive_only` or `open`.
It restricts guests, not the block owner's administrative access.

- Closed denies new work and new visits; existing authorized participants may
  still retrieve their retained exchanges. Closing never discards queued work.
- Receive-only admits bounded queued requests but issues no execution permits.
- Open may issue permits if the recipient/device/local policy permits execution.

The initial queue limit is 32 unfinished shared requests per recipient block.
Unassigned rejection/expiry, definitely-failed delivery or an observed native
final (including withheld export) releases capacity once in a service transaction.
Uncertain work still counts until explicit owner resolution releases its slot;
that action neither cancels native work nor authorizes replay.
Admission requires an online server transaction and explicit server acknowledgment;
Firestore offline writes or a local optimistic UI are not accepted work. A caller
can save a local draft, but it cannot silently send it on reconnect.

Schedules use an IANA timezone and half-open local-time intervals, evaluated
against authoritative UTC time. Nonexistent spring-forward minutes never open;
repeated fall-back minutes match both occurrences. Overnight intervals explicitly
span into the next day. Manual closure wins over the schedule. Admission and
dispatch each recheck current policy, so a queued request can remain pending
after the door closes. No new dispatch after its deadline.

### Proposed movement contract

Movement is not implemented; [#179](https://github.com/wkh237/tmt/issues/179)
owns the proposed contract for one authoritative local trajectory per identity
UUID, with revision/map revision, route/waypoints and service-derived timing.
The browser interpolates without per-frame persistence or model calls. A retarget
samples the prior trajectory at acceptance and validates destination/obstacles
before replacement; client coordinates/time never assert authority and stale
revisions conflict. Exact grammar, grid/collision, clock/restart and acceptance
tests remain refinement work; movement never implies work availability or starts
a conversation automatically.

## Dispatch and local exchange reuse

Shared admission/routing is not a second native task engine. It records who may
request work and where it was routed. Native `RequestService` remains the owner
of actual delivery, immutable final responses, attention and retention.

Discussion-board behavior is tracked in [#211](https://github.com/wkh237/tmt/issues/211),
with shared guidance in [#213](https://github.com/wkh237/tmt/issues/213). Board
posts are distinct domain content from core exchange records: a post never
implicitly creates, replies to or acknowledges a native `x` record. Reuse the
transport and storage infrastructure, not the request state machine; those issues
own the detailed acceptance.

| Shared routing state | Who advances it and what it means                                                         |
| -------------------- | ----------------------------------------------------------------------------------------- |
| `queued`             | Service accepted immutable request fields and reserved queue capacity                     |
| `dispatching`        | Service atomically issued one dispatch ID to one active device; not evidence of execution |
| `linked`             | Assigned connector reported its durable local mapping and native delivery evidence        |
| `uncertain`          | A dispatch may have crossed the local effect boundary; never authorize automatic replay   |
| `rejected`           | Service or authorized connector definitively refused before local delivery                |
| `expired`            | Service proved no dispatch was issued before the admission deadline                       |

Completion is a separate projection: no final, shared final available, export
withheld, or shared content expired. `sent` does not mean running or successful;
an immutable final does not itself assert success. Native `x` acknowledgment
continues to mean handled, not cancelled. Remote read/ack revisions, when added,
must not silently acknowledge native exchanges or replace native attention state.

The connector projects typed service observations, not CLI output/error documents:

| Native observation                  | Shared projection                                                                        |
| ----------------------------------- | ---------------------------------------------------------------------------------------- |
| `Prepared` with committed mapping   | `linked`, evidence `prepared`; not delivered                                             |
| `Sending` without a settled outcome | `uncertain`; no automatic resend                                                         |
| `Sent`                              | `linked`, evidence `sent`; no success assertion                                          |
| `Uncertain`                         | `uncertain`; later existing evidence may resolve it                                      |
| `DefinitelyFailed`                  | `rejected` only with authoritative definitely-unsent evidence                            |
| Stored immutable response           | Independent available/withheld completion; never a synthetic successful execution status |

Reports accept only public IDs and bounded evidence/reason codes. Native failure
JSON, receipt proofs, endpoint details and raw exception strings never cross this
boundary. Service handlers verify the assigned device, permit and allowed state
transition; valid schema alone does not prove evidence. These are authenticated
device observations, not independent remote attestation: a malicious owner/device
can fabricate its own report. The UI must not imply that the cloud verified the
agent's actual reasoning or execution success. A withheld result may
become available after explicit export approval, using the same native final,
without another dispatch. Once content is published it cannot be replaced.

Client-generated exchange IDs are random and scoped by world. Repeat submission
of the same ID and exact immutable fields returns the existing request. Any
different actor, recipient, body, revision binding or expiry is a conflict, not
an overwrite. The service derives the actor from verified authentication, never
from payload labels. Authorization is rechecked on retries before revealing data.

Dispatch requires a fresh online service permit bound to world, exchange,
assigned device, agent, policy revisions and expiry. It references the current
publication ID and monotonic membership/device/local policy revisions. A stale
generation cannot be renewed into a new dispatch. Only one dispatch ID is issued
for an exchange in v1. Do not automatically reassign it after a lease
expires: the old device may already have delivered. Recovery without proof is
`uncertain` and requires explicit human resolution/new request.

The connector must durably reserve the dispatch ID, exact request and native
request/attempt IDs **before** effectful delivery. The mapping and native prepare
must share the existing storage owner's transaction boundary. Mark sending before
transport using the existing lifecycle. A crash before that transition can resume
only the same prepared attempt; a crash after it never automatically re-sends.
Existing local state can be projected/uploaded again without repeating transport.
No network operation, tmux delivery or model call occurs inside a retried cloud
transaction. At-least-once network delivery is expected; distributed exactly-once
execution is not promised.

Remote orchestration requires a typed seam and adapter-owned transactional
correlation mapping around the existing request service. Current native prepare
is not a remote-idempotency API. Do not shell out to `tmt talk`, scrape stdout, open
raw SQLite from the bridge, or duplicate SQL/lifecycle policy. Its crash fixtures
must prove the commit/effect windows above before remote execution is enabled.

Native reply receipts are unkeyed local correlation, **not remote credentials**.
The connector never sends receipts, pane IDs, socket paths or local database IDs
to visitors. It reads actual stored finals and exports approved content only.
Native reply deadlines/90-day retention remain owned by Rust, not overwritten by
Office's queue expiry. A late native final may still become available after the
remote UI stopped waiting. Duplicate identical final publication is idempotent;
different final content conflicts and cannot replace the first result.

## Revocation and failure semantics

Revocation blocks new reads/admission/permits/result uploads at the backend as
soon as committed and checked. Already disclosed content cannot be recalled.
An issued permit is a bounded capability with a maximum 15-second validity;
the connector also checks its live local policy immediately before sending.
A disconnected device gets no new permits. There is an unavoidable race between
revocation and an already-authorized local side effect: v1 does not claim to
cancel running agent work or retroactively revoke a delivered message.

Stopping a connector prevents future dispatch and reporting, not existing native
work. On reconnect it reconciles its original dispatch IDs and stored finals;
no bulk re-send. Re-pairing creates a new device identity and does not inherit
old dispatch authority. Revoked devices cannot upload old results with a cached
token. Results remain local until the owner explicitly resolves export/access.

The API uses `401` for missing/invalid authentication, `403` for denied current
authority, `409` for conflicts/version or state mismatch, `410` for known expired
resources, `413` for bounds, `429` for rate/queue limits and `503` for unavailable
service. Resource existence must not be disclosed before authorization; denied
visitors get a generic denial instead of differentiated expiry/not-found details.
Errors never imply cancellation or safe replay unless definitely-not-dispatched
evidence exists. Clients keep the same exchange ID after an ambiguous submit.

## Bounds, retention and social context

These are conservative PoC defaults, not existing CLI settings:

- Work requests: at most 4,096 Unicode scalar values and 16 KiB UTF-8; shared
  finals: at most 16,384 scalars and 64 KiB. No silent truncation or blob URLs.
  An oversized local final is `withheld`, preserving the complete local result.
  Admission and the connector each validate wire bounds before native prepare;
  final export validates before upload. Native exact-text validation remains
  unchanged (currently 1 MiB). These are explicit narrower transport bounds,
  not a second implementation of native text policy. Invalid scalar encodings
  reject; empty and whitespace-only exact text remain valid as in native TMT.
  Request overflow returns `413`, never a partially delivered request.
- Request queue deadline: one hour; permitted range one minute to 24 hours.
  Queue expiry and result retention are independent, and never cancel local work.
- Shared content retention: 90 days from acceptance (request) or first final
  publication (result). Identical retries do not renew it. Rules/service reject
  access after expiry even if TTL physical deletion has not happened.
- Idempotency tombstones outlive content by 24 hours and contain only correlation
  and terminal metadata. Identical replays carry the original absolute deadline
  and reject after logical expiry, including after physical cleanup. The dedup
  guarantee is bounded by record retention, not an infinite history: after
  tombstone deletion a changed deadline cannot be compared with old fields.
  Clients must never reuse an ID or silently turn an expired retry into a new
  submission. An explicit new request uses a fresh random ID and new admission.
- Blocks: the implemented [home block v1](../contracts/block-v1.md)
  accepts 16 curated objects on a 32x32 grid within a 64 KiB ceiling,
  rendered with repository-controlled SVG/CSS. Custom props require the versioned
  sandbox successor; no arbitrary artwork is accepted by the current codec.
- One opted-in social session per agent: at most 10 minutes and 20 agent turns,
  with a caller-configured cost ceiling. Zero workspace tools/private context by
  default. No automatic social session merely because agents stand nearby.
- Whiteboard v1: bounded independently owned objects with revision-checked writes,
  not a full CRDT editor or an unbounded world document. Object limits and the
  drawing library require a concrete contract before implementation.

Browser caches are memory-only for private Office data by default. Logout or
membership loss clears views and subscriptions; persistence requires an explicit
trusted-device choice in a later slice. Cached data never authorizes execution.
Firestore TTL is eventual cleanup, not a deadline, lock or permission boundary.

## Revision-bound reviews

A review request identifies repository owner/name, PR number and the exact
40-character head commit. A changed head makes feedback stale for the new head.
The agent may return preliminary findings; no automatic GitHub comment, approval,
merge or assertion that the human owner endorsed them. Fetching private code and
publishing feedback each require their own existing access/authorization. Never
upload a local repository simply because a remote visitor named its URL.

## Verification boundary

Schema validation proves structural compatibility only. The scenario vectors in
`extensions/tmt-office/contracts/scenarios.json` define required outcomes, not evidence that
services, rules or connectors enforce them. Their actual implementations require
causal authorization, crash and revocation tests. Cloud activation and billable
services require separate operator authorization; this protocol provisions nothing.

## Primary sources

- [Firestore rule conditions](https://firebase.google.com/docs/firestore/security/rules-conditions): privileged server clients bypass rules; rules are not query filters.
- [Firestore transactions](https://firebase.google.com/docs/firestore/manage-data/transactions): callbacks may rerun and client transactions fail offline.
- [Firebase sessions](https://firebase.google.com/docs/auth/admin/manage-sessions): token lifetime and revocation are distinct from current application grants.
- [Firebase custom tokens](https://firebase.google.com/docs/auth/admin/create-custom-tokens): a trusted server can authenticate a distinct device principal; signing credentials stay server-side.
- [Firestore TTL](https://firebase.google.com/docs/firestore/ttl): deletion is asynchronous; logical expiry must be enforced separately.
