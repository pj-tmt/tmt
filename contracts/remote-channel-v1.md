# Remote channel protocol v1

**Status: proposed, not implemented.** This document owns the remote channel: device identity,
trust grants, wire values, the operations remote admits, the extension channel API and backend
bindings. tmt-lead reviews it before runtime/SDK code. The
[security design](https://github.com/wkh237/tmt/issues/478#issuecomment-5910827518),
[M1 ruling](https://github.com/wkh237/tmt/issues/478#issuecomment-5911118171) and
[transport-layer decision](https://github.com/wkh237/tmt/issues/478#issuecomment-5911165578) are
its inputs. The owner's 2026-10-02 decisions on [#955](https://github.com/wkh237/tmt/issues/955)
supersede the M1 rules for mandatory hold, per-agent allowlists, own-results-only reads, forced
grant expiry and serve windows.

## User path

1. **Start.** `tmt remote start` runs the remote door and the routes of enabled extensions. It does
   not enroll, restart or reconfigure agents, and needs no `--channel` flag or separate MCP setup.
   Native-channel enrollment is a local driver concern.
2. **Pair once.** `tmt remote pair` authorizes one device: the owner opens the printed link or
   enters the code on that device, sees the same four words on both sides and confirms once in the
   terminal. The device is now a trusted source for all current and future owner-local agents. A
   time limit, hold mode or agent allowlist is offered at that confirmation and is off by default.
3. **Talk.** A trusted device talks, checks and reads results the way the local CLI does. Its
   messages go straight through the ordinary core dispatch path, with no per-message approval.

Automatic validation is remote's job and never adds a user step. A design that needs another user
step goes to the owner before it enters this contract.

The four hard lines are: authenticated pairing; no arbitrary shell or command endpoint; no paste
into an enrolled pane; no automatic resend after an uncertain outcome. In addition, membership of an
extension resource (for example a shared colab page) never implies machine or agent access.

## Owners and layers

The message layer owns signed envelopes, correlation, durable append/subscribe/ack semantics,
sessions, replay, trust grants, optional hold, uncertainty and audit. A binding moves those messages
and rejects unauthenticated traffic at its edge; HTTP status, a document ID or a WebSocket
connection never grants authority. Changing bindings must not change the signature, scope or
grant rules.

All listening and remote state belong to `extensions/tmt-remote`. One extension-owned transport
trait exposes `append`, `subscribe` and `ack`; it does not execute core work or decide authority.
The same message service admits every binding before effects. Core never listens or stays resident.
Remote uses `$TMT_EXECUTABLE api` and documented ordinary JSON commands; it never opens core SQLite,
imports core behavior crates, calls host adapters or scrapes panes itself. The
[extension API](extension-api.md) owns core JSON resources, errors, limits, retention and
durable request/dispatch semantics.

Extensions are apps on remote. Colab is the first: it owns pages, Yjs state, epochs, its content
keys, page membership, renderer and bridge ledger, and consumes the
[extension channel API](#extension-channel-api). An extension does not ship its own door, device
sign-in, pairing, backends or deploy.

This profile protects against unpaired clients, malicious pages/other origins reaching the door,
replay, scope expansion and accidental duplicate sends. Loopback, CORS and a route prefix are not
credentials. Same-user malware, a compromised browser/add-on, malicious selected executable and
compromised OS account are outside this profile. Non-extractability restricts key export; it is not
hardware isolation or protection from code that can invoke the key. `local-v1` does not encrypt
operation content; cloud bindings need the encryption profile in
[Backends and deploy](#backends-and-deploy) before use.

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
as the core public API accepts. Grant agent allowlists contain such core references.
Syntax validation does not establish identity existence or grant authority. Core request IDs
retain their `req_...` form. Clients tolerate additive response fields; incompatible required
fields or semantics need another protocol major. Unknown profiles/bindings fail closed.

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

Implementation must verify lockfile/platform/MSRV/advisory evidence. There is no WASM or
third-party JS signing library. Node conformance targets Node 24; browser persistence evidence
still requires actual MV3/IndexedDB tests.

## Device identity

A device is one key holder: a browser add-on, a browser page origin served by the remote door or a
deployed backend app, or a `tmt` CLI on another machine. Each device has exactly one Ed25519
device key, generated on the device. Extensions never create a second owner-device identity.

A browser device generates `crypto.subtle.generateKey({name:"Ed25519"}, false, ["sign","verify"])`
and stores the private CryptoKey itself by IndexedDB structured clone in its own origin; it exports
only the public key. Never JSON-serialize, sync, export or transfer private material to page or
content scripts. Machine and CLI device keys use an OS-backed store or an owner-only 0600 key file
under remote's own subtree; no non-extractability/hardware claim is made for a software file. Use
OS CSPRNG/WebCrypto entropy, never a clock, UUID string or `Math.random` as key/code entropy. Fixed
seeds are test-only.

Device kinds are `addon` (exact installed `chrome-extension://<id>` origin), `browser` (the exact
HTTPS origin of a deployed app, or the door's exact loopback origin) and `cli` (literal origin
`cli`). Reject null, opaque and wildcard origins. Origin is bound in enrollment and every envelope.
Signing never occurs in content scripts or in untrusted page content; an extension that renders
untrusted HTML does so in a sandboxed opaque-origin frame without access to device keys.

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

Examples are illustrative, not valid credentials/vectors. `clientId` is the device ID. `windowId`
names the remote run that issued the session; it rotates on every start. `payload` is exact JSON
bytes encoded as base64url (`e30` is `{}`). Do not parse/reserialize the payload before hashing it.
Transport JSON formatting is not signed content. The decoded payload must pass its operation's
strict admission before effects.

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
`{sessionId,serverTimeMs,grantRevision,expiresAtMs}`. `expiresAtMs` is the grant expiry, or null
when the grant has none; a session also ends when remote stops. Opening a session is silent and
never asks the owner to pair again. The client verifies the paired machine key before trusting it.
One session per client; creating another invalidates the previous one without widening scope.

Normal messages also require a timestamp within 60 seconds of machine time. Normal client sequences
start at 1 and increase by exactly one. One request is in flight per session. Under remote's
authority lock, verify signature, audience, origin, live grant, timestamp, scope and expected
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
permits a captured-envelope replay or automatic new send. Expiry/revoke/stop is checked again at
the effect fence. Bindings cannot waive those checks because an edge previously accepted a
signature.

A `browser` device on the door's own origin may instead hold a door session: after one signed
`session.open` over the door, remote sets a 256-bit random token as an HttpOnly, SameSite=Strict
cookie (Secure on HTTPS), stores only its SHA-256 and binds it to the device, grant revision and
remote run. Browsers cannot set authorization headers on WebSocket construction, so tokens never
move into query strings. The cookie is a carrier for the same authenticated device context, not a
second credential model: every request and upgrade rechecks grant, revocation and expiry, and a
state-changing operation still needs a fresh device signature over its exact intent.

## Durable log: append, subscribe and ack

Each paired client sees one machine-owned ordered stream of its admitted-request receipts and
correlated state notifications. Another client's IDs/cursors disclose nothing. The log is a remote
delivery journal over core resources, not a second conversation database or core attention queue.
Remote retains bounded held/uncertain payloads and immutable operation/request references; core
alone owns conversation history and retained finals. Final notifications can reference a core
request; `result` retrieves its current retained body through the public API. No permanent
final-body copy or new retention lease is created in remote.

`append(requestEnvelope)` authenticates/adopts the logical request and stores its ID, client
ownership and frozen payload/digest atomically before returning acceptance. Under a hold-mode grant
this appends a held record and signed `held` response only; it does not call core. Receipts
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
operation/request ownership records survive journal eviction for 30 days after their last state
change. Limit these to 1000 operations/client and refuse new adoption at capacity; never evict an
uncertain operation to admit another. After this recovery horizon an owned read returns
REMOTE_STATE_UNAVAILABLE, never permission to resend. Revoked/expired grants cannot use these
records to regain access.

## Pairing

Pairing is the one-time authorization of a device as a trusted source. Only local
`tmt remote pair` opens an offer on a running remote. A client cannot initiate or extend pairing.
One offer at a time; replacing it explicitly cancels the previous offer. The terminal prints a
pairing link for browser devices and the same code as text for add-on and CLI devices. The link
carries the descriptor in its path and the code only in its URL fragment; trusted page code removes
the fragment before any other script runs, and the code never enters an HTTP URL, log or analytics.
The descriptor carries profile/binding, machine/window/offer UUIDs, address and a random 128-bit
server challenge (32 lowercase hex characters). No private key is included in a URL, ordinary
web-page DOM or log.

The code is **16 random bytes**, displayed as 26 uppercase RFC 4648 base32 characters grouped for
**copy/paste**, one use, ten-minute expiry. Decode after removing ASCII spaces/hyphens only,
rejecting invalid alphabet/nonzero unused bits. It is not a short numeric/word password: an observed
known-message HMAC allows offline guesses, and three attempts/expiry do not prevent that attack. The
high entropy is mandatory; do not describe this scheme as PAKE.

The device proposes its public key, kind, exact origin, proposed name and a random 128-bit
clientNonce. It does not propose agents, scopes, mode or expiry; the owner's confirmation sets them.
Names are 1–64 nonblank UTF-8 bytes without controls; origin is at most 128 bytes.

Enrollment bytes are LP(`tmt-device-pair-v1`), LP(profile), LP(machineId), LP(windowId),
LP(offerId), LP(decoded serverChallenge), LP(decoded clientNonce), LP(kind), LP(origin), LP(name)
and LP(raw public key). Submit those fields plus full base64url HMAC-SHA256(codeBytes,
enrollmentBytes), and an Ed25519 signature over LP(`tmt-device-pair-possession-v1`) ||
LP(enrollmentBytes) || LP(raw HMAC). Machine checks proof in constant time, strict key/possession
signature and live offer/challenge before pinning the candidate. The pair request may wait for
local confirmation only until the offer deadline; identical pending candidates coalesce, while a
competing candidate cannot replace the pinned one. Enrollment is the only unpaired message form; it
authorizes no core work.

Both sides show four words derived from SHA-256(LP(`tmt-local-key-fingerprint-v1`) || LP(raw public
key)): take the first 44 bits as four successive unsigned 11-bit indexes into the fixed English
BIP-39 2048-word list, in list order. Use the
[BIP-39 English list](https://github.com/bitcoin/bips/blob/master/bip-0039/english.txt); pin its
revision, bytes/digest and examples in the fixture slice. This is comparison text, not a recovery
mnemonic. The terminal shows kind, full origin, proposed name and the words, and asks for one
confirmation. Defaults are all current and future agents, the default scopes, `direct` mode and no
expiry. The same prompt offers, without requiring them, a final name, a time limit, `hold` mode and
an agent allowlist. The owner may narrow authority, never enlarge it beyond this profile.

Enrollment JSON names are `profile`, `machineId`, `windowId`, `offerId`, `serverChallenge`,
`clientNonce`, `kind`, `origin`, `name`, `publicKey`, `mac` and `signature`; binary keys/proofs use
base64url, challenge/clientNonce use 32 lowercase hex characters. Reject unknown fields.

After confirmation, atomically create one grant and consume the offer. Receipt JSON is
`{grant,machinePublicKey}` with the grant fields defined below. The response carries base64url
exact `receipt` bytes and `serverProof`. Define
`K_response = HMAC-SHA256(codeBytes, LP("tmt-device-pair-response-key-v1") || LP(enrollmentBytes))`.
`serverProof` is the full HMAC-SHA256(K_response, LP("tmt-device-pair-response-v1") || LP(exact
receipt JSON bytes)). The device derives this key and verifies the proof before pinning the machine
key or accepting the grant; it verifies matching machine/profile/device key/kind/origin and a
well-formed grant. Exact candidate retry is idempotent until the original offer deadline; preserve
only K_response, the candidate digest/proofs and exact receipt for that bounded lost-response
recovery. Verify matching candidate and possession on retry; never accept K_response as a reusable
enrollment token. Raw code is erased after confirmation; no grant exists before local confirmation.
Delete the recovery key at the original deadline. Three failed code proofs, expiry, owner
refusal/no confirmation, stop or process exit cancels pending pairing and erases its secret. No
grant or device private key is stored for a failed/cancelled offer; sanitized audit may record
failure only.

Lost key, changed add-on origin, revocation or expiry requires pairing again. No P-256 downgrade,
export-based backup, automatic key replacement, passkey renewal or remote grant extension.

## Trust grants and remote lifecycle

A grant makes one device a trusted source on one machine. Grant JSON names are `clientId`,
`machineId`, `profile`, `publicKey`, `kind`, `origin`, `name`, `agents`, `scopes`, `mode`,
`issuedAtMs`, `expiresAtMs`, `revision` and `disabled`. `agents` is the string `"all"` (every
current and future owner-local agent, the default) or a sorted unique list of at most 256 core
identity UUIDs. `mode` is `direct` (default) or `hold`. `expiresAtMs` is null (default, no expiry)
or the time limit the owner chose at pairing. `revision` is a positive integer; `disabled` is
false on issue. Names are presentation only; rename preserves UUID authority, and a retired
identity's same-name replacement inherits nothing. After pairing, authority may only be narrowed
or revoked through local management (`tmt remote devices`).

Default scopes are `agents.read`, `status.read`, `check.read`, `talk` and `results.read`. The owner
may remove scopes at pairing. Future core capabilities do not silently become remotely callable;
a new scope needs a revision of this contract.

`tmt remote start` runs until `tmt remote stop`, with no default idle or hard deadline; the owner
may pass a run limit. Its background lifecycle follows Office's start/stop/status shape. The door
address and route prefix are stable across restarts and are not credentials. Stop disables the
door before acknowledgment and cancels pending pairing and held operations. Revoke disables a
device before acknowledgment. No request/effect not yet fenced may succeed afterward. Already
committed core work is not undone; report it accurately. Restart issues a new window and session
namespace; grants survive. Unconfirmed held work is cancelled; dispatching/uncertain work recovers
its original operation, never becomes falsely unsent.

## Operations

Payloads for core API operations are the existing API envelope, decoded without rewriting input.
Remote narrows supported operations/authority before core calls.

| Logical operation                                                                  | Scope and public core mapping                                                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capabilities`                                                                     | Signed paired discovery of supported subset, fixed suite and core bounds.                                                                                                                                                                     |
| `agents.list`                                                                      | `agents.read`; `tmt ls --json` projected to permitted UUID/name/presence and delivery status, without pane address/cwd/process/profile.                                                                                                     |
| `identities.status`                                                                | `status.read`; input restricted to permitted UUIDs. Self-report is not readiness or completion.                                                                                                                                              |
| `check`                                                                            | `check.read`; one permitted agent, the bounded capture `tmt check --json` returns locally. Read-only; it never writes to a pane.                                                                                                             |
| `dispatch.create`                                                                  | `talk`; one permitted direct request recipient, anonymous core originator plus remote provenance (below). `direct` grants dispatch after admission; `hold` grants hold for local approval. No fan-out, room or announcement in v1. |
| `dispatch.show`, `operation.show`                                                  | `talk`; only journal-owned operation IDs; core immutable receipt or remote held state.                                                                                                                                                       |
| `requests.show`, `result`                                                          | `results.read`; any request the local `tmt result` can read, through the public API or `tmt result --json`.                                                                                                                                 |
| `requests.list`, global `changes.cursor`, `references.resolve`, `rooms.roster`     | Unsupported in v1; later projections need explicit scoped admission.                                                                                                                                                                         |
| `notes.read`, `rooms.write`, `rooms.retire`                                        | Unsupported in v1.                                                                                                                                                                                                                            |
| `identityHooks.*`, `skills.install`, `skills.remove`                               | Never remotely callable; JSON consent cannot manufacture local lifecycle/install authority.                                                                                                                                                   |
| reply/answer, X acknowledgment, config, pair, run/resume, approvals, installation | Never remotely callable. Result/log ack does not reply or acknowledge core work.                                                                                                                                                              |
| any other command, argv or shell                                                   | Never; there is no generic command endpoint.                                                                                                                                                                                                  |

`agents.list`, `check`, `operation.show` and `result` are adapter helpers over ordinary public JSON
commands, not new core API operations. SDK `api(op,input)` cannot reach local management/argv
through an invented operation. Delivery status is the read-only `delivery` projection that core
owns in its public `ls`/API JSON: `channel` (enrolled native channel ready), `paste` (ordinary paste
delivery), `not_ready` (enrolled but not ready, with core's local recovery hint) or `not_running`.
Remote forwards it unchanged and never infers it from panes; until core publishes that projection,
`agents.list` reports presence only.

Helper payloads are `agents.list:{}`, `check:{agentId,lines?}`, `operation.show:{operationId}` and
`result:{requestId}`. `dispatch.create` payload is exactly
`{version:1,operation:"dispatch.create",originator:"anonymous", input:{operationId,recipientIds:[agentId],message,kind:"request"}}`.
Signed envelope operation and core operation must agree, and envelope id must equal
input.operationId. Other supported core reads use their documented `{version,operation,input}`
envelope. Discovery returns `{version:1,profile:"local-v1",binding,operations,limits}` with only the
admitted operation subset and effective byte/rate bounds.

A remote device is never a local identity. Core records the request with an anonymous originator;
remote prefixes the frozen message with one provenance line naming the device,
`[remote: <device name>]`. A `cli` device additionally names the sending identity it resolved on
its own machine, `[remote: <device name> / <identity name>]`; that name is presentation only and is
never mapped to an identity on the receiving machine. Callers reject a caller-selected `identity`.

On a paired `cli` device, ordinary `tmt talk` addressed to a remote agent is the send action: it
freezes the operation ID and exact bytes, appends `dispatch.create`, and waits for the durable
reply through `result`, as a local talk waits for its reply. Remote agents are addressed by their
plain name, as in local `tmt talk`; a local identity with that name always wins. A name with no
local match resolves to the one paired-machine agent of that name, and the command output names
the machine. `<name>@<machine>` (the machine name recorded when this device paired) selects explicitly
and is needed only when the plain name is ambiguous across paired machines; ambiguity refuses and
lists the qualified candidates rather than choosing one.

The wire-independent client boundary is `@tmt/remote-client`. `RemoteClient` has
`listAgents():Promise<{id,name,delivery?}[]>`, `send({operationId,agentId,message})`, read-only
`operation(operationId)`, `check(agentId)` and `result(requestId)`. Send/operation use SendState;
result uses ResultState as defined below. No selection/URL/title/note fields are reformatted by the
SDK: they are already inside the frozen message. `ClientError` is `{code,message,retryAfterMs?}`,
with a sanitized message at most 256 UTF-8 bytes and retryAfterMs an integer from 0 to 60000, with
code `unpaired`, `closed`, `scope_denied`, `rate_limited`, `input_invalid` or `unavailable`. An
unknown failure after send may have begun becomes uncertain, preserving the same
operationId/message. No implicit send from recovery or automatic retry.

## Dispatch, hold and uncertainty

A new talk freezes one remote request/core operation UUID, permitted agent UUID and exact final
message before append. SDK persists them first. No edit under an existing operation ID.

Under a `direct` grant, a trusted device's admitted request goes straight to core: after
authentication, grant/scope/sequence checks and durable adoption, remote durably marks it
`dispatching` and calls existing `dispatch.create`. Remote adds no readiness or typing inference of
its own: core owns idempotency, acceptance, the one-shot advisory wake and its ordinary delivery
protections, including native-channel delivery, never pasting into an enrolled pane and the `!`
transport protection. The earlier [readiness contract](https://github.com/wkh237/tmt/issues/600)
does not gate this path.

Under a `hold` grant, append stores ownership, exact core envelope/hash and `held` state before
returning `{state:"held",operationId}`. **There is no core requestId yet.** Approval is only local
`tmt remote approve <operationId>`: show frozen bytes, source/current name/UUID, require explicit
confirmation, recheck grant and recipient, and durably mark `dispatching` before the exact core
call. No client can approve itself. Refusal/cancellation is a correlated operation response, not an
agent final.

Append core's accepted response with request IDs under the original correlationId. Agent completion
is read through core and published under that same correlation as metadata
`{operationId,requestId,resultState:"pending"|"replied"|"unavailable"}` without a message body; the
explicit result read supplies the body. No terminal-output completion fallback.

Core/transport timeout, process crash, lost reply or uncertain wake retains the same frozen
intent/operation ID and reports `uncertain`, not a new send. Recover through authorized
`dispatch.show`. A definitive `DISPATCH_NOT_FOUND` after confirming the owned child stopped allows
retry of the **same** intent/ID after authority revalidation; never retry a claimed wake or replace
an uncertain operation with a new ID. Log adoption/replay likewise returns its original acceptance.
Core-final expiry is `unavailable`, not failed processing or permission to resend. Reads never renew
retention or acknowledge attention.

Signed response payloads discriminate `{state:"held",operationId}`,
`{state:"accepted",operationId,requestId}`, `{state:"uncertain",operationId, requestId?}`, and
`{state:"refused"|"cancelled",operationId,reason?}`. `result` returns `{state:"pending",requestId}`,
`{state:"replied",requestId, message}` (including an empty final), or
`{state:"unavailable",requestId,reason?}`. Pending/not-retained/expired distinction follows the
public core observation; never assert why a body is unavailable without evidence. Optional reason is
a sanitized string of at most 256 UTF-8 bytes; never expose raw process output.

## Browser use and audit

The trusted UI that sends (the add-on shell, or an extension's trusted parent chrome on the door)
owns recipient selection, final message formatting and exact preview. It shows all characters, with
a separate escaped view for hidden controls; the SDK signs those same frozen bytes without adding
text after preview. Agent/content changes explicitly create new intent. Render replies and check
captures as inert plain text, not HTML. Core preserves stored message and applies its public
size/`!` transport protection; explain that adaptation, do not silently rewrite the reviewed
message. Refuse credentialed URLs rather than secretly dropping fields from the preview.

Signing is allowed only from trusted UI after an explicit gesture. That action may authorize bounded
own-state/reply observation while its UI remains active; closing observation never cancels
recipient work. The add-on has no externally_connectable, page-message signing, external message
handler, remote scripts, content-script credentials or broad page scraping. Shell permissions are
activeTab, scripting and contextMenus. Popup capture and browser contextMenus.onClicked are the only
entry points. Background hands transient capture and frozen intent to the popup through
extension-owned IndexedDB; page messages cannot initiate that handoff. Exact door host permission is
added for real SDK integration. Worker restart loads CryptoKey and frozen IDs from IndexedDB,
creates a new session and recovers operation state before any explicit retry. Never automatically
resend.

Remote keeps files only in its own subtree of the data root reported by `tmt api` operation
`storage.root`, with owner-only directories, 0600 secret/state files, no-follow bounded
regular-file admission and durable atomic state replacement. Never rewrite core DB/config or
provider settings. Local append-only audit records time, device/request/operation IDs, resource
UUIDs, digest, grant revision, decision and sanitized code before effects and outcome afterward.
Never log code, MAC, signature, private key, session token, message/reply, check capture, URL/title
or reply receipts. It is not tamper-proof against the OS user or secure erasure. Audit failure
before effect refuses; after an effect it preserves partial/uncertain recovery.

Default budgets: 120 authenticated calls/device/minute, 20 new sends/device/minute, 16 outstanding
held intents/device, and 60 approvals/recipient/minute. Send counters persist across restart.
Unauthenticated edge traffic is globally bounded to 20 attempts/minute and 32 concurrent
connections. Rate/quota refusal is explicit and creates no hidden queue. Bound storage/counters;
cannot-write/over-capacity fails closed before effects.

Message errors are signed `{error:{code,message},operationId?}` responses after authentication.
`REMOTE_INPUT_INVALID`, `REMOTE_SCOPE_DENIED`, `REMOTE_REPLAY`, `REMOTE_INTENT_CONFLICT`,
`REMOTE_CLOSED`, `REMOTE_INPUT_TOO_LARGE`, `REMOTE_RATE_LIMITED` (optional retryAfterMs),
`REMOTE_CURSOR_EXPIRED`, `REMOTE_CORE_UNAVAILABLE` and `REMOTE_STATE_UNAVAILABLE` have the rules
above. Unsupported operation/profile/binding is REMOTE_INPUT_INVALID. Preserve permitted core
resource/errors, not a conflicting exchange engine. Pre-auth rejection is generic and cannot
authorize retries or reveal grants.

## Extension channel API

Remote offers extensions five parts. Each is consumed through remote; an extension never
reimplements one.

**Device context.** A request, upgrade or relay frame from a paired owner device reaches the
extension with the authenticated device context `{deviceId, kind, origin, name, owner:true,
grantRevision}`. A cloud edge attributes a non-owner principal it authenticated for that extension
as `{principal, owner:false}`; on the local door a non-owner request arrives without a device
context and the extension authenticates it (below). Extensions query the current principal from the CLI
and browser SDK and subscribe to device revocation and renaming events (for example to rotate page
epochs). An extension that needs its own keys generates them on the device and asks the device key
to certify them: the device signs LP(`tmt-ext-cert-v1`) || LP(extension) || LP(purpose) ||
LP(raw extension public key) || LP(decimal issuedAtMs), where purpose is `sign` or `enc`. The
certificate binds an extension key to a device; it grants no remote authority by itself, and
extension cryptography stays owned by the extension.

**Route mounting.** The door mounts each enabled, owner-installed extension under
`/x/<extension>/`, forwarding HTTP requests, static assets and WebSocket upgrades to the extension
process over an owner-only local socket in the extension's data subtree, together with the device
context. Remote owns Host, Origin and CSRF admission, framing, connection/body limits and TLS; the
extension owns its responses, content security policy and headers. All mounted extensions share one
browser origin and therefore one browser trust domain; mounting is limited to owner-installed
extensions, and untrusted content renders only in sandboxed opaque-origin frames. Plain HTTP is
loopback-only; an opt-in non-loopback bind requires HTTPS and WSS with a user-supplied certificate.
Upgraded WebSocket tunnels do not use the door's edge connections, so open pages cannot starve
remote operations, pairing or page loads. Each mounted extension has its own tunnel cap and idle
bound: a tunnel with no bytes in either direction for the idle bound is closed, and an upgrade
beyond the cap is refused with HTTP 503 and `Retry-After` before it reaches the extension. An
extension keeps one WebSocket per page and reconnects after an idle close or a refusal.

**Relay.** Remote carries opaque, namespaced logs for extensions and never decrypts or interprets
their payloads. A namespace is `<extension>:<path>` (for example `colab:<space>/<page>/<stream>`).
Operations are append with create-only per-stream sequence (an exact retry returns the original
receipt), subscribe from a scoped opaque cursor with bounded catch-up paging, ack as a cursor
checkpoint only, an object store keyed by object ID with chunked transfer, and an ephemeral
awareness lane that is never stored. On the `local` backend an extension may instead serve its own
relay namespaces behind its mounted WebSocket; remote then only splices bytes and the extension
keeps admission. The remote-run relay applies to cloud backends. Before accepting an append, subscription, object transfer or
awareness frame, remote calls the extension's synchronous admission hook with the device context
and frame metadata; the extension decides membership, role, epoch and writer checks. Revocation terminates live subscriptions. Remote
enforces per-object size caps and per-namespace quotas and expiry declared by the extension. Relay
ack is never core X acknowledgment, task completion or send.

**Operations and agent status.** Extensions send agent work only through
[Operations](#operations) with the device context of the user who acted, a caller-frozen operation
ID and exact bytes. Comments, sync, replay and rendered content never dispatch. An extension ledger
maps onto the operation receipts and `operation.show` found/not-found recovery; results are
readable only for operations its device owns. Agent delivery status comes from `agents.list`.

**Backends.** Extensions declare the resources they need per backend (collections or paths, Rules
or Worker admission fragments for their namespaces, TTL fields, indexes, blob storage). Remote
provisions and deploys them with its own resources, as below.

Principals stay separate. Owner devices are paired through remote and are the only principals that
can call operations. People an extension shares with (page members, link holders) are never paired
by remote. On a cloud backend the edge and the extension's admission fragments authenticate them. On
the local door, a mounted-route request without an owner-device session is forwarded to the
extension without a device context; the extension authenticates it itself (for example by link-key
possession or a member device signature) and attributes it as `{principal, owner:false}`. Either
way they reach only the routes and relay namespaces the extension admits, never the remote
operation routes, and never receive operation scopes.

**Members' own agents.** A non-owner member may ask agents only on their own machine, never the
owner's. The member's browser holds a device paired with the member's own machine, and the ask
travels as an ordinary operation from that device to that machine under that machine's own grant.
The owner's machine never executes it, and page membership adds no operation scope anywhere. The
extension records the ask and the reply in the shared resource, attributed to the asking member and
to the answering agent and machine. Everyone who can see that resource sees them, like comments.
Visibility is the extension's rule, not a remote grant.

## Backends and deploy

The same message, relay and admission owners serve every backend: `local` (the door plus extension
stores under the data root), then Firestore, then Cloudflare. `tmt remote deploy <backend>` creates
one deployment per backend in the owner's own account, composing remote's resources with every
enabled extension's declared resources. Deploying into a real account requires the owner's
explicit authorization for that account at deploy time; tests use emulators, local workerd or
Miniflare only.

| Backend      | Mapping; not implemented                                                                                                                                                                                                                                                                                                                                                                 |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `local`      | `loopback-http` binding below for operations; mounted extension routes and WebSocket relay on the same door; state under the data root.                                                                                                                                                                                                                                                |
| `firestore`  | Append envelope and relay documents per machine/namespace; subscribe from cursor via snapshot listeners; ack through a device-scoped checkpoint. Rules deny unauthorized writes at the edge. Rules cannot verify arbitrary Ed25519 signatures: the owner machine verifies every operation envelope before effects, and the edge admission design must be specified here before use. |
| `cloudflare` | Append to a per-machine Durable Object (and per-namespace objects for relay) through a Worker; subscribe from opaque cursor over WebSocket; ack a scoped prefix. The Worker rejects unsigned/unknown traffic with HTTP 404 before forwarding. Message-layer authorization remains authoritative.                                                                                     |

The owner machine connects outward to a cloud backend and stays authoritative for operations; an
edge only limits abuse. Operation payloads crossing a cloud backend must be end-to-end encrypted to
the machine key under a reviewed encryption profile added to this contract; relay payloads are
already encrypted by their extension. Until both the edge admission and that encryption profile are
specified, `firestore` and `cloudflare` are not permitted.

## Provisioning on start and pair

`tmt remote start` mounts every enabled extension and prepares its relay namespaces and owner-machine
bridge, and pairing a device makes it known to every mounted extension through the device context.
No extension adds its own sign-in, pairing, bridge enrollment or machine grant step. An extension
asks for an extension key certificate silently on first use.

## Transport binding: `loopback-http`

The door address is `http://127.0.0.1:<port>/`. Remote operations live under `/r/<32 lowercase
hex>/`, a stable per-machine prefix that is not a credential; extensions live under
`/x/<extension>/`. Only the running remote binds IPv4 loopback by default. Require exact numeric
Host/bound port, reject forwarded-host authority, ambient bearer authentication, duplicate framing
headers, queries/fragments, percent escapes/dot segments or extra slashes on remote routes. Cookies
are admitted only as the door session described under signed envelopes. No unauthenticated GET
inventory.

| HTTP route        | Message-layer action                                                                                   |
| ----------------- | ------------------------------------------------------------------------------------------------------ |
| `POST /append`    | One signed request or session.open control envelope; authenticate before durable adoption/core access. |
| `POST /subscribe` | One signed subscribe control; bounded long-poll, signed response batch/cursor. No SSE.                 |
| `POST /ack`       | One signed ack control; no core attention mutation.                                                    |
| `POST /pair`      | Enrollment fields/proofs for an already machine-opened local offer; no client-created offer.           |

Route action and envelope kind/operation must agree. Body is one UTF-8 JSON document, Content-Type
application/json, one Content-Length, no transfer encoding, at most one request/connection;
Connection: close. Header/body acquisition times out within five seconds; pairing max 16 KiB,
headers max 8 KiB. All routes listed above are suffixes of the remote route prefix. Envelope
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
proposed origin, then terminal confirmation pins it. A CLI has origin `cli` and no browser Origin.
Missing/null/additional origin or envelope mismatch cannot silently pass as add-on traffic. Real
MV3 acceptance must prove the selected fetch context supplies this Origin; change the reviewed
binding if browser evidence requires it. CORS names only that exact allowed origin,
methods/headers, no wildcard/credentials. Bounded OPTIONS has no core effect or authority; every
actual request still requires its proof/signature.

## Current implementation and migration

Implemented today: a foreground deny-all door, pure `local-v1` canonical envelope framing,
`tmt-device-pair-v1` enrollment and possession builders, pairing-code text decoding, four-word
fingerprints over the pinned list (bitcoin/bips `ce1862ac` `bip-0039/english.txt`, SHA-256
`2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`, committed as
`extensions/tmt-remote/rust/tmt-remote/assets/bip39-english.txt`), strict Ed25519/HMAC
verification and `K_response`/`serverProof` derivation in Rust, and the matching TypeScript
builders. Their fixtures
are regenerated from the independent oracle (#1039); the superseded M1 enrollment vectors are
removed and envelope bytes are unchanged. Grants, receipts and the pairing ceremony are not yet
implemented.

Colab's working loopback door, sign-in and sync transport code relocates into `tmt-remote` as the
local door, device sign-in and relay where it meets this contract, rather than being rewritten.
The relocated door and `/x/<extension>/` route mounting are implemented (#1039), with only colab
allowlisted; mounted requests carry no device context until pairing lands. Local colab keeps
working until its routes mount on the remote door.

## Conformance and acceptance

Later fixtures pin canonical envelope/enrollment bytes and SHA-256 digests, Ed25519 public/signature
encodings, full MACs, four-word indexes/list digest, extension key certificates, stream cursors and
expected refusals with source/version provenance. Consume independent
[RFC 8032](https://www.rfc-editor.org/rfc/rfc8032#section-7.1) and
[RFC 4231](https://www.rfc-editor.org/rfc/rfc4231#section-4) vectors. Use a third implementation to
establish application canonical-byte expectations, never the product itself. Native/browser build
canonical bytes independently; Chrome non-extractable key signs → Rust verifies and fixture-native
key signs → Chrome verifies. Compare exact deterministic signatures/MACs too.

Use valid positive controls and single-condition negatives: changed logical op/
payload/audience/origin/time/sequence, changed decoded-payload whitespace, wrong/weak/noncanonical
key, out-of-range scalar, malformed base64url/lengths, bad MAC/challenge, three attempts, no
terminal confirmation, expired or revoked grant, stop, cross-device IDs/cursors and altered retry
intent. A `direct` grant reaches core exactly once per operation ID; a `hold` grant has no core
mutation or pane input before approval; an allowlisted grant refuses other agents; a grant with a
time limit refuses after it and one without never expires on its own. Delivery to an enrolled pane
is covered by core's no-paste tests, not re-implemented here. Test duplicate append, subscription
catch-up/reconnect, ack idempotency/isolation, cursor expiry/capacity, concurrent revoke/dispatch,
crash before/after core acceptance, no repeated wake and no automatic new ID. Extension tests cover
route isolation by prefix, device-context forwarding, admission-hook refusal before durable relay
state, revocation closing live subscriptions and a non-owner principal refused every operation.

Real MV3 Chrome 137 and current stable prove generate/store/reload/sign across worker suspension,
private export/wrap denial, public export, exact Origin, no-gesture/page-message refusal, exact
preview and inert/empty final rendering. Shell stubs are UI evidence only, not crypto/server
acceptance. Isolated real core/remote binaries, HOME/XDG/keys/browser profile, private tmux and
deterministic mock agents prove the integrated flow. Docker uses network none/internal loopback; no
host CLI fallback, real model/account/relay/credentials. Forced exit/timeout/assertion failure must
stop/reap listener/child/tmux/socket/buffers; run lifecycle acceptance twice and retain evidence on
cleanup failure.
