# Local attachment storage proposal

**Status: proposed object runtime/CLI; the [descriptor/manifest byte grammar](attachment-v1.md) is defined separately.**
Architecture/product primary: Colab. Remote owns generic backend, quota, object channel and originating transport; Colab owns references, content cryptography, membership/history admission, native consumers and UI. This joint contract is reviewed through [#1849](https://github.com/pj-tmt/tmt/issues/1849) under [Storage #1691](https://github.com/pj-tmt/tmt/issues/1691).

The terms MUST, MUST NOT and SHOULD describe requirements for the planned implementation, not claims about current runtime support. [Colab v1](colab-v1.md) retains its current owners and byte/crypto definitions; [Remote's channel contract](../../../contracts/remote-channel-v1.md) retains current Session/grant/mount authority. No core object API/database/attachment field, new credential, cloud implementation, dependency or deployment is authorized here. Remote never imports Colab code or interprets page/epoch/reference types. An object ID or URL grants no authority.

## Responsibility and scope

Remote supplies one installed-extension-scoped interface and trusted local filesystem implementation, with no provider-specific Colab branches. Its [coordinated carrier contribution](https://github.com/pj-tmt/tmt/issues/1849#issuecomment-5999213383) supplies the proposed private frame/type definitions and origin state machine; the public mappings and authenticated read capture below remain Colab-owned. These cross-squad requirements need exact-head owner review before implementation. The generic Core responsibility direction is settled; a concrete shared/core change still requires its normal owner review.

Local v1 starts with current documents, exact Chat/annotation message revisions and archived non-deleted page reads (#1853). Signed snapshots and retained-reference persistence belong to #2299; download/export and filesystem consumers remain #1854/#1855/#1867. Binary bodies stay out of Yjs, agent prompts/preambles/notes and automatic Ask payloads. Names, original-author labels, filenames and user-supplied principal JSON are not authority. Limits are bounded technical owner proposals, not another product questionnaire.

## Existing owners and implementation gaps

`Snapshot::capture` retains the writable-page fence. Its read purpose verifies the same owner log/head, device history and pinned cuts while allowing archive and refusing delete; frozen historical cuts remain read-only. Attachment capture joins exact decoded references and positive-sequence creation proof before opening an asset. `OwnerAdmission::authorize(Read)` and reader policy can permit an archived, non-deleted page, but `Access::Read` alone establishes neither an authenticated attachment reference nor historical-key eligibility. Those distinctions are mandatory implementation work, not a reason to omit archive/history from v1.

Existing reader Sessions are fixed to the current page epoch and expire/recheck current share/link/device policy. Reader tickets are one-shot upgrade proofs, not general object HTTP bearers. Browser Admission opens only verified wraps addressed to the actual device/link, retaining opaque handles within the existing 64-epoch window. The planned historical fetch in the final #1853 slice uses the same Admission/Objects/Worker owners through the object adapter; it does not substitute the current projection. The native server's possession of an epoch secret is not permission to disclose it to a reader.

Reuse the existing owner-log verifier, `page_policy_at`, authenticated cut validation, envelope/header/hash/signature validation, isolated decoder permit/deadline, strict own-record fold and wrap/public-key admission. Add one read capture at that owner. Do not build a second authority reducer or feed arbitrary client JSON into a decoder as proof. Existing writable capture remains the write/restore owner.

## Attachment read admission

Internal capture interface; Remote callback/peer-generation composition and activation remain planned:

```text
capture_attachment_read(actual_origin, exact_selector, request_deadline)
    -> AdmittedAttachmentRead | Denied | Unavailable
```

`actual_origin` is constructed by the socket/sync/local owner, never decoded from public JSON. `exact_selector` identifies a reference, not permission. `AdmittedAttachmentRead` contains only bounded internal data needed to verify an exact read:

- Current verified owner-log revision/hash; current page policy/epoch; exact installed bus/origin and sync peer generation or actual root-local request owner.
- Reference kind and exact document revision/cut or writer/message/revision or, in #2299, snapshot binding; canonical descriptor digest; asset epoch and original object ID, creator/writer provenance and signed membership revision.
- Verified serialized-byte SHA-256/length, Colab framed envelope hash and allowed range; opaque namespace/key derived from the admitted descriptor.
- Historical recipient entitlement, when needed, bound to the current actual principal; no unwrapped key, bearer or caller-selected principal is put into the Remote policy bytes.

This is an ephemeral capture for one callback, not a permit for a stream, later request or successor. Callback decisions are only allow/deny/unavailable. Release owner-store, registration and sync locks before isolated decoding, private IPC or backend I/O. Reacquire fresh authority and exact peer state before returning allow, and repeat capture at acquisition, each bounded chunk, effect and disclosure. Any intervening head, relevant source cut/reference, epoch, reader/registration generation, bus or origin change invalidates the capture. Return stale/unavailable; do not silently recapture a different reference, retry a mutation or move a result to another peer.

### Exact selectors and reference proof

The Colab-local selector is a strict tagged union, within the existing 2 KiB opaque policy-input bound after mapping. UUIDs, hashes and decimal revisions use existing canonical validators. Unknown/duplicate fields are rejected.

| Selector                                                                                      | Required authenticated source                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `document-current {attachmentId, descriptorHash, contentRevision}`                            | Exact descriptor in the current, cut-admitted document metadata projection at that revision. Neither HTML URL nor raw `objectId` is a descriptor. Current changes cannot satisfy an earlier requested revision.                                                                                                                                                                                                                                                                                                                                                                                            |
| `message {writerId,messageId,messageRevision,attachmentId,descriptorHash}`                    | Exact immutable own-record message revision admitted by the historical writer/device key and every later membership/device cut. A currently removed writer may have a valid retained record before its cut; a later forged revision cannot extend that cut.                                                                                                                                                                                                                                                                                                                                                |
| Planned #2299: `snapshot {snapshotId,attachmentId,descriptorHash,attachmentManifestHash}`     | Exact authenticated snapshot record and its immutable attachment manifest binding, preserving original creator/membership revision/source digest. No signed snapshot record/store owner is shipped. #2299 must introduce the authenticated record and retain its manifest through compaction; the existing manifest grammar alone is not proof. The manifest uses the existing encrypted Colab envelope boundary, with at most 128 bounded descriptors; its authenticated hash binds the exact list to that snapshot. Keep the existing sourceDigest meaning and original snapshot source bytes unchanged. |
| Planned #2299: `document-retained {checkpointId,contentRevision,attachmentId,descriptorHash}` | Explicit pinned, authenticated retained document projection containing that descriptor and its verified checkpoint/cut. Compaction must retain that proof or a self-contained manifest binding. Do not reconstruct a removed revision from latest content or return a different descriptor.                                                                                                                                                                                                                                                                                                                |

A descriptor binds object ID, asset epoch, original creator/writer and source provenance, framed envelope hash, serialized digest/length, plaintext length, filename/type and attachment ID. Label/originalAuthor/publisher names have no routing or authority role. Caller-controlled selector or policyInput must resolve to those actual authenticated bytes before a positive callback. The canonical descriptor/manifest grammar and native/TS byte vectors are defined in [attachment-v1](attachment-v1.md), the first #1853 slice. Its decoder/Worker understands bounded metadata/comment descriptors and preserves them through baselines. The second #1853 slice supplies ephemeral native/browser reference/read captures and a committed-object publication verifier; the final slice supplies the real channel adapter and callback composition. Snapshot/retained-reference persistence remains #2299; the lifecycle policy matrix is in [colab-v1](colab-v1.md).

Archive preserves references and permits reads under current admission, including exact immutable message revisions; upload, attach, edit and restore remain prohibited while archived. Delete/revoke denies new acquisition/disclosure immediately; retained physical bytes do not preserve access. Already downloaded plaintext cannot be recalled. Missing proof, key or object is an explicit unavailable attachment, not a successful empty body or an alternative current-version read. Current document, exact message revision and archive positive cases are required for #1853. Snapshot and retained-document persistence/positive cases are deferred to #2299; unsupported fallback cannot satisfy that later gate.

### Principal and history matrix

| Actual principal            | Current authority                                                                                                                                  | Historical asset eligibility                                                                                                                                                                                                         |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Paired mounted owner/editor | Exact live Remote Session/grant/revision and current Colab registered device/membership; reader-ticket branch still takes precedence if present    | Existing device/member wraps actually eligible for that principal, plus exact retained reference/cut. Do not expose all server-held epoch keys.                                                                                      |
| Link reader                 | Exact active one-shot-upgraded reader and sync peer, current unrevoked link/certificate/expiry/share policy, no Remote operation scopes            | Owner-signed link-specific wraps issued under the join/history policy at their membership revision and within retained bounds; not another link's wraps. Current revoke, delete or expiry denies even if the old descriptor remains. |
| Public reader               | Exact active public-reader peer and current public share policy, no invented Remote grant                                                          | Only verified owner-signed `PageShare.publishedKeys` applicable to that admitted public history. Private/server epoch secrets and another reader's wraps are not eligible.                                                           |
| Native root-local consumer  | Actual Colab root-local request admitted against the invoking public core storage root, existing keyring/space owner log and private serving owner | Native owner can use locally retained keys only for the exact admitted current/retained reference and non-deleted policy; no fabricated browser device/Session.                                                                      |
| Agent                       | One of the preceding actual contexts, not its display label or caller-provided agent UUID                                                          | Same reference/history rules. No ambient download URL, reader promotion or privileged cross-context fetch.                                                                                                                           |

The outer sync scope remains the peer's **current** space/page/epoch. A historical asset epoch is in its authenticated selector/descriptor, not a request to downgrade the peer Session. Changing history policy affects later joins; it does not invent or remove an earlier actually issued key retroactively. Current membership/link/device revocation and retention still fence each new server disclosure. Existing forwarding retains a bounded 64-epoch window; it is not permission to give a current-only join earlier keys. Policy narrowing/rekey must produce current-epoch references for permitted surviving content; if quota prevents re-sealing, narrow access anyway and keep the old complete reference (the Files row says it is being secured). The rekey of #2293 is the [colab-v1 contract](colab-v1.md#rekey-after-an-epoch-advance-2293). Never preserve old reader authority to avoid a quota error.

## Public sync grammar and originating peer

Add typed `object` and `object-result` variants to the existing Colab version1 RFC6455 grammar. Keep current 64 KiB message, 32 KiB decoded chunk and eight outbound frames. Use typed per-operation inputs instead of arbitrary `Value`/flatten, retaining duplicate/unknown-field rejection. This is proposed grammar:

```json
{
  "type": "object",
  "version": 1,
  "space": "<current space>",
  "page": "<current page UUID>",
  "epoch": "<current epoch>",
  "requestId": "1",
  "operation": "read",
  "input": {
    "reference": {
      "kind": "message",
      "writerId": "<writer UUID>",
      "messageId": "<message UUID>",
      "messageRevision": "<revision>",
      "attachmentId": "<UUID>",
      "descriptorHash": "<hex SHA256>"
    },
    "offset": "0",
    "count": 32768
  }
}
```

Public requests never carry installed extension ID, bus generation, origin handle, principal JSON, reader proof, backend name or arbitrary Remote policy bytes. `config` takes an admitted current content selector; `begin` takes the caller-frozen transferId plus a strict writable-target selector and frozen envelope digest/length/object identity; `part`, `commit` and `discard` address its original transferId; `status` carries that ID and exact frozen target scope. `read` takes the exact reference selector and range. Browser mapping cannot override the stored immutable intent. Only Colab maps these into the [generic operation shapes](#backend-operations-and-configuration) below.

`requestId` is a canonical positive u64 decimal counter, monotonically increasing per actual public connection generation. It is distinct from private bus request/callback counters and durable UUIDv4 transferId. No wrap/reuse; exhaustion closes the connection. At most one pending object request per public peer, with a bounded high-water mark and exact association to operation, transfer/ref/range, scope, peer generation and private request. Duplicate, mismatched or late results cannot satisfy a successor. Replies repeat the exact scope/requestId/operation and applicable transferId, then one typed result or error. Read results bind offset/total/digest/length and a bounded encrypted chunk. No automatic retry or whole-object response.

`sync::State::process` currently runs under the server lock. It must only validate/extract an admitted request and reserve the bounded pending slot there, then release that lock. The owned object adapter runs callback/I/O work outside it and re-enters the server for exact peer/generation/policy checks and bounded result enqueue. `Connection::poll`, close/drop and reader release invalidate pending work. Do not block the sole sync or private bus reader waiting for callbacks, and do not add unowned per-frame threads.

At upgrade, the existing reader-ticket branch wins over owner context. Associate its exact resulting `sync::Server::connect` peer ID/local generation with the proposed trusted Remote Pending origin. A valid Colab upgrade alone is insufficient: both sides must observe Established after Remote's actual Session attachment, browser upgrade write and supervisor adoption. Private origin-state messages carry lifecycle only; they do not certify reader identity. Reject object work while pending; no automatic retry. Closed reader/peer, released slot, abandoned write, failed adopt, bus replacement or shutdown invalidates this association. No mapping by device UUID, page, cookie or reader UUID across connections.

## Private carrier, scheduling and budgets

The coordinated proposal uses the Remote-owned carrier and type direction. The reserved `GET /.tmt/remote/object-channel-v1` zero-body/101 handshake and separate four-byte length/strict JSON driver are **new plumbing**, not RFC6455 or an existing registry. Keep the trusted static installed-extension declaration, stripped private metadata outside strict registration Context, exact safe door.sock and mount host, and no browser access to reserved routes. Existing `adopt()` does not report success; the Remote owner must expose actual adoption before Established. Failure/close/drop/release/bus invalidation is mandatory, including late activation after worker closure.

These finite technical budgets are proposed requirements, subject to implementation conformance; they are not deployed settings:

| Resource                     | Contract maximum                                                                                                             |
| ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Private setup/origin Pending | 15 s absolute; header acquisition 2 s, response write 1 s, clipped by setup                                                  |
| Private frame                | 65,536 JSON bytes plus four-byte prefix; 2 s absolute from first prefix byte; no renewal                                     |
| Chunk/policy/intent metadata | 32,768 decoded bytes / 2,048 decoded bytes / 8,192 serialized bytes                                                          |
| Admission callback           | 5 s absolute, clipped by request and immutable intent deadline; existing decoder 2 s and permit unchanged                    |
| Object request               | 30 s absolute including acquisition, queue, callbacks and I/O; later explicit steps cannot extend original intent expiry     |
| Response write               | 1 s, clipped by remaining request budget                                                                                     |
| Bus capacity                 | One active plus one setup candidate per extension; eight combined installation buses                                         |
| Requests and callbacks       | Eight each per bus, 32 each per installation; one callback at a time per request; one public pending object request per peer |
| Queue                        | Eight frames each direction, at most 524,288 JSON bytes plus prefixes; public queue remains eight                            |

The existing 16 live public tunnels, ordinary HTTP acquisition/write limits, 120 s sync idle and decoder bounds stay intact. A private bus must have its own supervisor capacity class rather than occupying permanent ordinary HTTP busy capacity or being subtracted as public live. Retain/close/join its socket/worker and bounded service jobs at stop. Keep reading/correlating callbacks while service jobs wait; no sole-reader deadlock, per-frame spawn or detached worker.

Private request/callback IDs are separate monotonically increasing positive u64 counters with bounded outstanding maps. Bus generation/kind/association and high-water checks prevent duplicate/late/cross-generation reply reuse. Existing local `ipc::exchange` has a shorter ordinary exchange budget; object local I/O needs its own bounded route-owned adapter, not a global timeout increase.

A deadline is not guaranteed cancellation of an in-flight filesystem syscall or fsync. Keep the original intent/state and quota charge, suppress late disclosure, and account for owned unfinished work through shutdown/recovery. Do not claim rollback, forcibly terminate a mutating worker, retry or allocate a replacement ID. A blocked syscall may delay join; readiness cannot claim successful cleanup while that worker remains. Backend conformance must show bounded step/cancellation behavior at controllable boundaries, with uncertain possible effects preserved.

## Backend, operations and configuration

Retain the object-safe synchronous `ObjectBackend` and LocalFs sketch in the existing consumer proposal: `id`, `capabilities`, `usage`, `begin`, `append`, `commit`, `status`, `stat`, `read`, `discard`, `remove_namespace`, each under an explicit I/O budget. Backend calls are available only to Remote's admitted service. Trusted composition injects the one LocalFs implementation under the extension data root (0700 directories, no-follow owned 0600 files); future adapters implement that same interface/conformance without Colab branches. No cloud implementation/dependency is approved.

### Generic backend interface sketch

The following signature projection aligns with the reviewed Remote backend library. The object channel, callbacks, configuration and Colab consumers remain proposed and unintegrated. Remote owns the actual types in `objects.rs`; this sketch omits derives and validation helpers. Opaque IDs and immutable input bindings carry no authority; only the admitted service invokes the backend.

```rust
use std::sync::atomic::AtomicBool;
use std::time::Instant;

type Digest = [u8; 32];
#[derive(Clone, Copy)]
pub struct NamespaceId([u8; 32]);
#[derive(Clone, Copy)]
pub struct OpaqueKey([u8; 32]);
#[derive(Clone, Copy)]
pub struct IntentId([u8; 32]);
#[derive(Clone, Copy)]
pub struct BlobKey { pub namespace: NamespaceId, pub object: OpaqueKey }
pub struct BeginSpec {
    pub intent: IntentId,
    pub key: BlobKey,
    pub payload_sha256: Digest,
    pub payload_bytes: u64,
    pub binding: Vec<u8>, // service-generated immutable input binding, <=2 KiB
}
pub struct OriginalIntent {
    pub spec: BeginSpec,
    pub adopted_at_ms: u64,
    pub expires_at_ms: u64,
}
// A receipt attests only committed raw storage bytes.
pub struct Receipt {
    pub intent: IntentId,
    pub key: BlobKey,
    pub payload_sha256: Digest,
    pub payload_bytes: u64,
    pub binding: Vec<u8>, // original service binding, not a permission token
}
pub struct Progress { pub intent: IntentId, pub next_index: u32, pub received: u64 }
pub enum BeginResult { Pending(Progress), Committed(Receipt), Terminal(TransferState) }
pub enum TransferState { Pending(Progress), Committed(Receipt), Expired,
    Unavailable, Unknown, Discarded, NotObserved }
pub struct Transfer { pub state: TransferState, pub original: Option<OriginalIntent> }
pub struct ReadPart { pub offset: u64, pub total_bytes: u64, pub bytes: Vec<u8> }
pub struct Usage { pub charged_bytes: u64, pub entries: u32,
    pub active_uploads: u32, pub retained_identities: u32 }
pub struct BackendCaps { pub max_payload_bytes: u64, pub chunk_bytes: u32 }
pub struct IoBudget<'a> { pub deadline: Instant, pub cancelled: &'a AtomicBool }
pub enum Limit { NamespaceBytes, ExtensionBytes, InstallationBytes,
    NamespaceEntries, ExtensionEntries, InstallationEntries, ActiveIntents,
    RetainedExtension, RetainedInstallation }
pub enum BackendError { Invalid, Conflict, Missing, Unavailable, Cancelled, Deadline,
    Capacity(Limit) }
pub type BackendResult<T> = Result<T, BackendError>;

pub trait ObjectBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> BackendCaps;
    fn usage(&self, namespace: Option<NamespaceId>, io: &IoBudget<'_>)
        -> BackendResult<Usage>;
    fn begin(&self, spec: &BeginSpec, io: &IoBudget<'_>)
        -> BackendResult<BeginResult>;
    fn append(&self, intent: IntentId, index: u32, bytes: &[u8], io: &IoBudget<'_>)
        -> BackendResult<Progress>;
    fn commit(&self, intent: IntentId, io: &IoBudget<'_>)
        -> BackendResult<Receipt>;
    fn status(&self, intent: IntentId, io: &IoBudget<'_>)
        -> BackendResult<Transfer>;
    fn stat(&self, key: BlobKey, io: &IoBudget<'_>) -> BackendResult<Receipt>;
    fn read(&self, key: BlobKey, offset: u64, count: u32, io: &IoBudget<'_>)
        -> BackendResult<ReadPart>;
    fn discard(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<()>;
    fn remove_namespace(&self, namespace: NamespaceId, io: &IoBudget<'_>)
        -> BackendResult<()>;
}
```

One trusted `LocalFs` installation coordinator borrows Remote's held `Serving` lease and shares metadata/accounting and in-flight guards across extension-bound `LocalHandle` values. A validated `ExtensionId` name is not installed identity or admission; #1852 must obtain it from trusted registration and compose one coordinator, not reopen it per request. The trait handle's `usage(None)` means its extension, not the installation. Installation usage stays on the trusted coordinator; configuration readers receive only the admitted namespace projection. Other backends must provide the same shared accounting and trait semantics without Colab provider branches.

`BeginSpec` freezes caller input. `OriginalIntent` adds adoption time and staging expiry once at durable adoption; later requests compare the same frozen input, not a new caller clock or renewed deadline. `status` returns `Transfer`: compare its retained original's exact key/raw digest/length/binding before projecting its state under current original-principal admission. An absent original is `NotObserved`, not proof of no effect. Neither an original record nor a non-committed state is a committed receipt. `Capacity(Limit)` refuses a new adoption; it is not a terminal outcome of a previously adopted uncertain transfer and never permits retry or replacement identity.

`BackendCaps.chunk_bytes` defines canonical parts: exact full parts followed by the final remainder. A zero-byte generic raw payload commits without a part; Colab still supplies and validates its sealed envelope and independent plaintext/media limits. Every successful I/O result, including retained-original and usage fast paths, rechecks its budget after controlled ledger/I/O work before disclosure. Cancellation/deadline preserves any effect, charge and original state. Startup `LocalFs::open` reconciliation and an explicit owning-service `reconcile` call are separate from observational status; consumers do not invoke repair, timers, mutation retries or completed-object GC.

Generic method inputs and semantics remain:

| Method            | Input/result                                                                                                            | Fresh authorization and effect                                                                |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `objects.config`  | opaque namespace/policyInput -> effective backend/capabilities/limits                                                   | Current admitted content read; no inventory or mutation                                       |
| `objects.begin`   | transferId, namespace, opaqueKey, policyInput, payloadSha256/payloadBytes -> original pending progress/receipt/terminal | Exact current target write, original principal/intent and quota adoption                      |
| `objects.part`    | transferId, index, bounded bytes -> durable acknowledged progress                                                       | Original owner/current target admission per chunk; exact stored binding                       |
| `objects.commit`  | transferId -> committed immutable receipt or original state                                                             | Fresh effect callback; byte integrity/create-only publication; does not attach content        |
| `objects.status`  | same transferId, frozen namespace/policyInput -> pending/committed/expired/discarded/not-observed/unavailable/unknown   | Current original-owner admission; read-only lookup, no adoption/recovery mutation             |
| `objects.discard` | original transferId -> confirmed incomplete-staging disposition                                                         | Current allowed creator cleanup; not committed deletion or proof no uncertain effect          |
| `objects.read`    | opaque namespace/key/policyInput, payloadSha256/length, offset/count -> encrypted bounded part                          | Exact admitted reference/history capture at acquisition and disclosure; current original peer |

For paired owners Remote rechecks the exact live grant/Session before Colab callback. For extension-authenticated readers it checks the actual originating mounted connection and asks Colab's live reader owner; it invents no Remote grant. For root-local native requests only the registered Colab owner constructs the local-extension origin from actual private admission. Public input cannot choose this branch.

An original transfer UUID is frozen before begin, and every subsequent operation/status uses it. Expired/discarded/unknown/not-observed never grants re-execution or proves a possibly published effect absent. Exact repeated intent is conformance, not an automatic mutation retry. Original identity retention survives namespace deletion outside removed payloads.

Remote verifies SHA-256/length of serialized opaque bytes. Colab separately verifies the existing domain-framed Envelope hash, signature, asset scope, original writer key/cut and decrypted plaintext length/type. Do not hash JSON bytes as Envelope.hash or parse Colab cryptography in Remote. Store only encrypted bodies; authenticated descriptors/opaque references are bounded content metadata, never binary Yjs updates, agent prompts/preambles/notes or automatic Ask payloads.

Proposed quotas are 64 MiB/namespace, **512 MiB/extension**, 1 GiB/installation; entries 1,024/namespace, 8,192/extension, 32,768/installation; active intents 32/installation; retained original identities 16,384/extension and 65,536/installation. Charge entry reservations before bytes, full staging, unattached/history/rekey copies, retained metadata/index/deletion markers and cleanup-pending physical data. Retained intent metadata <=8 KiB is byte charged. Namespace-delete fences and commit/discard serialize; no quota release before confirmed physical cleanup. File-before-receipt crash recovery settles the same original ID before readiness, not through status. No terminal-ID GC into reuse or completed-object GC is approved.

### Consumer limits

Colab's proposed effective consumer limits are 8 MiB plaintext per file, 12 MiB serialized encrypted payload, 2 KiB per descriptor, 255 UTF-8 bytes per filename, 128 bytes per media-type label, 16 attachments per message and 128 document attachment entries. Apply existing strict bounded-string/control-character validation and exact decoded lengths. Raster previews allow only PNG/JPEG/WebP with at most 16 million pixels; unsupported preview content is not made active by its filename. Generic files remain explicit octet-stream downloads. These are Colab policy bounds, not fields that make Remote interpret media, message or page types. Effective limits are the intersection with backend/raw transfer support; incompatible support refuses rather than raising limits.

### Local durability and uncertainty

LocalFs adapts existing extension-state private/no-follow publication invariants to a bounded dynamic object tree; the existing fixed `Layout` is not already a blob store. Stage only the immutable original principal/intent, namespace/key, raw digest/length, bounded policy binding and acknowledged chunk checkpoint. Sync payload and checkpoint before acknowledging a part. Startup reconciliation removes unacknowledged partial tails and settles only that original intent before readiness. A status call observes, never performs that reconciliation or a commit.

Commit rereads the bounded serialized payload, verifies SHA-256/length and syncs it. Publish create-only using owned temporary-file/hard-link/directory-sync invariants, then record the matching index/receipt before acknowledgment. An existing publication name is recoverable only as the original's own admitted device/inode, with the original raw digest/length verified on reconciliation; equal bytes in another inode are not that publication. Unknown distinct bodies remain conservatively charged and are not overwritten or removed. If allocation cannot be examined, measured or recorded, the original stays unsettled and the shared coordinator refuses new adoption; observation of original IDs remains available, and restart must settle or refuse readiness. File publication before receipt remains a possible effect reconciled by the same original ID, never a replacement file or intent. Backend adoption computes/persists staging expiry once; the retained original binds that metadata to the frozen principal/namespace/policy/bytes. Later requests cannot renew it, and changed frozen input conflicts.

Serialize commit/discard and namespace deletion against the original intent/fence. Retain ID/tombstone records outside removable payload trees; deletion cannot make an old ID reusable. Cleanup-pending physical data stays charged until no-follow removal and directory sync confirm it gone. Quota/policy loss can leave an unattached encrypted object; it does not authorize a content reference or stale disclosure. Pending/committed/expired/discarded/unavailable/unknown/not-observed remain distinct; absent, expiry and deadline do not prove a possibly published effect had none.

`objects.config` projects server-selected trusted backend, supported interface/capabilities, effective bounds and their owning sources. Colab adds its consumer limits to the admitted display, without configuration authority. Suggested display: `Storage: Local`, max plaintext 8 MiB/serialized 12 MiB, upload availability and download availability; owner effective quotas may be shown without private paths/secrets. Readers see only their admitted namespace's read capability and relevant download bounds, not other pages/global usage or backend controls. No new framework or expansion of Remote #1769 settings is needed.

### Current-page configuration selector

Configuration does not require an attachment to exist. The Colab public `config` input is a strict current-page selector:

```json
{ "selector": { "kind": "page", "membershipRevision": "<current owner-log revision>" } }
```

The enclosing object frame supplies the actual current `space`, `page` and `epoch`. The selector must match the verified current owner-log revision and that exact admitted peer/page; it contains no attachment/object ID, descriptorHash, historical selector or backend choice. Native config uses the same current page selection under its actual root-local admission. A changed revision, deleted page, expired reader or inactive origin refuses; an archived page may expose read-only config under current read admission.

Colab derives the opaque namespace and bounded page-policy input at its existing admission owner. Remote `objects.config` evaluates the installed extension callback against that actual origin and returns effective capabilities even before a payload namespace has any entries. This read neither adopts an intent nor creates a namespace/file, reserves quota or grants upload permission. A reader gets the reduced read-only projection. Subsequent operations must admit independently; displaying `canUpload` does not prepare a send or authorize a transfer.

Attachment `read` instead requires the exact reference selector defined above. Never use a fabricated attachment or an unreferenced object ID to populate Storage configuration.

### Planned consumer sequence (not callable API today)

1. Parent freezes file bytes, explicit intended document/message target and one transferId under current write admission. Selection/drop does not replace a Chat draft, prepare/send an Ask or publish a reference.
2. Existing Colab crypto seals the asset; freeze its objectHash and serialized payloadSha256/length. `begin` adopts that exact intent, then explicit bounded `part` calls and `commit` verify/publish encrypted bytes.
3. Only a confirmed committed receipt permits the existing Writer/own-publication owner to attach the exact descriptor to the authenticated message/document. Recheck admission and immutable target. A successful storage commit alone is an unattached object, not delivered content. Lost content publication also uses its own original operation identity, not a replacement transfer.
4. Lost begin/part/commit acknowledgment permits only an explicit `status` of the original transfer under current authority. Unknown/unavailable stops effects and retains the user's draft/attachment state. Loading a page/config/card does not retry.
5. A user read/download requests its exact reference/range on the current peer. Fresh callbacks and final peer checks disclose bounded ciphertext only. The parent checks complete byte digest/length and the authenticated Colab envelope before decryption/preview or download.
6. Preview is only bounded PNG/JPEG/WebP (16 Mpixels); generic files including video are download-only `application/octet-stream` with attachment disposition/nosniff. No inline HTML/SVG/PDF/video, eval/inline-style or CSP relaxation. Parent owns trusted preview/download cleanup; renderer has no object authority. The existing image policy permits self/data raster sources; a Downloads-owned blob URL does not authorize a blob image source. Do not widen img-src or use a download URL for inline active/video content.

### Native/agent consumer

Shipped by #1867 as `tmt colab attachment read <page> --reference <exact selector file> --output <owned destination> --json` (the [colab-v1 contract](colab-v1.md#attachment-read-command-1867) owns it); the reference is the `reference` of an export manifest row. It is a Colab command, not a core object operation. The public invoking core resolves storage.root; Colab uses the existing keyring, verified owner log and admitted root-local serving route/in-process owner. No caller-provided agent/device UUID or public identity label establishes authority. Peer UID is socket sanity, not a principal grant by itself.

The private route validates the actual owner and exact reference/history capture before constructing `LocalExtension` on its registered bus. It uses no arbitrary object path/URL and no browser Session fabricated for native access. Remote delivers bytes to that exact requesting IPC path; Colab verifies/decrypts only after fresh final admission. Output uses the existing export no-follow/create-only/private 0700 directory/0600 file/manifest-last and owned-cleanup pattern. Agent JSON returns a bounded manifest/path and verified reference/digest, never plaintext bodies in stdout/prompts/notes. An agent needing non-local access must already have an admitted Colab content context; generic agents.read or object possession does not supply it. The native/export IO child must pin actual command/route DTOs and byte vectors before implementation acceptance; this proposal adds no new arbitrary fetch facility.

Native attach shipped by #2291 as `tmt colab attachment attach` (the [colab-v1 contract](colab-v1.md#native-attach-command-2291) owns it). It is the write counterpart: the serve seals a staged file as the local writer and uploads it as `LocalExtension` through a narrow Colab-side admission, so Remote's hub needed no change; it covers document attachments, not messages.

## Verification and delivery

#1853 MUST include positive current-document, exact immutable-message and archived-page reads, plus actual root-local native reference admission. Snapshot and retained-reference persistence/positives belong to #2299, and filesystem command/output delivery belongs to #1867. Unsupported binding or proof denies safely; it MUST NOT omit a positive case required by the current slice. Verify forged descriptors/cuts/keys, current-only versus shared-history joins, public/link/owner separation, expiry/revoke/delete/head-change races, Pending/Established/closed origin races, callback correlation/capacity, interrupted original-ID publication and conservative quota/cleanup. Native/TS DTO and envelope vectors must agree. Real process/socket/temp-file/object-URL cleanup, unchanged CSP and durable bytes/state matter alongside test counts.

The planned direct children already exist; no runtime worker or execution slot is reserved by this proposal:

| Issue | Owner and bounded result                                              | Delivery dependency                                          |
| ----- | --------------------------------------------------------------------- | ------------------------------------------------------------ |
| #1849 | Colab primary, Remote seam, UX states: coordinated docs-only contract | Exact docs-head owner review before implementation           |
| #1850 | Remote backend trait and local filesystem durability                  | Accepted #1849                                               |
| #1852 | Remote admitted channel/API/config                                    | #1850 and accepted #1849                                     |
| #1853 | Colab canonical descriptor, crypto and callback/read admission        | #1852 and accepted #1849                                     |
| #1854 | Colab Chat/annotation attachment consumer                             | #1853                                                        |
| #1855 | Colab document raster/download consumer                               | #1853                                                        |
| #1856 | Colab policy, revocation, retained history and lifecycle              | #1853/#1854/#1855                                            |
| #1867 | Colab export and admitted native/agent file I/O, owned temp cleanup   | #1853 and delivered #1856                                    |
| #1857 | Colab product acceptance, later bounded Infra execution               | Delivered primitive/consumer/lifecycle heads including #1867 |

Incomplete staging expiry is distinct from warning-only local page expiry; no automatic completed-object expiry/GC is approved. Later Firestore/Cloudflare profiles remain #1049/#1050 with their existing authorization. Tracking uses native parent/sub-issue links; preserve existing Project Epic values and leave new ones empty.
