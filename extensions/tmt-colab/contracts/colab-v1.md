# Colab protocol v1

**Status: proposed, not implemented.** Owner: tmt-colab-lead. This is the
normative contract for the local-build-only colab pilot, not authorization to
add dependencies, register a product, deploy a backend or execute agent work.
The terms MUST, MUST NOT and SHOULD express implementation requirements.

This document owns colab wire values, cryptography, membership, page state,
sync, renderer admission and bridge policy. [Architecture](../../../ARCHITECTURE.md#colab-extension)
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

## Planned local attachment storage

The [local attachment storage proposal](storage-v1-proposal.md) owns the coordinated
#1691/#1849 design for generic Remote storage and Colab references, read/history
admission, safe consumers and limits. Its operations and native commands are
proposed, not shipped by this contract. Existing membership, crypto, sync and
renderer owners remain authoritative; no new core object API or reader credential
is introduced.

The [attachment byte grammar](attachment-v1.md) defines #1853's descriptor,
manifest, bounded metadata/comment projection and runtime reference/history admission.
The mount-owned adapter consumes the neutral object channel; Remote declares Colab
Local. Its routed lifecycle gate uses the three shipped binaries.

### Internal attachment channel consumer

After sync hello, an `object` frame carries the current sync scope, a generated
`requestId` and one strict request: `config`, `begin`, `status`, `part`, `commit`,
`discard`, `verify`, `read`, `history`, `historynext` or `historycancel`. Responses
are correlated `object-result` frames in that same scope, containing only `ok` or
`error`. Eight outstanding requests are allowed per peer; queue failure is a bare
unavailable response and never a retry. Missing channels do not activate storage.
An immutable absolute deadline includes queueing, callbacks, I/O and final delivery.
No late data is disclosed; a possibly effected mutation answers unknown. A sent
callback timeout ends its generation rather than leaving outstanding authority.
A request timeout or unexpected frame also ends that shared generation for every
peer: correlations are never abandoned. Pending browser calls settle as storage
unavailable; recovery is the next validated page upgrade after Remote's cool-down,
not replay or polling. A request send serializes peers while the bounded frame is
written (at most 32,768 raw chunk bytes, a 65,536-byte frame and the contract's 1 s
write bound clipped to the original deadline); lock scheduling is not preempted.

Colab derives namespace, opaque key and minimal policy from the exact authenticated
target/reference; browser metadata cannot select Remote authority. Begin/status
retain the original transfer ID, descriptor and base. Parts are at most 32,768 raw
bytes. `verify` is an OwnerSession-only bounded prepublication read of that frozen
target; commit receipts alone do not authorize publication. Complete raw reads
verify length/digest, then existing Colab asset crypto and current reference fences.
Historical paging owns one read-only cursor per peer, reuses the sync catchup codec
and current entitlement wraps, and never renews its first deadline. Cancel, expiry,
peer close and generation replacement release its source and join owned workers.
Root-local library reads use the actual management keyring and require an established
channel; no browser-reachable plaintext route, upload command or second backend is introduced.
The only plaintext route is the owner-only `attachment-read` below, which answers the local
CLI of this data root and no other caller.

### Attachment access across page lifecycle (#1856)

A read fails with one of four reasons, mapped the same way from the native peer to the browser:
`denied` (the reader's access ended or never existed: sharing narrowed, member, link or
device removed, page deleted), `not-found` (the page, as this reader may see it, holds no
such reference; only `read` and `verify` answer it, mutations answer `unknown`),
`changed` (the disclosure moved while the read ran, so trying again may work) and
`unavailable` (everything else, including storage, the channel and a key epoch the
reader does not hold).

| Event                          | Existing attachment reads                                                       |
| ------------------------------ | ------------------------------------------------------------------------------- |
| Sharing, link or device change | Denied for whoever lost access; open previews and downloads end with the scope. |
| Archive                        | Native reads and catch-up still work; writes are denied. See below.             |
| Retention expiry               | A warning only; local reads and writes are unaffected and nothing is deleted.   |
| Delete                         | Denied; the page's ciphertext is removed. See below.                            |
| Incomplete upload              | Expires as staging; its original transfer ID is never re-executed.              |

Warning-only page expiry never deletes completed objects, and Colab runs no garbage
collection of Remote objects. Deleting a page keeps its encrypted objects charged to
the space until Remote's deletion lands (follow-up #2294). The browser does not open
an archived page in local v1 (follow-up #2298), so an already-open session ends in the
existing `changed` or `unavailable` state.

### Attaching files in Chat and annotation conversations (#1854)

The owner browser's shared composer can attach files to a Chat, annotation or reply
message. Picking, pasting or dropping only adds an inert local chip: nothing is stored,
sent or prepared, an Ask is never created and the message text is neither read nor
replaced. The explicit Send is the first effect. A message still needs text; attachments
are never part of an Ask payload, so a mentioned agent receives the text alone.

Limits are the consumer limits above: 8 MiB per file, 16 per message, the backend
`config` payload bound when lower, and filenames shortened to 255 UTF-8 bytes with
control characters and path separators replaced. The stored media type is decided from
the bytes, never the name or the browser's declared type: a PNG, JPEG or WebP whose header
declares at most 16 million pixels is stored under its raster type, everything else
(video bytes included) as `application/octet-stream`.

On Send the browser allocates the message ID once, then for each chip seals the asset
under that ID (message revision 1), freezes it with the message fence (the membership head, epoch and author; see [attachment-v1](attachment-v1.md)) as its base,
and runs `begin`, ordered `part`s and `commit`; progress is per chip and a failed Send
leaves the text and the other chips as they are. `refused` outcomes (including a
backend limit) never took effect and are retried only by an explicit action. A thrown
request or an `unknown` answer once the first request has left makes the chip
`unconfirmed`: Send is blocked until the user asks status of that same frozen original
(a pending transfer continues, never replays) or removes it. A status answer of
`expired`, `discarded` or `not-observed` attaches it again from the local bytes;
`unavailable` says nothing about the original and leaves it unconfirmed. Removing, or leaving the
composer unsent, releases stored originals best effort; the bytes are not kept in saved
drafts.

Only when every chip is committed does the Writer batch the verified publication records
and the message record (with its descriptors) in one own publication, inside the
discussion lock and after `verify` of each committed object. Publication requires the
fence to equal the base captured at seal time, so another device's or agent's write while
files upload never stops the Send. If the membership head or epoch changed meanwhile,
nothing is published, the affected chips show that the page changed and the next Send
uploads them again from the retained local bytes under the same message ID, releasing
the first original. Objects left unreferenced by a failed or abandoned Send stay in the
backend; Colab runs no garbage collection, so reclaiming them belongs to Remote (#2294). A message that has attachments cannot be
edited; deleting it removes its references.

Each attachment row opens bytes only on a trusted click, through the admitted read of
that exact message revision. A permitted raster previews from a `data:` URL after the
header is parsed again; every other file is an explicit `application/octet-stream`
download through the parent's blob-download lifecycle, revoked after hand-off and on
unmount. No inline video, active content, relaxed CSP or renderer capability exists.

### Page files in the Files panel (#1855)

The Files panel is a peer of Comments, Chat and Source, opened from a header action
(`Files`, with the count when non-empty). A writer always has it; a read-only link shows it
only when the page has files. Rows are the shared list rows with Preview, Download and, for
writers, Remove; opening bytes follows the same trusted-click rule as message attachments,
but through the admitted `document-current` read of the document's current content revision.

Choosing files again only adds inert local chips. "Add to page" uploads each chip as a
`content` asset bound to the digest of the source it is being written against, then
publishes the `intents` proof records first and only then writes the references through the
existing Save path as a typed `{set}` change (see Browser Save). A changed page never
attaches: a stale base or source digest makes the chips upload again from the retained local
bytes on the next "Add to page". Removing a file saves a typed `{remove}` change; its stored
bytes stay unreferenced and unreclaimed (#2294). A page holds at most 128 files and one
batch at most 16. The `tmt colab` command line has no attach entry yet.

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

The root-local engine exposes `OwnerRequest` / `OwnerAction` through
`Engine::apply`. An admitted management caller supplies an optional exact
transport digest and `RequestScope {initiating_page, affected_pages}`. Scope IDs
are sorted/unique and the initiating page belongs to the affected set. Scoped
member removal/role changes and link removal/Reset match the target's stored page
assignment during planning and the in-transaction recheck. Page actions bind a
singleton page set. An absent scope is reserved for root-local composition.
The replay digest purpose-separates normalized action, scope and transport bytes;
an exact replay returns the original outcome and signed head, and conflicting
bytes/scope return `CONFLICT`. A fresh scope mismatch returns `STALE_HEAD`.
With neither transport nor scope, existing root-local digests remain unchanged.
The runner implements member/link/device/epoch and page-policy actions. Public
publication requires trusted loopback composition; request bytes cannot assert
the backend's identity.
This API is not transport admission. The caller serializes through sync first,
then Registration; no request-carried field grants signing authority.

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
cuts MUST cover every affected stream in the reduced epoch; unseen offline updates
beyond the cut are rejected and reported to their writer. For a retained older epoch,
a later reduction may omit a cut only when an earlier verified owner-signed
`epoch.advance` sealed that exact page/epoch/namespace/stream. Its seal cut remains
the binding sequence, checkpoint and tail-hash bound. An unsealed, uncut stream
still refuses; current caller entitlement and disclosure fences remain unchanged.
`page.share` carries no cuts: a rotating share uses the separate `epoch.advance`.

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

### Implemented owner-browser registration (#1162)

Remote owns sign-in and pairing. Colab accepts `POST /api/devices/register` on
its owner-only mount socket, beneath remote's `/r/<prefix>/x/colab/` mount. The
request is strict JSON with exactly `deviceId`, `sign`, and `enc`. Each certificate
has exactly `publicKey` (canonical base64url key32), `issuedAtMs` (safe integer UTC
milliseconds), and `signature` (canonical base64url signature64). Both signatures
use the remote-owned `tmt-ext-cert-v1` input, with extension `colab` and their
respective `sign` or `enc` purpose. There is no added version or device-ID field
in those signed bytes. The request's device ID MUST match the authenticated
forwarded context; the strict verifier uses that context's `publicKey`.

Registration requires the full forwarded owner context with `owner:true`, a
canonical device ID/key and positive grant revision. Cookie-only and non-owner
requests cannot register. Colab trusts this header only on the owned 0600 socket
inside its owned 0700 directory; remote strips client-supplied context and
rechecks its live session/grant. Both key certificates MUST be no more than ten
minutes old and MUST NOT be future-dated. Syntax failure returns HTTP 400
`INVALID`; absent/non-owner context, device mismatch, signature failure or
revocation returns 403 `DENIED`; stale/future certificate returns 403 `EXPIRED`;
changed keys or remote identity binding returns 409 `CONFLICT`; state/keyring
failure returns 503 `UNAVAILABLE`. Success returns JSON `{chain, issuerStatement}`,
where the issuer statement is the exact revision-1 model envelope as JSON.
The device certificate's `membershipRevision` is `"1"`, matching that issuer
anchor; it is not the current owner-log head. Current membership, revocation and
page policy still govern admission and wraps.

If the owner log is absent, the existing owner transaction creates its initial
editor management member with no page assignments. Its fixed operation ID is
`00000000-0000-4000-8000-000000000001`; it is reserved for genesis. Existing
incompatible owner-member bindings fail closed. Local management key derivation
uses HKDF-SHA256 with owner seed as input, empty salt, and
`LP(label, space)` as info, with 32-byte output. The exact labels are
`tmt-colab-management-signing-seed-v1`,
`tmt-colab-management-encryption-seed-v1`, and
`tmt-colab-management-member-id-v1`. Ed25519/X25519 public derivation follows the
fixed suite; member ID uses the first 16 bytes of its independent output, setting
UUIDv4 version/variant bits. These private outputs never leave Keyring. The
independent Python vectors are `vectors/management-key-v1.json`.

A subsequent owner-store writer transaction checks the pinned management member,
revocation and existing binding before signing/persisting the colab certificate,
remote context and exact response. Device registration does not advance the
membership log. Colab certificates last 365 days; the same certified key binding
returns its exact saved response until fewer than 30 days remain, when fresh
remote certificates silently renew it. Every retry still requires fresh input
certificates and current owner context. Registration and local revocation are
serialized; a context older than the highest observed grant revision is denied.
A saved, verified certificate naming a later membership revision is renewed on
authenticated retry with the same keys and revision-1 issuer anchor, regardless
of its remaining lifetime. Renewal atomically replaces the saved chain and
response without advancing membership.

The trusted `Registration::revoke(deviceId, grantRevision) -> Result<bool>` callback
uses the owner engine for a known device: tombstone, cleared registration,
revoked device projection, owner-signed `device.revoke` cuts and affected-page
baseline/epoch/wrap transitions commit in one owner transaction. An unknown ID
uses the local tombstone transaction alone. Equal/older events and already-revoked
devices return false with no write or signature, independently of operation ID;
no later context revives a tombstone. Registered-device admission rechecks durable
revocation and certificate expiry.
The exact reserved socket `POST /.tmt/remote/device-events` consumes remote's
[local device events](../../../contracts/remote-channel-v1.md#extension-channel-api)
only with `tmt-device-event: 1` and a strict body. A revoke commits this callback
and shuts down every live tunnel of that device under the sync server lock before
HTTP success, including tunnels without hello. No-op events repeat neither writes
nor tunnel effects. Rename is validated and acknowledged without local
presentation state. Other reserved paths reject; remote refuses the `/.tmt`
subtree beneath browser mounts. There is no browser revocation route.

### Implemented mounted browser assets (#1253)

The foreground executable selects optional `serve --app-dir <absolute directory>`
first, then its embedded build, then the compile-time crate directory plus
`../../typescript/app/dist`, canonicalized at startup. An invalid explicit
selection or embedded inventory MUST fail with `COLAB_APP_UNAVAILABLE` before
Colab state is created. A missing, unsafe or incomplete checkout default MUST
still start the service with the owner build-hint placeholder.

When supplied, build-time `TMT_COLAB_APP_DIR` MUST name an absolute complete Vite
output directory. The build script requires `index.html`, `renderer.html`, `reader.html` and
`THIRD-PARTY-NOTICES.txt`, and validates them with flat `assets/` files using
the same names, media types, entry references and 128-file/16-MiB bounds as runtime
admission. Directories and files MUST be real, and files nonempty and regular.
Invalid supplied input MUST fail compilation. The generated embedded table uses
snapshots in Cargo's output directory; source mutation after generation cannot
change those bytes. Absent input generates no embedded assets and preserves local
checkout fallback. Startup passes embedded bytes through the same inventory
validation. Moving a binary with embedded assets requires no app directory, checkout, Node
or pnpm at runtime. There is no sibling app directory or data-root copy. Native release
activation and its archive/install proofs remain owned by infra's #1418.

Disk loading MUST admit only nonempty regular files through no-follow directory-
anchored opens. Both disk and embedded inventories MUST have at most 128 files
and 16 MiB total. The inventory requires
`index.html`, `renderer.html` and `reader.html`, and admits optional `THIRD-PARTY-NOTICES.txt`, and flat generated `assets/` files;
unknown output, symlinks and missing HTML entry references reject the inventory.
JavaScript and CSS are required. Supported asset suffixes are `html`, `js`, `css`,
`woff2`, `woff`, `ttf`, `otf`, `png`, `jpg`, `jpeg`, `svg`, `webp`, `ico` and `txt`.
Fonts use their `font/<suffix>` media type; JS/CSS/HTML/notices use UTF-8 text
media types. Assets MUST be exact startup bytes, including after files change or
are removed; adopting a rebuilt disk app requires restarting serve; adopting a rebuilt embedded app requires rebuilding the binary.

After existing API/event/upgrade dispatch, owner-context GET `/` and `/index.html`
MUST return the built HTML; GET of an inventory key MUST return its bytes and
content type. Asset access MUST NOT require Colab registration, since the app
performs that registration. Anonymous root GET retains private-space guidance.
Without owner context, exactly these static files are served (GET only, the same
policy headers as owner responses): `/read` (the bytes of `reader.html`),
`/renderer.html`, `/assets/reader.js`, `/assets/reader.css`, `/assets/reader-fold.js` and
`/assets/recovery.js`, plus the native `/assets/chrome.css` stylesheet, which is also available
without an app build. They are public static bytes with no secret and no API. Every
other asset request without owner context, including `/index.html`, `/reader.html`,
`/THIRD-PARTY-NOTICES.txt` and the hashed owner assets, returns 403. `/read` has no trailing
slash so the entry's relative `./assets/` and `./renderer.html` references resolve under the mount. Unknown paths and owner
non-GET static requests return 404. Invalid context is rejected by the existing
HTTP admission. Dot path segments, backslashes, doubled leading slashes, percent
encodings, queries and fragments MUST reject with 400. No request path is
normalized, joined to a filesystem directory or given a SPA fallback. Existing
API routes and registered-owner `/sync` admission/transport are unchanged.

App and reader chrome consume `@tmt/browser-ui/react`, `/static` and `/static.css`.
Native guidance embeds the same checked `design/browser-ui/generated/static.css` at
compile time, with Colab-owned host metrics and viewport styles. Cargo and installed
serving MUST NOT run Node or generate CSS. The same-origin `/assets/chrome.css` response
uses `text/css; charset=utf-8`, including when the optional app build is absent. Shared
presentation owns no routing, admission, page state or action/recovery capability.
Guidance MUST render its recovery status/script only when the admitted app inventory
actually contains `/assets/recovery.js`; without it, pairing/app-unavailable guidance stays visible.

Vite output MUST use relative URLs beneath `/r/<prefix>/x/colab/`, with no
third-party requests. The current app declares installed/system font fallbacks;
no external font service is used. App and native guidance responses share this
exact parent CSP, including guidance served without an app build:

```text
default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'
```

The exact owner-only `/renderer.html` route uses its own response policy instead
of the app policy, whether served from the embedded table or `--app-dir`:

```text
default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; font-src 'none'; media-src 'none'; worker-src 'none'; manifest-src 'none'; sandbox allow-scripts
```

The response `sandbox allow-scripts` MUST also make a directly opened renderer
opaque; it MUST NOT depend only on the embedding iframe attribute. A missing
renderer makes the app inventory incomplete, following the default/explicit
startup behavior above. Renderer bytes remain build-owned, not author content.
App/static responses retain `Cache-Control: no-store`, `Referrer-Policy:
no-referrer` and `X-Content-Type-Options: nosniff`. Non-app responses retain the
existing restrictive placeholder/API policy.

### Mounted read-only reader sessions (#1310)

The local socket admits explicitly read-only link and anonymous public sessions.
They use the [Remote route-mounting contract](../../../contracts/remote-channel-v1.md#extension-channel-api)
without Remote enrollment. Remote's Route mounting section owns subprotocol
forwarding, reserved-header stripping and logging policy; Colab owns these
capabilities and their before-delivery admission. These routes never confer owner
identity or agent access. The [reader link](#read-only-reader-link-1545) opens them in a browser.

`POST /api/readers/challenge` accepts strict JSON, exactly
`{kind:"public",space,page}` or `{kind:"link",space,page,chain}`. The chain is
canonical base64url of the model's exact link-device chain JSON (at most 16 KiB).
The server resolves the current local page/epoch, public or live assigned-link
policy, pinned `link.add` keys and statement, certificate lifetime and device
revocation. A private or deleted page denies readers; archive retains native reads. The browser does not open
an archived page in local v1 (follow-up #2298).
The response is exactly `{challengeId,nonce,space,page,epoch,chainDigest,expiresAt}`:
UUIDv4 challenge ID, nonce32, canonical scope, digest32, and safe-integer UTC
milliseconds. Public uses zero32 chainDigest. Challenge lifetime is 60 seconds.

`POST /api/readers/session` accepts exactly `{kind:"public",challengeId}` or
`{kind:"link",challengeId,signature}`. The link device signs exact LP
(`tmt-colab-reader-session-v1`, `1`, challengeId, nonce32, space, page, epoch,
chainDigest32, decimal expiresAt). Signature is canonical base64url signature64;
no bearer seed reaches the server. A proof attempt consumes its challenge;
expired/replayed challenges cannot issue capabilities. Recheck policy before
issuance. The existing device transaction persists a verified link-device
projection without replacing a different binding or tombstone, signing any
certificate/log statement, or giving the device a write path. Public issues no
certificate or device projection.

Success returns exactly `{principal,token,space,page,epoch,ownerKey,expiresAt}`.
The principal is a fresh server UUID distinct from certified writer identities;
token32 is a random single-upgrade capability, with only its hash retained.
Lifetime is ten minutes. At most 64 combined challenge/ticket/active entries
exist; expired unused entries are reclaimed without evicting active readers.
Malformed requests return 400 `INVALID`; failed authority/proof/replay returns
403 `DENIED`; expired challenges/tickets/sessions return 403 `EXPIRED`;
exhaustion returns 503 `CAPACITY`; state faults return 503 `UNAVAILABLE`.
Errors are JSON `{code}`. HTTP body and header bounds remain unchanged.
Known availability limitation: unauthenticated challenge requests can occupy all
64 entries for 60 seconds and deny new readers with `CAPACITY`. The mounted door's
abuse budgets bound exposure; this seam adds no per-client challenge quota.

Offer `colab-sync-v1` and `colab-reader-v1.<token>` to `/sync`. The server compares
fixed-size token-hash confirmations through the existing constant-time HMAC
verifier, consumes the ticket once, and selects only `colab-sync-v1`; it MUST
NOT echo the reader subprotocol. Tokens MUST NOT enter URLs, logs, renderer
messages, cookies or owner device context. Disconnection releases the capability;
reconnect and restart require a fresh challenge/exchange. An owner context does
not upgrade a supplied reader ticket into owner authority.

Only hello, catchup, subscribe and scoped ack are allowed, on the admitted
page/epoch. Deny content and own appends, referenced-upload acquisition/chunks
and awareness publication even for an editor link. Link catchup selects only
link-addressed wraps; public catchup selects none and obtains published keys
from signed statements. Owner catchup retains its existing exact bytes.
All operations, deliveries and pre-hello tunnels recheck session expiry, live
link/device/page policy and epoch under the sync owner. Revocation, narrowing or
rotation discards pending delivery and closes access; a surviving link seed
can still certify a fresh device after individual device revocation. Exclusion
requires link removal/narrowing plus rotation, never merely deleting a ticket.
Previously written bytes, keys and plaintext cannot be recalled. Mounted native
lifecycle tests and deterministic duplex blocked-transfer tests exercise these
fences.

### Read-only reader link (#1545)

`share link add` and `share link reset` print one openable link for the link they create:

```text
<door address>/x/colab/read#v=1&space=<spaceId>&page=<pageId>&link=<linkId>&rev=<n>&st=<hash>&seed=<seed32>
```

The CLI prints the relative form `x/colab/read#...` (field `readerPath`), like `page create`'s
`path`; while a door runs it also prints the complete link as `readerUrl` (door discovery below),
otherwise the owner prepends the Remote door address `tmt remote pair` printed. Everything is in
the fragment, which a browser never sends to a server. Path and query carry nothing, and the
seed appears nowhere else in any output. `rev` and `st` are the revision and canonical
base64url hash of the `link.add` statement that introduced the link: the link-device chain
must name that statement, and a wrong value only fails the server's chain check. The grammar is
strict: `v` is `1`; each of `v`, `space`, `page`, `link`, `rev`, `st`, `seed` appears exactly
once in any order; values are canonical (`space` and IDs as everywhere, `rev` a positive
decimal, `st` and `seed` unpadded base64url of 32 bytes); anything else, including percent
escapes, an unknown key or more than 512 bytes, is not a reader link.

The public `/read` entry removes the fragment from the address bar before any other work
(a reload therefore needs the full link again). In memory only, it derives the link keys from
the seed with the model's `link::Keys` derivation and wipes the seed. Its reader device is
also derived from the seed: Ed25519 signing seed `HKDF(seed, LP("tmt-colab-link-device-seed-v1",
space, link))` and a device ID from the first 16 bytes of `HKDF(seed, LP("tmt-colab-link-device-id-v1",
space, link))` with UUIDv4 version and variant bits set. It certifies that device as a `link`
issuer chain with the link's encryption public key, the `rev`/`st` statement and a fixed validity
window of `issuedAt` 0 to `expiresAt` 9007199254740991 (liveness is the link's, never the
certificate's). Ed25519 signing is deterministic, so every open presents a byte-identical chain
and the server keeps one device row per link however often the link is opened
(`authority-v1.json` `linkDevice` freezes the oracle bytes). The server never enforces this
derivation: any seed holder may still certify a fresh random device, as the revocation rules above
state. The entry then runs the challenge and session exchange above and opens
`/sync` with the ticket subprotocol; hello names the session's `principal`. It checks that the
returned owner key derives the linked `space`, verifies the owner log and link-addressed wraps
with the owner-browser rules, and pins and stores nothing in the owner app's records
(IndexedDB, local or session storage, cookies). It shows the page read-only and live, with no
editor, Ask, share, export or history control. The ten-minute session ends in a reconnect with
a fresh challenge; a `DENIED`/`STALE_EPOCH` result or a 403 from the exchange ends access
("Access ended"), which is what Reset, remove, narrowing or rotation produce.

Reader access is limited to whoever can reach the loopback or Remote door and holds the link.
The CLI help and the entry say so plainly.

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

| Namespace | Roots                                                                                                               | Fold and authority                                                                   |
| --------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `content` | `html` Y.Text; `meta` Y.Map containing `title` and optional `publisherAgent`, `originalAuthor`, `creationRecipient` | Shared page document, folded from admitted owner/editor streams in the current epoch |
| `own`     | `threads`, `messages`, `intents`, `replies` Y.Maps                                                                  | Separate document per writer; only that writer's signed stream mutates it            |

`meta.publisherAgent`, when present, is a nonempty string of at most 128 UTF-8
bytes without control characters. It is a publisher-asserted display-only label,
never an identity binding, agent ID or publication authority. CLI page create/write
captures it best-effort through the fixed public `identity show --json` command via
`tmt_invoke` and the invoking TMT executable (one-second deadline, 64 KiB output
cap). Unknown/unbound callers, invalid output and invocation failures produce no
label; an unknown CLI writer removes the previous label. Browser Save carries no
CLI label and so removes it the same way. Content folding and epoch baselines preserve it in the
committed update bytes; no descriptor field or extra signature is added.

`meta.originalAuthor`, when present, uses the same bounded label grammar. Initial page creation derives it from that request's already captured `publisherAgent`; there is no separate creator lookup or public creation selection. It is a publisher-asserted creation display snapshot, not authenticated proof of a person's identity. Supported native and browser source writes preserve it independently of the latest publisher; an unknown creation remains absent after later known writes. Legacy absence means Unknown author in trusted presentation, never a guessed or backfilled value. Fresh epoch baselines and content checkpoints preserve it. It never supplies authorization, an Ask recipient or an Ask default.

`meta.creationRecipient`, when present, is exactly `{machineId,agentId}`: a
canonical non-nil Remote UUIDv4 and canonical non-nil core UUID. It is an asserted
creation-time routing preference, not proof of authorship or authority. CLI creation
freezes one bounded public caller identity observation with one optional same-root
`tmt remote status --machine --json` observation before the create intent.
Unsupported, stopped, missing, malformed or unavailable observations leave the
complete pair absent and creation succeeds. Creation reuses this observation for
link presentation: unavailable or unsupported projection prints the committed path
with neutral unavailable-link wording and no next step, pairing read or automatic
opening. Only a validated running descriptor permits full links and the existing
separate presentation-only devices read; a validated stopped result may retain the
Colab-serve hint. Other commands retain their ordinary status discovery. Later supported source writes preserve
the pair; historical absence never backfills. Neither latest publisher nor display
labels can supply it. The future #1817 consumer may default only after an exact
authenticated current machine and one uniquely admitted agent UUID match this frozen
preference; this native/non-UI change does not integrate that UI default. Live grant
and send admission remain separate. Old strict readers reject this added metadata
rather than silently dropping it; mixed-version runtime acceptance is not claimed.

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
changing that thread's ownership or messages. Local v1 allows currently admitted owner devices to comment and reply, but only
the originating device may edit/delete its messages or resolve, reopen, reattach
and delete its threads. Cross-writer editor resolution remains planned; it
requires authenticated action provenance and a defined conflict order.

### Implemented raw own fold (#1264)

The owner-browser reader decrypts admitted own updates and checkpoints and folds
them into one document per authenticated writer in the page's single Worker.
The native isolated decoder and browser validate the exhaustive four map roots,
plain JSON values, absence of materialized list/shared-type mutations and resolved
dependencies. A present message `body` MUST be a string of at most 16 KiB UTF-8.
The 1,000-thread page limit sums map entries across writers, including retained
tombstone values; equal record keys in different writer documents count separately.
Both page folds enforce this limit before returning a view.

Raw values remain inert. Typed Ask records are defined below; typed discussion
records use the following strict immutable JSON grammar. Their authenticated,
device-signed own envelopes bind the writer, so discussion records need no extra
signature. The parent checks sender-to-stream binding, page/epoch scope and a
historical signing key captured from a cut-admitted envelope before display.
Historical keys grant no fresh write authority.

### Own-stream discussion records (#1427)

All discussion record kinds contain `version:1`, `kind`, `spaceId`, `pageId`, `epoch`,
`senderDevice`, `revision`, `deleted:boolean`, `deviceName` and `at`. IDs are canonical
UUIDv4, space IDs are canonical, and epoch/revision are positive decimal strings.
Labels are publisher-asserted plain text, at most 128 UTF-8 bytes, with no authority.
`at` is a canonical nonnegative decimal string of UTC milliseconds since the Unix
epoch, at most `8640000000000000` (the JavaScript Date limit). The publisher sets it
when publishing each revision, including tombstones. It is display-only: it never
determines ordering, revision selection, ownership or admission. The UI shows the
latest revision's relative time beside the author and marks revised live comments
as edited; device IDs remain available in a tooltip.

| Root/key                                       | Additional fields                                                                                                                                                                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `threads[threadId+":"+revision]`               | `kind:"thread"`, `threadId`, `anchor:null` or `{exact,prefix,suffix}`, `resolved:boolean`                                                                                                                                       |
| `messages[messageId+":"+revision]`             | `kind:"comment"`, `messageId`, `thread:{writer,id}`, `body:string`                                                                                                                                                              |
| `messages[actionId+":thread-status"]`          | `kind:"thread-status"`, `actionId`, `thread:{writer,id}`, `previous:null` or `{writer,id}`, `resolved:boolean`, `actor:"person"` or `"agent"`, `agentName:null` or string, `recipients:[{machine,agent,agentName,operationId}]` |
| `messages[operationId+":thread-notification"]` | `kind:"thread-notification"`, `operationId`, `status:{writer,id}`, `reason:"RECIPIENT_UNAVAILABLE"` or `"PREPARATION_FAILED"`                                                                                                   |

No unknown fields are accepted. A value claiming a typed kind rejects if its
root, key or field grammar is invalid in the browser or native decoder. Other
bounded raw values remain inert. Bodies are exact plain text: nonempty and at most
16 KiB UTF-8, or empty for a deleted comment. Deleted threads have null anchors.
Selectors require nonempty exact text at most 16 KiB UTF-8, with prefix and suffix
each at most 32 Unicode code points and 128 UTF-8 bytes.

Thread and comment records start at revision 1, with consecutive immutable revisions.
Messages retain the same thread reference. A gap, changed reference or resurrection
after a terminal tombstone yields no projected logical row. Writer ownership is
part of every reference, so foreign replies never grant edits to the target stream.
Missing verified targets remain inert until admitted. Duplicate immutable keys with
different values refuse publication. The existing thread-entry cap counts all
revisions and tombstones across writers; older text remains retained history.
Deletion is not secure erasure. Deleted threads retain attributed replies and
disable new replies.

Status and notification records are immutable single actions: `revision` is `"1"`
and `deleted` is false. An admitted owner device may resolve or reopen any live
non-Chat thread in the current page/epoch. Thread creation, anchor changes,
deletion and comment edits remain writer-owned. A designated null-anchor Chat
thread cannot acquire status actions. The thread record's legacy `resolved` value
is the initial state, never rewritten by a status action. Historical signing keys
prove record authorship, not status authority. Native status projection separately
requires owner-member provenance from the certificate chain and issuer verified
for every admitted own envelope of that writer at its membership revision;
ambiguous provenance, non-owner members and bridges cannot contribute status
actions; the browser applies the same rule (`Objects.statusWriter`). Their existing
thread, comment and Ask admission is unchanged. A later
revocation does not erase a valid earlier action within its signed committed cut,
but current admission remains required to publish another action.

A status action references the effective previous action, or null for the initial
state. The authenticated fold admits only same-thread ancestry rooted at null;
missing parents, cross-thread parents and cycles remain inert. A descendant has
its parent's depth plus one. The deepest action wins; equal-depth concurrent
branches use ascending ASCII `(writer, actionId)` ordering with the greatest pair
winning. Wall clocks and labels do not affect this order. Native reads, browser
views and exports use this effective status rather than selecting status from a
thread revision. A deleted thread ignores status actions.

`actor` and `agentName` are publisher-asserted display labels. A person action has
null `agentName`; an agent label, when present, is nonempty, at most 128 UTF-8
bytes and contains no control characters. Machine and operation IDs are canonical
UUIDv4; agent IDs use the existing nonzero core UUID grammar. A recipient list contains at most 1,000 distinct machine/agent
pairs with distinct operation IDs and labels of at most 128 UTF-8 bytes. Only a
person Resolve action can freeze recipients; Reopen and agent CLI actions have an
empty recipient list. A notification failure belongs to the status action's
writer and records a failure before adoption into the existing Ask ledger. It
cannot change thread status or grant a dispatch capability.

The generic own writer prepares at most 32 records in one update; thread creation
publishes its thread and opening message together. Preparation cannot change the
committed projection. Only admitted encrypted appends commit; failures preserve
parent drafts. Current device/page/epoch admission is required for every mutation.
Disconnected, read-only and inactive views disable actions. General member-role
UI remains planned.

Native `tmt colab threads <page> [--json]` reads the same authenticated current-epoch
conversation projection as export. Its page operand uses the same owner-catalog
short-prefix resolution as other page commands; thread IDs remain full UUIDs. `threads resolve <page> <thread>` and `threads
reopen <page> <thread>` publish an agent-labelled status action through the local
writer as one native publication job of `kind:"own"` (see Content publication), using its
shared content/own sequence and the same fenced ciphertext commit, offline or through
the running serve's `page-publish` route. Preparation edits only the writer's admitted
own structs in the isolated child. Its outcome handling is the page write's: an
uncertain result reports `COLAB_OUTCOME_UNKNOWN` with the original operation ID after
one read-only `publication_status`, and never falls back to an offline writer or
retries. JSON adds `operationId` when the action was published. Repeating
an already-effective status is a no-op. Missing or ambiguous thread IDs, deleted
threads and designated Chat threads cannot acquire a new status action. The fixed
public identity probe supplies a display-only caller name, or no name when unknown.
Agent CLI Resolve and Reopen have no recipients and never create an Ask or dispatch.

Ask on a comment rechecks its unambiguous verified writer, thread/message IDs and
revisions in the parent. It freezes stored quote/body and the real IDs in the
existing signed Ask input. Later edits or deletion do not alter a prepared excerpt.
Comment actions only publish discussion; explicit Send remains the sole Remote
dispatch action. Standalone asks retain their existing panel and framing.

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

The implemented decoder library producer/verifier for #1159 uses the existing
isolated child. Source admission is 2 MiB, title admission is 256 KiB, and
update-v1 admission is 2 MiB + 256 KiB + 1 KiB framing. The serialized
stream cap (limits table) applies to both modes. Verification sends update bytes,
authenticated source digest and title once; the child reconstructs source and
checks its digest and the commitment. Production checks its generated update
inside the child without transmitting both copies as input. Descriptor signing, encrypted publication, owner folds
and atomic epoch transitions remain caller-owned, later integration work.
[Baseline vectors](vectors/baseline-v1.json) pin update-v1 bytes and commitment
with a test-only fixed client ID; production uses a fresh identity. Their
[independent oracle](vectors/baseline-reference.py) covers only the fresh
`html`/`meta.title` plus optional `meta.publisherAgent`, `meta.originalAuthor` and `meta.creationRecipient` schema and requires no third-party libraries.
The complete two-key preference has two fixed-client lib0 object-key byte orders,
each with its own exact-byte commitment; this does not relax the deterministic
frozen export manifest.

The owner-local epoch engine stores baseline plaintext as strict JSON with exactly
`source` (the exact UTF-8 source string) and `update` (canonical base64url update-v1).
Its `html`/`content` object has sequence zero and the pinned local management member
as author, signed with that member's key; the root-signed epoch descriptor admits
this baseline independently of a device stream. Store retains the exact descriptor
and encrypted envelope together with the new secret, wraps, epoch and replay result.
The engine verifies stored owner statements, certificates and object chains from a
read snapshot, then materializes content and validates per-writer own roots through
the isolated decoder. It produces the new baseline outside the writer lock. The
commit rechecks every captured namespace cut and device projection before pinning
checkpoints; moving snapshots retry at most three times, then fail `STALE_HEAD`.
This owner-local library seam does not enable a browser management endpoint.

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
join either receives every bounded wrap or none. Store admission evaluates
history at the wrap's join revision; a current-history join rejects earlier
epochs, while existing holders keep their previously admitted history. Acceptance includes a page at
the epoch cap and a multi-page join that needs several wrap lists.

Existing retained quote selectors re-resolve at epoch reset and detach on
mismatch; they never imply permission to recreate absent discussion.
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

The owner-local engine commits `link.remove`, affected-page `epoch.advance`
statements and an optional replacement `link.add`, in that order, in one owner
transaction. The replacement ID must never have been used, and its seed must not
reproduce the removed link's pinned keys under the old ID. Seeds are borrowed for
model derivation and never stored in statements, projections or public replay
outcomes. The owner caller retains/distributes a replacement seed only after
success. A current-mode link add wraps the existing current epoch only; Reset
creates the fresh baseline/epoch before joining its optional replacement. Shared
joins and replacements use the same bounded history wrap lists as members.

Private pages admit named members only. Link pages additionally admit
link-certified devices at the link role. Public mode is loopback-only in v1;
Firestore and Cloudflare MUST reject public mode and key publication. Going
public first advances the epoch with a baseline, then publishes the new current
epoch key in an owner-signed statement, with bounded earlier keys under shared
history. Every later public-page rotation publishes its new key in that same
owner transaction. This discloses current live source and
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
revoked, not merely hidden by an index/edge projection. Removing a link that
covers several pages revokes it globally and rotates every other writable page
it covers in the same transition. Trusted UI MUST warn about that scope before
narrowing a page. Re-enabling link sharing
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
Semantic tests that do not measure timeout behavior MUST inject a larger bounded
invocation/readiness budget through the decoder configuration, including callers
that fold content or produce baselines. Production constructors MUST retain the
defaults below. Timeout tests MUST retain the production deadline and use a
controlled blocking child with a readiness signal rather than successful-work
wall-clock headroom. Output-limit tests MUST distinguish output exhaustion from
timeout. Successful reuse controls may use the semantic-test budget but MUST
reuse the same runner without clearing its cleanup fence. The archived hostile
corpus retains its separate per-case/suite budgets and diagnostic requirements.

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
prefix head. Namespace-specific plaintext remains raw merged Yjs update-v1.

Store may prune payloads through shared head `n` only when every namespace with
updates in that prefix has a committed checkpoint at exactly `n`, each binding
the same update hash in its signed previous-hash field. An unpaired checkpoint
is retained without pruning and is not selected for bootstrap. Completing the
pair atomically prunes both covered prefixes and superseded unpinned checkpoints.
Retain the existing update hash/receipt ledger, pinned checkpoints and the full
tail from `n+1` in both namespaces for chain verification and exact retries.
Never delete another namespace's payload solely because its sequence falls in
the prefix. A failed partner publication leaves the prior pair and updates
intact; this rule needs no new ledger or checkpoint schema.

For browser bootstrap, every observed namespace checkpoint for an author stream
MUST agree on one prefix sequence n and signed update head hash(n). Deliver all
checkpoints before the retained tail; that tail MUST be contiguous from n+1
across both namespaces. Receipt
ledgers support retries, not browser authority; no additional ledger proof or
signature scheme is required. Checkpoint plaintext is raw merged update-v1 bytes.
The browser MUST bound each checkpoint and the combined baseline, checkpoints
and retained tail plaintext by the 24 MiB read state budget, with at most 5,000
tail updates. Native writes (the CLI and the browser Save) retain the 200-update/4 MiB write tail budget and 256 KiB per update. Apply
checkpoints as single-item Worker steps before
the tail. Those unpublished steps may retain cross-writer pending dependencies;
the final tail step MUST resolve them and validate complete content before
publishing any view. The existing 2 MiB source projection cap remains in force.
Both namespaces count toward the combined plaintext budget.
The Worker MUST also bound total encoded content plus all own documents, including
pending structs/delete sets, and the serialized combined projection to 24 MiB each.
Pending fragments MUST survive candidate cloning; final catchup validates every
document before publication. Live content/own candidates commit only after all
validation succeeds. Failure terminates the Worker and flags the binding; reconnect
constructs fresh epoch documents. Own plaintext crosses the decoder boundary only
after the same authenticated scope, writer, chain and signed-cut admission as content.
The [raw own fold](#implemented-raw-own-fold-1264) defines its bounded projection.
Historical revoked-device objects MUST predate revocation and remain within the
exact signed namespace cut; checkpoint replacements require the cut's exact
envelope hash, and catchup MUST reach every nonempty signed tail endpoint before
publishing a view. The owner browser admits two kinds of author, exactly as native
does: a stream with an owner-member device chain, and a bridge named by a verified
`bridge.add` at the envelope's membership revision. A bridge's envelopes are admitted
only in the `own` namespace, only for a page its `bridge.add` lists, only when the
membership state at that revision has the page's current epoch, and verify against the
pinned `machineSignKey`; they stop at the bridge's signed `device.revoke` cut like any
device, and carry no owner-device provenance. Named-member and link author policy remains
outside this owner-browser slice (#1111/#1160). `contracts/vectors/bridge-own-v1.json`
freezes the signed membership logs and sealed envelopes that native and browser admission
both replay.

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

| Default limit                                | Value                                         |
| -------------------------------------------- | --------------------------------------------- |
| Exact HTML source / snapshot source          | 2 MiB each                                    |
| Message body                                 | 16 KiB UTF-8                                  |
| Threads per page                             | 1,000                                         |
| Update-envelope plaintext                    | 256 KiB                                       |
| Compaction trigger per stream                | 200 updates or 256 KiB tail                   |
| Per-device append rate                       | 10/s sustained, burst 50                      |
| Per-page decoder concurrency                 | 1                                             |
| Rust decoder batch deadline                  | 2 seconds                                     |
| Updates after checkpoints a write accepts    | 200 updates, 4 MiB                            |
| CLI page write, whole source                 | 2 MiB; within the tail above                  |
| Browser Save, whole source                   | 2 MiB; the same tail and page budget          |
| Browser Save upload window                   | 10 s from the `save` frame, at most 64 chunks |
| Native compaction trigger, per device stream | 50 updates or 1 MiB                           |
| New page source, per published update        | 192 KiB of text                               |
| Page budget (browser load, gzipped)          | 5,000,000 bytes                               |
| Rust decoder new-baseline source             | 2 MiB                                         |
| Rust decoder read state (all bytes)          | 24 MiB, at most 5,000 updates                 |
| Rust decoder input/output streams            | 208 MiB each                                  |
| Linux decoder address-space limit            | 512 MiB                                       |
| Spark deletion budget                        | 500/page/day                                  |

The read caps are derived once, from measurement (#1627), not from the write caps.
Decoding peaked near 9 bytes of child memory per state byte, so 24 MiB of state stays
inside the address-space limit. A stream frame is sized for the worst case, a state of
control characters that JSON escapes to six bytes each, plus base64. Decode time grows
with the size of the largest text block times the number of updates appended to it
(about 0.6 ms per MiB per update), not with raw size alone: a 2 MiB block with 1,000
appends decoded in 1.15 s, an 8 MiB block with 1,000 appends exceeded the 2 second
deadline. Compaction removes that cost: the same 8 MiB block with 1,000 appends, merged
in steps of 50 appends, decodes as one checkpoint in 91 ms, and each merge step takes about
0.27 s (2 MiB: 0.07 s). A page that exceeds a read cap
or the deadline fails alone with `COLAB_CAPACITY`, naming the page, the limit and the next
step; `ls` and `show` list the other pages. Reads accept these caps, but a write refuses
once one more update would take the updates after the devices' checkpoints past the
200-update/4 MiB write budget, or the whole page past the read caps or past 5,000,000 bytes
gzipped as one stream: `page write` fails with `COLAB_CAPACITY` naming the page, the limit and
the way out (`tmt colab export`, then `tmt colab page create --file`), and the page stays
readable. Browser read admission matches the native read caps.

Native compaction follows the compaction rules above. After a write, `tmt-colab` combines the
local device's own stream once it holds 50 updates or 1 MiB after its last checkpoint: the
decoder child merges only that stream's updates with `merge_updates_v1` (a device's stream may
depend on structs another device wrote, so the merge does not build or project the page), the
device seals a checkpoint per namespace at its stream head, and the store prunes the covered
prefix when both namespaces are paired. Other devices' streams are never touched. Combining is
best effort and repeatable. Before publishing either checkpoint, the owner decodes the page with
the candidate merged streams and requires its HTML, complete metadata and every writer's own
projection to equal the current page. A mismatch or decoder failure publishes nothing and leaves
the write durable and every update in place.

Linux sets and verifies its address-space limit before reading child input;
failure rejects the job. On macOS and platforms without enforced memory limits,
run with deadline/output containment and report `memory limit unavailable`: in JSON
page receipts and in `serve --json` (`memoryLimit`). Human output never repeats it.

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

**Channel boundary: split.** Trusted browser Send and the encrypted own-stream
ledger belong to Colab; Remote owns operations, grants, hold approval, dedup and
receipt recovery. Local v1 uses the asking browser directly. There is no native
bridge ledger, native Ask route or additional SQLite migration.
The browser wraps the same verified Session held by registration and Live with
Remote's operations helper. The wrapper opens nothing and never reopens on
uncertainty: the helper resyncs its sequence and reads the original operation ID.
Each mounted tab opens its own Remote session and binds its sync WebSockets through
the SDK's `transportUrl`; tabs on one paired device may remain live concurrently.
After a socket disconnect, a read through the old verified operations helper
distinguishes ordinary session end from limit eviction. Ordinary end or
unrecoverable sequence state signals Registration to reconnect silently; eviction
stops that tab and shows the reported limit, optional Remote settings URL and
the `tmt remote settings sessions-per-device <n>` command. It MUST NOT reopen
automatically after eviction or resend an Ask. Closing a tab affects only its
own bindings.
The SDK returns verified pre-effect eviction from Send and operation reads as
`refused` states with `REMOTE_SESSION_EVICTED`, a positive `limit` and optional
`settingsUrl`. The adapter retains that eviction for the lifetime of the old
Session, including across later generic ENDED reads or opaque socket close.
Registration rebuilds both the RemoteClient and page AskControllers with the new
shared Session; old-session attempts cannot dispatch, and recovery reads the original IDs.

Only explicit Send in trusted parent chrome dispatches agent work. Comments,
sync, replay, compaction, reload and renderer messages MUST NOT dispatch. Ask
agent uses the same send path for annotation turns and page Chat. On explicit Enter,
the parent freezes the admitted quote/comment, page title, canonical mounted HTTP(S)
page URL and chosen agent/machine before signing.
Credentialed URLs and malformed Unicode refuse. Paused rendering or later edits
cannot replace frozen text. The shared plaintext message input sends without a confirmation screen
or an on-demand delivered-message view. The own stream retains frozen bytes and
destination UUIDs. The pane states that asks and replies are visible to everyone with page access.

Remote prepends its contract-defined `[remote: <device name>]` line and LF. The
recipient receives that line before the message; the parent signs and sends
only the frozen message below it. The verified device name and grant revision
and session expiry are pinned to the frozen intent; rename, revision change or session
end refuses dispatch.
No second prefix or post-freeze formatter is permitted. Delivery readiness is always unavailable in local v1; presence never becomes
channel readiness. The adapter drops the SDK's unknown delivery projection, and
Colab carries no delivery-readiness field.
Grant mode and expiry are shown only when supplied by verified Remote evidence;
missing policy remains unavailable, with no inferred hold warning.

The canonical send input is LP(`tmt-colab-send-v1`, version, space, page, thread,
messageIds, machine, agent, operationId, finalBytesDigest, senderDevice,
grantRevision, grantExpiresAt, issuedAt, expiresAt). `grantRevision` and the
verified session expiry reference the actual Remote grant; no fabricated grant
or client ID is used. `grantExpiresAt` is canonical millisecond text or `none`
when the verified session has no expiry. The sender is the certified asking device.
The digest binds exact unprefixed transport bytes. Sorted unique message IDs use
the existing list framing. The non-extractable extension signing key signs these
bytes. Default validity is one hour, maximum 24 hours. The mounted controller
caps composed delivered messages at 64 KiB, below core's request bound, and
checks expiry again after durable own publication and immediately before Send.
A mutable Yjs field is never execution authority.

Before any Remote call, a per-device/operation Web Lock reserves immutable signed
metadata in existing Colab IndexedDB and publishes the exact signed input,
signature and final bytes in the asking device's encrypted own stream. Plaintext
final bytes are not duplicated in the local metadata record. Same ID and signed
input returns the existing operation without sending; changed input/signature
is `INTENT_CONFLICT`. Interrupted reservation cannot authorize a second effect.
The existing Writer owns both namespaces, shared sequence/signing, durable exact
ciphertext staging and append retry. No second stream owner or keyring is added.

The browser ledger publishes `dispatching` before one signed Remote
`dispatch.create`, with envelope ID equal to the frozen operation UUID, anonymous
originator, one agent UUID and exact unprefixed message. Remote owns dispatch
admission and core owns acceptance/advisory wake/final retention. Accepted is not
an agent final. Valid states are:

```text
dispatching -> accepted | held | uncertain | failed | refused | cancelled | expired
held -> accepted | uncertain | refused | cancelled
uncertain -> accepted | held | refused | cancelled | abandoned
```

Remote governs hold approval and rechecks its own grant at release. Local v1
does not supply a Colab page-policy hook at later approval. Browser observation
never dispatches or approves. Timeout, interrupted output or reopened
`dispatching` becomes uncertain. Recovery uses only `operation.show` with the
original ID; absence remains uncertain and permits no same-ID retry or replacement
operation. Explicit abandon records `MAY_HAVE_BEEN_DELIVERED`; it neither
proves absence nor cancels recipient work.

### Own-stream Ask records

Ask records are inert JSON values in the existing per-writer own Yjs roots:

| Root/key                             | Value                                                                                                      |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `intents[operationId]`               | `{version:1,kind:"ask",signed:{operationId,senderDevice,input,signature,finalBytes},agentName,deviceName}` |
| `messages[operationId+":"+revision]` | `{version:1,kind:"ask-state",operationId,revision,state,requestId,reason}`                                 |
| `replies[operationId]`               | `{version:1,kind:"ask-reply",operationId,requestId,agentId,body}`                                          |

`agentName` and `deviceName` are display-only, publisher-asserted labels, each
bounded to 128 UTF-8 bytes. The asking publisher takes them from the verified
frozen intent (agents.list and owner session echo). They are outside the signed input
and never establish authority, route work, or replace the agent/device UUID.
Readers may show a UUID fallback when a label is empty.

Binary fields use canonical base64url. State revisions are positive canonical
decimal strings, ordered numerically. `requestId` and `reason` are explicitly
null when absent. Reasons are bounded sanitized codes, never raw transport
errors. Verified pre-effect Remote refusals preserve `REMOTE_SCOPE_DENIED`,
`REMOTE_INPUT_INVALID`, `REMOTE_RATE_LIMITED`, `REMOTE_INTENT_CONFLICT`,
`REMOTE_CLOSED`, `REMOTE_SESSION_ENDED`, `REMOTE_SESSION_EVICTED`, `REMOTE_INPUT_TOO_LARGE`,
`REMOTE_STATE_UNAVAILABLE` or `REMOTE_CORE_UNAVAILABLE`; unknown refusal codes become
`REMOTE_REFUSED`. A verified pre-admission Send refusal, including session end,
is definitive: it records refused with the reviewed code. A session-end Send
refusal then signals Registration to reconnect and requires new explicit input capture.
An eviction Send refusal instead stops the page with the verified limit/settings
notice; it remains refused, not uncertain, and cannot reopen automatically.
Adopted Sends never return refused; unknown outcomes remain uncertain. A typed
SDK `sequence_unavailable` Send outcome becomes uncertain
(`REMOTE_SEQUENCE_UNAVAILABLE`) and signals Registration to reconnect. The adapter never reopens. A session-ending result
read leaves the existing accepted record unchanged and stops observation until
reconnect. Every refused operation/result read leaves the ledger unchanged;
an eviction operation read stops observation with the same typed notice and
never changes the prior operation outcome or dispatches again.
missing operations do not prove absence. Transient state/core-unavailable refusals
continue bounded backoff. Read refusal copy is ephemeral, never an own-state
transition. Fresh input capture is required for each later Send. Unknown-effect errors
remain uncertain. SDK error handling branches only on the exported class and
reviewed code, never text or an unverified error-shaped object. Records are immutable under
the parent-owned publication API.

Viewers admit the encrypted own envelope and its writer chain before interpreting
records. They verify the intent's signature and message digest, space/page,
operation ID and sender-to-stream binding. State/reply records join only to an
intent in that same authenticated stream. State revisions respect the state
machine and preserve an established request ID. Reply agent UUID and request ID
must match the intent and accepted receipt correlation. Content author claims
cannot choose another writer or agent. The native isolated own decoder validates
Ask syntax/digests and bounds; it holds no keys or dispatch capability. General
annotations join verified Ask records to their existing thread and current comment
IDs, displaying agent replies inside the thread. There is no second discussion store.

Only the asking device observes its unresolved operations and publishes finals,
through Remote's device-owned result operation and request IDs already in its
admitted ledger. Empty finals are valid. The page copy is exact UTF-8, bounded
to 16 KiB; a larger retained core final yields `REPLY_TOO_LARGE`, with no silently
truncated copy. Unavailable core history yields `RESULT_UNAVAILABLE`. Core remains
final/retention authority; page copies follow page access and retention.

On page open and visible-again, read-only observation makes one sequential
catch-up pass over at most the 256 newest unresolved asks from the asking device,
regardless of age. Older asks outside that pass keep explicit re-check. Failed
older reads are not automatically retried. Continued polling after that pass and
after Send/re-check uses 2-second to 30-second backoff while visible, pauses hidden
pages and stops after a two-hour operation horizon. Annotations and Chat then show
a Lucide Clock with `no reply yet`, keeping the accepted ledger state and read-only re-check action.
Closing the asking browser delays page publication until it
reopens; it never cancels work. Synchronization and other viewers cannot perform
result reads as the asking Remote device.

The browser `RemoteClient` adapter consumes Remote's served operations SDK and
its verified session/responses. It does not compose raw envelopes, read Remote
storage or invoke core directly. Uncertainty recovers on the existing session;
only session end requires Live to replace it before original-ID recovery. Frozen byte/signature vectors live in
`vectors/send-preview-v1.json`, with independent Python regeneration.

The frozen `Link:` for a new mounted Ask or Chat turn is the page's short owner URL,
`<origin>/p/<shortId>`, derived from its admitted full page ID and current discovery catalog.
Its signed scope retains the full space/page/thread/message IDs; the prefix grants no authority.
An older captured Ask without a short prefix retains its canonical full owner fragment URL.
The source URL is still admitted against the exact full page fragment before shortening. A source URL whose
fragment is anything else (a reader seed, any other key), or that has a query or a `/read` path, is refused
rather than stripped, so a reader link can never reach agent text. After an accepted, held or uncertain
Send the admitted turn and inline outcome remain in their conversation; the input can compose another explicit turn.

### Member machines

Local v1 is owner-only: mounted writers are certified devices of the pinned
revision-1 owner member on this machine. The asking browser sends under its own
paired Remote grant to that same machine; page membership creates no agent
permission. Read-only page viewers receive the admitted Ask records without
Remote result credentials. Reply attribution combines the admitted asking device
and its signed intent's agent/machine UUID, never an author name in reply content.
The page copy is published by that asking device, not a native machine stream.

Cross-member machines and Colab before-effect page/member fencing at held/offline
adoption belong to the cloud stage. They require the asking page device and the
destination's certified page device to resolve to the same member at the latest
locally verified owner head, with current role/page/epoch and Remote grant policy.
Owner-machine fallback is forbidden. Page authorization and Remote agent grants
remain separate. Local v1 does not claim that cross-member or delayed-approval
page-policy fence, offline automatic dispatch or native reply publication.

### Admitted agent status (#1844)

The trusted parent Agents view observes presence through the existing current
Remote context and signed `agents.list` normalization owner. Active, offline and
unknown are directory values, never inferred from a transport failure. Duplicate
names retain machine and stable agent-ID presentation; labels confer no authority.
Colab page admission, Remote session reads and directory reads are separate health
observations. Known session-end categories and verified eviction/refusal codes remain distinct from unexpected
unavailable reads; raw diagnostic messages and credentials are not displayed.

Opening or explicitly rechecking this view is read-only: no Ask destination-cache
admission, preparation, publication, dispatch, session reopen, pairing, grant change
or automatic retry. Existing Ask actions retain their separate fail-closed lifecycle
response. Current page/client generation fences late results. A transient directory
failure can retain a last successful snapshot clearly marked stale with its check
time; ended/evicted/scope-denied, lost admission or a replaced page/client clears the
cached observation and pending read. Restoring the same admission requires a new read. An empty
successful directory is separate from an unavailable read. The view does not change
conversation drafts or admit a message recipient.

## Renderer and live anchors

HTML runs in an opaque-origin iframe whose `src` is the same-mount
`renderer.html` document, behind the separate response CSP defined above.
It MUST NOT use `srcdoc` or inherit inline permissions from trusted chrome.
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

After the bootstrap document loads, the parent sends exactly one source-init
message containing `type:"colab.render.bind"`, `renderId`, `sourceDigest`,
`source` and `theme` (`"light"` or `"dark"`), plus one MessagePort. The renderer accepts only these exact fields,
valid UUIDv4/digest metadata and source at most 2 MiB UTF-8, from
`event.source === window.parent`; origin is not an admission predicate for this
opaque channel. Direct top-level opening cannot initialize source. Invalid or
non-parent messages do not initialize it; after one valid init all later init
messages are ignored. No source enters a URL or request body. The bootstrap
replaces its own document using `document.open/write/close`, retaining the
response policy and prepending a trusted acknowledgement before author source.
Bootstrap load and initial source-document load are the only permitted loads;
further document loads/navigation tear down the frame.

The parent derives the effective theme from its explicit root `data-theme` choice,
otherwise the current OS preference. Only user theme activation records a choice;
without one, OS changes remain live. Before author code runs, the bootstrap projects
the effective theme onto the renderer root as `data-theme="light"` or `"dark"`.
After binding, the parent sends the current value again to cover initialization races,
and sends subsequent changes over the existing port as exactly
`{type:"colab.render.theme", renderId, theme}`. The renderer accepts only the current
render ID and the two theme values. Theme changes update the same document without
source replacement or loss of interactive state. Renderer release/teardown removes
the parent theme subscription; author messages cannot choose the parent theme.

Author CSS must use the explicit root `data-theme` selectors to follow Colab's choice;
the bundled skill's starter includes them and matching `color-scheme` declarations.
CSS keyed only on `prefers-color-scheme` continues to follow the OS. This cosmetic
projection does not rewrite source, inject palettes, force author backgrounds or
change the existing white canvas for unstyled pages. Author code may tamper with it;
it grants no capability or truth about the page.

For a fixed look, author CSS uses an unconditional palette and `color-scheme`,
without theme-attribute or OS-media palette overrides. The bootstrap overwrites an
author-pinned `<html data-theme>` with Colab's effective theme; the attribute is
not an opt-out. Explicit author CSS colours/backgrounds still determine the page.

The initial window handshake carries renderId and transfers a MessagePort;
accept its reply only with `event.source === frame.contentWindow` and matching
renderId. Highlight requests/results use that bound port; selections use the
bound window channel. Port events do not have the window-source predicate. Close it on rerender/teardown and discard stale
messages. Allow only bounded selection/rectangle/anchor-result inbound,
known-thread view actions and narrow highlight/scroll/theme outbound. No secrets, signing/send capabilities or bridge actions cross it.
Interactive page scripts can intercept the port and forge a schema-valid quote;
port possession does not prove selection truth. Frame data is untrusted text,
never HTML in parent UI. A selection cannot silently
change the preview or sign anything.

Anchors use the W3C Web Annotation TextQuoteSelector fields
[`exact`, `prefix`, `suffix`](https://www.w3.org/TR/annotation-model/#text-quote-selector).
The trusted bootstrap installs selection capture and cosmetic resolution before
author HTML. Both walk rendered text in document order, exclude head,
script/style/template/noscript and hidden descendants, collapse Unicode whitespace
to one ASCII space and trim outer spaces. Entity decoding and repaired markup are
the browser DOM's behavior; offsets are Unicode code points with UTF-16 range maps.
There is no source parser or Yjs relative-position binding. CSS visibility and
script-mutated DOM may change cosmetic feedback; neither selection nor successful
highlight proves source truth or authority.

Bound window selection messages to the current frame and renderId. Selection adds
`selector:null` or the bounded quote/context fields to `type`, `renderId`, `text`,
plus an optional `rect:{x,y,width,height}` in frame viewport coordinates. Its four
numbers must be finite, at most 1,000,000 in absolute value, with nonnegative
width/height. The parent clamps this cosmetic rectangle to the frame and visible
window when positioning its Annotate control and input popover; it never treats geometry as authority.
The bootstrap hides the affordance while a pointer drag or keyboard selection is in
progress and captures after release (coalescing other selection changes at the next
animation frame). Its rectangle follows the selection's focus end, including backward
selections; layout updates refresh only completed selections. Legacy text-only messages
supply no anchor. Unmodified, non-repeating C and the existing Alt+Enter request the same
view outside input/textarea/select/contenteditable/textbox targets, with IME and keyCode
229 guards, using exactly `type:"colab.render.annotate",renderId`; it cannot send. A trusted
parent action captures the quote. Page-wide comments have null anchors.

The parent highlight request on the bound MessagePort contains exactly
`type:"colab.render.highlight"`, `renderId`, `requestId` and `anchors`, whose entries
are exactly `{id,selector}`. Comment bodies, reply text and discussion display labels
MUST NOT enter the author-code frame; the parent reconstructs these narrow objects
instead of forwarding caller records. Results contain `type:"colab.render.anchors"`, `renderId`,
`requestId`, `resolved` IDs and optional `positions:[{id,top}]`; top is finite,
nonnegative and at most 1,000,000. Position IDs must be unique resolved IDs. The parent admits only unique IDs from its
current request; stale frame/request results cannot change current feedback.
Batches are at most 1,000 anchors and 256 KiB UTF-8. Collection is bounded to 20,000
text nodes, 256 Ki code points and 20 ms; matching considers at most 64 candidates
per anchor with a 20 ms batch deadline. Over-budget, missing, changed or ambiguous
matches detach. A unique exact quote must match every supplied prefix/suffix.
There is no fuzzy automatic attachment. DOM mutations trigger bounded debounced
resolution; replacement/navigation/disposal closes ports, observers and highlights.

CSS Highlights, or pointer-inert range overlays when unavailable, leave author text
nodes intact. Resolved ranges keep a light highlight and a small square right-margin
marker in the frame's document flow, with a count for anchors on the same line. Hover
shows the bounded quoted text. Comment first-line tooltips stay in parent chrome. A marker posts exactly `type:"colab.render.open-thread"`,
`renderId`, `requestId`, `id` on the bound port; the parent accepts only a known ID in
its current highlight request and opens that thread's anchored parent window. Forging this view-only
action cannot publish, sign or send. Resize and DOM changes re-resolve positions;
thread-list navigation scrolls the window to the reported anchor offset (the bounded
viewport-coupled fallback scrolls inside its frame). Author scripts can tamper with these APIs, DOM or cosmetic results;
the parent sends bounded quote/tooltip data, never full records, keys or application capability through the channel.
Detached threads retain their quote and require a fresh selection plus explicit
parent confirmation to reattach. Re-resolve against every fresh render and epoch;
absent own history is never recreated automatically.

### Inline annotation conversations (#1587)

The Annotate control beside a selection opens one plain trusted-parent input in a
small anchored window at that span. On the first committed turn, the same input
continues below the thread's user turns, agent state and admitted replies; Comments
and Chat do not open automatically. The header and composer stay stationary; only
messages scroll. The window fits its header, quote, turns and composer without
reserved history height,
growing away from its selection edge up to the available viewport height. Only
history scrolls beyond that cap; the field and Send stay visible independently of
document bounds. The shared conversation history opens at the bottom and follows
new record identities while the reader is within 24px of the bottom. Above that
threshold, arrivals preserve the reading position and show a "New messages" text
action at the history's bottom edge. Activating it or manually returning to the
bottom clears it. This device's own new turns always follow latest. A delayed
admitted reply is an arrival; edits to existing replies, cloned publications and
size changes are not arrivals. Size changes retain following, and reaching the
bottom after resizing also clears pending state, including when every message fits.
Its placement is cosmetic; the captured quote
selector owns the thread anchor. With the mention list closed, Enter sends,
Shift+Enter inserts a newline, and Esc closes the input (an unsent draft is kept).
While the list is open, its first available option is highlighted; Up/Down wrap,
Home/End jump, and Enter, Shift+Enter or Tab inserts the highlighted agent without sending.
Esc closes only the list and keeps the text. Typing filters the options; no match
closes the list. Clicking an option is equivalent to Enter. IME composition does
not navigate, choose or send. There
is no confirmation screen or automatic send. The popover closes with its ×, with Escape from anywhere
inside it, with a press outside it, and with a selection cleared by a page click while
no nonblank message is typed; none of these interrupts a send in flight. Typed text is kept
for the page (see Draft retention) together with bound mention UUIDs and restored, with a "Draft kept"
note, when the same selection is annotated again or its known thread is reopened.
After a thread exists, an outside page press collapses it even with a typed draft;
Close never resolves or sends. Existing writer-owned Resolve collapses only after
a successful explicit action; failure retains the window and draft. A live source revision preserves the mounted composer, its unsent text and frozen
quote/rectangle, even when the quote no longer exists in the updated page. Explicit
Send uses that captured quote; cosmetic resolution may then show the thread as
detached. After cosmetic resolution finds the quote missing, both the composer
and open thread show: "This text changed on the page; your note keeps the original quote."
Renderer loading leaves discussion inputs, focus, caret and agent list usable;
sending uses the frozen quote and remains subject to current connection admission.
Open threads, Comments/Chat panels and their drafts also survive source
revisions. The window scroll offset is retained on a best-effort basis within the
new document's bounds; no exact re-anchoring is required. A different page ID resets
this page-local state. Chat, thread replies and new annotations share one Message
field (accessible name only), one status row and one Send. With no bound or exact
mention, Send records a plain comment and prepares no Ask, including in Chat.
Exact typed `@name` binds on whitespace, punctuation or Enter only when one current
directory entry matches; picked listbox options bind the same machine/agent UUID
pair. Unknown names remain plaintext with "No agent named @name. This posts as a
comment." Ambiguous names open the list and block Send until explicitly chosen or
removed. Fuzzy matches never select a destination. Removing/replacing a token removes
its binding; repeated mentions of the same pair count once. A fresh untouched
composer seeds the admitted `creationRecipient` regardless of presence. A unique current UUID
match supplies its name; an absent/ambiguous entry or failed discovery uses the neutral
`@Creator` label with that same UUID pair, never a publisher name. Initialization waits
for the first directory result; typing first suppresses the default.
Unknown provenance leaves it blank; publisher names and prior repliers never select
recipients. A removed default is never reinserted during directory replacement.
Bound mentions are atomic, square, flat chips: a solid online dot, muted offline
state, or a dashed not-found border. Unknown/unavailable presence is labeled explicitly.
Same-name agents include their machine name. Hover/focus exposes the full name,
machine and state; the trailing remove action drops the binding without sending.
Missing/ambiguous UUID entries block Send until replaced/removed. Renames retain
the bound UUID pair and original message text. Chip decoration never enters the plaintext.

One Send freezes exact text and context, records one comment through the existing
own stream, then prepares and sends one independent Ask per distinct mentioned UUID
pair, at most eight. A ninth distinct recipient blocks before the comment or any Ask.
Each Ask has a new operation ID and its own signed record, outcome and reply; a
recipient's failure neither retries nor suppresses its siblings. The verified Remote
grant and current machine/agent UUIDs retain routing admission. An offline saved
agent on an online machine may receive through the core inbox; a disconnected
session or unavailable/replaced directory fences mention Sends while keeping the
same editor, draft and caret. Directory recovery uses only explicit Reconnect or
Try again. A plain comment remains available under content-write admission while
directory discovery loads or fails.

Each Ask freezes and signs the exact composed text with the real thread and message
IDs. A trusted "Ask again" action can prepare one new operation for one unsent
recipient on the original comment, without another comment, upload or sibling Send.
It refreshes the directory and matches the original machine/agent UUID pair; names
never reroute it. Local preparation rejection or explicit `adopted: false` retains
the original capture only in that composer's memory, cleared by a new Send or close;
it does not survive reload. Missing adoption evidence or a thrown Send is uncertain.
A signed refusal is eligible only in the asking device's own stream, with state
`refused`, no reply, null request ID and a reviewed `REMOTE_REFUSAL_CODES` reason.
Every own Ask on the same thread/message/recipient must remain provably unsent;
accepted, held, dispatching, uncertain, failed, expired, cancelled, abandoned or
unknown-refusal records block another operation. Re-check/Abandon remain the existing
recovery choices for uncertainty, never permission to Ask again.
Preparation uses `retryOf` absent for ordinary Send, null for a trusted local failure,
or the old operation UUID for a signed refusal. The fresh signed intent keeps the
same thread, sole message ID and machine/agent UUIDs, with a new operation ID. A
signed-refusal action captures the current original comment and only preceding
conversation; a local action re-admits its earlier captured references/revisions.
A pair-scoped Web Lock encloses the block-check, a durable marker write and adoption.
The existing IndexedDB record store holds only the latest replacement operation UUID
under the space/page/own-device/thread/message/machine/agent key. A marker whose own
admitted record is absent or not a typed pre-effect refusal blocks even a stale tab.
Read/write errors fail closed. Release is allowed only on authoritative `adopted: false`;
a later admitted typed refusal permits the next explicit replacement. There is no
history or plaintext in the marker. Existing per-operation locks and immutable record
rules still apply. The shared action fences trusted events, double activation, blocked/unmounted/replaced bindings and stale
context. All old outcomes remain unchanged; the new outcome is its own row and
there is at most one action per recipient group. Reload may restore a signed-refusal
action only if the entire own group still satisfies the predicate; it never sends.
Prior user comments and verified agent replies are included as quoted data, in the existing
projection order, with their captured references/revisions rechecked before signing.
Display timestamps never determine record ordering or authority. Deleted/stale
context and byte overflow refuse; no context is silently truncated. A failure after
comment publication retains that comment and shows the outcome; it never automatically resends.
The input and conversation show the composed text; no surface offers an on-demand
view of the transport message. Frozen bytes, including the quote and captured
context, remain in the existing Ask record. Held approval stays inline; approval
itself happens on the machine through Remote.
The existing ledger, expiry, recheck, abandon and uncertainty rules still apply.
A transient observation read failure shows the existing warning in the anchored
window. Subsequent permitted observation continues through the same bounded
observer; a successful cycle clears the warning without another dispatch.

The Comments overlay lists every annotation and page-level thread, with a quote
snippet, participants, display time and open/resolved status. Selecting a row scrolls
the window to its anchor and expands that thread inside the overlay; a margin marker
reopens the same thread in its anchored window. Page-owned per-thread drafts also
survive switching to the explicit Comments details view. Agent replies join verified Ask records to comment IDs and
remain attributed to their agent. Overlay geometry does not reflow page content.

#### Draft retention (#1924)

An unsent, deliberately edited, nonblank message (annotation, thread reply or Chat) is
also saved on the device so a reload or hot-module reload restores it. The saved form is
one record per space, device and page, AES-GCM encrypted with a device-local key that is
separate from the title-hint key and bound to that scope as authenticated data; it holds
the exact plaintext, the intact mention bindings and the draft's target (a thread
reference, the frozen quote selector, or Chat), at most 32 drafts per page with the oldest
dropped first and each message within the comment limit. Restoring only fills the
composer. It never opens a window, prepares, sends, signs, revokes or reopens a
Session, and nothing in it is authority: recipients are revalidated against the current
directory as for an in-tab draft. A draft returns only to the exact space, device, page
and target; a record that fails decryption, scope, version or shape checks is ignored
and replaced by the next write. A selection draft is listed under Comments as Saved drafts
with its quote and text until that quote is selected again or the draft is discarded; a
reply whose thread no longer exists is listed the same way. Nothing is deleted,
re-anchored or sent automatically. The send ledger keeps the original operation for any
adopted send, so a reload shows ledger state and never becomes a fresh send. When the
device cannot store drafts (unavailable, blocked, full), the draft stays in the tab and a
single line says `Drafts are not saved on this device.`; success is claimed only after
the storage transaction completed. Source-editor text is outside this record.

### Page Chat (#1645)

The header Chat action replaces the standalone Ask action. Chat opens a fixed right
parent overlay (a full-screen mobile sheet) without resizing or reflowing the page.
Messages scroll inside it; one shared plaintext message input remains at the bottom.
Chat uses the same shared history owner and scroll policy as inline annotation
conversations: it opens at the bottom and follows new record identities within
24px of the bottom. Above that threshold, arrivals preserve the reading position
and show "New messages" at the history's bottom edge; activating the action or
manually returning to the bottom clears it. This device's own new turns always
follow latest. A delayed admitted reply is an arrival; edits to existing replies,
cloned publications and size changes are not arrivals. Size changes retain
following, and reaching the bottom after resizing clears pending state, including
when every message fits. Closing retains the draft and admitted history. Enter
explicitly captures, freezes, signs and sends;
there is no confirmation screen or automatic send. Composed text remains in the
conversation; frozen transport bytes stay in the Ask record. The pane states that the conversation is visible to
everyone with page access.

Each asking device uses one designated null-anchor discussion thread in the existing
page/epoch own stream: `threadId` equals its writer device UUID. Creation batches the
thread and opening comment atomically under the existing writer lock; a duplicate
creation refuses. Subsequent turns reply to that thread through the existing writer.
There is no new store, record kind or field. Ordinary Comments excludes designated
Chat threads; Chat shows every device's page-visible Chat history, while each input
continues only its asking device's thread. Chat offers deletion of individual messages,
not its designated thread, so deleting a message does not prevent further turns.
Existing unthreaded Ask records remain
readable in Chat. The same captured-context validation and byte limits apply.

Verified agent replies join only to their originating comment IDs. Chat and annotation
threads share one turn component and attribution: each reply has one agent-name,
agent-role and time byline followed by plain reply text. Pending, held and failed
outcomes appear as a quiet mark-plus-word line in the requester turn; delivery state
and recheck/abandon controls disappear when a reply exists, including an empty reply.
Chat uses sided square tinted turns; annotation threads use one column with square
User/Bot avatars and an ink rail on the agent body. Resolve/Reopen/Close thread
controls have visible text, Lucide icons and accessible labels. After the existing
two-hour observation window, an accepted turn awaiting a reply shows a display-only
reply timeout; it keeps its accepted ledger state and read-only recheck action. No
elapsed time authorizes a resend or changes durable ordering. Chat text never enters
the author-code renderer; highlights still carry only anchor IDs and quote selectors.

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

| Type         | Additional fields / behavior                                                                                                                        |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello`      | `device, membershipRevision, cursors`; authenticated session/proof from upgrade, page/epoch admission before catch-up                               |
| `catchup`    | `membershipHead, baseline, optional baselineObject, streams, more`; bounded pages of each stream's namespace checkpoints and subsequent tail        |
| `subscribe`  | `cursors`; observe only the admitted page/current epoch                                                                                             |
| `append`     | `streamId, seq, envelopeHash, envelope`; create-only, exact frozen retry returns original receipt                                                   |
| `save`       | `operationId, baseSha256, sourceSha256, source[, attachments]`; owner device only; the browser's whole-source Save, prepared and committed natively |
| `savestatus` | `operationId`; owner device only; what the page recorded for an earlier `save`                                                                      |
| `saveresult` | `operationId, state` and `revision` or `code, message`; the one reply to a `save` or `savestatus`                                                   |
| `receipt`    | `streamId, seq, envelopeHash`; durable acceptance, not task completion                                                                              |
| `broadcast`  | `streamId, seq, envelopeHash, envelope`; subscriber must verify before applying                                                                     |
| `ack`        | `cursors`; scoped delivery positions only                                                                                                           |
| `awareness`  | `device, data`; bounded ephemeral presence, never persisted or authority                                                                            |
| `error`      | `code`; one of DENIED, EXPIRED, STALE_EPOCH, INVALID, GAP, CAPACITY, CONFLICT, RESYNC_REQUIRED                                                      |

### Implemented stream subset (#1156, #1166)

The externally driven local sync module implements the strict operations below.
Registration is #1162; `serve` composes mounted socket sync in #1211. Remote owns upgrade admission and
supplies the authenticated principal. The module takes an already-upgraded
nonblocking duplex stream, not HTTP headers.

Every client message is one UTF-8 JSON object with exactly the common fields
`version:1, type, space, page, epoch` and the operation fields listed below.
Epoch is a positive canonical decimal string. Duplicate/unknown fields, nulls,
wrong types, noncanonical values and unsupported operations reject.

| Client type  | Exact additional fields                                        | Implemented behavior                                                                                                       |
| ------------ | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `hello`      | `device, membershipRevision, cursors`                          | Device matches principal; one successful hello per connection starts server-driven catchup, then live delivery.            |
| `subscribe`  | `cursors`                                                      | Empty list starts live delivery; nonempty list resolves cursors and starts catchup, then live delivery.                    |
| `append`     | `streamId, seq, envelopeHash, envelope`                        | Inline update or object reference; verify complete exact bytes and durably append before receipt.                          |
| `chunk`      | `objectId, envelopeHash, index, count, bytes`                  | Complete the connection's pending referenced append or save; no standalone upload or partial append.                       |
| `save`       | `operationId, baseSha256, sourceSha256, source[, attachments]` | Owner device only. Inline source or `{objectId}` plus chunks; see Browser Save below.                                      |
| `savestatus` | `operationId`                                                  | Owner device only. Answers from the root-local operation record; see Browser Save below.                                   |
| `ack`        | `cursors`                                                      | Resolve retained scoped positions and release one frame credit; no deletion, core acknowledgment or application authority. |
| `awareness`  | `device, data`                                                 | Device matches principal; at most 4 KiB canonical base64url bytes, ephemeral.                                              |

`cursors` has at most 256 strict objects `{streamId, namespace, seq, envelopeHash}`,
unique by stream/namespace. Sequence zero is an explicit bootstrap sentinel and
requires zero32 hash; an omitted namespace also bootstraps. A nonzero cursor must
match an exact retained update or checkpoint identity in that namespace. ACK validates
that identity even after its payload is pruned: partial chunks report the last admitted
position while consuming the existing frame credit, without granting read authority.
Hello/subscribe catchup additionally requires the matching payload; a retained receipt
alone cannot resume catchup after pruning. Unknown, wrong-namespace and hash-substituted
cursors return `RESYNC_REQUIRED`. A client restarts with zero/omitted cursors to receive
the latest paired prefix checkpoint and retained tail.
If compaction invalidates a cursor during catchup, catchup stops with
`RESYNC_REQUIRED` instead of silently skipping data. Store preserves the durable
receipt ledger, so pruning never permits accepting a sequence again.

`append.seq` is positive canonical decimal text. `streamId` equals the principal
and signed author. `envelopeHash` is canonical base64url hash32. `envelope` is either
canonical base64url of the exact frozen model envelope JSON or the strict object
`{objectId}` (canonical 64-character lowercase hex). These alternatives have no
optional fields. Signed scope, sequence, object ID, hash and `update` kind must
match. Current caller-owned namespace/role/revision admission precedes the model
signature verifier. The server never opens ciphertext or decodes Yjs.

Server `receipt` has `streamId, seq, envelopeHash`; `broadcast` additionally has
`envelope` in the same inline/reference shape. Both use the common fields.
New appends broadcast to admitted subscribers, including the subscribed sender;
exact retries return the original receipt without a second broadcast. Server
awareness has `device, data`. Scoped errors have `code` from the table above.
Malformed, oversized, binary or inbound server-only frames close with code 1008
and reason `INVALID`. Capacity never evicts accepted receipts or payloads.

#### Browser Save (#2032)

The browser publishes its whole source over this socket, never as a content update it signs
itself. Only a registered owner device may send `save` or `savestatus`; readers and public readers
are denied by the same per-frame admission as `append`, and the socket gains no body route and no
change to any body cap.

`save` carries `operationId` (a client-chosen generated ID), `baseSha256` (canonical base64url
SHA-256 of the UTF-8 source the editor started from), `sourceSha256` (the same for the new source)
and `source`: inline canonical base64url when it is at most 32 KiB, otherwise the strict object
`{objectId}` (lowercase hex SHA-256 of the source) followed by `chunk` frames whose `envelopeHash`
is `sourceSha256`, exactly 32 KiB each except the last. At most one upload assembles per
connection, at most `SAVE_CHUNKS` (64) chunks and 2 MiB, and it completes within `SAVE_UPLOAD`
(10 s) of the `save` frame; anything else, a digest that does not match the bytes, or a second
request while one assembles ends the connection with `INVALID` and commits nothing.

`attachments` is optional and is a typed change of the document's `meta.attachments`, never
free-form metadata: `{set?: descriptor[], remove?: attachmentId[]}`, each list at most 128. Every
`set` descriptor is a valid document asset (`namespace` `content`, `source.kind` `document`)
whose `sourceDigest` is the SHA-256 of the `source` being written, so a change cannot ride on
another source; IDs are unique across both lists. Descriptors are immutable: a `set` of an
existing ID must be identical (a no-op), a `remove` of an absent ID, a different descriptor
under an existing ID and a result over 128 are refused. The list is applied removals first, new
descriptors appended, and an empty result removes the key (absence is the no-attachment
grammar). The serve checks structure and source binding when the frame assembles; the isolated
decoder re-checks them and applies the change in the same atomic content update as the source;
native preparation additionally requires each added descriptor to name this space, page and
epoch and to be witnessed by its creator's own-stream creation proof, so the browser publishes
that proof first and waits for its own fold to admit it. A change that violates a check is a
protocol error (`INVALID`) or a terminal `rejected` result and commits nothing; a change
that leaves the list as it is publishes nothing. The browser's private `Fold.prepareContent`
applies the same change and both preparers consume
[`attachment-change-v1.json`](vectors/attachment-change-v1.json) (oracle
`attachment-change-reference.py`).

The serve verifies the digest, then prepares natively exactly as `tmt colab page write` does
(`page::prepare_publication` in the isolated decoder) from its own fresh read-only snapshot,
outside the sync lock, with the base-source digest as a precondition. It then re-takes the lock
only to commit through `commit_publication` with the same fences, so a page that moved while the
save prepared refuses as `COLAB_STALE_BASE` and never overwrites the newer content. A commit is
signed by the root-local writer (`Keyring::local_writer`), the same stream the CLI publishes on,
never by a browser device: the browser holds no content-signing authority, and its edit appears
in catch-up under the root-local stream. Subscribers receive the same ordered broadcasts as for a
CLI write. A source equal to the page's current source is `unchanged` and publishes nothing.

`saveresult` has `operationId` and `state`: `committed` or `unchanged` with the opaque `revision`,
`rejected` with the stable `code` and a person-readable `message`, or, for `savestatus` only,
`pending` (the save is still preparing, even if its connection is gone, so its outcome is not
final) or `absent` (the operation never reached the page, so nothing changed). It is the one reply to a `save`
or `savestatus`. A wide fan-out may end the originator's connection with `RESYNC_REQUIRED` before
its reply; the browser then reconnects and sends one `savestatus` for the same ID, answered from
the root-local operation record by (page, root-local stream, `operationId`). A reused
`operationId` with different bytes is `COLAB_OPERATION_CONFLICT`, and so is one whose recorded
outcome exists when the new save would change nothing (an operation ID answers only for the
save that used it, never `unchanged` for another, including a different attachment change). The browser never resends a
save: an unanswered status names the original ID, and a later Save is a new operation. The
browser refuses a source over 2 MiB before sending, naming its size and the limit.
Measured with incompressible fixtures through the real door: a fully different 1.5 MiB source
replaces a 1.5 MiB page repeatedly, exactly 2 MiB saves onto a small page, and 80 consecutive
8 KiB-growth saves each land. Replacement of a page by a fully different 2 MiB source is bounded
by the same tail as the CLI write above.

### Implemented catchup and chunk protocol

Owner discovery on the mounted socket uses read-only `GET /api/session` and
`GET /api/pages`. Missing or non-owner context returns 403 JSON `{code:"DENIED"}`.
Session returns exactly `{deviceId, publicKey, grantRevision, name}`; revision
is decimal text. It works before Colab extension-key registration and forwards
no credential. Pages returns exactly `{spaceId, ownerKey, revision, pages, pageIds}`;
`pageIds` is sorted, at most 1,000 entries, each exactly `{pageId, deleted}`. It includes
the existing retained deleted-page IDs solely to prevent historical prefixes rebinding;
no title/content of deleted pages is exposed. Its non-deleted IDs exactly match `pages`.
Browser discovery caps the complete serialized metadata response at 512 KiB.

A serve running from an installed release (`<lib>/tmt-colab/releases/<id>/tmt-colab`)
compares itself with `<lib>/tmt-colab/current` (`serve_release.rs`). When another release
with a different `receipt.json` `version` is active, `GET /api/serve-release` (same admission
as session) returns exactly `{running, installed}`; otherwise exactly `{running}`. Versions
are at most 64 characters of `[0-9A-Za-z.+-]`. The check reads only the install layout (one
bounded receipt read, cached for a minute), and any doubt, a development binary or a reinstall
of the same version is "not stale". The serve also prints one stderr warning (never in
`--json`): `Colab <installed> is installed, but <running> is still running.` and
`Restart \`tmt colab serve\` to update.`; the tab row says the same. Nothing restarts,
signals or changes state; a serve that predates this route cannot report itself.
`pages`is sorted by page ID, at most 1,000 entries, each exactly`{pageId, epoch, sharing, history, archived, retentionDays, lastUpdateAtMs,
expiresAtMs, warnings}`. These time/expiry hints use the same verified projection
as `ls/show`, defined in [Local management CLI](#local-management-cli-1307).
Local creation/epoch records own
existence; sharing/history/archive/delete derive from verified owner statements.
Absent sharing/history mean private/shared. Deleted pages are excluded. Titles
are encrypted content and never returned here. `revision:"0"`means no owner log
has been initialized yet. State faults return 503 JSON`{code:"UNAVAILABLE"}`.

Catchup is server-driven. Strict hello additionally requires decimal
`membershipRevision` (`"0"` means no verified log). Its first page carries
`membershipHead, baseline, streams, more`. The exact head DTO is
`{revision, statementHash, ownerKey, statements, more}`. Statements are canonical
base64url of exact stored statement-envelope JSON after the client's revision,
at most 64 per page. While head/membership `more` is true, later pages carry
`membership:{statements,more}` before stream objects or wraps. The pinned retained
head is the target for this catchup; unknown, missing or above-head revisions
return `RESYNC_REQUIRED`. Clients independently verify owner signatures, chain,
root pin and target hash; a client-side fork cannot be detected from the unsigned
revision alone. A statement entry is either an inline canonical base64url string
or exactly `{statementHash}`, with a canonical base64url hash32 reference to the
model membership hash. Envelopes above 32 KiB use references. A referenced
statement is the only entry on its membership page and is followed immediately
by scoped server `chunk` frames with exactly `statementHash, index, count, bytes`
beyond the common fields. Object chunks retain their separate
`objectId, envelopeHash, index, count, bytes` shape; mixed identities and unknown
fields reject. The transferred bytes are exact stored statement-envelope JSON,
not reconstructed payloads or a new raw-JSON hash.

Membership pages contain at most 64 entries and 60 KiB of encoded inline data;
the first page additionally respects its metadata/baseline wire budget. When the
first page supplies a `baselineObject` (inline or referenced), inline statements
may fit that budget, but any statement reference is deferred to the next
membership page. Baseline chunks, if any, stay consecutive first, so there is
only one pending assembly.

Statement envelope admission uses the existing model cap:
`floor((768 KiB + 1 KiB) * 4 / 3) + 2 KiB`, or 1,051,989 bytes and at most 33
chunks. SQL checks that cap before loading the transfer bytes. Chunk bytes/order,
consecutive delivery and frame credit follow the object-transfer rules below;
assembly uses the same absolute two-second deadline. Partial bytes never advance
or persist the referenced statement. Before admission, clients check strict model
syntax and payload digest, the reference's model statement hash, pinned owner
root/space, owner signature, next revision and previous hash. The completed
membership page must agree with its advertised target head before log persistence
and dependent chain/wrap/content admission. Invalid/oversized/interrupted transfers
discard partial bytes without truncating durable statements; payload admission
remains 768 KiB.

After membership, pages carry `wraps` addressed to the caller-admitted recipient:
owner devices/the management member, only the reader's link, or none for public,
ordered by numeric epoch, kind, recipient and revision. They include retained
epoch-advance and history-join wraps for the 64 latest retained epochs through
the requested epoch, and are bounded by 512 entries and 60 KiB encoded bytes per
page. An empty list means no wraps exist. Each stream-object page carries
`chains:[{deviceId,chain}]` with exact chain transport as canonical base64url for
its author if not already sent on the connection (at most 64 per page). Retained
revoked-author chains can be delivered: clients MUST enforce the verified log's
[signed-cut restrictions](#decoder-isolation-compaction-and-limits) on historical
objects before applying them. A chain never grants current authority.

`baseline` is null or canonical base64url of exact model baseline-descriptor
JSON, bounded to 8 KiB. Its scope/revision must match the admitted page/epoch and
retained head. The caller verifies its exact signed `epoch.advance` binding.
The owner engine produces and persists reset baselines; mounted owner catchup
reads the exact descriptor through `Store::baseline`. Native sync delivers the
matching scoped stored encrypted object after caller admission.
When `baseline` is non-null, the first page MUST also contain exactly
`baselineObject: {envelopeHash, envelope}`. When `baseline` is null that field MUST
be absent. `envelopeHash` is canonical base64url hash32 and MUST equal the signed
descriptor's `objectEnvelopeHash`; `envelope` is the existing exact inline
base64url envelope or `{objectId}` followed immediately by consecutive chunk frames. Later pages MUST NOT
repeat either baseline field. The first page still has empty streams; baseline
retrieval does not advance any device stream or cursor. Missing stored objects or
descriptor mismatches resync; malformed or hash/scope/kind/revision mismatches
reject, never substitute old-epoch source. The same frame credit and
complete-object admission rules apply.

Before publishing a reset view, the browser MUST finish owner-log verification,
match every descriptor field to the signed epoch transition, obtain the admitted
new-epoch wrap, and verify the exact baseline envelope hash, management-member
signature and `html/content` sequence-zero context. Its author is the pinned owner
management member, not a device certificate; its revision equals the descriptor's
revision. Plaintext is strict JSON with only `source` and `update` as specified in
[current-view baseline](#current-view-baseline-and-history-modes). The dedicated
Worker verifies the source digest, LP commitment and exact source/title projection
from that identical update before initializing a fresh content document. Apply
current-epoch tails only after that initialization; publish nothing on any failure.
Old-epoch envelopes and a missing baseline for a reset epoch MUST reject. The
browser implementation is tested with signed fixtures; those fixtures do not
establish native mounted browser E2E.

The first page has empty `streams` and `more:true`. Later pages carry
`streams, more` and the applicable membership, wraps or chains fields. Each stream entry is exactly
`{streamId, namespace, checkpoint, tail}`. A checkpoint is null or
`{seq, envelopeHash, envelope}`; tail is a list of those same entries. A page
contains at most one object: either the latest paired prefix checkpoint for
bootstrap, or the next update after the resolved cursor/checkpoint. All bootstrap
checkpoints precede tails. Each stream's tail merges both `content` and `own`
namespaces in shared sequence order; updates from either namespace are required
to verify the signed previous-hash chain. The final page has
empty streams and `more:false`. Clients do not re-request pages. Clients verify
all log, envelope and chain/namespace bindings before applying an object; a page
or receipt is not that verification. A stream sequences namespaces together,
so namespace-tail sequence numbers may interleave rather than being consecutive.

After hello, every server-to-client application frame (metadata, chunks, final
page, receipt, broadcast, awareness and errors) consumes one frame credit. At
most eight frames are outstanding; each valid scoped client `ack` resolves its
cursors and releases exactly one credit. Empty/unchanged cursors are valid for
metadata and partial chunks; they grant no object admission. An ack with no
outstanding frame is `INVALID`, so credits cannot be banked. No frame is sent
while credit is exhausted, even if the socket is writable. The reference and
all chunks stay consecutive across credit releases, with no interleaved receipt
or live frame; clients admit only the fully reconstructed object. Live frames
awaiting credit use the existing bounded queue and overflow still resyncs.
Pre-hello live-only subscribe remains available without the hello credit flow.

Pages are generated only when that peer's outbound queue is empty, its buffered
write is complete and frame credit is available. Store reads use a transaction, current-epoch fencing, SQL-side
payload-length checks and a bounded inventory of at most 256 stream/namespace
pairs per page scope. A larger inventory returns `CAPACITY`, without eviction.
Every page rescans the inventory: appends to already visited namespaces and newly
created streams are included before completion. Under the same server lock that
observes no remaining objects and queues the final page, the peer becomes a live
subscriber. Appends after that boundary broadcast behind the final page. Callers
must serialize authority transitions and sync writes through that server owner.
An empty-cursor subscribe remains live-only and makes no historical-data claim.

Large catchup/broadcast envelopes reference `{objectId}` with their outer
`envelopeHash`, followed by server `chunk` frames using the same common scope
and exactly `{objectId, envelopeHash, index, count, bytes}`. Envelope JSON over
32 KiB uses chunks. Raw `bytes` are canonical base64url, nonempty and at most
32 KiB; every nonfinal chunk is exactly 32 KiB. `index` and `count` are JSON
integers: consecutive zero-based index, positive bounded count, index below count.
Transfer scope, object ID, envelope hash and count cannot change.
Baseline transfers use the model's non-update envelope ceiling (16 MiB plaintext
plus authentication tag, framed header and base64/JSON overhead), exposed by the
browser model as `MAX_ENVELOPE_JSON`; their chunk-count ceiling is that serialized
bound divided by 32 KiB, rounded up. This does not raise the update ceiling below.
The same one-transfer rule, absolute two-second deadline and queue bounds apply;
a maximum accepted size is not a delivery-time promise. Consumers retain
only one bounded incomplete object and apply nothing until exact reassembly,
model hash/signature and application admission succeed; abnormal close discards it.

Inbound append references reserve one transfer per connection. Its two-second
absolute acquisition deadline starts at the reference and never renews per chunk.
The serialized update cap is `(256 KiB + 2 KiB) * 4 / 3 + 2 KiB` bytes (integer
arithmetic), at most 11 chunks. Oversized aggregate bytes return `CAPACITY`;
invalid count/order/identity returns `INVALID` and discards the transfer. Deadline
expiry closes `INVALID`, including when no more input arrives. Completion alone
passes the full original append admission/signature/create-only checks; partial
bytes never reach Store or broadcast. Error, revocation, disconnect and drop
release incomplete bytes. The outbound object cap remains Store's 16 MiB + 2 KiB,
at most 513 chunks. No all-chunks-in-memory frame list is generated.

Both WebSocket frame and assembled-message payload caps are 64 KiB. The outbound
queue holds at most eight entries including a buffered write; a lazy object
transfer reserves an entry and emits one bounded frame per turn. Transfer bytes
are immutable/shared across broadcasts and bounded by the object cap per entry.
Overflow clears pending delivery and closes `RESYNC_REQUIRED`; catchup pages
larger than the queue budget are emitted lazily, never enqueued all at once.
A blocked write has a one-second deadline driven by the caller (`poll` or
`poll_at`). A transport that cannot close without flushing blocked ciphertext is
dropped. Authority is rechecked on every operation, delivery and caller-applied
change; previously written bytes cannot be recalled. Clients resync on abnormal
close. The foreground socket workers drive this library over accepted registered
owner upgrades. `registration::OwnerAdmission` reads a durable authority snapshot
on each check: active registration/device, certificate lifetime, pinned owner
management member and editor issuer, retained head and current page epoch. The
owner management member admits local pages regardless of its empty genesis page
list. Append additionally requires the current membership revision and an allowed
namespace, and returns the registered extension signing key for model signature
verification. Upgrade verifies the full remote binding and device chain; repeated
Read checks do not redo signatures or reserve the SQLite writer. Catchup takes its
head from `Store::owner_head` and the exact persisted reset descriptor through
`Store::baseline` when one exists. Workers preserve upgrade read-ahead, drive silent transfer/write deadlines,
apply the tunnel cap/idle bound, and close retained sockets before shutdown joins.
An established connection sends a WebSocket Ping every 30 s (`limits::KEEPALIVE`) and the
browser answers with a Pong, so a quiet tab keeps bytes moving both ways inside the 120 s
tunnel idle bound of both this server and the remote door; a browser cannot send pings itself.
Without it every idle tab reconnected every two minutes, and each reconnect costs one journaled
Remote read (1000 per device per day, after which the agent directory is refused, #2170).

The #830 fixture used 64 KiB frames/messages, queue 8, receipt/tail capacity 64,
16 sockets, ten-second connection lifetime, two-second handshake reads and
one-second writes. L2 must pin product caps separately, with positive large-
update/chunk controls, strict framing, gap errors and slow-subscriber closure;
these fixture numbers are not production capacity promises. An overflowing
subscriber is explicitly closed with RESYNC_REQUIRED and must catch up; accepted
durable payloads/receipts survive. Firestore listeners implement the same scoped
immutable-object/cursor semantics without pretending to be a WebSocket server.

Local storage uses extension SQLite/files and `colab-sync-v1` WebSocket. Colab's
local listener is only its owner-only socket `<dataRoot>/colab/door.sock`, which
remote mounts at `/r/<prefix>/x/colab/` (#1039): remote's door owns Host, Origin,
DNS-rebinding and cookie admission, and colab trusts the forwarded
`tmt-device-context` because only the owner can reach the socket. Its HTTP
handling uses bounded std-thread workers, workspace tungstenite and strict
framing. Upgrades require an active registered owner context or the page-scoped
[read-only reader ticket](#mounted-read-only-reader-sessions-1310). Public grants
no write/agent authority; a bare unauthenticated upgrade rejects before effects.

### `serve` and the Remote door (#1584)

`tmt colab serve` is the one command for browser access. After its socket is bound it
learns the door only through Remote's public CLI, run through the invoking core executable
with a bounded call each (three seconds, 16 KiB), never Remote's files:

- `tmt remote status --json` → `{running:true,origin,path:"/r/<prefix>"}` **attaches**: nothing
  is started and the door is never stopped. `{running:false,...}` is the only answer that
  starts a door (next bullet). A Remote error envelope (`{error:{code,message}}` with a
  non-zero exit, for example `REMOTE_SERVE_OUTDATED` from a serve that predates `status`) is
  shown as Remote wrote it, with no start attempt, because a second door would race the one
  that may be running. No answer at all (missing command, timeout, malformed) is the install
  line below.
- `tmt remote serve --json` is started as a supervised child in its own process group and its
  first stdout line `{state:"ready",address,...}` gives the door (15-second bound; Ctrl-C
  aborts the wait). If startup fails, Colab reads a bounded first line from stdout and stderr
  and shows a Remote error envelope through the same warning path as a status error;
  the message retains Remote's port and explicit `--port` choices. Other stderr diagnostics
  keep streaming. Colab passes no port: Remote owns port reuse. On Ctrl-C or SIGTERM Colab
  closes its socket, then sends SIGTERM to the child's whole group, SIGKILL after three
  seconds, and always reaps it. A door that exits on its own is reported once and the local
  space keeps running. A SIGKILL of Colab itself cannot clean up; run `tmt remote serve`
  separately to recover.
- No door can be started: Colab runs local-only. When Remote gave no answer at all, the
  warning is `Browser access needs the Remote extension: tmt extension install remote --yes`;
  when Remote answered but would not start, Remote's own error message if available,
  otherwise `The Remote door did not start; ...`.

Pairing is never done by `serve`: it reads `tmt remote devices --json` (devices not
`revoked`) and only reports. Running `tmt remote serve` separately keeps working and is the
attach path.

`serve --json` prints one line, then nothing until shutdown. Keys are stable and absent
facts are `null`: `spaceId`, `socket`, `profile:"colab-sync-v1"`, `state:"mounted"`,
`door` (`"attached"|"started"|"unavailable"`), `origin`, `url` (the app link
`<origin>/r/<prefix>/x/colab/`), `paired` (`true|false|null` when unreadable or no door),
`devices` (count), `pages` (count of non-archived pages, `null` if unreadable), `page` (the
first page's full link, or its relative path without a door), `shortLink` (its short owner link,
`null` without a page or door), `next` (commands still
needed: `tmt remote pair` unless paired, `tmt colab page create --title <title>` when there
is no page) and `warning`. Human output is the `LOCAL SPACE` detail view with the same
facts and the next step: `door`, `paired`, `open` (the page link or `create one: ...`) and
`pair` (`pair this browser once: tmt remote pair`, or `if this browser is new: ...` when
pairing is unknown), shown before `open` because the link needs a paired browser. Without a
door, `open` shows the relative path and the reason (the install line, or `browser access
unavailable: see warning`), never a command that `serve` replaces.

### Page links and opening the browser (#1614)

Every command that names a page tells a person where to open it, from one door and pairing
lookup per command (`tmt remote status --json`, then `tmt remote devices --json` while a door
runs; the same bounded calls as `serve`). `page create`, `ls`, `show` and the `share` commands
print the **short owner link**, `<origin>/p/<shortId>`, while a door runs. Remote owns that
root redirect into `<door>/x/colab/p/<shortId>`; it does not bypass pairing or grant admission.
Colab resolves that alias to an absolute same-origin mounted-root path from Remote's
`tmt-mount` header; missing or invalid mounted roots refuse, with no relative fallback.
Without a door they print the mount-relative alias `x/colab/p/<shortId>` and the
reason: the install line when Remote gave no answer; Remote's own message for an error envelope,
with one shared wording for `REMOTE_SERVE_OUTDATED` (`The running Remote serve is older than
this Colab. Stop it with Ctrl-C in its terminal, then run tmt colab serve.`; the `serve` row then only says `(Remote serve is outdated; see warning)`; else `run tmt colab
serve to get a full link`; never a manual `tmt remote serve`. When no paired device is known, the same pairing step
as `serve` follows (`pair this browser once: tmt remote pair`, or `if this browser is new: ...`).

`--json` results that name a page carry `path` (relative), `link` (full, `null` without a
door), `shortLink` (short owner link, `null` without a door), `paired` (`true|false|null`) and `next` (`["tmt remote pair"]` or `[]`). `ls` carries
`path`, `link` and `shortLink` on each page and `paired`/`next` once; its human rows lead with
the title (`Untitled page` when empty), then its shortest unique catalog prefix (at least eight characters) and the
audience/history. Each link is on its own indented line under the row. `show` retains the full
page ID and prints rows of words (`title`, `link`, `pair`, `page`,
`sharing`, `history`, `retention`, `membership`, `members`, `links`; `–` for none), never an
embedded JSON object. `page create` keeps `url`'s role under the name `link`.

`tmt colab open [PAGE]` explicitly opens the existing space home, or a page selected
through the same verified catalog and UUID-prefix resolver as `show`. It requires
running Colab and Remote services and never starts another service, pairs a browser,
changes access or writes content. Explicit opening ignores the automatic-open setting
and noninteractive-terminal suppression, using the shared `tmt-invoke` opener;
`--no-open` and `--json` suppress launching. Missing, deleted and ambiguous pages refuse
before opening. Archived pages still print their link, but the browser does not open an
archived page in local v1 (follow-up #2298). An unavailable
service prints the current link/path and next step. Opener failure warns once and
retains the link. JSON adds `spaceId`, full `pageId` (null for home), `running` and
`opened: false` to the existing path/link/shortLink/paired/next facts; a stopped Colab
adds `tmt colab serve` to `next`. Browser navigation still undergoes existing admission.

### Short owner-page aliases (#1688)

The shortest unique canonical page UUID prefix in the complete current catalog is the
`shortId`, with a minimum of eight ASCII characters. Archived pages participate even when a
listing hides them. Retained deleted IDs also participate, so a historical link cannot rebind
to a later page after deletion. No alias table, stored short ID, plaintext title or schema is added.
A missing catalog entry uses its full UUID rather than borrowing another page's prefix.
Previously copied aliases can become ambiguous when another page is created; they never
choose an arbitrary match.

Colab handles GET `/p/<shortId>` directly inside its mount, independently of Remote's root
redirect. Prefixes are bounded to 8–36 canonical lowercase UUID-prefix characters; malformed
or non-GET aliases receive the existing 404. Owner-context discovery uses the existing
verified metadata catalog, without Yjs decoding, and responds with a relative same-mount 302
and the existing no-store, no-referrer and CSP headers. A unique match targets the full
`#space=<spaceId>&path=%2Fpages%2F<pageId>` route. Ambiguous and unknown prefixes target
`#space=<spaceId>&path=%2Fshort%2F<shortId>`; the admitted parent shows every matching page
as a plain link row, or the existing unavailable-page view when none match. A sole deleted
match shows "This page was deleted"; deleted chooser rows are disabled and have no title. Choosing a live row
opens its full UUID route. Display title hints confer no authority.

Without owner context, the alias redirects to the private root guidance with only the
requested prefix, never the space ID or inventory. The existing paired-session recovery
retains that target across discovery/pinning; it creates no anonymous asset or API access.
The long owner fragment route remains supported. Reader links retain their capability
fragment and are never shortened into an owner alias.

`serve` (once the door is ready, attached or started) and `page create` open the link in the
default browser: the page when the space has exactly one, else the space home. Opening is
skipped, printing the link only, with `--no-open`, `--json`, the setting `open` off, no
terminal on stdout, `CI`, an SSH session without a display, Linux without `DISPLAY` or
`WAYLAND_DISPLAY` (WSL excepted) or no opener on `PATH`; `--open` overrides everything except
`--json`, `--no-open` and a missing opener. Openers: macOS `open`, Linux `xdg-open`, WSL
`wslview` then `explorer.exe`. The row reads `opened in your browser: <link>` or just the link;
a failing opener warns once and keeps the printed link. `serve --json` adds `opened`.

`tmt colab settings [open on|off] [--json]` shows or sets the one setting, stored as
`{"open":true|false}` in `<dataRoot>/colab/settings.json` (default on; unknown keys ignored; a
damaged file reads as the default with a warning). JSON: `{open,source:"default"|"settings.json"}`.

### `tmt colab stop` (#1594)

`tmt colab stop [--json]` asks the serving process of this data root to shut down. It never
signals a pid. The request is `POST /.tmt/colab/local/stop` on the owner-only
`door.sock`, a root-local route like page write: a request carrying a forwarded device
context, a non-POST or an upgrade is refused (403/400), and the mount never forwards the
reserved `/.tmt/` subtree from a browser, so no new network surface exists. The reply
`{stopping:true,door:"attached"|"started"|"unavailable"}` is written before the accept loop
sees the flag, then shutdown is exactly SIGTERM's: close the socket and workers, then stop the
door `serve` started (never an attached one). Pairings, grants and data are untouched.

The command then waits up to 10 seconds for the serve lock to release. Output: when nothing
runs (including no state at all) it succeeds with `Colab is not running`, JSON
`{state:"not-running",door:null}`; after a stop `Colab stopped` (`, and the Remote door it
started` for a started door) and, for an attached door, `Remote is still running; stop it with tmt remote stop`; JSON `{state:"stopped",door}`. A refused or unanswered request is
`COLAB_UNAVAILABLE`; a request accepted but a serve still running after the wait is
`COLAB_OUTCOME_UNKNOWN`; both exit 1 and are never retried. A `stop` sent while `serve` is still
waiting (at most 15 seconds) for a starting door is not answered until the wait ends.

The local space is loopback-only: there is no `--bind`, LAN or other
non-loopback mode. Other people's machines reach a page only through a cloud
backend (Firestore, then Cloudflare). L2 verifies owner-only socket admission,
body/acquisition caps and timeout/shutdown behavior; Host allowlisting is
remote's.

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
days of expiry. Cloud readers treat expired-but-present data as gone. Local
expiry never deletes local data: warnings begin seven days ahead in the browser and `ls/show`,
and expired local pages remain readable and writable unless signed archive/delete
policy forbids it. Local data is never automatically deleted.

Firestore Rules deny expired reads; owner browser/CLI cleanup removes expired
pages. Optional Blaze TTL is eventual physical cleanup, not timely revocation.
Spark compaction and expiry share the daily per-page delete budget; exhaustion
backs off and keeps data longer, never loses live data. DO alarms delete page
storage and R2 objects; R2 lifecycle cleans orphan staging only. Archive hides
and freezes; delete ceases access and removes ciphertext. Neither promises
secure erasure or recalls offline copies. The local owner engine signs retention
changes without scheduling expiry. Archive allows reads/catchup but denies writes;
delete denies reads, writes, catchup and queued delivery and removes the page's
ciphertext, checkpoints, baselines, wraps and epoch secrets in the owner
transaction. Signed policy, operation receipts and the reserved page ID remain;
exact replay returns the saved outcome without restoring deleted data.

The local-build management surface is defined in [Local management CLI](#local-management-cli-1307);
`serve` and `spaces` remain available. The remaining proposed surface is
`create <file|->`, `cat`, `edit (--file|--patch)`, `snapshot`, `restore`, `comment`,
`pair`, `devices [revoke]`, `approve`, `refuse`, `retry`, `abandon`.
Those commands are proposals, not installed usage guidance. Edit computes minimal text diffs as Yjs
operations so concurrent browser/CLI edits merge. Comment never dispatches.
Space home/CLI management expose sharing, threads, anchors, conversations,
members, snapshots, activity/expiry and held/uncertain sends. Enrollment, member
management and local-agent grants remain distinct controls.

### Local management admission (#1306)

The owner socket accepts `POST /api/management` with strict JSON containing exactly
`request`, `payload`, and `signature`: canonical base64url of the existing framed
management input (at most 1 KiB), exact payload JSON (at most 16 KiB), and signature64.
The HTTP body remains bounded to 64 KiB. The registered Colab signing key verifies
possession; no request-selected key is authoritative. The forwarded Remote context
must be live, owner-bound, registered, unrevoked and match `senderDevice`. The request
must match this space, use an admitted page context, and satisfy
`issuedAt <= now < expiresAt`, with the existing ten-minute maximum window.

Request payloads contain user selections only, separate from owner statement DTOs:

| Operation                                      | Exact request fields                                                         |
| ---------------------------------------------- | ---------------------------------------------------------------------------- |
| `epoch.advance`, `page.archive`, `page.delete` | `pageId`                                                                     |
| `page.share`, `page.history`                   | `pageId, mode`                                                               |
| `retention.set`                                | `pageId, days` (positive safe integer or null)                               |
| `member.add`                                   | `memberId, role, signKey, encKey, pages`                                     |
| `member.remove`                                | `memberId, pages`                                                            |
| `member.role`                                  | `memberId, role, pages`                                                      |
| `link.add`                                     | `linkId, role, pages, seed`                                                  |
| `link.remove`                                  | `linkId, pages, replacement` (required null, or a complete link-add request) |

Page IDs match the framed initiating context. Page sets are sorted, unique and
bounded to 256 generated IDs. The engine fences the affected assignment in its
plan and writer transaction; remove/re-role scope includes the complete stored
assignment. A non-null replacement requests atomic Reset through the link engine,
not a new signing operation. Requests cannot supply cuts, baselines, epoch numbers,
wraps, publishedKeys, grant revisions or signing instructions. Public publication
is selected by trusted local composition, never a browser-supplied backend flag.

Seeds are caller-held, transient inputs in the local stage: canonical base64url
seed32. They are never persisted in statements, projections, receipts or logs.
Before any relayed transport, the seed MUST be encrypted to the owner; plaintext
seed payloads through a relay are forbidden. A lost caller seed requires explicit
Reset, not regeneration during retry. Revocation of Remote devices/grants remains
separate from these management requests.

Browser replay binding is SHA256(LP(`tmt-colab-management-transport-v1`, requestBytes,
payloadBytes, signature64)). The engine binds that digest plus normalized action and
request scope in the existing owner-operation transaction. An exact eligible retry
returns its stored outcome/head without fresh signing, wraps or baselines; changed
signed bytes with the same operation ID conflict. Expired or revoked callers cannot
recover authority by retrying. New operations fence the expected owner revision.

The owner-only `POST /.tmt/colab/management` route takes exactly
`space, page, expectedRevision, operationId, operation, payload`; revision is canonical
positive decimal text and payload is canonical base64url of the same typed JSON.
The root-only `page.create` selection is `{pageId,title,source}` with optional bounded `publisherAgent` and optional complete `creationRecipient` as defined
in the content-root definition above; it accepts
revision `"0"` only when initializing owner genesis. Browser-signed management
cannot create pages. Its source/title bounds are 2 MiB/256 KiB in UTF-8; the
local route has a separate body cap for worst-case JSON escaping plus base64
(`LOCAL_CREATE_PAYLOAD_BYTES = 6 * (2 MiB + 256 KiB) + 1024`, body cap
`ceil(payloadCap / 3) * 4 + 2048`). Other management selections retain 16 KiB
payloads and mounted browser requests retain the 64 KiB body cap.
It is authorized solely by the owned private Unix socket. Any `tmt-device-context`
or `tmt-device-event` header, including an empty or malformed value, is DENIED
with 403 before payload parsing; forwarded headers never grant root authority.
Remote refuses browser forwarding into `/.tmt/`. Its digest is
SHA256(LP(`tmt-colab-local-management-transport-v1`, exactBodyBytes)). The root-local
caller supplies no fabricated browser device. Offline CLI composition can call the
same library service under the owner lifecycle lock; public CLI commands remain #1307.

Routes serialize through the sync lock before Registration, matching append/event
admission. The existing engine owns all transitions, atomic receipts and root signing.
Sync rechecks live subscriptions after the callback before sending queued data.
Success is JSON `{operationId, membershipHead:{revision, statementHash}}`, using the
operation's committed head even after later mutations. Clients refresh and verify
the owner log/wraps through bounded catchup; this reply is not authority. Errors are
400 INVALID, 403 DENIED/EXPIRED, 409 CONFLICT/STALE_HEAD and 503 CAPACITY/UNAVAILABLE.
The runner implements member/link/epoch and sharing/history/retention/archive/delete
actions. This socket composition selects loopback publication; the page-policy
engine owns rotation, published keys and deletion. Methods other
than POST and upgrade attempts are INVALID. Unknown reserved routes
remain unavailable. No schema, dependency or separate replay store is added.

### Local management CLI (#1307)

The root-local CLI is a client of the reserved management IPC while serving and
calls the same library service under the lifecycle lock offline. It MUST NOT
fall back to an offline writer after an IPC send. The owner runner alone signs,
mutates and records replay outcomes. Read commands MUST NOT initialize missing
state, change journal mode or migrate schemas.

Command names precede operands, following the shared CLI style audit (the lead's
#1307 decision replaces the positional-first #1111 sketch):

```text
ls [--archived]
show <page>
share mode <page> <private|link|public>
share link list <page>
share link add <page> [--seed-file <file|->] [--link-id <uuid>]
share link reset <page> <link> [--seed-file <file|->] [--link-id <uuid>]
share link remove <page> <link>
share member add <page> <member> <viewer|commenter|editor> --sign-key <key> --enc-key <key>
share member remove <page> <member>
share member role <page> <member> <viewer|commenter|editor>
share history <page> <shared|current>
retention <page> [<days>|forever]
archive <page>
delete <page> --yes
```

All commands support human output and one `--json` document. Human retention
reads show a day count or forever and omit policy fields absent from that view.
Mutation summaries use readable operation, expected revision and membership labels;
JSON retains canonical full IDs. Top-level `ls`
and `share link ls` have hidden `list` aliases, following the shared CLI style.
Every `<page>` operand, including `show`, `page read/write`, `export` and all
management commands, accepts a full UUIDv4 or a lowercase UUID-shaped prefix of
at least eight characters (#1720). One CLI resolver reuses the short-link helpers
against the complete verified owner catalog, including archived and retained
deleted IDs, before reading write/seed input or producing effects. A unique live
match passes its full ID to the existing domain operation; no alias state is stored.
Malformed syntax returns `COLAB_INPUT_INVALID`, no match returns
`COLAB_PAGE_NOT_FOUND`, and a unique deleted match returns `COLAB_PAGE_DELETED`
with the full `pageId`. Multiple matches return `COLAB_PAGE_AMBIGUOUS`, with no
effect and `candidates:[{pageId,shortId,title,deleted}]` in JSON. The message reads
"Page prefix <prefix> matches N pages: <shortId> (<title>), ...", using the same
shortest unique IDs and authenticated titles; empty titles read
"Untitled page", archived titles read "Archived page (title unavailable)",
unreadable titles read "title unavailable", and tombstones read "Deleted page".
The catalog and owner head are rechecked after title projection. Member, link
and operation IDs still require their full canonical UUIDs. Successful JSON and
confirmations always name the resolved full `pageId`, never the input prefix.
Audience widening and link addition/Reset MUST require explicit `--yes`; absent
confirmation sends and writes nothing. Each refusal is `COLAB_CONFIRMATION_REQUIRED`
and names the resolved full `pageId`, the consequence of that command alone (a
widening never prints the deletion text) and the retry, as
"<consequence> Run again with --yes to <action>." Shared-history pages add that the
history includes deleted text, snapshots, comments and agent replies. The full management commands (#1572)
also require `--yes` for member addition, role widening, history widening and
every deletion. Role reductions, member removal, archive and retention changes
are explicit commands without an additional confirmation. A seed is canonical
base64url seed32 from an owned regular 0600 file or bounded stdin (`--seed-file`,
for scripts and exact retries), never argv. Without `--seed-file`, `link add` and
`link reset` generate a fresh seed from the OS RNG. The seed is persisted nowhere
and appears in output only inside the [reader link](#read-only-reader-link-1545)
fragment (`readerPath`) that a successful add or reset prints once; `ls`, `remove`
and every other output never carry it. After an uncertain outcome, retry with the
same `--link-id` and `--seed-file`; a generated seed is not recoverable, so the
fallback is Reset.
Links created or reset by this v1 CLI always have the viewer role. Removal/Reset
capture complete verified assignments, as do member removal and role changes.
Member addition assigns only the initiating page. Its raw Ed25519/X25519 public
keys and canonical member UUID are caller-supplied; this is advanced or scripted
use. Member invitation flows come with the Firestore stage. Editors can change
page scripts for every viewer; member access never grants agent access.

Retention without a value reads verified policy without folding content. Day
counts are canonical positive safe integers; `forever` means null. Archive freezes
writes while leaving the page readable; retention and explicit deletion remain
available. Deletion removes local content, receipts, checkpoints, baselines, wraps
and epoch keys, retaining page, signed-policy and owner-operation tombstones.

Mutations accept `--operation-id` and `--expected-revision`; generated/default
values are captured once. Success is `{pageId, operationId, expectedRevision,
membershipHead:{revision, statementHash}}`; link creation/Reset also returns the
nonsecret replacement `linkId`. An explicit retry MUST retain the same IDs,
revision, selections and caller-held seed. Unknown IPC outcomes MUST retain this
nonsecret correlation, never regenerate a request or claim no effect.
An explicit delete retry with both operation ID and expected revision MUST still
reach the existing receipt after the page disappears from the visible catalog.
This is the sole deleted-page resolution exception; its operand must still resolve
uniquely against retained IDs, and `--yes` is still required. Retain the full ID
from the original result for retries so a later prefix collision cannot block one.
The owner engine returns the original committed head; a changed frozen request
conflicts and an unsaved request cannot revive the deleted page.

Link listings return `{pageId, membershipHead, links}`.
Retention reads return `{membershipHead, page:{pageId, retentionDays,
lastUpdateAtMs, expiresAtMs, warnings}}`.
Page lists return `{spaceId, membershipHead, pages}`; an uninitialized list has null
space/head and empty pages. Show returns `{spaceId, membershipHead, page, members,
links, discussions:"not-available"}`. Page fields are `pageId, title, epoch,
sharing, history, archived, retentionDays, lastUpdateAtMs, expiresAtMs, warnings`.
IDs, epochs and revisions retain canonical full values; times are UTC
milliseconds. Policy derives from verified owner statements; titles require the
authenticated fold and isolated decoder. Archived titles are null with
`title-unavailable` because the fold refuses archived pages. Members/links expose
ID, role, page assignments and revocation, never secret material.

Schema 5 adds nullable, nonnegative safe-integer `pages.last_update_at_ms`. The
server clock is sampled only when a new admitted content envelope is appended,
including initial creation and browser/local writes, in the receipt transaction.
The stored time never decreases for a page. Exact replay, failed/rolled-back
updates, own/discussion/Ask updates, reads, checkpoints and policy/epoch changes
never refresh it. Schema-4 migration preserves content and receipts and leaves
legacy times null without backfill. No client envelope time, file time, read time
or fabricated zero establishes expiry. The next admitted content edit starts
evidence for a legacy page.

Retention defaults to 30 days; forever is null. Finite `expiresAtMs` is
`lastUpdateAtMs + retentionDays * 86400000`, using checked arithmetic bounded to
safe integers. A legacy unknown finite expiry is null with `expiry-unavailable`
("expiry starts after the next edit"); an unrepresentable finite expiry is null
with `expiry-out-of-range`. Forever has no expiry or expiry warning, even with
unknown last-update time. At most one expiry warning is emitted: `expires-soon`
when expiry is in the future and at most seven days away; `expired` at or after
expiry. Otherwise warnings are empty, apart from a separate `title-unavailable`
CLI hint. JSON retains exact UTC milliseconds. Human `ls`, `show` and retention
reads use whole relative elapsed time, sampled once per result: `last edit 2 min
ago`, `expiry in 30 days`, and a dim `expires in 30 days` line under each list
link. Finite expiry within seven days has a `◷` waiting mark; past expiry reads
`expired 2 days ago`. Expiry intervals use days, hours or less than an hour, matching
browser retention hints rather than UTC calendar dates. The footer says "Expiry
never deletes your local copy." CLI state values are lowercase UX display copy:
`kept forever`, `starts after the next edit`, and `beyond the supported range`.
They omit redundant type labels and final periods; browser hints keep their
sentence form. A verified `expiry-out-of-range` warning shows `retention out of
range` instead of an unreadable day count. Human reads never change the projection,
policy or stored time; JSON still exposes the exact configured day count.

One verified owner-log projection supplies these hints to CLI and `/api/pages`.
Discovery metadata never grants signed policy or write authority. Local expiry
never automatically deletes or denies access. Discussion summaries in management
listings remain deferred; the page view projects verified discussion separately.

Failures exit 1. JSON is `{error:{code,message}}` with page/operation/revision/link
correlation when captured; human failures use styled stderr. Management codes
map explicitly to `COLAB_INVALID`, `COLAB_DENIED`, `COLAB_EXPIRED`,
`COLAB_CONFLICT`, `COLAB_STALE_HEAD`, `COLAB_CAPACITY`, `COLAB_UNAVAILABLE`.
CLI failures add `COLAB_INPUT_INVALID`, `COLAB_CONFIRMATION_REQUIRED`,
`COLAB_PAGE_NOT_FOUND`, `COLAB_PAGE_AMBIGUOUS`, `COLAB_PAGE_DELETED`,
`COLAB_OUTCOME_UNKNOWN`; existing state failures keep
their codes. An older schema with a
known migration path returns `COLAB_STORE_OUTDATED`: "This space was saved by an
older Colab." Its next action is "Start or restart tmt colab serve to update it."
A schema newer than this binary returns `COLAB_STORE_NEWER`: "This space was
saved by a newer Colab." Its next action is "Run tmt upgrade, then try again."
Human output uses a separate styled `hint:` line for those sentences. JSON uses
runnable commands in its top-level `next` array: `["tmt colab serve"]` for an
older store and `["tmt upgrade"]` for a newer one. It adds numeric `storeSchema` and `supportedSchema` fields in
`error`. The supported version is derived from the migration history. Refused
reads leave the database unchanged; only the existing initializing/serving paths
apply migrations, never an inspection command. Operation correlation preserves
these recovery fields. Success exits 0. Neither acknowledgments nor unsigned output
create browser authority; browser refresh still uses verified catchup.
The reserved management socket's unsuccessful response body is the exact textual
management code with its corresponding HTTP status. The CLI maps that existing
wire directly; malformed, mismatched or interrupted replies are unknown outcomes,
with captured correlation and no offline fallback.

## Root-local page creation

`tmt colab page create --title <title> [--file <path|->] [--json]` creates a private
page at epoch 1. The title is nonempty and bounded to 256 KiB UTF-8. Omitted
`--file` means empty source; a file or stdin uses the page-write reader (2 MiB
UTF-8, five-second stdin EOF deadline). A source over 192 KiB is published as ordered
content updates of at most 192 KiB of text each (stream positions 1..n), so every reader admits them as
ordinary updates and a 2 MiB page fits the 200-update/4 MiB write tail; the isolated
decoder keeps its composition limits (limits table).
Capacity rejects rather than truncating source or title.

Creation initializes a fresh local space under the serve lifecycle lock. It
uses the reserved management IPC while serving and the same lifecycle-locked
owner service when stopped. An uncertain IPC result never falls back, resends
or creates a replacement page. IDs and selections are frozen once per invocation.
The Engine prepares the Yjs document in the isolated decoder outside the writer
reservation, then commits the create-only page row, revision-one owner genesis
when needed, private `page.share`, epoch secret, eligible owner/device wraps,
certified local-writer encrypted initial content and exact operation receipt in
one transaction. An error rolls back that complete page transition; keyring and
empty database initialization may remain. Deleted page IDs cannot be reused.

Already admitted, unexpired, unrevoked owner devices receive wraps through the
existing recipient planner. Later owner-device registration supplies missing
forward wraps for existing non-archived pages inside the same authenticated device
transaction, without a new log operation. Current history supplies only the current
key; shared history supplies at most 64 retained epochs. Registration creates at
most 512 missing wraps per transaction and rejects capacity atomically. Exact
registration retries reuse existing device/epoch wraps without growth or fresh
signing; revoked devices receive nothing. These wraps confer no new Remote rights.

JSON success is `{spaceId,pageId,title,path,operationId,membershipHead}` with
`membershipHead:{revision,statementHash}` (decimal text and canonical base64url
hash32). `path` is relative to the Remote door address:
`x/colab/#space=<spaceId>&path=%2Fpages%2F<pageId>`. It pins the space using the
browser router's existing fragment format. The CLI learns the door only through
Remote's public `tmt remote status --json` (`{running,origin,path}`, one call with a
three-second cap, run through the invoking core executable), never Remote's files. See
[page links](#page-links-and-opening-the-browser-1614) for what is printed with and without a door.
JSON
failures use `{error:{code,message}}`,
with generated `operationId` and `pageId` when available for inspection after an
unknown outcome. Existing input/capacity/denial/conflict/stale-head/state/unknown
codes apply; failures exit 1. The browser home empty state names this command.

## Root-local page source CLI (#1438)

`tmt colab page read <page> [--json]` captures existing state through the
owner-authenticated fold and isolated decoder. Raw stdout is exact admitted UTF-8
source, without an added newline; stderr carries the verified head, epoch and
page revision. JSON is `{spaceId,pageId,source,title,epoch,membershipHead,
revision,memoryLimit}` with optional bounded `publisherAgent` and `originalAuthor` and optional complete `creationRecipient`, omitted
when absent. `membershipHead` is `{revision,statementHash}` with decimal
revision and lowercase hex hash. Reads never initialize or migrate state.
Archived reads retain the fold's current inactive-page restriction; deleted
operands fail through the shared CLI resolver described above.

`tmt colab page write <page> --file <path|-> [--expected-revision <token>] [--json]`
retains the title and prepares the replacement as an ordered batch of text deltas in the
isolated child against admitted Yjs structs (`page::prepare_publication`). It does not
recreate the shared document, execute HTML, advance membership or restore
discussion/authority state. Source is bounded to 2 MiB. Each update is at most 256 KiB, and
the batch added to the tail retained since the last checkpoint is at most 200 updates and
4 MiB, the effective CLI write limit (limits table); composed decoder input/output and the
5,000,000-byte gzipped page budget still apply, as do the deadline and cleanup limits.
A source over 2 MiB refuses with `COLAB_CAPACITY` naming its size and the 2 MiB limit; a
batch past the tail or page budget refuses with `COLAB_CAPACITY` naming the page and the
size or count it would reach. The 2 MiB source cap is the most a write accepts; whether a
replacement fits also depends on the retained tail. A fully different 1.5 MiB source replaces
a page of that size repeatedly, while a fully different 2 MiB replacement of a freshly created
2 MiB page is refused for the 4 MiB tail. A source equal to the page's current source publishes
nothing: it exits 0 with `changed:false` and the captured revision. Stdin has a
five-second EOF deadline. Invalid UTF-8, capacity, stale-base and inactive-page writes
reject without mutating page content. The browser Save below is the same preparation and commit with a different front door.

The opaque `revision` is `v1:` plus lowercase hex SHA-256 of framed domain
`tmt-colab-page-revision-v1`, space, page, decimal membership revision, head hash,
decimal epoch and serialized ordered namespace-cut payloads. It fences content
appends as well as membership/epoch/checkpoint changes; it is not the membership
revision alone. With an expected token, a mismatch returns `COLAB_STALE_BASE`.
Without one, the write captures its base when it starts. Commit rechecks that
same base in a writer transaction, never silently rebasing a replacement.
All failures exit 1. JSON errors use the extension's `{error:{code,message}}` shape.

Keyring derives a root-local device using independent labels
`tmt-colab-cli-device-id-v1`, `tmt-colab-cli-signing-seed-v1` and
`tmt-colab-cli-encryption-seed-v1`, under the existing management HKDF framing.
The ID uses the first 16 bytes with UUIDv4 version/variant bits. The revision-1
management member certifies its keys for 365 days. Existing keys/certificates
must match. An expired, verified non-revoked chain is renewed for the same derived
device; replacement commits atomically with the append and receipt. Revoked devices
fail closed, including receipt replay; a frozen expired request must be prepared again.
This device is not a Remote registration and grants no browser session or agent
operation authority. Private material stays in Keyring.

After preparation releases the read snapshot, the caller tries the serve lifecycle
lock. Holding it selects the offline existing-state writer: `page::commit_publication`,
then a best-effort combine of this device's own tail. A held lock selects the running
serve through the existing owned 0600 Unix socket. One device transaction checks the
pinned chain and the frozen base/head/epoch/positions, then commits the device chain,
every signed encrypted append of the batch and the scoped terminal outcome together, or
records a terminal rejection and no content; a commit never splits a batch. Exact frozen
retries return the original outcome bytes without re-signing, re-appending or overwriting
later content; changed bytes conflict. JSON success is
`{spaceId,pageId,epoch,membershipHead,revision,sourceSha256,memoryLimit,changed}` plus,
when `changed` is true, `{operationId,streamId,count,seq,envelopeHash}`. `revision` is the
page revision read, after the write and its best-effort combine, by the writer while no
other writer can run (the serve under its sync lock, or this process under the serve
lifecycle lock): the token a next `--expected-revision` carries. The retained outcome's
`committedRevision` is the one before the combine, and a reply lost to doubt reports it, so
a next write then refuses stale instead of losing an update. `seq` and `envelopeHash` are
the batch's last position. Hashes use
lowercase hex except the model envelope hash, which is canonical base64url. A rejected
outcome exits 1 with its code: `COLAB_STALE_BASE`, `COLAB_CAPACITY`, `COLAB_PAGE_INACTIVE`,
`COLAB_STATE_MISSING` or `COLAB_STREAM_GAP` (this device's stream moved on; read again).

Serving writes use POST `/.tmt/colab/local/page-publish` with the version-2 write DTO
`{version:2,action:"write",signedJob,packet,chain}` (publication section below). The
server verifies the job and every envelope against its own local writer key, never a
request-selected one, prepares one broadcast per entry before the transaction, commits
through `commit_publication` and replies 200 with `{outcome,revision}`: the exact retained
outcome JSON, committed or rejected, and for a committed outcome the page revision read
under the sync lock after the combine. A failure before any effect (denied, invalid, unavailable,
capacity) uses the `{error:{code,message}}` envelope with a 4xx/5xx status. Forwarded
device-context or event headers are DENIED; wrong methods/upgrades and unknown/duplicate
fields reject. The reserved router shares management's local-header denial. The route's
body cap is the standalone write bound (`LOCAL_WRITE_BYTES`); other HTTP routes keep
64 KiB. Acquisition, frame, queue and response bounds remain in force; the client bounds
its response and absolute read deadline. A publish reply waits up to `PUBLISH_REPLY` from the end of
the client's send: up to the acquisition bound before the serve has read the request, then
`PUBLISH_COMBINE` (four decoder deadlines) because the serve combines this device's own
tail before it answers, then a response interval. The serve publishes no combine later than
`PUBLISH_COMBINE` after it has read the request; a later combine is abandoned without
publishing. A reply is therefore never followed by a change to the page it reports, and an
immediately following write is prepared against the page it was told about.

An IPC failure never falls back to an offline writer, changes the operation identity or
resends. A failure before the request was fully written is a plain `COLAB_UNAVAILABLE`:
the server acts only on a complete body. A serve of another build is told apart from doubt
by the answers it gives before it reads the job: an untyped 404 (the route does not
exist, as on a serve older than CLI batches) or 413 (the body is over its cap), whether
it arrives after the body or while the CLI is still sending it (a reply that is complete by
its `Content-Length` stands even when the connection is reset or already closed after it, as
Linux does when the server closes with request bytes unread and macOS does at the writer's
half-close; an incomplete one is a failed read), and a typed
`COLAB_SERVER_MISMATCH`, which a serve answers (409) to a `LocalWrite` whose `version` is not
`LOCAL_WRITE_VERSION` (2), read before any other field. The CLI exits 1 with
`COLAB_SERVER_MISMATCH`, "Nothing was written", and the fix (stop and serve again so the
server runs the installed build, or upgrade the CLI if the server is newer); it neither
reports an unknown outcome nor resends. A wire change that an older serve cannot read bumps
`LOCAL_WRITE_VERSION` or takes a new route name. Typed refusals, a 404 included, keep their
own codes. After the request was fully sent, any other lost, malformed, mismatched or late
reply leaves the original operation in doubt, and the CLI reads one original-key status
(`page::publication_status`, read-only, with the frozen chain; no new route) from its own
store snapshot. A retained committed or rejected outcome resolves the write as above.
With none, the CLI exits 1 with `COLAB_OUTCOME_UNKNOWN` naming the original operation ID.
Unknown is observational absence, not proof that nothing was published. There is no
durable job file: a retry is a fresh preparation from a new snapshot. If the first batch
landed, an identical source is a no-op and `--expected-revision` refuses with
`COLAB_STALE_BASE`; a late commit of the first batch rechecks its own frozen base, so it
cannot double-apply over newer content.

Serving locks sync before Registration and prepares bounded transport before the
transaction. A new committed batch queues, per entry in sequence order, a `broadcast`
with the normal scoped position/envelope fields and `chains:[{deviceId,chain}]`; the chain
identifies the local author and is repeated to permit certificate renewal. The browser
admits chains on its serialized executor before envelope authentication and Worker
application. Larger envelopes use the existing reference and lazy chunk transfer, with the
chain retained on the completed broadcast. One bounded, immutable committed batch occupies
one send-queue slot per peer, with an independent lazy cursor over its entries and chunks.
Only one existing wire frame is emitted per poll; the unchanged `SEND_QUEUE_FRAMES`
unacknowledged-frame credits still pause delivery until ACK, and authority is rechecked
before every disclosure. Separate queued deliveries that overflow the queue and stalled
writes still end in `RESYNC_REQUIRED`. Exact replay and rejected outcomes broadcast nothing. Queue failure does not
undo a durable outcome. The service never receives or decodes plaintext source.

### Content publication (#1908, #1928, #1934)

`publication.rs` exports pure native content-job, original-ID outcome and proposed local IPC
codecs. They have no Store, keyring, registration, route or transport capability.
The #1928 `page::commit_publication` adapter authenticates the current root-local writer and commits a sealed
content batch and its original terminal outcome in one immediate SQLite transaction.
`page::publication_status` uses a caller-supplied current writer chain in a read-only snapshot;
it never issues or repairs a certificate. Both return exact retained terminal JSON bytes. A complete
original-key replay ignores stale effect epoch/head/base but still requires current authority.
New effects reserve outcome capacity and use a content savepoint; admitted domain rejection
rolls back all content/stream/receipt/device/time changes before recording rejection, while unexpected errors
roll back the enclosing transaction. UNKNOWN is genuine absence and is never persisted.
The #1934 `page::prepare_publication` library captures one authenticated snapshot
for its genesis issuer, owner head, epoch, cuts, devices and complete content/own projections.
It reuses isolated causal preparation and returns explicit Noop with captured base and memory
profile, or a frozen Write with one signed job, exact ordered sealed envelope packet and chain.
Noop precedes operation ID, sequence, encryption and certificate issuance; publisher-only changes
are writes. Write preparation preserves foreign state and metadata, admits combined retained tail
and new deltas, and signs through the existing private local Keyring writer. Native evidence binds
source digest, actual decoder memory profile and the exact chain hash. Preparation has no durable
effect or authority promotion; later commit rechecks current authority and the frozen base.
`page::prepare_own_publication` freezes the same job for the status action's one
own-namespace update (`kind:"own"`, see the manifest below): the commit appends each entry
to the stream namespace its kind names, and a serve broadcasts each entry as any other.
For an own job `nativeEvidence.sourceSha256` is the digest of the page's current source at
preparation: evidence only, since the job carries no source edit; `memoryLimit` is the
decoder profile of the own preparation and `chainHash` binds the writer chain as for content.
Both native single-edit and batch preparation include all own bytes in the checked whole-state
raw fastpath and use one gzip stream at the unchanged 5,000,000-byte budget. `tmt colab page write`
and the serving route above are its production callers. Browser Save still publishes through
its single-update path and does not use these types; `tmt colab threads resolve|reopen`
publish an own-kind job; there is no durable caller recovery.
Syntax and signature success neither authorizes effects nor proves a retained terminal outcome.

All new DTOs use camelCase, reject unknown/duplicate fields at every nested boundary, and
reject explicit null for optional `nativeEvidence`. IDs/counters/hashes reuse the model's
canonical UUIDv4, space ID, positive-u64 decimal, lowercase hex32 and base64url rules.
The manifest is `{version:1,operationId,spaceId,pageId,epoch,streamId,kind:"content"|"own",
membershipHead:{revision,statementHash},baseRevision,entries,packetBytes,packetHash}` plus
optional `{sourceSha256,memoryLimit,chainHash}` native evidence. `baseRevision` is `v1:` plus
lowercase hex32. Each entry is `{namespace,seq,envelopeHash,envelopeBytes}` with
`namespace` equal to the manifest `kind`; a job never mixes namespaces;
byte/count fields are positive unsigned JSON integers. `SignedJob` is `{manifest,signature}`.
The packet is the exact concatenation of original envelope JSON slices in entry order,
without re-encoding. SHA256 of those raw bytes is `packetHash`; `envelopeHash` is the model's
separate envelope hash domain. Every strict envelope must be an update in the namespace named
by `kind` ("content" or "own") and match the
manifest space/page/epoch/device/membership revision/sequence/length/hash. Sequences are
contiguous with checked overflow; later `prevHash` values match the previous envelope hash.
The first previous hash retains model syntax; the native publication transaction uses the
shared append owner to compare it with the actual admitted device head.

The signature input is the existing four-byte-big-endian LP frame over, in order:
`tmt-colab-publication-v1`, ASCII `1`, operationId, spaceId, pageId, epoch, streamId,
ASCII `kind` (`content` or `own`), membership revision, statementHash ASCII, baseRevision ASCII, entries blob,
packetBytes decimal ASCII, decoded packetHash32, native evidence blob. Entries blob starts
with u32-BE count then concatenates each entry's LP `[namespace (equal to `kind`),seq,decoded envelopeHash32,
envelopeBytes decimal ASCII]`. Native blob is byte0 when absent, or byte1 followed by LP
`[sourceSha256 ASCII,exact decoder memoryLimit spelling,decoded chainHash32]` when present.
`jobDigest` is canonical base64url32 of SHA256(signature input). Job and every envelope
signature are verified with the caller-supplied admitted public key using the strict model
helpers; manifest data never selects an authority key.

Entries are 1..decoder::WRITE_TAIL_UPDATES; each envelope JSON is at most limits::UPDATE_BYTES,
each decoded ciphertext minus its 16-byte tag is at most decoder::UPDATE_BYTES, and their
sum is at most decoder::WRITE_TAIL_BYTES. For N entries the packet cap is
`min(N*limits::UPDATE_BYTES,((WRITE_TAIL_BYTES+N*2048)*4/3)+N*2048)` with checked arithmetic.
Entry lengths sum to packetBytes and the complete actual packet length; trailing bytes reject.
SignedJob and outcome JSON are at most 64 KiB; chain bytes at most 16 KiB. The standalone local
write JSON bound is `4*ceil(MAX_PACKET_BYTES/3)+65536+4*ceil(16384/3)+2048`, where
MAX_PACKET_BYTES uses N=WRITE_TAIL_UPDATES. These are codec bounds, not increased runtime
acquisition/route/source limits or final whole-state gzip acceptance.

`JobKey` is exactly `{operationId,jobDigest,spaceId,pageId,originalEpoch,streamId}`.
Outcomes always bind that complete original key: `{status:"unknown",key}`;
`{status:"rejected",key,code}` where code is exactly COLAB_STALE_BASE, COLAB_CAPACITY,
COLAB_PAGE_INACTIVE, COLAB_STATE_MISSING or COLAB_STREAM_GAP; or
`{status:"committed",key,count,finalPosition:{seq,envelopeHash},committedRevision}` plus
optional complete nativeEvidence. A supplied expected job pins count/finalPosition/evidence;
committedRevision uses the same v1 token grammar. Variant-inappropriate fields reject.
Unknown is observational absence of a retained terminal outcome, never proof of no effect.

The local write DTO `{version:2,action:"write",signedJob,packet,chain}` is served at
`/.tmt/colab/local/page-publish`; `{version:2,action:"status",key}` stays reserved and
uninstalled, because the CLI reads status from its own store snapshot. Write packet/chain are canonical base64url exact bytes;
write requires complete native evidence with chainHash equal to SHA256(raw chain), plus job
and envelope signatures under the supplied key. Chain issuer/certificate/local-keyring
admission is performed by the native library adapter. Status is bounded 64 KiB original-ID lookup,
with no write fields or new effect. Version 1 is rejected by these codecs alone.
No-op creates no job/ID/sequence; own/checkpoint/HTML/asset jobs are unsupported. Schema 6 scopes
content outcomes in the existing globally unique owner_operations ledger without changing
legacy rows. Terminal identities and exact outcome/digest/scope/identity bytes share the
existing page count/byte budgets across epochs; exact replay adds no charge or page time.
There is no expiry, eviction or pending UNKNOWN row. Browser Save/Writer adoption,
recovery/fold barriers and gzip parity remain unintegrated.

## Plaintext page export (#1309)

Export v1 emits `page.html`, `conversations.json`, `conversations.md`, one
`attachments/<attachmentId>` file for each attachment it could read, and `manifest.json` (see
[Conversation export](#conversation-export-1574) and [Attachments in an export](#attachments-in-an-export-1867)). Native captures one
owner-authenticated read snapshot through its isolated decoder; the mounted
browser captures one admitted committed projection through its connection executor.
HTML is the exact admitted UTF-8 source, including CR/LF, Unicode and NUL; export MUST NOT
inject renderer CSP/bootstrap, normalize source or execute it. The title, epoch
and verified membership head MUST belong to that same snapshot. A later write
cannot change an already captured bundle. This head is locally verified, not a
claim of globally current membership.

The manifest is UTF-8 JSON with these fields:

| Field               | Value / meaning                                                                                                       |
| ------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `format`            | `tmt-colab-page-export`                                                                                               |
| `version`           | JSON integer `1`                                                                                                      |
| `spaceId`           | Pinned space ID                                                                                                       |
| `pageId`            | Exported page UUID                                                                                                    |
| `title`             | Exact admitted title                                                                                                  |
| `originalAuthor`    | Optional bounded creation display label, omitted when unknown; never identity proof                                   |
| `publisherAgent`    | Optional latest CLI publisher display label, omitted when unknown                                                     |
| `creationRecipient` | Optional complete `{machineId,agentId}` creation preference; omitted when absent                                      |
| `exportedAtMs`      | Safe-integer UTC milliseconds                                                                                         |
| `membershipHead`    | `{revision, statementHash}`; decimal revision, lowercase hex SHA-256                                                  |
| `epoch`             | Current snapshot epoch as canonical positive decimal text                                                             |
| `plaintext`         | `true`                                                                                                                |
| `discussions`       | `{included:true, scope:"current-epoch", format:"tmt-colab-conversations", version:1}`                                 |
| `attachments`       | Always present, possibly empty: one row per attachment, see below                                                     |
| `files`             | `page.html`, `conversations.json`, `conversations.md` as `{name, sizeBytes, sha256}`; bytes and lowercase hex SHA-256 |

Export copies the optional creation preference from the same captured admitted view
before asynchronous work; it does not acquire or infer a new recipient.

The manifest MUST NOT list its own digest: that would be circular. The CLI result
names the resolved full `pageId` and lists all four files with their byte sizes and SHA-256. Export contains no roots,
wraps, bearer seeds, sessions or private keys. It is a readable copy, not an
import/backup format; retained history and snapshots are not promised as portable files. Raw own records, earlier revisions and other epochs are
not exported.

`tmt colab export <page> [--dir <destination>] [--json]` reads existing local
state only. It MUST NOT initialize missing state or run migrations. The
current native fold refuses inactive pages, so archived pages fail
with `COLAB_EXPORT_INACTIVE` and the explanation "archived or deleted pages
cannot be exported yet"; deleted operands fail earlier with `COLAB_PAGE_DELETED`.
Archived exports are not supported; deleted pages remain denied.

The destination names an existing parent directory, defaulting to the current
directory. Export creates a fresh UUID-named 0700 subdirectory with regular
0600 files. The user-selected parent may contain symlink aliases: resolve it
once to a canonical directory, then use no-follow descriptors and report that
canonical path. Created entries MUST NOT follow symlinks or replace existing
entries; parent traversal is refused and source/title MUST NOT choose paths.
Private staging uses exclusive files, byte/digest checks and sync before
descriptor-relative, create-only
publication into an exclusively reserved directory. `manifest.json` publishes
last. Publication rechecks the canonical destination path and directory/file
identities and MUST NOT use a replacing rename or follow symlinks. On failure, clean only this invocation's
checked staging; preserve foreign entries and any partial output. Report the
original canonical partial destination in the human error and JSON
`error.partialDirectory`; if an ancestor moved, the reported path is where
publication began, not a claim that the files remain reachable there.
A returned success means all four files were published and staging was removed;
this is not a crash-recovery guarantee.

The CLI human disclosure and successful JSON `disclosure` say exactly:
"This creates an unencrypted copy of the page. Anyone with these files can read it."
The mounted browser uses a trusted-parent Export panel and the same disclosure.
Capture MUST copy committed source/title, pinned space/page, current epoch and
verified owner head together through the existing connection executor. Await a
ready connection and refuse closed/blocked bindings, missing admitted keys/heads
and inactive pages. A source textarea draft, sample adapter or renderer message
MUST NOT supply this bundle. Source changes after capture cannot alter either file.
Browser serialization MUST match native field for field and byte for byte for
identical inputs, including field order, lowercase hex hashes and decimal strings.
The shared `vectors/export-v1.json` fixture injects `exportedAtMs` and pins those
exact bytes, with intentional Unicode/control-byte source and title data.

Preparing exposes no download. A ready panel offers one explicit trusted-click
request per file; it identifies partial requests and allows repeated requests
of the same frozen bytes. A requested download is not confirmation that a file
was saved; the user checks browser downloads. Failure exposes no new download
request. Native create-only filesystem and permission guarantees do not apply
to browser-managed downloads. Parent-owned Blob URLs use attachment filenames
`page.html`, `conversations.json`, `conversations.md` and `manifest.json`, never source/title paths. Revoke each URL after
bounded download handoff and all outstanding URLs on close/navigation or blocked
binding cleanup. The renderer receives no export handler, URL or capability.
Archived browser export is not supported; deleted pages remain denied.

### Attachments in an export (#1867)

`attachments` lists, in order, the page's document attachments in list order and then the
live comments of the exported epoch (not deleted, written by the sender, of this page)
ordered by writer, message ID and numeric revision, each with its attachments in list order.
Each row has exactly these fields in this order: `attachmentId`, `source` (`document` or
`message`), `reference` (the exact selector, with its keys in the order the
[storage proposal](storage-v1-proposal.md) gives them: a document reference carries the page
revision the export captured), `filename` and `mediaType` (display only, never a path),
`plaintextBytes` (decimal text), `state`, then `reason`, `sha256` and `file` where they apply.

Each file is read at export time under the authority in force then, by its own `reference`,
so a page, sharing or epoch that moved leaves that row unavailable instead of reading another
version. `state` is `included` with the plaintext `sha256` and `file: "attachments/<id>"`
(names come from the ID, never the filename); `missing` when this page no longer holds the
reference; or `unavailable` with `reason` `denied` (access ended), `changed` (the disclosure
moved while it ran), `too-large` or `unavailable` (storage, the channel, no serve, or bytes
that disagree with the descriptor). A row that is not `included` has no `sha256` and no `file`
and nothing of the object is disclosed. One export includes at most 64 MiB of attachment
bytes and spends at most 120 s reading: later rows are `too-large` and `unavailable`
respectively, and are not requested.

Native export asks the running serve for each file over the `attachment-read` route; the CLI
never opens storage or a backend, so with no serve every row is `unavailable`. The mounted
browser reads through its existing admitted read, then offers each included file as a download
under its own filename beside the page files, and lists the other rows with their outcome.
Both build the same rows and `manifest.json` bytes (`vectors/export-v1.json`); the file list
published is the three page files, the included attachments in row order, then the manifest.
Archived and deleted pages stay denied as above.

### Attachment read command (#1867)

`tmt colab attachment read <page> --reference <file> [--output <directory>] [--json]` writes one
attachment into a new private UUID-named subdirectory of `--output` (default: the current
directory), created the way export creates its directory: `attachment.bin` then
`manifest.json` last, regular 0600 files in a 0700 directory, no symlinks, no replacement,
only its own staging cleaned. `--reference` is the exact `reference` of an export manifest row
(at most 2 KiB). The manifest is `{format:"tmt-colab-attachment-read", version:1, pageId,
reference, file:{name,sizeBytes,sha256}}`; the result lists the directory, files, size and
digest and never the bytes. Authority is the owner-only socket of this data root, not a
caller-named agent or device: the serve checks the reference against the current page, access
and epoch, reads the bytes through its established object channel and verifies them before
answering. With no serve, or no established channel, it refuses as unavailable.

The serve answers `POST /.tmt/colab/local/attachment-read` with body
`{version:1, page, selector}` (strict fields; the selector as in `--reference`). A request
carrying a Remote context or event header is denied. A success is the exact plaintext
(`application/octet-stream`, at most 8 MiB) read within 15 s of the request; a refusal is the
JSON error of the publish route with the page fault code (`COLAB_DENIED`, `COLAB_STALE_BASE`,
`COLAB_STATE_MISSING`, `COLAB_UNAVAILABLE`, `COLAB_INPUT_INVALID`, `COLAB_SERVER_MISMATCH`). A
read has no effect, so it is never retried and any doubt is unavailable.

### Native attach command (#2291)

`tmt colab attachment attach <page> <file> [--type <media-type>] [--json]` adds one file to a
page's document attachments, authored by this data root's local writer; `attach <page> --resume
<slot>` finishes an earlier attempt. Document attachments only: a message attachment needs a native
message author, which does not exist yet. It needs the running serve, an established object channel
and a page the local writer may edit now; an archived or deleted page refuses `COLAB_PAGE_INACTIVE`
and a revoked writer `COLAB_DENIED`, before anything is staged. The file is a regular file of the
caller opened without following a link, at most 8 MiB, checked before and while it streams. Its name
is the label and `--type` (default from the extension, else `application/octet-stream`) an inert
media-type label.

**Slot.** `POST /.tmt/colab/local/attachment-stage` with `{version:1,page,filename,mediaType}`
opens a slot: a random 32-hex ID and a private 0700 directory under the serve's root, answered as
`{slot,path}`; no route accepts a path. The CLI prints the slot to stderr, copies the file into
`path` (create-new, 0600, bounded while streaming) and reports its SHA-256 to
`POST /.tmt/colab/local/attachment-attach` with `{version:1,page,slot,sha256?}`. The serve re-hashes
the copy, seals it under the page's current epoch as the local writer (the descriptor and object of
a browser upload), and writes the ciphertext, descriptor, base and a fresh transfer ID to the slot
before the first request leaves, removing the plaintext. Both routes deny a Remote context or event
header.

**Upload and publication.** The serve's native owner admits `LocalExtension` only, for the upload
methods of that one frozen original and the verify reads of its committed bytes: the creator is the
local writer, the source a document and the page writable at the captured base (a discard needs a
writable page, not the base). It asks `status` first, begins only an unobserved transfer, resumes at
`nextIndex`, reads the committed ciphertext back and authenticates it, publishes the creation proof
and then the `meta.attachments` change bound to the sealed source digest, each through the single
writer. Progress is read from the page and the backend, never remembered, so a retry never uploads
or publishes twice. A page edited after sealing, at any step before the list is
published, is a terminal `COLAB_STALE_BASE`: the serve discards the uploaded original, disposes the
slot and lists nothing. `--resume` of that slot is refused; the next attach opens a new slot and
never re-authors silently.

**Recovery.** A reply lost after the serve started is retried only by explicit `--resume <slot>`,
never by a new slot for the same file. One attach runs per slot at a time; a finished slot keeps its
descriptor, with no file or ciphertext, for 10 minutes so the retry returns the same attachment.
Slots that never recorded a transfer go after 10 minutes and started ones after 25 hours, after the
original is discarded. At most 16 unfinished slots exist.

Success is `{pageId,slot,attachment:{attachmentId,descriptorHash,filename,mediaType,plaintextBytes,
reference}}`; `reference` is the exact selector `attachment read` takes. A refusal is the JSON error
of the publish route with the page fault code.

### Rekey after an epoch advance (#2293)

`epoch.advance` always commits first and is unchanged: narrowing never waits for a re-seal, and no
failure of one reverts it. Its baseline still carries the document's descriptors, which name
objects sealed under the old epoch. The serve then re-seals each one under the current epoch, and
only a descriptor whose `epoch` differs from the page's waits for it, so the work is read from the
page and never remembered.

**Window.** Between the advance and the swap the document references old-epoch objects. A holder
of the revoked key can read those bytes only with raw access to the object store, and only until
the swap and until the old object is deleted, which needs the Remote delete operation of #2294.
Until then the old object stays and is charged; this contract makes no erasure claim.

**Swap.** The serve reads the old attachment through the root-local read, stages the plaintext in
a slot that records the attachment it `replaces`, and runs the native attach pipeline: seal under
the current epoch as the local writer, a new `attachmentId`, upload, creation proof, then one Save
whose `DocumentChange` removes the old ID and sets the new descriptor, bound to the source digest.
Readers therefore see the old complete reference or the new complete one, never a half swap. A
descriptor is swapped inside the baseline neither now nor later: its creation proof is an own
record of the new epoch and cannot exist before it.

**Triggers and bounds.** One worker runs at serve start, after any management change, when an
object channel is established, and after an unfinished pass on a backoff that starts at a minute, doubles to an hour and starts over on any of the other triggers. A pass takes pages whose epoch
is past 1, at most 4 attachments per page and 120 s per page; the rest wait for the next pass.
Quota exhaustion, a lost channel, a crash or a serve stop leave the old reference, and the next pass
resumes the slot's frozen transfer ID without a second upload. A page that moved meanwhile, an edit
or a further advance, is a terminal stale base for that attempt: the original is discarded and the
swap is derived again from the new page, at most three times per pass, never over the edit. An
attachment removed meanwhile is left removed.

**Status.** Each Files row of a page whose epoch is newer than its file says `Securing after a key
change. Still available.`; the line is derived from the page and has no state of its own.

### Browser page title hints (#1564)

The paired owner browser uses accepted Live folds as its only title source. Home
rows and the share/manage dialog show the last known title; the current page also
updates the parent tab title. UUIDs remain routing identities and appear under
Details. Without a usable title, chrome shows “Untitled, not opened in this browser
yet”. Opening the page replaces a stale or missing hint with its verified folded
title. Home never opens content or a decoder just to obtain labels.

These are browser-local display hints, not membership, policy or access evidence.
The Colab keyring owns a non-extractable AES-GCM-256 key per device in its existing
IndexedDB store. Title records contain only a fresh 12-byte nonce and ciphertext;
AAD is the JSON array `["tmt-colab-title-cache-v1", space, device, page]`. Record
lookup is likewise scoped to space/device/page. The title shares the existing
fold projection bound. Cached title plaintext and this local key never enter discovery,
routes, logs, the renderer, or the decoder Worker. No native schema, title endpoint,
Remote certification or network protocol is added.

Cache absence, corruption, scope mismatch or storage failure falls back to the
unopened label without denying page reading or management. Only publication of an
accepted Live fold may update the cache, through the current mounted registration
and tab lifetime. Reload can read the persisted encrypted hint; a different paired
device or browser profile has its own cache. Leaving a page restores the home tab
label. Local samples retain their existing source and title behavior.

### Conversation export (#1574)

The two companion files are frozen from the same captured snapshot as `page.html` and
hashed in the manifest; the manifest is published last. They carry the page's authorized
discussion for the **current epoch only**: verified threads, comments and Ask
conversations, no raw own records and no earlier revisions. Native capture retains each
authenticated writer's decoded `own` projection and that writer's historical signing key
from its cut-admitted envelopes while folding; the browser reuses its committed `own`
state and `ownSigningKey` inside the existing connection-executor capture. Both then apply
the same rules; no new decoder, parser or Remote fetch is involved. The vector
`vectors/export-v1.json` (generated by `vectors/export-reference.py`, an independent Python
oracle) pins the exact bytes of both files and the manifest for native and browser tests.

**Projection.** A writer without a historical key contributes nothing. A record joins only
inside the stream that carried it: `senderDevice` must equal that writer, and `spaceId`,
`pageId` and `epoch` must equal the capture's. A logical thread or comment is the newest
record of consecutive revisions starting at 1; a gap, a changed thread reference or a
resurrection after a tombstone yields nothing. Tombstones are exported (`deleted:true`,
empty comment body, null anchor). A comment appears under the thread it references only
when that thread was exported. An Ask is exported only when its signature verifies under
its writer's key and its operation, sender, space and page match; its state records, in
revision order, must follow the allowed ledger transitions with one request ID, and its
reply must belong to an `accepted` state with the same request and agent. Any
disagreement drops that whole Ask. An Ask with no state records is exported as `uncertain`.
Names (`deviceName`, `agentName`) and `at` times are labels asserted by the writer; stored
replies are writer-authenticated reports, not agent signatures.

**`conversations.json`** is compact UTF-8 JSON in exactly this field order (no trailing
newline):

```text
{format:"tmt-colab-conversations", version:1, spaceId, pageId, title, epoch,
 membershipHead:{revision, statementHash},
 threads:[{writer, id, revision, anchor:null|{exact,prefix,suffix}, resolved, deleted,
           deviceName, at,
           comments:[{writer, id, revision, deleted, body, deviceName, at}]}],
 asks:[{writer, operationId, deviceName, agentName, agent, machine, thread, messageIds,
        issuedAt, expiresAt, message, state, reason, requestId,
        reply:null|{requestId, agentId, body}}]}
```

`membershipHead.statementHash` is lowercase hex, like the manifest. `at` is the stored
decimal string; `issuedAt` and `expiresAt` are JSON integers. A thread with an effective
status action also includes `status`, the immutable action fields plus `ref:{writer,id}`
and causal `depth`, in the same declared field order as the native/browser status
vectors. It is omitted for legacy-only state. The effective action's admitted
pre-ledger failure records, when present, are included as `notifications` in
operation-ID byte order. Failures with a missing/unauthorized action or an
operation outside its frozen recipient list are inert. Normal delivery outcomes
remain in the existing Ask ledger. The Markdown reading includes the status
actor/time labels and these failure reasons from the same frozen view. Threads are ordered by
`writer + ":" + id`, comments inside a thread likewise, asks by `writer + ":" +
operationId`, all by byte order (never a locale comparison). Bodies and messages are exact
UTF-8, including controls. Order never depends on `at`.

**`conversations.md`** is a plain reading of the same data and is deterministic: a
`# Conversations` heading, a header list (page title, page ID, space, epoch, head), one
line stating that names and times are writer-asserted labels, then `## Threads (n)` and
`## Asks (n)`. For reading, threads are ordered by their `at` and comments by theirs, asks
by `issuedAt`, each with ties broken by the `writer:id` key; the number of the thread,
comment or ask follows that order. A time renders as `YYYY-MM-DD HH:MM:SS UTC` before
year 10000 and as `unix ms N` after. Display text is untrusted, so: a label (page title,
device or agent name) renders as an inline code span whose delimiter is longer than any
backtick run inside it (padded with one space when it starts or ends with a backtick or
space; an empty label renders `(none)`); a body, quote, message or reply renders in a
fenced block whose fence is at least three backticks and longer than any backtick run
inside. In both, controls other than LF and TAB (for labels, LF and TAB too), DEL, C1
controls, U+2028/2029, U+200E/200F, U+202A-202E, U+2066-2069 and U+FEFF render as
`\u{hex}` (lowercase, no padding), so display text cannot forge a heading, a fence or an
attribution. The JSON keeps exact bytes; the Markdown is for reading.

A bundle whose two conversation files together exceed 8 MiB fails with
`COLAB_EXPORT_TOO_LARGE` (browser: `EXPORT_TOO_LARGE`) instead of truncating. Archived
pages stay denied with the other export denials, because the fold has no archive-readable
path yet (see above); deleted pages stay denied. A failed capture or publication exposes
no download and follows the create-only rules above.

### Trusted browser space management (#1308)

The paired owner app exposes sharing/history, member add/remove/role, link
create/remove/Reset and epoch advance in parent chrome. Home and page chrome open
the same dialog; local samples remain read-only. Home defaults to active pages,
offers an archived filter, and provides retention days/forever, archive and explicit
delete confirmation. There is no unarchive action. Policy and complete recipient
assignments derive from the contiguous verified owner log, never discovery labels.

Metadata catchup reuses bounded frame/chunk assembly and owner-log admission,
then closes before content decoding. Baseline ciphertext can be consumed as part
of wire framing but is never decrypted or materialized for management. A failed
preview does not remove home management access. Retention defaults to 30 days.
Chrome presents the native durable last-update/expiry display hints as relative
retention time, with the absolute local date on hover. The hint stays inside the
page card under its status badge. The seven-day warning and the expired state
add a waiting mark and body text color; ordinary hints use dim text. The copy is the
CLI's lowercase wording (`expires in 6 days`, `expired 2 days ago`, `kept forever`,
`expiry starts after the next edit`, `expiry beyond the supported range`); the
local-copy note is separate (the dialog's retention form and the CLI footer). The collapsed Details line includes the local last-edit date.
Local expiry never automatically deletes data or ends access.

Confirmation discloses shared/current history scope, the 64-epoch limit, editors'
script power, renderer self-navigation limits and separate Remote device/agent
grants. Narrowing and Reset identify affected pages and explain revocation and
rotation; previously public content cannot be made private again. Requests freeze
exact selections, initiating context, ID, revision, expiry, signature and any new
seed before send. Only explicit, unexpired byte-identical retry is offered after
uncertainty; stale/expired requests require fresh review. No reload, reconnect,
timer or tab close submits a management request.

The mounted tab lifetime fences preparation and POST. Current registration owns
each prepared request; a replacement session refuses old views and mutation
retries but may verify an earlier acknowledgment read-only. Management does not
open a separate Remote session. Closing one tab closes only its metadata sockets
and prevents new work; already-started effects retain their existing outcome.
An acknowledged or uncertain policy change closes stale Live/writer/Ask state; subsequent
refresh never dispatches an Ask again.

A transport acknowledgment is followed by verification of its exact signed-log
revision/hash and requested change, tolerating later commits. Failed refresh stays
acknowledged/awaiting verification. Archive verifies through the same owner-readable
page. Delete verifies discovery absence plus the signed delete change through
another readable page; the frozen initiating context survives removal. Last-page
deletion stays acknowledged/awaiting verification when no readable context supplies
signed evidence. A post-delete DENIED/1008 close is neither a rejection of the
acknowledged change nor completion evidence. Verification retries only read.

New link IDs/seeds appear only after verification with an explicit copy action;
they are never persisted or logged. The reader browser flow is separate, so this
dialog does not fabricate a reader URL. Lost seeds require Reset. Delete confirmation
identifies the page, cessation of access and removal of backend ciphertext, and
states that copies already made cannot be recalled.

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
epoch admission, owner-only socket admission, unauthenticated-upgrade denial and
bounded socket cleanup; Host/Origin/rebinding denial is remote's door.
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
