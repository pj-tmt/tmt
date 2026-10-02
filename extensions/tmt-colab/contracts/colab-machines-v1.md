# Colab machine-sender amendment v1

**Status: proposed, not implemented.** Owner: tmt-colab-lead. This amendment
defines the first bounded machine-origin path within the existing colab space.
It requires colab-owner, tmt-remote-lead security and tmt-lead core review before
merge. It authorizes no runtime, dependency, deployment, provider configuration,
release, migration, live experiment or additional member.

Parent: [#478](https://github.com/wkh237/tmt/issues/478); owning design:
[#828](https://github.com/wkh237/tmt/issues/828#issuecomment-5932303929);
docs slice: [#863](https://github.com/wkh237/tmt/issues/863). The
[colab-v1 contract](colab-v1.md) owns existing object bytes, strict cryptography,
membership, device chains, epochs, page disclosure/retention, sync, pairing and
the bridge ledger. The [public extension API](../../../contracts/extension-api.md)
owns core dispatch, request/final resources and retention. Those definitions
remain authoritative unless an explicit proposed extension below names a change.
The terms MUST, MUST NOT and SHOULD express future implementation requirements.

## Outcome and owners

Machine A explicitly sends to one agent on B through a page in their shared
Firestore colab space. A signs and appends an intent to A's own stream. B's
existing bridge adopts it into its local ledger, holds for local approval,
dispatches through public core API, then publishes authenticated operation metadata
in B's own bridge stream and recipient-only encrypted receipt/final payloads.
A's wait/result observes
that correlation. There is no additional relay, machine inbox transport,
conversation database, pairing system or request/reply owner.

The first usable network target is the same Firestore space. Runtime work remains
colab-owned and starts only after colab L1-L6 local acceptance; Firestore F1/F2
comes next, then Cloudflare. The amendment can be merged as documentation before
those implementation gates. It does not make either network backend usable.
Cloud public mode remains excluded. Core stays invocation-owned, daemon-free and
reachable only through the invoking `$TMT_EXECUTABLE` public process API.

This first path permits an **explicit local Send on A** and is **hold-only on B**.
The browser's post-readiness preference in colab-v1 does not authorize machine
direct mode. [#600](https://github.com/wkh237/tmt/issues/600) must have its reviewed
readiness implementation accepted before a later machine-direct proposal may
enable it; merging a design or observing an idle/online cue is insufficient.
Autonomous agent permissions, source-agent identity claims and CLI spelling are
deferred. A model response, page script, sync, replay, restore or compaction MUST
NOT create a send action or approve one.

## Current gap and principal separation

Colab-v1's `bridge.add` grants only send-state/reply publication for that bridge's
own ledger entries. It cannot author send intents or edit content. Its device
certificate grammar admits member/link issuers, and its pairing grant is bound
to a browser device key. Neither permits a native machine sender. This amendment
proposes an explicit principal and typed grant rather than treating those existing
bindings as interchangeable.

| Principal kind | Key authority                              | Allowed machine-path use                                                          |
| -------------- | ------------------------------------------ | --------------------------------------------------------------------------------- |
| `device`       | Existing member/link device chain          | Existing colab-v1 roles/grants, unchanged                                         |
| `machine`      | New owner-signed `machine.add` binding     | Own signed intents only; no content, human-device certification or bridge results |
| `bridge`       | Existing owner-signed `bridge.add` binding | States/results only for operations in that bridge's ledger                        |

A machine can independently hold sender and bridge bindings. Shared machine ID,
Auth UID, page key or endpoint does not transfer authority between them. Principal
kind and key binding MUST agree in stream admission, wrap resolution, grant lookup,
intent verification and result verification. Do not resolve a bare ID/key by trying
other principal kinds. Ambiguous bindings reject; grant lookup never falls back
from machine to device. For this first path, keys and writer IDs across different
principal kinds MUST be distinct; no implicit same-key dual role is offered.

### Owner-signed machine binding

Propose two new operations in the existing root-signed membership log:

| Operation        | Strict typed payload / meaning                                                                                                                                                                           |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `machine.add`    | `machineId, senderId, signKey, encKey, pages`; generated IDs and raw Ed25519/X25519 public keys use colab-v1 admission; nonempty sorted unique page list, at most 256; enroll a machine-sender principal |
| `machine.remove` | `machineId, senderId, cuts`; remove that binding, commit both existing namespace cuts as applicable, and atomically advance affected page epochs with baseline/wraps                                     |

`senderId` is a separately allocated writer UUID bound to the source machine,
not a browser `deviceId` or B core identity. The resolved principal is
`(spaceId, kind=machine, machineId, senderId, signKey)`. This is a direct root
binding; it does not mint a `device.cert`, delegate certification or give a member
role. `pages` grants only scoped page admission and own intent publication.
It confers no local-agent permission on any machine.
Existing bindings cannot be overwritten or re-keyed by another `machine.add`;
replacement requires explicit removal and a fresh sender ID/key binding. Do not
reuse a removed sender ID or resolve its operations through the replacement.

Only the existing root owner's `tmt colab` signs these statements. The root key
does not move to A, B, browsers or cloud storage merely to add a sender. Adding
a machine requires that owner machine online and explicit owner management.
Any management transport remains with colab's existing owner-management boundary;
this proposal adds no cloud signing authority or pairing ceremony. Existing
clients that do not support these operations fail closed on the unknown log
operation; no compatibility or shipped-capability claim is made.

Machine addition under the no-history default uses the existing atomic page
epoch/baseline admission. `device.revoke` remains a device operation, not a
machine removal alias. Machine exclusion uses `machine.remove` plus epoch
rotation; B-local grant revocation remains the immediate effect-control path.
Removal cuts and key distribution follow colab-v1's revocation/freshness limits.

### Explicit stream and wrap admission extension

To avoid pretending `authorDevice` already admits machines, propose a separate
machine object header, with domain `tmt-colab-machine-object-v1` and ordered LP
fields:

```text
version, suite, space, page, epoch, kind, namespace, object,
authorKind, authorMachine, authorId, membershipRevision,
streamSeq, prevEnvelopeHash
```

`authorKind` is exactly `machine`; `authorMachine` and `authorId` resolve the
current `machine.add` binding. All other fields/bounds retain colab-v1 meanings.
The header stays within its existing 1,024-byte bound. Existing object-key,
signature and envelope-hash constructions consume these exact header bytes;
AEAD and signatures therefore bind the kind, machine and writer. No unsigned
Firestore document field chooses a principal. Machine stream ID is `senderId`
within `(space, page, epoch, authorKind=machine)`; contiguous update chains,
create-only acceptance, checkpoint/cut commitments and frozen envelope retries
retain their existing owners. Backend scoped writer projections MUST include
principal kind and binding ID; signed validation at the client/bridge remains
required even when Rules admitted ciphertext.

The machine's admitted namespace is `own` only, with root `intents` only in this
first path. Existing page/thread references confer no write authority. The caller
selects an existing admitted page/thread context; machine thread/content editing
is not added. A machine update/checkpoint touching `content`, messages, threads
or replies MUST reject before application. B continues using its existing bridge
own stream and restricted result role; this is not another bridge namespace.
Foreign Yjs decoding retains colab-v1 isolation and untrusted-output validation.

Extend the existing signed wrap recipient grammar with explicit
`recipientKind=machine`, resolving `recipientId=senderId` and X25519 key only
through `machine.add`. The existing wrap domain/field order and owner HPKE sender
signature remain unchanged; add new enum/admission vectors. Unknown kinds reject.
This does not reuse a `device`/`bridge` wrap as a machine wrap. Only currently
eligible machine bindings receive new epoch keys; no old-history key is granted
implicitly. A shared epoch key still allows page data disclosure as colab-v1
states; principal restrictions authenticate accepted writes, not selective
confidentiality among holders of that key.

## Destination-local agent authorization

The owner explicitly creates a grant **locally on B**, selecting A's verified
machine identity, page scope, allowed B agent UUIDs and expiry. No browser token,
remote caller or unsigned server index can issue it. Keep `colab-pair-v1` unchanged:
the root log already pins the machine sender, so no second enrollment exchange,
offer/code/transcript or machine variant of a browser-device grant is needed.
The grant is a new typed local record, not a widened existing device grant:

```text
grantId, requesterKind=machine, spaceId, pageIds,
sourceMachineId, senderId, senderKey,
destinationMachineId, agentIds, issuedAt, expiresAt,
revision, disabled, mode=hold
```

IDs, key encodings, sorted unique lists and timestamps use colab-v1 rules.
Page and agent lists are nonempty and at most 256 each; expiry is finite and
later than issue time. Grant duration and admission quotas require explicit
policy selection; do not infer the browser's 30-day pairing default for machines.
Persist in B's existing extension-owned local grant store atomically before
reporting success. Grant issue/narrow/revoke and dispatch fencing share B's
existing authority lock/transaction boundary. A neither reads B-local grants nor
copies their IDs/revisions. At adoption B requires exactly one unambiguous matching
machine-typed grant for the exact source principal/key, page and agent, and records
its ID/revision in the ledger. Recheck that same record at the effect fence; never
fall back to another grant or silently adopt a newer revision after revoke/change.
Changed authority refuses the pending send; a new send needs a new explicit local
action. This slice adds no revised-grant reapproval policy.

B resolves the exact typed record, source principal, pinned key, self destination,
space/page, agent, revision, expiry and disabled state at effect time. A grant
must not outlive source removal as observed by B, and a replaced key/ID requires
a fresh explicit grant. Immediate local revoke wins against later adoption or
approval; already committed core work is not undone. A signed log reduction made
elsewhere takes effect only when B verifies it. “Latest verified” is not proof
of global freshness; disclose withheld-revocation limits rather than claim instant
remote exclusion. The owner can revoke B's local grant immediately.

## Frozen machine intent and adoption

Explicit local Send on A displays exact UTF-8 final bytes, destination machine,
agent UUID, page/thread and validity. A pins the expected B bridge binding from
the verified log at Send time (`bridge.add` for B and that page). If none exists,
show waiting for B and do not publish an intent; a name/transport endpoint is not
a substitute. Local source selection or
formatting finishes before signing. No source-agent attribution is fabricated.
A persists one pending operation and its frozen input before publication; it is
an intent/receipt reference, not another core request or conversation history.

Propose `tmt-colab-machine-send-v1`, a typed variant of colab-v1 Send. Ordered LP
fields are:

```text
version, space, page, originalEpoch, thread, messageIds, machine, agent,
operationId, finalBytesDigest, senderKind, senderMachine,
senderId, issuedAt, expiresAt,
coreInputDigest
```

`senderKind` is exactly `machine`; `machine` is B and `senderMachine` is A.
`senderId` replaces the browser-only `senderDevice` field in this explicitly
new domain; the browser `tmt-colab-send-v1` bytes are unchanged. `messageIds` uses
the existing framed list. Digest fields are raw SHA256. Transport contains exact
binary `{intent, signature, finalBytes, coreInput}` inside A's encrypted own
intent. Signature is by A's resolved machine-sender key, not a bridge key.
`originalEpoch` uses the existing positive epoch grammar and is immutable.
`intentDigest` is SHA256 of these canonical intent bytes. Different field, key,
operation, source/destination or principal kind cannot share authority.

`coreInput` is the exact serialized UTF-8 public API request frozen on A:

```json
{
  "version": 1,
  "operation": "dispatch.create",
  "originator": "anonymous",
  "input": {
    "operationId": "<frozen operation UUID>",
    "recipientIds": ["<B agent UUID>"],
    "message": "<exact reviewed final UTF-8>",
    "kind": "request"
  }
}
```

The illustration is not valid credential/fixture input. Strictly validate the
decoded request against the signed fields and final bytes, with no additional
recipient, room, identity, operation or unknown field. Hash the original bytes;
B MUST pass that same frozen request to core without reserialization/reformatting.
Respect core capabilities' JSON/input and exact-message limits independently of
colab envelope/chunk limits. Reject unsupported capacity before publication.
Sender-machine authentication is colab provenance, not authority to impersonate
an identity in B's same-user core API.

Intent validity retains colab-v1's one-hour default/24-hour maximum. Neither
offline waiting, receipt observation, grant update nor local approval renews it.
At original adoption B requires signed originalEpoch = authenticated outer epoch
= current verified page epoch, and verifies page/own stream, machine principal
and signature, final/core input digests, current typed grant, intent window and
exact recipient. At the effect fence an already-held intent may continue across
an unrelated epoch advance only through explicit local approval and current
sender/key/page/grant/expiry/archive admission. Original epoch need not remain
current; the original signature, bytes and adopted epoch remain authoritative.
Until then no core dispatch or wake occurs. Same operation ID and
intent returns the existing record; another sender or changed digest/input is
`INTENT_CONFLICT` and cannot overwrite it. B's operation ledger binds principal
kind/source key and both digests so collisions cannot transfer result ownership.

## Dispatch uncertainty and final mapping

The existing bridge ledger/state machine remains the sole owner. Persist held,
then dispatching before the exact `dispatch.create` call after local approval.
Approval is B owner's action on frozen bytes; it cannot silently edit them.
Expired/revoked/out-of-scope held work becomes expired/refused with a reason.
Offline arrival waits only within its intent/page lifetime; display waiting for B.

Validate the core receipt's operation and sole recipient against the ledger
before recording it. The receipt does not expose request kind or originator;
those are checked in the frozen core input, not invented receipt fields.
A queued item maps the frozen operation to the
created **B core request ID**. A failed recipient item is a definite failure, not
an accepted request waiting for a final. A successful process exit alone does
not validate either state. Core acceptance, wake outcome and durable final remain
separate; uncertain/absent wake does not invalidate a committed acceptance or
authorize another wake.

Timeout, lost output, invalid receipt or crash after dispatch may have started
leaves uncertain; restart never repeats dispatch automatically. Recover only
the owned operation with `dispatch.show`. Matching receipt recovers its original
mapping; `DISPATCH_NOT_FOUND` or lookup failure leaves uncertain. Explicit same-ID
retry requires confirmed original-child termination and the full current fence,
using the same bytes and ID. Today's process API cannot establish that termination
after a bridge crash, so post-crash retry remains disabled. No new process API is
assumed. Abandon stops tracking, says may have been delivered and cancels nothing.

Only B's immutable accepted mapping authorizes `requests.show(requestId)`.
`changes.cursor` is a B-local observation hint, never a remote scope/cursor or
proof of a final. Core retained final bytes, including empty text, are completion
evidence. Terminal text, runtime exit/idle and page status are not. The recipient-encrypted
copy is a colab projection of the core final, not an agent signature or second
completion service. No colab operation invokes core reply/answer/ack on A's behalf.

### A's own operation observation and B's authenticated replies

A wait/result accepts only operation IDs in A's persisted own intent set and
reads the existing admitted page/bridge streams. It neither invents an A core
request ID nor calls B's core with a caller-supplied request ID. Wait is a bounded
observer; timeout, interruption or closing A stops observation, not B's work.
Resuming observation is read-only and never republishes intent. Command spelling
is deferred; these are proposed semantics, not shipped `tmt result` behavior.

Propose a B-bridge attestation with domain `tmt-colab-machine-result-v1` and LP:

```text
version, space, page, originalEpoch, senderKind, sourceMachine, senderId,
machine, agent, operationId, intentDigest, coreInputDigest,
grantId, grantRevision, state, finalState, observedAt,
resultContextDigest, recipientEnvelopeHash
```

`senderKind=machine`, `machine=B`; `state` is an existing ledger state.
`grantId/revision` are B's recorded adoption grant, empty/zero only when no grant
was selected. `finalState` is `pending`, `retained`, `expired`, `unavailable` or
`none` (no accepted request). Metadata carries no core request ID, receipt detail,
final body or core submission/expiry timestamps. When no recipient payload is
published, both result digests are zero32; this is metadata only, not a final.

B signs with the key pinned by verified `bridge.add` for the intended destination
and page. Its existing signed bridge object also authenticates ordered publication.
A pins that expected B bridge binding when freezing intent; no transport key or
different same-machine binding substitutes for it. Validate the full original
intent/agent/page/source/digests and operation mapping before displaying state or
final; reject mismatched decrypted request IDs, wrong bridge/key, cross-operation replies
and conflicting finals. Accepted receipt is not final; pending means no final
observed, and expired/unavailable is not failed processing or permission to resend.
Do not claim the bridge currently online merely because an old record exists.

### Recipient-only receipt and final payloads

This is an **explicit exception to colab-v1's page-copy disclosure rule** for the
machine path: receipt details and final payloads MUST NEVER be encrypted under
a page epoch key. Page members get metadata only, even after an epoch advance;
there is no owner-authorized page-history disclosure path for these payloads in v1.
A's intent, question text and frozen final bytes live in A's own stream under
the page epoch key and are visible to readers of that epoch under colab-v1.
B's receipt/final are recipient-only; a private question must not be sent through
a shared page. Use the existing HPKE Base suite, with a new exact context domain:

```text
LP("tmt-colab-machine-result-context-v1", version, suite,
  space, page, originalEpoch, sourceMachine, senderId,
  recipientKind, recipientXPublic, machine, bridgeEdPublic,
  agent, operationId, intentDigest, coreInputDigest,
  grantId, grantRevision, resultId)
```

`suite` is exactly `base-x25519-hkdfsha256-aes256gcm`, as in colab-v1 HPKE wraps.
`recipientKind=machine`; `recipientXPublic` is A's exact pinned `machine.add`
encKey. `machine` and `bridgeEdPublic` identify expected B; resultId is a fresh
generated UUID. Context is at most 1,024 bytes; HPKE info and AAD are those same
exact context bytes. Use library HPKE with fresh encapsulation randomness,
reject all-zero DH and freeze the resulting bytes for retransmission. B signs:

```text
LP("tmt-colab-machine-result-seal-v1", version, context,
  enc32, SHA256(ciphertextWithTag))
```

The recipient envelope contains exactly binary `{context, enc, ciphertext,
signature}`. Its hash is SHA256 of
LP("tmt-colab-machine-result-envelope-v1", version, context, enc32,
ciphertextWithTag, signature64). The page metadata attestation binds that hash
and SHA256(context). Storage uses the existing immutable object/chunk and page
admission owners with this explicit recipient-envelope variant, not the
page-key `seal` construction or another transport. Implementing decoders must
admit this new variant explicitly. No existing colab object decoder is claimed
to support it today.

HPKE plaintext is strict typed JSON: `version, operationId, intentDigest,
coreInputDigest, originalEpoch, state, requestId, receipt, final`. Request ID is
null before accepted mapping; otherwise it is only B's recorded core ID.
Receipt is null or the matching immutable core receipt. Final is null or exactly
the core final observation (`status` not_submitted/retained/expired/unavailable,
with its existing bounded fields). Retained carries exact response UTF-8,
including empty text. Match plaintext state/final to attestation, original intent
and receipt. No body on refused/expired intent, failed dispatch or uncertain
without accepted mapping. Core error/lookup failure yields sanitized unavailable
observation, never a fabricated final. Respect core input/output/final and colab
chunk bounds: bounded reassembly, expected-signature/context/hash verification,
HPKE opening, typed plaintext validation, then application.

Before sealing AND before publication, under B's result-disclosure fence,
require the source machine principal still admitted to the page at B's latest
verified head with that exact encKey, the intended B bridge still admitted, an
active/unexpired page, and no local result-disclosure revocation. Local grant
revoke MUST also disable future result disclosure for that principal's operations;
grant expiry alone prevents new effects but does not erase accepted-result rights.
Never resolve a replacement recipient key. If either fence fails, keep the
outcome B-local only and do not publish recipient payload. Sealing must not occur
inside a retried storage transaction; publication rechecks the authorization
snapshot. A verified removal before that fence means no late seal/publication.
This does not imply B has seen every remote removal: withheld-revocation limits
remain. Already-sealed, published or cached ciphertext addressed to A's key can
still be decrypted by its holder and cannot be recalled.

A verifies expected B signature, exact recipient/context and metadata hashes
before HPKE opening with its pinned machine key, then validates the plaintext.
Wrong recipient key, source kind, original epoch, page/agent/operation/digest or
B signer rejects. Receipt/final publication never silently republishes intent.
Page-key metadata discloses exactly version, space/page/original epoch, source
kind/machine/writer, destination machine, agent UUID, operation and intent/core
input digests, selected grant ID/revision, ledger/final state, observation time
and recipient-context/envelope hashes. Generic sync metadata additionally exposes
writer IDs, sequence, size/timing and public keys as in colab-v1. No machine-path
receipt detail or final text is disclosed merely by holding a page epoch key.

## Epochs, revocation, offline state and retention

| Event                                            | Pending intent / effects                                                                                                                                     | Observation / retained data                                                                       |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| B offline or observer timeout                    | Wait only until frozen expiry; no new operation                                                                                                              | Resume read-only by own operation and authenticated B reply                                       |
| Intent/grant expiry or local revoke before fence | Reject dispatch; approval cannot extend validity                                                                                                             | Preserve truthful recorded state within colab retention; no automatic regrant                     |
| Epoch advance while unadopted/held               | Unadopted old-epoch input rejects; already-held input may continue only by explicit local approval and current principal/grant/page fence; no reseal/reissue | Accepted work remains accepted; epoch reset never replays sends; payload remains recipient-only   |
| Machine removal/key replacement                  | Stop new adoption/approval when verified; local grant revoke is immediate                                                                                    | Rotation excludes removed recipient wraps; already copied data cannot be recalled                 |
| Bridge removal/key replacement                   | No substitution of the expected B signer; future result continuity needs separately reviewed owner action                                                    | Keep prior verified observations as historical copies, never infer a new result                   |
| Page archive/delete/expiry                       | No new adoption or dispatch; an unknown old outcome remains unknown                                                                                          | Stop page reads/writes per colab policy; core work/finals are not cancelled or deleted by this    |
| Core final retention ends                        | No new send authorized                                                                                                                                       | Core read reports expired/unavailable; previously recipient-encrypted bytes are historical copies |

B records the signed original epoch and adopted outer epoch with intent digest.
Same-operation republication under a new epoch is neither a new adoption nor
renewed validity; refuse it without replacing the ledger. Already dispatched work
may still complete. B may publish metadata for prior-epoch work under current
bridge authority, while payload is sealed only to the original exact authorized
recipient under the result fence above. A newcomer with a new page key sees
metadata, never that late receipt/final. If the page, source or bridge is no longer
admitted, payload observation is unavailable; the owner can inspect B core
independently, without granting A arbitrary reads.

Local pending input/ledger mapping use existing colab storage and bounded recovery
owners. Accepted mapping survives intent expiry for truthful result observation;
expiry prevents new effects, not a rewrite to unsent. Recipient ciphertext and
page metadata follow colab page retention; their disclosure differs as specified
above, independently of core prompt/final retention. Reads
or waits renew neither. Backend removal is eventual and cannot erase offline
copies. Device revocation cannot exclude a surviving link-seed holder; this
amendment preserves colab-v1's Reset link plus rotation boundary. Machine removal
likewise cannot retract keys/plaintext already copied.

## Deferred policy and implementation proof

The slice deliberately leaves these actual policy choices open: autonomous sends
under a separately reviewed source-agent capability; source-agent attribution;
machine direct/hold preference after #600; finite grant duration/quota defaults;
and user-facing send/wait/result/grant command spelling. Existing page roles,
browser direct preference and platform activity are not defaults for these choices.
The approved minimal path is owner-local B grant, explicit local A Send, hold-only.
These open choices do not block this proposed docs slice or authorize runtime work.

Before machine implementation acceptance, colab-owned follow-ups MUST prove:

- Independent Rust/browser/third-implementation byte vectors for machine log
  payloads, machine header, recipient-kind wraps, intent/core-input and bridge
  attestation; valid positives and single-field principal/key/scope substitutions.
  Unknown enums/operations, browser signatures/grants reused as machines, bridge
  intent forgery and machine content/reply checkpoints reject with no effects.
- Owner root custody, local grant commit/narrow/revoke races and exact-key/page/
  agent/expiry fences. Explicit local Send/hold; no Send from sync, restore,
  scripts, replay or autonomous model output. Denied/held work creates no core
  request or pane input.
- Two isolated TMT homes in demo Firestore emulators after L1-L6: shared space,
  correct ciphertext/Rules/epoch admission, explicit A intent, B hold/approve,
  one public core acceptance, deterministic agent's durable/empty final, A own
  observation and unchanged core ownership. No real model/account/deployment.
- Conflict/duplicate adoption, offline expiry, grant/machine revoke, page freeze,
  epoch advance before/after dispatch, lost output, malformed/mismatched receipt,
  crash with disabled retry, confirmed-child same-ID retry, no repeated wake,
  and no operation replacement on unavailable result. Verify durable state, not
  only exits or UI labels.
- Forged/wrong bridge key, operation/digest/agent/page/source substitution and
  arbitrary request-ID reads; accepted versus final versus unavailable; wait
  interruption/resumption, core expiry and page retention/history limits.
- Original-epoch/signature/outer mismatch and same-operation new-epoch reseal
  rejection; unrelated epoch advance with admitted held intent and explicit local
  approval is a positive control. Member-add baseline followed by late old final
  gives newcomers metadata only. Verified removed machine gets no late seal or
  publication; wrong/replaced encKey, local result revoke and revocation between
  seal/publication reject. Previously sealed ciphertext remains decryptable as
  the documented limitation. Independent HPKE/context/signature vectors prove
  recipient-only receipt/final bytes and page-key inability to decrypt them.
- Existing bounded decoder/stream/chunk/ledger quotas and cleanup owners, private
  sockets and deterministic agents, no member hosting. Run lifecycle acceptance
  twice with confirmed child/socket/state cleanup. No fixture proof is claimed by
  this documentation or by colab's earlier crypto/renderer spikes.

Each follow-up needs its own authorization, bounded issue/PR and required reviews.
This amendment does not start those follow-ups or broaden the docs member's topic.
