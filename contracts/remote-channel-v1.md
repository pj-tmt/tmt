# Remote client protocol v1

**Status: proposed, not implemented.** This document owns remote wire values, authority and
transport bindings. tmt-lead reviews it before runtime/SDK code. The
[security design](https://github.com/wkh237/tmt/issues/478#issuecomment-5910827518),
[M1 ruling](https://github.com/wkh237/tmt/issues/478#issuecomment-5911118171) and
[transport-layer decision](https://github.com/wkh237/tmt/issues/478#issuecomment-5911165578) govern
this proposal. M1 specifies `local-v1` over `loopback-http` only. Every send is held for the owner's
local approval. Relay deployment, direct delivery, PAKE/Noise and official packaging are deferred.

## Owners and layers

The message layer owns signed envelopes, correlation, durable append/subscribe/ack semantics,
sessions, replay, grants, approval, uncertainty and audit. A binding moves those messages and
rejects unauthenticated traffic at its edge; HTTP status, a document ID or a WebSocket connection
never grants authority. Changing bindings must not change the signature, scope or approval rules.

All listening and remote state belong to `extensions/tmt-remote`. One extension-owned transport
trait exposes `append`, `subscribe` and `ack`; it does not execute core work or decide authority.
The same message service admits every binding before effects. Core never listens or stays resident.
Remote uses `$TMT_EXECUTABLE api` and documented ordinary JSON commands; it never opens core SQLite,
imports core behavior crates, calls host adapters or scrapes panes. The
[extension API](extension-api.md) owns core JSON resources, errors, limits, retention and
durable request/dispatch semantics.

M1 protects against unpaired clients, malicious pages/other origins reaching loopback, replay, scope
expansion and accidental duplicate sends. Loopback, CORS and a random route are not credentials.
Same-user malware, a compromised browser/add-on, malicious selected executable and compromised OS
account are outside this profile. Non-extractability restricts key export; it is not hardware
isolation or protection from code that can invoke the key. `local-v1` does not encrypt content or
provide relay confidentiality.

## Bytes, IDs and the fixed M1 suite

Text is exact UTF-8, without normalization; reject unpaired Unicode surrogates in decoded strings
rather than replacing characters. `LP(x)` means four-byte unsigned big-endian byte length followed
by x. Lists are a four-byte count followed by their LP elements, in order. Integers in canonical
bytes use decimal ASCII, without sign/leading zeros; zero is `0`. Every specified concatenation ends
at its last field, with no separator or trailing newline. List order is bytewise ascending for
fields specified as sorted. Fingerprint bit indexes are most-significant-bit first.

JSON is UTF-8 without BOM, duplicate/unknown request members or non-finite numbers. JSON integer
fields are at most 2^53-1; sequences are decimal strings bounded by 2^64-1. Binary fields are
unpadded RFC 4648 base64url; reject invalid alphabet, padding, nonzero unused bits and incorrect
decoded lengths. Remote-generated UUIDs are canonical lowercase UUIDv4; referenced core identity
UUIDs are lowercase canonical hyphenated, non-nil UUIDs of any version or variant,
as the core public API accepts. Enrollment `agentIds` are such core references.
Syntax validation does not establish identity existence or grant authority. Core request IDs
retain their `req_...` form. Clients tolerate additive response fields; incompatible required fields or semantics need another protocol major. Unknown
profiles/bindings fail closed.

The accepted [M1 crypto spike](https://github.com/wkh237/tmt/issues/597#issuecomment-5911199408)
selects the following fixed profile; there is no algorithm negotiation:

- **Ed25519:** ordinary signatures over canonical bytes, not Ed25519ph or a second whole-message
  prehash. Raw public keys are 32 bytes, signatures 64. Browser/SDK uses native WebCrypto; Chrome
  137 is the minimum. Probe actual key generation/sign/verify before pairing and fail clearly if
  unavailable.
- **Rust verification:** `ed25519-dalek = "=3.0.0"`, `VerifyingKey::from_bytes`, reject weak keys at
  enrollment, and `verify_strict` on every message. No `legacy_compatibility`, `hazmat` or batch
  verification. The spike records BSD-3-Clause/MSRV 1.85 and no independent audit covering this
  exact graph.
- **Hash/MAC:** SHA-256 and full 32-byte HMAC-SHA256. Native pins are `hmac = "=0.13.0"` and
  `sha2 = "=0.11.0"`, using constant-time `verify_slice`. WebCrypto uses HMAC with `hash:"SHA-256"`.
  No truncated tags or custom crypto. The spike records MIT/Apache-2.0/MSRV 1.85, not an
  exact-version audit.

Implementation must verify lockfile/platform/MSRV/advisory evidence. M1 has no WASM or third-party
JS signing library. Node conformance targets Node 24; browser persistence evidence still requires
actual MV3/IndexedDB tests.

The add-on generates `crypto.subtle.generateKey({name:"Ed25519"}, false, ["sign","verify"])`. Store
the private CryptoKey itself by IndexedDB structured clone in the extension origin; export only the
public key. Never JSON-serialize, sync, export or transfer private material to page/content scripts.
Machine signing keys use an OS-backed store or an owner-only 0600 key file under remote's own
subtree; no non-extractability/hardware claim is made for a software file. Use OS CSPRNG/WebCrypto
entropy, never a clock, UUID string or `Math.random` as key/code entropy. Fixed seeds are test-only.

## Signed envelopes

Every paired message, including reads and controls, has this envelope:

```json
{
  "version": 1,
  "profile": "local-v1",
  "kind": "request",
  "id": "00000000-0000-4000-8000-000000000001",
  "correlationId": null,
  "machineId": "00000000-0000-4000-8000-000000000002",
  "windowId": "00000000-0000-4000-8000-000000000003",
  "clientId": "00000000-0000-4000-8000-000000000004",
  "sessionId": "00000000-0000-4000-8000-000000000005",
  "sequence": "1",
  "timestampMs": 1790770000000,
  "origin": "chrome-extension://example",
  "operation": "dispatch.create",
  "payload": "e30",
  "signature": "<base64url raw signature>"
}
```

Examples are illustrative, not valid credentials/vectors. `payload` is exact JSON bytes encoded as
base64url (`e30` is `{}`). Do not parse/reserialize the payload before hashing it. Transport JSON
formatting is not signed content. The decoded payload must pass its operation's strict admission
before effects.

Canonical signature input is the LP concatenation of: ASCII `tmt-message-v1`, decimal version,
profile, kind, id, correlationId (empty bytes when null), machineId, windowId, clientId, sessionId,
sequence, decimal timestampMs, origin, operation, and the **raw 32-byte SHA-256 of decoded
payload**. These fields bind logical operation, payload, audience and origin independently of
physical URL/method/document path. Unknown envelope fields are rejected. Moving the same envelope to
a different operation/path cannot reinterpret it; binding operation and signed operation must agree.

Kinds are `request`, `response` and `control`. Requests have null correlationId; responses correlate
to exactly one request/control ID. A response has its own UUID and the same client/machine audience;
the machine signs it with its pinned key. Response operation matches the correlated request/control
operation; its origin echoes the pinned client origin. Controls are `session.open`, `subscribe`, or
`ack`, with null correlationId. Application errors and held/accepted/final states are signed
response payloads, not HTTP states. Replies from agents remain core-owned; remote publishes their
correlated availability/results, never fabricates an agent signature.

For a committed key, `session.open` is a signed control using `sessionId:"new"`, `sequence:"0"` and
payload `{clientNonce}` (random 128-bit lowercase hex). Its timestamp must be within 60 seconds of
machine time. Reusing that nonce within timestamp validity is refused. Return a machine-signed
response whose sessionId is the fresh session UUID and whose payload is
`{sessionId,serverTimeMs,grantRevision,expiresAtMs}`. The effective expiry is the earliest grant,
window or idle deadline. The client verifies the paired machine key before trusting it. One session
per client; creating another invalidates the previous one without widening scope.

Normal messages also require a timestamp within 60 seconds of machine time. Normal client sequences
start at 1 and increase by exactly one. One request is in flight per session. Under remote's
authority lock, verify signature, audience, origin, live grant/window, timestamp, scope and expected
sequence; consume the sequence durably before any effect. Concurrent duplicates have one winner.
Invalid signatures do not advance it. A consumed sequence stays consumed even if downstream work
fails. Stale, replayed or reordered messages cause no effect. Machine responses have an independent
increasing session sequence starting at 1; clients reject non-increasing live response sequences.
The SDK serializes controls and requests; use waitMs:0 when interactive work is queued, so a
long-poll does not race a send. A lost response requires session recovery, not a guessed sequence. A
retried logical request gets a fresh response envelope around its original receipt payload,
correlated to the retry ID in the current session. The log cursor governs historical ordering, not a
reused live-session counter. On reconnect, old signed log entries are accepted only as historical
data for the subscribed audience, never as a fresh command; the new signed subscribe response binds
their ordered IDs/digests and cursor to the current control ID/session.

Lost sequence/session state requires a new signed session and ID-based receipt recovery. It never
permits a captured-envelope replay or automatic new send. Expiry/revoke/close is checked again at
the effect fence. Bindings cannot waive those checks because an edge previously accepted a
signature.

## Durable log: append, subscribe and ack

Each paired client sees one machine-owned ordered stream of its admitted-request receipts and
correlated state notifications. Another client's IDs/cursors disclose nothing. The log is a remote
delivery journal over core resources, not a second conversation database or core attention queue.
Remote retains bounded held/uncertain payloads and immutable operation/request references; core
alone owns conversation history and retained finals. Final notifications can reference a core
request; `result` retrieves its current retained body through the public API. No permanent
final-body copy or new retention lease is created in remote.

`append(requestEnvelope)` authenticates/adopts the logical request and stores its ID, client
ownership and frozen payload/digest atomically before returning acceptance. For writes that need
approval this appends a held record and signed `held` response only; it does not call core. Receipts
distinguish journal acceptance, core acceptance, wake outcome and agent final. Failure before
durable adoption is a refusal, not a silent queue. Failure after adoption is recovered by ID; a
missing transport response is not proof that append failed.

Idempotency compares `(clientId,id)` and logical intent (operation and exact payload bytes). An
explicit retry uses a fresh signed envelope/sequence with the **same request ID and intent**.
Signature/session/time may change; logical intent may not. It returns the original receipt without a
new log entry or core effect. A cross-client collision or changed intent refuses. For
dispatch.create, envelope id equals input.operationId, the UI-frozen core operation UUID; the SDK
must not allocate a replacement ID. Other requests get fresh IDs. SDK persists the dispatch ID,
intent and exact serialized payload bytes before append. Identical request IDs do not guarantee
exactly-once agent processing, only the bounded journal/core dispatch behavior.

`subscribe(controlEnvelope)` takes payload `{cursor,limit,waitMs}`. Cursor is null initially or a
server-issued opaque token, scoped to client, machine and stream incarnation; never parse, order,
increment or transfer it. Limit is 1–50; waitMs is 0–25000. Catch up from the cursor, then wait
until one new entry or the bounded deadline. Return a signed response with ordered entries,
`nextCursor`, `reason:"changed"|"timeout"` and `hasMore:boolean`. Entries are `{cursor,envelope}`;
cursor is the position after that entry, and envelope is a machine-signed response/notification
correlated to the adopted request. Subscribers receive metadata and state, not duplicate client
payloads. The signed batch payload binds their exact bytes/order. Set nextCursor to the last
returned entry cursor, or to the input cursor on timeout; initially an empty stream returns its
beginning cursor. Live read responses are not copied into the journal; append only a signed metadata
notification `{requestEnvelopeId,operation,state:"observed"}` correlated to that read ID, and
deliver the full signed read result directly. Response entries contain state/receipt references
rather than full final bodies. Release frozen payloads after confirmed core acceptance/cancellation;
keep only intent digests, ownership and immutable core references, without duplicate permanent
prompt history. After the initial beginning cursor is issued, a timeout has empty entries and an
unchanged nextCursor. Cursor expiry returns `REMOTE_CURSOR_EXPIRED`; recover with own operation
IDs/fresh snapshot, never resend work. Sessions change without deleting durable stream state. Core
`changes.cursor` is not exposed as this cursor.

`ack(controlEnvelope)` takes `{cursor}` and acknowledges only the successfully observed
client-stream prefix. It is monotonic/idempotent and cannot acknowledge another client's cursor. It
is a delivery checkpoint, not core X acknowledgment, task success, cancellation, deletion or
retention renewal. Controls and their responses do not create entries requiring another ack,
avoiding ack loops. No implicit acknowledgment on subscribe/read. Enforce ack at or before the last
successfully subscribed position; a client cannot skip unseen entries. Bound retained journal
entries to 24 hours and 1000 entries/client; acked prefixes may be compacted earlier; refuse new
adoption if unacknowledged capacity is exhausted. Expired journal metadata can require fresh
own-state recovery; it does not alter core prompt/final retention. Separate bounded
operation/request ownership records survive journal eviction until grant expiry plus 24 hours. Limit
these to 1000 operations/client and refuse new adoption at capacity; never evict an uncertain
operation to admit another. After this recovery horizon an owned read returns
REMOTE_STATE_UNAVAILABLE, never permission to resend. Revoked/expired grants cannot use these
records to regain access.

## Pairing profile: `local-v1`

Only local `tmt remote pair` opens an offer in an explicitly running/open server. A client cannot
initiate or extend pairing. One offer at a time; replacing it explicitly cancels the previous offer.
The terminal supplies a descriptor with profile/binding, machine/window/offer UUIDs, address and
random 128-bit server challenge (32 lowercase hex characters). No code/private key is included in a
URL, ordinary web-page DOM or log.

The code is **16 random bytes**, displayed as 26 uppercase RFC 4648 base32 characters grouped for
**copy/paste**, one use, ten-minute expiry. Decode after removing ASCII spaces/hyphens only,
rejecting invalid alphabet/nonzero unused bits. It is not a short numeric/word password: an observed
known-message HMAC allows offline guesses, and three attempts/expiry do not prevent that attack. The
high entropy is mandatory; do not describe this scheme as PAKE.

The client proposes its native non-extractable Ed25519 key, kind (`addon` or `cli`), exact origin,
proposed name, requested agent UUIDs, and a random 128-bit clientNonce. Names are 1–64 nonblank
UTF-8 bytes without controls; origin is at most 128 bytes. Agent UUIDs are distinct, sorted, at
most 256. Requested scope list is sorted, distinct and drawn from this profile; requested mode is
always `hold`.

Enrollment bytes are LP(`tmt-local-pair-v1`), LP(profile), LP(machineId), LP(windowId), LP(offerId),
LP(decoded serverChallenge), LP(decoded clientNonce), LP(kind), LP(origin), LP(name), LP(raw public
key), the framed agent UUID list, the framed scope list, and LP(`hold`). Submit those fields plus
full base64url HMAC-SHA256(codeBytes,enrollmentBytes), and Ed25519 signature over
LP(`tmt-local-pair-possession-v1`) || LP(enrollmentBytes) || LP(raw HMAC). Machine checks proof in
constant time, strict key/possession signature and live offer/challenge before pinning the
candidate. The pair request may wait for local confirmation only until the offer deadline; identical
pending candidates coalesce, while a competing candidate cannot replace the pinned one. Enrollment
is the only unpaired message form; it authorizes no core work.

Both sides show four words derived from SHA-256(LP(`tmt-local-key-fingerprint-v1`) || LP(raw public
key)): take the first 44 bits as four successive unsigned 11-bit indexes into the fixed English
BIP-39 2048-word list, in list order. Use the
[BIP-39 English list](https://github.com/bitcoin/bips/blob/master/bip-0039/english.txt); pin its
revision, bytes/digest and examples in the fixture slice. This is comparison text, not a recovery
mnemonic. Terminal shows full origin/key fingerprint; owner confirms the same words, final name,
permitted UUIDs/scopes and expiry. The owner may reduce requested authority, never silently enlarge
it.

Enrollment JSON names are `profile`, `machineId`, `windowId`, `offerId`, `serverChallenge`,
`clientNonce`, `kind`, `origin`, `name`, `publicKey`, `agentIds`, `scopes`, `mode`, `mac` and
`signature`; binary keys/proofs use base64url, challenge/clientNonce use 32 lowercase hex
characters. Reject unknown fields.

After approval, atomically create one grant and consume the offer. Receipt JSON is
`{grant,machinePublicKey}`; grant contains the fields defined below, with `agentIds`, `scopes`,
`issuedAtMs`, `expiresAtMs`, positive integer `revision` and `disabled:false`. The terminal-selected
name and narrowed authority are included. The response carries base64url exact `receipt` bytes and
`serverProof`. Define
`K_response = HMAC-SHA256(codeBytes, LP("tmt-local-pair-response-key-v1") || LP(enrollmentBytes))`.
`serverProof` is the full HMAC-SHA256(K_response, LP("tmt-local-pair-response-v1") || LP(exact
receipt JSON bytes)). Client derives this key and verifies the proof before pinning the machine key
or accepting the grant; verify matching machine/profile/client key/kind/origin, hold mode, valid
expiry and that granted agents/scopes are subsets of those requested. Exact candidate retry is
idempotent until the original offer deadline; preserve only K_response, the candidate digest/proofs
and exact receipt for that bounded lost-response recovery. Verify matching candidate and possession
on retry; never accept K_response as a reusable enrollment token. Raw code is erased after
confirmation; no grant exists before local confirmation. Delete recovery key at the original
deadline. Three failed code proofs, expiry, owner refusal/no confirmation, close or process exit
cancels pending pairing and erases its secret. No client grant or client private key is stored for a
failed/cancelled offer; sanitized audit may record failure only.

M1 add-on origin is its exact installed `chrome-extension://<id>` origin. Reject
null/opaque/wildcard/ordinary web origins; a CLI uses the literal `cli`. Origin is bound in
enrollment and every envelope. Signing never occurs in page scripts or content scripts. Lost key,
changed add-on origin or revoked/expired grant requires new local pairing. No P-256 downgrade,
export-based backup, automatic key replacement, passkey renewal or remote grant extension in M1.

## Grants, windows, scope and operation mapping

Grant fields are client/machine IDs, profile, pinned key, kind/origin, owner name, allowed agent
UUIDs, scope set, mode, issued/expiry timestamps, revision and disabled state. The grant JSON names
are `clientId`, `machineId`, `profile`, `publicKey`, `kind`, `origin`, `name`, `agentIds`, `scopes`,
`mode`, `issuedAtMs`, `expiresAtMs`, `revision` and `disabled`. Default expiry is 30 days; no M1
grant may exceed it. Names are presentation only; rename preserves UUID authority, and a retired
identity's same-name replacement inherits nothing. Authority may only be narrowed through local
management.

`serve` opens a foreground window for one hour by default, at most 24 hours, with a 15-minute idle
deadline. Authenticated authorized activity may reset idle, never the hard deadline; failures/scans
cannot. No `--always`/background service install. `close` disables the window before acknowledgment,
cancels pending pairing/approvals and rotates window/route. Revoke similarly disables a client
before acknowledgment. No request/effect not yet fenced may succeed afterward. Already committed
core work is not undone; report it accurately. Restart creates a new window/session namespace.
Grants survive if live; local approval does not survive. Unconfirmed held work is cancelled;
dispatching/uncertain work recovers its original operation, never becomes falsely unsent.

Default scopes are `agents.read`, `status.read`, `talk.hold` and `results.own`. Payloads for core
API operations are the existing API envelope, decoded without rewriting input. Remote narrows
supported operations/authority before core calls; future core capabilities do not silently become
remotely callable.

| Logical operation                                                                 | M1 authority / public core mapping                                                                                                                 |
| --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capabilities`                                                                    | Signed paired discovery of supported subset, fixed suite and core bounds.                                                                          |
| `agents.list`                                                                     | `agents.read`; `tmt ls --json` projected to allowed UUID/name/presence only, no pane/address/cwd/process/profile leakage.                        |
| `identities.status`                                                               | `status.read`; restrict input to permitted UUIDs. Self-report is not readiness or completion.                                                      |
| `dispatch.create`                                                                 | `talk.hold`; one permitted direct request recipient, anonymous originator, frozen intent held until local approval; no fan-out/announcement in M1. |
| `dispatch.show`, `operation.show`                                                 | `results.own`; only journal-owned operation IDs; core immutable receipt or remote held state.                                                      |
| `requests.show`, `result`                                                         | `results.own`; only request IDs from this client's core acceptance; public API detail or `tmt result --json`.                                      |
| `requests.list`, global `changes.cursor`, `references.resolve`, `rooms.roster`    | Unsupported in M1; no unrestricted history/activity/reference disclosure. Later projections need explicit scoped admission.                        |
| `notes.read`, `rooms.write`, `rooms.retire`                                       | Off/unsupported in pilot; later explicit resource/operation grants.                                                                                |
| `identityHooks.*`, `skills.install`, `skills.remove`                              | Never remotely callable in M1; JSON consent cannot manufacture local lifecycle/install authority.                                                  |
| check, reply/answer, X acknowledgment, config, pair, run, approvals, installation | Never remotely callable in M1. Result/log ack does not reply or acknowledge core work.                                                             |
| `x.<ext>.<op>`                                                                    | Naming reserved; no declaration/dispatch implementation or implicit consent.                                                                       |

`agents.list`, `operation.show` and `result` are adapter helpers, not new core API operations.
Ordinary public JSON commands complement `tmt api` as described in extension-api. SDK
`api(op,input)` cannot reach local management/argv through an invented operation. Reject
caller-selected `identity`; the pilot sends `originator:"anonymous"`, as a local browser owner,
never as a claimed agent. Provenance is reviewed `[browser]` text and authenticated journal
ownership.

M1 helper payloads are `agents.list:{}`, `operation.show:{operationId}` and `result:{requestId}`.
Their outputs are the SDK DTOs below. `dispatch.create` payload is exactly
`{version:1,operation:"dispatch.create",originator:"anonymous", input:{operationId,recipientIds:[agentId],message,kind:"request"}}`;
no room, identity or extra recipients. Signed envelope operation and core operation must agree, and
envelope id must equal input.operationId. Other supported core reads use their documented
`{version,operation,input}` envelope. Discovery returns
`{version:1,profile:"local-v1",binding:"loopback-http",operations,limits}` with only the admitted
operation subset and effective byte/rate bounds.

The wire-independent shell boundary is `@tmt/remote-client`. `RemoteClient` has
`listAgents():Promise<{id,name}[]>`, `send({operationId,agentId,message})`, read-only
`operation(operationId)`, and `result(requestId)`. Send/operation use SendState; result uses
ResultState as defined below. No selection/URL/title/note fields are reformatted by the SDK: they
are already inside the frozen message. `ClientError` is `{code,message,retryAfterMs?}`, with a
sanitized message at most 256 UTF-8 bytes and retryAfterMs an integer from 0 to 60000, with code
`unpaired`, `closed`, `scope_denied`, `rate_limited`, `input_invalid` or `unavailable`. An unknown
failure after send may have begun becomes uncertain, preserving the same operationId/message. No
implicit send from recovery or automatic retry.

## Approval, response correlation and uncertainty

A new talk freezes one remote request/core operation UUID, permitted agent UUID and exact final
message before append. SDK persists them first. The remote journal stores ownership, exact core
envelope/hash and `held` state before returning `{state:"held",operationId}`. **There is no core
requestId yet.** Approval is only local `tmt remote approve <operationId>`: show frozen bytes,
source/current name/UUID, require explicit confirmation, recheck grant/window/recipient, and durably
mark `dispatching` before the exact core call. No client can approve itself. No edit under an
existing operation ID. Refusal/cancellation is a correlated operation response, not an agent final.

Owner approval is the owner's send action under core's delivery protections; M1 does not infer
typing/busy/readiness. Direct mode stays disabled pending
[the later readiness contract](https://github.com/wkh237/tmt/issues/600). Call existing
`dispatch.create`; core owns idempotency, acceptance and the one-shot advisory wake. Append its
accepted response with request IDs under the original correlationId. Agent completion is read
through core and published under that same correlation as metadata
`{operationId,requestId,resultState:"pending"|"replied"|"unavailable"}` without a message body;
the explicit result read supplies the body. No terminal-output completion fallback.

Core/transport timeout, process crash, lost reply or uncertain wake retains the same frozen
intent/operation ID and reports `uncertain`, not a new send. Recover through authorized
`dispatch.show`. A definitive `DISPATCH_NOT_FOUND` after confirming the owned child stopped allows
retry of the **same** approved intent/ID after authority revalidation; never retry a claimed wake or
replace an uncertain operation with a new ID. Log adoption/replay likewise returns its original
acceptance. Core-final expiry is `unavailable`, not failed processing or permission to resend. Reads
never renew retention or acknowledge attention.

Signed response payloads discriminate `{state:"held",operationId}`,
`{state:"accepted",operationId,requestId}`, `{state:"uncertain",operationId, requestId?}`, and
`{state:"refused"|"cancelled",operationId,reason?}`. `result` returns `{state:"pending",requestId}`,
`{state:"replied",requestId, message}` (including an empty final), or
`{state:"unavailable",requestId,reason?}`. Pending/not-retained/expired distinction follows the
public core observation; never assert why a body is unavailable without evidence. Optional reason is
a sanitized string of at most 256 UTF-8 bytes; never expose raw process output.

## Browser use and audit

Shell owns selection-only capture, recipient selection, final message formatting and exact preview.
It shows URL/title/note/source marking and all characters, with a separate escaped view for hidden
controls; SDK signs those same frozen bytes without adding text after preview. Agent/content changes
explicitly create new intent. Render replies as inert plain text, not HTML. Core preserves stored
message and applies its public size/`!` transport protection; explain that adaptation, do not
silently rewrite the reviewed message. Refuse credentialed URLs rather than secretly dropping fields
from the preview.

Signing is allowed only from add-on UI after an explicit gesture. That action may authorize bounded
own-state/reply observation while its UI remains active; closing observation never cancels recipient
work. No externally_connectable, page-message signing, external message handler, remote scripts,
content-script credentials or broad page scraping. Shell permissions are activeTab, scripting and
contextMenus. Popup capture and browser contextMenus.onClicked are the only entry points. Background
hands transient capture and frozen intent to the popup through extension-owned IndexedDB; page
messages cannot initiate that handoff. Exact loopback host permission is added for real SDK
integration. Worker restart loads CryptoKey and frozen IDs from IndexedDB, creates a new session and
recovers operation state before any explicit retry. Never automatically resend.

Remote keeps files only in its own subtree of the data root reported by `tmt api` operation
`storage.root`, with owner-only
directories, 0600 secret/state files, no-follow bounded regular-file admission and durable atomic
state replacement. Never rewrite core DB/config or provider settings. Local append-only audit
records time, client/request/operation IDs, resource UUIDs, digest, grant/window revision, decision
and sanitized code before effects and outcome afterward. Never log code, MAC, signature, private
key, random route, message/reply, URL/title or reply receipts. It is not tamper-proof against the OS
user or secure erasure. Audit failure before effect refuses; after an effect it preserves
partial/uncertain recovery.

Default budgets: 60 authenticated calls/client/minute, five new held sends/client/minute, 16
outstanding held intents/client, and five approvals/recipient/minute. Send counters persist across
restart. Unauthenticated edge traffic is globally bounded to 20 attempts/minute and 32 concurrent
connections. Rate/quota refusal is explicit and creates no hidden queue. Bound storage/counters;
cannot-write/over-capacity fails closed before effects.

Message errors are signed `{error:{code,message},operationId?}` responses after authentication.
`REMOTE_INPUT_INVALID`, `REMOTE_SCOPE_DENIED`, `REMOTE_REPLAY`, `REMOTE_INTENT_CONFLICT`,
`REMOTE_CLOSED`, `REMOTE_INPUT_TOO_LARGE`, `REMOTE_RATE_LIMITED` (optional retryAfterMs),
`REMOTE_CURSOR_EXPIRED`, `REMOTE_CORE_UNAVAILABLE` and `REMOTE_STATE_UNAVAILABLE` have the rules
above. Unsupported operation/profile/binding is REMOTE_INPUT_INVALID. Preserve permitted core
resource/errors, not a conflicting exchange engine. Pre-auth rejection is generic and cannot
authorize retries or reveal grants.

## Transport binding: `loopback-http` (M1)

Descriptor address is `http://127.0.0.1:<bound-port>/r/<32 lowercase hex>`. Only foreground serve
binds IPv4 loopback; no LAN/wildcard option or redirect. Require exact numeric Host/bound port,
reject forwarded-host authority, cookies, ambient bearer authentication, duplicate framing headers,
queries/fragments, percent escapes/dot segments or extra slashes. No unauthenticated GET inventory.

| HTTP route        | Message-layer action                                                                                   |
| ----------------- | ------------------------------------------------------------------------------------------------------ |
| `POST /append`    | One signed request or session.open control envelope; authenticate before durable adoption/core access. |
| `POST /subscribe` | One signed subscribe control; bounded long-poll, signed response batch/cursor. No SSE in M1.           |
| `POST /ack`       | One signed ack control; no core attention mutation.                                                    |
| `POST /pair`      | Enrollment fields/proofs for an already machine-opened local offer; no client-created offer.           |

Route action and envelope kind/operation must agree. Body is one UTF-8 JSON document, Content-Type
application/json, one Content-Length, no transfer encoding, at most one request/connection;
Connection: close. Header/body acquisition times out within five seconds; pairing max 16 KiB,
headers max 8 KiB. All routes listed above are suffixes of the descriptor route prefix. Envelope
payload bounds come from core capabilities plus a fixed 8 KiB metadata budget; base64 wire bound is
exactly `4 * ceil(decodedLimit / 3) + 8192`. Subscribe bounds include at most 50
metadata/notification entries; full core bodies use a separate bounded result request, never an
unbounded batch. Core subprocess deadline is 15 seconds and advertised output cap is enforced;
failed write observation is uncertain.

Except for the bounded enrollment form on /pair, unsigned/unknown/revoked/expired requests receive
HTTP 404 with body `{}` and no machine key/inventory. Edge may verify the same Ed25519 envelope
before handing it to the message service; its authorization is still rechecked there. Other HTTP
outcomes only report delivery: 200 for signed response, 202 for adopted request, 400/413 for bounded
framing, 429 for edge rate refusal and 503 for unconfirmed transport. Never infer logical
acceptance/failure from HTTP alone.

An add-on's exact HTTP Origin must match the envelope/grant origin. Pending pairing checks the
proposed add-on origin, then terminal approval pins it. A CLI has origin `cli` and no browser
Origin. Missing/null/additional origin or envelope mismatch cannot silently pass as addon traffic.
Real MV3 acceptance must prove the selected fetch context supplies this Origin; change the reviewed
binding if browser evidence requires it. CORS names only that exact allowed origin, methods/headers,
no wildcard/credentials. Bounded OPTIONS has no core effect or authority; every actual request still
requires its proof/signature.

## Reserved future profiles and bindings

`relay-v1` reserves an owner-controlled relay profile with balanced PAKE and authenticated
end-to-end encryption. No algorithms/messages/deployment or compatibility claim is specified. M1
rejects this profile.

| Reserved binding | Future mapping only; not implemented                                                                                                                                                                                                                                                                                                                                                                                                         |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cloudflare`     | Append signed envelopes to a per-machine Durable Object through a Worker; subscribe from opaque cursor over WebSocket/long-poll; ack a scoped log prefix. Worker rejects unsigned/unknown traffic with HTTP 404 before forwarding. Message-layer authorization remains authoritative.                                                                                                                                                        |
| `firestore`      | Append envelope documents to a collection per paired machine; subscribe from cursor via snapshot listeners; ack through a device-scoped checkpoint. Rules deny unauthorized writes at the edge. Firestore Rules cannot be assumed to verify arbitrary Ed25519 signatures: a future reviewed trusted signature-verification admission service and Rules/device-key linkage are required. Public keys stored in documents alone grant nothing. |

Both preserve correlation, replay, scope, approval and audit in the common message owner. Future
bindings must specify their edge admission and encryption profile before use; neither is a permitted
M1 transport or deployment plan.

## Conformance and acceptance

Later fixtures pin canonical envelope/enrollment bytes and SHA-256 digests, Ed25519 public/signature
encodings, full MACs, four-word indexes/list digest, stream cursors and expected refusals with
source/version provenance. Consume independent
[RFC 8032](https://www.rfc-editor.org/rfc/rfc8032#section-7.1) and
[RFC 4231](https://www.rfc-editor.org/rfc/rfc4231#section-4) vectors. Use a third implementation to
establish application canonical-byte expectations, never the product itself. Native/browser build
canonical bytes independently; Chrome non-extractable key signs → Rust verifies and fixture-native
key signs → Chrome verifies. Compare exact deterministic signatures/MACs too.

Use valid positive controls and single-condition negatives: changed logical op/
payload/audience/origin/time/sequence, changed decoded-payload whitespace, wrong/weak/noncanonical
key, out-of-range scalar, malformed base64url/lengths, bad MAC/challenge, three attempts, no
terminal confirmation, stale grant/window, revoke/close, cross-client IDs/cursors and altered retry
intent. Denied/held work has no core mutation or pane input. Outer-envelope JSON whitespace changes
remain a valid positive control because those formatting bytes are not signed. Test duplicate
append, subscription catch-up/reconnect, ack idempotency/isolation, cursor expiry/capacity,
concurrent revoke/approve, crash before/after core acceptance, no repeated wake and no automatic new
ID.

Real MV3 Chrome 137 and current stable prove generate/store/reload/sign across worker suspension,
private export/wrap denial, public export, exact Origin, no-gesture/page-message refusal, exact
preview and inert/empty final rendering. Shell stubs are UI evidence only, not crypto/server
acceptance. Isolated real core/remote binaries, HOME/XDG/keys/browser profile, private tmux and
deterministic mock agents prove the integrated flow. Docker uses network none/internal loopback; no
host CLI fallback, real model/account/relay/credentials. Forced exit/timeout/assertion failure must
stop/reap listener/child/tmux/socket/buffers; run lifecycle acceptance twice and retain evidence on
cleanup failure.
