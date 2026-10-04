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
`/assets/recovery.js`. They are build-owned public bytes with no secret and no API. Every
other asset request without owner context, including `/index.html`, `/reader.html`,
`/THIRD-PARTY-NOTICES.txt` and the hashed owner assets, returns 403. `/read` has no trailing
slash so the entry's relative `./assets/` and `./renderer.html` references resolve under the mount. Unknown paths and owner
non-GET static requests return 404. Invalid context is rejected by the existing
HTTP admission. Dot path segments, backslashes, doubled leading slashes, percent
encodings, queries and fragments MUST reject with 400. No request path is
normalized, joined to a filesystem directory or given a SPA fallback. Existing
API routes and registered-owner `/sync` admission/transport are unchanged.

Vite output MUST use relative URLs beneath `/r/<prefix>/x/colab/`, with no
third-party requests. The current app declares installed/system font fallbacks;
no external font service is used. The app response CSP is exactly:

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
revocation. A private or deleted page denies readers; archive retains reads.
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
`path`; the owner prepends the Remote door address `tmt remote pair` printed. Everything is in
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
the seed with the model's `link::Keys` derivation, wipes the seed, generates a fresh Ed25519
device key and certifies it as a `link` issuer chain (certificate window: ten minutes of skew
before now, 24 hours after), then runs the challenge and session exchange above and opens
`/sync` with the ticket subprotocol; hello names the session's `principal`. It checks that the
returned owner key derives the linked `space`, verifies the owner log and link-addressed wraps
with the owner-browser rules, and pins and stores nothing in the owner app's records
(IndexedDB, local or session storage, cookies). It shows the page read-only and live, with no
editor, Ask, share, export or history control. The ten-minute session ends in a reconnect with
a fresh challenge; a `DENIED`/`STALE_EPOCH` result or a 403 from the exchange ends access
("Access ended"), which is what Reset, remove, narrowing or rotation produce. Every
challenge adds a link-device projection on the server, so a holder that opens the link often
adds one row per open until the link is removed or reset.

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

### Implemented raw own fold (#1264)

The owner-browser reader decrypts admitted own updates and checkpoints and folds
them into one document per authenticated writer in the page's single Worker.
The native isolated decoder and browser validate the exhaustive four map roots,
plain JSON values, absence of materialized list/shared-type mutations and resolved
dependencies. A present message `body` MUST be a string of at most 16 KiB UTF-8.
The 1,000-thread page limit sums map entries across writers, including retained
tombstone values; equal record keys in different writer documents count separately.
Both page folds enforce this limit before returning a view.

