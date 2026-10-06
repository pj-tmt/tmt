# Remote channel protocol v1

**Status: proposed, not implemented.** This document owns the remote channel: device identity,
trust grants, wire values, the operations remote admits, the extension channel API and backend
bindings. Remote owns this contract; tmt-lead reviews changed core contracts and cross-squad
seams before runtime/SDK code. The
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
operation content; cloud bindings need the encryption profile in [Backends and
deploy](#backends-and-deploy) before use. Browsers do not isolate cookies by port on loopback:
another local user's listener on `127.0.0.1` could receive the door session cookie when the owner's
browser requests it at a matching path. The cookie is scoped to the mount space under the machine's
unpredictable 80-bit route prefix (#1094, shortened in #1687), so such a listener must already know
the prefix. Local processes can read it directly from `POST /sdk/mount` or an active
`GET /sdk/pair-offer`, so it narrows only listeners that never query the door, not a local attacker
that does. Cross-origin web pages cannot read these Origin-gated responses or a redirect's
`Location`; the prefix is still not a credential. That cookie grants extension page and relay access only, until
stop, revocation or idle expiry, never an operation or pairing action; state-changing operations
still need a fresh device signature.

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
Each open adds an independent session for the device, bound to the same grant and revision;
opening never widens scope. `limits::DEFAULT_SESSIONS_PER_DEVICE` caps sessions at 8 by default.
`tmt remote settings sessions-per-device <n>|off` sets a positive limit or disables it. An
absent `sessionsPerDevice` key in Remote's `settings.json` means 8; explicit null (`off`) means
unlimited. Both plain settings and JSON show the limit and its source (`default` or
`settings.json`); JSON uses null for unlimited. Serve reads the setting on each open, with no
restart. Damaged or unreadable settings use the default of 8. The active limit ends the least
recently used sessions of that device until the new session fits; other devices are unaffected.
The session table is keyed by sessionId with clientId indexed. Migration preserves grants and
maps each former single-session row, including its counters, to one entry. Persisted rows never
restore live authority across a serve restart.

Normal messages also require a timestamp within 60 seconds of machine time. Normal client
sequences start at 1 and increase by exactly one, independently for each session. The signed
canonical envelope binds sessionId and its sequence together: changing sessionId invalidates the
signature, and an envelope signed for one session cannot be accepted in another. One request is
in flight per session. Under remote's authority lock, verify signature, audience, origin, live
grant, timestamp, scope and expected sequence; consume the sequence durably before any effect.
Concurrent duplicates have one winner. Invalid signatures do not advance it. A consumed sequence
stays consumed even if downstream work fails. Stale, replayed or reordered messages cause no
effect. Machine responses have an independent increasing session sequence starting at 1; clients
reject non-increasing live response sequences. The SDK serializes controls and requests; use
waitMs:0 when interactive work is queued, so a long-poll does not race a send. After a lost
response on a still-live session, the SDK may resynchronize before its next call with at most
two scope-free read-only `capabilities` probes: the next client sequence, then the unresolved
current sequence only after a verified `REMOTE_REPLAY` refusal. Each probe is newly signed;
never replay a captured envelope or send during recovery. If both probes are refused, the
session is unusable and recovery requires a new signed session and ID-based receipt recovery. A
lost recovery response also ends sequence recovery; never make a third guess. A retried logical
request gets a fresh response envelope around its original receipt payload, correlated to the
retry ID in the current session. The log cursor governs historical ordering, not a reused
live-session counter. On reconnect, old signed log entries are accepted only as historical data
for the subscribed audience, never as a fresh command; the new signed subscribe response binds
their ordered IDs/digests and cursor to the current control ID/session.

Lost sequence state may use the bounded read-only recovery above while the session is still
live. Lost session state or exhausted sequence recovery requires a new signed session and
ID-based receipt recovery. It never permits a captured-envelope replay or automatic new send.
Expiry/revoke/stop is checked again at the effect fence. Bindings cannot waive those checks
because an edge previously accepted a signature.

A `browser` device on the door's own origin may instead hold a door session: after one signed
`session.open` over the door, remote sets a 256-bit random token as an HttpOnly, SameSite=Strict
cookie scoped to `Path=/r/<prefix>/x/`, the mount space (Secure on HTTPS), stores only its
SHA-256 and binds it to the device, grant revision and remote run. Browsers cannot set
authorization headers on WebSocket construction, so credential tokens never move into query
strings. Tabs share one browser cookie (the most recently issued token); Remote retains every
live token for the device. The cookie scopes the device context, while signed messages and
transports belong to individual sessions. The cookie is a carrier for the same authenticated
device context, not a second credential model: every request and upgrade rechecks grant,
revocation and expiry, and a state-changing operation still needs a fresh device signature over
its exact intent. Door sessions live only in the running remote and do not survive restart.
Revocation, grant expiry/revision change and stop end all device sessions and tokens.
`limits::SESSION_IDLE` is 12 hours without activity for a session that has had a transport;
`limits::SESSION_UNATTACHED_IDLE` is 60 seconds without activity for a session that has never
had one. Traffic and authenticated requests count as activity. Maintenance runs even without new
requests: the door's existing 100 ms event loop runs session maintenance at most once per
`limits::SESSION_MAINTENANCE_INTERVAL` (one second). Request paths still check session expiry
themselves; last-transport closure ends authority immediately. Maintenance prunes persisted
session rows only when an old run or expired end notice needs removal.

The SDK's `transportUrl(session, mountedWebSocketUrl)` adds exactly `?tmt-session=<sessionId>`
to a mounted WebSocket URL. This value is a non-secret identifier, never a credential: Remote
requires a live cookie, checks that the identified live session belongs to that cookie's device,
and otherwise returns the generic 404 before forwarding. Only upgrade requests may carry it;
ordinary page URLs cannot carry it and it must never enter browser navigation/history. Remote
strips it before forwarding to the extension, never logs it and never places it in Location or
Referer. The SDK requires the door-matching WebSocket scheme (http → ws, https → wss) on the
current door's mount space, without other query parameters or fragments. Existing tunnels
without the parameter attach to the cookie's session.

Opening another session leaves existing sessions and tunnels live. The last upgraded transport
closing (including tab close or a dropped transport while backgrounded/asleep) ends its session;
other tabs stay live. A failed upgrade is not an established transport. Ending closes remaining
tunnels while held work survives the session end, bound to the device grant. Only stop, revoke,
and grant expiry/revision change cancel held work. Dispatching and uncertain work retain their
original operation IDs, frozen intent and recovery behavior. A limit eviction uses
`REMOTE_SESSION_EVICTED` with the active positive `limit` and optional `settingsUrl`; the door
omits the URL until #1769 adds the Remote settings page. The SDK exposes `RefusalError.limit`
and optional `settingsUrl`, also on send/operation refused states. Absent/null URLs mean
command-only guidance; when present, Colab shows the settings link plus `tmt remote settings
sessions-per-device <n>`. The SDK normalizes the URL and requires the door's origin. Transport
close/idle expiry uses `REMOTE_SESSION_ENDED`. After verifying the device signature and current
grant, Remote can sign the distinct end reason for `limits::SESSION_END_NOTICE` (60 seconds);
afterward admission is the generic 404, also exposed by the SDK as `REMOTE_SESSION_ENDED`.
Reopening is silent: the page sends another signed `session.open` from its stored device key,
with no owner step or new pairing. The caller reattaches its transport using the new session; no
send is retried.

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
is `http://127.0.0.1:PORT/pair#CODE`, with 26 ungrouped base32 characters in the fragment.
A synchronous same-origin bootstrap captures and removes that fragment before loading any other
script. The page fetches the public current offer descriptor from `GET /sdk/pair-offer`; there is
one offer at a time and no additional trust boundary. The code never enters an HTTP request URL,
log or analytics.
The descriptor carries profile/binding, machine/window/offer UUIDs, address and a random 128-bit
server challenge (32 lowercase hex characters). No private key is included in a URL, ordinary
web-page DOM or log.

The command always prints the link. Browser opening defaults to on and uses Remote's own
`settings.json` `open` boolean (`tmt remote settings open on|off`). `--open` overrides the setting
and terminal, CI and SSH/display checks. Only `--no-open`, `--json` and a missing platform opener
suppress an explicit open. Without `--open`, no stdout TTY, CI, SSH without a display and Linux
without a display (except WSL) suppress opening. Opener failures warn without ending pairing.
Owner confirmation still requires an interactive terminal or `--json`.

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
or revoked through local management (`tmt remote devices`). The separate
[planned settings designation](#remote-settings-browser-authority) does not widen this agent grant.

Default scopes are `agents.read`, `status.read`, `check.read`, `talk` and `results.read`. The owner
may remove scopes at pairing. Future core capabilities do not silently become remotely callable;
a new scope needs a revision of this contract.

The shipped `tmt remote serve` runs in the foreground until Ctrl-C, SIGTERM or `tmt remote stop`,
with no default idle or hard deadline. Without `--port`, it reuses its last successfully bound
IPv4-loopback port; on first use it selects an unused port. If the remembered port is busy, serve
refuses with `REMOTE_PORT_BUSY`, names the port, tells the owner to stop the process using it to
retain browser pairing, and offers `--port <n>` to choose a new origin explicitly. Human output is
an `error:` line and a separate `hint:` line; `--json` joins them as `error.message`.
`--port 0` explicitly selects a random unused port; an explicit nonzero busy port refuses without
fallback. The actual bound port is remembered for the next run, including after an explicit
selection. Human startup output puts the full door URL on its own line with no trailing punctuation.
Remote schema 6 replaces each existing 32-hex-character prefix once with 16 lowercase RFC 4648
base32 characters (`a-z2-7`, 80 random bits), then keeps it stable. Old links and cookie paths
stop working. Machine IDs, keys, origin-bound pairings, grants and the remembered port survive;
browsers reopen their sessions under the current path without pairing again. An origin change
requires browser pairing at the new origin. Neither address nor route prefix is a credential.
Shutdown closes the door and cancels pending pairing and held operations. Held work survives
transport close, session idle expiry and eviction. After approval, any later session of the same
device can recover the result by operation ID. The existing per-device hold bound
(`budgets::HELDS`, 16) still applies, so held work cannot pile up past it when sessions end. The
approve prompt is unchanged: the owner is not told which tab. Approval publishes machine-signed
metadata to the device's shared journal even when no session is live; its cursor orders
historical envelopes, which do not open sessions. Revoke disables a device before
acknowledgment. No request/effect not yet fenced may succeed afterward. Already committed core
work is not undone; report it accurately. Restart issues a new window and session namespace;
grants survive. Unconfirmed held work is cancelled; dispatching/uncertain work recovers its
original operation, never becomes falsely unsent.

### Remote settings browser authority

**Planned for [#1769](https://github.com/pj-tmt/tmt/issues/1769), not implemented.**
This section defines Remote's settings and paired-device page; it does not claim a shipped
management SDK, route or browser capability. Until implementation lands, the local CLI remains
the settings/device management path. Current landing, pairing and error-page presentation adoption
does not implement this feature.

A paired channel owner-device is not automatically a settings administrator. Loopback, Host,
Origin, a route prefix, door cookie, display name or client-supplied owner flag cannot establish
administrative authority. A page or package cannot choose this policy, and Colab's device context
and content membership remain unchanged.

The machine owner may designate one already-paired `browser` device on the door's own origin,
using an optional choice inside the existing owner-only terminal pairing confirmation or later
owner-only local management. This exercises existing local authority; it is not a second pairing
or sign-in ceremony. No device is designated by default, on first pairing or on session open.
HTTP/SDK clients cannot directly create, remove, transfer or restore a designation.

Remote persists the designation in its owner-only state, bound to the machine, device UUID and
paired public key, not to a name or IP address. It survives a serve restart, but never restores a
Session: the browser must open a newly verified Session under its current live grant. Revoke,
re-pair or key change ends the designation; local replacement must clear the old designation
before issuing a replacement grant, and a new grant never inherits it. Expired or
disabled grants cannot exercise it. Rename retains the designated identity while the existing
grant-revision change ends old sessions. Local removal/replacement of the designation immediately
fences subsequent effects. Tabs using the same designated device key share that identity, not a
separate per-tab administrator role.

Browser management uses the existing signed-envelope admission: pinned machine/key/origin,
live Session and grant revision, timestamp, sequence/replay checks and bounded input. A cookie
admits only the page/transport. Every mutation rechecks the current designation and grant at the
effect fence; a cached capability or earlier authenticated read grants no authority. Local
designation removal, grant authority loss and browser effects share the ordering fence: if removal
wins, no later effect commits; if an effect already committed, report that outcome accurately.

The initial non-designated-device disposition is **read-only, not held**. Live paired devices may
read the bounded Remote settings/device projection; attempted settings writes, rename or revoke
receive a signed refusal before effects, with zero writes and zero held intents. Unpaired, revoked,
expired and extension-only principals receive no management inventory. The UI explains read-only
access and the local CLI path; it cannot promise pending approval. The local CLI remains usable
when no browser is designated. Existing default agent scopes, `direct`/`hold` modes, pairing expiry
and approvals are unchanged.

The management surface is typed and Remote-only: effective `open` and `sessions-per-device`
settings with their sources, bounded paired-device/session-count/activity metadata, and designated
browser actions to change those settings, rename or revoke a paired device. Reads expose no keys,
cookies, session tokens or core inventory. Settings use the CLI's existing store, validation and
locking; absent/default session cap 8 and `off`/unlimited semantics remain unchanged. Device
mutations reuse Remote's grant-revision, session cleanup and device-event behavior. Explicit
trusted UI actions authorize writes; presentation state does not. A self-rename/revoke can end the
caller Session after a committed effect; lost acknowledgment must not be reported as no effect.
Freeze each mutation's identity/input and recover unknown outcomes read-only without automatic
resend or a replacement operation ID.

Browser management never changes core settings, provider settings, drivers, extension installs or
argv, and never approves, rejects, cancels or releases held operations. `tmt remote approve` and
held-operation cancellation remain terminal-only. There is no generic config/command endpoint,
and a management designation cannot widen agent or extension authority. Typed wire/SDK payloads,
refusals and mutation recovery must be specified with the implementation before any such operation
is advertised as supported.

Implementation acceptance must prove same-loopback browsers with different keys cannot impersonate
the designated browser, designation is local-only and never automatic, non-designated writes cause
no effects or holds, and revoke/re-pair/key change, expiry, restart, rename and concurrent removal
obey the lifecycle and effect fence above. The actual settings/device page separately requires
product native/browser acceptance, UX review and publication evidence; a shared presentation
package or current-page adoption cannot satisfy those requirements.

### Browser entry

Human `serve` output links to the same-origin `/` browser entry. JSON readiness, status and SDK
`address` retain the protocol `/r/<prefix>` base; navigating there still gets the generic refusal.
The entry initially reads only this origin's saved browser record through the existing SDK owner.
Absent local data means no saved pairing; validated data means saved pairing with access unchecked.
Malformed or inaccessible data is unconfirmed. No descriptor, admission, inventory or work request
runs merely to display that state. Complete identity/origin/address/key-pin validation and the
existing non-extractable device-key consistency check precede an explicit connection attempt.

Connect makes one fresh `session.open` attempt and verifies its signed result against the
saved machine pin. A verified response confirms access at the displayed checked time, never
administrator designation. An opaque404 with an unchanged current descriptor recheck is still
not a signed refusal reason: access could not be verified, without inferring revocation, eviction
or another permanent cause. Only a specifically verified refusal may report Access refused. Transport, changed or
malformed descriptor and unverified response failures remain unconfirmed; a public descriptor
cannot replace a machine trust pin. Async results and new admission/recheck requests are fenced
to their page attempt, including after signing. Departure cannot undo an already dispatched request. No automatic
re-pair, grant repair, work resend or session admission on page load exists. Owner pairing still
uses the fragment-erasing bootstrap, fingerprint comparison and terminal confirmation.

### Local CLI discovery

Local extensions such as Colab discover or supervise Remote through these public CLI JSON
interfaces, never by reading Remote's private state. Discovery does not pair devices or grant
access. `origin` and `path` have no trailing slash; mounted Colab URLs are
`origin + path + "/x/colab/…"`.

`tmt remote status --json` is read-only. It emits exactly one of these documents and exits 0:

```text
{"running":true,"origin":"http://127.0.0.1:<port>","path":"/r/k7qxm4tz2pbwn6rh"}
```

```text
{"running":false,"lastPort":49152}
```

`lastPort` is the remembered nonzero integer port, or `null` if none has been recorded. It is
advisory, not a live address. Live `origin` and `path` come only from the running serve through its
owner-only control socket. Stopped inspection creates nothing and does not initialize or migrate
state. Unsafe or unreadable state, an unavailable lifecycle lease, and failure to reach or read a
live serve return the standard `{"error":{"code":"REMOTE_…","message":"…"}}` document with a
nonzero exit. A running serve that predates `status` or `stop` (alpha.1 answered
`REMOTE_INPUT_INVALID` "Unknown control operation."; later serves answer
`REMOTE_CONTROL_UNSUPPORTED`) yields `REMOTE_SERVE_OUTDATED` from either command: it must be
stopped by hand (Ctrl-C in its terminal) and started again, since `stop` cannot reach it. Status contains no other fields, secrets, cookies or device inventory.

`tmt remote stop --json` asks the running serve to shut down through its owner-only control
socket, then waits up to 40 seconds after acknowledgment for the lifecycle lease to be released
and the control socket removed. It uses the same shutdown flag as SIGTERM: listeners, active
sessions, tunnels and pending work close; stored pairings and grants remain. It emits exactly
`{"stopped":true}` after confirmation, or `{"running":false}` if no serve was running, and exits 0.
An unsafe/unresponsive control socket, a held lease without a reachable serve, or unconfirmed
shutdown returns the standard Remote error document and exits nonzero. Stop never identifies or
signals a process by PID and never starts or migrates stopped state. A concurrent successor may
make shutdown unconfirmed; stop does not send another request to shut down that successor.

A supervising extension can start `tmt remote serve --json` as its child. Serve emits one readiness
line on stdout after binding the door and control socket and recording the bound port:

```text
{"profile":"local-v1","binding":"loopback-http","state":"ready","address":"http://127.0.0.1:<port>/r/k7qxm4tz2pbwn6rh","machineId":"<uuid>","windowId":"<uuid>","startupCoreCalls":2}
```

The stable fields a supervisor reads are `state:"ready"` and `address`. This descriptor has no
separate `origin` or `path` fields: `address` is their concatenation, with no trailing slash.
`machineId` is the stable machine UUID and `windowId` is the UUID for this serve run. The other
fields identify the local profile, binding and two startup core calls. A startup failure emits the
standard error document and exits nonzero, including when the remembered port is busy. Serve remains
in the foreground. The supervisor stops only the child it started; attaching through status does not
give it ownership of an existing serve. Pairing remains explicit through `tmt remote pair`, and the
existing owner-only mount socket and grant rules apply to both attach and supervised-start use.

## Operations

Payloads for core API operations are the existing API envelope, decoded without rewriting input.
Remote narrows supported operations/authority before core calls.

| Logical operation                                                                               | Scope and public core mapping                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capabilities`                                                                                  | Signed paired discovery of supported subset, fixed suite and core bounds.                                                                                                                                                          |
| `agents.list`                                                                                   | `agents.read`; `tmt ls --json` projected to permitted UUID/name/presence and delivery status, without pane address/cwd/process/profile.                                                                                            |
| `identities.status`                                                                             | `status.read`; input restricted to permitted UUIDs. Self-report is not readiness or completion.                                                                                                                                    |
| `check`                                                                                         | `check.read`; one permitted agent, the bounded capture `tmt check --json` returns locally. Read-only; it never writes to a pane.                                                                                                   |
| `dispatch.create`                                                                               | `talk`; one permitted direct request recipient, anonymous core originator plus remote provenance (below). `direct` grants dispatch after admission; `hold` grants hold for local approval. No fan-out, room or announcement in v1. |
| `dispatch.show`, `operation.show`                                                               | `talk`; only journal-owned operation IDs; core immutable receipt or remote held state.                                                                                                                                             |
| `requests.show`, `result`                                                                       | `results.read`; any request the local `tmt result` can read, through the public API or `tmt result --json`.                                                                                                                        |
| `requests.list`, global `changes.cursor`, `references.resolve`, `rooms.roster`                  | Unsupported in v1; later projections need explicit scoped admission.                                                                                                                                                               |
| `notes.read`, `rooms.write`, `rooms.retire`                                                     | Unsupported in v1.                                                                                                                                                                                                                 |
| `identityHooks.*`, `skills.install`, `skills.remove`                                            | Never remotely callable; JSON consent cannot manufacture local lifecycle/install authority.                                                                                                                                        |
| reply/answer, X acknowledgment, core/provider config, pair, run/resume, approvals, installation | Never remotely callable. Result/log ack does not reply or acknowledge core work.                                                                                                                                                   |
| any other command, argv or shell                                                                | Never; there is no generic command endpoint.                                                                                                                                                                                       |

`agents.list`, `check`, `operation.show` and `result` are adapter helpers over ordinary public JSON
commands, not new core API operations. SDK `api(op,input)` cannot reach local management/argv
through an invented operation. The [planned Remote settings/device surface](#remote-settings-browser-authority)
is a typed Remote-only boundary, not permission to invoke core management. Until implemented, it
is not part of the supported subset above. The proposed read-only `delivery` projection belongs to core's
public `ls`/API JSON: `channel` (enrolled native channel ready), `paste` (ordinary paste
delivery), `not_ready` (enrolled but not ready, with core's local recovery hint) or `not_running`.
Remote forwards it unchanged and never infers it from panes; until core publishes that projection,
`agents.list` reports presence only. This projection is advisory, not an input-readiness lease;
the [public dispatch readiness and input-safety contract](extension-api.md#dispatch-readiness-and-input-safety)
owns the shipped guarantees and limits. Diagnostic `check` capture and presence never prove
safe input.

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

The wire-independent client boundary is `@tmt/remote-client`. The shipped browser entry exports
`operations(session, {timeoutMs?})`, accepting the existing verified `Session` and opening nothing.
Its `RemoteOperations` interface has `listAgents():Promise<RemoteAgent[]>`,
`send({operationId,agentId,message}):Promise<SendState>`, read-only
`operation(operationId):Promise<SendState>` and `result(requestId):Promise<ResultState>`.
`RemoteAgent` contains `id`, `name`, `presence` and optional core-owned `delivery`, forwarded
unchanged. Send/operation use SendState; result uses ResultState as defined below. No
selection/URL/title/note fields are reformatted: they are already inside the caller's frozen
message. The caller owns durable operation IDs and intent; the SDK stores no dispatch bytes in
IndexedDB. Explicit identical re-sends rebuild the same payload bytes.

Verified pre-effect refusals on send/operation return
`{state:"refused",operationId,reason}`, with the signed code `REMOTE_SCOPE_DENIED`,
`REMOTE_INPUT_INVALID`, `REMOTE_RATE_LIMITED`, `REMOTE_INTENT_CONFLICT`, `REMOTE_CLOSED`,
`REMOTE_SESSION_ENDED` or `REMOTE_SESSION_EVICTED`.
The generic pre-admission HTTP 404 maps to `REMOTE_SESSION_ENDED`; this is a session-ended
signal, not a signed response. The exported `RemoteRefusalCode` union contains those codes
plus `REMOTE_INPUT_TOO_LARGE`, `REMOTE_STATE_UNAVAILABLE` and `REMOTE_CORE_UNAVAILABLE`.
These other signed refusals on send/operation, and all refusals on listAgents/result, throw
exported `RefusalError {code:RemoteRefusalCode,retryAfterMs?,limit?,settingsUrl?}`. Its message is sanitized and at
most 256 UTF-8 bytes; retryAfterMs is an integer from 0 to 60000. An unavailable core/state error
never turns an adopted uncertain send into refused; the server's signed SendState owns that
classification. Callers branch on error class and code, never raw server messages.

Unknown outcomes throw exported `ClientError {code:ClientErrorCode,message,operationId?}`,
where `ClientErrorCode` is `transport_failure`, `timeout`, `unverifiable_response` or
`sequence_unavailable`. Send/operation retain the original `operationId`. Recover an unknown
outcome through read-only `operation(originalId)` on the existing session using the bounded
capabilities probes above. Signed refusals also leave sequence consumption ambiguous;
the next call first synchronizes with the same bounded probes, then performs the caller's call.
Exhausted recovery reports `sequence_unavailable`: this session can no longer be used, so the
caller reopens and then observes the original ID. Session-ended refusals
also require a caller-owned reopen; reopening adds a session and leaves other live tunnels intact. No implicit
send from recovery, automatic dispatch retry or replacement operation ID is permitted.

## Dispatch, hold and uncertainty

A new talk freezes one remote request/core operation UUID, permitted agent UUID and exact final
message before append. SDK persists them first. No edit under an existing operation ID.

Under a `direct` grant, a trusted device's admitted request goes straight to core: after
authentication, grant/scope/sequence checks and durable adoption, remote durably marks it
`dispatching` and calls existing `dispatch.create`. Remote adds no readiness or typing inference of
its own: core owns idempotency, acceptance, the one-shot advisory wake and its ordinary delivery
protections, including native-channel delivery, never pasting into an enrolled pane and the `!`
transport protection. The [public dispatch contract](extension-api.md#dispatch-readiness-and-input-safety)
states those protections and the legacy-pane/typing limits; remote never reads pane buffers to
infer readiness. The earlier [readiness issue](https://github.com/pj-tmt/tmt/issues/600) does not
gate this path or reinstate mandatory hold.

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

One narrow exception to the gesture rule is `tmt-ext-cert-v1`: the browser SDK may certify an
extension key without a gesture, for example on an extension's first use. The SDK takes the
extension name from the door's mount mapping for the calling page, never from a caller argument or a
visible path segment, so a page certifies keys only for its own extension. It runs only in
same-origin trusted extension code: `/sdk/remote-v1.js` loads only into trusted extension chrome,
and sandboxed opaque-origin renderer frames never load it or reach the device key. The SDK signs a
new certificate on every call, using the same device key and the current `issuedAtMs`; certificates
are not cached. Each certificate carries `issuedAtMs`, and verifiers enforce their own freshness
policy. The exception never covers operations or `session.open`, whose gesture and signing rules
are unchanged.

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
extension with the authenticated device context `{deviceId, kind, origin, name, publicKey,
owner:true, grantRevision}`, where `publicKey` is the grant's raw 32-byte Ed25519 device key as
canonical unpadded base64url, the only form remote forwards. A cloud edge attributes a non-owner
principal it authenticated for that extension as `{principal, owner:false}`; on the local door a
non-owner request arrives without a device context and the extension authenticates it (below).
Extensions learn the current principal from the forwarded device context on each request
(`tmt-device-context` on the local door). A page that needs the principal asks its own extension
backend, as Colab does through `/api/session`; the browser SDK does not expose the principal.
Extensions also receive the device events defined below (for example to drop a device's extension
key or rotate page epochs). An extension that needs
its own keys generates them on the device and asks the device key to certify them: the device signs
LP(`tmt-ext-cert-v1`) || LP(extension) || LP(purpose) || LP(raw extension public key) || LP(decimal
issuedAtMs), where extension is its mounted name (a lowercase ASCII letter, then lowercase letters,
digits or hyphens, at most 32 bytes) and purpose is `sign` or `enc`. The certificate binds an
extension key to a device; it grants no remote authority by itself, and extension cryptography stays
owned by the extension. The extension verifies it against the `publicKey` of a device context with
the matching `deviceId`.
Revocation does not change the mathematical validity of a retained certificate signature;
the certificate grants no live authority. A revoked grant loses its forwarded owner-device
context and cannot reopen a door session. Extensions learn revocation from the durable
`device.revoked` tombstones below and own their device-bound key cleanup and revision handling.

**Device events (local).** Remote delivers one channel to each mounted extension using ordinary
HTTP/1.1 `POST /.tmt/remote/device-events` on that extension's existing owner-only
`<dataRoot>/<extension>/door.sock`. This path is extension-local, outside its browser routes.
Mounts refuse the entire `/.tmt` subtree, including
`<prefix>/x/<extension>/.tmt/remote/device-events`, with 404 before forwarding. The callback carries
`tmt-device-event: 1`, `Content-Type: application/json`, one `Content-Length`, `Connection: close`
and the usual remote-set `tmt-mount`. Browser request headers never forward `tmt-device-event`;
extensions trust a callback only on the owner-only socket with that exact remote-only header and
reserved path. The header alone is not authority on a browser-facing transport. Remote validates
the socket and its directory's ownership, private permissions and no-symlink rule exactly as for
route mounting. No browser route, device session, signature, credential or new listener is added.

The body is exactly one strict UTF-8 JSON object, with no duplicate or unknown fields:

- `{"type":"device.revoked","deviceId":"<device UUID>","grantRevision":N}`.
- `{"type":"device.renamed","deviceId":"<device UUID>","grantRevision":N,"name":"<current name>"}`.

`deviceId` is the grant's `clientId`, a canonical lowercase UUIDv4. `grantRevision` is the positive
JSON integer grant revision (at most 2^53-1). Names follow the existing 1–64 nonblank UTF-8 byte,
no-control rule. Extensions reject unknown types, extra or missing fields and invalid values;
`name` is present only for `device.renamed`. These events convey device state, never agent work
or extension membership authority.

Delivery is level-triggered from durable grants, without a journal or cursor. Each sweep sends a
revoked tombstone for every disabled grant and the current name for every other grant, including
the initial name at revision 1. Intermediate renames may coalesce. Tombstones are retained, so a
revocation while an extension or remote is stopped is replayed on recovery. Remote starts with a
full sweep and repeats successful sweeps every five seconds, also covering a restarted extension
at the same socket. A committed revoke or changed rename wakes the worker immediately. Each
request has a one-second total connect/write/reply-head deadline; a joined worker performs socket
I/O outside the grant lock and local mutation acknowledgment.

Only a valid 2xx response acknowledges an event. Every other status, missing or unsafe socket,
connection failure, timeout or malformed response retries current durable state with bounded
backoff (1, 2, 4, 8, 16, then at most 30 seconds); a committed mutation wakes that wait too.
An extension acknowledges only after applying or durably recording the event. Lost replies and
successful periodic replay can duplicate events. Extension consumers therefore apply revisions
idempotently per `deviceId`: an equal revision causes no repeated effects, and an older event
never undoes a newer one. A revoke at revision N wins over any rename at a lower revision.
Consumers retain the applied revision with device-bound state; they must not rotate epochs again
for a replayed revoke. Delivery is asynchronous: remote revokes authority and ends sessions before
acknowledging the local command, while extension cleanup may still be awaiting recovery. A rename
of a revoked device refuses, so its tombstone cannot become a name event.

`tmt remote devices rename <clientId> <name>` changes presentation only, preserving the device ID,
key and authority. A changed name advances the grant revision and ends old-revision sessions and
tunnels; the device silently reopens with signed `session.open`. Repeating the current name is a
no-op with the same revision. Both rename and revoke work through serve's control socket when
running, or directly under the serve lock when stopped.

**Route mounting.** The door mounts each enabled, owner-installed extension under
`/r/<prefix>/x/<extension>/`, inside the machine's route prefix, forwarding HTTP requests, static
assets and WebSocket upgrades to the extension process over an owner-only local socket in the
extension's data subtree, together with the device context. Remote owns Host, Origin and CSRF
admission, framing and connection/body limits; the extension owns its responses, content security
policy and headers. All mounted extensions share one browser origin and therefore one browser trust
domain; mounting is limited to owner-installed extensions, and untrusted content renders only in
sandboxed opaque-origin frames. There is no mount space at the door root: `/x/` answers 404 without
a redirect; the explicit Colab short alias below is the only root redirect into the mount space.
Page URLs contain the prefix, so a mounted reply keeps
the door's `Referrer-Policy: no-referrer` unless it narrows it to `same-origin`; any other policy is
dropped. The door is loopback-only; other machines reach it only through cloud backends. Upgraded
WebSocket tunnels do not use the door's edge connections, so open pages cannot starve remote
operations, pairing or page loads. Each mounted extension has its own tunnel cap and idle bound: a
tunnel with no bytes in either direction for the idle bound is closed, and an upgrade beyond the cap
is refused with HTTP 503 and `Retry-After` before it reaches the extension. An extension keeps one
WebSocket per page and reconnects after an idle close or a refusal.

On a mounted WebSocket upgrade, the door forwards the client's `Sec-WebSocket-Protocol` value
unchanged, including every offered subprotocol in its original order, and returns the extension's
selected `Sec-WebSocket-Protocol` value in its `101` response unchanged. These values may carry
extension-issued bearer tokens; the door never logs, audits or persists them. On every mounted
request, including upgrades, client-supplied `tmt-device-context`, `tmt-device-event` and `tmt-mount`
headers are stripped. Only the door sets these headers: `tmt-mount` identifies the actual mount,
`tmt-device-context` is added only for an authenticated owner-device session, and `tmt-device-event`
is reserved for the local callback above.

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

**Backends (Proposal).** Extensions use the single
[backend declaration contract](#proposal-extension-backend-declarations). Remote composes those
resources with its own through [authorized deploy](#proposal-authorized-deploy).

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
stores under the data root), then Firestore, then Cloudflare. The following cloud bindings,
encryption profile, backend declarations and deploy behavior are **Proposal, not implemented**.
Specifying them does not enable a cloud backend; implementation and the proposed
[cloud conformance gates](#proposal-cloud-conformance) are required before use.

| Backend      | Mapping                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `local`      | [Loopback HTTP](#transport-binding-loopback-http), mounted extension routes and WebSocket relay; state under the data root.                                      |
| `firestore`  | **Proposal:** authenticated, bounded encrypted inbox documents; owner-machine responses; snapshot listeners and device-scoped checkpoints.                       |
| `cloudflare` | **Proposal:** Worker admission before per-machine Durable Object storage; per-namespace relay objects; bounded WebSocket delivery and device-scoped checkpoints. |

### Proposal: shared cloud admission

A deployment has a stable Remote-generated UUID and separate operation and extension namespace
roots. Only paired owner devices reach the operation root. Non-owner extension principals reach
only their admitted extension namespaces, as defined by the
[extension channel API](#extension-channel-api); neither an extension credential nor page membership
grants operation access.

The owner machine connects outward using a deployment credential restricted to its machine and
connector role. The edge authenticates that connector separately from device traffic; no device or
extension route can publish admission records or machine responses. Neither cloud service holds the machine
encryption private key or a device private key, calls core, or owns Remote's authoritative journal.

Remote publishes an edge admission projection containing the machine/window IDs, device ID,
public key, kind, origin, profile, grant revision, disabled state and expiry. Only the machine
connector writes it. Projection updates never decrease revisions; a disabled tombstone cannot become live
again. A projection also has a provider-time lease of at most 60 seconds, renewed automatically
while the machine is connected. Missing, expired or conflicting projections refuse admission.
Stop, revocation or narrowing ends the applicable edge connection/subscription when observed.
Propagation can lag; edge acceptance never substitutes for a live local grant.

Before adoption, the machine authenticates the cloud carrier, decrypts it and verifies the enclosed
[signed envelope](#signed-envelopes), including exact header agreement. It applies the existing
grant, timestamp, session, sequence, scope, operation and replay admission under its authority lock,
then the existing [durable adoption](#durable-log-append-subscribe-and-ack) and
[effect fence](#dispatch-hold-and-uncertainty). A stale projection, valid provider credential,
storage notification or encrypted upload cannot waive any check. Disconnected machines cannot
accept new operations; expired session/time evidence must be recovered, never turned into an
offline send queue.

The edge applies the existing [abuse budgets](#browser-use-and-audit) as admission ceilings,
carrier byte bounds and bounded temporary storage before forwarding; the machine keeps the
persistent authoritative counters. Physical chunks do not allocate new logical-call budgets. An
upload or document receipt is transport evidence only. Operation acceptance requires the machine's
authenticated response. Backends cannot mint log cursors or acknowledge the machine stream: checkpoints carry the encrypted signed `ack` control,
and listeners/WebSockets carry responses to encrypted signed `subscribe` controls. Catch-up,
retention, isolation and no-resend behavior keep their existing definition owners. Switching
backends preserves logical IDs and exact intent, never creates a new operation. Idempotency
compares the decrypted logical operation and exact payload bytes, not randomized ciphertext.

### Proposal: Firestore Auth and Rules

Remote deploys an HTTPS admission service, Firebase Auth and default-deny Firestore Rules. A paired
device obtains a Remote-scoped custom token through a possession challenge, then uses Firebase Auth
to access the deployment; this is automatic transport authentication, without another user sign-in
or pairing. The service pins the device key from the live projection and issues a single-use
128-bit random challenge, valid for at most 60 seconds. The device signs LP(`tmt-cloud-auth-v1`) ||
LP(deploymentId) || LP(machineId) || LP(windowId) || LP(clientId) || LP(origin) || LP(raw challenge).
The service checks the signature and matching live projection before consuming the challenge.
Browser HTTP Origin must equal the pinned origin; CLI traffic has no HTTP Origin and signs `cli`.
Failed challenges disclose no machine/device inventory and obey the unauthenticated abuse bound.

The Auth UID is the device's `clientId`; trusted token claims bind deployment, machine and grant
revision. The issuer alone supplies those claims. Rules recheck the corresponding live admission
document on every request, matching the authenticated UID and claims; a refreshed token, cached
claim or ordinary Firebase user cannot restore expired/revoked access. Clients cannot write Auth
claims, admission projections, upload permits or response records. Rules do not verify Ed25519 or
HTTP Origin; the service checks device proof, and the machine independently checks every envelope.

Before each upload, the admission service verifies the signed carrier header, encapsulation,
ciphertext length/digest and live projection, and reserves quota. It creates a permit scoped to that
exact carrier digest and device with a 60-second upload deadline. A repeated permit request for the
same carrier returns that permit without allocating storage again. The permit authorizes only a
bounded create-only manifest and its fixed number of chunks, not operation adoption. Quota includes
incomplete uploads; refusing or expiring one creates no hidden queue.

Rules admit only that device's permitted machine inbox path, exact permit fields, chunk indexes and
sizes, and server-time creation/expiry fields. Chunks contain at most 256 KiB of decoded ciphertext;
the manifest has at most 8 KiB of metadata and is created last. The permit fixes the total decoded
length and chunk count, using the effective carrier bound below. The machine assembles only a
complete manifest, checks every chunk and the signed ciphertext digest, and refuses truncation,
extra chunks or conflicting bytes before decryption. This chunking is transport framing, never a
second logical operation. Provider document limits must also pass deployment validation.

Only the scoped connector writes machine responses; clients read only their own response path.
Responses use the same bounded manifest/chunk framing. Queries are constrained to that device's
path, never a deployment inventory; expired/incomplete uploads are reclaimed within the temporary
storage quota, with logical expiry enforced before access even when provider TTL deletion lags.
Snapshot listeners deliver ciphertext, not authoritative operation state. Device checkpoint writes
are bounded create-only encrypted controls for machine verification, not edits to a trusted cursor. Extension collections
and blob paths have their own declared admission and no overlap with these roots.

[Server SDK access bypasses Rules](https://firebase.google.com/docs/firestore/security/rules-conditions),
so the admission service and connector also require separately scoped IAM credentials. No Admin
credential reaches a client. The deploy plan accounts for required service access rather than
claiming Rules protect privileged code. A refresh failure stops transport; reconnect recovers
owned state through a fresh signed session, without resending work.

### Proposal: Cloudflare Worker and Durable Objects

Only the Worker is publicly reachable. It checks exact route/framing, byte and rate bounds,
browser Origin (absent for `cli`), live admission projection and the device signature over the
carrier before forwarding. Unknown, unsigned, expired or revoked operation traffic receives the
generic pre-auth refusal; syntactically valid ciphertext alone grants nothing. Verification must
conform to the existing strict Ed25519 profile; native Worker crypto availability is not proof of
its strict-refusal semantics.

A per-machine Durable Object isolates bounded transport inbox/outbox state and per-device
connections. Connector authentication is restricted to that machine and is separate from paired
device proof; only the connector publishes admission projections and responses. The object never
decides operation authority, decrypts operation payloads or executes core. HTTP/WebSocket success
means delivery only. Connection admission grants no session or scope, and each application frame
passes the same checks. Missing projection leases, quota exhaustion and storage failures refuse
rather than queue invisibly. Invalidating a device projection closes its subscriptions.

Relay objects and blob bindings are separate per declared extension namespace. The
[extension admission hook](#extension-channel-api) applies to append, subscription, objects and
awareness before relay effects; an unavailable hook refuses. A shared-page user cannot reach the
operation object through relay credentials, a namespace alias or a Worker fragment.

### Proposal: end-to-end operation encryption

The fixed carrier profile is `cloud-hpke-v1`, using
[RFC 9180](https://www.rfc-editor.org/rfc/rfc9180.html) base mode: DHKEM(P-256, HKDF-SHA256)
(`0x0010`), HKDF-SHA256 (`0x0001`), AES-128-GCM (`0x0001`). There is no negotiation, plaintext
fallback or Ed25519-to-ECDH secret conversion. The existing Ed25519 machine key remains the pinned
authentication identity; Remote generates a distinct P-256 machine encryption key and keeps its
private material under its existing owner-only state policy. Extension content keys remain
extension-owned.

The public descriptor is exactly `{version,profile,machineId,keyId,revision,publicKey,notBeforeMs,
expiresAtMs,signature}`. Version is 1, profile is `cloud-hpke-v1`, keyId is a Remote-generated
UUID, revision is positive and validity times are JSON integers with notBeforeMs < expiresAtMs.
PublicKey is the RFC 9180 uncompressed 65-byte P-256 point, strictly validated and encoded using
the existing base64url rules. The machine signs LP(`tmt-cloud-key-v1`) followed by LP of each
preceding descriptor value in field order, using decimal integers and the raw public-key bytes.
The device verifies that signature with the machine key pinned by
[pairing](#pairing), machine/profile/time agreement and a nondecreasing persisted revision before
using the encryption key. Equal revisions must have identical descriptor bytes. A backend cannot
replace the trust root or authorize a key downgrade.

The machine durably publishes a new key/revision before accepting traffic for it and refuses
retired key IDs. It retains old decryption material only for bounded already-adopted recovery,
never to accept expired new traffic. Lost or compromised authentication keys require pairing
again; an authenticated encryption-key rotation does not create another device identity or grant.
Withholding a newer descriptor can cause refusal, never a bypass. This static recipient-key
profile makes no forward-secrecy claim after recipient-key compromise.

A request/control carrier is exactly `{version,profile,keyId,header,enc,ciphertext,signature}`.
Version/profile are 1/`cloud-hpke-v1`. Header contains exactly the enclosed envelope's fields
from version through operation, excluding payload and signature. Let `H` be that envelope's
[canonical signature input](#signed-envelopes) through operation, without its final payload digest;
all existing field/encoding rules apply. `enc` is the 65-byte RFC encapsulated P-256 public key,
and `ciphertext` includes the 16-byte AEAD tag; both use strict base64url. Plaintext is the exact
complete signed envelope JSON, retaining the decoded payload bytes without reserialization.

Define `B = LP("tmt-cloud-carrier-v1") || LP("1") || LP("cloud-hpke-v1") || LP(keyId) || LP(H)`.
HPKE info is B; request AAD is `B || LP(raw enc)`. Use a fresh encapsulation/context for each
new carrier and exactly one HPKE Seal at sequence zero. The device additionally signs
`B || LP(raw enc) || LP(decimal ciphertext byte length) || LP(SHA-256(raw ciphertext))` with its
existing Ed25519 key. This outer proof lets the edge authenticate before decryption; the machine
checks it independently and requires every decrypted header field to match. Both the carrier and
the inner signature must pass. Cloud encryption covers controls, reads, sends, results and check
captures, not just message text; the unpaired enrollment form remains the
[pairing](#pairing) protocol, never an operation.

A response carrier has the same fields plus `requestDigest`, the base64url SHA-256 of the
request's outer signature input; it echoes that request's keyId and enc. Its header describes the
machine-signed response inside. Let `R` be its B value, and
`E = LP("tmt-cloud-response-v1") || LP(raw requestDigest)`. From the request's HPKE context derive
`responseKey = Export(E || LP("key"), 16)` and
`responseNonce = Export(E || LP("nonce"), 12)`. AES-128-GCM seals the exact complete signed
response with AAD `R || LP(raw enc) || LP(raw requestDigest)`. The machine's outer signature
covers that AAD followed by LP(decimal ciphertext byte length) and LP(SHA-256(raw ciphertext)).
The client verifies the pinned machine signature, request binding, AEAD and enclosed signed
response before interpreting any state.

Each context produces exactly one response ciphertext, frozen before publication. Identical
transport retransmission returns only those frozen bytes; a changed response, recovered receipt or
later notification uses a fresh request/control context. A lost context requires fresh signed
session/ID-based recovery, never reuse of a key/nonce with new plaintext. Asynchronous state arrives
inside responses to fresh subscribe controls; log entries keep their machine signatures.
Clients retain context secrets only for the bounded in-flight request and discard them on
completion or abandoned observation. The machine freezes the encrypted response durably before
publishing it and never regenerates different bytes under that context after a crash. Transport
recovery expires within the existing journal retention/capacity; expired ciphertext is unavailable,
not permission to resend. Core final retention and journal ownership are unchanged.

For an advertised maximum signed-envelope JSON length N, ciphertext is at most N + 16 decoded
bytes. A carrier reserves 8 KiB for its bounded header/metadata and has a 65-byte decoded
encapsulation; the conservative wire cap is
`4 * ceil((N + 16) / 3) + 4 * ceil(65 / 3) + 8192`. The binding advertises N for
each request/response class before allocating or uploading; insufficient provider capacity refuses.
Batch responses retain the existing bounded subscription policy. Ciphertext chunks never change
this total cap. IDs, operation names, origin, sizes, timing, key revisions and traffic remain
visible; content secrecy is not metadata secrecy or protection from a compromised app host.

### Proposal: extension backend declarations

An enabled owner-installed extension supplies a strict version-1 declaration per backend:
`{version,extension,backend,resources,admission}`. Extension uses the existing mounted-name
grammar; backend is `local`, `firestore` or `cloudflare`. A declaration has at most 64 resources
and 64 KiB of UTF-8 JSON. Remote reads only installed, owner-approved artifacts, never downloads or
executes a declaration-carried command.

Each resource is `{name,kind,path,limits,ttlField,indexes}`. Name uses the extension-name grammar
and is unique in the declaration; kind is `log`, `checkpoint`, `blob` or `awareness`. Path is a
relative namespace path: no absolute path, empty/dot segment, escape or wildcard outside that extension's root. Limits specify
positive JSON integers `maxObjectBytes`, `maxNamespaceBytes` and `maxEntries`, bounded by Remote's
advertised backend limits. TtlField is null or a declared expiry field; indexes contain at most 64
`{field,direction}` entries, where direction is `asc` or `desc` and field is declared by the
resource. Awareness is ephemeral, with no stored objects, TTL field or indexes. Page membership,
epoch/writer checks and expiry policy remain extension-owned; declarations cannot redefine grants,
signed operations or the machine journal.

Admission is `{artifact,digest,entryPoint}`: an installed relative artifact path, its lowercase
SHA-256 hex digest and its namespace entry point; `local` names the extension's existing admission
hook. Remote composes each artifact only with namespace-scoped resource/context capabilities;
operation credentials, projections and response publication are unavailable to fragments. Reject
cross-root reads/writes, reserved operation routes, catch-all grants, arbitrary IAM roles,
overlapping resources or a composition that cannot enforce this boundary. The same
[principal and relay rules](#extension-channel-api) apply; a fragment cannot confer owner-device
context. Unsupported resource kinds, admission requirements, quotas or provider limits fail the
whole plan before provisioning. Routes/assets may be mounted only through the declared namespace,
never an extension-selected public operation URL. Extensions requiring changes submit a new
declaration, not a second backend/sign-in/deploy owner.

### Proposal: authorized deploy

`tmt remote deploy <backend>` composes Remote's resources and all enabled extension declarations
into one deployment in the owner's account. Before effects it shows the exact account/project,
deployment ID, region, enabled extensions and declaration digests, resources, roles/bindings,
Rules/Worker changes, quotas/expiry/index policy and destructive changes. The owner explicitly
authorizes that plan for that target account at deploy time. Existing pairing, start, account login
or extension installation is not authorization to create or change real account resources.

Firestore provisions the admission/token service and its scoped service role, Auth configuration,
operation admission/permit/inbox/outbox/checkpoint collections, composed Rules, indexes/TTL and only
declared extension/blob resources. Cloudflare provisions the Worker, per-machine and declared
per-namespace Durable Object bindings, scoped connector credentials and only declared blob resources.
Private device/machine encryption material is never provisioned to either provider. A backend
with missing required services/permissions is refused before publishing an active binding.

Remote records deployment identity, plan/declaration digests and per-resource outcomes durably in
its own state. Validate all declarations first; publish a usable binding only after the complete
authorized plan succeeds. A partial failure reports completed/pending resources and leaves new
routes disabled. Exact-plan retry resumes owned provisioning by stable resource identity, without
duplicating resources or sending agent work. A changed plan/account needs new authorization.
Never overwrite or delete unrelated account resources; deletion/data loss requires explicit
authorization in that concrete plan. Preserve an existing deployment on failed upgrade or report
its actual partial availability; never claim provider rollback restored data.

[Start and pair](#provisioning-on-start-and-pair) automatically prepare the namespaces/bridge of
already authorized local or deployed resources and publish the device admission projection. They
do not authorize first deployment, new account resources or expanded provider permissions. Adding
an enabled extension with undeployed requirements leaves that cloud extension unavailable until
an authorized deploy; local availability and existing operation authority are unchanged.

### Proposal: cloud conformance

Before cloud implementation acceptance, independent HPKE and application byte vectors plus
browser/native interoperability must cover key descriptors, header/AAD/digest binding, strict
point/base64/signature refusal, response-key derivation and nonce reuse prevention. Mutating one
condition from a valid control must refuse before journal/core effects. No test may infer
operation acceptance from provider status or a stored manifest.

Firestore emulator and local workerd/Miniflare cases must prove unknown/cross-machine/cross-device
and extension-only principals denied, clients unable to alter grants/receipts, live-lease and
revocation fences, complete-only chunk assembly, bounded abandoned uploads, quota/storage failure,
subscription/checkpoint isolation and lost-response recovery without new effects. Test a stale
edge that admits a locally revoked device: the machine still performs no effect. Backend migration
retains the same operation ID and exact intent. Declaration collisions/privilege expansion and
unapproved or partly failed deploy plans publish no new usable binding. These are proposed gates,
not evidence of implemented cloud support.

## Provisioning on start and pair

`tmt remote start` mounts every enabled extension and prepares its relay namespaces and owner-machine
bridge, and pairing a device makes it known to every mounted extension through the device context.
No extension adds its own sign-in, pairing, bridge enrollment or machine grant step. An extension
asks for an extension key certificate silently on first use.

## Transport binding: `loopback-http`

The door address is `http://127.0.0.1:<port>/`. Remote operations live under `/r/<16 lowercase
base32>/`, a stable per-machine prefix that is not a credential; extensions live under its
`x/<extension>/` subtree. Under the prefix, exactly the four POST operation routes below and that
subtree exist, and no path reaches both. Only the running remote binds IPv4 loopback by default.
Require exact numeric Host/bound port, reject forwarded-host authority, ambient bearer
authentication, duplicate framing headers, queries/fragments, percent escapes/dot segments or extra
slashes on remote routes. Cookies are admitted only as the door session described under signed
envelopes. No unauthenticated GET inventory.

| HTTP route        | Message-layer action                                                                                   |
| ----------------- | ------------------------------------------------------------------------------------------------------ |
| `POST /append`    | One signed request or session.open control envelope; authenticate before durable adoption/core access. |
| `POST /subscribe` | One signed subscribe control; bounded long-poll, signed response batch/cursor. No SSE.                 |
| `POST /ack`       | One signed ack control; no core attention mutation.                                                    |
| `POST /pair`      | Enrollment fields/proofs for an already machine-opened local offer; no client-created offer.           |

`GET /p/<id>` at the door root accepts exactly 4–64 ASCII characters from `[0-9A-Za-z_-]`
and returns `302` with the same-origin `Location: /r/<prefix>/x/colab/p/<id>`,
`Cache-Control: no-store` and `Referrer-Policy: no-referrer`. It permits navigation without Origin
or the door's exact Origin; a cross-origin Origin receives the generic HTML 403. Other alias
shapes or methods receive the generic HTML 404 after door framing admission. The alias reads no
cookie, sets no cookie and grants no authority; the session cookie remains scoped to
`Path=/r/<prefix>/x/`. Colab owns short-ID resolution inside its mount.

Browser assets live at the door root, disjoint from the route prefix, under the same Host, path and
framing rules. `GET /pair` serves the pairing page with `default-src 'none'; script-src 'self';
style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
`GET /sdk/pair.js` serves the synchronous fragment-erasing bootstrap, which then loads the SDK.
`GET /sdk/pair-offer` returns only the current unconfirmed offer's
`{profile, binding, machineId, windowId, offerId, address, serverChallenge}` descriptor with `no-store`; absent, expired, cancelled
or confirmed offers return the generic JSON 404. Cross-origin loads refuse. The code remains
exclusively in the captured fragment. Old `/pair/<descriptor>` paths refuse; no compatibility route
remains. `GET /` serves a static pairing landing page; malformed pairing-page paths receive generic
HTML errors with their refusal status. These pages load only the embedded same-origin
`GET /sdk/pages.css` stylesheet, using the shared design tokens (the same `header` group as Colab's header) and system font
fallbacks without network fonts. Protocol refusals below `/r/` remain JSON. `GET /sdk/remote-v1.js` serves the device SDK as
`text/javascript; charset=utf-8` with `nosniff`; the path names the SDK interface version, not a
build, so it is not cached across upgrades. `POST /sdk/mount` takes exactly `{path}` from a page on
the door's own origin and answers `{machineId, windowId, address, extension, mount}`: this run's
identity and the mounted extension that contains `path` in the door's own mount mapping, or null. It
scopes honest use, such as which extension a page certifies keys for, and is not a security
boundary: mounted extensions share one browser trust domain and only owner-installed extensions
mount. None of these routes reads the door cookie or grants authority.

Route action and envelope kind/operation must agree. Body is one UTF-8 JSON document, Content-Type
application/json, one Content-Length, no transfer encoding, at most one request/connection;
Connection: close. Header/body acquisition times out within five seconds; pairing max 16 KiB,
headers max 8 KiB. The four POST message-layer routes in the table are suffixes of the remote route prefix. Envelope
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
removed and envelope bytes are unchanged. The pairing ceremony is implemented through
`tmt remote pair` and `/pair`: one offer per run, the owner's terminal confirmation, the default
grant and the receipt with `serverProof` (#1039). Door sessions (`session.open` on `/append`, the
mount-space cookie and the device context it carries to mounts) and `tmt remote devices` list and
revoke are implemented (#1039). Local rename and level-triggered device events over mounted
extension sockets are implemented (#1100). The device SDK in `remote-client` (non-extractable WebCrypto device
key, pairing client with `serverProof` verification, `session.open` client and `tmt-ext-cert-v1`
certification) and the Rust and Python certificate vectors are implemented, as are the remote-served
pairing page, `/sdk/remote-v1.js` and `/sdk/mount` (#1039).

Colab's working loopback door, sign-in and sync transport code relocates into `tmt-remote` as the
local door, device sign-in and relay where it meets this contract, rather than being rewritten. The
relocated door and `/r/<prefix>/x/<extension>/` route mounting are implemented (#1039), with only
colab allowlisted. Colab serves only its owner-only socket and has no door of its own; an owner
session's device context reaches it through the mount (#1039).

Core recognizes Remote through the shared native installer. Installation does
not start `serve`, pair a device, grant operation authority or change
`<dataRoot>/remote/` state. Archive publication remains separately gated.

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
