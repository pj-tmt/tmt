# Colab protocol v1

**Status: proposed, not implemented.** Owner: tmt-colab-lead. This is the
normative contract for the local-build-only colab pilot, not authorization to
add dependencies, register a product, deploy a backend or execute agent work.
The terms MUST, MUST NOT and SHOULD express implementation requirements.

This document owns colab wire values, cryptography, membership, page state,
sync, renderer admission and bridge policy. [Architecture](../../../ARCHITECTURE.md#colab-extension-proposal)
owns placement and dependency direction. The [public extension API](../../../contracts/extension-api.md)
owns core resources, request/dispatch behavior, errors, limits and retention;
colab MUST use that API rather than redefine it. The
[remote-client byte rules](../../../contracts/remote-channel-v1.md#bytes-ids-and-the-fixed-m1-suite)
own LP framing, list framing, exact UTF-8 and canonical binary encodings. Only
those byte primitives are reused here; the [channel boundary](#channel-boundary)
names the sections that move to remote. Extension contracts and vectors remain under this extension,
following the [Office contract convention](../../tmt-office/contracts/README.md#single-source-of-truth).

Inputs are the [owning design](https://github.com/wkh237/tmt/issues/828#issuecomment-5932303929)
(revision 9, including the lead's baseline, decoder and trusted UI decisions), its
[revision 5 security review](https://github.com/wkh237/tmt/issues/828#issuecomment-5933591928),
and the [#829 report](https://github.com/wkh237/tmt/issues/829#issuecomment-5933403999),
[squad acceptance](https://github.com/wkh237/tmt/issues/829#issuecomment-5933426455)
and [security acceptance](https://github.com/wkh237/tmt/issues/829#issuecomment-5933470946).
#829 acceptance is bounded spike evidence, not production-crypto acceptance.
The [#830 final report](https://github.com/wkh237/tmt/issues/830#issuecomment-5933724127)
and [squad acceptance](https://github.com/wkh237/tmt/issues/830#issuecomment-5933744205)
supply decoder, renderer, anchoring, door and TLS evidence. They leave production
containment and durable transport to the named slices.
Baseline and hostile-corpus containment acceptance remain C0 review gates.

## Channel boundary

Colab is an app on remote. The [remote channel contract](../../../contracts/remote-channel-v1.md#extension-channel-api)
owns owner-device identity, door route mounting, the opaque relay, agent
operations, agent status and backends/deploy; colab consumes that API and ships
no door, sign-in, pairing or backend of its own once its routes mount on the
remote door. Colab keeps canonical values and content cryptography, encrypted
objects, links, page membership and roles, epochs, sharing and rotation,
snapshots, decoder isolation, renderer and anchors, Send and its ledger, page
retention policy and conformance. Each affected section below carries a marker:

| Section                                   | Disposition                                                                                                                                          |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Trust root, statements and device chains  | Split: space ID, owner membership log and page members' device certificates stay; owner-device enrollment and chains move to remote device identity. |
| Browser management requests               | Stay as colab requests carried over the relay; the sender is the remote device context.                                                              |
| Local sign-in and owner-device enrollment | Move to remote device identity and pairing.                                                                                                          |
| Pairing and machine-local grants          | Retired: pairing moves to remote; agent access follows remote trust grants.                                                                          |
| Explicit Send and bridge ledger           | Split: explicit Send and the ledger stay; dispatch and recovery use remote operations.                                                               |
| Sync and backend admission                | Split: edge admission, bindings and transport frames move to the remote relay; page, epoch, role and writer checks stay as colab's admission hook.   |
| Retention and management                  | Split: page expiry policy and warnings stay; backend enforcement uses remote-provisioned resources that colab declares.                              |

Until the remote implementation lands, the moved sections describe the local
colab pilot; its working code relocates into `tmt-remote` rather than being
rewritten. The retired machine-sender amendment's principles (page membership
never implies agent access; frozen operation ID and bytes; uncertainty recovery;
recipient-only results) are owned by the remote channel contract.

## Product boundary and threat model

A space is one colab instance holding many live pages. Pages are collaboratively
edited HTML source, comments and agent conversations; there is no publish step
or immutable-version collaboration model. Snapshots are named restore points.
People use the browser URL; agents use the CLI. A share URL grants page access
at its role and MUST NOT grant local-agent access. A member may ask only their
own agents on their own machine ([Member machines](#member-machines)); page
membership never reaches the owner's or another member's agents.

The extension protects against an untrusted storage/sync service reading private
page content, forging authorship or authority, rolling back already-observed
membership, or converting stored data into agent work. Link holders and members
MUST remain within their roles. Page HTML MUST NOT reach app keys, sessions,
the bridge or another page through the renderer channel.

The app JavaScript host, compromised browsers and same-user malware are outside
this protection. A malicious app host can invoke non-extractable keys. Metadata
(space/page/device IDs, sequence numbers, sizes, timing and public keys) is
visible to storage. Public data is intentionally disclosed. Revocation cannot
recall plaintext or keys already copied; lazy deletion is not physical erasure.

A client is only as fresh as its highest verified membership head. A backend
cannot forge a higher owner-signed head or roll back a head the client retained,
but can withhold a newer revocation the client has never seen. “Latest verified”
MUST NOT be described as globally current. Machine-local grants independently
fence agent effects.

Local acceptance precedes Firestore, then Cloudflare. Background service mode,
a per-page dashboard record store and official installation/release registration
are outside v1. `serve` runs in the foreground. A future background lifecycle
must follow Office's start/stop/status shape.

## Canonical values and cryptographic suite

All domains below are exact ASCII. `LP(...)` and framed lists use the linked
Remote byte primitive; colab specifies its own field orders and bounds here.
Version is ASCII `1`. No JSON reserialization, delimiter concatenation, Unicode
normalization, signature prehash variant or algorithm negotiation is allowed.

| Value                             | Admission                                                                                                     |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Space ID                          | 32 lowercase RFC 4648 base32 characters, no padding                                                           |
| Generated IDs                     | Canonical lowercase non-nil UUIDv4; core agent references use the public API's canonical non-nil UUID grammar |
| Object ID                         | 64 lowercase hex characters representing 32 internally generated random bytes                                 |
| Epoch, revision, sequence         | Positive canonical decimal u64 strings, at most 20 bytes; zero only for defined sentinels                     |
| Time                              | Nonnegative UTC milliseconds, safe integer at most 2^53−1, at most 16 decimal bytes                           |
| Digest / public key / root secret | Exactly 32 raw bytes in cryptographic inputs                                                                  |
| Ed25519 signature                 | Exactly 64 raw bytes                                                                                          |
| ID list                           | Sorted, unique, at most 256 IDs; framed size at most 10,244 bytes                                             |
| Binary transport                  | Canonical unpadded base64url; reject padding, alternate encodings and nonzero unused bits                     |

Strict decoders MUST reject duplicate/unknown fields, unsupported kinds/versions,
invalid Unicode, incorrect lengths, noncanonical integers, trailing bytes and
ambiguous encodings before any authority use. Serialized JSON is UTF-8 without
BOM; exact payload bytes are retained and hashed without reconstructing JSON.
Mutable byte inputs MUST be copied before asynchronous crypto operations.

The fixed suite is `aes256gcm-hkdfsha256-ed25519-v1`: AES-256-GCM with a 128-bit
tag, HKDF-SHA256, ordinary Ed25519 and RFC 9180 HPKE Base
DHKEM(X25519, HKDF-SHA256)/HKDF-SHA256/AES-256-GCM
(0x0020/0x0001/0x0002). Browser operations use native WebCrypto, not custom
curve/cipher implementations. Rust uses the #829 exact pins: `aes-gcm` 0.11.1,
`hpke` 0.14.1, `ed25519-dalek` 3.0.0, `sha2` 0.11.0, `hmac` 0.13.0 and
`getrandom` 0.4.3. Implementing slices MUST verify the resolved feature graph,
platform, MSRV, licenses and advisories; no independent audit covers this exact
graph. Yjs update-v1 uses Yjs 13.6.32 / yrs 0.28.0; yrs requires the separately
reviewed MSRV 1.95 change in #841 before adoption. The #830 lock scan found
smallstr 0.3.1 / RUSTSEC-2026-0215 (unmaintained, no patched version), not a
known vulnerability; implementing slices must record its disposition. Do not
silently substitute the older MSRV-compatible yrs 0.26 pin.

The joint documented browser feature floor is Chromium 137, Firefox 130 and
Safari 17. These are not tested minimum binaries. Probe generate/sign/strict
verify, X25519 generate/derive and secure-context availability before use; fail
closed with an update-browser explanation, with no weaker fallback.

### Strict signature admission

Every enrollment, statement, certificate, object, intent, wrap and pairing
signature MUST use one strict verifier. Browser admission requires A32/R32/S32,
masked Edwards y < p for both A and R, rejection of all eight torsion encodings
including sign-bit/negative-zero variants, and little-endian S < L, followed by
native WebCrypto verification. Byte comparisons implement guards; curve
operations stay native. Rust requires canonical recompressed non-weak public
keys and `verify_strict`, with no legacy, hazmat or batch path. Raw browser
verification alone MUST NOT authorize anything. Valid mixed-order positives in
the accepted corpus MUST NOT be categorically rejected.

### Immutable encrypted objects

`seal` MUST generate the object ID internally with OS/WebCrypto CSPRNG entropy.
It MUST reject caller-supplied IDs and deterministic fixture inputs. Derive:

```text
Kobject = HKDF-SHA256(epochSecret32, salt=empty,
  info=LP("tmt-colab-object-key-v1", header), L=32)
nonce = twelve zero bytes
```

Exactly one seal is allowed per derived key. A retry retransmits the same frozen
envelope; a new seal generates a new ID. The zero nonce is safe only with this
single-use derived-key rule. Ceilings are 2^32 objects per page epoch and 16 MiB
plaintext per encrypted chunk; operation-specific limits below are smaller.
Aggregate accounting and same-sequence arbitration MUST be enforced.

The canonical header is at most 1,024 bytes and is the AEAD associated data. The field order below extends
#829 with `namespace`; old #829 vectors do not cover it. `streamId` is the
`authorDevice` ID within the `(space, page, epoch)` scope, matching #829's stream
cut identifier; no unsigned transport ID selects a writer. A stream sequences
both namespaces together. The authenticated page, epoch and author uniquely
identify that stream.

| Domain                         | Ordered fields after domain                                                                                                   | Meaning                                                                                  |
| ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `tmt-colab-space-id-v1`        | ownerEdPublic                                                                                                                 | SHA256, first 20 bytes, lowercase base32                                                 |
| `tmt-colab-object-v1`          | version, suite, space, page, epoch, kind, namespace, object, authorDevice, membershipRevision, streamSeq, prevEnvelopeHash    | Header / AAD; kind `update`, `checkpoint`, `html`, `asset`; namespace `content` or `own` |
| `tmt-colab-signature-v1`       | header, nonce12, SHA256(ciphertextWithTag)                                                                                    | Device signature                                                                         |
| `tmt-colab-envelope-hash-v1`   | header, nonce12, ciphertextWithTag, signature64                                                                               | SHA256 of the exact complete envelope                                                    |
| `tmt-colab-membership-v1`      | version, space, revision, previousStatementHash, operation, SHA256(payloadBytes)                                              | Owner signature; operation at most 32 ASCII bytes                                        |
| `tmt-colab-membership-hash-v1` | statementBytes, ownerSignature64                                                                                              | SHA256 of exact signed statement                                                         |
| `tmt-colab-device-cert-v1`     | version, space, issuerKind, issuerId, deviceId, deviceEdPublic, deviceXPublic, membershipRevision, issuedAt, expiresAt        | Issuer kind `member` or `link`                                                           |
| `tmt-colab-stream-cut-v1`      | version, streamId, namespace, checkpointEnvelopeHash, checkpointSeq, tailHeadSeq, tailHeadHash                                | Namespace-bound checkpoint commitment and authenticated tail boundary                    |
| `tmt-colab-wrap-v1`            | version, suite, space, page, epoch, recipientKind, recipientId, recipientXPublic, signerEdPublic, membershipRevision, purpose | At most 1,024 bytes; suite `base-x25519-hkdfsha256-aes256gcm`; purpose `epoch-key`       |
| `tmt-colab-hpke-info-v1`       | wrapHeader                                                                                                                    | HPKE info; wrapHeader also HPKE AAD                                                      |
| `tmt-colab-wrap-signature-v1`  | version, wrapHeader, enc32, SHA256(wrappedCiphertextWithTag)                                                                  | Owner signature authenticates HPKE Base sender                                           |

A transport object contains exactly `{header, nonce, ciphertext, signature}`
as binary fields. `header` is the exact framed input, not a JSON header rebuilt
by the verifier. Decrypt only after syntax, signature, chain, stream order,
epoch and role admission; plaintext shape validation still precedes application.
For a stream's first update `prevEnvelopeHash` is zero32; every successor update
cites the exact preceding update envelope hash. A checkpoint's sequence is its
covered update head n, and its previous-hash field binds that update head hash;
it is published separately by immutable object ID and does not consume an update
sequence or replace an update at n. A non-stream object (`html` snapshot or
`asset`) uses sequence `0` and zero32 previous hash, and a separately validated
signed descriptor; it MUST NOT advance a stream. `update` and `checkpoint`
require a positive sequence. An `html` object is `content`; an asset's signed
descriptor binds its namespace and page reference.

Wrap transport is strict binary JSON `{header, enc, ciphertext, signature}`:
header is the exact framed wrap input (at most 1,024 bytes), enc is 32 bytes,
ciphertext is exactly 48 bytes (32-byte epoch secret plus 16-byte tag), and
signature is the 64-byte owner signature over the wrap-signature input.
Wrap lists are sorted by `(recipientKind, recipientId)` bytewise, unique and at
most 512 entries. Invalid or oversized data rejects; it is never truncated.

Wrap recipient kinds are `member`, `device`, `link`, `bridge`. The verified log
must resolve the recipient ID/key and owner signer; header values are not
self-authorizing. HPKE all-zero DH results MUST reject. Owner-signed wrapping
binds space, page, epoch, recipient, key, signer, revision and purpose.

### Link derivation and key storage

For a random 32-byte link seed, pin the following independent derivations:

```text
EdSeed = HKDF-SHA256(linkSeed, empty,
  LP("tmt-colab-link-signing-seed-v1", space, linkId), 32)
XSeed = HKDF-SHA256(linkSeed, empty,
  LP("tmt-colab-link-encryption-seed-v1", space, linkId), 32)
joinProof = HMAC-SHA256(linkSeed,
  LP("tmt-colab-join-v1", space, linkId))
```

Import `EdSeed` as an Ed25519 seed and `XSeed` as the X25519 private input with
standard X25519 clamping, using native/Rust library primitives. Conversion from
Ed25519 key material to X25519 is forbidden. `link.add` pins both derived public
keys. `epoch.advance` wraps to surviving link X25519 keys, allowing a holder to
join without the owner online. New link-X25519 and namespace vectors are required
in L1; #829 proved only the signing derivation. Join proof is edge admission,
not membership authority or an agent grant.

Keep browser root HKDF and signing/encryption private CryptoKeys non-extractable
in IndexedDB. Link import may temporarily obtain the public half from the
already-present bearer seed, then reimport private keys non-extractable. Byte
clearing is best effort. Production HPKE MUST use persisted non-extractable
recipient handles, not fixture raw-secret APIs. Strip the fragment with
`history.replaceState` after controlled import; never include it in requests,
logs, analytics, previews, agent text or renderer messages. Deliberate secret
sharing/recovery requires trusted UI; a non-extractable key cannot simply be
exported later. Native secrets belong to the extension keyring, separate from
configuration, argv, output and logs. Same-user filesystem permissions do not
isolate mutually hostile agents.

## Trust root, statements and device chains

**Channel boundary: split.** Owner-device enrollment and chains move to remote device identity; the space ID, owner membership log and page members' device certificates stay.

The URL's space ID pins the Ed25519 owner key through the space-ID derivation.
A substituted key MUST reject. The owner signs a hash-chained membership log,
starting at revision `1` with previous hash zero32. Successors advance revision
by exactly one and cite the prior signed statement hash. Clients persist their
highest verified `(revision, hash)` before applying dependent state; lower
heads, forks or unknown operations report “space state rolled back” and apply
nothing. Backend membership/index records are only edge-admission projections.

A statement transport contains exactly `{statement, payload, signature}`; exact
binary payload bytes are hashed in the statement. An owner-statement payload is
at most 768 KiB serialized, including all nested material; exceeding any cap
invalidates the statement, never truncates it. Payloads decode as strict
typed JSON. The operation-specific fields below are exact; keys/digests are
canonical base64url, IDs/numbers follow the value table. History access
follows the page's `page.history` mode, never possession of a new key.

| Operation       | Payload fields / semantic constraints                                                                                                                    |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `member.add`    | `memberId, role, signKey, encKey, pages`; role viewer/commenter/editor; see [history modes](#current-view-baseline-and-history-modes) for earlier epochs |
| `member.remove` | `memberId, cuts`; remove current authority and advance affected page epochs                                                                              |
| `member.role`   | `memberId, role, cuts`; reductions commit affected streams; promotion grants no retroactive authorship                                                   |
| `link.add`      | `linkId, role, linkSignKey, linkEncKey, pages`; role viewer/commenter/editor; keys match pinned derivations                                              |
| `link.remove`   | `linkId, cuts`; revoke every device certified by the link and rotate affected epochs                                                                     |
| `device.revoke` | `deviceId, cuts`; revoke the selected identity/sessions/grants and rotate affected epochs; a surviving link seed remains a separate bearer capability    |
| `bridge.add`    | `machineId, machineSignKey, encKey, pages`; an owner machine with the restricted bridge role, not editor                                                 |
| `epoch.advance` | `pageId, epoch, cuts, baseline, wraps`; next epoch, exact baseline descriptor, remaining-recipient signed wraps                                          |
| `page.share`    | `pageId, mode, epoch, publishedKeys`; private/link/public; key publication only for loopback public mode; earlier epochs only under `shared` history     |
| `page.history`  | `pageId, mode`; `shared` (default when absent) or `current`; owner control only; applies to later joins                                                  |
| `retention.set` | `pageId, days`; positive safe-integer day count or null for forever                                                                                      |
| `page.archive`  | `pageId`; hide from active lists and freeze writes                                                                                                       |
| `page.delete`   | `pageId`; cease access and remove backend ciphertext; never revive through replay                                                                        |

The root owner is implicit, not an assignable member role; v1 has no co-owner
or signing delegation. Every log statement is produced and signed by the owner's
`tmt colab`, whose keyring alone holds the root key. Page creation allocates a page ID, initial epoch
and baseline through the owner; `create` imports source without executing it.

### Browser management requests

**Channel boundary: stays,** carried over the relay with the remote device context as sender.

The owner's enrolled browser never receives the root key. To request sharing,
member/link changes, epoch advance, script policy or retention, it submits a
device-signed typed management request to the owner's `tmt colab`. Canonical
input is LP("tmt-colab-management-v1", "1", space, page, expectedRevision,
operationId, operation, SHA256(payloadBytes), senderDevice, issuedAt, expiresAt).
The payload is exact strict typed JSON describing the requested operation's
user-selected fields above, not machine-computed cuts, baselines or wraps;
the request names the affected page, not a caller-selected signing key. Transport
contains exactly `{request, payload, signature}` as binary fields. Operation
IDs are unique, and the validity window is at most ten minutes. Space-wide
member actions name their affected page set in the typed payload; the page
field binds the initiating page context, never widens that set.

Verify the live owner-device session, strict possession signature, certificate
chain, owner member binding, page scope and expiry before signing. Under the
owner mutation lock, identical operation/digest retries return the recorded
signed outcome without re-signing. A conflicting digest rejects; a new operation
must match the expected log revision before signing. Non-owner devices cannot invoke this signing path even
with editor permission. The owner's `tmt colab` checks current policy, computes
any epoch baseline through the isolated decoder, and atomically signs/commits
the resulting statements and transition. A request cannot supply an unverified
baseline to be signed. The browser verifies returned owner-signed statements
through the usual log admission; a transport success alone is not a new head.

On cloud backends membership, sharing, rotation and other root-signed changes
require the owner's machine online. The cloud service never holds the root key
or signs in its place. An offline management request remains unavailable; it
MUST NOT be executed later without rechecking its expiry and expected revision.

`cuts` is a sorted unique list of `{pageId, epoch, namespace, cut}` where `cut`
is the exact framed stream-cut bytes. At most 512 cuts are allowed, sorted and
unique by `(pageId bytewise, epoch numerically, namespace bytewise, streamId
bytewise)`; duplicates or out-of-order entries invalidate the statement.
Its signed payload scope resolves a single
device stream and namespace; the framed cut's namespace MUST match that wrapper.
Both namespaces are committed when affected. The
checkpoint hash is either hash32 or zero-length `none` paired with checkpoint
sequence `0`. Require checkpointSeq <= tailHeadSeq; an empty tail uses seq `0`
and hash zero32. A nonempty tail must resolve through its exact chain. Reduction
cuts MUST cover every affected stream; unseen offline updates beyond the cut
are rejected and reported to their writer.

A certificate chain is bounded, versioned material, not arbitrary recursive
certificates. It contains exactly `{version:1, issuerStatement, deviceCertificate,
issuerSignature}` with at most 16 KiB serialized bytes. `issuerStatement` is an
already-verified owner-log statement hash; `deviceCertificate` is the exact
framed device-cert input; `issuerSignature` signs it. For `member`, the resolved
`member.add` key certifies the device. For `link`, the resolved `link.add`
signing key does so. The chain is owner → member/link → device, maximum two
signature edges, no delegated issuers or cycles. The canonical chain digest
is SHA256 of `LP("tmt-colab-chain-v1", "1", issuerStatement,
deviceCertificate, issuerSignature)`; L1 supplies new grammar vectors.

Certificate fields MUST match the space, resolved issuer, device keys, applicable
membership revision and validity interval. A valid certificate cannot outlive
issuer revocation or grant roles not present in the current log. Bridge streams
use the owner-pinned machine key from `bridge.add`; a bridge cannot certify
human/link devices. Keys from transport records cannot replace log bindings.

### Local sign-in and owner-device enrollment

**Channel boundary: moves** to remote device identity and pairing.

`serve` prints a single-use sign-in URL whose secret is a uniformly random
128-bit code carried only in its fragment, with expiry at most ten minutes.
The URL also identifies a nonsecret code ID and pinned space. Trusted browser
code imports/removes the fragment before renderer creation; it never enters
HTTP URLs, logs or analytics. The code stays only in the owner's running process
memory; durable expiry/consumption state stays in the extension's private subtree.
Server exit invalidates outstanding codes; a restart issues a fresh one.

The browser generates device Ed25519/X25519 keys and a nonce16. It sends the
code ID, space, device ID, both public keys and nonce, with canonical input
LP("tmt-colab-signin-v1", "1", codeId, space, deviceId, deviceEdPublic,
deviceXPublic, nonce16). Its proof is full HMAC-SHA256(code, input); possession
is a device signature over LP("tmt-colab-signin-possession-v1", "1", input).
The secret code is never sent as a transport field. Wrong proofs/signatures,
expired codes, replay and second use reject without certifying or issuing a
session.

After verifying the code proof and possession, the owner's `tmt colab` issues
the framed `device.cert` under the owner's member signing key. The owner's
member ID/key binding is pinned in revision 1's owner-signed `member.add`
statement with editor role. Only that initial member is the owner's management
principal; later member additions cannot claim it or reuse its keys. The owner
management member cannot be removed or re-roled. Root
ownership remains implicit and is not a role
that another member can obtain. This certificate identifies an owner-enrolled
device for page/management access; it grants no local-agent access. Enrollment
consumes the code and records the exact device binding atomically, before
returning the certificate/session.

The server issues a 256-bit random session token, stores only its SHA256 hash,
and binds it to the certified device, space, finite expiry (at most 24 hours)
and current revocation state. Deliver it as an HttpOnly, SameSite=Strict cookie
scoped to the space (Secure on HTTPS); the browser automatically presents that
Cookie header on admitted API/upgrade requests. Browser WebSocket construction
cannot set an arbitrary authorization header, so tokens MUST NOT be moved into
query strings as a workaround. The token never enters a URL, renderer message
or log. `device.revoke` invalidates its sessions immediately when the
local server applies the verified statement. Every API/upgrade checks expiry,
device binding and revocation; a cached token cannot revive a revoked device.
The sign-in code, device certificate and server session do not create a machine-
local agent grant; that requires the separate pairing ceremony below.

## Page state, roles and epochs

Each device has one signed append stream per `(space, page, epoch)`. Sequences
are contiguous from `1`; readers buffer bounded gaps and apply n only after
n−1. Identical envelopes replay without changing state; two different envelopes
for one sequence freeze and flag the stream. Backends store create-only at the
scoped `(streamId, seq)` key. One browser tab owns the device's write lock with
`navigator.locks`; other tabs relay through it. CLI writes hold a file lock.

`namespace` is authenticated in the header, AEAD and signature. `content` and
`own` have separate documents and decoders; unsigned routing cannot choose a
document. The following roots are exhaustive:

| Namespace | Roots                                               | Fold and authority                                                                   |
| --------- | --------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `content` | `html` Y.Text; `meta` Y.Map containing only `title` | Shared page document, folded from admitted owner/editor streams in the current epoch |
| `own`     | `threads`, `messages`, `intents`, `replies` Y.Maps  | Separate document per writer; only that writer's signed stream mutates it            |

Mixed-root updates or other root names/types reject before atomic application.
Sharing, roles, script policy, retention and local grants MUST NOT be Yjs state.
Author fields and Yjs client IDs are untrusted; identity/role derive from the
verified stream. Cross-stream references confer no authority. Character-level
attribution is not promised; the claim is “changed by a writer with edit
permission at revision R.”

Viewers read only. Commenters open threads, write/edit/delete their own messages
and resolve their own threads. Editors also edit shared content, make/restore
snapshots and resolve any thread. The root owner additionally controls the log.
Bridges publish send state and agent replies only for their own ledger entries.
Unauthorized values are ignored and flagged, never promoted into authority.
Role admission applies equally to updates and checkpoints. A commenter/bridge
cannot hide content inside a checkpoint; a demoted editor cannot publish fresh
content beyond its committed cut by citing an old membership revision.

Threads contain an ID, optional anchor and resolution state. Messages contain
an ID, thread reference and exact plain-text body. Intents contain the immutable
signed send input, its signature and frozen final bytes. Replies contain the
operation correlation, ledger state and bounded core-final copy. Deletions are
writer-owned tombstones; references to another writer's thread do not permit
changing that thread's ownership or messages. Editor resolution of another
writer's thread is an attributed action in the editor's own stream, not a write
into the other writer's document. Conflicting resolution projections MUST use
verified log revision, then stream sequence and bytewise writer ID as the stable
tie-break order; they never authorize sends.

### Current-view baseline and history modes

Every epoch advance, including member addition under `current` history,
MUST atomically commit an owner-signed baseline descriptor and an encrypted copy
of the exact current HTML source under the new epoch. The descriptor binds
`pageId, epoch, sourceDigest, baselineCommitment, title, objectEnvelopeHash, membershipRevision` in
the signed epoch payload. The owner computes it from its authenticated fold
through the isolated decoder. The baseline object contains source text and one
update-v1 produced once by the owner from a fresh Y.Doc inserting that source/title.
It uses an `html` object in `content`, with sequence zero; the signed descriptor
distinguishes it from snapshot HTML objects. Its commitment is
SHA256(LP("tmt-colab-baseline-v1", "1", exactSourceBytes, exactUpdateBytes));
the signed descriptor binds both that commitment and sourceDigest. Every client
applies that identical update and struct identity, never inserts the HTML
independently. Only new-epoch content updates fold on top; old-epoch updates MUST
reject for the new document. Isolated decoding must also verify the update
materializes exactly the committed source/title. L1/L3 vectors pin the baseline
bytes/commitment, cross-client convergence, old-epoch denial and digest mismatch.
The baseline is an explicit epoch reset, not a checkpoint reattributing others'
old updates to the owner.

A page's history mode is `shared` by default. A member joining a `shared` page
receives, in the same owner transition as `member.add`, owner-signed wraps of
every retained earlier epoch key of that page; no epoch advance is needed, and
persisted content, own streams, comments, deleted text and snapshots stay
readable to them. The owner may set `page.history` to `current` per page. A
member joining a `current` page receives only the current source/title through
an epoch advance with a baseline, and no earlier epoch keys, old own streams,
deleted text or old snapshots. A mode change affects later joins only; keys a
recipient already holds are never recalled. Trusted share UI MUST state which
mode applies before adding a member or link.

Forward wraps are bounded. Under `shared`, a page shares at most its 64 most
recent epochs (the current epoch plus 63 earlier, the same bound as public
`publishedKeys`); older epochs are not wrapped to later joiners, and the share
UI says that history before that point is not shared. One join is delivered as
one or more wrap lists of at most 512 entries each, all committed in the same
owner transition (one local SQLite, Firestore or DO storage transaction), so a
join either receives every bounded wrap or none. Acceptance includes a page at
the epoch cap and a multi-page join that needs several wrap lists.

Existing anchors remap through quote/context at the epoch reset and detach on
mismatch; old Yjs relative positions MUST NOT be applied to a new document.
Offline edits in the old epoch MUST NOT be silently reissued under the new one;
authority and an explicit new edit are required.

Links follow the same mode. Under `shared`, `link.add` carries wraps of the
retained earlier epoch keys to the link key. Trusted share UI MUST then state
plainly that anyone with the link can read the page's whole shared history,
including deleted text, snapshots, comments and agent replies. Under `current`, a link join reads
everything in the current epoch since its last advance and no earlier epochs; an
owner can cut that window with an epoch advance. There is no automatic
per-link-holder history reset.

### Rotation and sharing

An epoch advance generates a fresh secret and wraps it only to remaining
recipients eligible under the resulting sharing mode. Commit statement, baseline, wraps, page epoch
and removed-writer edge projections together: one local SQLite transaction,
Firestore transaction or DO storage transaction. Before commit the old epoch
remains valid; after commit stale writes reject and clients fetch the higher
revision before writing. Reads, subscriptions, appends, compaction and scoped
acks MUST recheck applicable admission when authority changes. A revoked device's
identity, session, grant, old epoch key or checkpoint cannot recover its revoked
authority. A retained seed for a surviving link is an independent capability:
its holder can decrypt that link's new-epoch wrap and certify a fresh device.

Rotating a link token alone is not revocation. `device.revoke` on a link-certified
device revokes only that device identity, sessions and grants; it does NOT exclude
a bearer retaining the link seed. Trusted share UI's Remove device action MUST
state this limitation and direct removal of a link holder to Reset link.
Excluding a link holder requires `link.remove` (revoking that link and every
device certified by it), plus an epoch advance whose wraps go only to intended
remaining recipients. If sharing continues, explicit owner `link.add` creates a
NEW link identity/seed distributed only to intended holders. Reset link MUST
perform this removal/rotation, not just change a URL or hide an edge record.

Private pages admit named members only. Link pages additionally admit
link-certified devices at the link role. Public mode is loopback-only in v1;
Firestore and Cloudflare MUST reject public mode and key publication. Going
public first advances the epoch with a baseline, then publishes only the new
epoch key in an owner-signed statement. This discloses current live source and
everything protected by that key thereafter: own streams, comments, intents,
agent-reply copies and attachments. Earlier epochs are published too unless the
page history mode is `current`: trusted confirmation MUST state plainly that
going public publishes the whole shared history, including deleted text,
snapshots, comments and agent replies, and MUST state that exact
scope. Public HTML with private discussion is not supported by this key boundary.
Public readership grants no writing, device certification, grant or Send access.

Every audience-narrowing mode change (`link` → `private`, `public` → `private`,
`public` → `link`) is one atomic owner transition: advance the epoch with its
baseline, filter wraps by the NEW mode, and remove/disable every existing page
link with `link.remove`. Private recipients are the implicit root owner, named
members, their certified devices and bridges only; neither link principals nor
link-certified devices receive a private-epoch wrap. Link-device edge admission
ends and all their subscriptions terminate in the same transition. Links are
revoked, not merely hidden by an index/edge projection. Re-enabling link sharing
requires an explicit owner action creating a NEW link identity; selecting link
mode never reactivates a removed identity or its old bearer seed.

`page.share` publishedKeys is a list of strict `{epoch, key}` entries: epoch is
a canonical positive decimal string, key is a canonical binary 32-byte epoch
secret. Entries are unique, sorted by numeric epoch and at most 64. Public mode
contains exactly the new current epoch plus, under `shared` history, the retained
earlier epochs; all earlier entries are below the current epoch. Other
modes require an empty or absent list. Null is not a list. These syntax bounds do
not replace the resulting-mode recipient filtering and atomic transition above.

Leaving public also ends public subscriptions and stops public distribution of
new keys/objects. Already-public content/history remains public forever. Clients
derive sharing from the log, never a mutable page index.
The server index (`pageId, state, lastUpdateAt, expiresAt, epoch, writer list`)
is an admission/management projection; trusted space-home labels derive from
verified statements and decrypted metadata.

### Snapshots and restore

A snapshot contains a self-contained encrypted copy of exact source text and
a device-signed descriptor with canonical input
LP("tmt-colab-snapshot-v1", "1", space, page, snapshotId, authorDevice,
membershipRevision, sourceDigest, objectEnvelopeHash).
Its author must be owner/editor at creation. It remains for page
retention and is removed only explicitly or with page expiry/deletion; it MUST
NOT depend on stream updates compaction may delete. Restore checks current edit
permission and creates a new minimal text diff under the current epoch. It MUST
NOT restore membership, sharing, grants, intents or replies. Missing or
undecryptable snapshots return “unavailable”, never substitute newer state.
Snapshots are history, not a page-version switch.

## Decoder isolation, compaction and limits

No process holding authority (local server, bridge or CLI parent) may decode or
merge foreign-writer Yjs updates in-process. Servers never decode Yjs at all:
they store opaque ciphertext. The #830 hostile-update smoke with yrs 0.28 on
rustc 1.95 produced five caught panics and one timeout in 261 cases. Catching
panics is insufficient. Isolation is crash/resource containment for malformed
data, not a sandbox against decoder code execution.

Rust decoding/merging MUST run in a bounded child, re-invoking `tmt-colab`
through `tmt-invoke`, with an owned process group, deadline, input/output caps,
platform memory limit where supported, and confirmed cleanup. Give the child
only necessary plaintext bytes on stdin, no keyring/grant paths or secret
environment and no network use. It retains the OS user's residual filesystem
authority; no filesystem or network sandbox claim is made. The existing invoke
leaf owns process lifetime, not decoder semantics; missing launch controls must
be reviewed in the implementation slice, not assumed to exist today.

Browser foreign-update decoding MUST run in a dedicated Web Worker with a time
budget, terminated on overrun. Give it only needed plaintext update/document
inputs, never keys, signing handles or bridge capabilities. A same-origin Worker
can access IndexedDB/network; this is not key isolation against hostile decoder
code. Its output is untrusted.

Validate child/Worker output against namespace roots, types, role and size limits
before atomic application. Panic, timeout, invalid output or cleanup failure
applies nothing and dispatches nothing; reject the update and flag the stream.
No repeated launch may bypass unconfirmed cleanup. At most one decoder runs per
page. The retained hostile corpus uses a one-second per-case deadline and
45-second suite budget, distinct from the production defaults below. L2/L4
must prove termination/backpressure at those production bounds. #830 established
process-time containment, not macOS memory containment. Its fixture budgets
MUST NOT be advertised as measured production limits.
Keep the hostile corpus and timeout/cleanup/failure-propagation gates. Minimized
reproducers should be checked against upstream fixes before submission to yrs;
this contract does not authorize external reporting by itself.

Compaction is by the stream's own original device, from only that writer's
verified update set using Yjs `mergeUpdates` / yrs `merge_updates_v1`. Never use
`encodeStateAsUpdate` of the merged shared document: that would reattribute
others' updates. A checkpoint covers seq 1..n and embeds the authenticated update head
hash at n, preserving namespace-specific updates and dependencies without
advancing the update chain. A namespace checkpoint covers that namespace's
subset within the shared sequence prefix; the signed descriptor binds the
prefix head. Commit the checkpoint before deleting only covered updates of its namespace.
Retain the update hash/receipt ledger needed to verify interleaved namespaces
and exact retries after payload prune; never delete another namespace's payload
solely because its sequence falls in the prefix.
Crash before deletion retains replay-safe redundant data; concurrent tail
updates survive. A gone device's stream remains as signed data within quotas.
There is no cross-writer compaction checkpoint in v1; the epoch baseline above
is a distinct owner-authorized reset.

Authority reductions commit the exact checkpoint envelope hash, checkpoint
sequence and tail head/hash. A revoked device's newly signed replacement cannot
be admitted solely because it cites the old head. Checkpoint signatures do not
prove current authority. Dependency/delete-set preservation, concurrent
compaction and revocation cuts need Rust/browser interop evidence. Load cost
must be bounded and measured in L3/L4 before a performance promise. Compare decoded state-vector client clocks, not
encoding byte order; declare all schema root types before projection.

| Default limit                       | Value                        |
| ----------------------------------- | ---------------------------- |
| Exact HTML source / snapshot source | 2 MiB each                   |
| Message body                        | 16 KiB UTF-8                 |
| Threads per page                    | 1,000                        |
| Update-envelope plaintext           | 256 KiB                      |
| Compaction trigger per stream       | 200 updates or 256 KiB tail  |
| Per-device append rate              | 10/s sustained, burst 50     |
| Per-page decoder concurrency        | 1                            |
| Rust decoder batch deadline         | 2 seconds                    |
| Rust decoder baseline plaintext     | 2 MiB                        |
| Rust decoder aggregate update batch | 256 KiB, at most 200 updates |
| Rust decoder input/output streams   | 4 MiB each                   |
| Linux decoder address-space limit   | 512 MiB                      |
| Spark deletion budget               | 500/page/day                 |

Linux sets and verifies its address-space limit before reading child input;
failure rejects the job. On macOS and platforms without enforced memory limits,
run with deadline/output containment and report `memory limit unavailable`.

These are pinned v1 defaults; tuning MUST preserve cryptographic ceilings and
bounded admission. Enforce bounds before allocating/decoding, not only after
merge. L2/L4 must prove serialized checkpoint/chunk budgets, bounded gap/queue
accounting and durable prune/receipt behavior with the wire below. #830's
in-memory fixture does not establish these durable properties. Budget exhaustion backpressures
or rejects explicitly; it MUST NOT silently discard accepted durable data.

## Pairing and machine-local grants

**Channel boundary: retired.** Pairing moves to remote; agent access follows remote trust grants.

Page enrollment, including the one-time sign-in link printed by `serve`, grants
page access only. Agent access requires `colab-pair-v1`, distinct from Remote
`local-v1`. Machine-local grants contain `grantId, deviceKey, agentIds,
expiresAt, revision` and exist only on the owner's machine. Pairing scopes the
exact device, machine, space, agents and expiry.

The machine creates an offer with an offer ID, pinned space/machine/key, random
nonceM16 and expiry no more than ten minutes away, and shows a one-time uniformly
random 16-byte code C. Derive K with HKDF-SHA256(C, salt=ASCII offerId,
info=ASCII `tmt-colab-pair-v1`, L=32). C is not the four displayed words and is
not a password to stretch. Locate/tag constructions replace the early unframed
sketches in the design.

Every domain in this table includes version `1` immediately after its label:

| Domain                               | Fields after version / result                                                         |
| ------------------------------------ | ------------------------------------------------------------------------------------- |
| `tmt-colab-pair-offer-v1`            | offerId, space, machineId, machineEdPublic, nonceM, expiresAt                         |
| `tmt-colab-pair-device-v1`           | offerId, deviceEdPublic, certificateChainDigest, nonceD16                             |
| `tmt-colab-pair-transcript-v1`       | offerBytes, responseBytes; SHA256 = T                                                 |
| `tmt-colab-pair-possession-v1`       | T; device signature                                                                   |
| `tmt-colab-pair-grant-v1`            | grantId, revision, offerId, deviceEdPublic, agentIds, expiresAt, T; machine signature |
| `tmt-colab-pair-receipt-v1`          | grantBytes, grantSignature64; immutable receipt                                       |
| `tmt-colab-pair-recover-v1`          | offerId, T; separate device recovery signature                                        |
| `tmt-colab-pair-offer-tag-v1`        | offerBytes; HMAC(K)                                                                   |
| `tmt-colab-pair-device-tag-v1`       | T; HMAC(K)                                                                            |
| `tmt-colab-pair-grant-transcript-v1` | offerBytes, responseBytes, grantBytes; SHA256 = G                                     |
| `tmt-colab-pair-grant-tag-v1`        | G, grantSignature64; HMAC(K)                                                          |
| `tmt-colab-pair-recover-tag-v1`      | recoveryBytes, deviceRecoverySignature64; HMAC(K)                                     |
| `tmt-colab-pair-locate-v1`           | No fields; HMAC(C)                                                                    |

Offer, response, chain and receipt are each at most 16 KiB. Agents are a sorted
unique nonempty list of at most 256 core UUID references. Fingerprint is
SHA256(LP(`tmt-colab-pair-fingerprint-v1`, T)), without a separate version field:
first 44 bits, four 11-bit indices in the BIP39 English list at bitcoin/bips
commit `ce1862ac6bcffa1dd20aad858380e51e66e949ea`, file SHA256
`2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`.
The four words confirm the transcript out of band, not 128-bit code entropy.

The browser verifies tagOffer before trusting the machine key, then sends its
chain, nonceD, transcript possession signature and tagResp. The machine checks
all of them. Both display matching words. The owner confirms in the terminal
and chooses the exact agents and expiry (default 30 days). The browser MUST
retain its intended offer/response and compare returned approved grant fields.

Consume the offer, commit the grant and store the immutable exact receipt in
one transaction before returning machine signature/tagGrant. Three failed tags,
expiry, reuse or lack of confirmation close an uncommitted offer. “Nothing
stored” applies only before commit. A lost receipt can leave a real grant.
Recovery submits exactly `{offerId, T, possessionSignature, tag}`, reconstructing
the recovery input and verifying both device signature and HMAC. Until offer
expiry + ten minutes, return the same receipt byte-for-byte, with no new grant,
renewal or changed scope. Later show “result unknown”; `devices` exposes the
machine's grant. Altered transcripts/devices/offers/tags MUST reject.

## Explicit Send and bridge ledger

**Channel boundary: split.** Explicit Send and the ledger stay; dispatch and recovery use remote operations.

Comments, sync, replay, compaction and HTML scripts MUST NOT dispatch agent work.
Only explicit Send in trusted parent UI signs an immutable intent after showing
the exact final text, agent UUID, destination machine/online state, that the
ask and its reply are visible to everyone who can see the page and, only under a
`hold` grant, that the send waits for local approval.
All effectful actions (Send, share, approve, delete) live in trusted parent
chrome. The canvas retains a visible boundary and the selection popover is
parent-drawn and clamped to it. Ask agent is a separate confirmed step from
Comment; page-drawn controls never execute trusted effects. Screen layout,
visual tokens and navigation remain owned by design section 13. The preview
contains the selected quote, comment, page title and link without fragment.
Freeze exactly the preview bytes seen by the sender, including when live render
is paused; never silently replace them with current source before signing.

The canonical send input is LP(`tmt-colab-send-v1`, version, space, page, thread,
messageIds, machine, agent, operationId, finalBytesDigest, senderDevice, grantId,
grantRevision, issuedAt, expiresAt). The device signs it and stores the exact
final UTF-8 bytes in its encrypted own stream. Default intent validity is one
hour, maximum 24 hours. Enforce the linked core request limit on final bytes;
message-body bounds alone do not bound a composed request. A mutable Yjs field
is not execution authority.

For each operation ID the durable bridge ledger transitions:

```text
fence → dispatching → accepted | failed | uncertain   (direct grant, default)
fence → held → dispatching                            (hold grant, after approve)
fence | held → refused | expired
uncertain → accepted (receipt recovery) | abandoned
uncertain → dispatching (explicit eligible retry only)
```

The fence checks signature/device chain, current local grant revision,
revocation/expiry, intent window, self machine ID, space/page, selected agent
scope, sender authority at the latest locally verified head, and operation dedup.
Same ID/digest returns recorded state; different digest is `INTENT_CONFLICT`.
Run the fence under the bridge lock immediately before dispatch; under a `hold`
grant also run it at adoption into held. Approval does not extend validity.
Revocation while held or offline blocks dispatch.

Persist dispatching, then call public `dispatch.create` with frozen operation ID,
recipient UUID and exact bytes, anonymous originator. Core owns request/wake
semantics. A valid receipt becomes accepted; a definite core failure becomes
failed; timeout, lost output or crash becomes uncertain. Restarted dispatching
records become uncertain, never automatically resend.

Recovery reads `dispatch.show`: found becomes accepted; not found stays uncertain.
An explicit same-ID/bytes retry reruns the full fence and requires the original
child confirmed stopped. Within one live bridge invocation the process owner
reports `Cleanup::Confirmed`. Retry is eligible only on `Confirmed`; any other
started-failure cleanup, including `CallerOwned`, disables retry and keeps the
entry uncertain. Colab uses only default fresh-group invocation for core and
decoder children, so it never receives `CallerOwned`. `NotStarted` means no
call was made and retains the normal fence/approval path. After a bridge crash today's API does not establish
original-child identity/termination, so retry remains disabled with that reason.
No shared API extension is assumed. Abandon stops local tracking and says “may
still have been delivered”; it neither proves non-delivery nor cancels accepted
work. No automatic new operation or repeated wake is permitted.

The grant mode comes from the remote trust grant: `direct` (the default)
dispatches right after the fence, `hold` keeps the send held for local
`approve`.

### Member machines

A member with commenter or editor role may Ask agent, but only agents on a
machine of their own; the owner's agents answer only the owner. The member's
browser holds a device paired with that machine through remote, and the ask
travels as an ordinary remote operation under that machine's own grant, as the
[remote channel contract](../../../contracts/remote-channel-v1.md#extension-channel-api)
defines. The owner's machine never executes it, and page membership adds no
operation scope anywhere. Owner machines keep `bridge.add`.

To write into the page, the member's machine joins as one of that member's
certified devices through the ordinary device chain and holds the member's role;
it needs no owner statement. Before dispatch its bridge also checks that the
asking page device and its own page device resolve to the same member at its
latest verified log head, so a page intent cannot name another principal's
machine. Member removal or role reduction revokes that machine's page device
with the member's other devices.

Asks and replies are recorded in the page's own-namespace streams under the page
epoch key, attributed from the signed stream: the ask to the member, the reply
to "<agent> on <member>'s machine". Content fields never name the author.
Everyone who can see the page sees them, like comments; there is no private
ask. The asker sees the destination machine's online state, and offline asks
wait and expire as above.

`devices revoke` revokes machine-local grants immediately. Member removal on
that machine also revokes corresponding grants in the same local transaction.
Removal elsewhere takes effect locally only after the bridge verifies the log;
UI MUST disclose that delay. Page membership and agent grants are separate.

Bridge replies belong only to operations in its ledger and its own signed stream,
correlated by operation ID. It reads core only for request IDs it created, observes
`changes.cursor` and retrieves finals with `requests.show`. Core remains the
final/retention authority; the encrypted page copy respects core's final-size
bound. A page-supplied request ID MUST NOT enable arbitrary result reads.
Cloud bridges connect outward as bridge members; offline sends wait and expire,
showing “waiting for <machine>”. The user's own computers share a Firestore
space; there is no separate relay.

## Renderer and live anchors

HTML runs in an opaque-origin iframe behind trusted prepended strict CSP.
Scripts run on every page: the frame uses `sandbox="allow-scripts"`, without
same-origin, top navigation, popups, forms or modals. There is no static mode.
The renderer MUST deny app storage,
keys, cookies/session, bridge access, network APIs, other pages and top navigation.
It MUST NOT claim complete exfiltration prevention: #830 observed iframe
self-navigation leakage despite CSP. Tear down any frame navigating after its
initial render. The tested CSP begins `default-src 'none'`, permits inline page
scripts/styles and data images, and denies connect-src, form-action, base-uri,
object-src and frame-src. Production CSP/selection message schemas and byte caps must be
frozen and attacked in L3; a spike CSP is not a general sanitizer audit.

There is no script policy and no automatic static mode. Who may view or edit a
page is the creator's choice through its sharing mode, and the creator owns that
risk. The share dialog MUST state plainly that anyone who can edit the page can
change what its scripts do for every viewer, together with the self-navigation
limit above. Renderer isolation is unconditional. Editors use a trusted parent
source editor bound to content; page scripts cannot edit content or change
sharing. Dashboard record
storage is deferred.

On a content change, debounce about 300 ms and replace the frame with a fresh
renderId/port, tearing down the old ones. A viewer may pause live updates. Bind
each renderId to SHA256 of the exact captured source UTF-8 bytes; a Yjs state
vector is only sync metadata and MUST NOT identify the rendered bytes alone.
Interactive JavaScript state is lost on each replacement. Paused preview and
Send retain the captured source/quote/final bytes.

The initial window handshake carries renderId and transfers a MessagePort;
accept its reply only with `event.source === frame.contentWindow` and matching
renderId. Subsequent traffic uses only that bound port; port events do not have
the window-source predicate. Close it on rerender/teardown and discard stale
messages. Allow only bounded selection/anchor-result inbound and highlight
outbound. No secrets, signing/send capabilities or bridge actions cross it.
Interactive page scripts can intercept the port and forge a schema-valid quote;
port possession does not prove selection truth. Frame data is untrusted text,
never HTML in parent UI. A selection cannot silently
change the preview or sign anything.

Anchors use canonical text from an inert parse of the exact captured source:
a space before/after p, div, section, article, h1–h6, li, ul, ol, tr, td, th,
table, pre, blockquote and br; collapse Unicode whitespace runs to one ASCII
space and trim outer spaces. Offsets are Unicode code points, excluding head,
script/style/template/noscript and elements with hidden (including descendants).
CSS visibility and script-mutated innerText are not authoritative; CSS-only
hidden text remains in the canonical source and MUST be exposed in the quote
preview. parse5 7.3.0 source locations map canonical code points to source UTF-16
offsets. L3 must freeze a shared Rust/browser extraction corpus covering entities
(including omitted semicolons), astral text, repaired HTML, cross-tag selections
and namespaces; the bounded #830 fixture alone does not prove full parser parity.
Trusted code maps canonical offsets to source offsets and stores start/end Yjs
relative positions on content html, quote and ±32 code points of context.
Every render resolves positions to source and back to canonical text and checks
against the live DOM before highlight. Deleted text or quote/mapping mismatch
detaches the thread; preserve the quote and require explicit reattach. Fuzzy
suggestions never apply without confirmation. Fuzzy search is limited to a
4,096-code-point window, 64 candidates and 20 ms per thread; exceeding any bound
detaches instead of performing an unbounded search. Epoch resets use the baseline
remapping rule above. No anchor is valid across a stale renderId.

## Sync and backend admission

**Channel boundary: split.** Edge admission, bindings and transport frames move to the remote relay; page, epoch, role and writer checks stay as colab's admission hook.

**Colab-v1 deliberately replaces #478 signed-edge admission** with Auth/Rules
or server-session admission of ciphertext, plus client verification and
machine-bridge signature/grant admission before effects. It claims no inherited
Remote transport authority. Before-effect verification is not edge verification.

Bindings transport immutable encrypted objects, signed statements, wraps and
bounded scoped cursors. Read/subscribe require page admission; append additionally
requires current writer/epoch, scoped stream and create-only sequence. Ack is a
scoped sync cursor only, never core X acknowledgment, task completion, deletion
permission or Send. Reconnect re-verifies the highest retained log head and
current epoch before accepting data/writes. Index fields or a successful socket
upgrade cannot establish authorship. Removed access terminates live subscriptions.
`colab-sync-v1` uses strict typed JSON control frames with version `1`, type,
space, page and epoch. The backend moves bytes, not Yjs state vectors. The wire
operations are:

| Type        | Additional fields / behavior                                                                                        |
| ----------- | ------------------------------------------------------------------------------------------------------------------- |
| `hello`     | `device, cursors`; authenticated session/proof from upgrade, page/epoch admission before catch-up                   |
| `catchup`   | `membershipHead, baseline, streams, more`; bounded pages of each stream's namespace checkpoints and subsequent tail |
| `subscribe` | `cursors`; observe only the admitted page/current epoch                                                             |
| `append`    | `streamId, seq, envelopeHash, envelope`; create-only, exact frozen retry returns original receipt                   |
| `receipt`   | `streamId, seq, envelopeHash`; durable acceptance, not task completion                                              |
| `broadcast` | `streamId, seq, envelopeHash, envelope`; subscriber must verify before applying                                     |
| `ack`       | `cursors`; scoped delivery positions only                                                                           |
| `awareness` | `device, data`; bounded ephemeral presence, never persisted or authority                                            |
| `error`     | `code`; one of DENIED, EXPIRED, STALE_EPOCH, INVALID, GAP, CAPACITY, CONFLICT, RESYNC_REQUIRED                      |

### Implemented stream subset (#1156)

The externally driven local sync module implements the strict subset below.
The complete catchup/chunk protocol remains proposed under #1166; registration
is #1162. Remote owns upgrade admission and supplies the authenticated principal.
The module takes an already-upgraded nonblocking duplex stream, not HTTP headers.

Every client message is one UTF-8 JSON object with exactly the common fields
`version:1, type, space, page, epoch` and the operation fields listed below.
Epoch is a positive canonical decimal string. Duplicate/unknown fields, nulls,
wrong types, noncanonical values and unsupported operations reject. There are no
optional fields in these implemented variants.

| Client type | Exact additional fields                 | Implemented behavior                                                                              |
| ----------- | --------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `hello`     | `device, cursors`                       | Check principal/page and cursor syntax, then `RESYNC_REQUIRED` until #1166.                       |
| `subscribe` | `cursors`                               | Empty list starts live delivery; nonempty list returns `RESYNC_REQUIRED`.                         |
| `append`    | `streamId, seq, envelopeHash, envelope` | Inline update only; verify and durably append before receipt.                                     |
| `ack`       | `cursors`                               | Empty list is a no-op; nonempty list returns `RESYNC_REQUIRED`.                                   |
| `awareness` | `device, data`                          | Device must match principal; data is at most 4 KiB of canonical base64url bytes, never persisted. |

`cursors` has at most 256 strict cursor objects, unique by stream/namespace.
Sequence zero is allowed only as an empty cursor sentinel with zero32 hash.
Inline `envelope` is canonical base64url of the exact frozen model envelope JSON;
retries preserve these bytes. `append.seq` is positive canonical decimal text,
`streamId` equals the authenticated device and signed author, and `envelopeHash`
is the canonical base64url 32-byte model envelope hash. Signed space/page/epoch,
sequence and kind must match; this slice accepts only `update`. Header namespace,
role and membership revision are admitted through current caller-owned policy,
then the model's strict signature verifier runs. The server never opens ciphertext.

Server `receipt` has `streamId, seq, envelopeHash`; `broadcast` additionally has
`envelope`. Both use the same common fields. New appends broadcast to admitted
subscribers, including the sender if subscribed; exact retries return the original
receipt fields without a second broadcast. Server awareness has `device, data`.
Scoped operation errors have `code` from the table above. Malformed, oversized,
binary or unsupported client frames close with code 1008 and reason `INVALID`.
Chunks, object references and inbound server-only frames are unsupported here.
An append whose resulting broadcast exceeds the frame cap returns `CAPACITY`
before persistence. Capacity never evicts accepted receipts or payloads.

Both WebSocket frame and assembled-message payload caps are 64 KiB. The outbound
queue holds at most eight frames including one buffered write; overflow clears
pending delivery and closes with reason `RESYNC_REQUIRED`. A blocked write has a
one-second deadline driven by the caller. A transport that cannot send the close
without flushing blocked ciphertext is dropped instead. Clients must resync on
any abnormal close. Authority is rechecked on every operation, delivery and
caller-applied authority change. Previously written bytes cannot be recalled.
An empty-cursor live subscription makes no claim about missed historical data.

### Planned catchup and chunk protocol

A cursor is `{streamId, namespace, seq, envelopeHash}`, scoped by the frame's
space/page/epoch. The namespace checkpoint and retained update-chain hashes
resolve it; unknown or pruned cursors require checkpoint/tail resync rather than
assuming a trusted head. A hello cannot select a different writer identity than
its authenticated session. Control/awareness are unsigned hints, never log
authority. Chunk transfer uses `{type:"chunk", version:1, space, page, epoch,
objectId, envelopeHash, index, count, bytes}` with consecutive zero-based index,
positive bounded count and immutable transfer identity. Reassembly must respect
object/operation caps, deadline and queue budget, then verify the exact envelope;
partial data cannot apply. Large envelopes in append/catchup/broadcast reference
`objectId, envelopeHash` until bounded chunk assembly completes. No stream
sequence is accepted twice merely because payloads were pruned.

The control-frame grammar above is a C0 proposal derived from the #830 wire
recommendation, not tested canonical production framing. L1/L2 must freeze its
strict nested decoders, independent bytes and operation-specific errors.

The #830 fixture used 64 KiB frames/messages, queue 8, receipt/tail capacity 64,
16 sockets, ten-second connection lifetime, two-second handshake reads and
one-second writes. L2 must pin product caps separately, with positive large-
update/chunk controls, strict framing, gap errors and slow-subscriber closure;
these fixture numbers are not production capacity promises. An overflowing
subscriber is explicitly closed with RESYNC_REQUIRED and must catch up; accepted
durable payloads/receipts survive. Firestore listeners implement the same scoped
immutable-object/cursor semantics without pretending to be a WebSocket server.

Local storage uses extension SQLite/files and `colab-sync-v1` WebSocket; the
HTTP door uses bounded std-thread sockets, workspace tungstenite and strict
framing. Every HTTP request/upgrade requires exact configured Host, never a
wildcard/forwarded-host fallback. Loopback admits `127.0.0.1:<port>` or
`localhost:<port>` as configured; API and upgrades also require exact app Origin,
no CORS. Upgrades need an enrolled-device session or link-device proof; a public
reader can use only an explicitly read-only public session. Public does not
make write/agent upgrades unauthenticated. DNS rebinding, hostile/missing Origin
and unauthenticated upgrades MUST reject before effects.

The local space is loopback-only: there is no `--bind`, LAN or other
non-loopback mode. Other people's machines reach a page only through a cloud
backend (Firestore, then Cloudflare). L2 verifies Host allowlisting,
body/acquisition caps and timeout/shutdown behavior.

Firestore uses Hosting, Anonymous Auth for link holders/bridge connector and
named Google sign-in for named members, Spark by default. Rules admit uid,
member projection and immutable link-device enrollment using the join-proof
hash; enforce create-only scoped sequence, current epoch and expiry. No public
mode is allowed in cloud v1. Blobs/checkpoints are chunked Firestore documents,
without Cloud Storage or Functions. A Function may be introduced only for a
specifically justified check Rules cannot express. F1 MUST prove concurrent
budget counters in emulators; per-uid limits remain best effort and anonymous
UIDs permit Sybil abuse. Owners can disable links/remove devices; App Check is
optional and not a general authority guarantee.

Cloudflare uses a static-asset Worker and one hibernating WebSocket PageRoom DO
per page with the same sync wire, SQLite rows at most 2 MB, retention alarms and
optional R2. Join proof or Access admits ciphertext; client log verification
still decides authority. Real account/deployment actions require separate owner
authorization. Tests use demo Firestore emulators and local workerd/Miniflare
only, no cloud accounts, billing or deployments.

## Retention and management

**Channel boundary: split.** Page expiry policy and warnings stay; backend enforcement uses remote-provisioned resources that colab declares.

Cloud expiry is 30 days after last page update by default, with a per-page
positive day count or forever override. Each write sets expiry; checkpoints and
referenced blobs needed for the live page MUST last at least as long as the page.
The owning device's compaction or owner's cleanup refreshes them within seven
days of expiry. Readers treat expired-but-present data as gone. Warnings begin
seven days ahead in the browser and `ls/show`. Local data is never automatically
deleted.

Firestore Rules deny expired reads; owner browser/CLI cleanup removes expired
pages. Optional Blaze TTL is eventual physical cleanup, not timely revocation.
Spark compaction and expiry share the daily per-page delete budget; exhaustion
backs off and keeps data longer, never loses live data. DO alarms delete page
storage and R2 objects; R2 lifecycle cleans orphan staging only. Archive hides
and freezes; delete ceases access and removes ciphertext. Neither promises
secure erasure or recalls offline copies.

The planned CLI surface is `serve`, `spaces`, `ls [--archived]`, `show [--json]`,
`create <file|->`, `cat`, `edit (--file|--patch)`, `snapshot`, `restore`, `comment`,
`share mode/link/members/remove`, `retention`, `archive`, `delete`, `pair`,
`devices [revoke]`, `approve`, `refuse`, `retry`, `abandon`. These are proposed
commands, not installed usage guidance. Edit computes minimal text diffs as Yjs
operations so concurrent browser/CLI edits merge. Comment never dispatches.
Space home/CLI management expose sharing, threads, anchors, conversations,
members, snapshots, activity/expiry and held/uncertain sends. Enrollment, member
management and local-agent grants remain distinct controls.

## Conformance and acceptance gates

C0 needs squad-lead, Remote security and core-lead review before implementation;
baseline review and #830 hostile-corpus/containment evidence cannot be silently
treated as complete implementation acceptance.
L1 MUST supply strict typed decoders, chain/payload schemas and independently
frozen vectors in Rust, browser and a third implementation, reusing #693's oracle
approach. A test-time Python requirement must be justified in L1. Spikes are
read-only input, not production modules.

Required L1 gates include:

- Sign-in HMAC/possession and owner management bytes,
  typed payloads and expected-revision fencing; L2 proves expiry, replay/second-use
  denial, wrong-device signatures, atomic enrollment, token-hash persistence and
  device-revocation denial. Owner-only management rejects editor substitution,
  stale/offline requests and unverified baseline signing.
- Namespace field bytes and single-field negatives; relabeling, mixed roots,
  content-as-own checkpoints, demoted-editor checkpoints and cut-namespace
  substitution/mismatch. The namespace addition changes #829 cut bytes as well
  as object headers; both require new vectors.
- Purpose-separated link X25519 derivation, signed link encryption key and
  surviving-link rewrap after rotation, plus baseline and snapshot descriptor
  vectors and no-old-key bootstrap negatives.
- A documented limitation vector proves retained-seed access and fresh device
  certification while a link survives individual device revocation. Exclusion
  vectors prove denial after link reset/removal plus rotation, with wraps only
  to intended remaining recipients. Retained-link-seed vectors cover both
  link-to-private and public-to-private when links are present: no private wraps
  for link principals/devices, no reactivation of old links.
- All #829 domains, wraps, transcript/recovery, cut envelope hashes and strict
  malformed encoding mutations; ciphertext interoperability both ways.
- The complete 148-vector Ed25519 corpus in Chromium, Firefox and WebKit, including
  nine accepted positives, mixed-order positives and raw-verifier bypass controls.
  The harness MUST fail nonzero for missing/skipped engines, unavailable/error
  results, wrong row counts or missing controls; deliberately test that failure.
- Independent review of production browser HPKE/admission glue, immutable async
  byte snapshots, non-extractable recipient-key API, object ceilings and fork
  arbitration. No deterministic fixture seeds/intermediate secrets in product APIs.

L2 proves real temporary SQLite transactions, rollback/fork persistence, current
epoch admission, Host/Origin/rebinding/upgrade denial and bounded door cleanup.
L3/L4 prove two browsers and CLI concurrently edit/annotate, persist/reopen,
namespace/role isolation, decoder hostile-corpus containment, dependency/delete-set
compaction, concurrent tails, revoked checkpoint replacement, baseline resets,
`shared` history joins, `current` baseline joins and snapshot restore after compaction/demotion/public transition.
L3 also proves retained-seed access while a link survives (the documented
limitation), denial after Reset link plus rotation, and atomic link-to-private /
public-to-private transitions with links present: removed link-device admission,
terminated subscriptions and no private-epoch decrypt through retained seeds.
Renderer attacks need external request capture and positive controls for resource
loads, nested frames, forms/popups, self/top navigation, refresh, document
replacement, stale frame/port/source, forged selections and live anchor mapping.

L5 proves atomic pairing/receipt recovery, revoked/expired held grants, cross-page/
machine substitution, operation conflicts, restart uncertainty, confirmed-child
retry and no duplicate wake. Use an injected core port plus a real built TMT,
isolated home/private tmux and deterministic agent; observe durable core reply in
the page. L6 proves management, expiry, archive/delete, loopback public disclosure
and public-to-private subscriptions. Acceptance browser suites run
twice with child/socket/state leak checks. Cloud acceptance is later: demo
Firestore emulators with two isolated TMT homes, then local workerd/Miniflare
alarms/R2; injected clocks cover TTL that emulators do not implement.

Each slice has its own issue and reviewable PR below 1,500 changed lines;
dependents wait for merge. Workspace/lockfile/component changes require the two
lead rule. Architecture guard and runtime CI-scope registration land with first
code; private documentation ownership in the component map creates no release.
Local implementation/developer command guidance lands in L2/L3. Official
packaging and cloud deployment remain separate decisions; no gate authorizes them.
