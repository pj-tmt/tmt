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
names the sections that move to remote. Extension contracts and vectors remain under this extension.

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

Warning-only page expiry never deletes completed objects. After a committed page delete,
Colab removes its derived object namespace through the negotiated object channel. The
native callback owner rechecks the retained signed deletion, independently of the deleted
payloads; browser peers cannot relay namespace removal or usage calls. Removal is idempotent.
Removal frees the page's file data; a small fixed record per retained identity and the
namespace fence remain, and the capacity charge keeps counting them. Only a definite
`Removed` means file data is freed: CLI delete reports `storage` as `freed`,
`pending` (no established channel), `unsupported` (a v1 channel), or `unknown` (no definite
removal answer). A bounded rotating sweep in the serve's maintenance worker retries retained
committed deletions on channel establishment/replacement and unfinished maintenance passes;
an offline delete remains pending until that channel exists. Neither cleanup nor a lost
cleanup reply reverses the committed page deletion.

Both removal and usage check negotiated `Bus::supports` before requesting them; unsupported
methods write nothing and show `unsupported_hint()` verbatim. Capacity refusals query the
affected namespace's `objects.usage` through a native callback owner, carrying `chargedBytes`,
`entries`, `activeUploads`, and `retainedIdentities`. The displayed amount is a conservative
allocation charge, not disk or billing bytes; unknown usage displays no guessed amount.

Paired local browsers open archived pages through read-purpose catch-up. A newly committed management
archive ends that page's open sync connections with
`RESYNC_REQUIRED`; their existing replacement path re-admits the read-only page.
Writes, Ask, upload and thread status actions are removed; native admission still
refuses their stale captures. Retained unsent drafts remain selectable with Copy.
Archived export stays refused. An owner can restore an archived, non-deleted page with
`page.unarchive`. Restore preserves the page ID, source, history, epochs, keys, recipients and
sharing policy; deletion remains terminal. It restarts the retention clock by atomically
updating `pages.last_activity_at_ms` with the signed statement, page projection and operation receipt.
`pages.last_update_at_ms` remains the content-edit time; restoration does not change it.
An exact committed retry returns its original receipt without restarting that clock again.
A restore also ends existing page connections with `RESYNC_REQUIRED`. Replacement sessions
must verify the current owner log and permissions before writes become available; retained
drafts never send automatically. Cloud publication resumes through the existing activation
path, publishing the open state and confirming its refreshed expiry through the normal cloud
write. A local restore does not fabricate a confirmed cloud expiry.

### Attaching files in Chat and annotation conversations (#1854)