This is bounded raw-state admission. Exact thread/message/intent/reply field
schemas, tombstone interpretation and resolution projection remain deferred.
Raw references, signatures and operation IDs confer no authority or effect.
No comment UI, own writer API, Send execution or arbitrary core-result read is
implemented by this fold. The trusted binding returns detached writer-keyed raw
maps; renderer source remains content-only and the UI states that own data is not
displayed. The owner-browser author-policy subset remains unchanged.

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
update-v1 admission is 2 MiB + 256 KiB + 1 KiB framing. The existing 4 MiB
serialized stream cap applies to both modes. Verification sends update bytes,
authenticated source digest and title once; the child reconstructs source and
checks its digest and the commitment. Production checks its generated update
inside the child without transmitting both copies as input. Descriptor signing, encrypted publication, owner folds
and atomic epoch transitions remain caller-owned, later integration work.
[Baseline vectors](vectors/baseline-v1.json) pin update-v1 bytes and commitment
with a test-only fixed client ID; production uses a fresh identity. Their
[independent oracle](vectors/baseline-reference.py) covers only the fresh
`html`/`meta.title` schema and requires no third-party libraries.

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
The browser MUST bound each checkpoint and their aggregate catchup plaintext by
the existing 4 MiB Worker state budget, independently of the retained tail's
200-update/256 KiB budget. Apply checkpoints as single-item Worker steps before
the tail. Those unpublished steps may retain cross-writer pending dependencies;
the final tail step MUST resolve them and validate complete content before
publishing any view. The existing 2 MiB source projection cap remains in force.
Both namespaces count toward the aggregate checkpoint and tail plaintext budgets.
The Worker MUST also bound total encoded content plus all own documents, including
pending structs/delete sets, and the serialized combined projection to 4 MiB each.
Pending fragments MUST survive candidate cloning; final catchup validates every
document before publication. Live content/own candidates commit only after all
validation succeeds. Failure terminates the Worker and flags the binding; reconnect
constructs fresh epoch documents. Own plaintext crosses the decoder boundary only
after the same authenticated scope, writer, chain and signed-cut admission as content.
The [raw own fold](#implemented-raw-own-fold-1264) defines its bounded projection.
Historical revoked-device objects MUST predate revocation and remain within the
exact signed namespace cut; checkpoint replacements require the cut's exact
envelope hash, and catchup MUST reach every nonempty signed tail endpoint before
publishing a view. Named-member, link and bridge author policy remains outside
this owner-browser slice (#1111/#1160).

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

**Channel boundary: split.** Trusted browser Send and the encrypted own-stream
ledger belong to Colab; Remote owns operations, grants, hold approval, dedup and
receipt recovery. Local v1 uses the asking browser directly. There is no native
bridge ledger, native Ask route or additional SQLite migration.
The browser wraps the same verified Session held by registration and Live with
Remote's operations helper. The wrapper opens nothing and never reopens on
uncertainty: the helper resyncs its sequence and reads the original operation ID.
Only session end or unrecoverable sequence state signals Registration to reconnect.
Registration rebuilds both the RemoteClient and page AskControllers with the new
shared Session; pending previews close, and recovery reads the original IDs.

Only explicit Send in trusted parent chrome dispatches agent work. Comments,
sync, replay, compaction, reload and renderer messages MUST NOT dispatch. Ask
agent is separate from Comment. The parent freezes the admitted quote/comment,
page title, fragment-free HTTP(S) URL and chosen agent/machine before signing.
Credentialed URLs and malformed Unicode refuse. Paused rendering or later edits
cannot replace frozen text. The preview displays destination UUIDs, verified
presence, exact delivered UTF-8 and a separate escaped control-character view;
it warns that the ask and reply are visible to everyone with page access.

Remote prepends its contract-defined `[remote: <device name>]` line and LF. The
parent includes that line in the delivered preview, while signing and sending
only the frozen message below it. The verified device name and grant revision
and session expiry are pinned to the preview; rename, revision change or session
end refuses Send from the pending preview.
No second prefix or post-preview formatter is permitted. Delivery readiness is always unavailable in local v1; presence never becomes
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
original ID; absence remains uncertain. No same-ID retry or replacement operation
is offered in v1. Explicit abandon records `MAY_HAVE_BEEN_DELIVERED`; it neither
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
preview (agents.list and owner session echo). They are outside the signed input
and never establish authority, route work, or replace the agent/device UUID.
Readers may show a UUID fallback when a label is empty.

Binary fields use canonical base64url. State revisions are positive canonical
decimal strings, ordered numerically. `requestId` and `reason` are explicitly
null when absent. Reasons are bounded sanitized codes, never raw transport
errors. Verified pre-effect Remote refusals preserve `REMOTE_SCOPE_DENIED`,
`REMOTE_INPUT_INVALID`, `REMOTE_RATE_LIMITED`, `REMOTE_INTENT_CONFLICT`,
`REMOTE_CLOSED`, `REMOTE_SESSION_ENDED`, `REMOTE_INPUT_TOO_LARGE`,
`REMOTE_STATE_UNAVAILABLE` or `REMOTE_CORE_UNAVAILABLE`; unknown refusal codes become
`REMOTE_REFUSED`. A verified pre-admission Send refusal, including session end,
is definitive: it records refused with the reviewed code. A session-end Send
refusal then signals Registration to reconnect and requires a fresh preview.
Adopted Sends never return refused; unknown outcomes remain uncertain. A typed
SDK `sequence_unavailable` Send outcome becomes uncertain
(`REMOTE_SEQUENCE_UNAVAILABLE`) and signals Registration to reconnect. The adapter never reopens. A session-ending result
read leaves the existing accepted record unchanged and stops observation until
reconnect. Every refused operation/result read leaves the ledger unchanged;
missing operations do not prove absence. Transient state/core-unavailable refusals
continue bounded backoff. Read refusal copy is ephemeral, never an own-state
transition. A fresh preview is required for later Send. Unknown-effect errors
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
threads/comments UI is separate from the minimal Ask panel.

Only the asking device observes its unresolved operations and publishes finals,
through Remote's device-owned result operation and request IDs already in its
admitted ledger. Empty finals are valid. The page copy is exact UTF-8, bounded
to 16 KiB; a larger retained core final yields `REPLY_TOO_LARGE`, with no silently
truncated copy. Unavailable core history yields `RESULT_UNAVAILABLE`. Core remains
final/retention authority; page copies follow page access and retention.

Read-only observation resumes on load and after Send/re-check, uses 2-second to
30-second backoff while visible, pauses hidden pages and resumes when visible,
and stops after a two-hour operation horizon. After that, explicit re-check
remains available. Closing the asking browser delays page publication until it
reopens; it never cancels work. Synchronization and other viewers cannot perform
result reads as the asking Remote device.

The browser `RemoteClient` adapter consumes Remote's served operations SDK and
its verified session/responses. It does not compose raw envelopes, read Remote
storage or invoke core directly. Uncertainty recovers on the existing session;
only session end requires Live to replace it before original-ID recovery. Frozen byte/signature vectors live in
`vectors/send-preview-v1.json`, with independent Python regeneration.

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
message containing `type:"colab.render.bind"`, `renderId`, `sourceDigest` and
`source`, plus one MessagePort. The renderer accepts only these exact fields,
valid UUIDv4/digest metadata and source at most 2 MiB UTF-8, from
`event.source === window.parent`; origin is not an admission predicate for this
opaque channel. Direct top-level opening cannot initialize source. Invalid or
non-parent messages do not initialize it; after one valid init all later init
messages are ignored. No source enters a URL or request body. The bootstrap
replaces its own document using `document.open/write/close`, retaining the
response policy and prepending a trusted acknowledgement before author source.
Bootstrap load and initial source-document load are the only permitted loads;
further document loads/navigation tear down the frame.

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

| Type        | Additional fields / behavior                                                                                                                 |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello`     | `device, membershipRevision, cursors`; authenticated session/proof from upgrade, page/epoch admission before catch-up                        |
| `catchup`   | `membershipHead, baseline, optional baselineObject, streams, more`; bounded pages of each stream's namespace checkpoints and subsequent tail |
| `subscribe` | `cursors`; observe only the admitted page/current epoch                                                                                      |
| `append`    | `streamId, seq, envelopeHash, envelope`; create-only, exact frozen retry returns original receipt                                            |
| `receipt`   | `streamId, seq, envelopeHash`; durable acceptance, not task completion                                                                       |
| `broadcast` | `streamId, seq, envelopeHash, envelope`; subscriber must verify before applying                                                              |
| `ack`       | `cursors`; scoped delivery positions only                                                                                                    |
| `awareness` | `device, data`; bounded ephemeral presence, never persisted or authority                                                                     |
| `error`     | `code`; one of DENIED, EXPIRED, STALE_EPOCH, INVALID, GAP, CAPACITY, CONFLICT, RESYNC_REQUIRED                                               |

### Implemented stream subset (#1156, #1166)

The externally driven local sync module implements the strict operations below.
Registration is #1162; `serve` composes mounted socket sync in #1211. Remote owns upgrade admission and
supplies the authenticated principal. The module takes an already-upgraded
nonblocking duplex stream, not HTTP headers.

Every client message is one UTF-8 JSON object with exactly the common fields
`version:1, type, space, page, epoch` and the operation fields listed below.
Epoch is a positive canonical decimal string. Duplicate/unknown fields, nulls,
wrong types, noncanonical values and unsupported operations reject.

| Client type | Exact additional fields                       | Implemented behavior                                                                                                       |
| ----------- | --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `hello`     | `device, membershipRevision, cursors`         | Device matches principal; one successful hello per connection starts server-driven catchup, then live delivery.            |
| `subscribe` | `cursors`                                     | Empty list starts live delivery; nonempty list resolves cursors and starts catchup, then live delivery.                    |
| `append`    | `streamId, seq, envelopeHash, envelope`       | Inline update or object reference; verify complete exact bytes and durably append before receipt.                          |
| `chunk`     | `objectId, envelopeHash, index, count, bytes` | Complete the connection's pending referenced append; no standalone upload or partial append.                               |
| `ack`       | `cursors`                                     | Resolve retained scoped positions and release one frame credit; no deletion, core acknowledgment or application authority. |
| `awareness` | `device, data`                                | Device matches principal; at most 4 KiB canonical base64url bytes, ephemeral.                                              |

`cursors` has at most 256 strict objects `{streamId, namespace, seq, envelopeHash}`,
unique by stream/namespace. Sequence zero is an explicit bootstrap sentinel and
requires zero32 hash; an omitted namespace also bootstraps. A nonzero cursor must
match an exact retained update or checkpoint in that namespace. Unknown, wrong-
namespace, hash-substituted and pruned cursors return `RESYNC_REQUIRED`; a retained
receipt alone is insufficient after its payload is pruned. A client restarts with
zero/omitted cursors to receive the latest paired prefix checkpoint and retained tail.
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

### Implemented catchup and chunk protocol

Owner discovery on the mounted socket uses read-only `GET /api/session` and
`GET /api/pages`. Missing or non-owner context returns 403 JSON `{code:"DENIED"}`.
Session returns exactly `{deviceId, publicKey, grantRevision, name}`; revision
is decimal text. It works before Colab extension-key registration and forwards
no credential. Pages returns exactly `{spaceId, ownerKey, revision, pages}`;
`pages` is sorted by page ID, at most 1,000 entries, each exactly
`{pageId, epoch, sharing, history, archived}`. Local creation/epoch records own
existence; sharing/history/archive/delete derive from verified owner statements.
Absent sharing/history mean private/shared. Deleted pages are excluded. Titles
are encrypted content and never returned here. `revision:"0"` means no owner log
has been initialized yet. State faults return 503 JSON `{code:"UNAVAILABLE"}`.

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
days of expiry. Readers treat expired-but-present data as gone. Warnings begin
seven days ahead in the browser and `ls/show`. Local data is never automatically
deleted.

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
The root-only `page.create` selection is exactly `{pageId,title,source}`; it accepts
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
```

All commands support human output and one `--json` document. Top-level `ls`
and `share link ls` have hidden `list` aliases, following the shared CLI style.
Audience widening and link addition/Reset MUST require explicit `--yes`; absent
confirmation sends and writes nothing. A seed is canonical base64url seed32 from an
owned regular 0600 file or bounded stdin (`--seed-file`, for scripts and exact retries),
never argv. Without `--seed-file`, `link add` and `link reset` generate a fresh seed from
the OS RNG. The seed is persisted nowhere and appears in output only inside the
[reader link](#read-only-reader-link-1545) fragment (`readerPath`) that a successful add or
reset prints once; `ls`, `remove` and every other output never carry it. After an uncertain
outcome, retry with the same `--link-id` and `--seed-file`; a generated seed is not
recoverable, so the fallback is Reset.
Links created or reset by this v1 CLI always have the viewer role. Removal/Reset
capture complete verified assignments. Member, history, retention, archive and
delete commands are deferred beyond v1.

Mutations accept `--operation-id` and `--expected-revision`; generated/default
values are captured once. Success is `{operationId, expectedRevision,
membershipHead:{revision, statementHash}}`; link creation/Reset also returns the
nonsecret replacement `linkId`. An explicit retry MUST retain the same IDs,
revision, selections and caller-held seed. Unknown IPC outcomes MUST retain this
nonsecret correlation, never regenerate a request or claim no effect.

Link listings return `{membershipHead, links}`.
Page lists return `{spaceId, membershipHead, pages}`; an uninitialized list has null
space/head and empty pages. Show returns `{spaceId, membershipHead, page, members,
links, discussions:"not-available"}`. Page fields are `pageId, title, epoch,
sharing, history, archived, retentionDays, lastUpdateAtMs, expiresAtMs, warnings`.
IDs, epochs and revisions retain canonical full values; times would be UTC
milliseconds. Policy derives from verified owner statements; titles require the
authenticated fold and isolated decoder. Archived titles are null with
`title-unavailable` because the fold refuses archived pages. Members/links expose
ID, role, page assignments and revocation, never secret material.

Until durable last-update evidence is available, both timestamp fields are null
and warnings contain `expiry-unavailable`; human output states this plainly.
No file time, read time or fabricated zero establishes expiry. Retention defaults
to 30 days; forever is null. Local policy never automatically deletes or denies
access. Discussion summaries await verified own folding (#1264/#1110).

Failures exit 1. JSON is `{error:{code,message}}` with operation/revision/link
correlation when captured; human failures use styled stderr. Management codes
map explicitly to `COLAB_INVALID`, `COLAB_DENIED`, `COLAB_EXPIRED`,
`COLAB_CONFLICT`, `COLAB_STALE_HEAD`, `COLAB_CAPACITY`, `COLAB_UNAVAILABLE`.
CLI failures add `COLAB_INPUT_INVALID`, `COLAB_CONFIRMATION_REQUIRED`,
`COLAB_PAGE_NOT_FOUND`, `COLAB_OUTCOME_UNKNOWN`; existing state/schema failures
keep their codes. Success exits 0. Neither acknowledgments nor unsigned output
create browser authority; browser refresh still uses verified catchup.

## Root-local page creation

`tmt colab page create --title <title> [--file <path|->] [--json]` creates a private
page at epoch 1. The title is nonempty and bounded to 256 KiB UTF-8. Omitted
`--file` means empty source; a file or stdin uses the page-write reader (2 MiB
UTF-8, five-second stdin EOF deadline). The initial content update is at most
256 KiB and the isolated decoder keeps its existing 4 MiB composition limit.
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
browser router's existing fragment format; no Remote host discovery is added.
Human output identifies the page and says to open the path under the Remote door
address printed by `tmt remote pair`. JSON failures use `{error:{code,message}}`,
with generated `operationId` and `pageId` when available for inspection after an
unknown outcome. Existing input/capacity/denial/conflict/stale-head/state/unknown
codes apply; failures exit 1. The browser home empty state names this command.

## Root-local page source CLI (#1438)

`tmt colab page read <page> [--json]` captures existing state through the
owner-authenticated fold and isolated decoder. Raw stdout is exact admitted UTF-8
source, without an added newline; stderr carries the verified head, epoch and
page revision. JSON is `{spaceId,pageId,source,title,epoch,membershipHead,
revision,memoryLimit}`. `membershipHead` is `{revision,statementHash}` with decimal
revision and lowercase hex hash. Reads never initialize or migrate state.
Archive/delete reads retain the fold's current inactive-page restriction.

`tmt colab page write <page> --file <path|-> [--expected-revision <token>] [--json]`
retains the title and prepares a minimal text delta in the isolated child against
admitted Yjs structs. It does not recreate the shared document, execute HTML,
advance membership or restore discussion/authority state. Source is bounded to
2 MiB, updates to 256 KiB, and composed decoder input/output to 4 MiB; existing
fold/count/deadline/cleanup limits still apply. Stdin has a five-second EOF deadline.
Invalid UTF-8, capacity and inactive-page writes reject without mutation.

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
lock. Holding it selects the offline existing-state writer; a held lock selects the
running serve through the existing owned 0600 Unix socket. One device transaction
checks the pinned chain,
base/head/epoch/positions, then commits the device chain, signed encrypted content
append and exact operation receipt together. Existing create-only append/quota/
conflict semantics are reused. Exact frozen retries return the original receipt,
without re-signing or overwriting later content. Changed bytes conflict. JSON
success is `{spaceId,pageId,epoch,membershipHead,revision,streamId,seq,envelopeHash,
sourceSha256,memoryLimit}`; hashes use lowercase hex except the model envelope hash,
which is canonical base64url.

Serving writes use POST `/.tmt/colab/local/page-write`. Its strict prepared DTO is
`{version,operationId,spaceId,pageId,epoch,membershipHead,baseRevision,sourceSha256,
memoryLimit,chain,envelope}`; version is 1, operationId is a frozen UUIDv4, and
chain/envelope are canonical base64url. The request contains no plaintext source,
private key or epoch key. Forwarded device-context or event headers are DENIED;
wrong methods/upgrades and unknown/duplicate fields reject. The reserved router
shares management's local-header denial. One body-cap rule gives this route
512 KiB and retains 64 KiB for other HTTP routes. Acquisition, frame, queue and
response bounds remain in force; the client bounds its response and absolute read
deadline. An IPC failure or uncertain response never falls back to an offline
writer, changes the operation identity or automatically resends.

Serving locks sync before Registration and prepares bounded transport before the
transaction. A new committed append queues a `broadcast` with the normal scoped
position/envelope fields and `chains:[{deviceId,chain}]`; the chain identifies the
local author and is repeated to permit certificate renewal. The browser admits
chains on its serialized executor before envelope authentication and Worker
application. Larger envelopes use the existing reference and lazy chunk transfer,
with the chain retained on the completed broadcast. Exact replay returns the
original receipt without another broadcast. Slow or revoked peers use existing
resync/admission failure behavior; queue failure does not undo a durable receipt.
The service never receives or decodes plaintext source.

## Plaintext page export (#1309)

Export v1 emits exactly `page.html` and `manifest.json`. Native captures one
owner-authenticated read snapshot through its isolated decoder; the mounted
browser captures one admitted committed projection through its connection executor.
HTML is the exact admitted UTF-8 source, including CR/LF, Unicode and NUL; export MUST NOT
inject renderer CSP/bootstrap, normalize source or execute it. The title, epoch
and verified membership head MUST belong to that same snapshot. A later write
cannot change an already captured bundle. This head is locally verified, not a
claim of globally current membership.

The manifest is UTF-8 JSON with these fields:

| Field            | Value / meaning                                                            |
| ---------------- | -------------------------------------------------------------------------- |
| `format`         | `tmt-colab-page-export`                                                    |
| `version`        | JSON integer `1`                                                           |
| `spaceId`        | Pinned space ID                                                            |
| `pageId`         | Exported page UUID                                                         |
| `title`          | Exact admitted title                                                       |
| `exportedAtMs`   | Safe-integer UTC milliseconds                                              |
| `membershipHead` | `{revision, statementHash}`; decimal revision, lowercase hex SHA-256       |
| `epoch`          | Current snapshot epoch as canonical positive decimal text                  |
| `plaintext`      | `true`                                                                     |
| `discussions`    | `not-included`                                                             |
| `files`          | `[{name:"page.html", sizeBytes, sha256}]`; bytes and lowercase hex SHA-256 |

The manifest MUST NOT list its own digest: that would be circular. The CLI result
lists both files with their byte sizes and SHA-256. Export contains no roots,
wraps, bearer seeds, sessions or private keys. It is a readable copy, not an
import/backup format; referenced assets, retained history and snapshots are not
promised as portable files. Discussion export is deferred until own-namespace
folding exists (#1110 / #1264 slice C); native fold validation of own updates
still applies, but their projections are not included.

`tmt colab export <page> [--dir <destination>] [--json]` reads existing local
state only. It MUST NOT initialize missing state or run migrations. The
current native fold refuses inactive pages, so archived and deleted pages fail
with `COLAB_EXPORT_INACTIVE` and the explanation "archived or deleted pages
cannot be exported yet". After the archive/delete read-policy split (#1348),
archived exports are enabled separately; deleted pages remain denied.

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
A returned success means both files were published and staging was removed;
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
`page.html` and `manifest.json`, never source/title paths. Revoke each URL after
bounded download handoff and all outstanding URLs on close/navigation or blocked
binding cleanup. The renderer receives no export handler, URL or capability.
Archived browser export remains deferred until #1348; deletion stays denied.

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