The owner browser's shared composer can attach files to a Chat, annotation or reply
message. Picking, pasting or dropping only adds an inert local chip: nothing is stored,
sent or prepared, an Ask is never created and the message text is neither read nor
replaced. Local pending raster previews recheck already-picked bytes through
`previewType` and the pixel bound, without reading a Remote object or preparing
an upload. The draft caches display URLs until removal, publication or disposal;
late preview/file reads cannot repopulate a replaced or disposed draft.
The [attachment presentation rule](../../../design/gui-style.md#message-attachments)
owns pending row and panel overlay appearance. Each panel registers its mounted
composer's existing draft and current attachable fence; replacement/unmount
invalidates stale registration. The local composer drop and full-panel drop
use one add route, never two. Readable count/size metadata is checked during
drag; protected native file data is rechecked as a complete batch on drop.
Rejected batches add nothing. A blocked or archived panel cannot add or publish.
The explicit Send is the first remote effect. A message still needs text. Attachment bytes are
never part of an Ask payload; the frozen message lists their metadata (#2464): after the comment
text, a blank line and `Attachments sent with this comment on "<quote>":` (or `... in the page chat:`
when the thread has no anchor), then one line per file of the sent message in its own order,
`- [<n> of <total>] <first 8 hex of the attachment ID> <name as a JSON string> (<media type>, <n> bytes)`, then
`Read one: tmt colab attachment read <full page ID> <id>`. Placement is exactly what the admitted
records hold: the files came with this comment, and the comment sits on the thread's quoted text,
JSON-quoted and cut at 60 characters, or in the page chat. A file has no position inside the text,
and a document attachment has no anchor, so neither is invented; the page's document files are not
listed. A message that is only files shows `(no text; only the attachments below)` as its text. The
name and the quote are untrusted text: each is JSON-quoted and the invisible and direction-changing
characters U+00AD, U+200B-200F, U+2028-202E, U+2060-206F and U+FEFF are written as `\uXXXX`, so
neither can open a line or a section. No bytes, keys, seeds or object
references appear. A message without files, or a deleted one, adds nothing, so its frozen bytes are
unchanged. Earlier user turns of a Chat conversation list their files the same way, without the `Read
one` line, inside their own `User (...)` block, so each file stays with the comment it came with. The block is part of the signed final bytes (see
`vectors/send-preview-v1.json`, `withAttachments`) and counts against the composed-message cap.

Limits are the consumer limits above: 8 MiB per file, 16 per message, the backend
`config` payload bound when lower, and filenames shortened to 255 UTF-8 bytes with
control characters and path separators replaced. The stored media type is decided from
the bytes, never the name or the browser's declared type: a PNG, JPEG or WebP whose header
declares at most 16 million pixels is stored under its raster type; a `%PDF-` prefix is
`application/pdf`; nonempty valid UTF-8 without NUL or other C0 controls except tab, LF and CR
is `text/plain`; everything else (video bytes included) is `application/octet-stream`.
Text, including HTML-shaped text, and PDFs are download-only and never rendered as HTML or
raster previews. The native and browser detectors share `vectors/attachment-media-type-v1.json`.

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

Visible message raster tiles read through the admitted path for that exact message
revision, serially and without preloading hidden images. The header is parsed again with
`previewType` before a bounded PNG/JPEG/WebP becomes a `data:` thumbnail. A tile retains
its display URL within the same live binding, disclosure scope and reference lifetime,
including scrolling away and back; replacement, deletion or unmount clears it. A trusted
viewer activation may read another image of the same message. Thumbnail bounds, grid,
overflow, file-card presentation, viewer controls and alignment are defined by
[Message attachments](../../../design/gui-style.md#message-attachments).
Non-images and Files remain click-only; downloads use `application/octet-stream` through
the parent's blob-download lifecycle, revoked after hand-off and on unmount. A trusted
click selects the current binding and exact reference for the still-listed attachment ID
when its serial read starts. A missing binding or reference, or a replacement during the
read, shows a retry message; it never replays a stale read or retries automatically. No inline
video, active content, relaxed CSP or renderer capability exists.

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
are outside v1. `serve` runs in the foreground.

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
The probe also requires every frozen mixed-order signature known answer to match native policy.

### Strict signature admission

Every enrollment, statement, certificate, object, intent, wrap and pairing
signature MUST use one strict verifier. Browser admission requires A32/R32/S32,
masked Edwards y < p for both A and R, rejection of all eight torsion encodings
including sign-bit/negative-zero variants, public-point decompression for A and R,
and little-endian S < L, followed by native WebCrypto verification. The bounded
decompression guard uses only public 32-byte inputs and a fixed exponentiation chain;
signing, verification and secret-key operations stay native. Rust requires canonical recompressed non-weak public
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
rulesToken = base64url(HKDF-SHA256(linkSeed, empty,
  LP("tmt-colab-link-rules-token-v1", space, linkId), 32))
```

`rulesToken` (43 characters, unpadded) is the bearer value of the link's cloud `links` row: a
holder presents it once in its self-admission `members` create. It is derived apart from every
other value, so the cloud learns nothing else of the seed. The owner derives it when it creates
or resets the link, while the seed is borrowed, and keeps only the token in its Store until the
link ends; a link created before this derivation existed has none and needs `share link reset`.
`authority-v1.json` `linkRulesToken` freezes the oracle value.

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

| Operation        | Payload fields / semantic constraints                                                                                                                    |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `member.add`     | `memberId, role, signKey, encKey, pages`; role viewer/commenter/editor; see [history modes](#current-view-baseline-and-history-modes) for earlier epochs |
| `member.remove`  | `memberId, cuts`; remove current authority and advance affected page epochs                                                                              |
| `member.role`    | `memberId, role, cuts`; reductions commit affected streams; promotion grants no retroactive authorship                                                   |
| `link.add`       | `linkId, role, linkSignKey, linkEncKey, pages`; role viewer/commenter/editor; keys match pinned derivations                                              |
| `link.remove`    | `linkId, cuts`; revoke every device certified by the link and rotate affected epochs                                                                     |
| `device.revoke`  | `deviceId, cuts`; revoke the selected identity/sessions/grants and rotate affected epochs; a surviving link seed remains a separate bearer capability    |
| `bridge.add`     | `machineId, machineSignKey, encKey, pages`; an owner machine with the restricted bridge role, not editor                                                 |
| `epoch.advance`  | `pageId, epoch, cuts, baseline, wraps`; next epoch, exact baseline descriptor, remaining-recipient signed wraps                                          |
| `page.share`     | `pageId, mode, epoch, publishedKeys`; private/link/public; key publication only for loopback public mode; earlier epochs only under `shared` history     |
| `page.history`   | `pageId, mode`; `shared` (default when absent) or `current`; owner control only; applies to later joins                                                  |
| `retention.set`  | `pageId, days`; positive safe-integer day count or null for forever                                                                                      |
| `page.archive`   | `pageId`; hide from active lists and freeze writes                                                                                                       |
| `page.unarchive` | `pageId`; restore an archived, non-deleted page and restart its retention clock                                                                          |
| `page.delete`    | `pageId`; cease access and remove backend ciphertext; never revive through replay                                                                        |

`page.projects` and `project.manage` are not in this table: they are management requests that change
display-only owner-store state and append no statement (see [Projects](#projects-3747)).

**Page scope.** `epoch.advance`, `page.share`, `page.history`, `retention.set`, `page.archive`, `page.unarchive` and
`page.delete` are page-scoped: each names exactly one page in `pageId` and changes only that page's
policy (creating a page appends its first `page.share`). Every other operation (`member.*`, `link.*`,
`device.revoke`, `bridge.add`) is not page-scoped, even when its payload lists pages: it can change who or
what a writer is. A rule that asks whether a statement can touch a page reads its page scope from the
payload, and an operation that is not listed here as page-scoped is treated as touching every page.

The root owner is implicit, not an assignable member role; v1 has no co-owner
or signing delegation. Every log statement is produced and signed by the owner's
`tmt colab`, whose keyring alone holds the root key. Page creation allocates a page ID, initial epoch
and baseline through the owner; `create` imports source without executing it.

### Browser management requests

**Channel boundary: stays,** carried over the relay with the remote device context as sender.

The owner's enrolled browser never receives the root key. To request sharing,
member/link changes, epoch advance, script policy, retention or display-only projects, it submits a
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

The reserved owner-socket `GET /.tmt/remote/describe` implements Remote's
[module-description contract](../../../contracts/remote-channel-v1.md#module-descriptions-and-composed-shell).
It reads only the existing bounded owner catalog/fingerprint projection and fresh page-summary
cache; it starts no refresh, fold, subscription or new state. Both counts exclude archived
pages: Pages counts the remaining pages, and `needsYou` counts waiting pages, matching the
in-app Needs-you chip (two waiting items on one page count once). An empty catalog has known
zero counts. The Pages count comes from the catalog alone and stays known when summaries
are unavailable. If any non-archived summary is missing or stale, the answer keeps its rail
and menu but omits `needsYou`, never inventing zero. An unavailable catalog
projection or a count above the descriptor bound returns a typed refusal. The reply is
display-only and does not change page admission. Browser mount traffic cannot call it.

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

The app also builds the bundled page runtime (#1693) as one flat asset, `assets/colab-runtime.js`: React,
ReactDOM and htm in one script that sets `window.tmt`, followed by the unmodified Tailwind browser
script. It is an ordinary inventory file, and `/assets/colab-runtime.js` is on the anonymous list above
so a reader can load it. No page loads it until the opt-in slice ships, and the renderer CSP is
unchanged. The build pins it: `typescript/app/runtime-pin.json` holds the exact versions of React,
ReactDOM, htm and `@tailwindcss/browser` and the asset's SHA-256 and length, and `scripts/verify-runtime.mjs`
fails the app build when the bytes or versions differ, when the script exceeds 1.5 MiB, ends a script
tag, reads `process.env`, references a source map or uses `eval`/`new Function`, or when the notices lack
any of those four packages. `scripts/runtime-notices.mjs` appends htm's and the Tailwind browser
script's licenses to `THIRD-PARTY-NOTICES.txt`, since the default build bundles neither. A changed
dependency or output is therefore a reviewed `runtime-pin.json` diff.

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
`/renderer.html`, `/assets/reader.js`, `/assets/reader.css`, `/assets/reader-fold.js`,
`/assets/colab-runtime.js` and `/assets/recovery.js`, plus the native `/assets/chrome.css` stylesheet,
which is also available without an app build. They are public static bytes with no secret and no API. Every
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

Native app Vite output MUST use relative URLs beneath `/r/<prefix>/x/colab/`, with no
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

The local socket admits link sessions, which store only what their link's role allows (below), and anonymous public sessions, which are read-only.
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
revocation. A private or deleted page denies readers; archive retains native reads.
Paired local browsers also admit archived reads;
bearer reader entry is unchanged by that paired-page behavior.
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

Only hello, catchup, subscribe, scoped ack and, for a link whose role allows
it, append (with its upload chunks) are allowed, on the admitted page/epoch. A
link session stores its own originals as its link's device: the stream is the
device its chain certifies, and each object names the current membership head. Every holder of
one link derives the same device, so all of them write one stream with one key (v1: one stream per
link): two holders that send at once both seal the stream's next sequence, the serve stores the
first, and the second original is refused as `CONFLICT` with nothing written and nothing frozen, so
the stream stays open to every holder. The refused original was never stored: its sender drops it,
reconnects, and seals its next original at the stream's new head, and the browser says its edit
was not sent and keeps the draft. A lost connection is not a refusal: the exact bytes are retried.
The link's current role decides the namespaces. An `editor` link stores `update`
objects in `content` and `own` (and `own` checkpoints), a `commenter` link in `own`
only, and a `viewer` link nothing, from the first upload frame on. No link has the serve save page
text: `save`, `savestatus` and the upload chunks of a save are denied for every link, and a browser
never seals page content. An `editor` link asks for a text change with an `edit-request` record in
its own stream (Own-stream edit requests), which the owner's serve decides. Every other frame,
awareness publication, usage, compaction and every
object request (Ask, Chat, attachments), is denied for every link and for public
sessions. What a
stored original means is the fold's call as for any author: a `content` original
from a link that is not an editor, or any original from a viewer, is refused there.
Link catchup selects only
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
<door origin>/read/<linkId>#v=1&space=<spaceId>&page=<pageId>&link=<linkId>&rev=<n>&st=<hash>&seed=<seed32>&root=<ownerKey32>&role=<role>
```

The CLI prints root-relative `/read/<linkId>#...` as `readerPath`; while a door runs it
also prints the complete short link as `readerUrl`. The share dialog copies the same single
openable capability after signed-log verification. Without a door, prepend its origin.
The path names the link ID and must match the fragment; all authority stays in
the fragment, which a browser never sends to a server. No query is accepted, and the
seed appears nowhere else in any output. `rev` and `st` are the revision and canonical
base64url hash of the `link.add` statement that introduced the link: the link-device chain
must name that statement, and a wrong value only fails the server's chain check. The grammar is
strict: `v` is `1`; each of `v`, `space`, `page`, `link`, `rev`, `st`, `seed`, `root`, `role`
appears exactly once in any order; values are canonical (`space` and IDs as everywhere, `rev` a
positive decimal, `st`, `seed` and `root` unpadded base64url of 32 bytes, `role` one of `viewer`,
`commenter`, `editor`); anything else, including percent escapes, an unknown key, a link printed
before `root` and `role` existed, or more than 512 bytes, is not a reader link (the longest
form is about 300 bytes; the entry names a well-formed pre-`root` link as out of date and says to
ask the owner for a new one, without any request). `root` is the owner's root public key: every reader rejects a link
whose `root` does not derive `space`. `role` is only a hint for the one self-admission create of
a hosted reader; the capability comes from the signed link statements, and a hosted reader stays
read-only whatever `role` says. `share link add --role viewer|commenter|editor` (default `viewer`)
chooses the role the link statement carries, and `share link reset` keeps the replaced link's role.
A link whose `role` is `editor` makes the `/read` entry offer the owner page's Source drawer and mark itself
"Can edit" instead of "Read-only": a save is an `edit-request` record that the link's device seals into its own
stream (Own-stream edit requests), which the owner's serve may apply. The serve stays the authority:
the fragment's `role` only decides whether the drawer is offered, and a narrowed or removed link ends the session
as before. A viewer or commenter link never shows it.

The public `/read/<linkId>` entry removes the fragment from the address bar before any other work
(a reload therefore needs the full link again). In memory only, it derives the link keys from
the seed with the model's `link::Keys` derivation and wipes the seed. A link has one derived
device per transport, so each transport writes a stream of its own and a race on one never forks
the other: `door` (the owner's serve, through the Remote door) and `hosted` (the cloud log, written
by a holder's browser directly; Remote #3122 writes it). A device is Ed25519 signing seed
`HKDF(seed, LP(<seed label>, space, link))` and a device ID from the first 16 bytes of
`HKDF(seed, LP(<id label>, space, link))` with UUIDv4 version and variant bits set (the cloud Rules
admit only a UUIDv4-shaped stream). The `door` labels are `tmt-colab-link-device-seed-v1` and
`tmt-colab-link-device-id-v1`, which every link has always had; the `hosted` labels are
`tmt-colab-link-hosted-device-seed-v1` and `tmt-colab-link-hosted-device-id-v1`. This entry is the
door's holder and derives the `door` device. It certifies that device as a `link` issuer chain
with the link's encryption public key, the `rev`/`st` statement and a fixed validity window of
`issuedAt` 0 to `expiresAt` 9007199254740991 (liveness is the link's, never the certificate's).
Ed25519 signing is deterministic, so every open presents a byte-identical chain and the server
keeps one device row per transport however often the link is opened (`authority-v1.json`
`linkDevice` and `linkHostedDevice` freeze the oracle bytes of the two). The server never enforces
this derivation: any seed holder may still certify a fresh random device, as the revocation rules
above state. The owner derives both devices while `share link add` or `reset` borrows the seed, in
the transaction that appends the `link.add` statement, and keeps their chains as device rows beside
the link's admission token: each chain names that statement, and the rows are never registrations,
because this machine does not write those streams. `link.remove` revokes every device the link
issued. A link added before the `hosted` device existed has no `hosted` chain: its owner resets it.
The cloud snapshot publishes the chain of each live link's unrevoked devices with the writers'
chains, so whoever follows a link's streams finds them; an ended link's chains are not published
again. A link's `own` records written at the local door stay on this machine in v1: the cloud copy
of a link's records comes from the holder that wrote them there, on the `hosted` stream, so the two
transports never put two different originals at one sequence of one stream. The entry then runs the challenge and session exchange above and opens
`/sync` with the ticket subprotocol; hello names the session's `principal`. It checks that the
returned owner key equals the link's `root` (a mismatch ends access) and derives the linked
`space`, verifies the owner log and link-addressed wraps
with the owner-browser rules, and pins and stores nothing in the owner app's records
(IndexedDB, local or session storage, cookies; an editor link's writer keeps its state in memory,
for the tab). It shows the page read-only and live, with no
editor, Ask, share, export or history control. The ten-minute session ends in a reconnect with
a fresh challenge; a `DENIED`/`STALE_EPOCH` result or a 403 from the exchange ends access
("Access ended"), which is what Reset, remove, narrowing or rotation produce.

Reader access is limited to whoever can reach the loopback or Remote door and holds the link.
The CLI help and the entry say so plainly.

## Page state, roles and epochs

Each device has one signed append stream per `(space, page, epoch)`. Sequences
are contiguous from `1`; readers buffer bounded gaps and apply n only after
n−1. Identical envelopes replay without changing state; two different envelopes
for one sequence freeze and flag the stream, except the stream a link's holders share, where a
live session's second original is refused and nothing is frozen. Backends store create-only at the
scoped `(streamId, seq)` key. One browser tab owns the device's write lock with
`navigator.locks`; other tabs relay through it. CLI writes hold a file lock.

`namespace` is authenticated in the header, AEAD and signature. `content` and
`own` have separate documents and decoders; unsigned routing cannot choose a
document. The following roots are exhaustive:

| Namespace | Roots                                                                                                                                                                      | Fold and authority                                                                   |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `content` | `html` Y.Text; `meta` Y.Map containing `title` and optional `publisherAgent`, `originalAuthor`, `creationRecipient`, `layout`, `layoutReason`, `work` ([Work](#work-4050)) | Shared page document, folded from admitted owner/editor streams in the current epoch |
| `own`     | `threads`, `messages`, `intents`, `replies` Y.Maps                                                                                                                         | Separate document per writer; only that writer's signed stream mutates it            |

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

`meta.layout`, when present, is `column`, `full` or `canvas`: how Colab frames the
page (`column`: a text column of at most 1280 px; `full`: blocks may run edge to edge;
`canvas`: the page draws its own frame, and Colab applies none). A page without the key is
`freeform`, which is an absence, never a stored value: a page created before this key
existed reads as `freeform` with no migration, and no create or write can choose it.
`meta.layoutReason` is present exactly when `meta.layout` is `canvas`: one nonempty,
trimmed line of at most 1024 UTF-8 bytes without control characters, saying why the page
draws its own frame. A dashboard has neither key. Both are display and framing hints,
never authority. The browser frames from the verified, folded metadata, so a change made on
another device or by the CLI reframes an open page without a new source. Source writes
(CLI, patch, browser Save), checkpoints, compaction and epoch baselines preserve both keys
in the committed update bytes; only `page create` and `page layout` set them, and a native
or browser projection with an unknown kind, a reason on a non-canvas page, a canvas without
a reason or a dashboard with a layout is rejected.

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
the originating device may edit/delete its messages or resolve, reopen and reattach
its threads. The browser offers no whole-thread deletion; retained thread tombstones
still read normally. Cross-writer editor resolution remains planned; it
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

### Own-stream edit requests (#2452)

A link holder who cannot seal content asks the page's owner to save a change with one typed
record in `intents[operationId]`: `version:1`, `kind:"edit-request"`, `operationId` (a canonical
UUIDv4, also the key), `baseSha256` (the SHA-256 of the source the holder edited from, unpadded
base64url), `source` (the whole proposed source: UTF-8, at most 131072 bytes) and `sourceSha256`
(its SHA-256, which the decoder checks), and optionally `hunks`: one to 64 `{oldStart,old,new}`
of a Save frame's anchored hunks (`oldStart` at least `1`, `old` not empty, at most 65536 bytes of
`old` and `new` text together). No other field exists, and an absent `hunks` is never `null`. The
record is immutable, as the other typed records are: a later update can neither replace nor remove
it. In v1 one request is exactly one own update of at most the decoder's `UPDATE_BYTES`, and sealed
it is one log document of at most 393,216 bytes (the canonical fields one hosted document holds):
a request is never split, and a text that needs more is not sent through a link. The browser
refuses a source over its bound, and an update whose sealed bound is over the document, before it
seals or sends anything. The record carries no authority; the owner's serve alone decides what it does with it.
While it runs, the serve settles each request of a link device once, after the device's original
is stored (an append at the door, or an original read from the cloud, and once per page when the
page is first read after a start): the link must still be a live editor covering the page, and
the request is then saved as a browser Save with the same hunks would be, signed by the root-local
writer and naming no link. A page that moved since `baseSha256` takes the hunks as a Save does:
changes that do not overlap merge, an overlapping one is refused as `COLAB_PATCH_CONFLICT`, and a
request without hunks is refused as `COLAB_STALE_BASE`. Each request's decision (`applied`,
`conflict` or `refused`) is kept per page epoch, at most 1024 of them, and is final: a request is
saved at most once, and one that was refused is never evaluated again. Only the link's own answer
refuses a request: the link is gone, revoked or expired, is not an editor, or does not cover the
page. A request whose attempt could not finish, or that no answer could be had for (a lock, a
clock or the store failing), stays undecided and is tried again. At 1024 decisions for the epoch
the serve saves nothing more, and further requests wait undecided. If the serve stops between
the commit and the decision, the retry finds the change in place (`unchanged`, so `applied`) or
hunks that no longer match (`conflict`); the change is never applied twice. A link that ended
before the decision leaves its stream out of the page, so its request is neither seen nor
applied, and its holder's session ends with the link. The holder learns the outcome from the
page itself, and the browser says no more than that: once the request is sent, a page that shows
exactly the proposed text is applied; a page that shows any other text (another edit, or a merge)
is reported as changed, because part of the request may be missing; a page that has not changed
after ten seconds is reported as waiting for the owner's Colab, and a request that is refused or
never settled reads the same. Both keep the draft, offer to copy it, and never send the request
again. A hosted editor link is answered the same way, by the page it shows, and an edit too large
for one request reads the same too; only what the browser can say before it has a request to
answer differs: not sent (refused before any effect, or an occupied ID because another holder of
the link sent first) and unconfirmed (an uncertain send, never sent again).
`vectors/edit-request-v1.json` (generated by `edit-request-reference.py`) pins the grammar and
the bounds for browser and native; a string `{"fill":text,"count":n}` there stands for `text`
repeated `n` times.

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

A comment's optional `sequence` (#2442) is its Lamport position in the thread: a canonical
decimal string from `1` to `9007199254740991` (2^53 - 1). A new comment takes one more than
the highest sequence the writer has admitted in that thread, tombstones included (capped at the
maximum), or `1` for a thread's opening comment; every revision of a comment, edits and its tombstone, carries the same
value, and a chain whose sequence changes is dropped like a changed thread reference. New writes
always carry it. A comment written before the field has none and reads as `0`. Comments are shown
and exported in this one order, in live updates, after reload, in Chat, in `conversations.json` and
`conversations.md`, and in the earlier conversation sent to an agent: ascending sequence, then
writer ID, then message ID, all by byte order. It is display order only: a writer can claim any
sequence, so it never grants authority, selects a revision or proves when something was said, and
`at` never decides it. `vectors/discussion-v1.json` (`sequenceCases`) pins the grammar and the
order for browser and native.

| Root/key                                       | Additional fields                                                                                                                                                                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `threads[threadId+":"+revision]`               | `kind:"thread"`, `threadId`, `anchor:null` or `{exact,prefix,suffix}`, `resolved:boolean`                                                                                                                                       |
| `messages[messageId+":"+revision]`             | `kind:"comment"`, `messageId`, `thread:{writer,id}`, `body:string`, optional `sequence`                                                                                                                                         |
| `messages[actionId+":thread-status"]`          | `kind:"thread-status"`, `actionId`, `thread:{writer,id}`, `previous:null` or `{writer,id}`, `resolved:boolean`, `actor:"person"` or `"agent"`, `agentName:null` or string, `recipients:[{machine,agent,agentName,operationId}]` |
| `messages[operationId+":thread-notification"]` | `kind:"thread-notification"`, `operationId`, `status:{writer,id}`, `reason:"RECIPIENT_UNAVAILABLE"` or `"PREPARATION_FAILED"`                                                                                                   |

**Proposal records and agent CLI (#1773, slice 1)**

A proposal is a null-anchor discussion thread whose immutable head metadata includes
`proposal:{proposalId,title,body,proposer:{machineId,agentId,label}}`. Its proposal ID
is the thread ID; it cannot be the writer's reserved Chat ID. Proposal metadata
remains identical in subsequent thread revisions, including a tombstone. Title and
label are nonempty text bounded to 200 and 64 Unicode code points respectively;
body is nonempty text bounded to 4096 UTF-8 bytes. Proposal and machine IDs are
canonical UUIDv4; agent IDs use the existing canonical Core UUID grammar. These
asserted labels and IDs do not authenticate an agent or grant publication authority.
At most 200 distinct writer/proposal pairs may be retained per page, counting
revisions and deleted proposals once, alongside the existing thread-entry limit.

`messages[actionId+":proposal-decision"]` is an immutable status action containing
all common discussion fields, `actionId`, `thread:{writer,id}`, `previous:null` or
`{writer,id}`, and `decision:"approved"|"declined"`. Revision is `"1"` and deleted is
false. Only the same authenticated owner-device provenance as resolution actions
contributes a decision. The first causal decision is final: roots precede all their
descendants, and concurrent roots choose the smallest ASCII `(writer,actionId)`
pair. Later decisions are refused by the native preparation and browser discussion
writer; descendants, orphans and cycles cannot replace a root in the fold. Clocks
and labels never choose a winner. Decision and resolution are independent; Resolve
and Reopen keep the recorded decision. Without a decision action the decision is
open. A deleted proposal ignores decision actions.

`proposal add PAGE --title TITLE --body BODY [--id UUID]` publishes the complete
record first through the existing own publication, then reads the page and appends
`<tmt-proposal data-id="<proposalId>"></tmt-proposal>` through the existing
expected-revision, lock-or-serve source publication. These are two recoverable
publications, not one transaction. New records require the public caller's canonical
agent ID and the existing `tmt remote status --machine --json` machine observation.
No Remote private state is read. A stopped or unavailable provenance lookup refuses
new record creation; recovery of an existing local-writer record uses its retained
metadata. Source placement is append only; `--after` is deferred.

The JSON result includes `proposalId` and `placed:true|false`. A failure after an ID
is allocated retains it in the error result; `placed:false` means placement was not
confirmed. After uncertainty, read `proposal ls PAGE --json` and `page read PAGE
--json`, then retry with the same `--id`, title and body. Recovery never changes
retained metadata or mints a replacement ID. It publishes only a missing record or
placeholder step, and a stale page base refuses placement. One exact generated
placeholder confirms placement; multiple copies stay detached and are left alone.
HTML is untrusted position data, never record, destination or activation authority.
A missing or duplicate placeholder leaves the authenticated proposal detached;
a placeholder without a record supplies no proposal. The trusted parent draws the
card from admitted records; labels and author HTML never choose a destination.
The shared inline renderer channel carries admitted proposal/question-group IDs and integer
heights clamped to 0–2000 px (at most 250 slots: 200 proposals and 50 question groups),
scoped to the render ID and request ID. A slot is `{id,height}` or, for an answer card,
`{id,height,before}`; both heights have the same bound. For a chip-form group (its entry in the
questions request carries `form:"chip"`) a slot is `{id,height,chip:{width,height}}`: `height` is
the open panel's, 0 while collapsed, and `chip` the chip's own integer size, 1–1000 by 1–200;
`chip` is refused for any other group and next to `before`. Compact question
blocks retain a 1px inert marker in that placeholder and render at the bounded
after spacer's top using the authored column's horizontal bounds; the placeholder never carries the
whole expanded block's height. Split logical groups request one physical entry
per question ID, while structural wrappers use their group ID. These remain
cosmetic entries under the same page-wide slot bound. The iframe inserts the after spacer
beside exactly one live `tmt-proposal[data-id]`, `tmt-question[data-id]`,
`tmt-questions[data-id]` or `tmt-review[data-id]`. A group uses its parent-selected active question for placement.
Its before spacer reserves trusted state above the authored description;
the after spacer reserves controls below it. Responses use explicit closed key sets selected by the currently requested slot kind:
`{id,top}` for proposals, or `{id,top,beforeTop,descriptionHeight,left,width}` for
an admitted question group with a requested `before` height, or
`{id,top,chip,left,width}` for a requested chip-form group, where `chip` is the closed
`{top,left,width,height}` page rectangle of the frame's inline placeholder (finite, nonnegative,
width and height positive, within the cosmetic bound), `top` the open panel spacer's top and
`left`/`width` the content box of the block that holds the chip, with the bounds of an answer card.
Answer geometry is not admitted for a proposal or an unrequested group. `left` and `width` are finite
iframe-viewport CSS pixels measured from the actual authored group (or standalone
question) bounds; `left` is nonnegative and `width` is positive. Their sum must fit
the current frame width, each value must fit the existing 1,000,000px cosmetic
rectangle bound, and the resulting horizontal bounds must fit the parent
viewport. Invalid claims are rejected, not normalized into admitted placement.
Top values are
finite and within 0–1,000,000; `beforeTop` is at most `top`, and `descriptionHeight` is
finite within 0–2000. Unknown,
missing or duplicate IDs receive no slot. Source replacement retires all slots;
removed or moved spacers and inner-scroll fallback detach cards into Comments.
The parent leaves a transparent description gap between its trusted bands; authored text
stays in the frame. Parent ResizeObserver measurement coalesces to one update per animation frame and
only sends changed integer heights. Height requests retain admitted positions and
composer focus until a valid reply detaches a slot. Changed normalized parent-owned slot dimensions restart the viewport-growth baseline; identical requests and frame reports do not.
Cosmetic top convergence stops after three
changing rounds per trigger until the next trigger. Malformed, duplicate, stale
or over-budget reports are ignored; geometry grants no publication, dispatch or
focus authority.

Changing the parent-selected question requests current cosmetic placement on the
existing ordered port. Controls stay detached until a current response arrives;
old description geometry never authorizes a new Send. Compact rows retain
viewer drafts while closed. Digits inside opened controls can update local
choices, while Send requires current placement. The trusted block uses the
admitted authored container bounds. Viewport narrowing retires bounds that no
longer fit until a current reply reattaches the block. The existing inline-slot
owner measures its complete height. Floating Chat/Annotate controls clear visible,
attached answer footers using parent-window bounds, and hide temporarily when no
room remains; header entry points stay available. Placement and visibility never
change answer, recipient, draft or publication admission.

Approve/Decline is an explicit trusted parent activation. An in-flight fence and
live proposal/finality admission precede decision publication. The immutable
decision is the commit point. Within the same activation, the parent publishes
`Approved: <title>` or `Declined: <title>` as an ordinary comment and prepares and
sends the existing Ask with that real thread/comment correlation, frozen page
link and the proposer's exact machine/agent UUID pair. Current directory admission
still applies; names never select another agent. Failure retains the decision and
shows existing Ask failure, held or uncertain state. Recovery is an explicit
existing Ask recheck or resend; reload, sync and rendering never resume sending,
roll back the decision or create another automatic request. Follow up opens the
shared composer inside the card under its history, pre-addressed to that UUID
pair; Send remains explicit. Send, Escape and Cancel close that composer without
opening an anchored thread window. The card retains any unrecorded delivery
failure for explicit recovery after the composer closes; signed Ask outcomes own
recovery once admitted. Proposals expose no Delete thread action.
Resolve folds the proposal to one line with its title, retained decision/resolved
mark and Reopen action; body, author and conversation history return on Reopen.
In Comments, resolved proposals use the same plain row as other threads: full
muted title, author/time, Proposal · Resolved state and Reopen. Proposal rows
prefix their Open/Approved/Declined/Resolved state with Proposal; annotations do
not. Resolved cards and rows use the ordinary resolved-thread mark, never the
Open mark. Opening the resolved row discloses retained history without
reopening the proposal or changing the pending count. This explicit opening
acknowledges a locally unseen resolution; rendering or filtering does not.
Comments mixes proposals with ordinary annotations and offers a Proposals only
filter. Filtering never changes status, sends a request or changes the header
count. The one header pending count is computed by `openThreadCount`: unresolved
proposals without a final decision plus unresolved ordinary annotations. Approved
or declined proposals are not pending even before resolution; proposal-linked
threads contribute once, and Chat/deleted threads never contribute. Reopening an
undecided proposal adds it back; reopening a decided proposal keeps its decision
and does not add a pending item. `--after` placement remains deferred.

`proposal ls PAGE` reads authenticated proposal threads, their retained decision and
independent resolved state. Human rows print the shortest unique UUID prefix (at least
8 characters). `resolve` and `add --id` accept those prefixes from the current
page's authenticated proposals, refusing missing or ambiguous matches before any
write. Only a full canonical UUID can start a new proposal with `add --id`. `proposal resolve PAGE ID` uses the existing owner-device
resolution action, sends no notifications and is a no-op when already resolved.
Resolve refuses missing, ambiguous or deleted proposals. The generic thread Reopen
operation retains the proposal and decision. Proposal dispatch does not run from
CLI, render, sync or recovery.

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
Ordinary comment actions only publish discussion. An explicit trusted Send,
Approve or Decline activation may dispatch through the existing Ask path. Standalone asks retain their existing panel and framing.

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
`html`/`meta.title` plus optional `meta.publisherAgent`, `meta.originalAuthor`, `meta.creationRecipient`, `meta.layout` and `meta.layoutReason` schema and requires no third-party libraries.
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

### Owner-page readmission (#2557)

An open page is bound to one page epoch. When that epoch advances or an authority recheck
drops its sync socket, the socket ends with `STALE_EPOCH`, `DENIED` or `EXPIRED`. For the
owner's own page this is not a lost grant: the page reopens itself for the new epoch without a
reload, keeping drafts and the open panel.

- A reopen starts at most three times in 60 s per page; within one reopen, up to five open
  attempts run at 0, 1, 3, 8 and 15 s, so a lagging epoch or admission is retried.
- A refusal that ends access (a revoked device, `Access ended`, a deleted page, a closed or
  ended session) stops the reopen at once. Readmittable and unclassified refusals retry; after
  the last attempt or an exhausted budget the page shows its terminal card.
- A bare 404, 502, 503 or 504 from the door's mount is the extension restarting or upgrading
  (#2929), not a refusal of the device: the open page waits and retries silently, with no limit
  while the tab is visible, and does not spend a reopen. Every retry opens a fresh Remote
  Session first, so a device revoked or unpaired meanwhile still ends the page.
- A route reload that delivers a newer binding ends the reopen in flight and the binding an
  earlier reopen produced, so one page keeps one sync socket.
- A read-only link reader is never reopened: the same codes end its access ("Access ended").

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
budget that grows with the input it reads (the limits table), terminated on overrun. Give it only needed plaintext update/document
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
verified update set: the state encoding of exactly those updates, never of a
document that also holds another writer's. The native writer merges them with yrs
`merge_updates_v1`. A browser device encodes the state of the document that holds
exactly its own admitted `own` updates (Yjs `encodeStateAsUpdate`), which also drops
the deleted content a merge keeps. A browser checkpoint is published only when that
encoding fits one update and decodes back to the same state vector, the same bytes
when encoded again and the same own projection, and carries no unresolved
dependency. Never use `encodeStateAsUpdate` of the merged shared document: that would
reattribute others' updates. A checkpoint covers seq 1..n and embeds the authenticated update head
hash at n, preserving namespace-specific updates and dependencies without
advancing the update chain. A namespace checkpoint covers that namespace's
subset within the shared sequence prefix; the signed descriptor binds the
prefix head. Namespace-specific plaintext remains raw merged Yjs update-v1.

Store may prune payloads through shared head `n` only when every namespace with
updates in that prefix has a committed checkpoint at exactly `n`, each binding
the same update hash in its signed previous-hash field. An unpaired checkpoint
is retained without pruning and is not selected for bootstrap. Completing the
pair atomically prunes both covered prefixes and superseded unpinned checkpoints.

The dormant native publication tracker adds a fence only for an explicitly tracked own
stream. Its initial pending set is the current paired checkpoint plus its contiguous
retained tail; older pruned updates are covered by that checkpoint. A stream without
a checkpoint starts from its entire contiguous retained tail. The cut is not
presumed published. Only exact publish acknowledgements or verified readbacks advance
confirmation, either one consecutive update or both originals of a checkpoint cut.
Update pruning cannot cross that confirmed prefix. The latest paired checkpoint stays
retained while historical checkpoints may be superseded, so a new checkpoint and its
unconfirmed tail remain publishable without storing every historical checkpoint.
Missing bootstrap bytes or a noncontiguous tail refuses with `ResyncRequired`; a wrong
binding, uid, hash or epoch cannot advance progress. This bounded metadata lives in the
existing Store, copies no payloads and enables no cloud sync or serve wiring by itself.
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
There is no cross-writer compaction checkpoint in v1 except the
[owner fold](#owner-fold-3983); the epoch baseline above is a distinct owner-authorized reset.

#### Stream position and native cloud bootstrap

A stream's **position** is its latest sequence and the hash it has there. The Store defines it in
one place, `position` in `store.rs`, and no other code reads the receipts table for it: it is the
latest receipt or, when later, the head that the stream's latest checkpoint binds. A stream built by
appends and checkpoints has a receipt at its checkpoint's sequence (the Store refuses a checkpoint
without one), so for it the two are the same; only a bootstrap leaves a checkpoint without its
receipt. An update must follow the position: the next sequence, citing its hash.

When this machine holds nothing of a foreign stream (no receipt and no checkpoint), native cloud
inbound starts it from the cloud's latest paired checkpoint, ordinary or an owner fold, if the
provider no longer holds the stream's log from sequence 1 or holds no log of it at all. It lists
the page's checkpoints once per pass, only for a stream that needs one, and takes the stream's latest
sequence. Both namespaces must be there with one signed head; for a fold the anchor is read by
exact ID from the log. A pair that is not whole yet, or no checkpoint at all, changes nothing and is
looked for again only when the page's records change. Each object is admitted by the code that reads
a stored checkpoint or fold (signature, role, epoch, a revoked device's signed cut, a fold's signer
list and anchor), and the Store installs the pair in one transaction: both checkpoints and, for a
fold, its anchor as the receipt at its sequence. The page's activity time is refreshed as for any
admitted update. The same pair again is a replay. A stream that holds a receipt or a checkpoint is
never joined to a pruned prefix, and a frozen stream takes nothing: both stay stopped with the
visible line. The tail then follows from the next sequence under ordinary continuity, the serve
announces each stored checkpoint as for a local compact, and nothing is queued for publication. A
pair that is not admitted, a malformed record or a missing anchor stops that writer, and no other,
with its line.

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
| Automatic combine of a device's own stream   | 50 updates or 1 MiB tail, after a write       |
| Per-device append rate                       | 10/s sustained, burst 50                      |
| Per-page decoder concurrency                 | 1                                             |
| Rust decoder batch deadline                  | 2 seconds                                     |
| Browser decoder time budget, per command     | 2 seconds, plus 1 second per MiB of input     |
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
best effort and repeatable. The serve tells live subscribers which checkpoints it
stored (`checkpoint` notice, see the sync frames); an open browser settles that
before a read fenced by the page revision. Before publishing either checkpoint, the owner decodes the page with
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

### Owner fold (#3983)

Core and security accepted the direction in
[#3983](https://github.com/pj-tmt/tmt/issues/3983#issuecomment-6283370867). The object model, the
browser's admission and fold-read, the native store and catch-up, and this machine's writer
(`tmt colab page compact`) ship; what the cloud channel cannot carry yet is the last bullet.

A **fold** is a checkpoint that an owner-member device signs for another owner-member device's stream
(its own included), so the owner's `page compact` can clear every device's tail. It reuses what a
checkpoint is: it covers seq 1..n, binds the update hash at n, does not advance the stream, is
published by immutable object ID, and is paired across both namespaces at the same n before
anything is pruned.

- **Object.** A new kind `fold` in the `tmt-colab-object-v1` header, not a framing version. A `fold`
  header carries one extra trailing field, `coveredDevice`, so its frame has fourteen elements where
  every other kind has thirteen (the domain tag included). A reader of today's release requires
  thirteen, so it refuses a fold as invalid by that count before it reads the kind or any trailing
  field; a reader that knows the kind ties the count to it: a fold without the field, or any other
  kind with it, is invalid. `authorDevice` is the signer; the covered stream's `streamId` is
  `coveredDevice`; `streamSeq` is n and `prevEnvelopeHash` is the hash of the covered stream's update
  envelope at n. The plaintext is raw merged update-v1 of exactly the covered stream's verified
  updates 1..n in the header's `namespace`, built by the checkpoint rules above (never the merged
  shared document).
- **Who.** The signer is an owner-member device with a verified chain. The covered device is an
  owner-member device of the page's epoch. Bridge streams, link, commenter and named-member streams,
  and any device without an admitted chain keep the own-stream rule: they stay signed data and still
  count against the page cap.
- **Attribution.** The writer of every `own` root, thread and message is the covered device, never the
  signer: a reader takes identity from `coveredDevice`, not from the signature. A fold replaces the
  per-edit attribution of the updates it covers with its own statement, which names the signer and the
  covered range (the trade-off own-stream compaction already makes).
- **Bounds.** n is at most the covered stream's admitted head. For a revoked covered device n is at most
  the `tailHeadSeq` of its signed revoke cut, never past it (the cut an owner advance sealed after
  the fold's `membershipRevision` stands in for it, as for an update), so a fold cannot cover a tail
  the cut refused. The owner publishes a fold only after the existing source-equality guard: it decodes the
  candidate merged streams and requires the HTML, the complete metadata and every writer's own
  projection to equal the current page; a mismatch or decoder failure publishes nothing.
- **Anchor.** The store keeps the covered stream's update envelope at n un-pruned when it prunes a
  fold's prefix (at most one update envelope, 256 KiB, per covered stream). A reader admits a fold only
  with that anchor: an `update` envelope of `coveredDevice` at `streamSeq` n whose envelope hash is the
  fold's `prevEnvelopeHash`, verified under the covered device's chain, so the fold names a head the
  covered device itself signed and not a value the signer asserts. A reader applies nothing from the
  anchor; its state is in the fold. Catch-up delivers the anchor with the fold, before the tail, and
  the tail stays contiguous from n+1. A fold without its anchor is refused (`INVALID`) and nothing is
  applied. A stream has one sequence across its namespaces, so the one anchor serves the fold of both
  namespaces; it is the update at n, whichever namespace that update is in.
  On the wire each catch-up frame still carries one object. The anchor is its own frame, a `streams[]`
  entry `{streamId, namespace, checkpoint: null, anchor, tail: []}` whose `anchor` is the update entry
  (`namespace` is the update's), and the fold frame follows it with `streamId` the covered device and
  `checkpoint` the fold entry. The extra `anchor` key is what an older reader refuses (it requires
  exactly four keys). `chains` carries the chains of both the signer and the covered device. A reader
  keeps the anchor for the catch-up (one per covered stream, bounded) because one anchor serves the folds of both namespaces, and applies nothing from it.
- **Signer revoke commits its folds.** The `device.revoke` payload of a device that signed folds gains
  `folds`: a sorted, unique list of the fold envelope hashes the owner accepts from it (at most 512).
  A fold whose `authorDevice` is a revoked device is admitted only when its envelope hash is in that
  list; an unlisted fold is refused (`INVALID`), whatever `membershipRevision` it cites, because a
  stolen key can backdate that revision and, once a prefix is pruned, no reader can recheck the folded
  state against the updates it replaced. The list holds only retained folds: a fold superseded by a
  later paired checkpoint or fold of the same stream and namespace is pruned and not listed. A revoke
  that would list more than 512 is not signed; the owner first re-folds those streams under another
  owner device, which supersedes the revoked signer's folds, and then revokes, so a fold is never left
  unlisted after its prefix was pruned. Unseen offline folds beyond the list are rejected like unseen
  updates beyond a cut. A revoke cut for the covered stream may name a fold as its checkpoint
  (`checkpointEnvelopeHash` the fold's hash, `checkpointSeq` n).
- **Concurrency.** A fold does not advance the covered chain: the covered device's update n+1 still
  cites the hash at n and survives as an ordinary tail update, so an edit in flight during the fold is
  never dropped and the device continues from the fold. If the device also seals its own checkpoint at
  m, the greatest paired n wins; both fold the same verified updates and cannot disagree.
- **Store.** A fold is stored as the covered stream's checkpoint, with the signing device recorded
  beside it (schema 14, `checkpoints.signer`; empty means the stream's own device). A fold in one
  namespace alone prunes nothing: the stream is pruned to n only when both namespaces with changes
  up to n have a checkpoint at n, and the update at n stays as the anchor until a later checkpoint or
  fold of the stream supersedes it. A fold never replaces another checkpoint and must bind an update
  the stream holds. The native snapshot reads the anchors it needs from the store.
- **Wire.** Catch-up delivers the anchor frame and then the fold, as above. `append` of a fold sent by
  another device is not accepted yet: the only writer is this machine's own (below), which stores
  folds directly. The serve announces a `checkpoint` notice whose `streamId` is the covered device for each
  fold a browser's Compact wrote.
- **Writer.** `tmt colab page compact` (and the browser's Compact, which asks the same route) first
  combines the root-local device's own stream, then folds the streams of other owner-member devices
  with this machine's local writer as the signer, in steps: at most 16 namespace merges per run and
  the caller's deadline; streams not reached are counted and the next run continues. A fold is
  published only after the source-equality guard above, once per run for all of its folds. A stream is
  left as it is, and named with its reason in the report, when it is a bridge, link or commenter
  device or has no admitted owner chain (`not-owner-device`); when its merge would exceed one
  object (`too-large`); or when its changes did not merge into exactly the same page (`refused`).
  The report gains `folded` (each `{stream, through, mergedObjects}`), `left` (each `{stream, why}`)
  and `unfolded` (a count); the keys are absent when empty.
- **Cloud.** The publisher creates the exact signed anchor as a `log` document at the covered
  stream's original sequence, before the paired namespace folds in `checkpoints` chunks. The
  document's `uid` names the authenticated publisher, not the signing device. Existing Rules
  check identity, shape and continuity (the space owner can bootstrap the anchor without its
  predecessor); readers independently verify authority and signatures. The signer and covered
  device's chains precede the objects, even when the signer has no data stream to publish.
  The chosen fold batch carries its signer proofs, so a concurrent newer fold cannot substitute them.
  Publication advances only after the anchor and both complete namespace originals are confirmed;
  an uncertain result is reconciled by exact ID, never by resealing or choosing different bytes.
  The hosted reader validates complete chunks and their object IDs, admits the anchor-only frame,
  then the fold pair under the covered stream, then its contiguous n+1 tail. It applies nothing
  from the anchor. Missing or invalid anchors, chains or pairs refuse without applying their data.
  Existing discovery supplies the anchor on first load; a newly discovered fold on a later wake
  can use a bounded, budget-charged exact-ID log read. Neither Rules nor declaration changes.
  Native cloud inbound starts a stream it holds nothing of from the fold pair and its anchor
  ([native cloud bootstrap](#stream-position-and-native-cloud-bootstrap)); a stream it holds a prefix
  of stays stopped with its visible line, directing the reader to keep editing on the machine that
  created the page.

Frozen vectors, replayed by native and browser admission, ship with the code and must cover: a fold
admitted; an unlisted fold from a revoked signer refused; a listed fold from a revoked signer
admitted; a missing anchor refused; an anchor whose hash differs from `prevEnvelopeHash` refused; a
covered device that is not owner-member refused; a fold past the covered head or past a revoked
device's cut refused; an unknown kind rejected before its trailing field is read.
`contracts/vectors/fold-v1.json` freezes them (see its README entry). Native replays each case
through the model and a real store (the fold is written with the store's own fold call and read back
through the snapshot), except the two `advanceThenRevoke` cases, which the browser alone replays
through `Admission` and `Objects` with the whole catch-up.

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

Only an explicit trusted Send, Approve or Decline activation in parent chrome
dispatches agent work. Ordinary comment publication,
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
admission and, once a send is admitted, its attribution to the person at the machine
([Identity attribution](../../../contracts/remote-channel-v1.md#operations)); the ledger
still sends only the anonymous form. Core owns acceptance/advisory wake/final retention. Accepted is not
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
the parent-owned publication API: the device's stream leader (the tab holding its writer lock)
refuses, before sealing anything, a write to an own key that the stream already holds with a
different value, whichever tab wrote the first value, because two tabs are two Yjs clients and
admitting both would let the higher client ID win rather than the first write. The refusal is a
typed `OwnRecordConflict` to the submitting tab only: nothing is appended, its connection and the
stream are untouched, and a byte-identical value is already present, not a conflict. A recovery
pass that meets it records nothing and reads the operation again at the next observation.

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
older reads are not automatically retried. Continued visible polling uses 2-second
to 30-second backoff. A newly accepted explicit Send wakes the current read wait
and restarts its cadence; for ten minutes after the latest accepted Send in that
controller, read steps are capped at three seconds (two, then three seconds).
Reads never extend this active window. It is ephemeral per controller; a Session
replacement restarts observation from the fast step without carrying the window.
Polling pauses hidden pages and stops after the unchanged two-hour operation
horizon. Annotations and Chat then keep a static status, the accepted ledger state and read-only re-check action; activity is presentation only, as defined below.
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

### Agent working-state line (#3331)

A sent Chat or annotation message uses only the current Remote agent row's optional
`activity: {state, sinceMs}` and owner `observedAtMs`. Missing or invalid telemetry
shows plain `Sent`, without animation. A valid working state whose phase time is
missing or later than its owner observation also keeps plain `Sent` on this line:
the current Ask's turn cannot be classified, while shared chips still show the
known agent-wide working state. Unknown, ended, stale and refused telemetry keep
their existing unknown behavior. Sending shows a wordless pending mark that follows the
[busy-mark delay](../../../design/gui-style.md#transient-states); held, refused,
uncertain and failed outcomes keep their existing delivery actions. A verified reply,
including an empty reply, replaces the line. Presence and runningDriver never prove
working, delivery, completion or admission, and no activity observation dispatches.

Phase times belong to the owner's wall clock. The anchor for an explicit Send is
`observedAtMs` from the latest pre-Send directory read if it is younger than twice
the current Ask cadence interval; otherwise one directory read is made at Send.
Without that fresh owner-clock anchor the line stays `Sent`; the viewer's intent
issuedAt is never substituted. Working since the anchor, including equality, shows
`⠹ <name> is working · m:ss`; earlier working shows
`◌ <name> is finishing an earlier turn · 3m`, with a static queued mark. Only an observed
working phase at or after the anchor followed by idle, without a reply, shows
`✓ <name> finished without replying · 2m ago`. Explicit unknown/ended or stale
telemetry shows `✓ sent · <name>'s activity is not reported`. Durations use owner
`observedAtMs − sinceMs` plus monotonic local time since acquisition, never an
owner/viewer wall-clock subtraction. No elapsed time claims that work is stuck.
Anchors and observed transitions are page/controller-local, never persisted or sent.

Optional activity acquisition shares the existing visible Ask observer across all
recipients and surfaces. Starts are at least 60 seconds apart, including refused
or failed attempts: the next permitted time is charged before acquisition starts.
New Sends, observer restarts and visibility changes never reset that time. There
is at most one inventory read in flight. Explicit Send destination/admission and
fresh owner-anchor reads are independent of this optional spacing; it never
refuses or delays Send.

Only an owned Ask awaiting a reply within its two-hour horizon permits activity
acquisition. Hidden, closed, replaced or no-longer-admitted generations discard
late results. A visible return reads only if the interval permits. One continuing
visible two-hour Ask allows at most 120 optional acquisitions per controller;
there is no lifetime cap that stops activity for later Asks in a long-lived tab.
Fresh activity is younger than 120 seconds; without another acquisition it becomes
static unknown, never an indefinitely working animation. Stopping observation
leaves plain `Sent` or the last non-working state. This is a controller-local rate
bound, not a cross-tab or per-device daily quota. Remote's current observational
reads are not journal-adopted; its separate call limits still apply.

Every shared agent chip in the current page uses the same observation, with no
additional reads. Its robot becomes a labelled working spinner only for a unique
matching agent with fresh, valid working telemetry; idle, ended, unknown, stale,
hidden and stopped observations keep the robot. This is agent-wide activity,
not a claim that the agent is working on this particular Ask: a queued Ask may
still name an agent working on an earlier turn. Name-only chips with ambiguous
matches keep the robot. Valid working state needs no phase timestamp or owner-clock
anchor; those clocks govern the separate elapsed-time copy. The chip never changes
recipients or Send admission.
Only that spinner and the finished check use the working/success colour; all text,
queued and unknown marks stay neutral. Reduced motion keeps working text static;
words carry every state, and text and marks remain visible under forced colours.
`NO_COLOR` is a terminal convention, not a browser setting.

Hosted currently has no `agents.list` operation and adds no call or native fallback;
missing observation stays `Sent`. A future hosted inventory cadence is no faster
than 60 seconds, with a 120-second freshness bound and the same Ask horizon. These
cost bounds are policy, not a measurement of billed Firestore requests.

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

Document image sources may be `colab-attachment:<full attachment ID or unique id8>`. The trusted
host resolves them only against the current page's verified document descriptors, using the
same admitted `document-current` read as Files (including reader tickets). Only byte-checked PNG,
JPEG and WebP data URLs enter the existing renderer port; no object URL, key or new network
permission enters the frame. A host-generated image request ID and render ID fence every reply.
References are resolved in document order, at most 128 unique references and 64 MiB of declared
plaintext per render; aliases share one read/charge. Unknown, ambiguous, non-raster, unreadable
and over-budget references produce inert alt-text placeholders, never a fetch fallback. Scope or
binding changes clear display copies and suppress late replies. The Local object backend supports
this path; Firebase MVP has no blob resource and displays the not-here placeholder. `data:` is
reserved in author guidance for tiny icons, not a workaround for the 2 MiB HTML cap.

On a content change, debounce about 300 ms and replace the frame with a fresh
renderId/port, tearing down the old ones. A viewer may pause live updates. Bind
each renderId to SHA256 of the exact captured source UTF-8 bytes; a Yjs state
vector is only sync metadata and MUST NOT identify the rendered bytes alone.
Interactive JavaScript state is lost on each replacement. Paused preview and
Send retain the captured source/quote/final bytes.

After the bootstrap document loads, the parent sends exactly one source-init
message containing `type:"colab.render.bind"`, `renderId`, `sourceDigest`,
`source` and `theme` (`"light"` or `"dark"`), and optionally `layout` (`"column"`,
`"full"`, `"canvas"` or `"freeform"`; absent is `freeform`), plus one MessagePort. The renderer accepts only these exact fields,
valid UUIDv4/digest metadata and source at most 2 MiB UTF-8, from
`event.source === window.parent`; origin is not an admission predicate for this
opaque channel. Direct top-level opening cannot initialize source. Invalid or
non-parent messages do not initialize it; after one valid init all later init
messages are ignored. No source enters a URL or request body. The bootstrap
replaces its own document using `document.open/write/close`, retaining the
response policy and prepending a trusted acknowledgement before author source.
Bootstrap load and initial source-document load are the only permitted loads;
further document loads/navigation tear down the frame.
The parent sets no deadline on the `colab.render.bound` acknowledgement, which follows the author
document's own `load`: a very large page binds late and shows its loading state meanwhile, and only
navigation or the page being left ends the frame (#3982).

**Page links (#4083).** The bootstrap installs ordinary link interception before author HTML
executes. A trusted primary, keyboard, modified or middle activation of `a[href]` prevents the
frame's default navigation. A literal same-page `#id` scrolls locally through the existing bounded
anchor path without setting location. Other links post exactly
`{type:"colab.render.link",renderId,href,newTab}` to the parent; `href` is at most 8 KiB UTF-8 and
`newTab` is boolean. The parent requires the current ready frame's `event.source`, exact
fields/types/bounds and render ID. Invalid, stale or disposed requests do nothing. A render ID and
a bootstrap claim never authenticate a gesture or author content.

The trusted URL classifier uses the app's base, never an authored `base` element. Only a same-origin
`/p/SHORT` with a valid page prefix, or an exact same-space owner link, can select app navigation.
Only the page ID/prefix and one bounded cosmetic element fragment survive; extra/duplicate query
or capability fields, credentials, controls and invalid routes are refused. Short-link resolution
retains its cosmetic anchor through normal page admission. This adds no cross-space read authority.
Outside URLs allow HTTPS or HTTP on literal `localhost`, `127.0.0.1` or `[::1]`; other schemes are
refused with a trusted notice, without echoing their payload.

A same-space request navigates directly only when the parent observes
`navigator.userActivation.isActive` at receipt. Activation proves recent interaction, not which
link the person used. Without activation, even a forged current-render request requires the same
trusted Open/Cancel confirmation. At most one in-place navigation is allowed per render lifetime.
Modified/middle intent opens a new app tab without consuming that guard; plain intent uses the
existing router owner. Opening a tab leaves the current page's links available.
This is the only exception to renderer requests being unable to select a page or navigate.

Every outside URL requires confirmation, regardless of activation. A trusted bar below the header
shows its complete frozen URL as inert selectable text, with Open, Copy link and Cancel. One
pending request is retained; later frame requests cannot replace its destination. Escape or an
outside press cancels. Replacing/stopping/leaving the render clears the request. Only a trusted
parent Open activation opens the admitted frozen URL, using
`window.open(url,"_blank","noopener,noreferrer")` for a new tab. Open clears the request; there is no
automatic retry. A null return does not prove popup blocking, so the bar always offers Copy link
and says "If no tab opens, copy the link instead." Copy retains the bar. No destination is fetched
merely because a request or confirmation exists. Scripted location changes, forms and any missed
navigation retain the existing teardown and self-navigation limitation; sandbox/CSP are unchanged.

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

The cards the parent draws over the page take the scheme the page paints, not the app's (#4081).
The renderer measures the document's own canvas colour (the root's background, else the body's)
over the white frame canvas and sends the parent exactly
`{type:"colab.render.scheme", renderId, scheme}` with `scheme` one of `light` or `dark` (relative
luminance under 0.179). A page that paints no background is the white frame canvas, so it is
`light` in either theme. It sends the first value at the document's `load`, and again only when
the value changes, as the page repaints itself, the theme changes or the OS preference changes.
The parent accepts only the current render ID and those two values and scopes the cards to the
light or dark tokens; until the first value they keep the app theme. The value is cosmetic:
author code may send it, and it grants no capability.

A `column` or `full` layout makes Colab own the page frame. Before author code runs, the
bootstrap sets `data-colab-layout` on the renderer root and adds the stylesheet generated
from the app's `src/page-frame.css` in a CSS layer declared ahead of every author style.
The top-level blocks, the element children of `<body>` that the frame did not add itself,
share one centred column of `min(1280px, 100% - 32px)`; `<body>` is a flex column with a
16 px gap, 8 px above a block marked `data-gap="tight"` and 40 px above `"loose"`, and a
block marked `data-width="full"` runs edge to edge with its text padded onto the column.
An author's width, `max-width`, margin and the block gap, also with `!important` in a
stylesheet, do not apply to those blocks or to `<body>`'s own inline margin and padding;
colours, type and the layout inside a block stay free. An inline `style` attribute
with `!important` still wins: the frame is a cosmetic projection like the theme and author
code may tamper with it. `canvas`, `freeform` and an absent layout set nothing and add no
stylesheet. The frame's own spacers are not blocks, and the parent draws its bands
and question lines on the same column. The layout is the page's verified `meta.layout`; the
browser binds the frame from the live view, so a change that arrives without a new source
(a `page layout` from the CLI or another device) reframes the open page.

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
A trusted plain click or tap inside a current resolved highlight requests the same
known-thread view action as its keyboard-reachable marker. The shortest matching quote
wins, with stable thread-ID ordering for equal lengths. Hover uses a pointer-inert outline.
Drag/text selection, long presses, modified or synthetic clicks and editing targets do not
open a thread. Selection mode gives an existing highlight precedence over creating a new
annotation. Replacement clears resolved geometry; the parent still checks the current
render/request and known thread ID. Neither highlighting nor opening publishes or sends.

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
supply no anchor. Unmodified, non-repeating C annotates an existing completed selection while mode is
off; otherwise it toggles parent-owned selection mode. It stays outside
input/textarea/select/contenteditable/textbox targets, with all modifiers, handled-event,
IME and keyCode 229 guards. The frame requests only `type:"colab.render.toggle-selection",renderId`;
the parent enables it only for the admitted comment surface. Its bound port projection is
exactly `type:"colab.render.selection-mode",renderId,enabled` (boolean). Mode is cosmetic:
one hover outline, a completed text selection or a text-bearing click captured through
the same bounded quote owner opens the existing anchored discussion. Textless clicks
show a temporary polite hint and create nothing. Source replacement reprojects mode;
page changes reset it. Readers have no toggle. Toggling never prepares or sends an Ask.
Escape consumes one level: a nested menu or mention list, then the active drawer (Chat draft retained and busy sends uninterrupted), then the anchored window (draft retained), then selection mode. The frame sends only `type:"colab.render.key",renderId,key` with the closed key set `Escape|@`, a current-render cosmetic notification to the same parent ladder; it grants no write authority. A separate seven-key `K` branch carries only `type`, `renderId`, `key`, and boolean `ctrlKey`, `metaKey`, `altKey`, `shiftKey`; the parent requires the current ready frame, exact keys and platform modifier with Alt/Shift false, and may only open its inert quick switcher. This key request cannot choose a page, navigate, close another overlay or send; selection requires a trusted parent action. Page-link requests have only the bounded activation/confirmation exception above. Outside the composer, the C editing/IME/repeat/modifier guards apply to Escape too. The bottom-right toggle stays at its content inset; horizontally overlapping anchored windows fit above its reserved strip instead of moving it into the header.
Alt+Enter still explicitly requests the annotation view with exactly
`type:"colab.render.annotate",renderId`; it cannot send. A trusted parent action captures
the quote. Page-wide comments have null anchors.

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

### Dashboards (#3648)

A dashboard is an ordinary page with structured content, not authored HTML. It is the
name the UI, the CLI (`tmt colab dashboard`) and the skill use; its stored value keeps the
older name `shell` (`meta.pageType`, `meta.shell`, `shell.json`, `shell-v1.json`), which is
signed data and never migrated. A space
may have many dashboards; none is a default or Home. Creation, membership, archive,
delete, epoch, signed content publication and revision fences retain their existing
owners. Only the owner and the owner's admitted agents edit a dashboard in v1;
shared viewers read it. References grant no access to their targets.

The content document retains its existing `html` and `meta` roots. A dashboard has an
empty `html` text and `meta.pageType = "shell"`; `meta.shell` holds the complete
closed value below as plain JSON, not nested executable markup. The empty text
is a wire container, never rendered as the dashboard's content. A dashboard cannot have
HTML text or document attachments. The discriminator and dashboard value occur
together. Ordinary HTML pages have neither field. The isolated native decoder
and browser fold validate this on every incoming state and prepared action;
raw Yjs updates cannot bypass the same schema. A dashboard's title stays in
`meta.title` so the ordinary list and search can display it.

```json
{
  "version": 1,
  "items": {},
  "order": []
}
```

`items` maps canonical generated UUID item IDs to one of these closed objects:

- page: `{kind:"page", pageId, note?, audience?}`;
- anchor: `{kind:"anchor", pageId, anchor, note?, audience?}`;
- reading: `{kind:"reading", pageId, readingId, note?, audience?}`.

`pageId` and `readingId` are canonical generated UUIDs. Targets belong to the
dashboard's space. An anchor is exactly
`[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`: it resolves by exact ID to exactly one
element, never by CSS/XPath, and scrolling or highlighting grants no action
authority. A reference to another dashboard is a link only, never expanded.
Deleted, absent, ambiguous or unreadable targets stay unavailable references;
they are never automatically removed or admitted.

Optional fields are omitted, never null. `note` is at most 512 UTF-8 bytes;
`audience` at most 128. Text is inert and contains no control characters or
unpaired surrogates. `order` contains every item ID exactly once. There are at
most 256 items. The whole dashboard value, encoded as compact UTF-8 JSON with
lexically sorted ASCII field/UUID keys and integral numbers normalized, is at
most 131072 bytes. Existing update, history and transport limits apply too;
the dashboard bound does not measure retained Yjs history.

An optional `summary` is
`{line1,line2,attribution:{writerId,timeMs,displayAgent?}}`. `line1` is at most
24 Unicode code points and `line2` at most 60, with the same inert-text rules.
The write owner records its authenticated writer UUID and owner-clock Unix
milliseconds. Optional `displayAgent:{name,id?}` is the existing Core-resolved,
validated display metadata, never proof of agent authority. Missing agent data
is omitted and never interpreted as a person. Actions cannot supply attribution.
There are no layout, style, section-heading, smart-search or seen-state fields.

Each action is trial-applied to a detached admitted document. Only a schema-valid
whole result becomes an ordinary signed content publication. The seven closed
bodies are:

| Action            | Body                                                  | Effect                                                       |
| ----------------- | ----------------------------------------------------- | ------------------------------------------------------------ |
| `item.add`        | `{itemId,item,beforeId?}`                             | Add a new reference; omitted placement appends.              |
| `item.remove`     | `{itemId}`                                            | Remove the reference and order entry, never the source page. |
| `item.move`       | `{itemId,beforeId?}`                                  | Move an existing reference; omitted placement moves it last. |
| `item.set`        | `{itemId,item}`                                       | Replace the whole item, preserving its position.             |
| `summary.set`     | `{line1,line2}`                                       | Set both lines and system attribution.                       |
| `summary.clear`   | `{}`                                                  | Remove summary; absent already is a valid no-op.             |
| `reading.suggest` | `{itemId,pageId,readingId,beforeId?,note?,audience?}` | Add a reading reference under the same add rules.            |

Extra/missing fields, invalid types, null optionals, overlimits, duplicate or
unknown IDs, before-self, unknown placement and an incomplete order refuse the
whole action with `COLAB_INPUT_INVALID`, leaving the base untouched. A changed
revision is `COLAB_STALE_BASE`: read again, without an automatic retry. Adding
a caller-chosen item ID makes an uncertain outcome inspectable by reading it
back. Suggesting a reading never marks it read or sends a notice; those remain
the source reading's explicit person action and existing notification owner.

`tmt colab dashboard create --title TITLE --json` creates an empty dashboard and never
opens a browser. `dashboard read PAGE --json` returns
`{spaceId,pageId,title,pageType:"shell",shell,epoch,membershipHead,revision}`.
There is one mutating subcommand per action: `item-add`, `item-remove`,
`item-move`, `item-set`, `summary-set`, `summary-clear`, `reading-suggest`.
Each takes `PAGE --expected-revision REV --json` and, except `summary-clear`,
`--file PATH|-` containing its exact JSON body. Revision is required. Existing
space selection and short page-ID resolution apply. No raw Yjs or whole-dashboard
replacement CLI is added. Ordinary publication receipts, no-op and retained
original-operation/unknown-outcome handling apply; no resend or offline
fallback follows a possibly published request.

HTML-only source operations refuse a dashboard with `COLAB_INPUT_INVALID` rather
than erase it. Export writes `shell.json` beside the usual metadata, never
`page.html`. The owner app draws a dashboard as a greeting, three counts and numbered chapters (one
per distinct, available, non-archived source page) joined by hairlines, with older pages in a list;
the shared and hosted readers open it with a neutral notice explaining
`tmt colab dashboard read`. Everything shown is computed from the space list and the owner chapter
details, never stored: the greeting and the first count are the sum of the pages' needs-you counts;
a page not waiting and edited within seven days is in progress; waiting chapters come first, then
in progress, each by update time with the dashboard's `order` as the tie-break; a chapter shows the
passage that most needs the owner (a pending item an item names, else the first by rank, else the
page's latest comment, else the item note). From 900 px a chapter's "Open to decide" opens a panel
beside the dashboard (the app's drawer: Esc, a click outside or Close dismisses it) listing what waits on
that page and its latest comment, with the page and passage links; narrower, it opens the page. The
panel is the page route's `open` search parameter (a page ID, dropped when it is not a chapter of the
dashboard), so Back, Forward, a reload and a pasted link keep it; it changes nothing and is never state of
the shell. The page never marks a reading read. List/search use its
title and needs-you counts do not treat dashboard references as authored questions or pending readings. No Today
integration or migration of older collection data exists. Native and browser
consume the shared `shell-v1.json` vectors.

### Work (#4050)

An ordinary HTML page may carry work: a two-line summary and a task list, in the content
document's `meta.work` beside its HTML. A page with tasks is just a page; there is no other
tracker concept. A dashboard never carries work (it keeps its own summary), and a page with
`meta.work` has no `pageType`. The isolated native decoder and the browser fold validate the
value on every incoming state and prepared action, so raw Yjs cannot bypass the schema. Writing a
page's HTML source and rotating its epoch keep the value exactly. A page that has had no work carries no `meta.work`; its first action starts from the
empty value, and an action that leaves it empty writes nothing.

```json
{
  "version": 1,
  "next": 4,
  "summary": { "line1": "", "line2": "", "attribution": {} },
  "tasks": { "T-3": {} },
  "order": ["T-3"]
}
```

`version` is `1`. `next` is the number the next task takes, from 1 to 10000000. `tasks` maps
`T-<n>` (a decimal number from 1 to 9999999 without a leading zero, always below `next`) to a
task; a removed number is never handed out again, so `next` only grows. `order` lists every task
ID once, in creation order; there is no move action. There are at most 256 tasks. The whole
value, encoded as compact UTF-8 JSON with lexically sorted ASCII field and ID keys and integral
numbers normalized, is at most 131072 bytes, so a replacement stays within one update. Existing
update, history and transport limits apply too.

The optional `summary` is the dashboard's `{line1,line2,attribution}`: `line1` at most 24 and
`line2` at most 60 Unicode code points (where it stands, and what comes next), the same inert-text
rules, and attribution that the write owner records (its authenticated writer UUID, owner-clock
Unix milliseconds and the optional Core-resolved `displayAgent`, never proof of agent authority).
An action cannot supply attribution.

A task is a closed object whose optional fields are omitted, never `null`:

| Field          | Value                                                                                                                                                                        |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `title`        | Required; 1 to 120 code points of inert text                                                                                                                                 |
| `state`        | Required; `todo`, `in-progress`, `blocked`, `in-review` or `done`                                                                                                            |
| `detail`       | The spec: scope and what to do, at most 4096 UTF-8 bytes of plain text; line feeds allowed, no other control characters                                                      |
| `reason`       | Present exactly when `state` is `blocked`: 1 to 256 bytes of inert text                                                                                                      |
| `assignee`     | `{kind:"agent",name,id?}` (the shell's display-metadata shape) or `{kind:"person",name}`; a name is 1 to 128 bytes                                                           |
| `acceptance`   | 1 to 8 criteria, each `{kind:"test",name}` (a test name), `{kind:"check",command,result?}` (a command and, once run, its result) or `{kind:"person",who}` (a person or lead) |
| `needsSignoff` | `true` when the task waits for the owner's sign-off; omitted otherwise                                                                                                       |
| `externalId`   | At most 128 bytes: the external issue a driver syncs; the GitHub driver's is `#N` (see [GitHub driver](#github-driver-4053))                                                 |
| `child`        | A page ID (a generated UUID): the page this task follows (see [A task that follows a page](#a-task-that-follows-a-page-4118)); never together with `externalId`              |

Text is inert and contains no control characters (except the line feeds of `detail`) or unpaired
surrogates. A criterion records what makes the task acceptable; nothing in this version ticks it
or runs a command. `needsSignoff` is stored and counted; it gates no state change. The owner
answers it from the band by moving the task to `done` (sign off) or back to `in-progress`.

Each action is trial-applied to a detached admitted document. Only a schema-valid whole result
becomes an ordinary signed content publication. The nine closed bodies are:

| Action          | Body                                                                     | Effect                                                                                                                   |
| --------------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `task.add`      | `{task:{title,detail?,assignee?,acceptance?,needsSignoff?,externalId?}}` | Add a `todo` task as `T-<next>` and count `next` on; the caller never chooses the ID, and it is returned.                |
| `task.set`      | `{taskId,title,detail?,acceptance?,needsSignoff?,externalId?}`           | Replace the editable fields as a whole (an omitted optional is cleared); never state, reason, assignee or child.         |
| `task.state`    | `{taskId,state,reason?}`                                                 | Set the state; `reason` is required for `blocked` and refused for any other state, and a move out of `blocked` drops it. |
| `task.assign`   | `{taskId,assignee?}`                                                     | Set the assignee; omitted clears it.                                                                                     |
| `task.link`     | `{taskId,pageId}`                                                        | Set `child`; the same page again is a no-op, and another page while one is set is refused (unlink first).                |
| `task.unlink`   | `{taskId}`                                                               | Remove `child`; absent already is a valid no-op.                                                                         |
| `task.remove`   | `{taskId}`                                                               | Remove the task and its order entry; its number is never reused.                                                         |
| `summary.set`   | `{line1,line2}`                                                          | Set both lines and system attribution.                                                                                   |
| `summary.clear` | `{}`                                                                     | Remove the summary; absent already is a valid no-op.                                                                     |

Extra or missing fields, invalid types, `null` optionals, overlimits, a caller-chosen or unknown
task ID, an unknown state or kind, a `reason` without `blocked` (or `blocked` without one), a
dashboard and any caller-supplied attribution refuse the whole action with
`COLAB_INPUT_INVALID`, leaving the base untouched. A changed revision is `COLAB_STALE_BASE`: read
again, without an automatic retry. The revision fence, not a caller ID, makes a retried
`task.add` safe: after a publication that did commit, the repeat is stale. An action that changes
nothing is a no-op with the ordinary no-op receipt. The page's own HTML is unchanged by every
action.

Derived values are computed from the tasks and never stored or edited. The stage is
`discussion` with no task, `done` when every task is done, and `in-progress` otherwise. Progress
is the done count over the total. Who is working is the assignees of `in-progress` tasks, each
once, in task order. Sign-off is the count of `in-review` tasks with `needsSignoff`. The browser
folds the value and validates it, and the page route draws it as a status band under the header:
the two summary lines, the stage, progress, who works and the sign-off count, with the task list
and a task's detail opening in place and pushing the page down. On the owner device the open task
carries controls (state, sign-off, assign, edit, convert to Colab, remove) and the list an add-task field, each sending
one closed action as a [browser work action](#browser-work-actions-4051); the value changes only
when the page's own stream brings the commit back, and a refusal is shown with the serve's words.
Every other device sees the band read-only. The band scrolls with the page, and a page with no
summary and no task draws none. Page export includes it as `work.json`
(see [Plaintext page export](#plaintext-page-export-1309)). Native and browser consume the shared
`work-v1.json` vectors.

#### A task that follows a page (#4118)

A task with a `child` follows that page, and a page is the child of at most one task. The task's
own `state` is kept, ignored while it follows and shown again when it stops, and `task.state` is
refused for it. The task shows the stage of its page as a state: `discussion` (no task) is `todo`,
`in-progress` is `in-progress`, and `done` is `done`, or `in-review` when the task has
`needsSignoff`. A blocked task inside the page does not propagate (the page stays `in-progress`).
`child` and `externalId` exclude each other on one task, so a task's state has one owner. Both
validators enforce the exclusion, the id form and the closed actions, so raw Yjs cannot bypass them.

The value never holds the page's stage: derived values take a resolver from page ID to stage, and
a task whose page the resolver cannot answer for (deleted, unreadable, not asked) shows its own state.
The page's own stage, progress, workers and sign-off count are computed from the states the tasks
show, so a page whose tasks all follow finished pages is `done`; they are returned with the states of
the tasks that follow a page as `derived.states` (`{T-n: state}`, present only when some task does). A
page's stage is resolved through the pages it follows, at most 8 pages deep; a page already being
resolved, or one past the depth, answers nothing, so its task shows its own state and the resolution
always ends. The native resolver is the work index (below); the browser takes a resolver from the
owner's rows and never reads another page's document, and a reader without them shows a task that
follows a page without a state.

`task.link` is admitted by the write command, like the work-in-progress limit, because one page's
document cannot see another's. It is refused as `COLAB_LINK_REFUSED` when the page is the task's own
page, is not a page of the space (deleted, or one the owner cannot read), already has a parent (a task
of this page or of any other, naming it), or has the task's page below it (a loop). Two writers can
still race to link one page, or to close a loop; as with the limit this is a working agreement, not a
lock. Readers then resolve it the same way: a page's parent is the task of the page with the lowest ID,
then the lowest task number, and a loop is cut as above. An archived page keeps its link and its stage
is followed. The work-in-progress limit does not count a task that follows a page as held: the work is
held where it is, by that page's own tasks.

`tmt colab task ls|add|set|state|assign|link|unlink|convert|rm PAGE ...` and `tmt colab summary set|clear PAGE` run
the actions above, one command per action, and never open a browser. A write applies to the
revision it just read, or to `--expected-revision`; `task add` returns the new `taskId`. `task ls
--json` returns `{spaceId,pageId,title,work,derived,epoch,membershipHead,revision}`; a page with no
work returns the empty value, and `derived` carries `states` when a task follows a page. A dashboard
is refused with `COLAB_INPUT_INVALID`. `tmt colab ls` shows `work` (`stage`, `done`, `total`,
`working`, `signoff`, the tasks following pages counted by the states they show) on each page that has
work, and one line under its row; a page that is followed also carries `parent` `{page,task}` and says
"part of T-n of PAGE". `task link PAGE TASK CHILD` and `task unlink PAGE TASK` run the two actions
(CHILD is named as any page is). `task convert PAGE TASK` makes a new column-layout page titled like
the task, with its detail as paragraphs of escaped text, then runs `task.link` for it; it prints
`childPage`. Making a page moves the space, so the link is written on the revision read after it and
only while the task is still the one copied; otherwise nothing is linked and the command says which
page exists and the `task link` command that finishes it. It never searches for a page to adopt, so
running it again makes another page. A task that follows a page or has an `externalId` is refused
before any page is made.

An agent _holds_ a task while it is assigned to that agent (`assignee.kind` `agent`, by name) and the
task is `todo` or `in-progress`. The work-in-progress limit bounds the tasks an agent holds across
every non-archived page of the space: 1 by default, `tmt colab settings wip-limit N` for the space
and `wip-limit AGENT N|default` for one agent, stored in `settings.json` as
`{"wip":{"default":N,"agents":{NAME:N}}}` with N from 1 to 1000. It is an admission rule of the
write command, not a rule of the stored value. The command refuses a change that adds a held task
(`task.add` or `task.assign` to an agent, or `task.state` moving an assigned agent's task into `todo`
or `in-progress`) when the agent would then hold more than its limit, as `COLAB_WIP_LIMIT`
naming the agent and every task it would hold, with nothing written. Every other change is
admitted, so lowering the limit, a write from another device or a person's tasks never invalidate
published state or block finishing, unassigning or removing a task. The count reads each page's work
through a derived file keyed by the page's own progress (`work-index.json` beside the settings: the
epoch and the content counts, not the membership head, so another page's creation or sharing
refolds nothing; a page whose tasks follow pages also keeps each task's state, assignee, sign-off
flag and `child`, which is what resolution and `task.link` read), which holds nothing the pages do not
and may be deleted; a revocation alone can
leave an entry stale until the page's next write. Two commands racing on different pages may
both be admitted: the limit is a working agreement, not a lock.

#### Browser work actions (#4051)

The owner browser changes work through the sync socket's `work` frame (`operationId, action, body`),
owner device only, never a link or a public reader. `action` is one of the nine closed actions above, or `task.convert`,
and `body` its closed body; neither carries attribution, and the write is the root-local writer's,
with no display agent. The serve prepares it from a snapshot of its own outside the sync lock, like a
browser Save, and runs the write command's admission first: the task the body names exists, the
action applies, no agent ends up over its work-in-progress limit (`COLAB_WIP_LIMIT`, naming the agent
and every task it would hold) and a link refers to a page of the space and closes no loop
(`COLAB_LINK_REFUSED`). Both callers run the same function, so the limit cannot drift between the
command and the browser. An action the contract does not name is a protocol violation. The action
applies to the revision the serve reads when it prepares, so the action's own preconditions fence it
(a task that is gone is `COLAB_INPUT_INVALID`), and the commit's fences turn a page that moved
meanwhile into `COLAB_STALE_BASE`; nothing is written on a refusal.

A tenth action, `task.convert` with the body `{taskId}` and nothing else, makes the task a page of its
own, as `tmt colab task convert` does, and is the only browser action that makes a page. Page
creation is otherwise root-only, so this is a narrow authority of the owner device (`Access::Publish`,
checked again under the sync lock when the page is made): the page is a column page titled like the
task, its first text is the task's title and detail as escaped paragraphs, and no title, source,
layout or agent comes from the caller. The serve refuses a task that is gone, follows a page or has
an `externalId` (`COLAB_INPUT_INVALID`) before anything is made; makes the page with the root-local
`page.create` in process; reads the task again and runs `task.link` through the shared admission only
if the task is still the one copied. The page's ID is a name-based ID over the space ID and the
operation ID (a generated-ID shape; its creation is recorded under a second ID derived the same
way), so the same operation never makes two pages, a replay finds its page, and a new operation makes
a new one. A committed `workresult` carries `childPage`. If the link then fails, the result is
rejected and still carries `childPage`: the page exists and the task is unlinked, nothing is retried,
and `task.link` finishes it. `savestatus` for the operation carries `childPage` whenever that page
exists, so a lost reply says whether the page was made (`absent` with a page) and whether the link
landed (`committed`). No page is ever searched for or adopted.

The one `workresult` has a `saveresult`'s fields: `state` `committed` with the `revision` (and `taskId`
for a `task.add`), `unchanged` for an action that changes nothing, or `rejected` with `code` and
`message`. The operation ID is recorded with the commit, so `savestatus` on it answers a lost reply,
and an ID that already has a recorded outcome never answers `unchanged` for another action. Like any
frame, a reply is acknowledged by the browser's `ack`.

#### GitHub driver (#4053)

`tmt colab task github push PAGE TASK` and `tmt colab task github sync [PAGE]` link tasks to
GitHub issues. Both run the owner's `gh` on the owner's machine with the owner's sign-in, argv only
(the program is `gh` on PATH, or the file named by `TMT_COLAB_GH`, which is how tests supply a
scripted fake), and write to the page only through the ordinary `task.set` and `task.state` actions
under the revision fence. A failure writes nothing. The repository is the space's one setting,
`tmt colab settings github-repo OWNER/REPO|none`, stored in `settings.json` as
`{"github":{"repo":"OWNER/REPO"}}`; with none set both commands are `COLAB_GITHUB_REPO_UNSET`.
Nothing edited on GitHub is read back except the closing described below.

`push` is a command for one task and never part of `sync`: it publishes the task's title, detail
and acceptance to GitHub, possibly to a public repository, and prints the repository and the issue's
URL. A task that already has an `externalId`, or follows a page, is refused. The issue body carries a hidden marker
(`<!-- tmtcolab<32 hex digits> -->`, one alphanumeric token: the first 16 bytes of the SHA-256 of the page ID, a NUL and the task ID, so the digest identifies the task without revealing the page), the detail and the acceptance criteria
each inside a code fence longer than any run of backticks in them, so no mention or link in a page's
text acts, and every `#N` and `GH-N` reference in the title, detail and criteria is broken by a
zero-width space, so a page cannot link or close another issue. The page's own ID is published
in no form. Before creating, `push` searches the repository's issues for the marker and adopts the
one that carries it (the lowest number) instead of creating another, so a push whose record failed
(a stale revision, a crash) is safe to run again. It then records `#N` with `task.set`, the task's
whole editable part with its `externalId`, on the revision it read.

`sync` asks about every task with an `externalId` of the form `#N` (a decimal number from 1 to
999999999; any other ID belongs to something else) that is not `done`, on one page or on every
non-archived page of the space. It makes one `gh api graphql` call per at most 100 distinct issues
(`issue(number:N){state closedByPullRequestsReferences{nodes{merged}}}` under one alias each) and
asks about at most 200 issues in a run; the rest are reported `unchecked`, to be asked on a rerun. A task
becomes `done` (`task.state`, read again once after a `COLAB_STALE_BASE`) only when its issue is
`CLOSED` and a closing pull request has merged: GitHub's own `Fixes #N`. A task whose issue is
open, closed without a merged pull request, or missing is left alone. The result is
`{repo,calls,checked,rows}` with one row `{page,task,externalId,outcome}` per task, `outcome` being
`done`, `open`, `closed`, `missing`, `unchecked` or `failed` (a `failed` row adds
`error:{code,message}`). Any `failed` row makes the command fail with the rows in the same JSON
document; a failed check never moves a task.

Errors: `COLAB_GITHUB_REPO_UNSET` (no repository), `COLAB_GITHUB_UNAVAILABLE` (`gh` cannot be run, or
is not signed in; one such answer ends the run) and `COLAB_GITHUB_REFUSED` (`gh` refused, did not
answer within 60 seconds, or answered without what was asked; its message is carried, at most 400
characters).

### Answer blocks

An answer block is authored page HTML, not a new discussion record or CLI command:

```html
<tmt-question
  data-id="10000000-0000-4000-8000-000000000001"
  data-kind="single"
  data-agent="Agent name"
  data-other
>
  <p data-question>Which heading should we use?</p>
  <span data-choice>Keep it</span>
  <span data-choice>Change it</span>
</tmt-question>
```

`data-id` is a canonical UUID used only for control identity. Kinds are `single`,
`multiple`, `short` and `long`. The question has exactly one immediate
`data-question` child; choices are immediate `data-choice` children. Whitespace
normalizes as normal rendered quote text. At most 50 blocks, 20 distinct choices
per block, 200 code points per question/choice and 64 per agent name are admitted.
Duplicate IDs, nested blocks, empty text, control characters and invalid structures
supply no controls. Choice kinds require choices; text kinds have neither choices
nor `data-other`. Short answers allow 200 code points; long/Other text allows
8192 UTF-8 bytes. The complete comment remains within the existing 16 KiB limit.

Related questions can share one logical group across separate passages: set
`data-group="GROUP_UUID"` on each standalone `tmt-question`. A logical group has at
most 12 raw question placements, ordered as authored. Its ID holds the shared
question/group/review/reading namespace once; it cannot collide with a control ID
or a structural group's ID. A question inside a structural wrapper cannot also
supply `data-group`. Invalid references, conflicting identities and over-limit
logical groups supply no controls, rather than silently becoming standalone.

A structural group remains `<tmt-questions data-id="UUID">` with exactly one direct
`<p data-title>` and at most 12 direct question children. Its title has the question
text bounds. Optional legacy `data-label` retains its 24-code-point/control
validation; labels no longer create tabs. A lone question is an internal group of
one. The page markup check owns complete admission rules and ID-holder ordering.

All question forms render compact lines without tabs. Each line is a trusted
button with the admitted plain-text title, state word and chevron: waiting uses
`◆` and `Waiting for you`; answered uses `✓` and `Answered: <short answer>` with
one-line ellipsis. Only the mark carries the state colour; title, state text and
chevron remain neutral. Two or more adjacent questions have one muted
`Questions from <agent> · N of M waiting` header; a question placed alone has none.
Needs you counts the logical group once across every placement. Existing
`data-form="chip"` markup remains admitted under its existing attribute checks,
but uses the same compact lines instead of a tabbed panel.

Every question starts closed except an explicit Needs-you or notification arrival,
which opens the target, scrolls it into view and focuses its first control.
Open/closed state belongs to the viewer, never the page or answer records. Opening
reveals the existing parent-owned answer controls under a 1px left rule, without
a card frame. Answered questions are read-only until the existing Change answer
action is admitted. Answering closes the question and focuses the next waiting
question in the same group, or its own line when none remains. There is no
Dismissed state or dismissal action in this slice; closing a row acknowledges
nothing. Enter/Space toggles, Escape closes and returns focus to its line, and
Up/Down moves between a stack's lines. At narrow widths the state wraps under
the title while the chevron stays on its first line.

The authored text remains available to the normal quote/association owner while
the parent draws the compact title and controls. A local callback or frame claim
cannot provide a verified current answer. The whole question presentation follows
Colab's effective theme, including parent controls on a fixed-look author page.
Sibling in-page Proposal cards also follow the effective Colab theme throughout their
bodies, Approve/Decline/Follow up/Resolve controls and follow-up composers, even when
the author fixes a different page palette.

The parent group owns drafts, answer publication and visible recipient chips using
BrowserChips. Each physical block owns its Send scope: a split question sends only
that question, with defaults from its asking agent; a structural wrapper keeps one
batched Send for its questions. A Send never includes a different passage's draft.
Asking agent names resolve through the existing directory: one default chip for
each uniquely resolved identity in that block's authored order, deduplicated within
the cap of eight.
Until initial recipient discovery resolves and default chips are committed, Send is pending
and cannot publish; answer drafts remain editable. Resolved-empty or failed discovery retains
comment-only Send. Removing a default is intentional and directory refresh never restores it. The
same directory/UUID recipient policy rechecks explicit chips; absent or ambiguous
asking names add no recipient. Empty chips allow comment-only Send and show
`Stays on the page, no one is asked`; at eight, Add is disabled with `At most 8`.
Question/choice text cannot add recipients. The primary label is `Send <N> answers`,
singular `Send 1 answer` when one is eligible in a multi-question group, or
`Send answer` for a group of one in every state. For groups of several, with no
eligible answers and no batch in flight, the disabled action reads `Send answers`.
Its accessible name starts with that visible text, followed by ` · to <recipients>`
or ` · to thread only`.

Only trusted explicit Send publishes complete unsent drafts in its own block, in authored order, through
`sendAnswers`; one-question short-answer Enter uses the same one-answer path.
Choosing answers, opening/closing rows, render, reload and sync publish nothing. Each
answer is one plain-text comment, `Answer to “<question>”: <answer>`, followed by page
title and the visible recipient text. Multiple choices retain authored order;
Other uses `Other: <text>`. Partial Sends retain untouched and failed drafts.
A short, a long and an Other answer carry files through the attachment tray of comments
and Chat (`AttachButton`, `PendingAttachments`, paste and drop, the same limits and
notices; choices have no tray). The parent group holds one attachment draft per question
beside its text draft: files survive opening, closing and switching questions, and, like
the text, not a reload. Adding a file only makes an inert local chip; the same trusted Send
uploads each answer's files under that answer's own message ID and publishes them with its
comment, so one answer's refused or lost upload fails that answer alone, before any comment,
and its chips and text stay. A file in transfer holds back its own answer only. A recipient
retry is an Ask over the recorded comments and uploads or publishes nothing. A verified
answer shows its images once, in its answer block, the way a comment shows them; its
comment in the Conversation keeps its text.
Sending freezes the answer/selector/thread/recipient snapshot, holds its controls
while the other compact rows remain available for reading. Long-answer Enter
inserts a newline. Digits select choice tiles only from an opened answer panel or tile,
never while typing in a text or recipient control. A composed-cap refusal shows
`These answers are too long to send together. Send fewer answers.` with no effects.

An admitted multi-answer Ask shows `Asked <recipients> with <N> answers.` on its
participating questions; a single answer keeps `Asked <recipients>`. Replies render
once in the canonical first-recorded question's conversation. Other participating
questions with a reply show `Open reply`, named `Open reply · <canonical question
label>`, which opens that question and focuses its reply after current placement is
admitted. Recipient retries retain the same recorded batch, per recipient, using
the existing AskAgainAction; they never republish its comments or appear under
every participating question. Change answer is an ordinary one-question follow-up.

The renderer's existing text-map owner produces a normal QuoteSelector for the
rendered question text and its surrounding context. The UUID never appears as a
synthetic quote. Inactive authored questions collapse through a renderer-managed
visual attribute, never `[hidden]`, and remain in this canonical text map. Switching
disclosure changes and reloading therefore preserve quote context and verified answer association.
Existing verified annotation threads whose selectors resolve
inside that block are associated cosmetically; the frame cannot supply comments,
answer state or recipients. Concurrent first answers may create sibling threads.
The canonical thread prefers associated threads holding a nondeleted verified
device answer, ordered by writer/thread identity; only when none has an answer
does it use that identity order across all associated threads. Its latest device
answer wins by Lamport/writer/message order, and later answers append there. An
empty later sibling cannot displace an answered thread. Agent replies never mark
Answered; sibling history stays visible.
Removed markup leaves its ordinary annotation in Comments; removal of quote text
detaches it through normal annotation resolution. Parent `inline-slots.ts` owns
one proposal/question measurement batch, preserving sibling positions and focus.
Agents read these ordinary annotations through `tmt colab threads PAGE --json`.

The existing `colab.render.questions` parent request is a closed
`{type,renderId,requestId,blocks,threads,groups}` object, at most 256 KiB.
Blocks are `{id,title}` (at most 50); threads are `{id,selector}` (at most 1000).
Groups are closed `{id,title,questions,active}` objects, plus `form:"chip"` for an opted-in
group and nothing else (at most 50), with a
canonical UUID, a bounded title or `null`, and 1–12 unique admitted question IDs.
Each block belongs to exactly one group; `active` is one of its members. An internal
lone group has its member's ID and a `null` title. The frame neither parses nor
supplies authoritative groups. Its closed response is
`{type,renderId,requestId,questions}`, with unique `{id,selector,threads}` objects
drawn only from the current request, exact quote text equal to the admitted title,
and only requested thread IDs. Unknown fields, stale IDs and over-budget responses
are ignored. No answer, recipient, count or activation authority enters this channel.

### Recommended reading

A reading anchor is inert page markup asking a person to read one passage:

```html
<tmt-read
  data-id="10000000-0000-4000-8000-000000000401"
  data-agent="Agent name"
  data-why="Read before deciding"
>
  <h2 data-title>Hosting</h2>
  <p>The passage to read.</p>
</tmt-read>
```

Its canonical lowercase UUID shares the question/group/review namespace. Exactly one immediate
`data-title` child supplies a whitespace-normalized title of 1–200 code points, without control
characters. `data-agent` is required (1–64 code points); `data-why` is required (1–200 code points).
Neither attribute permits control characters. At most 50 reading anchors are admitted per page.
Reading anchors cannot contain, or sit inside, reading anchors, questions, question groups or
review sections. Invalid anchors remain ordinary readable content without trusted controls.
Keep an ID for the same reading request; use a new ID when a changed passage needs a fresh read.

A trusted parent `Mark read` records an ordinary verified device comment on the heading's
annotation thread, then the existing person Resolve. The fixed comment is `Read “<title>”`,
`Reading: <full UUID>`, `Page: <page title>` and the uniquely resolved asking agent's mention,
each on its own line. Only that exact device-comment form associates the reading ID; HTML,
scrolling, agent replies and agent Resolve cannot mark it read. The effective person Resolve
carries the same publisher-assertion limitation as review closes, not a new authentication claim.
The canonical thread prefers an admitted reading comment, then the existing stable thread order.
The existing review-close notification guarantee applies: one attempt per committed person
Resolve, with frozen operation IDs and only the named agent as recipient. Other mentions in the
heading, reason or passage do not add recipients. Unknown or ambiguous agents leave a visible
notification warning rather than guessing a recipient. A retry reuses an already recorded
comment; reload and reconnect never resend a notification. No separate request store is added.
Removing the markup removes its controls while its ordinary thread record remains.

The shared parent band sits immediately after the passage title, before its content. It uses
`Please read · <agent>: <why>` and trusted `Mark read` / `Next ↓` actions. `Mark read` is enabled
only when the page's discussion, status coordinator and the passage's selector are all live; until
then it is disabled, a muted `Connecting…` appears beside it on the same row if the wait passes
400 ms (its live region says `Mark read is available once the page has finished connecting.`;
nothing below the band moves), and a click that cannot act reports a failure instead of doing
nothing. After a person read it
becomes the neutral `Read` done band in the same spot, with the recorded time. Only an accepted
notification (including an admitted reply) says `<agent> notified`. Held, refused or unavailable
delivery says `<agent> not notified`; uncertain or missing outcomes say
`<agent> notification unconfirmed`. The named recipient is the admitted notification recipient,
not a later authored requester rename. A real `Why` button after `Next ↓` toggles the muted
reason below, with `aria-expanded` and `aria-controls`; it never retries or publishes. There is
no hover-only reason. Archived/read-only pages keep state without actions and expose any reason
as static text. Outcomes come from the existing status-notification projection, not Resolve or
Send adoption. No display path retries.

The parent exposes an ordered `{id,title,state}` projection (`unread` or `read`) for navigation.
Jump and `Next ↓` only scroll and highlight a currently resolved heading; they never open a
thread or publish. `read=<canonical UUID>` is an optional navigation hint on the existing owner
fragment. It survives full/short-page routing, waits for the current heading resolution, and is
inert when missing or stale. It is not accepted on hosted reader links. Explicit Ask capture
recognizes only that exact owner-page hint and removes it from the admitted page URL; other
query/fragment members remain refused rather than being stripped.

The existing `colab.render.questions` exchange also carries `readings` of `{id,title,closed}`
(at most 50) and returns only requested `{id,selector,threads}`. Heading quotes must equal the
parent-admitted title and references may name only requested threads. The separate cosmetic
`colab.render.scroll-reading` request has exactly `{type,renderId,requestId,id}`, bound to that
current exchange and a resolved reading ID; it has no publication authority. Unknown fields or
stale requests do nothing. Reading slots consume only the remaining existing 250-slot budget,
in document order, and never displace an existing proposal, question or review control.

### Review sections

A review section is authored page HTML, not a new discussion record, CLI command or concept. It
asks the reader to confirm one part of a page, and underneath it is an ordinary annotation thread:

```html
<tmt-review data-id="10000000-0000-4000-8000-000000000301" data-agent="Agent name">
  <h2 data-title>Hosting plan</h2>
  …the content to confirm, which may hold answer blocks…
</tmt-review>
```

`data-id` is a canonical UUID used only for control identity; it shares one namespace with
questions and groups and never appears as a quote. The section has exactly one immediate
`data-title` child, whose whitespace-normalized text (1–200 code points, no control character)
is its title. `data-agent` names the asking agent (at most 64 code points). At most 50 sections
are admitted per page, and a section may not sit inside another section, a question or a question
group. `tmt-question` and `tmt-questions` may sit inside a section. An ignored section supplies no
controls, and its questions remain ordinary questions. The page markup check below owns the
admission rules and reason codes.

**Thread.** The renderer's existing text-map owner produces a normal QuoteSelector for the
heading text and its surrounding context. A section's thread is an ordinary anchored annotation
thread whose quote is exactly that whole heading; a thread anchored on part of the heading, or
anywhere else in the section, stays an ordinary annotation. Only the reader's explicit action
creates it, never render, sync or reload. Concurrent first actions can create sibling threads. The
canonical thread prefers one holding a nondeleted verified device section comment (below), then
orders by writer and thread identity, like an answer block's.

**State.** The parent derives it from the canonical thread only; page HTML never supplies it.

- _Closed_: the thread's effective status is a Resolve whose `actor` is `person`. A
  `threads resolve` by an agent is labelled `agent` and leaves the section waiting. The label is
  the publisher's assertion, so this guards against an agent confirming in the reader's place; it
  is not authentication, and page HTML can publish no status at all.
- _Changes requested_: not closed, and the newest verified device section comment is a Needs
  change note with no admitted agent reply to it.
- _Waiting_: anything else, including a section with no thread yet.

**Section comments** are fixed plain text. The head is `Confirmed “<title>”` (optionally
`: <note>`), `Needs change to “<title>”: <note>` or `Answered “<title>”`, followed by
`Page: <page title>` and the visible recipient mention text. A Confirm note is one line of at most
200 code points without control characters; a Needs change note is required and at most 8192 UTF-8
bytes; the whole comment stays within the 16 KiB comment limit. Only verified device comments
record an action: agent replies and bridge comments never do.

**Actions** are explicit trusted parent activations. Choosing, rendering, reloading and syncing
publish nothing.

- `✓ Confirm and close` records the Confirmed comment, naming the asking agent when `data-agent`
  resolves to one directory entry (page names grant no routing authority), then publishes the
  person Resolve through the thread-status coordinator, which tells the named agent once. A
  thread an agent resolved is reopened first. If the comment lands and the Resolve fails, the
  section stays waiting and a retry in the same tab publishes only the Resolve.
- `✎ Needs change…` records the note and sends one Ask per recipient through the answer-block
  path. It publishes no status, so the thread stays open and the section shows _Changes
  requested_ until the agent replies. The note applies to the whole section; a reader who wants to
  point at one spot uses the normal Annotate.
- `Reopen`, offered on a closed section, is the ordinary person Reopen and notifies no one. An
  agent's `threads reopen` has the same effect on the section.
- Answering the questions inside: when an explicit Send of answers leaves every admitted question
  inside the section, grouped or lone, with a verified current device answer, that same
  activation then publishes the section's person Resolve (with an `Answered` opening comment and
  no mention when the thread is new). A partial group does not close it. If the Resolve fails the
  answers stay recorded and the section stays waiting; a state is never derived as closed.
  Changing an answer later does not reopen the section.

**Presentation.** The parent draws one band below the section in the shared needs-you look: a
light band with no frame, the mark plus a state word, who and why in neutral text, and inline text
actions on the right. `◆ Please confirm` names the asking agent and the section, a requested change
shows a neutral `◇ Changes requested` without the fill (it waits for the agent), and a closed section folds to one neutral line (who closed it,
when, the note) with Reopen. The note, the fields and `Goes to @<agent>` or `Stays on the page, no
one is asked` follow. It follows Colab's effective theme and takes its position from the inline slot channel (`{id,height}`, no
answer geometry). A closed section fades but stays expanded: the frame sets a renderer-managed
attribute from the parent's closed set, so its content stays in the DOM, selectable and in the
text map, and annotations inside it keep resolving. The page head's `Needs you N` menu counts
each waiting review section once, alongside undecided/unresolved proposals, unread reading anchors and unanswered question
blocks. A question group with any unanswered questions counts once; a standalone question is
one block. Changes requested from an agent and closed sections do not count. IDs are never
counted twice. These are projections of the admitted layer state after anchor resolution, not
stored counters. They are separate from `openThreadCount`: an open section thread also counts
in Comments like any open annotation, while a waiting section with no thread still counts in
Needs you. Archived pages retain cosmetic navigation without publication controls. The owner-list
count uses the same definition over its verified static-source fold (see [Page summary](#page-summary-2588)); collection per-item details are separate.

The header menu groups targets in proposal, reading, answer and review order, omitting empty groups.
Search results and in-app page links expose real hrefs. Modified clicks retain native browser
activation; Ctrl/Cmd+Enter opens the selected result in a new tab and Shift+Enter in a new
window, leaving the search open in the current tab. Plain click/Enter retain existing navigation.
Header target links open the full page, with the existing reading hint for reading targets;
ordinary header selection retains the local scroll/focus behavior below.
Picking a target only scrolls and focuses its first available action (or its retained content
when read-only); a compact question group opens on its first unanswered question. It never
marks read, answers, confirms or sends. At zero the header shows neutral `✓ Nothing waits`,
without a menu; compact headers show `✓` with the same accessible name. If the list empties
while the menu is open, that mark takes keyboard focus rather than the body. Neither a menu count
nor a rendered target grants writer admission.

**Channel.** The `colab.render.questions` request carries a closed `reviews` array of
`{id,title,closed}` (at most 50, unique IDs, none shared with a question, group or reading anchor) and its
closed response carries `reviews` of `{id,selector,threads}`, drawn only from the current
request: the quote must equal the admitted title and only requested thread IDs are accepted. The
frame returns, for a section, only threads whose quote is exactly the whole heading. `closed`
only changes the fade, so a change of it keeps the resolved anchors; a new title does not. Unknown
fields, stale IDs and over-budget responses are ignored.

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
Close never resolves or sends. The anchored collapse action uses ChevronUp with accessible name "Collapse thread" and help "Collapse · Esc"; Resolve uses CircleCheck, named "Resolve thread" in the anchored window and "Resolve" in the drawer. Existing writer-owned Resolve collapses only after
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
its binding; repeated mentions of the same pair count once. Each fresh message in
Chat, an annotation or a reply seeds the admitted
`creationRecipient` when the page/viewer-device creator toggle is on (the default).
Only one current admitted UUID match supplies a chip and name; missing/ambiguous
admission or disabled talk hides the toggle and seeds nothing. Initialization waits
for the first directory result; typing first suppresses the default. Removing a
chip suppresses it for that message only, never reseeding during directory replacement.
When that draft has no bound mention and one creator is resolved, the existing
hint makes the comment-only outcome explicit. Its `Ask @<creator>` action inserts
a manual bound chip plus a space at the start, preserving text and shifting the
selection; it changes no preference and sends nothing. Chat uses the chip as its
recipient cue; annotation/reply hints derive their audience as before.
An explicit on adds a missing creator chip to the open draft; off removes only a
proven default-added chip, preserving manually typed/picked mentions and surrounding
text. The preference is a storage-safe, device-local convenience, never authority.
Provenance stays in editing/page memory and is stripped from the unchanged encrypted
v1 draft format; restored unknown-provenance mentions count as manual.
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
same editor, draft and caret. Directory recovery uses only the page's reconnect (automatic
and explicit) or Try again. A plain comment remains available under content-write admission while
directory discovery loads or fails.
Composer copy comes from the page connection owner: ended shows the same ended
sentence/action as the page, reconnecting shows "Reconnecting…" with Send disabled
and the draft retained. Connection guidance takes precedence over recipient/discovery
hints, but never hides an explicit Send failure. Every pre-publication refusal keeps
the draft and displays a composer-local `role=alert` directly below the field,
with plain failure guidance and a closed reason reference. Only the failure mark
is coloured. Empty drafts keep Send disabled without an alert. That notice survives
connection changes and automatic editor normalization; a deliberate text edit,
explicit retry or successful Send clears it. Repeated
refusals are announced again. Already recorded or uncertain turns retain their own
status: S2 and S13 use neutral saved text with no failure mark or error colour,
still announced as alerts. A post-record failure clears the composer draft, keeps
the recorded fence and existing thread action, and never offers a resend.

Send references describe only the local branch, never error text or identifiers:

| Reference | Branch                                                 |
| --------- | ------------------------------------------------------ |
| S1        | Another Send is busy                                   |
| S2        | A prior message was recorded with an uncertain outcome |
| S3        | The page is archived                                   |
| S4        | Posting is on hold                                     |
| S5        | The discussion writer is unavailable                   |
| S6        | Current recipient or write admission refuses Send      |
| S7        | Mentioned recipients have no Ask binding               |
| S8        | The draft is empty                                     |
| S9        | Capturing the message failed                           |
| S10       | Attachment preparation refused                         |
| S11       | Attachment preparation threw                           |
| S12       | Recording the comment failed                           |
| S13       | An unexpected failure followed recording               |
| S14       | An attachment became stale                             |
| S15       | The own-record leader refused a record conflict        |
| S16       | The own append was refused                             |
| S17       | The Remote session ended                               |
| S18       | The Remote session was evicted                         |
| S19       | The Remote sequence is unavailable                     |
| S20       | Evaluating Send admission threw                        |

These references are ephemeral presentation state, never persisted, sent, logged
with message contents, or used as admission authority.

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

Answer batches (#2875). One trusted Send of an answered question group publishes one answer
comment per answered question, each in its own question thread, then asks each recipient
once. That Ask is the ordinary signed send input: `thread` is the canonical thread, the
thread of the first recorded answer in tab order, and `messageIds` are the sorted, unique
IDs of every recorded answer comment of that Send. No signed field, record kind, stored
record or Remote path changes. Before capture the parent rechecks that each comment is this
device's own undeleted answer comment (`Answer to “<question>”: …`) to a question of the one
`tmt-questions` group the admitted page source names, in an anchored non-proposal thread at
its recorded revisions, and that the answers follow the group's tab order with no question
twice; otherwise nothing is asked. The frozen message is the ordinary frame (page, link,
`Quote:` the canonical thread's anchor text) whose `Comment:` is `Answers to <N> questions:`
and one `<question>: <answer>` line per answer in tab order, without each answer's page and
recipient lines, and with no earlier conversation. A message over the 64 KiB cap is refused
before any comment is published. A failed comment means not confirmed recorded, including
an error before publication or a lost acknowledgement; it is excluded from this Ask and
its draft remains for a later explicit Send, which publishes it again. A late first
comment may leave an extra record superseded by `currentAnswer`, but no agent is asked
about that failed outcome. There is no same-ID recovery handle. One recorded answer is
the single-comment Ask above. The
reservation key and the `retryOf` check use the sorted message-ID set (the single-comment
key is its one-element case, unchanged), so a failed or uncertain batch Ask is retried as
the same batch for that recipient, never split and never per question. `conversationAsks`
returns a batch Ask for every question thread holding one of its comments; its reply, reply
notification and captured earlier conversation belong to the canonical thread alone. `Change
answer` is a new comment and an ordinary single-comment Ask; the batch is untouched.

The Comments overlay lists every annotation and page-level thread, with a quote
snippet, participants, display time and open/resolved status. Selecting a row scrolls
the window to its anchor and expands that thread inside the overlay; a margin marker
reopens the same thread in its anchored window. Page-owned per-thread drafts also
survive switching to the explicit Comments details view. Agent replies join verified Ask records to comment IDs and
remain attributed to their agent. Overlay geometry does not reflow page content.
Shared Chat, Source, Agents, Files, Comments and Export drawers leave an 8 px
header/right/bottom gutter (4 px through 480 px), with top/right/bottom safe-area insets, an opaque
square surface and a 1 px border. The header remains visible behind the scrim. All drawers use the shared leaf
Drawer with native modal focus containment at every width. The first header or
author-frame backdrop press dismisses only, without click-through; a second press
acts. The shared Close button retains draft/attempt owners; Chat keeps its audience
caption. Action and Manage radio menus use the shared top-layer Menu; autocomplete
remains a listbox. A nested menu consumes Escape before its drawer.
A trusted non-editing `@` key opens the existing Chat under the same admission as
the header Discussion action: a mounted, unarchived page and no other menu or
dialog. An anchored annotation does not block it. Only a mounted editable composer
receives focus and one literal `@` on opening; an already open Chat only receives
focus. Modified keys other than Shift, repeat and IME confirmation are ignored.
The existing draft, mentions and Send admission remain unchanged; no talk-scope
probe or dispatch occurs on opening.

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

### Owner page and space-home header

Owner page and space-home header presentation follows the [page-chrome reference](../../../.agents/skills/tmt-colab/references/page-chrome.md): one row that starts with the TMT mark and Colab (below 480px the mark only, the wordmark kept in the brand link's name), an original-author caption next to the title, and compact actions in More. The display-only Remote identity (`local · <device>`) names and titles the brand link; it never supplies connection state or authority, and the connection chip retains its existing owner (below 480px its words lead the More menu as a non-interactive line instead of a visible header chip).

### Transient reply notifications (#2726, #2727)

Chat, anchored Thread, Answer and Proposal replies use the shared presentation-only FIFO queue.
The parent reads admitted page Ask finals from this device's ledger and new live
comments in conversations where the authenticated Yours writer has participated.
Names, author HTML, renderer messages and Send outcomes cannot admit a reply.
Initial history and the binding's first available current view establish baselines,
including when it follows a cached loader snapshot; repeated
reads, edits, reconnects and remounts do not replay arrivals. Stable page/viewer,
writer and record identities select events; no new signed record or storage field
is introduced. Answer destinations use the question layer's checked association;
Proposal destinations use admitted proposal threads. Neither body labels nor
proposer display metadata can establish participation or choose a destination.

A destination already visible and following the history end suppresses its
arrival. Trusted activation opens the existing Chat Drawer and reveals the exact
reply, or scrolls the canonical Thread quote and opens its anchored window with
the existing unresolved-anchor fallback. It never sends or creates a conversation.
Actual Drawer/Menu/Manage/attachment owners, an open anchored Thread window and
document visibility suspend the queue; closures cannot resume it while another
active surface remains. Held items retain FIFO order and do not re-announce.
Before display or announcement,
resume rechecks every retained/pending event against the latest admitted view and
silently drops withdrawn destinations. Header counts remain the persistent record.

Answer/Proposal arrivals are suppressed only when their exact reply is in open,
visible block history. Activation selects the answer tab or unfolds proposal
history, scrolls and focuses the existing block; detached proposals use Comments.
The pointer-visible arrival outline uses the focus token and clears on blur or
the next pointer/key interaction. Activation never publishes a decision or Reopen,
creates a composer, or changes Send/recipient admission. Busy clicks are retained
behind the existing fence for at most five seconds; pending activation and reveal
are cancelled on page change, withdrawal or timeout with no late visible effect.
The destination remains accessible through normal Chat/Comments or inline blocks.
All four tags announce once on first display through the leaf's existing polite
region, deferred while suspended. There is no legacy announcer, relocation or
second event observer. Presentation grants no routing, recipient, Send or storage
authority.

### Page Chat (#1645)

Discussion → Chat and the floating Chat icon beside the selection-mode control
open the same retained Chat drawer without resizing or reflowing the page.
The floating Chat control hides while selection mode or an annotation window is open;
Close/Escape or a backdrop press returns focus to its opener. No press sends a message.
At compact widths the modal drawer covers the floating action: Close/Escape dismisses;
at wide widths a press at the exposed floating location dismisses through the backdrop.
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

Eligible verified replies in this device's Chat conversations carry a persistent
mark plus `Unread`, without a count, on the floating control; the Chat menu entry
also reads `Chat · Unread`.
The device-local seen position records reply identities, scoped to space, page,
epoch and viewer device; it is presentation only, not a signed read receipt.
Only a visible Chat history at its end advances that position. Closing, reload,
notification expiry/dismissal and reading older history do not acknowledge replies.
Other people's conversations and this device's own comments do not raise the mark.

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
Chat and annotation threads share flat, unboxed turns and the existing 3px agent
rail. Chat uses the shared Drawer header with `Chat`, a `○ Everyone with access` caption and
a `Close Chat` action; its identity row places the muted time at the right.
Spacing separates turns, while local-day headings derive only from admitted
timestamps. An empty Chat shows `No messages yet` and teaches `@` only where
that action is admitted. The pinned composer has one top rule, the existing
Attach and recipient controls at the left and Send at the right;
`Posts as a comment.` appears only without a recipient. In Chat, a manually edited
draft without the creator mention hides the creator-default toggle; a fresh default-only
draft shows it again. The stored preference and send decision are unchanged. Failed delivery shows
`✗ Not delivered` and the existing ask-again action labelled `Retry`; waiting,
held and uncertain states retain their existing recovery and announcements.
Thread actions retain their existing Lucide icons and accessible labels. Agent activity follows the bounded working-state observation above; after its two-hour horizon the status is static and keeps its accepted ledger state and read-only recheck action. No
elapsed time authorizes a resend or changes durable ordering. Chat text never enters
the author-code renderer; highlights carry only anchor IDs, quote selectors and bounded paint-only ordinals.
The internal highlight message separates an optional unsaved quote from numbered anchors;
its admitted thread count bounds unique positive safe-integer ordinals.
Saved-anchor positions are exact `{id,top,rect}` entries with one known unique ID,
a finite bounded navigation top and a final-text rect using the existing selection
rectangle keys/bounds. The parent keeps one position map and clamps cosmetic window
placement to the canvas below trusted chrome. Rectangles remain untrusted display
claims and grant no open, send, focus or routing authority. Invalid anchor
keys, duplicate IDs/ordinals or out-of-count ordinals reject the whole message.
Numbers derive from revision-1 author-clock creation time over non-deleted threads,
with writer/thread UUID ties; deletion renumbers and resolved threads keep their
list number but have no page marker. The same number labels the list, thread header
and hover hint. Creation time and ordinals are presentation only, never trust or
dispatch claims; signed records and Remote/native wire protocols do not change.

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

| Type            | Additional fields / behavior                                                                                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello`         | `device, membershipRevision, cursors`; authenticated session/proof from upgrade, page/epoch admission before catch-up                                                                       |
| `catchup`       | `membershipHead, baseline, optional baselineObject, streams, more`; bounded pages of each stream's namespace checkpoints and subsequent tail                                                |
| `subscribe`     | `cursors`; observe only the admitted page/current epoch                                                                                                                                     |
| `append`        | `streamId, seq, envelopeHash, envelope`; create-only, exact frozen retry returns original receipt                                                                                           |
| `save`          | `operationId, baseSha256, sourceSha256, source[, attachments \| hunks]`; owner device only; the browser's Save, prepared and committed natively                                             |
| `savestatus`    | `operationId`; owner device only; what the page recorded for an earlier `save`                                                                                                              |
| `saveresult`    | `operationId, state` and `revision[, merged, sourceSha256]` or `code, message[, conflicts]`, plus `childPage` where a `task.convert` made a page; the one reply to a `save` or `savestatus` |
| `work`          | `operationId, action, body`; owner device only; one closed work action of the page ([Work](#work-4050)), prepared and committed natively                                                    |
| `workresult`    | as `saveresult`, plus `taskId` for a committed `task.add` and `childPage` for a `task.convert`; the one reply to a `work`                                                                   |
| `usage`         | no fields; owner device only; how full the page is                                                                                                                                          |
| `usageresult`   | `usage` ([Page usage](#page-usage-and-requested-compaction-2846)) or `error: {code, message}`; the one reply to a `usage`                                                                   |
| `compact`       | no fields; owner device only; combine the root-local device's own stream                                                                                                                    |
| `compactresult` | `report` (the compaction report) or `error: {code, message}`; the one reply to a `compact`                                                                                                  |
| `receipt`       | `streamId, seq, envelopeHash`; durable acceptance, not task completion                                                                                                                      |
| `broadcast`     | `streamId, seq, envelopeHash, envelope`; subscriber must verify before applying                                                                                                             |
| `checkpoint`    | `streamId, namespace, seq, envelopeHash`; a notice that the serve stored a paired checkpoint; grants nothing                                                                                |
| `ack`           | `cursors`; scoped delivery positions only                                                                                                                                                   |
| `awareness`     | `device, data`; bounded ephemeral presence, never persisted or authority                                                                                                                    |
| `error`         | `code`; one of DENIED, EXPIRED, STALE_EPOCH, INVALID, GAP, CAPACITY, CONFLICT, RESYNC_REQUIRED                                                                                              |

### Implemented stream subset (#1156, #1166)

The externally driven local sync module implements the strict operations below.
Registration is #1162; `serve` composes mounted socket sync in #1211. Remote owns upgrade admission and
supplies the authenticated principal. The module takes an already-upgraded
nonblocking duplex stream, not HTTP headers.

Every client message is one UTF-8 JSON object with exactly the common fields
`version:1, type, space, page, epoch` and the operation fields listed below.
Epoch is a positive canonical decimal string. Duplicate/unknown fields, nulls,
wrong types, noncanonical values and unsupported operations reject.

| Client type  | Exact additional fields                                                 | Implemented behavior                                                                                                                                                                                                                                                                                                                         |
| ------------ | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello`      | `device, membershipRevision, cursors`                                   | Device matches principal; one successful hello per connection starts server-driven catchup, then live delivery.                                                                                                                                                                                                                              |
| `subscribe`  | `cursors`                                                               | Empty list starts live delivery; nonempty list resolves cursors and starts catchup, then live delivery.                                                                                                                                                                                                                                      |
| `append`     | `streamId, seq, envelopeHash, envelope`                                 | Inline update or object reference; verify complete exact bytes and durably append before receipt. Kind `checkpoint` is accepted only for the sender's own stream, namespace `own`, at its head (`streamSeq` the head, `prevHash` the head hash); it is stored and receipted, then announced to subscribers as a `checkpoint` notice (below). |
| `chunk`      | `objectId, envelopeHash, index, count, bytes`                           | Complete the connection's pending referenced append or save; no standalone upload or partial append.                                                                                                                                                                                                                                         |
| `save`       | `operationId, baseSha256, sourceSha256, source[, attachments \| hunks]` | Owner device only. Inline source or `{objectId}` plus chunks; see Browser Save below.                                                                                                                                                                                                                                                        |
| `savestatus` | `operationId`                                                           | Owner device only. Answers from the root-local operation record; see Browser Save below.                                                                                                                                                                                                                                                     |
| `work`       | `operationId, action, body`                                             | Owner device only. Runs the write command's admission, then commits as the root-local writer; see Browser work actions in Work.                                                                                                                                                                                                              |
| `usage`      | none                                                                    | Owner device only. One `usageresult` from the serve's own snapshot; no state changes.                                                                                                                                                                                                                                                        |
| `compact`    | none                                                                    | Owner device only. One `compactresult` after the same combine `tmt colab page compact` runs, under the sync lock.                                                                                                                                                                                                                            |
| `ack`        | `cursors`                                                               | Resolve retained scoped positions and release one frame credit; no deletion, core acknowledgment or application authority.                                                                                                                                                                                                                   |
| `awareness`  | `device, data`                                                          | Device matches principal; at most 4 KiB canonical base64url bytes, ephemeral.                                                                                                                                                                                                                                                                |

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
An admission check or catchup that cannot read the store because SQLite stays busy or locked past
its 2 s timeout answers `RESYNC_REQUIRED`, never `DENIED` (#2956): it says nothing about the
device, and the client resyncs. A refusal the store answered keeps its code.
An append that cites an earlier membership revision than the head is admitted when every membership
statement after the cited revision, up to the head, is page-scoped (see Page scope under the
statement table) and names a page other than the append's own: a page created or shared elsewhere
changes nothing for this page or this writer, and every authority fact (revocation, chain, role, page
policy, epoch) is read at the head in the same snapshot. Any other statement in that window, one that
names this page or is not page-scoped, leaves the append stale, and it answers `RESYNC_REQUIRED`,
never `DENIED` (#4088): the session's view is out of date, which says nothing about the device. A
window of more than 256 statements also answers `RESYNC_REQUIRED`. A revision the head has not
reached is `DENIED`, and so is one before the device certificate's own `membershipRevision`
(#4103). The refusal stores nothing. The owner browser keeps no copy of this rule: it retries the exact
bytes of an interrupted own append, sends nothing again when its catch-up shows those bytes already
at the stream's head (the receipt was lost), and drops one only when the serve answers
`RESYNC_REQUIRED` after the session has caught up past the revision the entry cites, because the
signed header cannot change; it then seals the next write under the head it caught up to.

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
`checkpoint` has `streamId, namespace` (`content` or `own`), `seq` and
`envelopeHash`, and nothing else: it names a checkpoint and carries none of it.
The serve sends it to admitted subscribers only after the checkpoint is stored and
paired, right after the receipt or result of whatever stored it: a browser's
checkpoint `append`, a `save`, a `compact`, `tmt colab page compact` or the combine
after a native write. An exact retry of a stored checkpoint sends nothing. A notice
grants nothing. A subscriber records it as an unseen cut, unless it already holds
that cut or a later one (a stale notice changes nothing); editing, rendering and
Save continue unchanged, and no cursor, cut or revision moves. The page revision
counts every checkpoint the serve holds, so a read fenced by that revision (a
document file, the export) first settles an unseen cut: the subscriber reopens its
connection once, and catch-up admits the checkpoint, verifying everything it
verifies today. A cut still unseen after that one reopen fails the read as
`changed`; nothing reopens a second time or on a timer. An ordinary reload or
reconnect admits checkpoints through catch-up as always. A notice that is not
exactly this shape closes the connection. Server
awareness has `device, data`. Scoped errors have `code` from the table above.
Malformed, oversized, binary or inbound server-only frames close with code 1008
and reason `INVALID`. Capacity never evicts accepted receipts or payloads.

#### Browser Save (#2032)

The browser publishes its whole source over this socket, never as a content update it signs
itself. Only a registered owner device may send `save`, `savestatus`, `usage` or `compact`; readers
and public readers are denied by the same per-frame admission as `append`, and the socket gains no body route and no
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

`hunks` is optional and is how an edit survives a change made elsewhere: the editor's change as
anchored hunks against the base it opened, `[{oldStart, old, new}]`. Each `old` is non-empty whole
lines (the removed lines with up to three lines of context on each side), `new` is what replaces
them, and `oldStart` is the 1-based line of the base where `old` starts. At most 64 hunks, and the
whole frame stays within the 64 KiB frame limit, so an edit too large to anchor in one frame is
saved as a whole source, as before. The serve uses them only when the page moved since the base
(`baseSha256` no longer matches the page): it places them on the page as it now is with the rules
of `page patch --diff` (each `old` must match exactly once at a line start, or once at its stated
line when it occurs more than once; all hunks apply or none does), prepares that result against
the page and commits it as for any save. A page still at the base is saved from `source` and the
hunks are not read. A page that moves again between the evaluation and the commit has its hunks
evaluated once more, at most three more times, and then refuses as `COLAB_STALE_BASE`. A save with
`hunks` and `attachments` is `INVALID`: an attachment change is bound to the editor's own source,
which a merge replaces. The browser tests each case with the shared evaluator's rules and never
evaluates hunks itself.

`saveresult` has `operationId` and `state`: `committed` or `unchanged` with the opaque `revision`
(a `committed` result of a merged save also has `merged: true` and `sourceSha256`, the digest of
the source the commit holds, which the editor never had, so it can tell when its own view shows
it), `rejected` with the stable `code` and a person-readable `message` (`COLAB_PATCH_CONFLICT`
also has `conflicts`, the 0-based indices, ascending and as sent, of the hunks that do not fit,
and nothing is written), or, for `savestatus` only,
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
no credential. Pages returns exactly `{spaceId, ownerKey, revision, projects, pages, pageIds}`;
`projects` is the space's [project names](#projects-3747), sorted by code point;
`pageIds` is sorted, at most 1,000 entries, each exactly `{pageId, deleted}`. It includes
the existing retained deleted-page IDs solely to prevent historical prefixes rebinding;
no title/content of deleted pages is exposed. Its non-deleted IDs exactly match `pages`.
Browser discovery caps the complete serialized metadata response at 2 MiB.

A serve running from an installed release (`<lib>/tmt-colab/releases/<id>/tmt-colab`)
follows the install's `current` selection (`serve_release.rs`); version direction belongs to
installation, not Colab. A different release with a different receipt version is checked
at most every five seconds while serving, including background and JSON modes. The release
must remain under the same `releases` directory, with an owned regular executable matching
its receipt SHA-256 (bounded to 512 MiB and ten seconds). The selected executable's hidden,
read-only `serve --upgrade-check --json --no-open` confirms its bundled app, existing root
keys and supported store before any cutoff. It does not migrate, open a door or pair a device.
A failed check leaves the old service live and prints one warning naming both versions and
`Run tmt colab stop, then tmt colab serve --background.` The same session-admitted
`GET /api/serve-release` returns `{running, installed, restartFailed:true}` for that failed
selection; otherwise the existing `{running, installed}` or `{running}` shapes apply.
Versions are at most 64 characters of `[0-9A-Za-z.+-]`; malformed selections and development
binaries cannot start a replacement. A same-version reinstall remains unchanged.

A verified replacement waits up to 60 seconds for an idle point, then fences new requests
and peer writes. Admitted page saves, attachment work and commits finish before the old
socket and store close. The new service opens the store through ordinary startup, applying
its existing migrations. Normal stop first reaps only a Remote door this serve started;
an attached door stays up. The selected release then execs in place and performs normal
startup, attaching or starting Remote as usual. An owned door has a brief outage and Remote
reuses its saved port, so page links stay unchanged. There is no guardian, ownership handoff,
browser opening, pairing or installer hook. Failure after the
cutoff makes no rollback claim: `COLAB_RELEASE_REPLACEMENT` and the private failure record
name the selected release and manual recovery, and the service exits unsuccessfully.
An older serve that predates this behavior still needs the named manual restart. Browser
reload remains explicit and follows the existing draft rules.
`pages`is sorted by page ID, at most 1,000 entries, each exactly`{pageId, epoch, sharing, history, archived, retentionDays, lastUpdateAtMs,
expiresAtMs, warnings, projects}`; `projects`is the page's [display-only project set](#projects-3747). These time/expiry hints use the same verified projection
as`ls/show`, defined in [Local management CLI](#local-management-cli-1307).
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
the client gives the assembly no deadline of its own (see the catchup paragraph on
reconstruction). Partial bytes never advance
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
The client gives neither a reconstruction nor the whole catchup a wall-clock deadline of its own: a
reconstruction ends with the last chunk, a protocol violation (order, count, hash or size), or the
connection closing, and the catchup ends with the page's final frame, so a large page or a busy
server only makes the page open slowly. Liveness stays with the server's 30 s WebSocket ping and
the connection's close handling (#3975).
A read of the page's own objects (attachment configuration and content) has no deadline either: it
ends with its reply or the connection closing. An upload request keeps its own 15 s bound (#3982).
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
The same one-transfer rule and queue bounds apply, and the client gives the reassembly no
deadline of its own (see the catchup paragraph on reconstruction); a maximum accepted size
is not a delivery-time promise. Consumers retain
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
list. Append additionally requires a membership revision that the window rule above admits (the
head, or an earlier one when every statement after it is page-scoped to another page; any other
earlier one answers `RESYNC_REQUIRED`) and an allowed namespace, and returns the registered extension signing key for model signature
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

**Modes (#2383).** Human `tmt colab serve` starts a detached worker and returns once the serve
lock is held, `door.sock` is bound and the door is resolved (attached, started or local-only).
It prints `Colab is running in the background`, the status rows and the page link, `tmt colab
stop` and `--foreground`. `--foreground` keeps the process attached until Ctrl-C or SIGTERM and
says how to run in the background next time. Bare `--json` is the foreground serve (supervisors
keep owning the process they start); `--background --json` prints the same line plus
`"background":true`. The launcher opens the page after the handoff, with the same settings and
flags as before; the worker never opens it. Rerunning human `serve` while it runs starts
nothing and succeeds: `Colab is already running` and the page link, opened under the same
settings; `--json` modes keep the error `COLAB_ALREADY_SERVING`. The handoff lifecycle (exact self-exec worker, one
startup deadline, no kill authority past the accept cutoff, no PID signals, the second-start
rule) is written once in the
[extension-serve contract](../../../contracts/extension-serve-v1.md). Colab
adds only its readiness record (the status rows and JSON line), the codes
`COLAB_STARTUP_UNCONFIRMED`, `COLAB_STARTUP_CANCELLED`, `COLAB_ALREADY_SERVING` and
`COLAB_READY_OUTPUT`, and a private bounded `colab/serve-error.json` for a failure after the
handoff. A door the worker started stays the worker's and stops with it.

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
  seconds while retaining the unreaped leader, then attempts a bounded reap and read-only
  group-absence confirmation. Reaping and all pipe/watcher joins share a five-second cleanup
  bound. No destructive group signal follows reap. An unexpected exit is observed without
  reaping and its remaining group is killed before reap; a failed startup uses the same abort.
  A door that exits on its own is reported once and the local space keeps running. Signal,
  reap, absence or join failure reports cleanup unconfirmed, preserving the original error.
  Darwin's KILL EPERM clears only after bounded reap and read-only ESRCH. A SIGKILL of Colab itself cannot clean up; run `tmt remote serve`
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
`<origin>/colab/`), `paired` (`true|false|null` when unreadable or no door),
`devices` (count), `pages` (count of non-archived pages, `null` if unreadable), `page` (the
first page's full link, or its relative path without a door), `shortLink` (its short owner link,
`null` without a page or door), `next` (commands still
needed: `tmt remote pair` unless paired, `tmt colab page create --title <title> --layout column` when there
is no page) and `warning`. Human output is the `LOCAL SPACE` detail view with the same
facts and the next step: `door`, `paired`, `open` (the page link or `create one: ...`) and
`pair` (`pair for page access: tmt remote pair`, or `if this browser is new, pair for page access: ...` when
pairing is unknown), shown before `open` because the link needs a paired browser. Without a
door, `open` shows the relative path and the reason (the install line, or `browser access
unavailable: see warning`), never a command that `serve` replaces.

Bare `tmt remote pair` grants page access without sending. Sending an Ask or Chat to an
agent requires a yes at the pairing prompt, `tmt remote pair --talk`, or the owner enabling
sending for that device in Remote settings. Only a refusal the SDK verifies as `reason:"REMOTE_SCOPE_DENIED",
scope:"talk"` becomes the retained own-stream reason `TALK_NOT_ENABLED` and the specific
read-but-cannot-send notice, which also names `tmt remote devices talk <device> on` with the
row's own device ID; an absent scope or a recipient restriction keeps the generic
permission copy. No Ask identity, record shape or retry rule changes.

### Page links and opening the browser (#1614)

Every command that names a page tells a person where to open it, from one door and pairing
lookup per command (`tmt remote status --json`, then `tmt remote devices --json` while a door
runs; the same bounded calls as `serve`). `page create`, `ls`, `show` and the `share` commands
print the **short owner link**, `<origin>/p/<shortId>`, while a door runs. Remote owns that
in-place public forwarding; it does not bypass pairing or grant admission.
Colab retains the short location and uses validated Remote `tmt-mount` only for its internal
transport, as defined below. Without a door they print root-relative `/p/<shortId>` and the
reason: the install line when Remote gave no answer; Remote's own message for an error envelope,
with one shared wording for `REMOTE_SERVE_OUTDATED` (`The running Remote serve is older than
this Colab. Stop it with Ctrl-C in its terminal, then run tmt colab serve.`; the `serve` row then only says `(Remote serve is outdated; see warning)`; else `run tmt colab
serve to get a full link`; never a manual `tmt remote serve`. When no paired device is known, the same pairing step
as `serve` follows (`pair for page access: tmt remote pair`, or `if this browser is new, pair for page access: ...`).

`--json` results that name a page carry `path` (root-relative `/p/<prefix>`), `link`
and `shortLink` (the same short owner URL, `null` without a door), `paired` (`true|false|null`) and `next` (`["tmt remote pair"]` or `[]`). `ls` carries
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
and noninteractive-terminal suppression, using the shared `tmt-invoke` opener; like every
browser handoff it only prints the link under an agent (nonempty `TMT_AGENT`) unless
`--open` is passed. `--no-open` and `--json` suppress launching. Missing, deleted and ambiguous pages refuse
before opening. Archived pages print their link and open read-only in the paired local browser. An unavailable
service prints the current link/path and next step. Opener failure warns once and
retains the link. JSON adds `spaceId`, full `pageId` (null for home), `running` and
`opened: false` to the existing path/link/shortLink/paired/next facts; a stopped Colab
adds `tmt colab serve` to `next`. Browser navigation still undergoes existing admission.

### Short owner-page aliases (#1688)

Remote forwards `/colab`, `/colab/`, `/p/<id>` and `/read/<id>` without device context.
Colab validates Remote's absolute `tmt-mount` and returns catalog-independent bootstrap
bytes, never a redirect or existence-dependent response. Missing/invalid mount refuses.
Page and link identifiers use the Remote entry grammar; actual page-prefix resolution
happens only after SDK admission and verified catalog discovery. Mounted owner assets,
API and sync keep their existing admission and cookie scope; there are no root aliases
for them. Public recovery admits through the SDK before loading protected app assets
in place, retaining the visible short URL. Mounted legacy entry canonicalizes only
after verified discovery and carries its owner reading or anchor hint over. Internal mount and
space pins remain separate from visible location; reload and history use short paths, with at
most one display-only fragment target: `#t=<threadId>`, `#read=<readingId>` or
`#anchor=<elementId>`; short-page resolution retains the anchor until the full page opens.
A plain `#<elementId>` names the same element, and the location then
keeps it as `#anchor=<elementId>`. A fragment that names no display target is ignored: the page
opens at the top and the location drops the fragment. Only a fragment that carries `space`, `path`
or `seed` refuses.
Reader entry uses only existing public reader assets and never obtains owner authority.
Public responses for active, absent, revoked and archived/deleted-associated IDs are
identical apart from any echoed display ID; they disclose no title, page, space or author.

The shortest unique canonical page UUID prefix in the complete current catalog is the
`shortId`, with a minimum of eight ASCII characters. Archived pages participate even when a
listing hides them. Retained deleted IDs also participate, so a historical link cannot rebind
to a later page after deletion. No alias table, stored short ID, plaintext title or schema is added.
A missing catalog entry uses its full UUID rather than borrowing another page's prefix.
Previously copied aliases can become ambiguous when another page is created; they never
choose an arbitrary match.

After admission, the parent resolves 8–36 canonical lowercase UUID-prefix characters
against the complete verified metadata catalog, without decoding Yjs. A unique live match
opens the internal full-page route while retaining the short public path. Ambiguous aliases
show every matching page as a plain link row; missing or malformed aliases show the existing
unavailable-page view. A sole deleted match shows "This page was deleted"; deleted chooser
rows are disabled and have no title. Choosing a live row emits its shortest unique public
alias. Display title hints and thread, reading and anchor fragments confer no authority. Unexpected fragments
are refused both at discovery and during history navigation, without changing the mount pin.
Reader capabilities retain their fragment and are never interpreted as owner aliases.
New Ask Link values use short owner URLs; already captured Ask bytes are not rewritten.

`serve` (once the door is ready, attached or started) opens the page when the space
has exactly one, else the space home. `page create` opens its committed page using
the short link, including when an agent has no terminal or requests `--json`.
Automatic opening is skipped with `--no-open`, the setting `open` off, `CI`, an SSH
session without a display, Linux without `DISPLAY` or `WAYLAND_DISPLAY` (WSL
excepted), or no opener on `PATH`. `serve` also skips without a terminal or with
`--json`. The explicit `--open` flag overrides the setting and environment checks;
`--no-open` and a missing opener still refuse, and `serve --json` still suppresses
launch. Openers: macOS `open`, Linux `xdg-open`, WSL `wslview` then `explorer.exe`.
The human row reads `opened in your browser: <short link>` or just the link; a
failing opener warns once and keeps the printed link without undoing creation.
`serve --json` and `page create --json` report `opened: true|false`; page creation
still emits exactly one JSON document with the short-link fallback.

When Colab decides to open, it calls Remote's hidden `tmt remote launch <short-target> --open`
once through the invoking core (target `/colab/` or `/p/<id>`; 15-second deadline), and
Remote opens the browser and connects an unpaired one. The launch URL and its code never reach
Colab, its output or its logs. The answer `{version:1,link,opened,state}` with `state` one of
`opened`, `skipped`, `no-opener` or `failed` sets `opened` and the row; `failed` warns once.
A versioned `{version:1,error}` is Remote's answer: Colab prints the short link with that
message and does not open a second time, and a call with no complete answer warns that Remote
did not answer in time and opens nothing more. A usage error, an unknown version or state, or
unreadable output means an older Remote, and Colab uses the platform opener above, never a
guessed URL. A suppressed open (`--no-open`, `--json`, the setting or the environment) calls
no seam, so no offer exists.

`tmt colab settings [open on|off] [--json]` shows or sets the one setting, stored as
`{"open":true|false}` in `<dataRoot>/colab/settings.json` (default on; unknown keys ignored; a
damaged file reads as the default with a warning). JSON: `{open,source:"default"|"settings.json"}`.
A change replaces the file whole under the shared document lock, so concurrent changes each
survive and a reader never sees a half-written file; one that cannot get the lock within five
seconds fails naming the file and the exact retry, and a file past 4,096 bytes is refused, never
replaced.

### Firestore deployment declaration

`tmt colab deploy-declaration --json` is a fixed machine entry: `tmt remote deploy firestore`
runs it through the installed `tmt`, and no person needs it, so it is hidden from help. It prints
one line, the six-field envelope of [`remote-channel-v1.md`](../../../contracts/remote-channel-v1.md)
in this order: `version:1`, `extension:"colab"`, `backend:"firestore"`, `declaration`,
`declarationDigest`, `artifact`. `declaration` and `artifact` are JSON strings holding the exact
UTF-8 texts of `extensions/tmt-colab/firestore/declaration.json` and `admission.rules`, compiled
into the binary so the release checksum and receipt cover them; the digest is SHA-256 of the
declaration's bytes and the declaration's `admission.digest` is SHA-256 of the Rules bytes. The
reply is a pure function of the binary: it reads no Colab state, door, network, clock, directory
or environment, so it works on a machine that never started Colab. A reply that cannot be
produced (a text over 64 KiB, a digest that disagrees with the Rules, a failed write) is
`COLAB_UNAVAILABLE`, exit 1, never `COLAB_INPUT_INVALID`: Remote reads that code as "no
declaration" because an older Colab answers an unknown command with it. Only a malformed
invocation, which Remote never sends, can still produce it.

`contracts/vectors/deploy-declaration-v1.json` is that line byte for byte, with the composed
goldens (`deploy-declaration-v1.firestore.rules`, `.firestore.indexes.json`, `.plan.json`) Remote's
`compose_firestore` example writes from it. `deploy-declaration-reference.py` independently rebuilds
the line and pins `admission.digest`; run it without `--write` to verify and with `--write` after a
reviewed edit, then regenerate the goldens into a staging directory, review the diff and copy them.
Remote's `deploy_vectors` test parses the vector with Remote's own owners, so either grammar
drifting fails a build. Any change to the Rules or declaration is a visible vector, digest and
golden change; adding attachments later is a new reviewed declaration, not a silent edit.

**Tenancy.** A Firebase project may serve several homes under the one declared root `x/colab/`.
Exactly one home deploys, because the live Rules carry that home's marker; the others are tenants.
Owner separation is Colab's: declared paths are literal collection names, so tenancy lives in the
document ID and fields. Every document carries `space` (and `page`, `epoch`, `stream` where they
apply), its ID is those fields joined by `_`, and every condition looks up the admission documents
of that same space and page. The Rules text is identical for every tenant. An emulator test proves
that owner A can neither read, list nor write owner B's space, and the reverse, including crafted
IDs, a missing `space` field and queries that omit a filter.

| Resource (path) | Kind       | Document ID                         | Holds                                                                                                                                                                                   |
| --------------- | ---------- | ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `spaces`        | checkpoint | `space`                             | the owner uid: first writer of a space ID owns it; never updated or deleted                                                                                                             |
| `pages`         | checkpoint | `space_page`                        | the owner-written admission row: current `epoch`, `state` (`open` or `archived`), `expiresAt`; deleting it denies every read and write of the page; the owner and any member may get it |
| `links`         | checkpoint | `space_page_link`                   | an owner-written enrollment secret and role for a share link; holders cannot read it                                                                                                    |
| `members`       | checkpoint | `space_page_uid`                    | who may read or write and in which role (`viewer`, `commenter`, `editor`, `bridge`)                                                                                                     |
| `log`           | log        | `space_page_epoch_stream_seq`       | one immutable encrypted envelope: `uid` (the writer), `hash`, `envelope`, `expiresAt`                                                                                                   |
| `checkpoints`   | checkpoint | `space_page_epoch_object_index`     | one immutable 32 KiB chunk of an encrypted object (a content checkpoint or an epoch's baseline): `uid` (the writer), `index` of `count` (at most 513)                                   |
| `authority`     | checkpoint | `space_page_epoch_kind_subject_seq` | owner-written exact signed membership statements, key wraps and certificate chains, in immutable chunks                                                                                 |

There is no `blob` resource and no presence resource: Awareness stores nothing on Firestore, and
cloud attachments are #2165's. The admission collections are declared ordinary resources so the
owner-approved plan shows everything the Rules read.

**Sealed-document mapping.** Each resource declares `sync.fields` as the exact canonical REST
`Document.fields` key set, with the restricted string/integer/boolean/null/timestamp/bytes Value
codec, `spaceField`, `uidField`, `writeModes` and optional `query`. These are provider routing and
codec facts, not a signature or role grant. Rules still admit each mutation. `spaces` declares
create; `pages` and `members` create/put/delete; `links`, `log` and `authority` create/delete;
`checkpoints` create/put/delete. Put is whole-fields and unconditional; create is create-only.
Only `spaces` binds the publisher through `uidField:owner`; the other resources use null so a
blanket creator-uid equality cannot deny owner maintenance of someone else's admitted record.

Members have one seven-field shape: `space,page,uid,role,link,token,seq`. Owner-written rows carry
`link:null,token:null`; self-enrollment carries the exact non-null strings checked against the
link secret and role. `seq` is a declaration-grammar constant, not a sequence: Rules pin it to the
integer `1` on every create and put, because the declaration's discovery needs an integer field.
Rows without it are refused; nothing has been deployed and no migration is introduced. The
`holders` discovery (owner-only list, equality on `space,page,link`) is how the owner finds the
rows a link enrolled. All other resource field sets are pinned by the declaration.

A page row is readable by an exact get to the owner and to any member, and never listed: it holds
only `space,page,epoch,state,expiresAt`, routing and lifetime hints that a hosted reader uses to
name the current epoch and the truthful "not in the cloud" state. They are never authority; the
signed membership, epoch advance and wrap verification decide everything.

The `log` regular shape binds `space,page,epoch,stream`, orders the integer `seq` ascending and
uses the same stream's logical cursor. Its `writers` discovery binds only `space,page,epoch`,
with no stream/cursor or first-sequence restriction. Only the space owner may bootstrap a log
at an arbitrary original sequence after pre-cloud compaction; members retain predecessor-uid
continuity, and consumers verify the signed checkpoint cut and contiguous tail. It returns real
documents in a complete bounded run; an opaque within-run continuation is not a stream cursor.
Late writers are found by repeated runs, even when another writer is far ahead. Checkpoints map `object` as their
stream field and **`seq = index + 1`** as the positive chunk sequence; the existing index-based
ID and first-chunk writer ownership do not change. Their `objects` discovery drops only object.
Checkpoint and authority limits declare `chunkBytes:32768`; all previous planning bounds remain.

**Epoch baselines.** From the second epoch on, the owner publishes that epoch's exact signed `html`
baseline original through the same `checkpoints` carrier, chunked like any checkpoint object, with
the object ID the lowercase hex of the `objectEnvelopeHash` its signed `epoch.advance` names, and
confirms it before that epoch's wraps. The first epoch has none. A reader finds the object from the
verified membership log, reassembles every chunk in order and verifies the hash, the signed
descriptor and the signatures itself; the rows grant no authority and add no collection or Rules.

Authority has exactly three kinds: `membership` (the original signed statement), `wrap` (the
original signed key-wrap envelope), and `chain` (the original certificate-chain JSON).
A baseline descriptor is represented only inside its signed EpochAdvance statement, never a
new standalone signature. Object envelopes name an author device but do not contain its chain.
Chain subjects therefore use the digest of the exact original chain bytes, not a device ID that
can acquire a replacement chain. The chain's issuerStatement and version are transport selectors;
the existing verifier resolves them against the signed membership statement before admitting
the issuer signature. Cloud bytes and unsigned routing fields are never authority on their own.

For every authority record, `subject` is lowercase SHA-256 of its complete exact original bytes.
Split those bytes in order into at most eleven 32 KiB pieces for membership, one for chain or
wrap; encode each as canonical unpadded base64url `bytes`. `seq` is the 1-based piece position,
`count` is the original piece count, and `uid` is the owner writer. Reassemble within the existing
kind-specific verifier caps, check count/order/digest and then signatures; do not reserialize,
truncate or infer authority from a partial record. IDs bind the page as well as space/epoch/kind/
subject/seq. Every field is immutable; even the owner cannot put or refresh an authority record.
Owner deletion cleans only expired records or records of an expired/deleted page. Physical TTL
is not provisioned.

Authority's regular query scope is `space,page,epoch,kind,recipient,subject`, ordered by `seq`;
its `records` discovery removes only subject. Membership/chain require `recipient:""` and are
readable to current page members. Wraps carry an owner-written recipient projection of one of two
forms: the reading uid, or `link:` and a link ID. A wrap is readable only to its recipient **and**
current page membership, with no owner exception: a uid wrap to that uid, a link wrap to a member
whose `members` row names that link (an owner row has no link and matches none). A wrap discovery
names the recipient, either the caller's uid or `link:<its link>`; omitting recipient or naming
another is refused. The existing wrap verifier still checks the cryptographic recipient/context;
this projection supplies no key authority. An owner's second machine reads its own wrap as
recipient. Queries do not disclose another recipient's wraps.

The native sealed-document codecs map these seven resources to canonical REST fields and
page-bound IDs. One dormant client shares the object bus reader and ordered request counter;
Changed notices coalesce as hints, and a terminal interest keeps its slot until confirmed
cancellation or generation close. Transport checksums do not authenticate payloads. Neither
the declaration nor these codecs enable cloud sync by themselves. The serve runs one worker per
object-channel generation: it asks Remote to activate, binds, and publishes the most recently
active page (opened in a browser or written by the CLI; another page is published when it is
next touched, or when a management change such as a link, member or policy concerns it). It
creates the owner's `spaces`, `pages` and own `members` rows, then one `links` row for each live
link of the page that has an admission token and the signed membership, chain and wrap originals,
the wraps addressed to those links among them, then each local original in sequence, always
authority before data; a position is recorded only after the exact original was created or read back
identical. A page whose publication stopped is reported once for the serve's lifetime and is not
tried again at that epoch; Remote declining to activate prints nothing, so a machine without cloud
sync stays local, and a restarted Remote is asked again.

The worker also ends a link in the cloud. A link it published that has since ended (removed,
reset, or sharing turned off) loses its `links` row first, so nobody can enroll through it, then
every `members` row the `holders` discovery finds for it, a bounded number per pass; only then is
the link forgotten. The owner Store records the (page, link) pair before the row is first written
(`link_publications`), so a restart finishes the removal. A refused delete of a row that is not
there reads as already gone, because Rules read the row being deleted. A change made through the
serve (a link added, removed or reset, a member or a policy change) wakes the worker for the pages
it concerns, without waiting for an edit. Until the removal completes, a removed link's holder may
still read the page's routing, authority and ciphertext records; it can decrypt nothing past the
epoch advance the removal made.

The same worker reads the active page back from the cloud; Changed notices only wake it, and the
Store's receipts are the position. A read is due when the page is first watched, after a notice,
and while a read is unfinished; a local write publishes and reads nothing. A change of the page's
membership records reads the membership log and the device chains again. A change of its log or
checkpoints reads the writers' tails and reads the chains again at most once per minute, because
the authority interest watches membership records only and this machine's own publications raise
the same notice: a writer that joins without a membership change is found within 60 seconds. A
read first compares the cloud's membership log with the owner
log on this machine, which must equal or be a prefix of it: a cloud log with revisions this
machine lacks, one that differs from it at the same revision, and one that is not a chain from the
owner are three separate refusals that stop the page, and neither is merged or replaced. It then
follows each stream whose chain the cloud holds and this machine did not write, from the Store's
own head for that stream. An original is admitted by the fold that reads stored objects (the
author's chain, its role, the epoch, the namespace, the signature) and applied by the serve's own
append path, so connected browsers receive it; the writer's verified chain is kept in the Store
(as a retained chain, not as one of this machine's devices) so a browser that opens the page
later can verify what the writer wrote. An original that is refused, a malformed cloud
record or a sequence the provider no longer holds stops that writer on that page and epoch only,
with one line on the serve's stderr, and every other writer continues. What arrived from the
cloud is never queued for publication. The stopped state lives in the serve's memory, shown by "Cloud sync stopped for
page" and "Cloud sync resumed for page" lines; no command or route reads it. Not implemented:
attachments, importing a checkpoint pair (a page the provider no longer holds from its start
needs a fresh copy), adopting a cloud-only membership log or choosing between forks, and
enrolling a second machine's own identity; two machines of one owner share the owner key and
state. Live convergence covers the active page only. V1 cloud sync does
not refresh log expiry: the existing Rules permit an owner expiry-only refresh, but the log
mapping deliberately exposes no put, so entries age out at their `expiresAt`. A long-lived
page needing log refresh requires a Remote follow-up, not a hidden compatibility path.

**What the Rules do.** Rules have no hash function and cannot verify Ed25519, so they admit
ciphertext, never authority; clients still verify every statement, chain and envelope.

- _Read_: the uid has a member row for that `space` and `page`, and the page's `expiresAt` is in
  the future; a `get` also requires the entry's own `expiresAt`. Revoking, archiving writes,
  advancing the epoch and deleting the page all change the one admission row each request
  rechecks, so they take effect on the next request. Old-epoch entries stay readable by members.
- _Append_ (`log`, `checkpoints`): create-only; `space`, `page` and `epoch` equal the page row
  (`state` is `open`); the writer's member row is not `viewer`; the ID equals the fields it is
  built from; the entry's `uid` is the writer's own; `seq` is at most 2^53−1 and the previous
  `seq` of that stream exists and carries the same `uid` (`getAfter`; a gap or another writer's stream is refused); a checkpoint chunk after index 0 requires
  chunk 0 of its object to carry the same `uid`; the field set and sizes are exactly those
  declared; payload strings are ASCII, so their size bounds count bytes exactly; `expiresAt` is
  in the future and at most 365 days from the request, independent of the
  page deadline. Sequence zero and
  ciphertext updates are refused. The owner may refresh only `expiresAt` within its own 365-day
  ceiling, preserving every other field. Owner cleanup may delete an expired entry or an entry
  of an expired or removed page; live entries of live pages cannot be cleaned up.
  Commenters and bridges may write checkpoints because each device seals checkpoints over its own stream (the compaction rules
  above); the role check keeps viewers out, and clients still verify every checkpoint's signed
  descriptor against the writer's cut.
- _Admission rows_ (`pages`, `links`, `members`): written only by the owner uid recorded on the
  space row. A link holder creates its own `members` row only by equality of `token` with the
  secret in the link's row, with that link's role (the secret is separate from every content key
  and never leaves the owner's row). It cannot update, promote or delete the row.

**Stated limits.**

- A space ID is first-writer-wins, so someone who knows an unused ID can squat it: an availability
  attack, because clients verify the owner signature chain against the space ID.
- Whoever creates `seq` 1 of a stream (or chunk 0 of an object) owns the rest of it: stream and
  object IDs are random or content-derived, so an admitted writer can only squat one it already
  knows before its owner writes it, an availability attack that clients detect by the signed
  chain and freeze. The uid binds the stream to one Firebase uid; it is not the signing device.
- A list query is admitted by membership and the page's expiry only: Firestore cannot compare an
  entry's own `expiresAt` with `request.time` in a query, so a query can return an entry past its
  own `expiresAt` while its page is live. Clients drop it. Physical TTL is not provisioned
  (Remote records `not-provisioned`); expiry is enforced in Rules, and removal is cleanup.
- Remote's current backend ceilings (12 MiB per object, 64 MiB and 1,024 entries per resource)
  bound the declared limits; they are the owner-approved plan's numbers, not enforcement. Page expiry is finite and at most
  `request.time + duration.value(365, 'd')` on create and update; entry create and owner expiry refresh enforce the same ceiling independently of
  the page deadline. The declaration digest covers these Rules without adding a declaration field.

**What Rules do not bound.** Rules enforce admission and per-document size bounds, not per-uid,
per-space or project-wide usage counters. Remote's resource limits remain planning bounds,
not enforced aggregate limits. Anonymous uids permit Sybil abuse, and a shared project's free
storage and daily-write quota are not protected. Rows leave only when the owner deletes them;
expiry denies access but does not remove stored rows. There is no free-plan TTL setting
(#2467 is on hold), so no automatic aging out is promised.

The Rules emulator suite (`tests/emulator/suite.mjs`, Docker image of the pinned firebase-tools,
Java 21, no network inside) loads the composed golden, not the bare fragment. It has no skip path
and runs locally; see [references/development.md](../../../.agents/skills/tmt-colab/references/development.md#firestore-rules-emulator).

### `tmt colab hosting-bundle --json`

This public release command reads the separate hosted inventory embedded in its running
executable. It accepts no digest argument, discovers no core, opens no state, and uses no
network or runtime app-directory fallback. An executable without a hosted inventory exits 1
with `COLAB_UNAVAILABLE`; the ordinary native app bundle does not satisfy it.

Success is one JSON object with exactly `{version:1,manifestDigest,files}`. `files` is a
path-sorted list of exactly `{path,bytesBase64}`; bytes use canonical standard Base64.
`manifestDigest` is lowercase SHA-256 of the canonical manifest JSON with top-level field
order `version,files` and file field order `path,sha256,length,contentType,csp`. Manifest
version is 1, length counts decoded bytes, contentType is bare, and every file carries CSP.
The build snapshots the manifest and bundle together and preserves the renderer CSP. It copies
Remote's built SDK byte for byte to three reserved routes: `/sdk/remote-v1.js` (Remote's
committed asset, checked against its own copy), `/sdk/remote-firebase-v1.js` and
`/sdk/THIRD-PARTY-NOTICES-remote.txt`. The last two are Remote's hosted-only build output
(`remote-client/dist`), and the build checks each against the SHA-256 and length in Remote's pin
manifest `remote-client/firebase-build-pin.json`. The pin must name exactly those two files; a
missing build, a missing, malformed or reshaped pin, or any mismatch fails the build, so
Remote's build must run first. The hosted tree cannot supply any of the three routes.
Files use site-root paths: `/index.html` is the HTML shell, alongside `/reader.html`,
`/renderer.html`, `/THIRD-PARTY-NOTICES.txt` and `/assets/...`. The shell uses absolute
`/assets/` URLs and no `<base>` so short-route rewrites preserve asset resolution.
Unsafe paths, reserved `/__` paths, SDK overrides and growth reaching
50% of Remote's v1 file-count, per-file or total-byte cap fail the build.

**Declared manifest.** `tmt colab deploy-declaration --json` declares the same manifest: an
executable that embeds a hosted inventory adds a last top-level `hosting` key to the declaration
text, holding the canonical manifest bytes above verbatim (inserted before the closing brace),
so the declaration digest covers them. Remote compares the `hosting-bundle` reply with that
section, paths, lengths and digests, before any provider call. An executable with no hosted
inventory declares no `hosting` key, and its declaration is the static text unchanged. The
envelope and the 64 KiB caps do not change, and the build fails an inventory whose manifest would
take the spliced declaration past 64 KiB (each entry carries its CSP), so a bundle that passes the
file caps cannot make `deploy-declaration` refuse itself. The golden vector `deploy-declaration-v1.hosting.json`
(with its manifest `deploy-declaration-v1.hosting.manifest.json`) is the reply such an
executable prints for a fixed five-file fixture; the independent oracle rebuilds it.

**Compatibility.** Rules and the declaration change additively first. A client at release N−1
keeps working with the Rules of release N. A Rules tightening ships only after the client that no
longer needs the old shape has been the deployed bundle for one release; the bundle and the Rules
deploy together under one plan, in the order Remote's deploy defines, and open or cached clients
may run the previous bundle against the new Rules.

The hosted entry glue (the first `firebaseRemote.connect` caller), its exact Firebase connect
origins and the short-route glue are not part of this command. Ordinary releases embed no hosted
inventory yet; the release build step is tracked on #2487.

### Hosted page consumer (#2452 B4, #3122)

On a Firebase Hosting origin the app opens its hosted reader, never the local sample preview or
`/api/serve-release`. A fresh entry offers Firestore sign-in and asks for a Colab share link.
The existing reader-link parser consumes the nine-key fragment; it is erased before asynchronous
effects, the root must derive the space, and the seed is consumed by the shared link/device key
owner before the first provider request. The link role is a Rules-checked enrollment hint, never signed
page authority. A verified editor link may send own-stream edit requests through the shared Source
drawer; it never seals content. Owner/member editing, Ask and cloud attachments remain absent.
Colab's own-record codec/preparation and fresh author accessor own admission; no hosted module
reimplements link-author or edit-request validation. Viewer and commenter links show read-only
copy and marks; editor links offer edit requests, whose sending is not owner application.

The entry fetches one exact `/tmt/declarations/colab.json`, hashes those raw bytes and passes them
to Remote's `firebaseRemote.connect(firebaseRemote.originLink(), declaration)`; the SDK owns
origin/config validation and Firebase. The same release declaration supplies all discovery caps.
After explicit sign-in, enrollment is one bounded create of the holder's
`members` row `{space,page,uid,role,link,token,seq:1}`. The token comes from Colab's link model;
existing enrollment is read, never replaced, and an uncertain create gets one exact readback,
never another enrollment write. The one-attempt memo survives recovery and hidden-tab resume for that
page and uid. The following exact `pages` get supplies epoch/expiry hints only; the verified owner
log selects the epoch. From epoch 2 on, the reader selects exactly one complete original from the
already-required bounded checkpoints discovery by the signed epoch advance's `objectEnvelopeHash`
(lowercase hex carrier object ID). Existing field/chunk/digest admission precedes the baseline
codec, Catchup/openBaseline/Admission.baseline and Worker commitment/source checks; only that exact
signed `html` original becomes the baseline, never also a checkpoint. Every other object keeps
ordinary checkpoint admission, including its header object ID. Zero or several matching originals,
or any unavailable or unverifiable baseline, retain `COLAB_HOSTED_RESHARE_REQUIRED`: “This page
changed its sharing since this link was made. Ask the owner to share it again.” Historical wraps
pass through the existing ordered wrap admission to select the current root. The baseline is
initialized once per fresh reader; log wakes reuse its encrypted original and admitted state,
without another checkpoints read or budget path. A signed epoch change ends the old generation
before fresh bounded bootstrap; cancellation, deadline, expiry, revocation and budget refusals stay
unchanged. A checkpoint is never synthesized into a baseline.

One active page owns exactly three interests: log, checkpoints and membership authority.
Subscriptions attach before bounded discovery. Notices only wake the consumer, which retains
its own verified positions and reconstructs original documents with the reviewed mapping.
Admission verifies the signed owner log and link pin, addressed `link:<id>` wraps, device chains,
paired checkpoints, revocation cuts and contiguous merged stream order before Objects/Catchup
and the existing fold publish any plaintext. Initial and recovery catch-up discover the log and checkpoints. Later log wakes discover the
bounded authority-chain inventory and query its candidate streams, known writers and checkpoint
writers from their admitted heads, without re-listing retained log history. Chains route reads;
only observed writers enter existing author verification. An empty candidate is checked again on
later wakes, so a dormant device's first write needs no membership notice or timer. Cost still
grows with chain count (each dormant candidate costs one empty 4-read query per refresh), new
originals and checkpoint chunks, rather than retained history on ordinary wakes. A failed or
partial finite page publishes nothing.

**Several holders of one editor link.** Every hosted holder derives the link's one `hosted` device
and writes its one stream with cloud log creates. Each holder signs in with an anonymous uid of its
own and enrolls a `members` row of its own by the link's secret. The Rules continue a stream
(`seq > 1`) for the uid that wrote its predecessor, or for a uid whose member row names the same
link as the predecessor writer's row. A row's `link` is set only by the link's secret and a holder
cannot edit it, so equal links mean one secret; an owner-enrolled row names no link and never
matches, and neither does a holder of another link. This is a tested Rules fact (the Colab Rules
suite): two uids of one link both continue the stream, every other writer is refused, and ending the
link removes the rows of all of its holders.

Holders are not told apart. They share one device, so the owner sees one link device and an applied
request carries no per-person attribution (a per-holder device would need registration rows, which
the link model does not have). A holder that loses a race for the next sequence has sent nothing:
Firestore answers an occupied ID with `ALREADY_EXISTS` (409, which the SDK reads back) or a Rules
refusal (403), and the client treats both alike. The draft stays, Save stays enabled and the reading
is the not-sent notice; the next deliberate Save catches up and sends one new original at the new head,
and nothing resends by itself. A continuation after another uid of the same link costs one more
billed lookup in the Rules (that writer's member row, five distinct documents against four); a
same-uid continuation, a first entry and an owner append cost what they did. These counts are derived
from the Rules text, not measured. A deployment still on the earlier Rules refuses a second uid (403,
not sent) until its owner redeploys. The door side is unaffected: the owner's serve applies appends,
every door holder shares the `door` stream, and a race for its next sequence is refused with nothing
frozen (Own-stream edit requests).

**Link-device authors.** A device chain issued by a link (`issuerKind:"link"`) is admitted by the
rules native author admission applies, never as a member and never in place of one. The issuer is
the link recipient the signed owner log's `link.add` created: the chain cites that statement's
hash, its signature verifies under that statement's `linkSignKey`, and its `membershipRevision`
lies from that statement up to the log head. A chain that names the other issuer kind is refused:
a member chain claiming a link, and a link chain claiming the owner member. An original of such a
device is admitted when, in the membership state at the revision it names, the link exists and
covers the page, the page is writable and link-shared at the envelope's epoch, and the link's role
is `commenter` or `editor` for `own` and `editor` alone for `content` (a viewer link is never an
author; in practice a link writes `own` only, since the owner applies edit requests). A
`link.remove` of that link, like a `device.revoke` of the device, ends its stream at the cut it
signed: originals past the cut are excluded, and a removal at or before the revision refuses the
original. Historical admission ignores current expiry, as it does for members. The device is
never an owner-device author, so its records carry no status authority.
`vectors/link-author-v1.json` pins these cases for native and browser.

Page selection and visibility changes fence the old generation immediately, close its interests,
and join reads, folds and guard work before any successor opens. Hidden pages hold no interests;
returning to the tab performs fresh verified catch-up. An elapsed idle `next` deadline keeps the
same interest; cancellation retires it. Transport/catch-up recovery has at most three attempts
under one deadline with cancellation-aware backoff. Rules/access, budget/rate, identity and
application refusals stop without automatic re-enrollment or a fresh listener; stopped and
recovering views never label retained content Live.

The caller-owned advisory guard shares ten percent of Remote's free daily reads per browser
origin, serialized across tabs with Web Locks and one IndexedDB record. Remote's budget model
owns the daily reset and warning/refusal percentages. Mutation shapes, own-log reservations,
bounds and the four publication outcomes follow Remote's
[hosted guard](../../../contracts/remote-channel-v1.md#hosted-own-record-publication-guard-3122),
including exactly one same-ID readback and no reseal/resend. The Source drawer keeps an
unconfirmed draft for a deliberate user decision and does not substitute requested text for
admitted content. A lost race for the next sequence keeps the draft, leaves Save enabled and reads as
not sent. Listener starts reserve
no guessed tail: the SDK reports every complete
initial server snapshot and later document-change batch, including own changes, and the guard
counts those raw documents before exposing a notice. A delivered batch that crosses refusal
remains charged and closes the reader. Corrupt/unavailable storage refuses; the guard is advisory,
not the Firebase project's aggregate counter. The hosted parent CSP permits only self and the
three Firebase hosts defined by Remote's transport; native parent and opaque renderer policies
retain their existing boundaries.

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

When the lock releases, no private failure record (including an older serve) or an empty
cleared record keeps the stopped result. A `cleanup` failure in `serve-error.json`, or an
existing unreadable record, returns `COLAB_OUTCOME_UNKNOWN`, names cleanup and asks for a
manual `tmt remote status --json` check; it never claims the owned door stopped. Every serve
mode clears the previous record at startup and records unconfirmed cleanup before releasing
its lock. There is no required successful-completion record and no new socket message.

The local space is loopback-only: there is no `--bind`, LAN or other
non-loopback mode. Other people's machines reach a page only through a cloud
backend (Firestore, then Cloudflare). L2 verifies owner-only socket admission,
body/acquisition caps and timeout/shutdown behavior; Host allowlisting is
remote's.

Firestore uses Hosting, Anonymous Auth for link holders/bridge connector and
named Google sign-in for named members, Spark by default. Rules admit uid,
the owner's member projection and link enrollment by equality with an
owner-written secret (Rules cannot hash a join proof); enforce create-only
scoped sequence, current epoch and expiry. The
[declaration subsection](#firestore-deployment-declaration) is the
implemented subset. No public
mode is allowed in cloud v1. Checkpoints are chunked Firestore documents,
without Cloud Storage or Functions; attachments over Firestore are not in the
first declaration (#2165), so a page's attachments stay local-only. A Function may be introduced only for a
specifically justified check Rules cannot express. F1 MUST prove admission, epoch
fences and deletion refusals in emulators; anonymous UIDs permit Sybil abuse. Owners can disable links/remove devices; App Check is
optional and not a general authority guarantee.

Cloudflare uses a static-asset Worker and one hibernating WebSocket PageRoom DO
per page with the same sync wire, SQLite rows at most 2 MB, retention alarms and
optional R2. Join proof or Access admits ciphertext; client log verification
still decides authority. Real account/deployment actions require separate owner
authorization. Tests use demo Firestore emulators and local workerd/Miniflare
only, no cloud accounts, billing or deployments.

## Retention and management

**Channel boundary: split.** Page expiry policy and warnings stay; backend enforcement uses remote-provisioned resources that colab declares.

Cloud lifetime defaults to 30 days from the last activity refresh. Owner overrides
are 1 through 365 days, with a maximum of 365 days from refresh; cloud lifetime
has no forever option. The default and override selection are client policy;
Firestore Rules enforce the finite 365-day admission ceiling, not the default.
The Rules admit owner-only expiry refresh of logs and checkpoint chunks, each at most 365 days from the request, without
changing ciphertext, routing fields or writer identity. An owner may delete
expired entries and entries of expired or removed pages, preserving live entries
of live pages and other tenants' data. Physical TTL remains eventual and is not
required for logical expiry.

The serve's cloud-sync worker applies the lifetime. A page's row carries `expiresAt` of
its retention days from the write: the policy's 30 days by default, an owner override of
1 through 365, and 365 for a page kept forever locally, since the cloud lifetime has no
forever option. The row is written again at once when its epoch, state or lifetime
changes (a log create is checked against the row's epoch and `open` state), and on
activity after at most one day, or half of a one-day lifetime. An activity refresh
updates only the page row. A row that lapsed while the owner was away is written again
by the next pass after the page is reopened from local, and its entries, which keep their
own deadlines, read again. Each confirmed write of the page row is recorded as the page's
cloud expiry, which the owner's page list shows (see [Local management CLI](#local-management-cli-1307)).
Renewal of entries near their own expiry awaits its own slice.
Logs and checkpoint chunks have independent deadlines and need renewal only near their
own expiry horizon, avoiding a rewrite of every entry on each page edit. Entries may
outlive their page, but page expiry still denies every content read. The binding must
renew needed entries before their own expiry and map expired-but-present cloud data to
absence.
Local expiry never deletes local data: existing local warnings begin seven days
ahead in the browser and `ls/show`, and expired local pages remain readable and
writable unless signed archive/delete policy forbids it. Local retention accepts
its existing positive day count or forever; this is separate from the finite
cloud lifetime. Local data is never automatically deleted.

The space's page expiry (#2600) is the days a page may sit without activity before it is
archived: 180 by default. `tmt colab settings page-expiry DAYS|off|default` sets it to a whole
number from 1 through 3650, to `off` (never archive for inactivity), or back to the default. It
is a machine-local setting in `settings.json` (`{"pageExpiry": N|"off"}`, absent for the
default; any other value makes the file read as damaged and the defaults apply) and settings
output reports it as `pageExpiryDays`, null for `off`. A page's own signed `retention.set`
overrides it (`forever` never archives), and one function decides which applies: a page with no
`retention.set` takes the space's setting. Each owner machine applies its own setting, so where
several machines serve one space the shortest wins. The setting signs nothing and changes no
stored statement. Nothing applies it yet; the scheduled archive of #2600 will.

The local management reads (`ls`, `show`, `retention`) report it per page; the catalog the
browser is served does not carry it. `lastActivityAtMs` is the page's effective activity (the later of the last activity and
the last content edit; null when neither was ever recorded), `archiveAfterDays` is the days that
apply to the page (its own `retention.set`, else the space setting; null for never), and
`archiveAtMs` is `lastActivityAtMs` plus those days, or null when the page is already archived,
never expires, has no recorded activity (an unknown activity is never an old one) or the time
would not fit a record. `ls --expiring` lists the pages whose `archiveAtMs` is within seven days
of now or already past. Human `show` and `retention` add `last activity` and `auto-archive` lines
(`never`, `in 12 days`, `◷ in 4 days` within seven days, `due now`, or `N days after last activity
(none recorded yet)`). Reading never refreshes the evidence.

Firestore Rules deny expired reads; owner cleanup of expired cloud data is admitted.
The browser/CLI cleanup caller is part of the pending relay binding. Optional Blaze
TTL is eventual physical cleanup, not timely revocation.
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

| Operation                                                        | Exact request fields                                                         |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `epoch.advance`, `page.archive`, `page.unarchive`, `page.delete` | `pageId`                                                                     |
| `page.share`, `page.history`                                     | `pageId, mode`                                                               |
| `retention.set`                                                  | `pageId, days` (positive safe integer or null)                               |
| `member.add`                                                     | `memberId, role, signKey, encKey, pages`                                     |
| `member.remove`                                                  | `memberId, pages`                                                            |
| `member.role`                                                    | `memberId, role, pages`                                                      |
| `link.add`                                                       | `linkId, role, pages, seed`                                                  |
| `link.remove`                                                    | `linkId, pages, replacement` (required null, or a complete link-add request) |

`page.unarchive` uses the same live owner management admission, expected owner-log head and
operation-receipt binding as archive. A new request requires an archived, non-deleted target;
an active or deleted target is refused. The owner signs exactly `{pageId}`: the browser cannot
supply replacement keys, epochs, recipients, history, expiry or restored content.

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
positive decimal text (the empty string for `page.projects` and `project.manage`, see [Projects](#projects-3747))
and payload is canonical base64url of the same typed JSON.
The root-only `page.create` selection is `{pageId,title,source}` with optional bounded `publisherAgent`, optional complete `creationRecipient` and, for an HTML page, an optional `layout` `{kind,reason?}` (the `kind`, and for a `canvas` its `reason`, as `meta.layout` and `meta.layoutReason` define; a dashboard has none) as defined
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

### Bundled agent guidance

`tmt colab skill [<name>]` prints human-only guidance before core discovery or
state access. Without a name it prints the embedded root `SKILL.md` exactly.
Names are compile-time keys (`layout`, `components`, `links`, `page-style`, `questions`),
not paths; unknown names return `COLAB_INPUT_INVALID` and list the valid keys.
Each root pointer names both `references/<name>.md` in the installed skill tree
and the offline command. Authority, pairing, bearer-seed, user-only sharing,
write/retry and receipt rules stay in the root, at most 10,000 bytes. Detail is
bundled verbatim in the references. The release archive's full skill tree equals
its source, and each archived reference equals the relocated executable's
`skill <name>` output; guidance probes create no config, home or workspace state.

### Local management CLI (#1307)

The root-local CLI is a client of the reserved management IPC while serving and
calls the same library service under the lifecycle lock offline. It MUST NOT
fall back to an offline writer after an IPC send. The owner runner alone signs,
mutates and records replay outcomes. Read commands MUST NOT initialize missing
state, change journal mode or migrate schemas.

Command names precede operands, following the shared CLI style audit (the lead's
#1307 decision replaces the positional-first #1111 sketch):

```text
ls [--archived] [--expiring]
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
project ls
project create <name>
project add <page> <name>...
project remove <page> <name>...
project rename <name> <to>
project delete <name>
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

Mutations accept `--operation-id` and `--expected-revision`. A non-frozen local
management intent may be rebuilt once after a verified pre-effect `STALE_HEAD`:
capture a fresh head, resolve the same full page ID and revalidate selections,
roles and admission. The affected targets and confirmation scope must remain
unchanged; otherwise refuse `COLAB_CONFIRMATION_CHANGED`. The rebuilt intent
has a new operation ID and generated link capability. A second stale head refuses
`COLAB_STALE_HEAD`. An explicit operation ID or expected revision freezes the
intent and never takes this path; neither does an unknown outcome. Success is `{pageId, operationId, expectedRevision,
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
lastUpdateAtMs, lastActivityAtMs, archiveAfterDays, archiveAtMs, expiresAtMs, warnings}}`.
Page lists return `{spaceId, membershipHead, pages}`; an uninitialized list has null
space/head and empty pages. Show returns `{spaceId, membershipHead, page, members,
links, discussions:"not-available"}`. Page fields are `pageId, title, epoch,
sharing, history, archived, retentionDays, lastUpdateAtMs, lastActivityAtMs,
archiveAfterDays, archiveAtMs, expiresAtMs, warnings`.
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
or fabricated zero establishes it. The next admitted content edit starts evidence for a
legacy page. The time is the page's last edit and is not what expiry is computed from.

Schema 15 adds nullable, nonnegative safe-integer `pages.last_activity_at_ms`, the page's
last activity: the server clock at the last new admitted update of either namespace or an
explicit owner `page.unarchive` (as defined above), so a
content edit, a comment and a reply all count. The store holds sealed bytes and cannot tell a
comment from an Ask record in the same `own` stream, so Ask records count too. The rules are
those of `last_update_at_ms`: sampled in the receipt transaction, never decreasing, and never
refreshed by exact replay, a failed or rolled-back update, a read, a checkpoint or a
other policy or epoch change. The migration leaves legacy times null without backfill. A page's
effective activity is the later of `last_activity_at_ms` and `last_update_at_ms`; null is
unknown, never old. No command reports the value yet.

Schema 16 adds `page_titles(page, progress, title)`, the owner's local plaintext title projection of an
active page: one row per page, `title` 1–512 UTF-8 bytes of one line of plain text (control and bidirectional formatting characters become spaces and whitespace collapses; a title that is then empty or too long is not stored and removes the old row). It is derived only from the owner's
own verified fold (a decoder-folded title of a page whose authority is writable), never from
browser input, and the summary workers are its only writers. A worker captures the page's
fingerprint before folding and installs the row in an immediate transaction only if the
page's full fingerprint, membership head included, is unchanged; a changed, archived or
deleted page installs nothing. The row stores the page's head-less progress key, and a
read serves only a row whose key equals the page's current one, so another page's creation
or sharing does not discard every title. Archiving a page, manually or by the inactivity
lifecycle once applied, drops its row in the same transaction as the archive flag; deleting
a page drops it with the page's other data and the tombstone stays. A later unarchive waits for the next
verified fold. The serve starts one fold of every page without a fresh summary when it starts, and each
list read schedules the same, so a store that gained the projection by an upgrade fills without
anyone opening the list; there is no backfill state. The cloud row's `expiresAtMs` never removes it. The projection is rebuildable,
so losing it loses no source or authority, and it never enters an owner statement, sync or cloud
payload, share-link bootstrap or export.

Each page in the owner-served `GET /api/pages` list may carry that row as `title`, a string of 1–512
UTF-8 bytes without a control character: only while the row is current for the page, so a page
changed since its last fold has none until the next fold, and never for an archived page. Titles
are added most recently updated page first (equal times by page ID), each only if the whole reply
stays within its 2 MiB bound; one that no longer fits is left out, never the catalog. Like the
summary it appears only in this owner list, not in a reader link's or a member's views, and not in
the CLI's `ls`/`show`; a client treats it as untrusted text.

Retention defaults to 30 days; forever is null. The expiry shown is the cloud copy's and is
never derived from local activity: `expiresAtMs` is the `expiresAt` of the page's owner row as
the cloud last confirmed it for the page's current epoch. The serve's cloud-sync worker records
it when its write of the row is confirmed (Store schema 11: one row per page holding the epoch
and that expiry, replaced by each confirmation and removed with the page's data), so an edit or
a retention override moves it only once a later write of the row is confirmed. A page with no
confirmed row for its current epoch has a null `expiresAtMs`. A page kept forever locally still
reports its confirmed row's expiry and the same warnings, since losing the cloud copy ends sharing;
with no confirmed row it has a null expiry and no warning. At most one expiry warning is emitted: `expires-soon` when the confirmed expiry is in
the future and at most seven days away; `not-in-cloud` when it is at or before now, or when the
page has no confirmed row for its current epoch (never confirmed, or the epoch moved) on a
machine that has confirmed a page row for some page. A machine that never has is local only:
null expiry and no warning. Otherwise warnings are empty, apart from a separate
`title-unavailable` CLI hint. JSON retains exact UTC milliseconds. Human `ls`, `show` and
retention reads use whole relative elapsed time, sampled once per result: `last edit 2 min
ago`, `expiry in 30 days`, and a dim `expires in 30 days` line under each list link. Finite
expiry within seven days has a `◷` waiting mark; a page that is not in the cloud reads
`not in the cloud`, and a local-only page `local only`. Expiry intervals use days, hours or less
than an hour, matching browser retention hints rather than UTC calendar dates. The footer says
"Expiry never deletes your local copy." CLI state values are lowercase UX display copy:
`kept forever`, `local only` and `not in the cloud`; a page kept forever locally with a confirmed
row reads `kept forever here · cloud copy expires in 6 days` (or `kept forever here · not in the
cloud`). They omit redundant type labels and final
periods; browser hints keep their sentence form. `expires-soon` prints one stderr line,
`<id>: <expiry> · Expiry never deletes your local copy`, and so does `not-in-cloud` with a
non-null `expiresAtMs`: a page that was in the cloud and whose confirmed expiry has passed.
`not-in-cloud` with a null `expiresAtMs` (a page not yet put in the cloud for its current epoch:
never published, or the epoch moved and the new one is not confirmed yet) is shown in the list
and in detail but prints no line, so a syncing machine does not repeat a warning for pages
nothing is wrong with. Human reads never change the projection, policy or stored time; JSON
still exposes the exact configured day count.

One verified owner-log projection supplies these hints to CLI and `/api/pages`.
Discovery metadata never grants signed policy or write authority. Local expiry
never automatically deletes or denies access. Discussion summaries in management
listings remain deferred; the page view projects verified discussion separately.

Failures exit 1. JSON is `{error:{code,message}}` with page/operation/revision/link
correlation when captured; human failures use styled stderr. Management codes
map explicitly to `COLAB_INVALID`, `COLAB_DENIED`, `COLAB_EXPIRED`,
`COLAB_CONFLICT`, `COLAB_STALE_HEAD`, `COLAB_CAPACITY`, `COLAB_UNAVAILABLE`.
CLI failures add `COLAB_INPUT_INVALID`, `COLAB_CONFIRMATION_REQUIRED`, `COLAB_CONFIRMATION_CHANGED`,
`COLAB_PAGE_NOT_FOUND`, `COLAB_PAGE_AMBIGUOUS`, `COLAB_PAGE_DELETED`,
`COLAB_PROJECT_NOT_FOUND`, `COLAB_OUTCOME_UNKNOWN`; existing state failures keep
their codes. An older schema with a
known migration path returns `COLAB_STORE_OUTDATED`: "The running Colab service is
older than this CLI." Its next action is "Run tmt colab stop, then tmt colab serve --background."
A schema newer than this binary returns `COLAB_STORE_NEWER`: "This space was
saved by a newer Colab." (with a copy to go back to, see Copy before a migration) Its next action is "Run tmt upgrade, then try again."
Human output uses a separate styled `hint:` line for those sentences. JSON uses
ordered runnable commands in its top-level `next` array:
`["tmt colab stop", "tmt colab serve --background"]` for an
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

**Copy before a migration (#4280).** `Store::open` copies an existing store before the first step of
a migration that will run, once, while it holds the writer lock: `space.db.pre-v<N>.bak` beside the
store, N the schema it is leaving, owner-only (0600), written as a `.partial` file that is synced and
renamed, then the directory is synced. A second upgrade from the same schema replaces that copy; after a
committed migration only the two copies at the newest schemas stay. A store with no schema yet and one
already current make none, and `Store::read` and `Store::write_existing` never migrate, so never copy.
If the copy cannot be written the migration does not run and the store is untouched:
`COLAB_STORE_BACKUP_FAILED`, "Colab could not save a backup of this space before updating it.
Nothing was changed.", with the hint "Free disk space or fix the permissions of the Colab data folder, then try again."
A copy is store data (it holds epoch secrets) and stays in the private data root. Recovery is manual:
when a store is newer than this build, the `COLAB_STORE_NEWER` refusal names the newest copy at a schema
this build supports, "This space was saved by a newer Colab. Run tmt upgrade to open it. To stay on
this version instead, stop Colab and replace space.db with the backup from before that update
(<path>); changes made since then are lost.", and is unchanged when there is none. No command restores a copy.

### Projects (#3747)

Projects are display-only names the owner uses to organize the page list; a page can be in
several. They carry no authority and are **not signed**: a project never changes who can read
or write a page, is never an owner-log statement and never enters a page's content or any
stream. A project exists on its own, so a new one may hold no page. They live in the owner store
(`projects`, one row per name, and `page_projects`, one row per page and name, schema 13; the
assignments are removed with a page's data when it is deleted, the page's tombstone row keeps
none, and the project stays). They reach browsers only in the owner-served `GET /api/pages`
reply, as `projects` on each entry and at the top level, and in the CLI's `ls`/`show --json`
pages: arrays sorted by code point, empty when a page is in none; the top-level array names
every project of the space, those with no page too. Their integrity is the Remote session's, like
the other owner-served reads; a client treats them as untrusted text. Schema 13 turns every
page label (the earlier name for this feature) into a project of the same name with the same
pages; no label command or field remains.

A name is 1 to 32 code points (`PROJECT_POINTS`) with no control character, no whitespace
(Unicode `White_Space`) other than single interior spaces and no leading, trailing or doubled
space; case is part of the name and two spellings are two projects. `vectors/project-name-v1.json`
pins the grammar for the owner store and the browser. A page is in at most 8 projects
(`PAGE_PROJECTS`), the space holds at most 256 names (`SPACE_PROJECTS`) and 1,000
page-in-project assignments (`SPACE_PROJECT_PAGES`), so the discovery reply stays within its
1 MiB cap. A write that would add a name beyond 256 is `CAPACITY`; a space migrated from labels
that already holds more keeps its names and refuses only a new one. The constants live in
`limits.rs`; the app's `bootstrap.ts` and its test hold the same numbers (it accepts up to 1,000
names, which a migrated space can hold).

There are two writes, both management operations available to a device-signed browser request
from an admitted owner device and to the local CLI over the same local management path as every
other command. Neither is a log operation: they append no statement, change no membership head
and record no operation receipt. Neither carries a revision: the signed `expectedRevision` field
is the empty string, any other value is `INVALID`, and a log operation with an empty field is
`INVALID` too. Each applies in one store transaction, so it applies fully or not at all. A
read-only reader link has no write path and cannot change projects, and there is no owner-only
gate beyond the paired owner device or local CLI that every management write already requires.

- `page.projects` replaces the projects of the pages it names. Its payload is exactly
  `{pages:[{pageId, projects}]}`: at most 64 entries (`PROJECT_REQUEST_PAGES`), page IDs
  strictly ascending, the request's own `page` among them, every `projects` array the complete
  new set for that page. Last writer wins per page. A name no project has yet creates that
  project. A malformed name, a repeated name, an unknown or deleted page, or a repeated page is
  `INVALID`; more than 8 names on a page, more than 1,000 assignments in the space or a name
  beyond 256 is `CAPACITY`; both refuse the whole request with no partial effect.
- `project.manage` changes the space's projects without naming a page. The signed frame binds
  one page, so this request names the space scope page
  `00000000-0000-4000-8000-000000000000` in `page` and no other request may; any other value is
  `INVALID`. Its payload is exactly `{action, name}` with `action` `"create"` or `"delete"`, or
  `{action:"rename", name, to}`; `to` is present for a rename and for nothing else, and `null`
  is not an absent `to`. `create` is idempotent. `rename` moves every page that is in `name`
  (archived pages included) to `to`; if `to` already exists the two merge and a page in both
  keeps one entry, and no page is deleted. `delete` removes the project and its assignments
  only, never a page. An unknown project for `rename` or `delete`, an invalid name or
  `to == name` is `INVALID`.

The CLI keeps the commands below to those two operations. `project create <name>` is
idempotent and reports `changed:false` for an existing name. `project add <page> <name>…` and
`project remove <page> <name>…` read a page's set and replace it, and report `changed:false`
without writing when nothing differs. `project rename <name> <new>` and `project delete <name>`
are one request each. A name that is not a project is `COLAB_PROJECT_NOT_FOUND` for `rename`,
`delete` and `ls --project`; a name outside the grammar is `COLAB_INPUT_INVALID`; a full page or
space is `COLAB_CAPACITY`. `project ls` lists each name with its page count, including projects
with none, and `ls --project <name>` lists only the pages in it.

### Page summary (#2588)

Each page in the owner-served `GET /api/pages` list may carry a display-only `summary` the
The home page list shows with its title: `{comments, waiting, needsYou, author?, work?, parent?}`. Like projects it is **not signed**,
is never an owner-log statement and never enters a page's content or any stream, and it
appears only in this owner list: not in a reader link's or a member's views, and not in the
CLI's `ls`/`show`. A client treats it as untrusted text.

- `comments` is the number of non-deleted comment records in the page's annotation threads,
  open and resolved, agent replies included. An annotation thread is a thread that is not
  deleted, carries no proposal and is anchored. Chat messages, status actions, proposal
  decisions and question answers are not counted; a question answer is the plain-text comment
  that starts `Answer to “`.
- `needsYou` is the number of pending targets, at most 350 (200 proposals, 50 readings,
  50 answer blocks and 50 reviews), with a target ID counted once. The order for deduplication
  and the page-header menu is proposals, readings, answers, reviews. A proposal waits while
  not deleted, not resolved and without a live decision. A reading is unread until its canonical
  exact fixed-form device comment and a person's Resolve exist. An unanswered group counts once
  while any admitted member lacks its canonical device answer; a standalone question counts once.
  A review waits unless person-closed or its latest device change request is still waiting for an
  agent reply. Agent prose, device labels and publisher clocks cannot substitute for provenance.
- `waiting` is exactly `needsYou > 0`, not a separate proposal-only definition.
- `author` is the page's creation label, `meta.originalAuthor` (the bounded display label
  derived from `publisherAgent` at creation; never identity proof), and is omitted when the
  page has none. Never the last editor.
- `work` is the page's tasks as the list shows them (see [Work](#work-4050)), omitted for a page
  with no `meta.work`, so a fresh summary without it is in discussion. It is
  `{stage, done, total, signoff, working, more?}`: the derived stage (`discussion` with no task,
  `in-progress` while any is not done, `done` when all are), the tasks done and in all, the tasks
  waiting in review for a sign-off, and the assignees of in-progress tasks, each once in task
  order, at most 3 as `{kind, name}` (never an identity), with `more` counting the rest and
  present only then. Every task that follows a page counts as that page's stage says
  ([A task that follows a page](#a-task-that-follows-a-page-4118)), so the stage agrees with the
  counts: `total` 0 is `discussion`, `done == total` is `done`. A followed page without a fresh
  summary is unavailable, and its task counts as its own stored state.
- `parent` is `{page, task, title?}`, the task that follows this page, omitted when none does. With
  several (two writers raced) it is the first by page ID, then by task number. `title` is that task's
  own title, clipped to 32 code points (a longer one ends in `…`), so the child's band can name the task
  without reading the parent page; a row without it (an owner from before) names the task by number.

The owner's static-source projection uses the same admitted markup, normalized quote context,
unique-match and containment rules, canonical thread order and verified device provenance as the
browser. `vectors/needs-you-v1.json` pins both owners' pending IDs. The owner never executes authored
scripts; later author-script DOM mutations remain cosmetic browser claims and can change which
anchors the renderer can display, not any saved record or admission. An unresolved quote never
adopts another heading's answer or read; a text/match-budget failure or the 200 ms projection
deadline leaves the summary unavailable, never a false zero. The decoder keeps its existing 2 s bound.

The list's needs-you filter keeps the pages whose summary says `waiting`. A page without a
summary is unknown, never a false zero, so the filter does not count it. Navigation back to Home
or a browser reload makes the existing owner-list read again. While a page that is not archived
still has no summary, the visible Home list also reads again every 2 s, at most 15 times, and stops
at once when every page has one or the route changes; a hidden tab does not read, and a re-read
leaves the window where the reader scrolled. Chapter reading
descriptors use the separate bounded owner-only read below; they never enter discovery.

The values come from the owner fold, which is too slow for a list read, so the serving process
keeps them in memory (no schema, no table), each valid for the page's _fingerprint_: its epoch,
the membership head and its receipt and checkpoint counts, read in the same store transaction
as the list, so an accepted write or a membership change makes it stale. A read replies at once
with the summaries that are fresh and omits `summary` for every other page; it never waits for the
fold and shows no placeholder. It also schedules a refresh of every stale page, most recently
updated first, run by one background worker, one page at a time, through the same materialize path
and 2 s decoder deadline as `threads`; the worker reads the order again every 8 pages
(`REFRESH_PAGES`), so a page that changed before its turn is served next, and a read that finds a
worker running schedules nothing. A run folds each page at most once: a page whose fold fails or
times out stays without a summary, and one that changes after its fold waits for the next read,
which tries the pages that have not failed at their fingerprint first. A restart refills the cache
lazily.
The cache holds each page's _own_ work (its derived values and, only when a task follows a page,
its task lines); the effective `work` and `parent` are resolved from the fresh entries on every list
read (`work_links.rs`) and never cached, because a page's stage changes when the page it follows
does.
`summary` is at most `{comments:4294967295, waiting, needsYou:350, author:128 bytes, work:{3
assignee labels of 128 bytes, counts to 256}, parent}` per page (each label and the author escape to
at most 256 bytes on the wire), so a full space of 1,000 pages with the longest labels measures
1,926,186 bytes and stays within the 2 MiB discovery cap (raised from 1 MiB for the work row; the
browser boundary test pins the complete encoded reply).

### Owner chapter details (#3578)

The numeric discovery summary stays small. The owner/paired-only
`GET /api/pages/<pageId>/summary-details` reads one canonical lowercase page UUID,
with no query, request body or upgrade. It uses the same Remote-authenticated owner
context and local-space owner-read admission as `/api/pages`; a member, reader or
hosted adapter gains no route or owner metadata. It never opens a page connection,
Subscribes, touches the active page, stores a record or sends a notification.

The JSON result is exactly one of:

- `{pageId, state:"ready", fingerprint, summary, readings, needs, latest?}`. `fingerprint` is the
  lowercase SHA-256 of the existing cache fingerprint's UTF-8 bytes, not a signature
  or admission token. `summary` has the discovery summary shape. `readings` is the
  complete ordered list of admitted, uniquely anchored reading descriptors from
  that same cached fold: `{id,title,agent,why,state:"unread"|"read"}`. The four text
  fields retain their markup bounds: canonical UUID, 1–200 title code points,
  1–64 agent code points and 1–200 reason code points, without control characters.
  Read state uses the same canonical device comment and person Resolve as the
  page; labels or agent prose never count as a read. Pending reading IDs must
  agree with the shared pending-target projection, including cross-kind ID
  deduplication. These descriptor links only open unread readings; they never mark read.
  `needs` names the pending items the count includes, ordered review, question, reading,
  proposal and by page order within a kind, at most 20 (the count stays `summary.needsYou`):
  `{kind:"review"|"question"|"reading"|"proposal", id, title, agent?, why?}`. `id` is the
  pending-target ID (a question's is its group's); `title` is 1–200 code points and `agent` at
  most 64; `why` appears on a reading only. A pending item the page no longer lists is left out.
  `latest`, absent when the page has no live comment, is `{who?, atMs, text}`: the newest
  non-deleted comment by its device clock, excluding the records the page controls write
  (answers, confirmations, reads), with whitespace folded to single spaces and at most 160 code
  points; `who` is the device label it was written under, display-only and never proof of
  authorship, and is absent when that label is empty once made inert. `needs` and `latest` come from the same fold and fingerprint as the rest.
- `{pageId, state:"pending"}` when no fresh cached fold is available and it has
  not failed at the current fingerprint. The existing bounded summary refresher
  may schedule it; the request itself never waits for a decoder or starts another
  worker.
- `{pageId, state:"unavailable"}` when the fold failed at the current fingerprint,
  the source is unavailable, the cache has no room for another page, or the complete
  result exceeds a projection or reply bound. No descriptors or numeric zero accompany pending/unavailable.

The owner samples the page's current fingerprint through the existing owner-read
transaction and returns only a matching cache entry. The summary and descriptors
are cached and replaced together, never assembled from different folds. The cache
holds at most 1,000 pages, including failed entries; a catalog read prunes deleted
pages, while an existing entry can be refreshed at capacity. A missing
or deleted page is denied. Malformed requests use existing `INVALID`; owner-context
refusals retain `DENIED`/`EXPIRED`, and failed store admission retains the existing
error mapping. There is no partial success or truncated descriptor list.

Each UTF-8 JSON reply is at most 192 KiB, with at most 50 descriptors, 20 named passages and no
pagination. Replies contain only these allowlisted descriptors and the one bounded `latest`
excerpt, never passage HTML, other discussion bodies or certificate bytes. The browser validates exact keys, byte/count/text bounds, UUID
uniqueness and states before rendering any descriptor as inert text. A malformed,
failed or stale result remains unknown, never an empty complete list. Discovery
keeps its 2 MiB and 1,000-page bounds.

The owner app's dashboard is the one browser consumer: it reads this route for its first 20
chapters' pages to show each chapter's passage, without writing anything. The route never marks a
reading read, and neither does opening a chapter from a shell.

## Root-local page creation

`tmt colab page create --title <title> --layout column|full|canvas [--reason <text>] [--file <path|->] [--json]` creates a private
page at epoch 1. The layout choice is required: `--layout column`, `--layout full` or
`--layout canvas --reason <text>` (`--reason` belongs to `canvas` only, and `canvas` needs
it). Without a choice the command refuses before it initializes state or reads `--file`, naming
the three choices and `tmt colab skill layout`; a reason that is empty, longer than 1024 bytes,
or holds a control character, and a `--reason` on another kind, are refused the same way. Freeform
is not a choice. On success the result adds `layout` (and `layoutReason` for a canvas) and
`read: ["layout","components"]`, the skill topics to read before writing markup; the human
result prints the layout and both `tmt colab skill` pointers. The title is nonempty and bounded to 256 KiB UTF-8. Omitted
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

`tmt colab page layout <page> column|full|canvas [--reason <text>] [--expected-revision <revision>] [--json]`
changes only the page's layout: the source, title and attribution are the page's own, and
the change publishes through the same atomic content batch as `page write`, through serve when
running or under its lifecycle lock when stopped. The kind and reason follow the same rules as
creation; moving to `column` or `full` removes the reason. Setting the layout the page already
has publishes nothing and reports `changed: false`. `--expected-revision` refuses a page that
moved since that revision. The JSON result is the write receipt plus `layout` and, for a
canvas, `layoutReason`. A page that has no layout (freeform) can be given one this way; there
is no command that returns a page to freeform.

`tmt colab page create --title <title> --template <name> [--json]` starts from a template instead
of a layout choice and a file: `--template` conflicts with `--layout`, `--reason` and `--file`,
because the template brings its own layout and source. The catalog is closed and compiled into the
binary: `report`, `review` and `compare` (layout `column`) and `board` (layout `full`). Each source
is under 8 KiB and passes the page check with no finding. It starts from the page-style starter
(`tmt colab skill page-style`) without that starter's `body` rule, which frames `canvas` and
`freeform` pages, and adds no width, margin or gap CSS of its own: those stay the frame's
(`data-width` and `data-gap` are the only declarations it uses). A one-line
HTML comment before each block names the rule and the `tmt colab skill` topic and section that
teaches it; the comments are not rendered. Each `{{uuid:NAME}}` placeholder becomes one fresh
canonical UUID per name on that page, so no two pages share a block ID. The create result adds
`template` (the human result a `template` row) to the layout fields and `read`. A template page is
an ordinary page: nothing stored marks it. `tmt colab template ls [--json]` lists the catalog as
`templates: [{name, layout, summary}]`; it reads no local state and needs no running serve. A
create without `--layout` or `--template` is refused as above, and the refusal names both.

`ls` and `show` report each page's layout from its verified metadata: JSON `layout`
(`column`, `full`, `canvas`, or `freeform` for a page without the key; `null` when the page's
title cannot be read, as for an archived page) and, for a canvas, the whole `layoutReason`;
the human views print `layout <kind>` and, for a canvas, `reason <text>` on one line, control
characters escaped and nothing truncated.

Already admitted, unexpired, unrevoked owner devices receive wraps through the
existing recipient planner. Later owner-device registration supplies missing
forward wraps for existing non-archived pages inside the same authenticated device
transaction, without a new log operation. Current history supplies only the current
key; shared history supplies at most 64 retained epochs. Registration creates at
most 512 missing wraps per transaction and rejects capacity atomically. Exact
registration retries reuse existing device/epoch wraps without growth or fresh
signing; revoked devices receive nothing. These wraps confer no new Remote rights.

JSON success is `{spaceId,pageId,title,path,operationId,membershipHead,warnings}` with
`membershipHead:{revision,statementHash}` (decimal text and canonical base64url
hash32), and `warnings` the [page markup check](#page-markup-check-2753) findings of the source (`[]` when clean). `path` is the root-relative short alias
`/p/<shortId>`; it carries no space and pins nothing. The CLI learns the door only through
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
revision,memoryLimit,usage}` (`usage` is defined under
[Page usage and requested compaction](#page-usage-and-requested-compaction-2846)) with optional bounded `publisherAgent` and `originalAuthor` and optional complete `creationRecipient`, omitted
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
`{spaceId,pageId,epoch,membershipHead,revision,sourceSha256,memoryLimit,changed,update,warnings}`
and `usage` (the page's usage after the write, omitted when the page cannot be read back
right after it) plus, when `changed` is true, `{operationId,streamId,count,seq,envelopeHash}`. `update` is
`{count,bytes}`, plus `minimal:true` when `changed` is true: the number of updates in the
committed batch (equal to `count`) and the total bytes of their sealed envelopes (the signed
manifest's `packetBytes`), so what an edit cost is measurable. `minimal` says the batch came from
the text diff, which every write does today; it stays so a future whole-replace path can say
`false`. `changed:false` reports `update:{count:0,bytes:0}` with no `minimal`, since no batch
exists. The numbers come from the frozen job, so a reply lost to doubt and answered from the
retained outcome reports the original batch. The human output adds one `update` line
(`1 update, 214 bytes`, or `none`). `revision` is the
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

### Ranged read and anchored patch (#2836)

`page read PAGE --lines A:B [--json]` reads the inclusive line range from the same
owner-admitted snapshot as the whole-page read, with the identical opaque `revision`.
Lines follow `page/lines.rs`: 1-based LF records include their terminating LF, CR remains
content, trailing LF adds no synthetic record, and an empty page is one empty record.
`rangeSha256` is lowercase SHA-256 of their exact concatenated UTF-8 bytes, with no
number prefixes, escaping, normalization or inserted newline. JSON retains the whole-read
metadata but its `source` is only those bytes, with `lineStart`, `lineEnd`, and
`rangeSha256`. Human output numbers each line and prints the range/hash; its verified
metadata remains on stderr. An out-of-page range refuses with `COLAB_READ_RANGE`.

`--grep TEXT [--context N]` finds non-overlapping occurrences of nonempty literal text,
including text across LF boundaries. Regex is not supported in v1. Context defaults to zero,
expands each match to surrounding lines, and overlapping or adjacent ranges merge. JSON
replaces the whole source with `matches` (occurrences) and `blocks`, each containing
`lineStart`, `lineEnd`, `source`, and `rangeSha256`; human output numbers those same blocks.
No match is a successful empty block list. `--lines` and `--grep` are exclusive, and
`--context` requires `--grep`. Pattern inputs are at most 4 KiB, context 0..1000 lines,
matches at most 1,000, and complete ranged/search output at most 8 MiB (including JSON
escaping or human number prefixes). Exceeding a result bound is `COLAB_READ_TOO_LARGE`
before any output or file publication, never a truncated result. The byte bound admits a
huge single line; it is not a line-length bound. Unflagged reads are unchanged.

`--out FILE [--lines A:B] [--json]` publishes only the exact whole-page or selected range
bytes to a new 0600 file, with no added newline, JSON, or display labels in the file. It
conflicts with `--grep`. JSON reports `path` (canonical parent), `bytes`, `sha256`,
`revision`, and `pageId`, plus the range fields when selected; human output reports the
path and the unencrypted-copy disclosure. The parent must exist; aliases are resolved once,
parent traversal, a non-UTF-8 destination, and an existing path (including a symlink) are
refused. File bytes also obey the 8 MiB output bound. Publication
uses the existing plaintext export's owned staging and exclusive no-follow file rules;
failed prepublication removes staging without replacing user data. This is a source-only
copy: `tmt colab export` remains the full-fidelity page/conversation/attachment export.

`tmt colab page patch <page> <form> [--expected-revision <token>] [--strict] [--json]` changes
part of a page's source without moving the rest. Exactly one form names the change; each carries
the text the caller expects, so the change is anchored to that text and not to the page as a whole:

- `--find TEXT --replace TEXT` (or `--find-file PATH`, `--replace-file PATH`, `-` for stdin): the
  find text must occur exactly once, anywhere on the page, overlapping occurrences included; the
  replacement may be empty. Files are read byte for byte.
- `--lines A:B --range-sha256 HASH --file PATH`: lines `A` through `B` (1-based, split on LF only,
  a CR is content, a final LF ends the last line instead of starting another, an empty page is one
  empty line) are replaced by the file's exact bytes, only while their `rangeSha256` is `HASH`. The
  range is the exact bytes from the first byte of line `A` through the LF that ends line `B`, or
  the end of the source when `B` is the last line and has none, so a replacement of whole lines
  ends with an LF. `page/lines.rs` is the one definition of these lines, shared with the ranged
  read (#2840) that reports each range's `rangeSha256`.
- `--diff PATH`: a unified diff. Anything before the first `@@` header is ignored; a hunk is read by
  the counts in its header, so a removed line that starts with `--` is content. A hunk's context
  and removed lines are matched exactly, with no fuzz and no whitespace tolerance, at the start
  of a line: it applies where that text occurs once, or, when it occurs more than once, where it
  starts at the hunk's stated old line. A hunk with no context or removed line (a pure insertion
  with no context) has nothing to anchor it and is refused. Hunks are placed against the page as
  read, may not overlap, and apply together or not at all.

The patch is evaluated, as a pure function of the source, against the exact source the page folds
to when the command reads it, and its result is written through the whole-source write
(`page write`'s preparation, commit and receipt), which commits the minimal text diff of old to
new source. Everything `page write` guarantees therefore holds unchanged: one atomic batch, the
source, tail and page limits (`COLAB_CAPACITY`), the markup check on the result (`--strict` refuses
findings in the result before anything is written), `changed:false` when the result equals the
source, the retained revision and the lost-reply behaviour, and the `update` field of the result.

A missing, ambiguous or changed anchor refuses with `COLAB_PATCH_CONFLICT`, naming the find text,
line range or hunk and, for an ambiguous match, how many there were; nothing is written. A
malformed diff, an empty find text, a `--find` without its replacement or a malformed line range
is `COLAB_INPUT_INVALID`. A concurrent edit elsewhere on the page does not refuse a patch: the
preparation refuses a page whose source is no longer the one the patch was evaluated against, and
the commit refuses a page that moved after preparation (`COLAB_STALE_BASE`), and the command then
evaluates the same patch against the page as it now is, preparing and committing again, at most 3
more times; an edit to the addressed text makes that evaluation fail with `COLAB_PATCH_CONFLICT`.
`--expected-revision` is the optional strict fence of the whole-source write: with it any change
to the page since that revision, anywhere, is `COLAB_STALE_BASE`, and nothing is retried.

### Page usage and requested compaction (#2846)

`page read --json` reports `usage`, the accounting a write is refused against:

| Field                              | Meaning                                                                                                                                                                                                      |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `updates`, `updateLimit` (200)     | Changes after the checkpoints, over every device's stream.                                                                                                                                                   |
| `tailBytes`, `tailBytesLimit`      | Their plaintext bytes (4 MiB).                                                                                                                                                                               |
| `checkpointBytes`                  | Plaintext bytes of the checkpoints.                                                                                                                                                                          |
| `otherDeviceUpdates`               | Of `updates`, those on other devices' streams. Those devices can combine them, or an owner device's [fold](#owner-fold-3983).                                                                                |
| `stateBytes`                       | The whole state, uncompressed: baseline, checkpoints and tail.                                                                                                                                               |
| `gzipBytes`, `gzipLimit` (5000000) | The state gzipped as a browser loads it. Measured from a `stateBytes` of 256 KiB; absent below, where `stateBytes` bounds it by 5% of the limit.                                                             |
| `limiting`                         | Which of the three `percent` is: `changes`, `recent-data` (the tail's bytes) or `stored-size` (the gzipped state); the first of the fullest wins a tie. Combining changes makes room only for the first two. |
| `percent`                          | The fullest of `updates`/`updateLimit`, `tailBytes`/`tailBytesLimit` and (`gzipBytes`, else `stateBytes`)/`gzipLimit`, rounded down; above 100 for a reader that holds more than a write accepts.            |

`page write` and `page patch` report the same `usage` in their JSON receipt, and their human output
adds a `usage` line only from 80%, saying how full the page is and, when this device has changes of
its own to combine, naming `tmt colab page compact`.

The numbers are the write path's own: the tail count and bytes `admit_deltas` refuses at, and the
gzipped state `admit_gzip` refuses at. A refusal caused by the tail (`COLAB_CAPACITY`: more than
200 changes or 4 MiB since the checkpoints) names `tmt colab page compact <page>` before the
export-and-recreate way out; one caused by the source size or the gzipped state does not, because
combining changes does not make those smaller.

`tmt colab page compact <page> [--json]` combines the root-local device's own stream, then folds the
other owner-member devices' streams ([owner fold](#owner-fold-3983)), with the
automatic combine's rules and a trigger of one change (`page/compact.rs`: the stream's own
verified updates, merged inside the isolated decoder, one checkpoint per namespace at the
stream's head). While `tmt colab serve` runs it asks the serve over the owner-only socket route
`POST /.tmt/colab/local/page-compact` with `{"version":1,"page":ID}`; otherwise it runs under the
serve lifecycle lock. The route answers local callers only: a forwarded device context or device
event is `COLAB_DENIED`, another body version `COLAB_SERVER_MISMATCH`, a malformed body
`COLAB_INPUT_INVALID`; it runs under the sync mutex that excludes every write, so a combine
never races a write through the serve. The reply is a `Report`: `pageId`, `compacted`,
`skipped` (`nothing-to-combine` when this device has no change after its checkpoints,
`not-combined` when it has but the merge could not be published, for example because it would be
over one checkpoint's size or did not finish in time), `by` (the device whose stream was combined),
`through` (the checkpoints now cover that stream's changes `1..=through`; 0 when nothing was
combined), `mergedObjects`, `sourceSha256` (the folded source after) and `before` / `after`
usage; `folded`, `left` and `unfolded` are described under the owner fold, and `compacted` is true when
either this device's stream or another's was combined. The folded source is the same before and after: the merge is compared with the page's own
projections before its first checkpoint is published, or nothing is. A second request finds no
change after the checkpoints and writes nothing (`compacted:false`, the stored state
byte-identical). Archived and deleted pages refuse as a write does.

A combine moves the page revision, so a revision read before it is `COLAB_STALE_BASE` for the
next write. The merged changes' per-edit attribution is replaced by the checkpoint, which is
signed by the device and covers `1..=through`; no ledger or schema is added. Only a device can
combine its own stream (the [owner fold](#owner-fold-3983) is the one exception). A browser device combines its own `own` stream by sealing a `checkpoint` at
its head and sending it as an `append` (the browser's Compact asks the serve to combine the
root-local stream, then does this); it counts its own changes as its own, so only the changes of
other devices are `otherDeviceUpdates` to it. Where a stream is rewritten, the state encoding a
browser publishes is far smaller than a merge, which keeps deleted content (bytes, Yjs 13.6.32):

| An own stream of                       | The updates | `mergeUpdates` | State encoding |
| -------------------------------------- | ----------- | -------------- | -------------- |
| 200 comments of 1 KiB, each once       | 215,052     | 213,787        | 213,787        |
| one 2 KiB reply record, rewritten 100x | 210,744     | 209,660        | 2,122          |
| one 20 KiB record, rewritten 300x      | 6,157,418   | 6,153,793      | 20,540         |

A merge of the last two would not fit the 256 KiB one update may hold, so only the state encoding
can be published as a checkpoint.

Merging keeps the text each rewrite deleted, so combining ends the tail, not the growth of the
checkpoint. Measured with `cargo test --release -p tmt-colab --test page churn -- --ignored
--nocapture` (a 20 KiB page of random words replaced whole each time, combined every 10
rewrites), the checkpoint grows by about the page's own size per rewrite and the gzipped state by
about 42% of that:

| Rewrites | Checkpoint | Gzipped state | `percent` |
| -------- | ---------- | ------------- | --------- |
| 100      | 2.05 MB    | 0.87 MB       | 17        |
| 200      | 4.11 MB    | 1.73 MB       | 34        |
| 300      | 6.16 MB    | 2.60 MB       | 51        |
| 400      | 8.21 MB    | 3.46 MB       | 69        |

Only a rebuild into a new epoch drops deleted text; compaction does not, so a page rewritten
whole a few hundred times reaches its gzipped budget however often it is combined. That is why
`percent` includes the gzipped state.

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

## Page markup check (#2753)

`tmt colab page check <file|-> [--page <page>] [--json]` reports what is wrong with an
agent-written page source before anyone opens it. It is a pure function of the source: it
reads one file (or bounded stdin, with the page-write reader's 2 MiB UTF-8 limit and
five-second EOF deadline) and needs no running serve, no local state, no core and no data
root. `--page` additionally resolves `colab-attachment:` references against that page's current
document attachments in the local store (read only; it starts nothing).

The source is parsed with the HTML tree-construction algorithm in a `template` fragment
context with scripting off, which is what `template.innerHTML` does, so every verdict is on
the browser's tree: implied end tags, formatting-element reconstruction, foster parenting,
foreign content and entity decoding land where the browser puts them. A source nested deeper
than 512 elements stops the parse with one `page-too-deep` finding, because a browser stops
nesting there and nothing past it can be judged.

A finding is `{code, severity, element, ordinal, id, line, message}`. `element` is the
lowercase tag name and `ordinal` the element's 1-based index among elements with that tag in
document order, counting ignored and nested ones. `id` is the authored `data-id` or `null`.
`line` is the 1-based line of the end of the element's start tag. `severity` is `ignored`
(the browser drops the block today) or `warning` (the block is admitted, or the problem is not
about a block). `message` is wording that may change; `code` is the stable part. A finding on a
skipped group adds `questionsSkipped`, the number of `tmt-question` elements inside it. Findings
are ordered by line, element, ordinal and code.

Success is `{format:"tmt-colab-page-check",version:1,ok,bytes,questions:{admitted,ignored},
reviews:{admitted,ignored},readings:{admitted,ignored},findings}`. With no finding it exits 0. With any finding the command exits 1 with
`COLAB_PAGE_CHECK`, and `--json` prints the same report with `ok:false` and the usual
`{error:{code,message}}` attached; human output lists each finding with its line first.

`page create` and `page write` run the same check on the source before any state changes. The
findings are warnings: on stderr in human mode and a `warnings` array (the finding objects,
`[]` when clean) in the JSON result, also for a write that changes nothing. A page being
created has no attachments, so every well-formed reference in its first source is
`attachment-ref-unresolved`. `--strict` turns any finding into a `COLAB_PAGE_CHECK` refusal
before anything is committed; the JSON error carries the report.

Reason codes (a block fails every rule listed; title rules wait for exactly one heading and
the choice-shape rules for a valid kind, so one defect is not reported twice):

| Code                                                              | Severity | When                                                                                                                                                                                      |
| ----------------------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `page-too-deep`                                                   | warning  | an element nests past 512 levels; nothing after it is checked                                                                                                                             |
| `question-page-limit`                                             | ignored  | more than 50 `tmt-question` on the page; the browser shows none of them, and this is the only finding (on the 51st)                                                                       |
| `question-id-invalid`                                             | ignored  | `data-id` missing, or not a canonical lowercase UUID                                                                                                                                      |
| `question-id-duplicate`                                           | ignored  | a well-formed `data-id` shared with another question or a group (one namespace, below); every holder is ignored                                                                           |
| `question-nested`                                                 | ignored  | inside another `tmt-question`                                                                                                                                                             |
| `question-group-child-invalid`                                    | ignored  | a `tmt-question` inside a `tmt-questions` group but not its direct child (inside another `tmt-question` it is `question-nested`); it never becomes a lone question                        |
| `question-heading-missing` / `-multiple`                          | ignored  | not exactly one direct child with `data-question`                                                                                                                                         |
| `question-title-empty` / `-too-long` / `-control`                 | ignored  | the heading text, whitespace-normalized, is empty, over 200 code points, or has a control character (below U+0020 or U+007F)                                                              |
| `question-kind-invalid`                                           | ignored  | `data-kind` is not `single`, `multiple`, `short` or `long`                                                                                                                                |
| `question-choices-too-many`                                       | ignored  | more than 20 direct `data-choice` children                                                                                                                                                |
| `question-choice-empty` / `-too-long` / `-control` / `-duplicate` | ignored  | a choice fails the same text bounds (200 code points), or two are equal after normalization                                                                                               |
| `question-choices-missing`                                        | ignored  | `single` or `multiple` without a choice                                                                                                                                                   |
| `question-choices-unexpected` / `question-other-unexpected`       | ignored  | `short` or `long` with a choice, or with `data-other`                                                                                                                                     |
| `question-group-id-invalid`                                       | ignored  | explicit `data-group` is not a canonical lowercase UUID                                                                                                                                   |
| `question-group-conflict`                                         | ignored  | `data-group` appears inside a wrapper or its ID collides in the shared control namespace                                                                                                  |
| `question-group-limit`                                            | ignored  | more than 12 raw standalone questions share a logical group; none of its members supplies controls                                                                                        |
| `question-agent-invalid`                                          | ignored  | `data-agent` over 64 code points or with a control character                                                                                                                              |
| `question-marker-misplaced`                                       | warning  | an element with `data-question` or `data-choice` whose parent is not a `tmt-question`: plain content to the browser                                                                       |
| `group-id-invalid` / `-duplicate`                                 | ignored  | a group `data-id` missing or not a canonical lowercase UUID, or shared (see the namespace below); every holder is ignored                                                                 |
| `group-nested`                                                    | ignored  | a group inside another group or inside a `tmt-question`                                                                                                                                   |
| `group-title-missing` / `-multiple`                               | ignored  | not exactly one direct `<p data-title>` child                                                                                                                                             |
| `group-title-empty` / `-too-long` / `-control`                    | ignored  | the title text fails the question heading bounds (non-empty, 200 code points, no control character)                                                                                       |
| `group-questions-too-many`                                        | ignored  | more than 12 direct `tmt-question` children                                                                                                                                               |
| `group-empty`                                                     | ignored  | the group passed its rules but none of its questions is shown                                                                                                                             |
| `group-label-too-long` / `-control`                               | warning  | a shown question's `data-label` is over 24 code points or has a control character; its tab shows `Q<n>`; the finding is on the question                                                   |
| `group-form-invalid` / `question-form-invalid`                    | warning  | `data-form` on a shown group or lone question has any value but `chip` (exact, lowercase); the invalid presentation hint is ignored                                                       |
| `question-form-member`                                            | warning  | a shown question inside a group carries `data-form`; it is ignored because the group decides; the finding is on the question                                                              |
| `reading-page-limit`                                              | ignored  | more than 50 reading anchors; none supplies reading controls                                                                                                                              |
| `reading-id-invalid` / `-duplicate`                               | ignored  | missing/noncanonical UUID, or an ID shared with a reading, review, question or group                                                                                                      |
| `reading-nested`                                                  | ignored  | contains, or sits inside, a reading, review, question or question group                                                                                                                   |
| `reading-heading-missing` / `-multiple`                           | ignored  | not exactly one immediate `data-title` child                                                                                                                                              |
| `reading-title-empty` / `-too-long` / `-control`                  | ignored  | normalized title empty, over 200 code points or with a control character                                                                                                                  |
| `reading-agent-invalid`                                           | ignored  | missing/blank, over 64 code points or with a control character                                                                                                                            |
| `reading-why-invalid`                                             | ignored  | missing/blank, over 200 code points or with a control character                                                                                                                           |
| `review-page-limit`                                               | ignored  | more than 50 `tmt-review` on the page; no section is shown (its questions are unaffected), and this is the only section finding (on the 51st)                                             |
| `review-id-invalid` / `-duplicate`                                | ignored  | a section `data-id` missing or not a canonical lowercase UUID, or shared with another section, a question or a group; every holder is ignored                                             |
| `review-nested`                                                   | ignored  | inside another `tmt-review`, a `tmt-question` or a `tmt-questions` group                                                                                                                  |
| `review-heading-missing` / `-multiple`                            | ignored  | not exactly one direct child with `data-title`                                                                                                                                            |
| `review-title-empty` / `-too-long` / `-control`                   | ignored  | the heading text fails the question heading bounds (non-empty, 200 code points, no control character)                                                                                     |
| `review-agent-invalid`                                            | ignored  | `data-agent` over 64 code points or with a control character                                                                                                                              |
| `image-data-oversized`                                            | warning  | an `<img>` whose `src` is a `data:image/` URL over 32 KiB                                                                                                                                 |
| `image-data-total`                                                | warning  | all `data:image/` sources together over 256 KiB; one finding, on the image that crosses it                                                                                                |
| `attachment-ref-invalid`                                          | warning  | an `<img>` `src` starting `colab-attachment:` that is not the scheme plus 8 lowercase hex digits or a full lowercase UUID                                                                 |
| `attachment-ref-too-many`                                         | warning  | more than 128 distinct well-formed references; one finding, on the 129th                                                                                                                  |
| `attachment-ref-unresolved` / `-ambiguous`                        | warning  | with `--page`, `page write` or `page create`: no, or more than one, current document attachment of the page matches the reference (an 8-digit token is a prefix, a full ID must be equal) |

Groups (`tmt-questions`, a `<p data-title>` and up to 12 direct `tmt-question` children). A group
reports the first structural rule it fails, in table order, and is then skipped whole: its
questions and any group inside it are silent, with no finding, and its finding carries
`questionsSkipped`. A group that passes shows its direct questions, each reporting its own
findings, and `group-empty` is added when none is shown. A lone `tmt-question` outside any group
stays a group of one. A tab label is the authored `data-label`, else `Q<n>` by the question's
order among the group's shown questions; a bad label only warns. The 50-question page limit
counts every `tmt-question`, grouped or not, and then reports nothing else, sections included.
A section that fails a rule is ignored alone: the questions inside it are judged as usual.

Question, group, section and lone-question IDs are one namespace and every holder of a shared ID
is ignored. Every group, and every section on a page within the 50-section cap, holds its own
`data-id` wherever it sits. A question holds its ID only as a
lone question or as a direct child of a group that passes the structural rules other than a
duplicate ID, so the questions of a group skipped for any other structural rule hold none, and
neither does a question wrapped in another element. Holders are counted once and never
retracted: the questions of a group skipped only for a duplicate ID still hold theirs.

Text is normalized as the browser client does: every run of JavaScript whitespace (`\s`,
which includes U+00A0 and U+FEFF and excludes U+0085) becomes one space and the ends are
trimmed; limits count code points.

One rule set, two implementations. The browser client's `inspectQuestions` and this checker
run the shared vectors `contracts/vectors/page-markup-v1.json`: each case gives a source, the
questions the page shows and the findings, with `scope:"parser"` cases run by both and
`scope:"checker"` cases (references, depth) by the checker alone. The browser's own
parser is the oracle, in headless Chromium; WebKit agrees on every parser case and Firefox
differs only on `<noscript>`, which Chromium and WebKit parse as markup. The vectors' `limits`
block must equal each side's named constants, and a test fails when a code in this table has
no case or a case names an unknown code. Four codes have checker unit tests instead: the two
image sizes need payloads of tens of KiB, and reference resolution needs a page's attachments.

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

For a [dashboard](#dashboards-3648), `shell.json` replaces `page.html` and
contains the canonical shell value, including summary attribution. The manifest
also carries `pageType:"shell"`; ordinary HTML exports omit it. An ordinary page
that has [work](#work-4050) also exports `work.json` right after `page.html`: the
canonical `meta.work` value (sorted ASCII keys, compact separators, raw UTF-8), so
a copy never loses its tasks or summary; a page without work has no such file.
All other snapshot, privacy, file-publication and failure rules are unchanged.

The manifest is UTF-8 JSON with these fields:

| Field               | Value / meaning                                                                                                                                                             |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `format`            | `tmt-colab-page-export`                                                                                                                                                     |
| `version`           | JSON integer `1`                                                                                                                                                            |
| `spaceId`           | Pinned space ID                                                                                                                                                             |
| `pageId`            | Exported page UUID                                                                                                                                                          |
| `title`             | Exact admitted title                                                                                                                                                        |
| `pageType`          | `"shell"` for a shell export; omitted for ordinary HTML pages                                                                                                               |
| `originalAuthor`    | Optional bounded creation display label, omitted when unknown; never identity proof                                                                                         |
| `publisherAgent`    | Optional latest CLI publisher display label, omitted when unknown                                                                                                           |
| `creationRecipient` | Optional complete `{machineId,agentId}` creation preference; omitted when absent                                                                                            |
| `exportedAtMs`      | Safe-integer UTC milliseconds                                                                                                                                               |
| `membershipHead`    | `{revision, statementHash}`; decimal revision, lowercase hex SHA-256                                                                                                        |
| `epoch`             | Current snapshot epoch as canonical positive decimal text                                                                                                                   |
| `plaintext`         | `true`                                                                                                                                                                      |
| `discussions`       | `{included:true, scope:"current-epoch", format:"tmt-colab-conversations", version:1}`                                                                                       |
| `attachments`       | Always present, possibly empty: one row per attachment, see below                                                                                                           |
| `files`             | `page.html` (or `shell.json`), `work.json` when the page has work, `conversations.json`, `conversations.md` as `{name, sizeBytes, sha256}`; bytes and lowercase hex SHA-256 |

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

### Attachment read command (#1867, #2464)

`tmt colab attachment read <page> (<attachment> | --reference <file>) [--output <directory>] [--json]` writes one
attachment into a new private UUID-named subdirectory of `--output` (default: the system
temporary directory, never the current directory, so an agent's checkout stays untouched),
created the way export creates its directory: `attachment.<ext>` then
`manifest.json` last, regular 0600 files in a 0700 directory, no symlinks, no replacement,
only its own staging cleaned. The CLI re-derives the media type from the verified plaintext
using the byte rules above, including for older attachments with stale signed labels. The output
manifest records that derived type, and the extension is `png`, `jpg`, `webp`, `pdf` or `txt` for
the corresponding derived type; every other byte sequence is `bin`. Neither the descriptor label
nor the serve's `Content-Type` can select the output extension. The author's file name never
reaches the path. `--reference` is the exact `reference` of an export manifest row
(at most 2 KiB). `<attachment>` is an attachment ID or a prefix of at least 8 hex characters (an
Ask lists the first 8; `tmt colab threads --json` the full ID). The CLI resolves it in the verified
local view to the exact reference, among the files of live comments in the verified discussion
projection (the one `conversations.json` is built from, so its revision-chain and deletion rules decide;
current epoch or the earlier-epoch window a message read searches) and, in the current epoch, the
page's document attachments; no match is `COLAB_STATE_MISSING`, a malformed or
too short prefix or one matching more than one file is `COLAB_INPUT_INVALID`, and it never guesses.
The read itself is then unchanged, so a revoked or ended access or a moved epoch reads as
unavailable and a deleted message no longer resolves; the manifest records the resolved reference. A
read stays valid against what binds its reference: a message attachment against the membership head,
epoch and author alone, so an unrelated record (a reply, a status or another writer's update) landing
while it reads never refuses it, and a document attachment against the page revision it was listed at. The manifest is `{format:"tmt-colab-attachment-read", version:1, pageId,
reference, file:{name,mediaType,sizeBytes,sha256}}`; the result lists the directory, files, `path`
(the written file), size and digest and never the bytes. Authority is the owner-only socket of this data root, not a
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

`tmt colab attachment attach <page> <file> [--json]` adds one file to a
page's document attachments, authored by this data root's local writer; `attach <page> --resume
<slot>` finishes an earlier attempt. Document attachments only: a message attachment needs a native
message author, which does not exist yet. It needs the running serve, an established object channel
and a page the local writer may edit now; an archived or deleted page refuses `COLAB_PAGE_INACTIVE`
and a revoked writer `COLAB_DENIED`, before anything is staged. The file is a regular file of the
caller opened without following a link, at most 8 MiB, checked before and while it streams. Its name
is a display label only. The serve derives the sealed descriptor's type from the staged plaintext
using the same raster/PDF/UTF-8 rules as the browser; no declared type or filename selects it.
`--type` is refused. An existing staging slot's type label does not override byte detection when
it is resumed.

**Slot.** `POST /.tmt/colab/local/attachment-stage` with `{version:1,page,filename}`
opens a slot: a random 32-hex ID and a private 0700 directory under the serve's root, answered as
`{slot,path}`; no route accepts a path. The CLI prints the slot to stderr, copies the file into
`path` (create-new, 0600, bounded while streaming) and reports its SHA-256 to
`POST /.tmt/colab/local/attachment-attach` with `{version:1,page,slot,sha256?}`. The serve re-hashes
the copy, seals it under the page's current epoch as the local writer (the descriptor and object of
a browser upload), and writes the ciphertext, descriptor, base and a fresh transfer ID to the slot
before the first request leaves, removing the plaintext. A slot stores no media-type label; the
serve derives it when sealing. Existing version-1 slots still resume: the obsolete `mediaType`
key is accepted and ignored, never written again; other unknown keys are refused. Both routes
deny a Remote context or event header.

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
Readers therefore see the old complete reference or the new complete one, never a half swap.
A second ordinary owner Save repairs complete `img src` values containing `colab-attachment:`
references to the old full ID or unique id8 with the new full ID, from the latest verified source and without an attachment delta.
Unrelated source bytes remain unchanged, including prose, examples, comments and other attributes;
ambiguous short references are left alone. If the editable values do not agree with the parsed
images, the source stays unchanged and the repair waits. A stale Save
is derived again, and an uncertain reply is settled by reading the current page before another
Save. Until that repair, an image reference shows the normal unavailable placeholder; it is
repaired on the next successful maintenance pass. A descriptor is swapped inside the baseline neither now nor later: its creation proof is an own
record of the new epoch and cannot exist before it.

**Triggers and bounds.** One worker runs at serve start, after any management change, when an
object channel is established, and after an unfinished pass on a backoff that starts at a minute, doubles to an hour and starts over on any of the other triggers. A pass takes pages whose epoch
is past 1, at most 4 attachments per page and 120 s per page; the rest wait for the next pass.
Before the swap, quota exhaustion, a lost channel, a crash or a serve stop leave the old attachment
reference, and the next pass
resumes the slot's frozen transfer ID without a second upload. A page that moved meanwhile, an edit
or a further advance, is a terminal stale base for that attempt: the original is discarded and the
swap is derived again from the new page, at most three times per pass, never over the edit. An
attachment removed meanwhile is left removed. Finished replacement slots retain their small
old-to-new mapping until a verified no-op, successful reference Save, or removal settles it.
They count toward the existing 16-slot bound; when full, another replacement waits. Restart
reconciles committed swaps before age cleanup, including swaps committed before the slot
finished, and repairs earlier mappings before a further key change replaces their target again.
Plaintext and ciphertext cleanup is unchanged.

**Status.** Each Files row of a page whose epoch is newer than its file says `Securing after a key
change. Still available.`; the line is derived from the page and has no state of its own.

### Browser page title hints (#1564)

The paired owner browser takes a page's title from the owner's local title projection when
`GET /api/pages` carries one (Schema 16), and otherwise from accepted Live folds. Home
rows and the share/manage dialog show the last known title; the current page also
updates the parent tab title. UUIDs remain routing identities and appear under
Details. Without a usable title, chrome shows “Untitled page”. Opening the page replaces a stale or missing hint with its verified folded
title. A projected title is current for the page and is shown instead of a browser hint
without opening the page or writing the hint. Home opens a page's content only to fill a missing
title hint for a page the owner has no title for, bounded and
cancellable; it never replaces a cached hint without opening the page. For such a page a title
changed elsewhere can remain stale in this browser until that page is opened.

Home admits only visible uncached rows: one read at a time, at most 24 attempted
pages and 16 MiB of admitted incoming sync frames per Home generation, including failed
reads. It uses the existing verified read catch-up and isolated fold, never an editor,
Ask or a write. Each connection closes and joins before its successor; leaving Home or
replacing the mounted session cancels its reads. A failure or exhausted budget keeps
the existing fallback; Home adds no plaintext title endpoint of its own.

The paired mounted app may show a display-only Remote session snapshot in the shared
header status slot. The existing signed SDK device list is read sequentially at mount,
visible-tab return and details opening, at most 20 pages of 50 devices with one read in
flight. Only a completed traversal supplies a total; refusal, timeout or the page bound
shows unconfirmed, never a partial sum or zero. Revoked/expired zero-session rows are omitted, the
current browser sorts first, and details show only positive-session rows, retaining nullable
activity and read time. Counts are live Sessions, not tabs; a closed tab can remain counted for about a minute. Successful
reads expose only same-origin `/settings`; hosted/detached apps or an absent admitted
Session hide the control. Session replacement fences late results and starts one read after
the stale read settles; close fences results and prevents that successor. This adds no
Remote operation, authority, subscription or periodic polling.

Only foreground page views (paired and shared-reader) send explicit `subscribe` after
catch-up. A valid Subscribe selects the active cloud page, and every successful store still
selects it. Existing early write wakes are unchanged; Hello and ACK, including Home title
reads, leave that page unchanged.

These are browser-local display hints, not membership, policy or access evidence.
The Colab keyring owns a non-extractable AES-GCM-256 key per device in its existing
IndexedDB store. Title records contain only a fresh 12-byte nonce and ciphertext;
AAD is the JSON array `["tmt-colab-title-cache-v1", space, device, page]`. Record
lookup is likewise scoped to space/device/page. The title shares the existing
fold projection bound. Cached title plaintext and this local key never enter discovery,
routes, logs, the renderer, or the decoder Worker. The cache adds no native schema, title
endpoint, Remote certification or network protocol.

Cache absence, corruption, scope mismatch or storage failure falls back to the
unopened label without denying page reading or management. Only an accepted foreground fold or an admitted missing-title Home fold may update
the cache, through the current mounted registration and tab lifetime. Home's fill is
serialized with foreground writes and cannot overwrite a usable cached hint. Reload can read the persisted encrypted hint; a different paired
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
           comments:[{writer, id, revision, sequence, deleted, body, deviceName, at[, attachments]}]}],
 asks:[{writer, operationId, deviceName, agentName, agent, machine, thread, messageIds,
        issuedAt, expiresAt, message, state, reason, requestId,
        reply:null|{requestId, agentId, body}}]}
```

A live comment with files carries `attachments:[{id, name, mediaType, sizeBytes}]` (full attachment
ID, the descriptor's label and media type, plaintext size as a JSON integer) in its descriptor order, and
`conversations.md` lists each as `- Attachment: <id> <name as a code span> (<type>, <n> bytes)` after
`Revision`; a comment without files has neither, so its bytes are unchanged.

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
`writer + ":" + id`, comments inside a thread by the sequence order of the discussion records
(ascending `sequence`, then writer, then ID; a comment without one is `"0"`), asks by `writer + ":" +
operationId`, all by byte order (never a locale comparison). Bodies and messages are exact
UTF-8, including controls. Order never depends on `at`.

**`conversations.md`** is a plain reading of the same data and is deterministic: a
`# Conversations` heading, a header list (page title, page ID, space, epoch, head), one
line stating that names and times are writer-asserted labels, then `## Threads (n)` and
`## Asks (n)`. For reading, threads are ordered by their `at`, comments by the sequence order
above, asks by `issuedAt`, each `at` or `issuedAt` order with ties broken by the `writer:id`
key; the number of the thread, comment or ask follows that order. A time renders as `YYYY-MM-DD HH:MM:SS UTC` before
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
the same dialog; local samples remain read-only. Home offers the tabs In progress, Discussion
and Done (the stage of each row's `summary.work`, a fresh summary without work being in
discussion), Archived and All, and opens on In progress (a link with `stage=all` opens on All; a mounted address is the list and no more, so a reload,
Back and Forward return to the tab the entry was left on); an
archived page counts only under Archived, and a page whose summary has not arrived is in no stage
tab. An empty In progress says so and points to Discussion and All, and never opens another tab by
itself. Its rows offer archive and delete, each behind a confirmation that
names the page, and a selection of rows takes them together (one signed statement per page, in
order, stopping at the first refusal), and the dialog provides retention days/forever, archive and explicit
delete confirmation. An archived row offers `Restore…` with a named confirmation, using the
same read/prepare/send/verify flow for `page.unarchive`. Policy and complete recipient
assignments derive from the contiguous verified owner log, never discovery labels.

Metadata catchup reuses bounded frame/chunk assembly and owner-log admission,
then closes before content decoding. Baseline ciphertext can be consumed as part
of wire framing but is never decrypted or materialized for management. A failed
preview does not remove home management access. Retention defaults to 30 days.
Chrome presents the native durable last-update/expiry display hints as relative
retention time, with the absolute local date on hover. The hint stays inside the
page card under its status badge. The seven-day warning and the not-in-the-cloud state
add a waiting mark and body text color; ordinary hints, including `local only`, use dim text. The copy is the
CLI's lowercase wording (`expires in 6 days`, `kept forever`, `local only`,
`not in the cloud`); the
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
