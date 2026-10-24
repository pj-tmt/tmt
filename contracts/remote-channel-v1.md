# Remote channel protocol v1

**Implementation status:** the local door and its signed agent operations are shipped; Colab page
sharing over Firestore (native sync and the hosted browser client) is shipped. Cloud agent operations
through Firestore have native invite/join and a carrier driver with emulator-only qualification; public
Core delivery activation remains pending. Cloudflare remains proposed. Native encryption and automatic key renewal enable no access by themselves.
Shipped means published code with fixture/emulator proof; real-account acceptance remains open.
This document owns host-driver authentication, local device identity/grants, wire values, operations,
the extension channel API and backend bindings. Remote owns this contract; tmt-lead reviews changed core contracts and cross-squad
seams before runtime/SDK code. The
[security design](https://github.com/wkh237/tmt/issues/478#issuecomment-5910827518),
[M1 ruling](https://github.com/wkh237/tmt/issues/478#issuecomment-5911118171) and
[transport-layer decision](https://github.com/wkh237/tmt/issues/478#issuecomment-5911165578) are
its inputs. The owner's 2026-10-02 decisions on [#955](https://github.com/wkh237/tmt/issues/955)
supersede the M1 rules for mandatory hold, per-agent allowlists, own-results-only reads, forced
grant expiry and serve windows.

## Owners and layers

Remote owns its listeners, provider transports and private state. Core owns its public dispatch,
request and input-safety semantics independently of Remote authentication. Remote uses only the
fixed public Core APIs/JSON commands: Core has no dependency on Remote device keys, pairing or
trust grants, and no Remote base-trust check is added to Core. Its existing safety checks remain.
Colab owns page/epoch/role/writer admission and content cryptography through the
[extension channel API](#extension-channel-api); content verification grants no machine access.

## Host driver authentication

Each host driver has exactly one authentication gate; gates never stack. Device trust is a
local-driver concern, not a shared Remote base credential required by every backend. Here driver
means a Remote hosting/transport backend, not Core's host adapter.

| Driver responsibility | Local (`loopback-http`, shipped)                                      | Firestore page sharing (shipped boundary)                                            |
| --------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Establish identity    | Pair the device key to the machine and exact origin.                  | Firebase Auth supplies the authenticated uid.                                        |
| Authorize             | Remote's persisted device grant, rechecked at the local effect fence. | Deployed Security Rules evaluate `request.auth` and the extension's declared policy. |
| Link                  | The local pairing offer/link and door entry.                          | The verified public Remote descriptor and the extension's share link.                |
| Revoke                | Disable the local grant and end its sessions.                         | Enforce identity/policy loss through Firebase Auth and deployed Rules.               |
| Progress detection    | Observe admitted pairing/session/operation state.                     | Observe Auth and Rules/transport outcomes through the existing bounded client.       |

These are driver obligations, not a new code trait, wire schema or shipped join wizard. Local
keeps its device key, origin binding, Host checks and every effect/revocation fence below.
Firestore's only authentication/authorization gate is Firebase Auth plus deployed Security Rules:
Remote adds no pairing, device signature or Remote-side admission service/hook in front of Firestore.
Public-config/declaration validation, bounds, budget guards and no-retry uncertainty remain; they
are not a second device-trust gate. A Firebase uid is not a paired local device or an operations grant.
Colab's existing content, page, epoch, role and writer checks remain application-owned on
publication and read. They add no Remote device proof to Firestore and grant no machine access.

### Proposal: deploy-selected sign-in and authorization policy

**Target design, not shipped (#3861).** Deployment selects sign-in providers and authorization
policy. The approved provider set is anonymous, Google, Email link (passwordless) and GitHub.
GitHub requires an owner-created OAuth app and console entry of its client ID/secret; the future
deploy stops with a fixed owner-console instruction, like the existing Google owner step. No secret
belongs in a share link. Policies are link holders or an email allow-list; company-domain policy is
outside v1. Invite links are multi-use by default (as today), with optional single-use at creation.
The provider/policy inputs, single-use option and machine-join flow remain proposals.
The current [anonymous bootstrap](#hosted-anonymous-bootstrap) and Rules remain unchanged.
[Native automatic opens](#native-automatic-opens) use the local driver only; manual
`tmt remote pair` retains its terminal ceremony.

## Transport binding: `loopback-http`

**Shipped local driver.** The door, threat model, pairing, device grants and signed-message
rules in this section belong only to this driver; they are not a Firebase authentication layer.
The common byte primitives may be reused by other codecs without importing local device trust.
The native [cloud encryption library](#end-to-end-operation-encryption) is separate from this driver
and is not a requirement on shipped Firestore page sharing. No protocol field or local safety gate changes.

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

The Remote-owned short-entry table forwards exactly `GET /read/<id>` in place to Colab's public
mount entry, without a Remote redirect. `/colab/…` is a route of the
[TMT web app](#public-browser-routes) and `/p/<id>` redirects into it; neither is forwarded. IDs contain exactly 4–64 ASCII `[0-9A-Za-z_-]` characters; on `/read/<id>`, query,
`tmt-session`, extra segments and upgrades refuse. Thread targets and reader capabilities
stay in browser fragments, never server paths. Navigation permits absent Origin or the
exact door Origin; cross-origin Origin and cross-site fetch metadata receive generic HTML 403.
Shared Host/framing admission and mount reply/deadline bounds remain unchanged.

Short entries never resolve a cookie/session, forward device context or trigger object setup.
Only the existing forwarded-header allowlist reaches Colab, with Remote-computed Host and
`tmt-mount`. The response is whatever Colab's public entry returns: Remote adds no catalog or
page lookup, strips `Set-Cookie`, rejects `101`, forces `Cache-Control: no-store` and retains
its existing Referrer-Policy rule. No unauthenticated GET inventory is added. Unmounted,
unsafe, missing or unavailable Colab sockets give the generic refusal page with the existing
mount status, independent of ID existence. The cookie remains scoped to `Path=/r/<prefix>/x/`;
ordinary mounted APIs/assets/WebSockets and anonymous admission are unchanged. Adding a route
or extension to the static Remote-owned table requires a contract change; extensions cannot
register root entries. Colab owns short-ID resolution and the browser's eventual app URL.

### Public browser routes

Browser assets live at the door root, disjoint from the route prefix, under the same Host, path and
framing rules. The door serves one TMT web app: GET/HEAD of exactly `/`, `/settings`, `/settings/`,
`/remote`, `/remote/`, `/remote/devices`, `/remote/connect`, `/remote/machine`, `/remote/settings`,
`/ops`, `/ops/`, `/digest`, `/digest/`, `/colab`, `/colab/` and `/colab/<segments>` (each segment `[A-Za-z0-9_-]+`, at most 160 bytes after
`/colab/`) answers the same static `shell.html`, whose only script is `/sdk/shell.js`. The list is
explicit, never a catch-all; `/sdk/*`, `/x/`, `/read/`, `/pair` and `/r/` keep their own handling,
and the document reads no cookie or session. Its policy is the pairing page policy below plus only
what the Colab route needs in the same document, `img-src 'self' data:`, `worker-src 'self'` and
`frame-src 'self'`, with `object-src 'none'`; it is no wider than Colab's mounted document policy.
`GET /p/<id>` answers 308 with `Location: /colab/p/<id>` (its query kept) and an empty body; a
malformed `/p` path receives the generic HTML 404 page. An app route and `/p/<id>` accept only a
plain query: unreserved bytes, `=` and `&`, with no escape and no `tmt-session`; the app writes
its own keys (`open`, `read`, `anchor`) there. Any other query is a malformed target (400). The client router switches routes in that
one document: each route is a module mounted into the shared web shell, and Colab's module is
loaded only from the composed shell's `entry` (below), never without an owner session. The rail is
always the same fixed list, Home, Colab, Remote, Ops, Digest, other described modules and
TMT settings, filled from the [composed shell](#module-descriptions-and-composed-shell) when the
browser holds an owner session; a module the answer does not describe stays listed, disabled, with
its reason. `/` is Home: it lists, per module,
the described Needs-you count, a waiting pairing offer from `/sdk/pair-offer`, and the modules'
described `home` cards; it reads nothing else. `/settings` and `/settings/` are TMT settings, its
sections listed in the module menu. `/ops` and `/digest` are Ops and Digest; their data have
their [owner browser reads](#owner-browser-reads). Until a page has a browser read, the page says it is not in the browser yet, names the terminal command
that shows the same facts and shows no values. Home preferences remain a
separate consumer. Remote is one app route with five paths, each a menu row of its own and one view of the page:
`/remote/` (Status, `/remote` is the same row), `/remote/connect` (Connect a device),
`/remote/devices` (Devices), `/remote/machine` (This machine's settings) and `/remote/settings`
(Remote settings). The selected row is whichever row the path names, so reload, Back and forward land
on the same row, and a switch between paths only changes the view shown. The module menu lists each
saved remote (or Status), the three rows for this machine, Add remote and Remote settings.
Pairing is not an app route: `/remote/pair` stays its own document with its own policy (below),
so its fragment-erasing bootstrap runs before any other script. It shows the same rail, with no
router, and its links load the app's routes.

Only legacy `/pair` GET/HEAD requests return 308 with constant `Location: /remote/pair` and an
empty body; the query is discarded without reflection. Host and Origin checks and cross-site
`Sec-Fetch-Site` refusal precede the redirect; CSP, no-store and no-referrer remain. Only that
redirect path admits a discarded query; other target grammars are unchanged. Browsers retain a
fragment across the redirect, and the pairing bootstrap erases it before any subresource.
Other methods and `/pair/…` refuse. The reader entry `/read/`, prefixed protocol/mount routes,
`/sdk/*` and the `/x/` refusal keep their existing behavior.

`GET /remote/pair` serves the pairing page with `default-src 'none'; script-src 'self';
style-src 'self'; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
The landing, pairing, settings and error pages share this policy and embed the shared TMT SVG favicon as data; images cannot make network requests.
`GET /sdk/pair.js` serves the synchronous fragment-erasing bootstrap, which then loads the SDK.
`GET /sdk/shell.js` is the same-origin entry that starts the TMT web app from the served SDK.
`GET /sdk/pair-offer` returns only the current unconfirmed offer's
`{profile, binding, machineId, windowId, offerId, address, serverChallenge}` descriptor with `no-store`; absent, expired, cancelled
or confirmed offers return the generic JSON 404. Cross-origin loads refuse. The code remains
exclusively in the captured fragment. Old `/pair/<descriptor>` paths refuse; no compatibility route
remains. Malformed pairing-page paths receive generic
HTML errors with their refusal status. These pages load only the embedded same-origin
`GET /sdk/pages.css` stylesheet, using the shared design tokens (the same `header` group as Colab's header) and system font
fallbacks without network fonts. Protocol refusals below `/r/` remain JSON. `GET /sdk/remote-v1.js` serves the device SDK as
`text/javascript; charset=utf-8` with `nosniff`; the path names the SDK interface version, not a
build, so it is not cached across upgrades. `POST /sdk/mount` takes exactly `{path}` from a page on
the door's own origin and answers `{machineId, windowId, address, extension, mount}`: this run's
identity and the mounted extension that contains `path`, including a valid enabled public short
entry in the same Remote-owned table or a web app route named for the extension (`/colab/…`), or null. It
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

### User path

1. **Start.** `tmt remote start` runs the remote door and the routes of enabled extensions. It does
   not enroll, restart or reconfigure agents, and needs no `--channel` flag or separate MCP setup.
   Native-channel enrollment is a local driver concern.
2. **Connect.** `tmt remote open /` opens the local page and enrolls a new browser with the
   read-only default grant. Native opens carry a short-lived fragment only to the opener; printed
   and shared page links remain code-free. Manual `tmt remote pair` remains available for advanced
   use and explicit sending policy.
3. **Talk.** A device with a sending grant uses the ordinary Core dispatch path, without
   per-message approval. Automatic browser enrollment grants only the four read scopes.

Automatic validation is remote's job and never adds a user step. A design that needs another user
step goes to the owner before it enters this contract.

The four hard lines are: authenticated pairing; no arbitrary shell or command endpoint; no paste
into an enrolled pane; no automatic resend after an uncertain outcome. In addition, membership of an
extension resource (for example a shared colab page) never implies machine or agent access.

Three standing guarantees hold on every path. Unauthenticated protocol traffic gets one generic
refusal and learns no inventory. Every effect rechecks the persisted grant inside its write
transaction, so a revoke orders after an in-flight effect. The serve lease is inherited by
invocation children, so a restart cannot overlap an orphaned effect. Uncertainty or a timeout never
mints a new operation ID: recovery reads core by the same ID.

### Local message layer and threat model

The message layer owns signed envelopes, correlation, durable append/subscribe/ack semantics,
sessions, replay, trust grants, optional hold, uncertainty and audit. A binding moves those messages
and rejects unauthenticated traffic at its edge; HTTP status, a document ID or a WebSocket
connection never grants authority. Changing bindings must not change the signature, scope or
grant rules.

All listening and remote state belong to `extensions/tmt-remote`. The `transport::Transport`
driver port carries exact envelope/result bytes through `append`, `subscribe` and `ack`; it does
not execute core work or decide authority. The local `routes::Routes` driver receives the shared
`message::MessageService` from foreground composition. That service hands ordinary signed calls
to session admission and the existing operation/journal owners before returning its signed result
or an opaque closed refusal. Subscribe is a finite journal read; ack is the caller's explicit
checkpoint, never an implicit consequence of delivery or failure. No driver retries an uncertain
effect. Core never listens or stays resident.
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
stop, revocation or expiry of the last eligible session, never an operation or pairing action;
state-changing operations still need a fresh device signature.

### Bytes, IDs and the fixed M1 suite

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

### Device identity

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

### Signed envelopes

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

Pairing is durable across session end, tab closure and serve restart until the owner removes the
device (or an explicit grant expiry is proved). The manual and automatic pairing pages first
try bounded session reopen with the stored identity. Success creates no grant or candidate:
manual entry shows `This browser is already paired.` and continues to `/remote/`, leaving the
manual offer and terminal untouched; automatic entry cancels its own fresh offer as before.
An absent record or a machine-signed revoked/expired grant permits new enrollment. A stored
machine-identity mismatch also permits the manual terminal-confirmed ceremony; the old record
is retained until the new grant commits. Automatic entry stops on mismatch and names the manual
`tmt remote pair` command. Unreadable storage, capacity and unconfirmed admission retain the stored
record and stop without enrolling. No replacement proof, envelope, receipt or grant-identity format is added.
Session limit/end and page-unavailable recovery never instruct a still-paired browser to pair again.

For a committed key, `session.open` is a signed control using `sessionId:"new"`, `sequence:"0"` and
payload `{clientNonce}` (random 128-bit lowercase hex). Its timestamp must be within 60 seconds of
machine time. Reusing that nonce within timestamp validity is refused. Return a machine-signed
response whose sessionId is the fresh session UUID and whose payload is
`{sessionId,serverTimeMs,grantRevision,expiresAtMs}`. `expiresAtMs` is the grant expiry, or null
when the grant has none; a session also ends when remote stops. Opening a session is silent and
never asks the owner to pair again. The client verifies the paired machine key before trusting it.
Each open adds an independent session for the device, bound to the same grant and revision;
opening never widens scope. The local per-device session limit defaults to off/unlimited.
Unlimited sessions remain bounded by the 60 s unattached expiry, 12 h idle expiry, signed rate-limited opens and the listener.
`tmt remote settings sessions-per-device <n>|off` sets a positive limit or disables it. An
absent `sessionsPerDevice` key in Remote's `settings.json` means unlimited; explicit null (`off`) means
unlimited. Both plain settings and JSON show the limit and its source (`default` or
`settings.json`); JSON uses null for unlimited. Serve reads the setting on each open, with no
restart. Damaged or unreadable settings use the unlimited default. The active limit ends the least
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
live session's own token and at most one latest-token alias per device. When its carrier ends,
the latest token follows a surviving same-device session with a live grant, exact current
revision and unexpired lifetime: attached first, then most recently active, then session ID.
If no session qualifies, Remote deletes every device mapping. Issuing the next cookie retires
the previous alias;
a request carrying that superseded alias during the brief in-flight window may fail.
The cookie scopes the device context, while signed messages and
transports belong to individual sessions. The cookie is a carrier for the same authenticated
device context, not a second credential model: every request and upgrade rechecks grant,
revocation and expiry, and a state-changing operation still needs a fresh device signature over
its exact intent. Door sessions live only in the running remote and do not survive restart.
Revocation, grant expiry/revision change and stop end all device sessions and tokens.
`limits::SESSION_IDLE` is 12 hours without activity while a session has a live transport;
`limits::SESSION_UNATTACHED_IDLE` is a 60-second inactivity grace for every session without
a live transport, including one that previously attached. Last-transport close starts fresh
grace; traffic and authenticated requests count as activity. Reattaching within the grace
resumes the same session. Detached sessions still count against the per-device cap until expiry.
At the cap, an open evicts only the device's most idle session without a live transport;
an attached session is never a capacity victim. If no detached session can make room, the
fresh signed open receives `REMOTE_SESSION_LIMIT` with the active positive `limit` and
same-origin `settingsUrl`. Close another page or change the owner-configured session limit;
capacity is not transient connection health and does not widen itself. The default is off/unlimited.
A rejected fresh open uses the existing machine-signed correlated response envelope with
`sessionId:"new"` and response sequence `1`, and allocates no Session, cookie or durable
sequence row. The fresh nonce is remembered within the existing bounded replay window;
unauthenticated or replayed controls still receive the opaque 404. The SDK verifies the
signature, correlation and rejected-open envelope before exposing `RefusalError` and its
signed limit/settings action. A fresh `session.open` with a valid stored-key signature,
current window/timestamp/origin and nonce can also prove only its own retained grant:
disabled yields `REMOTE_DEVICE_REVOKED` (even if expired), otherwise expiry yields
`REMOTE_GRANT_EXPIRED`. Stored policy/key validity and current identity are rechecked before
disclosure. Terminal replies reserve the nonce and charge the authenticated Call budget,
but allocate no Session, cookie, journal or operation. Unknown/wrong-key, malformed,
stale/replayed, budget-exhausted and deleted/corrupt identities stay opaque 404; ordinary
operations still require a live grant and never use this proof seam. No tombstones or
migration are added. After restart only a new current-window open can obtain this proof;
a same-key re-pair has a new identity and inherits no terminal result.
Maintenance runs even without new requests: the door's existing 100 ms event loop runs session maintenance at most once per
`limits::SESSION_MAINTENANCE_INTERVAL` (one second). Request paths still check session expiry
themselves. Explicit end, revoke, grant expiry/revision change, eviction and stop remain immediate.
Maintenance prunes persisted
session rows only when an old run or expired end notice needs removal.

The SDK's `transportUrl(session, mountedWebSocketUrl)` adds exactly `?tmt-session=<sessionId>`
to a mounted WebSocket URL. This value is a non-secret identifier, never a credential: Remote
requires a live cookie, checks that the identified live session belongs to that cookie's device
and current revision, and otherwise returns the generic 404 before forwarding. Only upgrade requests may carry it;
ordinary page URLs cannot carry it and it must never enter browser navigation/history. Remote
strips it before forwarding to the extension, never logs it and never places it in Location or
Referer. The SDK requires the door-matching WebSocket scheme (http → ws, https → wss) on the
current door's mount space, without other query parameters or fragments. Existing tunnels
without the parameter attach to the cookie's deterministic live carrier. An explicit identifier
remains pinned to that exact session and never follows a replacement.

Opening another session leaves existing sessions and tunnels live. The last upgraded transport
closing (including tab close or a dropped transport while backgrounded/asleep) starts the
session's reattach grace; other tabs stay live. A failed upgrade is not an established transport.
Ending closes remaining tunnels while held work survives the session end, bound to the device grant. Only stop, revoke,
and grant expiry/revision change cancel held work. Dispatching and uncertain work retain their
original operation IDs, frozen intent and recovery behavior. A limit eviction uses
`REMOTE_SESSION_EVICTED` with the active positive `limit` and optional `settingsUrl`; the door
omits the URL until #1769 adds the Remote settings page. The SDK exposes `RefusalError.limit`
and optional `settingsUrl`, also on send/operation refused states. Absent/null URLs mean
command-only guidance; when present, Colab shows the settings link plus `tmt remote settings
sessions-per-device <n>`. The SDK normalizes the URL and requires the door's origin. Session
idle expiry uses `REMOTE_SESSION_ENDED`. After verifying the device signature and current
grant, Remote can sign the distinct end reason for `limits::SESSION_END_NOTICE` (60 seconds);
afterward admission is the generic 404, also exposed by the SDK as `REMOTE_SESSION_ENDED`.
A real grant revision change uses `REMOTE_GRANT_CHANGED`; orderly serve shutdown uses
`REMOTE_SERVE_STOPPED`. Both close attached transports; the close itself carries no signed reason.
A still-reachable originating run can answer a verified request with its stored notice within the
same 60-second bound. A restarted run never answers the previous run's notice. An unreachable run,
expired notice, generic 404 or legacy `REMOTE_CLOSED` is unconfirmed, never proof of revocation.
Neither new code proves an effect was refused: retain its original identity and observe it after
re-admission without resending. Old SDK bundles treat these unknown signed codes as unconfirmed.
The served SDK's `reopenSession(previous, {retry:"bounded", signal})` performs only re-admission:
one coalesced series per previous verified Session, at most four opens within 20 seconds, each
attempt bounded to four seconds, with 250/500/1000 ms delays plus bounded jitter. The calling
connection owner controls cancellation, retrying another series and reattaching; the SDK owns no
second connection state machine. Same machine, origin, device identity and key pins are required;
no key, pairing or uncertain effect is recreated. Typed exhaustion is `unreachable`, `unconfirmed`
or `transient`. A verified session-limit refusal returns non-terminal `capacity`, is not
retried inside the series, and carries the verified `RefusalError` as `cause` with its limit
and optional settings action; identity/pin mismatch is `mismatch`, absent pairing is `unpaired`,
and unreadable storage is not absence. Verified fresh-open authority refusals return terminal
`revoked` or `expired`, detail `admission-refused`, with the verified `RefusalError` as `cause`;
neither is retried inside the series. The connection owner surfaces re-pair. An opaque 404
or unverifiable/unknown signed answer stays unconfirmed, never proof of revocation or expiry.
Cancellation cannot publish a Session. Previous SDK bundles leave an unknown expired code
unconfirmed, retaining any original operation ID.
An uncertain effect retains its original ID and is observed after admission, never resent.
Reopening is silent: the page sends another signed `session.open` from its stored device key,
with no owner step or new pairing. The caller reattaches its transport using the new session; no
send is retried.

### Durable log: append, subscribe and ack

Each paired client sees one machine-owned ordered stream of its admitted effect receipts and
correlated state notifications. Another client's IDs/cursors disclose nothing. The log is a remote
delivery journal over core resources, not a second conversation database or core attention queue.
Remote retains bounded held/uncertain payloads and immutable operation/request references; core
alone owns conversation history and retained finals. Final notifications can reference a core
request; `result` retrieves its current retained body through the public API. No permanent
final-body copy or new retention lease is created in remote.

`append(requestEnvelope)` authenticates every request. Effectful requests adopt the logical intent
and store its ID, client
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
beginning cursor. The fixed observational operations (`capabilities`, `agents.list`, `identities.status`,
`check`, `requests.show`, `result`, `dispatch.show`, `operation.show`, `remote.settings.show`,
`remote.devices.list`, `remote.management.operation`) return live signed results without adopting
the read ID, observed-read metadata or new ownership rows. `operation.show` is recovery observation:
stable or still-unresolved originals are pure reads; a definitive outcome learned for an unresolved
caller-owned original settles that original once, with its notification attributed to the observing
session. It never sends. A lost read reply requires a new signed
request, not journal recovery. Signature, freshness, session, scope, sequence and call-budget
fences still apply. Response entries contain effect state/receipt references
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
entries to 24 hours and 1000 entries/client; acked prefixes may be compacted earlier. A full stream
drops its oldest entries to hold a new one and never refuses work or a read for entry capacity; a
cursor behind the dropped position returns `REMOTE_CURSOR_EXPIRED`, so recover with own operation
IDs. Expired journal metadata can require fresh own-state recovery; it does not alter core
prompt/final retention. Separate bounded operation/request ownership records survive journal
eviction for 30 days after their last state change. Limit these to 1000 operations/client. To admit
a new operation, drop records left by reads, finished records (accepted, cancelled, refused) past
that horizon and, over the limit, the oldest finished records. Held, dispatching and uncertain
records are live work: never evict one to admit another, and refuse new adoption only when every
record is live or the frozen-byte caps are reached. A send under a dropped ID is a new adoption
that asks core for that operation ID before it creates anything. After this recovery horizon, or
once dropped, an owned read returns REMOTE_STATE_UNAVAILABLE, never permission to resend.
Revoked/expired grants cannot use these records to regain access.

### Pairing

Pairing authorizes one local device. Only the owner-only native control path opens offers;
a browser cannot initiate or extend one. Manual pairing and native automatic opens share one
in-memory offer, enrollment MAC/possession proof, machine receipt and default read-only grant.
The descriptor carries profile/binding, machine/window/offer UUIDs, address and a random 128-bit
server challenge (32 lowercase hex characters). No private key is included in a URL, DOM or log.

#### Native automatic opens

`tmt remote open [short-target]` opens a running local door (default `/remote/`). Targets are the
[web app routes](#public-browser-routes), `/p/<id>` or `/read/<id>`; query/fragment,
protocol/mount paths and external redirects refuse. Human output contains only the canonical
origin plus short target. `--json` returns the versioned result without opening; `--no-open` also
suppresses opening.
Other flags/settings/environment checks use the shared opener policy below.

The extension-only `tmt remote launch <short-target>` seam is hidden from help, takes the same
short target and `--open`/`--no-open`, accepts no `--json`, and always emits JSON. Its machine output
does not itself suppress opening; an extension passes `--no-open` when its own flags/JSON/agent
policy suppresses it. The shared CI/agent/display/SSH/open-setting checks still apply unless
`--open` explicitly overrides them; a piped machine result alone is not a suppression reason.
No platform opener means no offer. Success is exactly
`{version:1,link,opened,state}`: `link` is the canonical short URL, `opened` is boolean, and `state`
is `opened`, `skipped`, `no-opener` or `failed`. Command failures use `{version:1,error:{code,message}}`
and a nonzero exit; usage/unknown-command errors remain ordinary CLI usage errors. A consumer
recognizes an unsupported command as version skew and falls back to its existing opener plus
user-only pairing hint, never a guessed launch URL. Colab passes no code or private state and adds
no Remote crate dependency; its caller integration belongs to #3868. Opener injection remains
available to tests without launching a host browser.

With no active manual offer, each actual native launch opens a fresh **60 s**, first-use-only offer.
It replaces only a previous automatic offer. If control refuses with `REMOTE_PAIRING_BUSY`, the
opener instead launches the canonical plain short link once, reporting the existing `opened` or
`failed` state. It neither replaces nor cancels the manual offer, even if the opener fails. An
unpaired browser reaches its ordinary not-paired entry and receives no grant; a paired browser
keeps its ordinary access. Other preparation errors do not trigger a plain-open fallback.
For an automatic offer, the private launch argument is `http://127.0.0.1:PORT/remote/pair#auto=CODE`; it never reaches public CLI output,
errors, JSON, extension callers or logs. The public descriptor adds only `automatic:true` and the
validated `target`, not the code. One hashed synchronous inline script captures the fragment and
replaces history before imports, styles or any subrequest. A fragment change in an existing
`/remote/pair` document repeats capture/erasure and cancels its previous attempt; CSP adds only that exact script hash,
with `no-referrer`, `no-store`, exact Host/Origin and frame protection unchanged. Continuation
replaces the same tab with the validated short target. No preflight or second browser launch.

An unpaired browser creates a non-extractable key and uses the existing enrollment proof. The
first valid browser submission atomically creates exactly the four read scopes, with no sending,
settings designation, policy choice or terminal confirmation. The code is erased immediately;
**even an exact automatic replay refuses**, with no lost-response enrollment retry. The page
plainly refuses a used/cancelled/expired link and directs the owner to reopen from the machine,
using the page's own open command when known, otherwise `tmt remote open`. Transport/storage/proof
failures add `tmt remote pair` as the fallback. Commands render as code. Every
auto-confirmed device remains visible in `devices` with `issuedAtMs` and exact `origin`; serve
logs that device ID, time and origin, never the code.

A stored browser first proves its existing signed session/machine pin, then cancels rather than
enrolling again. Cancellation uses the same enrollment fields/HMAC plus `cancel:true`, with a
signature over LP(`tmt-device-pair-cancel-v1`) || LP(enrollmentBytes) || LP(raw HMAC), distinct from
enrollment possession. It requires the exact Origin, current auto offer/code and a live paired
browser key; it returns 204 and erases the offer. Unknown, cancelled, expired or already used
offers return the same generic 404. A paired-browser cancellation never widens or creates a grant.
Expiry is the fallback for an unused offer; opener failure requests exact-offer cancellation;
stop/process exit erases it. No auto-offer persistence or detached retry job exists.

**Local-attacker boundary.** The launch URL can be read from argv by the same user on macOS;
on Linux without `hidepid`, other local users on the same host can read `/proc/<pid>/cmdline`,
and a newly launched browser can retain its URL argument. The code is useful only until first use
or 60 s: first use wins, including a local attacker who wins that race. This is not protection
against same-user malware, root, browser extensions or OS/browser crash/history capture before
script execution. `replaceState` removes the normal browser entry after execution, not every
possible forensic copy. The owned 0700 state directory/0600 control socket do not hide argv.
Cross-origin web pages cannot obtain/use the code; no code enters request URLs, Referer, TMT
output or TMT logs. A failed/missing first script grants no access.

#### Manual pairing

`tmt remote pair` keeps the explicit terminal ceremony. Replacing a manual offer explicitly
cancels the previous offer. The terminal prints the browser pairing link and the code for add-on
and CLI devices. Its link is `http://127.0.0.1:PORT/remote/pair#CODE`, with 26 ungrouped base32
characters. The same synchronous bootstrap erases the fragment before any subrequest and fetches
only the public current descriptor from `GET /sdk/pair-offer`.

The manual command always prints its pairing link. Browser opening defaults to on and uses Remote's own
`settings.json` `open` boolean (`tmt remote settings open on|off`). `--open` overrides the setting
and terminal, CI, agent and SSH/display checks. Only `--no-open`, `--json` and a missing platform opener
suppress an explicit open. Without `--open`, no stdout TTY, CI, a nonempty `TMT_AGENT` (agent-run command), SSH without a display and Linux
without a display (except WSL) suppress opening. Opener failures warn without ending pairing.
Owner confirmation still requires an interactive terminal or `--json`.

The code is **16 random bytes**, displayed as 26 uppercase RFC 4648 base32 characters grouped for
**copy/paste**, one use, ten-minute manual expiry (60 s for automatic offers). Decode after removing ASCII spaces/hyphens only,
rejecting invalid alphabet/nonzero unused bits. It is not a short numeric/word password: an observed
known-message HMAC allows offline guesses, and three attempts/expiry do not prevent that attack. The
high entropy is mandatory; do not describe this scheme as PAKE.

The device proposes its public key, kind, exact origin, proposed name and a random 128-bit
clientNonce. It does not propose agents, scopes, mode or expiry; manual owner confirmation sets
them, while automatic confirmation selects only the read-only default.
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

In manual pairing, both sides show four words derived from SHA-256(LP(`tmt-local-key-fingerprint-v1`) || LP(raw public
key)): take the first 44 bits as four successive unsigned 11-bit indexes into the fixed English
BIP-39 2048-word list, in list order. Use the
[BIP-39 English list](https://github.com/bitcoin/bips/blob/master/bip-0039/english.txt); pin its
revision, bytes/digest and examples in the fixture slice. This is comparison text, not a recovery
mnemonic. The terminal shows kind, full origin, proposed name and the words, then asks the owner
to confirm the word match. New pairings are read only with no expiry. Without `--talk`, a separate
sending choice follows a confirmed match and defaults to no; `--talk` shows the sending grant before
word confirmation and skips that second prompt. Bare JSON confirmation is read only. Owner-side
`--agents` and `--hold` require `--talk` and narrow its agent allowlist and sending mode. A final-name
or expiry editor is not implemented. Enrollment cannot request sending authority.

Enrollment JSON names are `profile`, `machineId`, `windowId`, `offerId`, `serverChallenge`,
`clientNonce`, `kind`, `origin`, `name`, `publicKey`, `mac` and `signature`; binary keys/proofs use
base64url, challenge/clientNonce use 32 lowercase hex characters. Reject unknown fields except the automatic cancellation form above.

After confirmation, atomically create one grant and consume the offer. Receipt JSON is
`{grant,machinePublicKey}` with the grant fields defined below. The response carries base64url
exact `receipt` bytes and `serverProof`. Define
`K_response = HMAC-SHA256(codeBytes, LP("tmt-device-pair-response-key-v1") || LP(enrollmentBytes))`.
`serverProof` is the full HMAC-SHA256(K_response, LP("tmt-device-pair-response-v1") || LP(exact
receipt JSON bytes)). The device derives this key and verifies the proof before pinning the machine
key or accepting the grant; it verifies matching machine/profile/device key/kind/origin and a
well-formed grant. For manual offers only, exact candidate retry is idempotent until the original offer deadline; preserve
only K_response, the candidate digest/proofs and exact receipt for that bounded lost-response
recovery. Verify matching candidate and possession on retry; never accept K_response as a reusable
enrollment token. Raw code is erased after confirmation; no grant exists before manual owner confirmation or
first valid submission to an owner-opened automatic offer.
Delete the recovery key at the original deadline. Three failed code MACs across admitted enrollment
and automatic-cancellation candidates share one offer limit; signature or live-key refusal alone
is not a code guess. This limit, expiry, owner refusal/no confirmation, stop or process exit
cancels pending pairing and erases its secret. No
grant or device private key is stored for a failed/cancelled offer; sanitized audit may record
failure only.

Lost key, changed add-on origin, revocation or expiry requires pairing again. No P-256 downgrade,
export-based backup, automatic key replacement, passkey renewal or remote grant extension.

### Trust grants and remote lifecycle

Owner-local `devices talk <person|client-id> on|off` and `devices revoke <person|client-id>`
resolve an exact client ID first, otherwise a unique active device name. Duplicate names refuse
with the matching client IDs; no first-match selection occurs. Selection stays under the Store
lock through the existing grant mutation. Names are command selectors, not persisted authority.

A grant makes one device a trusted source on one machine. Grant JSON names are `clientId`,
`machineId`, `profile`, `publicKey`, `kind`, `origin`, `name`, `agents`, `scopes`, `mode`,
`issuedAtMs`, `expiresAtMs`, `revision` and `disabled`. `agents` is the string `"all"` (every
current and future owner-local agent, the default) or a sorted unique list of at most 256 core
identity UUIDs. `mode` is `direct` (default) or `hold`. `expiresAtMs` is null (default, no expiry)
or the time limit the owner chose at pairing. `revision` is a positive integer; `disabled` is
false on issue. Names are presentation only; rename preserves UUID authority, and a retired
identity's same-name replacement inherits nothing. After pairing, authority may only be narrowed
or revoked, except an explicit local-owner or designated-browser talk toggle may add or remove
only `talk`, preserving the agent allowlist, mode, expiry and every other scope; designation alone
widens nothing.

Supported scopes are `agents.read`, `status.read`, `check.read`, `talk` and `results.read`. New
pairings default to the four read scopes; sending requires explicit owner confirmation or `--talk`.
Existing grants remain unchanged, with no migration or automatic narrowing. Future core capabilities
do not silently become remotely callable; a new scope needs a revision of this contract.

`tmt remote serve` with human output starts a detached owner-device door; the modes, handoff and
stop semantics are the shared lifecycle in
[extension-serve-v1](extension-serve-v1.md). Foreground runs until Ctrl-C,
SIGTERM or `tmt remote stop`; accepted background serving has no idle or hard deadline.
Without `--port`, serve reuses its last successfully bound
IPv4-loopback port; on first use it selects an unused port. If the remembered port is busy, serve
refuses with `REMOTE_PORT_BUSY`, names the port, tells the owner to stop the process using it to
retain browser pairing, and offers `--port <n>` to choose a new origin explicitly. Human output is
an `error:` line and a separate `hint:` line; `--json` joins them as `error.message`.
`--port 0` explicitly selects a random unused port; an explicit nonzero busy port refuses without
fallback. The actual bound port is remembered for the next run, including after an explicit
selection. Human startup output puts the full door URL on its own line with no trailing punctuation.
Remote's side of the shared handoff: the launcher performs no core discovery or state
initialization, and Ready follows lease, listener, control and event admission with HTTP serving
starting only after Accept. The bounds are `limits::SERVE_STARTUP` (one absolute admission deadline),
`limits::SERVE_RECORD_BYTES` (one private record) and `limits::STOP_WAIT` (the launcher's cleanup
wait). The background worker admits and clears one private `remote/serve-error.json` only after
acquiring Serving, before runtime publication, and records fixed sanitized
version/phase/code/message on ordinary failure; it closes the file before releasing Serving. Duplicate
starts cannot clear it. Foreground preserves terminal errors and need not
create it. `status`/`stop` remain authoritative; run `serve --foreground` for terminal
troubleshooting. Launcher departure after handoff preserves held work; actual stop/restart cancels
pending holds through Approval. Frozen dispatching/unknown operation IDs remain recoverable and are
never resent. Lost output after Accept reports `REMOTE_READY_OUTPUT`; inspect `status` and use
`stop` before starting again.

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

#### Remote settings browser authority

**[#1769](https://github.com/pj-tmt/tmt/issues/1769) implementation: native/SDK and the shared-presentation settings page.** This section owns authority; the fixed management protocol
below specifies the native/SDK implementation. The local CLI remains the scripting management path.

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
browser actions to change those settings, rename or remove paired devices, including all others while keeping the caller. Reads expose no keys,
cookies, session tokens or core inventory. Settings use the CLI's existing store, validation and
locking; absent/default and explicit `off` are unlimited; explicit positive limits remain available. Device
mutations reuse Remote's grant-revision, session cleanup and device-event behavior. Explicit
trusted UI actions authorize writes; presentation state does not. A self-talk change or revoke can
end the caller Session after a committed effect; lost acknowledgment must not be reported as no effect.
Freeze each mutation's identity/input and recover unknown outcomes read-only without automatic
resend or a replacement operation ID.

Browser management never changes core settings, provider settings, drivers, extension installs or
argv, and never approves, rejects, cancels or releases held operations. `tmt remote approve` and
held-operation cancellation remain terminal-only. There is no generic config/command endpoint,
and designation alone cannot widen agent or extension authority; the explicit device talk toggle
is the sole scope-widening exception above. Typed wire/SDK payloads, refusals and mutation recovery
must be specified with the implementation before any such operation is advertised as supported.

Implementation acceptance must prove same-loopback browsers with different keys cannot impersonate
the designated browser, designation is local-only and never automatic, non-designated writes cause
no effects or holds, and revoke/re-pair/key change, expiry, restart, rename and concurrent removal
obey the lifecycle and effect fence above. The actual settings/device page separately requires
product native/browser acceptance, UX review and publication evidence; a shared presentation
package or current-page adoption cannot satisfy those requirements.

#### Remote management protocol

The #1769 implementation adds the following fixed operations on the existing signed
`POST /append` request channel. The settings/device page uses the shared browser presentation and Remote host metrics;
presentation carries no authority.
Every read and effect retains ordinary live-grant admission. Only a `browser` grant pinned to the
door's exact origin may use these cases. Agent scopes and direct/hold mode confer no management
write capability. Unsupported operations, including `remote.management.recover`, remain refused.

Management payloads are at most 1024 decoded UTF-8 bytes, strict JSON objects without unknown
fields. Mutation `operationId` is a canonical UUIDv4 equal to the signed envelope ID. Read
requests use a fresh envelope ID; original-operation lookup never adopts or executes a mutation.

| Operation                              | Exact input                                                                                                      | Result                                                                 |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `remote.settings.show`                 | `{}` or `{firestore:true}`                                                                                       | Settings view below                                                    |
| `remote.devices.list`                  | `{cursor:null\|string,limit:integer}`                                                                            | Device page below; limit 1–50                                          |
| `remote.settings.set`                  | `{operationId,setting:"open",value:boolean}` or `{operationId,setting:"sessions-per-device",value:string\|null}` | Management outcome below                                               |
| `remote.devices.rename`                | `{operationId,clientId,name}`                                                                                    | Management outcome; existing 1–64-byte nonblank/control-free name rule |
| `remote.devices.talk`                  | `{operationId,clientId,enabled:boolean}`                                                                         | Management outcome; changes only the talk scope                        |
| `remote.devices.remove-others.preview` | `{}`                                                                                                             | `{count:integer,fingerprint:string}`; designated browser only          |
| `remote.devices.remove-others`         | `{operationId,fingerprint}`                                                                                      | Management outcome; keeper is the authenticated caller                 |
| `remote.devices.revoke`                | `{operationId,clientId}`                                                                                         | Management outcome                                                     |
| `remote.management.operation`          | `{operationId}`                                                                                                  | Only the caller's original management outcome, with a live grant       |

A settings value is exactly `{open:boolean,source:"default"|"settings.json",
sessionsPerDevice:string|null,sessionsPerDeviceSource:"default"|"settings.json",warning:string|null}`.
The cap uses a canonical positive decimal string checked against the native target's `usize`
range, without JavaScript rounding; default is null (off/unlimited). This wire encoding
does not change the JSON file or CLI's numeric representation, validation or supported values.
Warning is null or `settings.json could not be read; defaults apply`. Malformed/unreadable fallback
reports default effective values and sources with that warning. A save does not evict current
Sessions; the existing Session opener applies the cap on the next open.

The empty-input settings view is exactly
`{settings,capabilities:{settingsWrite:boolean,devicesWrite:boolean},readOnlyReason:null|"local_cli_required"}`. Both write capabilities represent the current server
admission, never authorization for a later effect. With no current designation they are false and
the reason is `local_cli_required`.

The only additional `remote.settings.show` input is exactly `{firestore:true}`. Its view adds
exactly `firestoreLayers` and `firestoreBudget`, with the shapes and recorded-evidence limits
of the [optional status projections](#local-cli-discovery). `{firestore:false}`, extra keys
and every other input refuse as invalid. The snapshot uses the same recorded evidence as
status, without provider I/O or the deployment writer lock, and retains live-grant admission
before disclosure. The empty input and its result are unchanged; no new scope, operation,
journal entry or management receipt is created.

A device summary is exactly
`{clientId,name,kind,issuedAtMs,expiresAtMs,revision,revoked,talkEnabled}`, where kind is `browser`,
`addon` or `cli`, expiry is null or a JSON-safe millisecond integer and revision is positive and
JSON-safe; `talkEnabled` is the configured scope bit, not proof of live authority.
A list row adds exactly `{thisBrowser:boolean,liveSessionCount:integer,
lastActivityAtMs:null|integer,lastUsedAtMs:null|integer}`. The page is `{devices:[row],nextCursor:null|string}`. Store performs
revoked-row exclusion, UUID-keyset ordering and SQL limit+one-lookahead before materializing rows. Its 98-character
base64url cursor encodes only caller UUID and last UUID, is checked against the admitted caller,
and grants no authority. It is not a snapshot lease; concurrent grant changes may affect later
pages. The Session owner counts only live current-revision Sessions for the bounded page. Latest
activity is observed machine wall time minus the smallest monotonic idle duration; no live Session
means count zero and null activity. These are disposable observations, not persisted presence.
`lastUsedAtMs` is separate durable machine wall-clock metadata: the latest accepted authenticated
session open or sequence consumption, stored in that transaction and retained across restart.
Earlier rows have null until observed (`Not recorded`); no creation-time fallback or online claim.
The CLI device projection also adds this nullable JSON-safe timestamp. Revocations disappear
from CLI/settings inventory, while their internal rows and revocation replay remain intact.

Bulk preview covers every non-disabled device except the keeper, across page boundaries, up to
1,000 targets. Its fingerprint is lowercase-hex SHA-256 of compact UTF-8 JSON `[keeper,[[id,revision],...]]`, with targets sorted by ID;
it is an inventory comparison, not authority. The effect rechecks live designation and the exact
snapshot inside the same immediate transaction as all revocations and the immutable receipt.
Changed inventory yields `REMOTE_DEVICE_LIST_CHANGED` without mutation; above the bound yields
`REMOTE_DEVICE_REMOVAL_LIMIT`, never a partial batch. Keeper authority is unchanged; target
sessions end after commit. Local owner `tmt remote devices remove-others --keep <client-id>` uses
the same atomic inventory/revocation owner, with an explicit live keeper. A non-designated browser
can read inventory and sees its exact local `tmt remote devices designate <client-id>` command;
it cannot designate itself or execute removal. Re-pair short-circuit preserves the existing
designation because the device identity stays unchanged.

Browser device rows contain no keys, key fingerprints, cookies, Session identifiers, origins, agent grants or Core inventory.

A management outcome is one of:

- `{operationId,state:"committed",result:{settings}|{device:summary}|{removed:integer},sessionEnded:boolean}`;
- `{operationId,state:"refused",reason:<typed Remote refusal code>}`;
- `{operationId,state:"unknown",reason:"effect_outcome_unconfirmed"}`.

A setting result contains settings; individual device mutations contain the target summary.
Bulk removal contains only the removed count (0–1,000) and `sessionEnded:false`.
`sessionEnded` means the caller's old Session lost authority through a revision/revoke change,
not proof that a physical TCP socket has closed. Rename preserves the revision, live Sessions,
cookies and held work, and returns false; exact-repeat talk toggles do likewise, and revoked
grants cannot be enabled.
Repeated local revoke preserves the existing revoked revision. Device effects and
committed receipts share one immediate SQLite transaction; post-commit cleanup/events reuse the
existing device and Session owners after releasing live/Store locks. A cleanup, signing or transport
failure cannot remove that receipt or turn committed work into no effect.

Typed signed errors retain `{error:{code,message,limit?,retryAfterMs?,settingsUrl?,scope?}}` admission
conventions. A missing sending scope returns signed `REMOTE_SCOPE_DENIED` with `scope:"talk"`
and an optional same-origin `settingsUrl`, unlike a recipient allowlist refusal.
`REMOTE_MANAGEMENT_READ_ONLY` refuses non-designated effects before management intent adoption,
with no setting/device effects or management holds; ordinary sequence/budget/audit accounting remains.
`REMOTE_MANAGEMENT_UNAVAILABLE` means an original receipt is missing,
other-device or outside its deadline, never proof of no effect. `REMOTE_SETTINGS_UNAVAILABLE` is
used only for a proved failure before the settings-file first-touch boundary. Existing input,
rate, live-session, intent-conflict, device-not-found/revoked and state-unavailable codes retain
relevant meanings. Read refusals are SDK `RefusalError`; mutation signed refusals are refused
outcomes. Unknown transport, timeout or unverifiable acknowledgment raises `ClientError` with
original operationId. A verified generic state, ownership or response-publication failure is
phase-ambiguous after publication: the SDK raises `ClientError("outcome_unconfirmed")`, retaining
that original ID and unknown outcome. `REMOTE_STATE_UNAVAILABLE`, `REMOTE_MANAGEMENT_UNAVAILABLE`,
and generic closed/ended/core failures cannot prove that the mutation had no effect. Ordinary
reads retain `RefusalError`; agent operation behavior is unchanged. Only explicit input, scope,
rate, intent conflict, management read-only/capacity, device-list-changed/removal-limit, device not-found/revoked and proved
settings-before-touch refusal codes establish a pre-effect management refusal. A post-publication HTTP 404 alone is unknown, not a signed zero-effect
refusal. A prior ended Session that prevented publication is a refused unsent attempt.

Store persists exact intent digest, original caller UUID, grant revision, immutable adoption time,
30-day deadline and outcome. Management and agent operation IDs cannot collide. Identical
retained intent returns only its existing outcome; changed operation/input/caller conflicts.
No lookup renews the deadline. Expired identity rows remain replay tombstones; capacity is bounded
at 1000 per caller and 4000 installation-wide, refusing new adoption rather than evicting an
unknown ID into eligibility. These are cumulative retained-identity limits, not a rolling
30-day allowance. The 30-day horizon
only bounds outcome availability: expiry does not physically delete identity/receipt rows or
make their IDs eligible again. `REMOTE_MANAGEMENT_CAPACITY` is a signed pre-effect refusal with
no adoption/effect. Existing live original-ID reads remain available at capacity. Show the local
`tmt remote settings` / `tmt remote devices` path without retrying, resetting the database or
allocating replacement IDs; preserve any previous unknown outcome. Compaction/reclamation is
a deferred improvement.

Settings adoption and file publication are not one transaction. Persist the original unknown
receipt first, then recheck current Session/grant/key/revision/designation at the effect fence.
Lock order is live Session, Store immediate transaction, then settings.lock; no settings-lock
holder acquires live/Store. The shared settings writer marks uncertainty before the first
`Layout::file` creation or truncation. Only durable writer success and receipt settlement report
committed. A later file write/sync or receipt failure retains unknown. Crash, pending adoption and
settlement failure never authorize another setter; current values cannot establish that original
operation's outcome. Explicit original-ID lookup is read-only.

Local owner commands `tmt remote devices designate <client-id>` and
`tmt remote devices undesignate` select/remove the persisted machine/device/key designation
through the existing control socket or stopped serve lock. Designation validates a live browser
and exact current/remembered door origin. It is never automatic or remotely callable. Revoke or
same-key re-pair clears it; rename retains identity and live Sessions. The
optional pairing-confirmation choice is not implemented; later local designation
is available without another browser ceremony.

The SDK `management(session)` exposes the verified caller `clientId` and `settings`, `devices`, `previewRemoveOthers`, `removeOthers`, `set`, `rename`, `talk`, `revoke` and
`operation(originalId)` over the same serialized verified channel as agent operations. It opens
nothing and never resends a mutation or creates a replacement mutation ID after unknown outcome.
Freeze the original input/ID independently of subsequent form editing.

After self-rename, the same live Session can read its original receipt by ID, without replay. After
a lost self-revoke acknowledgment, make one read-only fresh-admission attempt with the same paired identity and the current trusted door
descriptor. If admitted, read the original operation. If refused, show loss of current access
separately from an accurate unknown outcome and the local `tmt remote devices` confirmation path;
do not assert committed revoke, mint an ID, resend or offer a mutation retry. Transport error,
stale/unavailable descriptor and malformed/unverified response remain unknown/unconfirmed, not
proof of permanent grant loss or committed revoke. Expiry, local revoke, re-pair and key change
retain existing refusal semantics. Designation loss with a still-live grant permits only ordinary
own-receipt reading. No historical/revoked-key admission, capture-based capability, new nonce
framework, agent/held-work recovery exception or general recovery route exists.

#### Remote settings page

`GET /remote/devices`, `/remote/machine` and `/remote/settings` serve the Remote-owned shared-presentation
page with the existing page CSP, origin checks and no-store policy. `/sdk/settings-v1.js` imports `/sdk/remote-v1.js`; it does not
embed another signer/channel implementation. The additive public SDK exports are
`FirestoreSettingsView`, `budget.parsePublishedLimits` and `budget.publishedLimitRows`. The sections are
Browser opening and Session limit per device (`/remote/machine`), Firestore and the remotes saved in
this browser (`/remote/settings`), and Paired devices (`/remote/devices`). Effective values, sources,
warning and write capabilities come from admitted server reads. The shared package supplies checked CSS
and static presentation contracts; Remote owns host layout and static listbox controls. Shared
presentation never supplies authority or management outcomes.

The read-only Firestore section uses the signed opt-in settings view, showing the same recorded
layer/prerequisite states and fixed next commands as `status --layers`. Any layer reporting
`support: not-implemented` is shown only as not available in this release, without its other
prerequisites or a command. Published free-plan limits are dated allowances, not actual usage
or measured headroom; their guard thresholds do not create a usage warning. Missing evidence
reports not configured without inventing layer verdicts. An unsupported, malformed or failed optional
read clears stale Firestore evidence and reports unconfirmed setup without replacing current settings
access, frozen management outcomes or drafts. Inspection never deploys, retries or repairs.

Enable sending and Revoke require an inline consequence and a second deliberate press against the
current admitted target; cancellation creates no intent or operation ID. This is presentation,
not authority. Changing a choice only edits a draft until Save.

An untouched unset/default cap remains unset; explicit off is unlimited and custom positive caps
remain decimal text. The default option describes the missing key and does not invent a reset
setter. Existing CLI setters do not remove keys. A custom 8 is explicit configuration, distinct
from an unset default. Ordinary, effect and recovery refreshes retain the current device-page cursor.
Device-name forms remain bounded to that page; navigation is refused with the unsent name focused
until it is saved or explicitly restored to the admitted name, so navigation cannot discard a draft.
Form drafts survive asynchronous outcomes and read refreshes. Non-designated
browsers retain visible values and persistent local-CLI read-only guidance with disabled mutation
controls. Read-only accessibility descriptions follow admitted editability on every render/refresh;
editable forms and device-name inputs do not reference hidden read-only guidance. Independent
session-limit help remains associated in both roles. Unknown or capacity outcomes disable further mutation submission without discarding
original input/ID or unsent draft text; no mutation retry/reset/replacement-ID action exists.

The page keeps current access separate from committed/refused/unknown outcome. Explicit
original-outcome reading and ended/grant-changed read refreshes use one
[bounded live-grant re-admission series](#signed-envelopes), then only the original lookup or repeated
read; no mutation is resent. Verified revoked/expired fresh-open proof shows current access loss;
opaque 404, unverifiable replies and other bounded reasons remain unconfirmed. Capacity shows the
existing capacity action. Verified access loss stops further refresh admission until reload/re-pair.
Both reopen modes bind to the existing paired identity/public trust pins and reject re-pair/key
replacement before network access. The stored machine pin must be a 32-byte Uint8Array equal in
every byte to the prior verified Session pin. Invalid/replaced pins or identity are refused before
network access: one-shot `reopenSession(previousSession)` throws `REMOTE_SESSION_ENDED`; the bounded
series ends as `mismatch`. For the one-shot call and its callers, an HTTP admission refusal rechecks
the descriptor without a second admission;
changed/unavailable/malformed descriptors and transport/unverified replies remain unconfirmed.
A one-shot refusal against the unchanged trusted descriptor shows current access loss; it proves
neither the original mutation committed nor permanent grant loss. No captured key is persisted as
a recovery capability. No-argument reopen behavior and ordinary agent operations stay unchanged.

#### Browser entry

Human `serve` output links to the same-origin `/` browser entry. JSON readiness, status and SDK
`address` retain the protocol `/r/<prefix>` base; navigating there still gets the generic refusal.
The entry validates this origin's saved browser record through the existing SDK owner. Absent
local data means Not paired; inaccessible or malformed data means Pairing unreadable. Complete
identity/origin/address/key-pin validation and the existing non-extractable key consistency check
precede any network request. Stored key presence alone never establishes live authority.

Each page open automatically checks once: read the current descriptor, open one verified
`session.open`, then observe `capabilities` through that Session's serialized signed lane. These
reads are not journaled. Check again is manual and reuses the current verified Session; an ended
session can be reopened only by that explicit check. An expired Session is silently reopened once
within the manual check; an ended capabilities response is not a pairing refusal. No polling,
automatic re-pair, grant renewal,
work dispatch or recovery resend occurs. Connected means this check verified current access,
never settings-administrator designation; open an app from its link in this browser.

A different machine keeps the saved pairing unchanged. Only a specifically verified signed
refusal reports Not accepted. Opaque404, transport, stale/malformed descriptor or unverifiable
response stays unconfirmed, without inferring revocation or another permanent cause; a public
descriptor cannot replace the saved machine pin. A descriptor-only recheck after opaque404 never
retries admission. Page lifetime fences prevent new requests after departure, including signing
continuations; per-check fences prevent late painting. Departure cannot undo an already sent
request. Details contains the short machine ID, viewer-local checked time, protocol address,
trust pin and pairing-versus-administration note. Owner pairing retains fragment erasure,
fingerprint comparison and terminal confirmation.

#### Local CLI discovery

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
stopped by hand (Ctrl-C in its terminal) and started again, since `stop` cannot reach it.
Ordinary CLI status adds the secret-free `signIn:{state,expiresAtMs}` projection; expiry is
nullable. Credential read/ownership errors report `unreadable`, and a transient writer lock
reports `busy`, preserving door status and a successful diagnostic exit with the existing
message line. Explicit/default remote selection failures still refuse. No credentials,
cookies or device inventory are exposed.

`tmt remote status --machine --json` selects an optional root-local observation; `--machine`
requires `--json`. It sends exactly `{"op":"status","machine":true}` through the same owner-only
control request, instead of the ordinary exact `{"op":"status"}`. A running supporting serve
returns exactly:

```text
{"running":true,"origin":"http://127.0.0.1:<port>","path":"/r/k7qxm4tz2pbwn6rh","machineId":"<canonical non-nil UUIDv4>"}
```

All four values come from that connected serving owner; `machineId` is the immutable machine
already captured for this run. Discovery does not open or initialize Store, read a machine key,
create a pairing offer or acquire an HTTP descriptor/Session. Ordinary parsing remains exactly
three keys; the opt-in parser requires exactly four, with the same origin/path validation and
canonical lowercase non-nil UUIDv4/variant validation for `machineId`. Partial, malformed or extra
fields fail closed. Stopped/absent inspection remains exactly the two-key document above, without
`machineId` even when stored state remembers one; legacy state is not migrated.

`tmt remote status --objects --json` selects a separate optional running-only observation;
`--objects` requires `--json` and cannot be combined with `--machine`. It sends exactly
`{"op":"status","objects":true}` to the same control owner and returns exactly:

```text
{"running":true,"origin":"http://127.0.0.1:<port>","path":"/r/k7qxm4tz2pbwn6rh","objectChannels":[{"extension":"<installed Local name>","state":"ready"}]}
```

`objectChannels` contains only static Local declarations, with no credentials, paths or backend
identities. Each entry has exactly `extension` and `state`; `state` is `ready` while the current
channel has not ended, or `starting` while its setup is in progress. An `unavailable` entry also
has exactly `reason`, one of `storage`, `setup`, `channel-ended` or `capacity`. Names must be valid
extension IDs and unique. This is a best-effort observation, not authority or a promise that the
next request succeeds; status never activates or retries a channel. The ordinary and machine
projections remain unchanged. Stopped inspection keeps the two-key stopped shape, without
`objectChannels`. Unsupported optional requests preserve their original error code/message;
malformed or silent replies are errors, never proof of stopped or healthy storage.

`tmt remote status --layers [--json]` reads the selected local snapshot, including while serve is
stopped, as defined by [native remote selection](#native-remote-selection); it is exclusive with
`--machine` and `--objects` and also has a human rendering. When serve is reachable, the CLI obtains
ordinary discovery with `{"op":"status"}`; it adds `firestoreLayers` from that local snapshot.
The door still accepts `{"op":"status","layers":true}` as a separate live observation returning the
ordinary three members plus `firestoreLayers`; its evidence uses the configured-default/sole-project
selection policy, like the signed settings projection. The list is empty until a Firestore deployment
has recorded evidence: neither read contacts the provider or starts anything. Otherwise it has one
entry per layer, in the order `sharing` (page sharing and collaboration),
`operations` (device operations) and `attachments`, each exactly `{layer,state,prerequisites}`. `state` is `enabled`,
`not-enabled` or `unknown`, derived from the prerequisites (any `not-enabled` makes it `not-enabled`, otherwise any
`unknown` makes it `unknown`). Each layer has its own prerequisites in a fixed order, as `{item,state}` plus, unless
`enabled`, a fixed `reason` and, only where a command exists, `next`: sharing has `project`, `sign-in`, `rules`,
`indexes`, `plan-tier`, `quota`; operations has `plan-tier`, `support`; attachments has `support`, `project`, `rules`, `quota`.
Reasons per item are `project`: `not-configured`, `access-lost`; `sign-in`: `provider-disabled`, `permission-missing`;
`rules`: `not-deployed`, `out-of-date`, `partial`; `indexes`: `building`, `not-ready`; `plan-tier`: `paid-plan-required`; `quota`: `headroom-low`, `exhausted`
(only from a recorded refusal); `support`: `not-implemented`. An `unknown` prerequisite has the reason `not-checked`;
missing factual evidence is never guessed. `next` is only `tmt remote deploy firestore`.
Tier and quota rows describe requirement satisfaction, not observed billing or project-wide headroom.
A layer requiring no paid plan has `plan-tier: enabled` without an observed tier. The shipped sharing
layer has `quota: enabled` without a quota observation because its browser and native paths require the
[Free-plan budget guard](#free-plan-budget-guard) for their own modeled usage. An explicitly recorded
quota refusal retains its fixed reason; no local counter is presented as aggregate project usage.
This policy does not advertise a guard for unimplemented attachments or satisfy a paid-required layer.
Ready sharing's CLI human line and SDK section include the fixed note: "Each client stops itself before the
free-plan limits. TMT does not check the project's remaining quota, so Firebase may still refuse requests."
The CLI follows the shared style's final-period omission; the SDK retains the period. The note is
presentation, not a new wire member or reason. Exhaustion and an unavailable guard remain use-time refusals; readiness
never promises the next request will succeed.
The sharing index prerequisite reads the authorized run's digest-derived `composite-index` steps, using the
same canonical index set as composition, authorization and verification; it makes no provider call.
Recorded Done/Adopted for every required composite means `enabled`; Pending or a recorded failed/refused
step means `not-enabled/not-ready` (not proof of live absence); Building means `not-enabled/building`.
A known not-ready step outranks building, and either outranks an unknown step. Missing/malformed/duplicate
identities or ambiguous outcomes mean unknown; an ambiguous/contradictory final verification cannot promote
otherwise finished composites to ready. A run with no composite evidence is unknown, never a verified empty set.
In particular, a pre-#3008 Complete record now reports its index prerequisite as unknown until the next
authorized deploy reconciles the indexes and records their evidence; reads do not convert records or change
other prerequisites/layers. Rules retain their existing independent meaning and aggregate before indexes.
Fixed index sentences are 'Firestore indexes are still building.', 'Firestore indexes have not been verified
as ready.' and 'Firestore index readiness has not been checked.'. The building human next step is
'Wait a few minutes, then run tmt remote deploy firestore.'; the other two are 'Run tmt remote deploy firestore.'.
JSON `next` remains the exact command string for all three; the wait wording is presentation only.
The SDK strictly validates the same item order, states/reasons and command against the Rust-owned fixture. Device operations are not offered on the free Firebase
plan, so they report `paid-plan-required` there and, until a release implements them, `support: not-implemented`. Every
word comes from a fixed table: no provider text, secret or path. The ordinary, `--machine` and
`--objects` projections are unchanged.

`tmt remote status --budget [--json]` selects a fourth optional running-only observation, exclusive with `--machine`,
`--objects` and `--layers`; it also has a human rendering, and plain `status` is unchanged. It sends exactly
`{"op":"status","budget":true}` and returns the ordinary three members plus `firestoreBudget`: a dated snapshot of the
Firebase no-cost plan's Firestore limits, never the project's usage, which only the Firebase console shows. Its fixed
members are `plan` (`no-cost`), `readOn` (the date the official pages were read), `resetsAt` (the fixed word
`pacific-midnight`: daily limits reset at midnight US Pacific time), `limits` (positive integers for document reads, writes and deletes per day, stored bytes, egress bytes
per month, free databases per project, composite indexes, single-field index configs, document bytes and Rules lookups
per request) and `guard` (the `warnPercent` and `refusePercent` of the budget guard). It holds no provider text, secret
or path, and the numbers live in the Remote implementation guide's dated table, not in this contract.

An old command rejects `--machine` before state work. Against an old live serve, the optional
projection preserves the standard nonzero error document and its original unsupported code/message:
`REMOTE_CONTROL_UNSUPPORTED`, or the exact legacy `REMOTE_INPUT_INVALID` / "Unknown control operation."
It does not rewrite that optional refusal to stop/restart, installation or repair advice.
Other connected peer errors stay errors; malformed/silent replies and unsafe/unavailable state
never imply stopped or a machine ID. No preceding ordinary status or second acquisition is required.

This observation uses the invoking executable and inherited core-root context, and grants no
routing or management authority. A consumer may omit its whole optional machine/identity hint
when unsupported, stopped, timed out or unavailable; it must not prevent creation, backfill
historical absence or infer another identity from labels/current agents. A creation hint is not
an atomic identity-directory/grant proof: use-time routing still requires both stored IDs to match
the authenticated current machine and a unique admitted agent under the current live grant.
The first supporting release is recorded after delivery, not inferred from an installed receipt
or a guessed minimum; a selected new CLI does not prove an old loaded door supports this projection.

`tmt remote stop --json` asks the running serve to shut down through its owner-only control
socket, then waits up to 40 seconds after acknowledgment for the lifecycle lease to be released
and the control socket removed. It uses the same shutdown flag as SIGTERM: listeners, active
sessions, tunnels and pending work close; stored pairings and grants remain. It emits exactly
`{"stopped":true}` after confirmation, or `{"running":false}` if no serve was running, and exits 0.
An unsafe/unresponsive control socket, a held lease without a reachable serve, or unconfirmed
shutdown returns the standard Remote error document and exits nonzero. Stop never identifies or
signals a process by PID and never starts or migrates stopped state. A concurrent successor may
make shutdown unconfirmed; stop does not send another request to shut down that successor.

`tmt remote restart --json --no-open` performs one background-only restart. Its fixed result
is `{outcome,reason}`: `restarted` or `not-running` with `reason:null`; `deferred` with
`busy`, `foreground` or `legacy`; or `unconfirmed` with `candidate`, `startup` or `unknown`.
Expected lifecycle refusals exit 0 with that typed result; malformed or lost replies are never
restart success. Ordinary status stays unchanged. An absent/stopped owner starts nothing.

The replacement checks Core capabilities, read-only schema/key/endpoint prerequisites and the
trusted mount registrations before cutoff. The owner-only control snapshot supplies this run's
background mode, window, origin, route and mount policy: persisted home, lease, machine route
and bound port remain authoritative, but do not record a live generation or background mode.
Foreground, unknown-mode and legacy owners are left running, including Colab-supervised doors.

Under the held serve lease, Remote briefly checks the existing offer, live-session, Store and
object-upload owners and sets the existing stop flag in the same control operation. An active
pairing offer, interactive preview/confirmation, pending held approval or observed in-flight
transfer defers without stopping or changing that work. Live-session effects, pending uploads
and committed reads use their existing activity/accounting/transfer-holder state. The shared
channel budget must have no outstanding admitted requests or callbacks, including a read
awaiting disclosure; registered page interests are long-lived and do not defer restart.
Unavailable observations cannot admit a restart.
There is no new admission fence: work arriving in the narrow check-to-stop race can be interrupted
as by ordinary stop. Normal pairing/approval cancellation remains unchanged.

After confirmed lease/socket cleanup, one replacement worker starts on the exact old endpoint,
with the same home/key/grants and trusted mounts, and a fresh window/sessions/cookies. There is
no port fallback, retry, PID signalling, successor stop or rollback promise after cutoff. Stop
and startup retain their existing 40-second and 35-second bounds and existing bounded cleanup.
A lost acknowledgment gets one read-only status observation; inspect status before a manual
retry, since a missing reply cannot prove that no effect occurred.

Core captures one pre-upgrade running observation through the verified existing executable.
Only actual replacement of a previously observed running Remote calls the captured replacement
with exactly `restart --json --no-open`, outside the installation lock with the captured Core
executable and secret-stripped environment. Initial installs, no-ops and stopped doors have no
restart outcome or hint. Success reports `Remote upgraded and restarted in the background.`;
deferred/failed/unknown outcomes retain the existing manual restart hints. A parent human upgrade
prints terminal-only `Upgrading Remote…` before applying a pending Remote replacement; its JSON
child remains silent. Colab attachment and fresh-session re-admission retain their existing owners.

A supervising extension can start `tmt remote serve --json` as its child. Serve emits one readiness
line on stdout after binding the door and control socket and recording the bound port:

```text
{"profile":"local-v1","binding":"loopback-http","state":"ready","address":"http://127.0.0.1:<port>/r/k7qxm4tz2pbwn6rh","machineId":"<uuid>","windowId":"<uuid>","startupCoreCalls":2}
```

Door readiness does not imply Local object-channel readiness; observe the optional running
`status --objects --json` projection separately. The stable fields a supervisor reads are
`state:"ready"` and `address`. This descriptor has no
separate `origin` or `path` fields: `address` is their concatenation, with no trailing slash.
`machineId` is the stable machine UUID and `windowId` is the UUID for this serve run. The other
fields identify the local profile, binding and two startup core calls. A startup failure emits the
standard error document and exits nonzero, including when the remembered port is busy. Serve remains
in the foreground. The supervisor stops only the child it started; attaching through status does not
give it ownership of an existing serve. Only the owner-controlled native open path or explicit
`tmt remote pair` may open pairing; the existing owner-only mount socket and grant rules apply
to both attach and supervised-start use.

### Operations

Payloads for core API operations are the existing API envelope, decoded without rewriting input.
Remote narrows supported operations/authority before core calls.

| Logical operation                                                                               | Scope and public core mapping                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capabilities`                                                                                  | Signed paired discovery of supported subset, fixed suite and core bounds.                                                                                                                                                          |
| `agents.list`                                                                                   | `agents.read`; `tmt ls --json` projected to permitted UUID/name/presence, delivery status and optional runtime/activity evidence below, without pane address/cwd/process/profile.                                                  |
| `identities.status`                                                                             | `status.read`; input restricted to permitted UUIDs. Self-report is not readiness or completion.                                                                                                                                    |
| `check`                                                                                         | `check.read`; one permitted agent, the bounded capture `tmt check --json` returns locally. Read-only; it never writes to a pane.                                                                                                   |
| `dispatch.create`                                                                               | `talk`; one permitted direct request recipient, core originator per Identity attribution (below) plus remote provenance (below). `direct` grants dispatch after admission; `hold` grants hold for local approval. No fan-out, room or announcement in v1. |
| `dispatch.show`, `operation.show`                                                               | `talk`; only journal-owned operation IDs; core immutable receipt or remote held state.                                                                                                                                             |
| `requests.show`, `result`                                                                       | `results.read`; any request the local `tmt result` can read, through the public API or `tmt result --json`.                                                                                                                        |
| `requests.sendNow`                                                                              | `talk`; the owner's own browser only, for a request this device sent ([Send now](#send-now)). Core runs `requests.sendNow` for the person.                                                                                         |
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

Optional `runningDriver` uses Core's [List runtime driver](identity-status-v1.md#list-runtime-driver)
provenance. Remote forwards only nonempty strings of at most 32 bytes in `[a-z0-9-]`; otherwise
it omits the key, never null. The SDK recognizes exactly `claude` and `codex`; unknown values
become an absent property without failing the list. Recognizing a new driver requires an additive
SDK release. Remembered `driver` is never a fallback; this field grants no dispatch authority.

Optional row `activity` forwards only Core's public [`ls --json` session.activity](extension-api.md):
`{state:"working"|"idle"|"unknown"|"ended",sinceMs:number|null}`. Omit the key, never null,
when absent or malformed: the state must be one of those four and `sinceMs` must be null or a
non-negative JavaScript-safe integer. Neither `lastActivityMs`, `providers` nor other session data
is disclosed; top-level Core `activity` is not a fallback. Each row also carries `observedAtMs`,
the owner's integer-millisecond wall clock sampled once when the Core list read returns.
Compare `sinceMs` with that owner time, never the viewer's clock; viewers clamp negative elapsed
values to zero. SDK `RemoteAgent` keeps both fields optional for older owners and independently
omits malformed values without failing the list. The same `agents.read` grant filters rows before projection; there is no extra Core call,
admission rule or limit increase. Activity grants no delivery, readiness, completion or dispatch authority.

Helper payloads are `agents.list:{}`, `check:{agentId,lines?}`, `operation.show:{operationId}` and
`result:{requestId}`. A client's `dispatch.create` payload is exactly
`{version:1,operation:"dispatch.create",originator:"anonymous", input:{operationId,recipientIds:[agentId],message,kind:"request"}}`.
Signed envelope operation and core operation must agree, and envelope id must equal
input.operationId. Other supported core reads use their documented `{version,operation,input}`
envelope. Discovery returns `{version:1,profile:"local-v1",binding,operations,limits}` with only the
admitted operation subset and effective byte/rate bounds.

A remote device is never a local identity: a caller that selects an `identity` or any originator but
`"originator":"anonymous"` is refused, and the signed client payload is always the anonymous form.
Remote prefixes the frozen message with one provenance line naming the device,
`[remote: <device name>]`. A `cli` device additionally names the sending identity it resolved on
its own machine, `[remote: <device name> / <identity name>]`; that name is presentation only and is
never mapped to an identity on the receiving machine. The core originator is set by the serve, not
the caller: see **Identity attribution** below.

**Identity attribution.** Only a `browser` grant from this door's origin, the owner's own paired browser, is
attributed; `cli` and add-on grants stay anonymous and never ask core, because a `cli` device may be an agent on another
machine. Once such a send is admitted the serve asks core for the person at this machine (`user.identity`, [extension API](extension-api.md#the-persons-identity))
and freezes the core payload with `"identity":"<identityId>"` in place of `"originator":"anonymous"`,
exactly one of the two. That makes core record the message as the person's own and keeps Digest from
holding it. The journal holds the frozen form: a held send replays exactly what was frozen, a retry
of a known operation ID asks core nothing, and a changed `user.name` takes effect on the next new
send.

A refusal of the person's name never blocks or retries the send: it is frozen anonymous. The send
result then carries the optional field `identity`, defined here once and present only when there is
something to say:

- `{"state":"anonymous","reason":"USER_NAME_UNAVAILABLE"|"USER_NAME_INVALID"|"USER_NAME_TAKEN","message":<core's message>}`
  when `user.identity` refused the name. The message is core's own and names `tmt config set user.name <name>`.
  Any other `user.identity` failure freezes anonymous and says nothing.
- `{"state":"person","name":<name>,"renamedFrom":<name>}` when core resolved the person under a
  `-user` name because an agent holds the plain one.

The field is about the freeze that created the operation: a later observation or a retry of an
operation that was already frozen carries it only if the stored receipt does. A plain success has no
`identity` field. Clients ignore its absence, parse it strictly when present (exact keys, `message`
and names non-empty text of at most 512 bytes), and treat a malformed one as an unverifiable reply.
How often a surface shows it is the surface's rule, not this contract's.

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
`RemoteAgent` contains `id`, `name`, `presence`, optional core-owned `delivery` forwarded unchanged,
optional `runningDriver` (`claude` | `codex`), `activity` and `observedAtMs` (see above for provenance/decoding).
Send/operation use SendState; result uses ResultState as defined below. No
selection/URL/title/note fields are reformatted: they are already inside the caller's frozen
message. The caller owns durable operation IDs and intent; the SDK stores no dispatch bytes in
IndexedDB. Explicit identical re-sends rebuild the same payload bytes.

Verified pre-effect refusals on send/operation return
`{state:"refused",operationId,reason}`, with the signed code `REMOTE_SCOPE_DENIED`,
`REMOTE_INPUT_INVALID`, `REMOTE_RATE_LIMITED`, `REMOTE_INTENT_CONFLICT`, `REMOTE_CLOSED`,
`REMOTE_SESSION_ENDED` or `REMOTE_SESSION_EVICTED`; a rejected fresh open exposes
`REMOTE_SESSION_LIMIT` with its signed limit/settings action, or `REMOTE_DEVICE_REVOKED` /
`REMOTE_GRANT_EXPIRED` as the authenticated retained-grant proof above. The expiry code is
not pre-effect evidence for an original operation.
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

### Dispatch, hold and uncertainty

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

A pending `result`, and `requests.show`, also carry Core's `messageState` when it reports one:
`{state, reason:{code,text}, latestMs, remainingMs, canSendNow}` exactly as
[Message state](request-response-v1.md#message-state) defines it, projected to exactly those keys (an additive Core field is not forwarded, and a malformed state is omitted). `canSendNow` is display only: the
owner's own browser sees Core's answer, every other grant sees `false`, and a `requests.sendNow`
from it is refused regardless.

### Send now

`requests.sendNow` `{requestId}` asks that a message the person sent be typed now instead of
waiting out the Digest hold. Only the owner's own paired browser may call it, and only for a request
this device sent; anything else is `REMOTE_SCOPE_DENIED` and Core is never asked. Remote carries the
ask: it resolves the person through `user.identity`, calls Core's `requests.sendNow` as that identity
and reads `requests.show` back. Core alone decides whether it may be sent and does the typing.

The reply is `{state, requestId, reason?, reasonText?, messageState?}`. `reasonText` is Core's own
sentence for `reason` (the table in [Send now](request-response-v1.md#send-now)),
forwarded as it is when it is one plain line of at most 256 bytes and left out otherwise, so no client
keeps a table of codes. It travels with Core's codes only (a refusal, or `DELIVERY_UNCERTAIN`), never
with a failure on this side, and `REMOTE_SCOPE_DENIED` has none. `state` is:

- `sent`: Core typed it. `already`: it had reached the agent, and nothing was typed again.
- `refused`: nothing was typed; `reason` is Core's code (`SEND_NOW_*`, `DELIVERY_AWAITING_APPROVAL`,
  `DELIVERY_CHANNEL_UNAVAILABLE`).
- `unconfirmed`: it may have been typed: `DELIVERY_UNCERTAIN`, or the call to Core failed on this
  side after the person was resolved (a timeout, an interruption, an I/O failure or an unreadable
  answer: `REMOTE_CORE_UNAVAILABLE`, `REMOTE_CORE_UNCERTAIN`, `REMOTE_IO`). Read `result` before
  asking again; Remote never retries.
- `unavailable`: nothing was typed: the person could not be resolved, or Core answered with some
  other error.

No journal row is kept; a repeat is answered from Core's own state, and the audit row
(`requests.sendNow`, the request as its resource) is written with the call's fence.

### Browser use and audit

The trusted UI that sends (the add-on shell, or an extension's trusted parent chrome on the door)
owns recipient selection, final message formatting and exact preview. It shows all characters, with
a separate escaped view for hidden controls; the SDK signs those same frozen bytes without adding
text after preview. Agent/content changes explicitly create new intent. Render replies and check
captures as inert plain text, not HTML. Core preserves stored message and applies its public
size/`!` transport protection; explain that adaptation, do not silently rewrite the reviewed
message. Refuse credentialed URLs rather than secretly dropping fields from the preview.

Effect signing is allowed only from trusted UI after an explicit gesture. An existing paired key
may sign bounded `session.open` re-admission without another gesture, from the same trusted
same-origin chrome; this cannot authorize an effect or widen a grant. The explicit gesture may authorize bounded
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
policy. The certificate exception never authorizes operations; paired-key `session.open` re-admission
follows the bounded same-origin rule above.

Remote keeps files only in its own subtree of the data root reported by `tmt api` operation
`storage.root`, with owner-only directories, 0600 secret/state files, no-follow bounded
regular-file admission and durable atomic state replacement. Never rewrite core DB/config or
provider settings. Local append-only audit records time, device/request/operation IDs, resource
UUIDs, digest, grant revision, decision and sanitized code before effects and outcome afterward.
In `audit.grant_revision`, 0 means no grant observed, allowed only for an unadopted unsupported cloud
carrier refusal with no logical operation ID or resources; a known peer records its real revision.
That metadata never confers authority.
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
`REMOTE_CLOSED`, `REMOTE_GRANT_CHANGED`, `REMOTE_SERVE_STOPPED`, `REMOTE_GRANT_EXPIRED`,
`REMOTE_INPUT_TOO_LARGE`,
`REMOTE_RATE_LIMITED` (optional retryAfterMs),
`REMOTE_CURSOR_EXPIRED`, `REMOTE_CORE_UNAVAILABLE` and `REMOTE_STATE_UNAVAILABLE` have the rules
above. Unsupported operation/profile/binding is REMOTE_INPUT_INVALID. Preserve permitted core
resource/errors, not a conflicting exchange engine. Pre-auth rejection is generic and cannot
authorize retries or reveal grants.

### Provisioning on start and pair

`tmt remote start` mounts every enabled extension and prepares its relay namespaces and owner-machine
bridge, and pairing a device makes it known to every mounted extension through the device context.
No extension adds its own sign-in, pairing, bridge enrollment or machine grant step. An extension
asks for an extension key certificate silently on first use.

### Current implementation and migration

Implemented today: a loopback door with human background startup and foreground JSON compatibility,
pure `local-v1` canonical envelope framing,
`tmt-device-pair-v1` enrollment and possession builders, pairing-code text decoding, four-word
fingerprints over the pinned list (bitcoin/bips `ce1862ac` `bip-0039/english.txt`, SHA-256
`2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`, committed as
`extensions/tmt-remote/rust/tmt-remote/assets/bip39-english.txt`), strict Ed25519/HMAC
verification and `K_response`/`serverProof` derivation in Rust, and the matching TypeScript
builders. Their fixtures
are regenerated from the independent oracle (#1039); the superseded M1 enrollment vectors are
removed and envelope bytes are unchanged. The pairing ceremony is implemented through
`tmt remote pair` and `/remote/pair`: one offer per run, the owner's terminal confirmation, the default
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

### Conformance and acceptance

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

## Extension channel API

Remote offers extensions the five services below, distinct from the
[host-driver authentication obligations](#host-driver-authentication). Device context, certificates,
events and mounting describe the local driver. Shipped Firestore uses its Auth/Rules gate and
[sealed-document client](#extension-scoped-sealed-document-client), not a paired-device wrapper.
Each service is consumed through Remote; an extension never reimplements one.

**Device context.** A request, upgrade or relay frame from a paired owner device reaches the
extension with the authenticated device context `{deviceId, kind, origin, name, publicKey,
owner:true, grantRevision}`, where `publicKey` is the grant's raw 32-byte Ed25519 device key as
canonical unpadded base64url, the only form remote forwards. The historical cloud-relay proposal attributes a non-owner
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
idempotently per `deviceId`: an equal revision causes no repeated authority effects; a name
snapshot may update presentation at the same revision. An older authority event never undoes a
newer one. A revoke at revision N wins over any rename at a lower revision.
Consumers retain the applied revision with device-bound state; they must not rotate epochs again
for a replayed revoke. Delivery is asynchronous: remote revokes authority and ends sessions before
acknowledging the local command, while extension cleanup may still be awaiting recovery. A rename
of a revoked device refuses, so its tombstone cannot become a name event.

`tmt remote devices rename <clientId> <name>` changes presentation only, preserving the device ID,
key, authority revision, Sessions, cookies and held work. A changed name wakes the device-event
worker without ending transports; repeating the current name is a no-op. Both rename and revoke
work through serve's control socket when running, or directly under the serve lock when stopped.

**Route mounting.** The door mounts each enabled, owner-installed extension under
`/r/<prefix>/x/<extension>/`, inside the machine's route prefix, forwarding HTTP requests, static
assets and WebSocket upgrades to the extension process over an owner-only local socket in the
extension's data subtree, together with the device context. Remote owns Host, Origin and CSRF
admission, framing and connection/body limits; the extension owns its responses, content security
policy and headers. All mounted extensions share one browser origin and therefore one browser trust
domain; mounting is limited to owner-installed extensions, and untrusted content renders only in
sandboxed opaque-origin frames. There is no mount space at the door root: `/x/` answers 404 without
a redirect; only the explicit public short entries below forward at the door root.
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
request, including upgrades, client-supplied `tmt-device-context`, `tmt-device-event`, `tmt-mount` and `tmt-origin`
headers are stripped. Only the door sets these headers: `tmt-mount` identifies the actual mount,
`tmt-device-context` is added only for an authenticated owner-device session, `tmt-device-event`
is reserved for the local callback above, and `tmt-origin` is the origin of an object-channel tunnel, below.

### Module descriptions and composed shell

`-` is reserved and can never be an extension or mount name. The door handles
`GET /r/<prefix>/x/-/shell` before extension lookup; it is not forwarded. It requires
an existing live owner-device session, rechecked after composition, and uses the existing
mount-space cookie. Missing or ended authority receives the generic 404. Display data grants
no page, operation or sending access.

On demand, Remote asks each enabled extension on its admitted owner-only mount socket:
`GET /.tmt/remote/describe`, with `tmt-describe: 1`, the actual `tmt-mount`, the door's
`Host`, no body and `Connection: close`. Browser traffic cannot reach the `/.tmt` subtree
or supply the callback marker. A callback has one second for connect/write/read and a
16 KiB body cap; only HTTP 200 with `application/json` is admitted.

The entire UTF-8 JSON object must match version 1; unknown or duplicate fields, missing
required fields, explicit null optionals, invalid strings or exceeded bounds refuse the
whole answer. Its fields are:

- `version: 1`.
- `rail: {label, icon, path, order}`: label 1–16 Unicode scalars; order 0–999.
- `menu`: up to 24 `{id, label, path, group?, count?}` items; label 1–40 scalars,
  group 1–24; count is `{value, kind}` with integer value 0–9999 and kind
  `needs-you` or `plain`.
- `settings`: up to 16 `{id, label, path}` items, label 1–40 scalars.
- Optional `needsYou`: integer 0–9999, equal to the module's own in-app Needs-you
  number. Absent means unknown and shows no badge; explicit null is refused.
- Optional `home`: up to eight `{id, title, path}` items, title 1–40 scalars.
- Optional `entry`: the path of the module's web app script, ending in `.js`. An entry that is not
  a valid path or not a script is dropped and the rest of the description stands; the app then
  offers the module's link instead of mounting it.

IDs match `[a-z0-9-]{1,32}` and are unique within their list. Labels contain no controls
or bidi controls and are text, never markup. Icons are names from `house`, `file-text`,
`monitor-smartphone`, `users`, `chart-column`, `search`, `settings`, `box`; an unknown
lowercase alphanumeric/hyphen name falls back to `box`, never an SVG or URL. Paths match
`^/[A-Za-z0-9._~/-]{0,255}$` with no `..` segment or `//`, query, fragment, URL or
escape. The door resolves every path inside that module's own mount, never another module.

The composed answer is `{version:1, modules:[{extension, rail, menu, settings, needsYou?,
home?, entry?}]}`. Remote adds its own module and sorts by rail order, then extension name.
After the existing authority checks, every due callback starts in parallel; composition
waits within one 300 ms deadline, not one wait per extension. This budget covers module
composition, not authority checks, which retain the blocking behavior of mounted routes. A successful answer is fresh for five seconds; a last good
answer remains usable for at most 30 seconds. Missing, unsafe, slow, malformed or
unknown-version answers never renew it. There is at most one in-flight callback per
extension; late results are cache-only, fenced to the socket incarnation. No retries,
poller or persisted cache exist; serve stop joins the finite workers. The browser shell renders
labels as text and resolves nothing itself; root navigation follows the
[public browser routes](#public-browser-routes). Remote displays extension-supplied card text,
including local page titles, as opaque display strings. It never parses, derives, stores or
forwards them beyond the owner-device shell answer. A module returns titles only for content its
owner can already read, and omits the title otherwise.

**Relay.** Remote carries opaque, namespaced logs for extensions and never decrypts or interprets
their payloads. A namespace is `<extension>:<path>` (for example `colab:<space>/<page>/<stream>`).
Operations are append with create-only per-stream sequence (an exact retry returns the original
receipt), subscribe from a scoped opaque cursor with bounded catch-up paging, ack as a cursor
checkpoint only, an object store keyed by object ID with chunked transfer, and an ephemeral
awareness lane that is never stored. On the `local` backend an extension may instead serve its own
relay namespaces behind its mounted WebSocket; remote then only splices bytes and the extension
keeps admission.

**Superseded cloud-relay proposal, not the shipped Firestore path.** The Remote-run relay and
synchronous admission-hook model below are retained for its bounds and no-effect guarantees;
its extra authentication gate is superseded by [one gate per driver](#host-driver-authentication).
The remote-run relay proposal applies to cloud backends. Before accepting an append, subscription, object transfer or
awareness frame, remote calls the extension's synchronous admission hook with the device context
and frame metadata; the extension decides membership, role, epoch and writer checks. Revocation terminates live subscriptions. Remote
enforces per-object size caps and per-namespace quotas and expiry declared by the extension. Relay
ack is never core X acknowledgment, task completion or send.

**Operations and agent status.** Extensions send agent work only through
[Operations](#operations) with the device context of the user who acted, a caller-frozen operation
ID and exact bytes. Comments, sync, replay and rendered content never dispatch. An extension ledger
maps onto the operation receipts and `operation.show` found/not-found recovery; results are
readable only for operations its device owns. Agent delivery status comes from `agents.list`.

**Extension backends.** Declaration admission, Firestore extension deployment and Colab sealed-document
sync are implemented. Extensions use the single [backend declaration contract](#extension-backend-declarations)
and [authorized deploy](#authorized-deploy). The proposed cloud agent operation resources are separate;
page sharing does not activate them.

Principals stay separate. Local owner devices are paired through Remote and are the only principals that
can call local operations. People an extension shares with (page members, link holders) are never paired
by remote. On shipped Firestore, Firebase Auth and deployed extension Rules authenticate/authorize them;
Remote adds no device proof or admission hook in front of that gate. On the local door, a mounted-route request without an owner-device session is forwarded to the
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

### Owner browser reads

Three owner-session, read-only door GETs give the web app real Ops, limit-history and Digest data. They reuse the
public JSON the terminal already reads; they add no grant, scope, pairing rule, write or push
channel, and dispatch stays unreachable from them. They are siblings of the composed shell's
`GET <prefix>/x/-/shell` and share its admission:

| Route                             | Reads (fixed argv through the supplied `tmt`, no generic command endpoint)          |
| --------------------------------- | ----------------------------------------------------------------------------------- |
| `GET <prefix>/x/-/ops`            | `ops squad ls --json` and `ops squad cron ls --json`, run in parallel               |
| `GET <prefix>/x/-/limits`         | `ops limits --json`                                                                |
| `GET <prefix>/x/-/digest/<range>` | `digest usage report --since <range> --json`, with `<range>` `24h`, `7d` or `30d`  |

The range is a path segment because the edge target grammar admits no query string beyond its two
fixed forms; any other path, a bare `/digest`, an unknown range, another method or an upgrade is the
generic 404 (a query string is refused by the edge before any session check). Cross-site
`Sec-Fetch-Site` is 403. A request needs a live owner-device session (the mount-space cookie); the
same device and grant revision are rechecked after the read and a changed or ended session gets the
generic 404, never the data. Replies carry the door's security headers (`no-store`, `nosniff`).
Remote reaches Ops and Digest through the public CLI only, with no crate or state-file dependency.
A read writes nothing in an initialized store. The first read on a fresh machine performs the same one-time,
idempotent initialization as opening the board (the core store, Ops's path markers and the owner's saved
identity through `user.identity`, which the [extension API](extension-api.md#the-persons-identity) scopes to
owner-authenticated sessions).

Every answer is HTTP 200 with one JSON document:

```
{"version":1,"state":"ok","observedAtMs":<ms>, ...payload}
{"version":1,"state":"unavailable","observedAtMs":<ms>,"reason":<reason>}
```

`reason` is one of `not_installed` (the extension command is absent: a fixed argv exits with
`USAGE_ERROR`; an Ops without `limits` is also absent), `core_unavailable` (spawn, interruption or I/O failure), `timeout`, `read_failed`
(non-zero exit, output that is not one JSON document, an unexpected `version` or, for Digest, a
report for another range), `too_large` and, for limits only, `not_sampled` (the board has kept no samples). An unavailable read is never an empty list and never
sample data; each reason is one empty state in the web app, and a client treats an unknown reason as
a generic unavailable. `observedAtMs` is the owner's clock when the read finished.

**Ops payload** (`state:"ok"`) is an allowlist projection, not the board document:

- `squads[]`: `name`, `layout?`, `lead? {id,name}`, `idleAlertMs?` (only when the squad's idle alert is
  enabled), `attention? {state,waiting,blocked}`.
- `agents[]`, the squad's lead first (`role:"lead"`) then its members (`role:"member"`): `id`, `name`,
  `squad`, `role`, `presence?`, `activity? {state,sinceMs}` (the four-state rule of `agents.list`),
  `state?` (the board's raw state text, at most 80 scalars, so free text stays readable), `stateTone?`
  (the board's colour token for it, `[a-z0-9-]` up to 24 bytes; a client maps tokens it knows and treats
  the rest as neutral), `task?`, `next?`, `model?`, `link?` (`https://` only, no whitespace),
  `pending?`, `waitingOnYou` (a count), `digest? {active,remainingMs,heldCount}` and `staleness?`.
- `timers[]`, or `null` when the schedule read failed: `id`, `squad`, `scheduleText`, `owner?`, `state?`,
  `nextMs?` (the first only) and `what?` (the first line of the job message).
- `truncated`: true when a bound below cut `squads`, `agents` or `timers`.

Never forwarded: a pane's id, target, cwd or location, process data, room IDs, owner IDs, unlisted row
fields, the rest of a schedule message and the person's own identity. Text has control and
bidirectional-control characters removed (controls become spaces), is trimmed and is cut to the limit
below; the browser renders it as inert text.

**Limits payload** (`state:"ok"`): `limitHistory[]`, the weekly-limit samples the board keeps in its local cache,
as printed by `tmt ops limits --json` (a read-only command; it needs no Core and never writes): `provider` (a driver
ID), `windowMinutes` (10080) and `points`, oldest first, each `[observedAtMs, percentLeft, resetsAtMs]` with the
percentage rounded to two decimals; the reset time rides with every point because the kept 8 days can span two
windows. The cache has at most 8 providers and 10-minute buckets, and the board samples only while it is open, so
history has gaps and a machine whose board never ran has none: that answer is `not_sampled`, never an empty list.
The short window is not served.

**Digest payload** (`state:"ok"`): `{since, report}` where `report` is the
[usage report document](../extensions/tmt-digest/contracts/usage-v1.md) unchanged. Digest keeps the
document's contents (names, UUIDs, token counts and content-free exchange metadata). The web app
derives a shorter window from the `24h` hourly `series`; there is no 5-minute read and no turn count.

**Bounds.** The door never schedules a read. One request at a time runs a given route; concurrent
requests wait for it (at most 12 seconds in total, then `timeout`), and a finished answer, ok or
unavailable, answers every request for the next 5 seconds, so any number of tabs cost at most one read
per route per 5 seconds. Nothing runs between requests and nothing is persisted. Each subprocess
captures at most 2 MiB of stdout; an answer is at most 512 KiB (Ops, limits) or 1 MiB (Digest) and a larger
one is `too_large`. Ops projects at most 32 squads, 256 agents and 64 timers; text fields are cut to
64 scalars (names, model), 80 (state, schedule text), 200 (task, next, pending), 300 bytes (link) and
120 scalars (`what`). The browser refreshes only while its route is visible and at the terminal's own
cadence: Ops every 5 seconds (the board's default reload) and Digest every 60 seconds, backing off
after failures. The SDK owns that timer and stops it with the route; the door has none.

**Vectors.** [`owner-reads-v1.json`](owner-reads-v1.json) holds, per case, what the public commands
print and the exact answer, the reasons, the strings that must appear nowhere in an answer and the
bounds. The Remote tests run every case through the door's reader and compare the reason set and the
bounds with the code.

### Object channel frames

**Status:** Remote ships the wire schema, Unix carrier, callback executor and lease-bound backend below.
The static production registry enables only Colab; no other extension declares objects, and no route, SDK setting
or user action enables them. Serve may open an empty ledger, but without a Colab adapter's current admission every
object operation is refused before a ledger effect. Transport-only negative fixtures do not establish Colab routed
acceptance: #1852 still requires Colab's upload/read proof on shipped binaries. Serve attempts each Local declaration
before door readiness. Subsequently, a validated Local websocket upgrade with an absent or ended channel joins one
bounded reactivation attempt; healthy channels are unchanged.
Failure forwards the upgrade without an origin, preserving page sync; an extension that needs objects on such a
connection may reconnect after the 1 s cool-down, and each new upgrade joins a fresh bounded attempt. Channel end, assets and status never trigger
setup. New generations inherit no old origins; no object operation is replayed. On an opened channel it answers `config` after current `acquire` and `disclose`
admissions, for a local-extension origin (the owner's limits) and for a mounted origin that stands (the reduced browser
limits, whether or not an owner session is bound). It also answers observational original-ID `status` and bounded raw-byte
`read` after fresh acquire and disclose admissions, with an exact current-context, metadata, cancellation and absolute
request-budget fence before disclosure. Original lookup is scoped by installed extension, caller transfer ID and actual
owner device, local extension or non-owner connection origin; a non-owner reconnect has a new scope. Status never adopts
or repairs an original. The service also answers `begin`, `part`, `commit` and `discard` through the existing local
backend algorithm after fresh acquire and effect admissions, then disclose admission for successful results. Later
upload callbacks use the scoped original's frozen input, and staging work clips its budget to the original expiry,
without renewal. A connection-scoped upload cannot be recovered after reconnect; its retained row stays charged and
bounded by staging expiry and quotas. A mounted request whose origin does not stand is answered `denied` without a
callback before an effect; loss after invoking a mutation is `unknown`. Requests, results
and admission replies carry no principal, role, permit, retry or scope: `method`, the result tag and the callback
identifiers are correlation only. An admit `context` states the owner device and grant revision as Remote established
them, for the extension's own decision; a browser or caller never selects it, and nothing in a context, identifier,
digest or origin grants authority or proves who may use a value.

**Legacy object framing.** A version-1 or version-2 frame is a 4-byte big-endian length (2 to 65,536) and that many bytes of one strict UTF-8 JSON object
without duplicate member names (compared after decoding escapes), unknown or missing members, trailing bytes, nesting
deeper than 8, or any number that is not an unsigned integer of at most 2^53-1 without sign, fraction or exponent.
Counters are canonical decimal strings spanning `u64`. UUIDs are lowercase version 4. Bytes are unpadded base64url
(canonical: no padding, standard alphabet or nonzero trailing bits), digests 64 lowercase hex digits, and every value
re-encodes to the same text. Structure faults (unknown or missing member, wrong JSON type, unknown `kind`, `method`,
origin kind, context kind, disclosure class or result tag) and value faults (a spelling, range or association outside its grammar) are distinct
classes. Encoding applies the same checks, in the canonical member order shown.

**Legacy carrier handshake.** Remote opens the channel on an extension's owner-only `door.sock` with one HTTP/1.1 request and the
extension answers with one reply; nothing else is sent before the reply. This is not RFC 6455 and not the extension's
browser routes.

```http
GET /.tmt/remote/object-channel-v1 HTTP/1.1
Host: <the exact admitted mount host>
Connection: Upgrade
Upgrade: tmt-object-channel-v1
Content-Length: 0
tmt-mount: <the exact selected mount>
tmt-object-channel: <1 or 2>
tmt-object-generation: <lowercase UUIDv4 Remote just generated>
```

The acceptor requires this method, this exact path (a query is a different path), `HTTP/1.1`, and exactly these seven
fields, each once in any letter case and in any order, with these values; any other field (a cookie, `Transfer-Encoding`,
a `Sec-WebSocket-*` field, a device context, an origin), a repeat, a missing field, a nonzero length or bytes after the
head is refused. A head is at most 8 KiB and 32 fields. The reply is `HTTP/1.1 101 Switching Protocols` with exactly
`Connection: Upgrade`, `Upgrade: tmt-object-channel-v1` and the same `tmt-object-generation`, and no body; the initiator
refuses any other status, field or generation. Remote first offers version 2; only a failure before a complete reply
head permits exactly one fresh connection offering version 1, within the same absolute setup budget. No complete
reply head (including a refused status or 101) permits fallback. A failed second attempt returns the existing failure;
no downgrade is cached between connections. Acceptors admit either version and otherwise retain silent refusal. Every later frame names
that generation, and a frame of another generation ends the channel. The extension reads the head within 2 s and writes
the reply within 1 s, both clipped by the setup bound the caller holds.
An acceptor whose router already read the head, through the blank line and within the router's own bound, to dispatch
on its path applies the same checks to those bytes and writes the same reply, and bytes the router read after the head
are refused as pipelined. `Host` and `tmt-mount` are the same values the door sets on every request it sends that
extension on its owner-only socket: `Host` is the admitted door host, as in the device-events callback, and `tmt-mount`
is the actual mount name. The acceptor takes both from what the door already gives it, never from the request being
validated, so both ends compare equal values.

**Frame waits.** A frame is complete 2 s after its first prefix byte, with no renewal for partial progress; the length is
checked before any allocation. Waiting for a frame to begin has no bound of its own and ends on a byte, the end of the
stream, a stop request or the caller's deadline. A frame is written within 1 s. A partial prefix or body, an end of the
stream inside a frame, a stalled write or an invalid frame ends the channel; none is retried.

**Direction and correlation.** The extension sends requests and admissions; Remote sends results, admits and origin
states. Each end applies every frame, sent or received, to one ledger before it is sent or queued. Request and callback
identifiers are strictly increasing per issuer against a remembered high-water mark, so none is reused. At most 8
requests and 8 callbacks are outstanding per channel (32 and 32 per installation, counted by the caller's shared budget
and returned when a channel ends). A request has at most one callback outstanding and ends only with its result when none
is. An admit must name an outstanding request with the same method and transfer; an admission must name the outstanding
callback and its request; a result must repeat the request's method and transfer. A frame the local end may not send, or that the
ledger refuses, is returned to the caller with nothing written and the channel stays usable. A received frame the ledger
refuses ends the channel, so a duplicate, stale, mismatched or out-of-order frame never satisfies a successor.

**Bus.** One thread reads while callers wait, and reading a frame spawns no thread. Received frames wait in a queue of at most 8
frames and 524,288 bytes; reading pauses while it is full, and the peer's own write bound ends a peer that outruns it.
Frames received before a fault are delivered before it is reported. The first fault is kept and every later call reports
it. Closing, or dropping, shuts the socket down, wakes every waiter and joins the thread.

**Origins.** Remote sets `tmt-origin: <originId>` only on a WebSocket upgrade to an extension that declares objects and has
an active channel, exactly once, as one lowercase canonical UUIDv4 it just generated, and never on any other request. A
connection without the header never gets object work: the extension denies it and the browser reconnects. An origin is
Pending while Remote forwards the upgrade, is established only after the owner session is attached, the browser has its
`101` and the tunnel runs, and ends at the first close: a close that wins is never followed by `established`. Every
upgrade is a new origin, so a reconnect, including within the session's reload grace, gets a new id, and no object work
carries across origins. An origin belongs to one channel generation; ending or replacing the channel ends all its origins
and a successor starts with none. `originId` is not a capability: only the extension, on the private bus, can name it, and
Remote's registry, not any frame, is authoritative when a request arrives and when its result is about to leave. Origin
state frames are lifecycle notices written in order by one sender, with a bounded backlog (an `established` and a `closed`
per tunnel); a write that fails or stalls, or a backlog past its bound, ends the channel and with it every origin, so a
late `closed` is safe and nothing relies on one. Closing an origin never waits on a socket or on the session owner.

**Mounted origins stand only while Remote says so.** A request naming a mounted origin is answered `denied`, with no
callback, unless that origin is established on this channel generation in Remote's registry and, when an owner session was
attached to its upgrade, the session owner confirms that same session is still live with the same device and grant
revision (not ended, idle-expired, revoked or replaced); an unknown, pending, closed, other-extension or other-generation
origin, or a session owner that cannot answer within its short bound, denies. The `acquire` and `disclose` callbacks name
`owner-session` (with that device and grant revision) or `mounted`, never what the browser or the extension chose. The
origin is checked before `acquire`, again before `disclose`, and once more before the result leaves, so an origin that
closed or a session that ended meanwhile discloses nothing. Every mounted origin sees the reduced limits.

**Admission outcomes.** Remote asks the extension afresh for every request and remembers no decision for a later one.
Before an effect, `deny` is answered `denied` and an `unavailable` decision `unavailable`; neither, and no missing
answer, is ever treated as an allow. After a mutating backend call is invoked, lost authority, cancellation, a spent
budget or disclose deny/unavailable is bare `unknown` under the normal write bound, and the channel keeps serving.
There is no rollback, charge release or retry. Mutating backend invalid, conflict and capacity errors pass through
as their wire classes, without claiming storage is unchanged; missing, unavailable, cancellation and deadline are
conservatively `unknown` (not-found is read-only). Original-ID status observes the retained outcome. A request whose own 30 s is spent before a callback is sent, whether it waited in the queue or used its time
between the two admissions, is answered with bare `unavailable` under the normal write bound without that callback,
and the channel keeps serving. Config and observations share this rule: no late data; a spent pre-admission result write
also falls back to bare `unavailable`. A frame write failing after admission ends the channel. Only a callback
that was sent and is left unanswered past 5 s, or past the request's remaining time, ends the channel and sends no result,
because a result is refused while a callback is outstanding. A request is read by one dispatcher that never waits for a
decision, so decisions are delivered while workers wait for them.

**Request** `{"version":1,"kind":"request","generation":<uuid>,"requestId":<counter>,"origin":<origin>,"method":<method>,"input":<input>}`,
with `origin` either `{"kind":"local-extension"}` or `{"kind":"mounted","originId":<uuid>}`.

| `method`          | `input` members, in order                                                                   |
| ----------------- | ------------------------------------------------------------------------------------------- |
| `objects.config`  | `namespace`, `policyInput`                                                                  |
| `objects.begin`   | `transferId`, `namespace`, `opaqueKey`, `policyInput`, `payloadSha256`, `payloadBytes`      |
| `objects.part`    | `transferId`, `index`, `bytes`                                                              |
| `objects.commit`  | `transferId`                                                                                |
| `objects.status`  | `transferId`, `namespace`, `policyInput`                                                    |
| `objects.read`    | `namespace`, `opaqueKey`, `policyInput`, `payloadSha256`, `payloadBytes`, `offset`, `count` |
| `objects.discard` | `transferId`                                                                                |

`namespace` and `opaqueKey` are 32 bytes (43 characters). `policyInput` is opaque, at most 2,048 decoded bytes (the
bound of the backend's policy binding). `payloadBytes` is at most 12,582,912. `bytes` of a part is 1 to 32,768 decoded
bytes. `index` is any `u32`: it does not depend on the transport ceiling of 32,768 or on a smaller backend part size
(3,071 or u32 max are valid; 2^32 is not), and the backend alone enforces part order, full parts and the final
remainder. `count` is 1 to 32,768, and `offset` is below `payloadBytes` (an empty payload reads from 0).

Version 1 supports exactly the seven calls above, with unchanged bytes. Version 2 also supports:

| Method                     | Input members                                         |
| -------------------------- | ----------------------------------------------------- |
| `objects.remove-namespace` | `namespace`                                           |
| `objects.usage`            | `scope`: `extension`, or `namespace` plus `namespace` |

Both calls require a local-extension origin; any mounted origin is denied before callback, effect or disclosure.
Callers check the negotiated link's support before either call; an unsupported call is never written on version 1. A version-1
link reports: "Restart Remote serve to enable this (tmt remote stop && tmt remote serve); if it still
fails, upgrade Remote." Support comes only from the negotiated link, never an installed receipt or CLI.
Removal is idempotent for unknown or already removed namespaces. It commits the existing namespace fence, drains
in-flight work and retains deleted original identities; a loss after invocation is `unknown`, without replay or rollback.
Usage is the existing ledger projection for the selected namespace or this extension, never the installation; its charge is rounded payload plus retained records and fences, plus the fixed tree base once if any namespace exists
for extension scope, not “bytes stored”; cleanup-pending bytes stay charged until removal is confirmed.

**Result** `{"version":1,"kind":"result","generation":<uuid>,"requestId":<counter>,"method":<method>[,"transferId":<uuid>],"ok":{...}|"error":{...}}`
has exactly one of `ok` and `error`. `transferId` repeats the original transfer on every result and error of `begin`,
`part`, `commit`, `status` and `discard`, and is absent for `config`, `read`, `remove-namespace` and `usage`. `ok` carries a `result` tag:

| `result`    | Further members                                                                                               | Answers                      |
| ----------- | ------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| `config`    | `projection` (`browser` or `local`), `backend`, `capabilities`, `limits`                                      | `config`                     |
| `pending`   | `nextIndex`, `received`, and `expiresAtMs` only for `status`                                                  | `begin`, `status`            |
| `progress`  | `nextIndex`, `received`                                                                                       | `part`                       |
| `committed` | `opaqueKey`, `payloadSha256`, `payloadBytes`                                                                  | `begin`, `commit`, `status`  |
| `state`     | `state`: `expired`, `discarded`, `unavailable`, `unknown`, `notObserved` (`discard` answers only `discarded`) | `begin`, `status`, `discard` |
| `read`      | `offset`, `totalBytes`, `bytes` (0 to 32,768 decoded bytes, within `totalBytes`)                              | `read`                       |
| `removed`   | none                                                                                                          | `remove-namespace`           |
| `usage`     | `chargedBytes` (0 to 2^53-1), `entries`, `activeUploads`, `retainedIdentities` (each a `u32`)                 | `usage`                      |

A pending `status` answer carries the actual stored expiry; a pending `begin` answer has none and never invents one.
`config` shows the server-selected backend (`{"id","source":"default","editable":false}`), its three boolean
`capabilities` (`immutableCreate`, `chunkedRead`, `recoverByOriginalId`) and `limits`: `payloadBytes` and `chunkBytes`
(the backend's canonical part size, never clamped to the transport) for `browser`, plus `namespaceBytes`,
`extensionBytes` and `installationBytes` for `local`. An empty generic read or payload is valid and is unrelated to a
consumer's own envelope or authority.

`error` is `{"code":<code>}` with `denied`, `unavailable`, `invalid`, `conflict`, `capacity`, `not-found` or `unknown`,
and a `limit` member exactly for `capacity`: `namespace-bytes`, `extension-bytes`, `installation-bytes`,
`namespace-entries`, `extension-entries`, `installation-entries`, `active-intents`, `retained-extension`,
`retained-installation` or `requests`. `not-found` answers only `read`; `unknown` only `begin`, `part`, `commit`, `discard` and
`remove-namespace`, where an effect may have happened. No error carries message text, and none proves that a possibly published
effect did not happen.

**Admit** `{"version":1,"kind":"admit","generation":<uuid>,"callbackId":<counter>,"requestId":<counter>,"boundary":<boundary>,"context":<context>,"operation":<operation>}`
asks the extension to decide at one boundary of one request; `requestId` is the request's own counter. `boundary` is
`acquire` (before work starts), `effect` (before a durable change; only `begin`, `part`, `commit`, `discard` and `remove-namespace` have one)
or `disclose` (before a result leaves). `context` states how the request actually arrived, as the service established it
and never as a browser selected it: `{"kind":"owner-session","originId":<uuid>,"deviceId":<uuid>,"grantRevision":<n>}`
(`grantRevision` 1 to 2^53-1), `{"kind":"mounted","originId":<uuid>}` or `{"kind":"local-extension"}`. `operation` is
`{"method":<method>,"input":<input>[,"disclosure":<disclosure>]}`, and `disclosure` is present exactly at `disclose`.

For `config`, `begin`, `status` and `read` the admit `input` is the request input. For `part` it is
`{"transferId","index","length","retained"}` with `length` 1 to 32,768, and for `commit` and `discard`
`{"transferId","retained"}`; `retained` is what the transfer froze, `{"namespace","opaqueKey","policyInput","payloadSha256","payloadBytes"}`.
For `remove-namespace` and `usage`, admit input repeats the request input. Their disclose classes are `removed`
(with no further fields) and `usage` (the same four projection fields as the result).
No callback carries the bytes of a part or of a read: a `bytes` member is an unknown member.

A `disclosure` names the exact class of result about to be disclosed, with the values but never the bytes. Its `class`
decides its members and the methods that may produce it, the same association as the result tags above:

| `class`    | Members                                                  | Result it announces | Methods                      |
| ---------- | -------------------------------------------------------- | ------------------- | ---------------------------- |
| `config`   | `projection`                                             | `config`            | `config`                     |
| `status`   | `nextIndex`, `received`, `expiresAtMs` only for `status` | `pending`           | `begin`, `status`            |
| `progress` | `nextIndex`, `received`                                  | `progress`          | `part`                       |
| `receipt`  | `opaqueKey`, `payloadSha256`, `payloadBytes`             | `committed`         | `begin`, `commit`, `status`  |
| `terminal` | `state`                                                  | `state`             | `begin`, `status`, `discard` |
| `bytes`    | `offset`, `length` (0 to 32,768)                         | `read`              | `read`                       |

`discard` discloses only `terminal` with `discarded`. A disclosure never turns an unknown, expired or unavailable state
into permission to retry or into proof that no earlier effect happened.

**Admission** `{"version":1,"kind":"admission","generation":<uuid>,"callbackId":<counter>,"requestId":<counter>,"decision":<decision>}`
answers one admit. `decision` is `allow`, `deny` or `unavailable`, and nothing else: the reply has no permit, principal,
role, target or scope member, and `unavailable` is neither permission nor a statement that no effect happened.

**Origin state** `{"version":1,"kind":"origin-state","generation":<uuid>,"originId":<uuid>,"state":<state>}` announces that
an origin is `established` or `closed`.

## Backends and deploy

Message framing, authentication and admission are scoped to their host driver. The `local` door and extension
stores are implemented, as are Firestore extension declarations, deployment and sealed-document
sync for Colab page sharing. The native Firestore **agent operation** binding below is emulator-only;
native invite/join is implemented, while final public Core delivery alignment remains pending. The admission-service proposal and
historical cloud conformance remain superseded. Native encryption and its automatic key lifecycle
are independent of production enrollment. Their specification
never enables agent access; the [historical conformance material](#proposal-cloud-conformance) is retained
under the [superseded-authentication note](#superseded-cloud-authentication-proposal), not a shipped Firestore gate.
Cloudflare remains proposed. Extension/page authority never substitutes for a local paired operation grant.

| Backend      | Mapping                                                                                                                                                                             |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `local`      | [Loopback HTTP](#transport-binding-loopback-http), mounted extension routes and WebSocket relay; state under the data root.                                                         |
| `firestore`  | **Shipped:** extension-scoped sealed-document sync for Colab page sharing. **Emulator-only agent binding:** native invite/join, encrypted inboxes, owner responses and peer checkpoints; public Core delivery remains gated. |
| `cloudflare` | **Proposal:** Worker admission before per-machine Durable Object storage; per-namespace relay objects; bounded WebSocket delivery and device-scoped checkpoints.                    |

### Native Firestore agent binding (emulator only)

`cloud_inbox` carries `cloud-hpke-v1` envelopes through the common `MessageService`, operation owner,
Store and journal. Firebase Auth/Rules admits access to `/m/<machineId>`, separate from every `/x/`
extension root. Confirmed native peer keys authenticate encrypted application content, not a second
Firestore login or local browser pairing ceremony. Scope, agent allowlist, hold, current revocation and
Core effect uncertainty retain their existing owners; cloud ingress never receives owner-browser powers.
A verified cloud request does not require the local door's ephemeral window, session or receive-time
freshness. That local gate is unchanged. Cloud timestamps must instead be within 60 seconds of the
carrier document's output-only provider `createTime`; offline owner wake-up is not the freshness clock.
An unprocessed carrier expires after `journal::RECOVERY` (30 days), with `REMOTE_CLOUD_MESSAGE_EXPIRED`.

Inbox and response paths are `/m/<machineId>/inbox/<operationId>` and
`/m/<machineId>/responses/<operationId>`. A document contains exactly `peer`, ciphertext `carrier` bytes and server
`createdAt`. This Rules-pinned server request time is only the query/order key; the codec sanity-checks
it within the existing 60-second band of output-only `createTime`, which alone establishes freshness.
The operation ID is a lowercase UUIDv4. There is no chunking, manifest, plaintext fallback
or provider-side operation authority. One carrier fits `firestore_limits::DOCUMENT_PAYLOAD_BYTES`;
`limits::CLOUD_ENVELOPE_BYTES` is the largest conservative envelope cap below that bound. The prepared
message cap also reserves worst-case JSON escaping/base64. Oversize refuses as
`REMOTE_CLOUD_MESSAGE_TOO_LARGE` before a write. Rules admit only the registered Firebase uid in its
machine inbox via its owner-written peer routing row, the receiving owner in responses, exact fields/size
and create-only storage. Exact-ID reads require the document's peer to match the caller's routed peer
(or the machine owner). A routed peer may also GET a missing ID (404), so observation can wait for a
response; a non-routed uid cannot. Response listing is denied. Neither writer
may update a carrier. Senders never delete. The receiving owner removes retained inbox/response pairs
after journal retention; the one transport table is pruned only after that cleanup. Native enrollment creates machine/peer registrations through ordinary Auth and owner-only Rules;
the carrier-only fixture may seed them independently. No real-project acceptance is claimed.
The owner-written routing row is `/m/<machineId>/peers/<Firebase-uid>` with `{peer,revoked,expiresAt}`;
create and exact GET each use one routing lookup. The local Store maps native peers to uids, revocation fences all of that peer's uid rows in the same
operation, and at most one route per native peer is live (a new one supersedes the old). Lost or renewed
Firebase credentials re-route from the signed native identity automatically, without a new pairing step.

The driver uses ordinary Auth, HTTP, cancellation and the persistent project budget shared with native
sealed-document sync. One shallow query of the selected machine's inbox, ordered and filtered by
`createdAt` and document ID, covers all native peers. Each reconciliation starts with a one-document
probe; only a full page permits follow-up pages, sized from retained transport rows and capped at
four documents. These rows are a sizing hint, never permission to skip an unseen timestamp or ID.
An empty idle inbox uses one query per minute, reserving one document plus the existing Rules lookups.
Rules allow only the receiving owner to list inbox pages with a limit from one through four;
enrollment retains its one-document limit and responses remain unlistable. Each page's retained
REST response is bounded by its requested document count times `limits::CLOUD_PROVIDER_BYTES`.
No collection-group scan or recursive wildcard is used. The machine query retains one completed
provider timestamp with a 120-second overlap and a durable exact-timestamp/ID continuation after
each durable outcome. A transient failure or capacity stop resumes that prefix; only a fully drained
bounded reconciliation advances the completed timestamp. Enrollment has a separate checkpoint. The existing journal handles repeated IDs and effects. Store/serve
lease ownership is unchanged: one additive `cloud_transport` table freezes exact request bytes/digest,
response ciphertext and publication attempt before HTTP. An ambiguous create is observed at that same
ID, never automatically re-posted or resealed. An already-attempted response publication keeps its
grant fence and goes directly to exact-ID observation, without reserving another commit that cannot
be sent. Its missing or failed observation remains unconfirmed. Request maxima are never refunded.
A frozen outbox row without a durable send claim makes
its first create directly, without a preparation GET; the usual Auth/budget/claim fence still runs.
Changed bytes under an ID refuse. A duplicate returns
only frozen response ciphertext. Context secrets stay in memory for the bounded in-flight call; after
sender restart, observe the original request ID, then use a fresh signed `operation.show` request and
new HPKE context to read the original journal outcome. The old encrypted response remains unread until
owner retention. Recovery never repeats the dispatch effect or reuses the lost context's key/nonce.

Transient network, Auth refresh, budget or Store failures before the response-publication claim
leave the checkpoint unchanged and retry on the next bounded poll with native backoff. Once that
attempt is durably claimed, an ambiguous or missing observation keeps its frozen reply unconfirmed
but lets the cursor and checkpoint advance, so a later fresh signed ID lookup can reach the journal.
The existing attempt flag survives restart; neither the mutation nor its response is retried or resealed.
The journal preserves an internal typed distinction between an absent/foreign/expired owned read
and a real Store failure. Only the cloud classifier consumes that distinction: the former is a
per-document permanent refusal, the latter remains retryable. Both keep the existing
`REMOTE_STATE_UNAVAILABLE` wire answer; no existence distinction or marker is exposed.
`REMOTE_CLOUD_PROVIDER_UNAVAILABLE` includes sanitized `error.provider` evidence: fixed `operation`
(`inbox-read`, `response-read`, `inbox-query`, `inbox-create` or `response-create`), numeric HTTP `status`,
and optional `errorStatus` from the canonical Google RPC status enum. Unknown status text is omitted;
provider body/message, token, carrier and resource identifiers never cross this diagnostic boundary.
`REMOTE_CLOUD_DOCUMENT_INVALID` carries a closed `error.reason`: `query-shape`, `name-shape`,
`fields-shape`, `peer-shape`, `create-time`, `indexed-time`, `time-delta`, `encoding` or `size`.
The reason contains no provider values or identifiers. Existing
`REMOTE_INTENT_CONFLICT` and `REMOTE_CURSOR_EXPIRED` are also per-document permanent refusals.
Permanent malformed, undecryptable, foreign, retired-key,
revoked or expired carriers receive one immutable refusal code in the same transport row and existing
audit transaction. They grant no operation authority and are de-duplicated across restart; a durable
refusal lets the checkpoint advance without stopping other peers. Where decryption provided a safe
response context, the owner freezes and publishes a signed refusal. An undecryptable carrier has no
response. A previously frozen accepted response is never replaced or resealed under a revoked
context. A no-response in-flight observation ends as `REMOTE_CLOUD_UNCONFIRMED`: it may still be
processed, so observe the same request ID and never resend. Only immutable request age reaching
RECOVERY is `REMOTE_CLOUD_MESSAGE_EXPIRED`.

`serve` composes the lease-owning native pairing service with the existing message service. One
per-project `Projects` owner shares Auth, HTTP, budget and cancellation with native extension sync;
explicit invite/join may sign in, while background recovery and sending never create an identity.
A replaced persisted Firebase uid gets a new Auth owner and reuses the same runtime/budget. Existing
extension consumers retain their old stopped identity, so another uid inherits no page membership.
With no enrolled cloud peer, serve starts no provider polling and spends no cloud budget. Its one
joined worker reconciles enrollment, route cleanup and the existing inbox driver; stop cancels I/O,
joins handlers/worker and flushes the common project budget before releasing the serve lease.
An idle machine uses one inbox and one enrollment query per minute. Each is conservatively charged
1 read plus 2 Rules lookups: 8,640 modeled reads/day/machine, 17.28% of Spark's 50,000 reads/day,
plus bounded hourly cleanup and descriptor renewal. These are modeled charges, not emulator wire
counts or observed headroom. All enrollment, routing and operation traffic uses the same daily guard.
Reopening the same Store resumes its native identity, trust, permissions and checkpoints.

The provisional public boundary is `tmt remote message --json`, one bounded version-1 JSON document
on stdin/stdout. `cloud_command` implements [Remote message v1](remote-message-v1.md), which owns the request and
answer schema, including the local-only `reply` handle. `target.name` is required (1–128 UTF-8
bytes, no controls); `target.id` may be omitted, but null or a noncanonical UUID is refused. The
sender seals and echoes the target exactly as sent, including an omitted ID.
The receiver resolves a bare name through Core's public `identity show`
lookup to one live local identity; no match (`REMOTE_AGENT_NOT_FOUND`) or ambiguity
(`REMOTE_AGENT_AMBIGUOUS`) is a durable per-document refusal. Before adoption, it freezes that UUID
and current name in the existing dispatch intent, applies the same recipient grant, and retains the
UUID-pinned expected-name check before effect. Recovery uses the frozen UUID, never a new name lookup.
An explicit target UUID keeps its existing pinned path and treats the name as the expected name.
The sender needs no directory or extra provider read. States are `sent`, `held`, `refused`,
`unconfirmed`, `unavailable`. Core owns preparation,
receipts and retention; `createdAtMs` is the immutable original creation time in integer milliseconds,
identical on every repeat. A request at least `journal::RECOVERY` old refuses with
`REMOTE_CLOUD_MESSAGE_EXPIRED` before any write; a future time beyond the 60-second skew bound refuses.
The signed dispatch carries that same creation time; the receiver checks it again before adoption,
so owner downtime cannot extend a queued request's lifetime.
A repeat must retain the exact prepared bytes, including creation time, sender and expected name.
A changed creation time conflicts with its frozen outbox row. The existing maintenance pass prunes
sender outbox rows only past RECOVERY plus clock skew; it adds no tombstone ledger or context secret.
Nonzero exit carries a JSON error code. Explicit/default/local selection never silently changes
backend. Core's side is `talk --remote`. Before effect, Remote calls Core's public
`identity show --json --expect-name=<expected-name> -- <uuid>` through the bounded subprocess owner.
Core atomically checks the pinned UUID's current name and lifetime; active, offline and unknown
presence all permit that identity check. `IDENTITY_CHANGED` (exit 5) maps to the durable
`REMOTE_AGENT_STALE` refusal and `NAME_NOT_FOUND` (exit 3) to `REMOTE_AGENT_NOT_FOUND`. Invalid success evidence
refuses with `REMOTE_CORE_UNAVAILABLE`. Never fall back to name-addressed `check`. Expected name is
retained with frozen native intent; it is not a caller-chosen role.
Full public `talk` acceptance belongs to #4539/#4540 after the Core #4536 seam and Remote #4757
activation. Pairing qualification injects the fake public Core boundary; it does not open that gate.
Real-provider acceptance remains separate.

### Native invite and join

`tmt remote invite [--remote <name>]` selects an owned deployment and prints a private one-use,
ten-minute code. `tmt remote join <code>` validates its public project configuration and pinned native
owner UUID/key. Both terminals show the same four key-fingerprint words. The owner answers
`Do the words match? [y/N]`, then `Let <person> send messages to your agents? [y/N]` in the same
ceremony. Both default to no. No grant or route exists before explicit word confirmation. Sending
defaults off and requires the second answer or later `tmt remote devices talk <person> on`.
Talk/revoke resolve an exact client UUID or a unique active person name; duplicate names require the
UUID. JSON, CI, agent and noninteractive invocation never consent. Ctrl+C cancels the active invite
and erases its secret. Neither command opens a browser or creates a local-browser session.

The code carries public connection metadata, the owner pin, an offer UUID, nonce, generation,
expiry and 128-bit secret. Firestore never stores that secret. Its separate enrollment transcript
binds every field, the candidate native UUID/key, authenticated uid, name and nonce. Existing HMAC,
Ed25519 possession, fingerprint and response-proof primitives are reused with distinct framing.
The mailbox is one create-only collection `/m/<machine>/enroll`: `<offerId>` holds a candidate,
`<offerId>-response` the owner's receipt. Responses contain only `{uid,kind,body,createdAt}`.
Candidates carry those fields plus `token` (32 bytes) for a join, or `token` and its canonical
native `peer` for a reroute. Rules bind the candidate uid to `request.auth.uid`, exact kinds/UUIDs,
bounded bytes and server request time.
Before exposing a code, the owner creates `/m/<machine>/invites/<offerId>` with
`{tokenHash,expiresAt}`. The join token is HMAC-SHA256 under the code over LP of
`"tmt-cloud-invite-token-v1"`, project, owner UUID and offer UUID. Only SHA-256 of that token is
stored in the invitation; it is neither the raw code nor its enrollment MAC/response key.
A join requires its matching unexpired gate. The owner attempts gate deletion when the ceremony ends
(confirm, refusal, cancellation or expiry). Cleanup is best-effort and never replaces the ceremony's
result; expiry refuses logically even when cleanup cannot reach the provider. These owner-only gates
have no client update or list capability.
Only the owner lists (limit one), writes responses or deletes; a caller can GET its own exact row or
a missing row. Kind and offer correlation are bound by the verified transcript. A competing, expired,
cancelled or reused code refuses. An identical confirmed candidate may observe only its original
receipt before the original deadline; this cannot create another grant. No offer secret or derived
response key is persisted. Immutable enrollment rows are retained at most until bounded hourly
cleanup observes them older than 24 hours; logical invite expiry remains ten minutes.

Machine metadata `/m/<machine>` is created under its locally generated UUIDv4. Its exact fields
are `{uid,authenticationKey,descriptor}` plus optional immutable `invitingOwner` (canonical UUID,
not itself). A create requires `request.auth.uid == uid` and either the admin-owned
`/owners/<uid>` allow-list or an unrevoked, unexpired `/m/<invitingOwner>/peers/<uid>` route whose
`peer` is this new machine UUID. Anonymous sign-in alone authorizes no agent-talk write.
Only the stored uid may update the descriptor; it cannot change uid, key or inviter provenance,
including old absence. Any authenticated caller may GET an exact machine row; there is no machine
list/delete. Both participants check the fetched key against the native/invite pin; a conflict
returns `REMOTE_CLOUD_MACHINE_CONFLICT` and is never overwritten.

The inviter alone may move a child's uid after verifying its native-signed reroute. Rules check
that `/m/<invitingOwner>.uid == request.auth.uid`, that only `uid` changes, and that `getAfter` of
the inviter's new-uid route is live, unexpired and bound to the child UUID. Route publication and
the child's uid-only masked update commit together; a route already live also satisfies Rules.
The key, descriptor and provenance stay unchanged. An uninvited owner's uid has no client update
exception: recovery belongs to the explicitly [authorized deploy](#authorized-deploy).
Peer-route CRUD stays with the stored owner; each row has exact `{peer,revoked,expiresAt}` fields.

Confirmation atomically persists the grant, exact receipt/proof, native trust, reroute-token hash
and pending uid route in the existing Store. The receipt carries `rerouteToken`, authenticated by
its existing response proof: HMAC-SHA256 under the response key over LP of
`"tmt-cloud-reroute-token-v1"`, project, owner UUID and native peer UUID. The joiner keeps only
this distinct capability in its private connection, not the code or response key. The owner writes
`/m/<machine>/reroutes/<peer>` with `{tokenHash}` alongside routes; revoke deletes it in that same
publication. A reroute create requires the matching capability hash as well as the new uid.
Old private connections still parse unchanged; pre-token test pairings need one re-pair. They
predate deployable native agent-talk Rules, so this compatibility exception has no production peers. Cloud publication runs outside SQLite and acknowledges only an unchanged
snapshot. Local revoke disables authority at the existing effect fence and marks every historical
uid route for cleanup in the same transaction. Offline cloud cleanup stays visibly pending; it never
restores local authority. Confirmed state, native keys and policy survive reopen/upgrades. An
unsupported saved connection version refuses with `REMOTE_CLOUD_UPGRADE_REQUIRED` and
`This connection needs a newer Remote. Run tmt upgrade; your pairing is kept.`

A changed sender uid creates `<rerouteId>` in the same mailbox with `kind=reroute`. Its confirmed
native key signs `{machineId,owner pin,peer,new uid,generation,timestampMs}`; the sender reserves
its increasing counter durably before signing. Rules bind the writer to the new uid and require
the peer's reroute token; native signatures and generation checks remain with the existing receiver. The owner checks
signature, pin, current non-revoked/non-expired grant, generation strictly greater than stored, and
timestamp within 60 seconds of provider createTime. It atomically replaces the live uid route and
writes `<rerouteId>-response`, signed over the exact request and ID. Old-generation replay cannot
roll back; substitution under another uid fails. No challenge document, extra confirmation or new
code is needed. Exactly one uid per native peer is live; all former uids lose carrier access.

### Superseded cloud authentication proposal

The shared cloud admission, Firestore admission service, Cloudflare device proof and cloud-conformance
sections below retain historical proposal material. Native operation encryption is specified separately. Their
paired-device projection, custom-token challenge and stacked envelope checks are superseded
by [one gate per host driver](#host-driver-authentication), not shipped Firestore authentication.
Do not implement them as an extra gate. Retained bounds, integrity, uncertainty and no-effect
requirements are not permission to enable cloud agent operations. Shipped layer-1 composition,
deployment and sealed-document transport below are unaffected.

### Proposal: shared cloud admission

**Authentication superseded:** see the [historical proposal status](#superseded-cloud-authentication-proposal).

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

**Authentication superseded:** see the [historical proposal status](#superseded-cloud-authentication-proposal).

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

### Shipped: layer-1 composition

The composition below is shipped and is not part of the superseded admission-service proposal.
In the `sharing` profile, Remote builds `firestore.rules` from text it owns (`rules_version`,
the native `/m/{machine}` agent-talk namespace, one `match /x/<extension>` wrapper per extension
of the plan, a default deny) and each extension's admission fragment, which it tokenizes and
rebuilds, never splices. The native policy comes from the same generated-cap artifact used by
carrier and enrollment qualification; extension fragments cannot replace it or escape their wrapper.
The exact composed bytes are covered by the deployment's Rules digest and existing authorization.
The full human plan names the native namespace and its bounds; JSON retains the existing Rules digest.
A fragment holds only nested `match` and `allow <methods>: if <condition>`
statements over an allow-list of request and document fields, operators and methods. Field/index
postfix access includes `request.auth.token.firebase.sign_in_provider`; `+` supports string-ID
composition without type checking. Nested match wildcards can compare document fields and enclosing
wildcards. `request.resource.id`, `request.query` and unbound or reserved wildcards are refused;
`matches` requires a literal pattern. Composition does not prove runtime membership or
declared-collection coverage: extensions verify those semantics in their emulator. `function`, `let`,
`path()`, bare `get`/`exists`/`getAfter`/`existsAfter` and absolute path literals are refused: Rules can
reach outside the wrapper through interpolated or concatenated path strings, and a user function can
shadow a built-in. Reads of the extension's own
documents use the macros `ext.get`, `ext.exists`, `ext.getAfter` and `ext.existsAfter`, which expand to a fixed
`x/<extension>` prefix. A condition must refer to `request.auth` or document data: `if true`, constants and time-only
conditions are refused. A recursive wildcard is allowed only inside the wrapper. Indexes come from the plan as
per-collection field overrides and declaration-derived collection composites: sorted equality scope fields,
then the declared sequence field ascending, cover stream reads with or without a cursor. Equality-only page
interests and discovery (optional sequence equality, document-name order) use Firestore index merging.
The canonical composite union is digest-bound and deduplicated across extensions by collection ID and ordered
fields; no extension-specific field names are built in. TTL is written only when the target provisions it.
Either more than 200 single-field configurations or more than 200 composites without billing is refused as
`too-many-index-configs`, with no artifact (limits checked 2026-10-14). Native inbox and enrollment
queries use the automatic collection-scope `createdAt` ascending index and its same-direction
`__name__` suffix; they require no composite. The superseded admission-service policy above
is not deployed.

### Proposal: Cloudflare Worker and Durable Objects

**Authentication superseded:** see the [historical proposal status](#superseded-cloud-authentication-proposal).

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

### End-to-end operation encryption

**Implemented library and key lifecycle:** native machine-to-machine descriptor sign/verify,
request seal/open and request-bound responses. The native Firestore driver is emulator-only;
production invite/join, final Core activation and browser interoperability remain separate work.
Browser enrollment remains deferred to its existing proposal.
Edge admission remains deferred; never add another Firestore authentication gate.

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
The sending machine verifies that signature with the receiving machine UUID and stable Ed25519
authentication key pinned by the proposed cross-person pairing ceremony, machine/profile/time
agreement and a nondecreasing persisted revision before
using the encryption key. Equal revisions must have identical descriptor bytes. A backend cannot
replace the trust root or authorize a key downgrade.

The machine atomically persists a new key/revision before advertising it and refuses retired key
IDs for new carriers. Keys are valid for 30 days and renew automatically 7 days before expiry,
on serve start and the existing door maintenance callback, without a new worker or timer. Previous
keys remain available until signed expiry plus `journal::RECOVERY`, only to open a carrier whose
trusted provider `createTime` fell inside that descriptor's validity. A carrier created after expiry
refuses. Advertising and sending still require a currently valid descriptor. Three retained keys
cover the 30-day lifetime, 7-day renewal lead and 30-day recovery bound; old two-key v1 rings read unchanged.
An owner offline past expiry renews before cloud traffic on its next serve start. A sender given an
expired descriptor receives retryable `REMOTE_CLOUD_DESCRIPTOR_EXPIRED`; neither side needs a user
step. Failed renewal preserves the prior admitted state and fails cloud crypto closed at expiry,
without changing the authentication key or local grants. The private version-1 keyring and exact
nine-field descriptor remain readable across upgrades; the compatibility fixture is frozen pre-change
v1 bytes, not evidence of a prior release. A future unsupported version refuses with
`REMOTE_CLOUD_UPGRADE_REQUIRED`, without resetting trust or suggesting re-pairing. Lost or
compromised authentication keys require pairing again; an authenticated encryption-key rotation does not create another device identity or grant.
Withholding a newer descriptor can cause refusal, never a bypass. This static recipient-key
profile makes no forward-secrecy claim after recipient-key compromise.

The native sender signs with its stable Ed25519 authentication key registered as the receiver's
peer/client ID, never a browser key or Firebase uid; `origin` is `cli`. The recipient machine is
bound by the signed envelope. The signed intent carries the target agent UUID; the common
admission/Core seam owns its authorization. This codec grants nothing and adds no replay ledger.

A request/control carrier is exactly `{version,profile,keyId,header,enc,ciphertext,signature}`.
Version/profile are 1/`cloud-hpke-v1`. Header contains exactly the enclosed envelope's fields
from version through operation, excluding payload and signature. Let `H` be that envelope's
[canonical signature input](#signed-envelopes) through operation, without its final payload digest;
all existing field/encoding rules apply. `enc` is the 65-byte RFC encapsulated P-256 public key,
and `ciphertext` includes the 16-byte AEAD tag; both use strict base64url. Plaintext is the exact
complete signed envelope JSON, retaining the decoded payload bytes without reserialization.

Define `B = LP("tmt-cloud-carrier-v1") || LP("1") || LP("cloud-hpke-v1") || LP(keyId) || LP(H)`.
HPKE info is B; request AAD is `B || LP(raw enc)`. Use a fresh encapsulation/context for each
new carrier and exactly one HPKE Seal at sequence zero. The sending machine additionally signs
`B || LP(raw enc) || LP(decimal ciphertext byte length) || LP(SHA-256(raw ciphertext))` with its
existing Ed25519 key. This outer proof authenticates the native peer before decryption; the receiving machine
checks it and requires every decrypted header field to match. Both the carrier and
the inner signature must pass. The codec accepts signed controls, reads, sends, results and check
captures; no driver enables cloud operations here. No unpaired enrollment form is an operation.

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

The library exposes consuming response contexts: each produces exactly one response ciphertext,
frozen before publication by the native emulator transport owner. Production activation is separate. Before reopening an adopted carrier, its journal owner
must return the frozen response rather than construct another sealer for changed plaintext; consuming
contexts do not provide a persistent replay ledger. Identical transport retransmission returns only those frozen bytes; a changed response, recovered receipt or
later notification uses a fresh request/control context. A lost context requires fresh signed
session/ID-based recovery, never reuse of a key/nonce with new plaintext. Asynchronous state arrives
inside responses to fresh subscribe controls; log entries keep their machine signatures.
Clients retain context secrets only for the bounded in-flight request and discard them on
completion or abandoned observation. The transport freezes encrypted responses durably before
publishing them and never regenerates different bytes under that context after a crash. Transport
recovery expires within the existing journal retention/capacity; expired ciphertext is unavailable,
not permission to resend. Core final retention and journal ownership are unchanged.

For an advertised maximum signed-envelope JSON length N, ciphertext is at most N + 16 decoded
bytes. A carrier reserves 8 KiB for its bounded header/metadata and has a 65-byte decoded
encapsulation; the conservative wire cap is
`4 * ceil((N + 16) / 3) + 4 * ceil(65 / 3) + 8192`. The binding advertises N for
each request/response class before allocating or uploading; insufficient provider capacity refuses.
Batch responses retain the existing bounded subscription policy. The native Firestore binding has
one document per carrier and no chunks. IDs, operation names, origin, sizes, timing, key revisions and traffic remain
visible; content secrecy is not metadata secrecy or protection from a compromised app host.

### Extension backend declarations

**Implemented:** strict declaration parsing and Firestore extension plan composition. Accepting a
backend name does not implement its binding; Cloudflare transport and provisioning remain proposed.

An enabled owner-installed extension supplies a strict version-1 declaration per backend:
`{version,extension,backend,resources,admission}` with optional `hosting` for Firestore. Extension
uses the existing mounted-name grammar; backend is `local`, `firestore` or `cloudflare`. A declaration has at most 64 resources
and 64 KiB of UTF-8 JSON. Remote reads only installed, owner-approved artifacts, never downloads or
executes a declaration-carried command.

Each resource is `{name,kind,path,limits,ttlField,indexes}`. Name uses the extension-name grammar
and is unique in the declaration; kind is `log`, `checkpoint`, `blob` or `awareness`. Path is 1 to 8
`/`-separated segments of lowercase ASCII letters, digits, `-` or `_` (at most 64 bytes each), resolved under
`x/<extension>/`: no absolute path, empty/dot segment, escape or wildcard, and no first segment equal to an
operation route name (`append`, `subscribe`, `ack`, `pair`) or the mount segment `x`. Two resources of a plan never
have equal or nested paths. Limits specify positive JSON integers `maxObjectBytes`, `maxNamespaceBytes` and
`maxEntries` (each below 2^53, `maxObjectBytes` not above `maxNamespaceBytes`), bounded by the
[backend ceilings](#backend-ceilings) below. Optional `limits.chunkBytes` is a positive integer below 2^53 and not above
`maxObjectBytes`; absence preserves existing declarations without an inferred chunk size.
Optional resource `sync` is `{fields,spaceField,uidField,query}`: 1..64 unique field names
using the index-field grammar, a declared string-valued space binding, and a declared
string-valued publisher uid binding or null. Query is null or
`{scopeFields,streamField,seqField,order}`: 1..8 unique exact-match scope fields,
including spaceField and streamField; seqField is distinct and excluded from scope,
all names are declared in fields, and order is `asc`. Stream is string-valued and seq
is integer-valued. Optional `sync.writeModes` is a nonempty unique list of `create`, `put`,
`delete`; absence allows only create. Log/blob resources cannot declare put. Optional
`query.discovery` is `{name,scopeFields,seqEquals}`: name uses the field grammar and is not
`stream`; scopeFields are exactly the regular scope without streamField, and seqEquals is null
or a canonical nonnegative decimal string below 2^53. No other filters or runtime mapping are accepted.
Missing sync means document transport is unavailable for that resource, not an inferred mapping.
TtlField is null or a field name (a letter, then letters or digits, at most 64 bytes); indexes
contain at most 64 distinct `{field,direction}` entries, where direction is `asc` or `desc` and field uses the same
name grammar. Field semantics and admission belong to Rules; sync declares transported names and query bindings, never grants. Awareness is ephemeral, with no stored objects, TTL field or indexes. Page membership,
epoch/writer checks and expiry policy remain extension-owned; declarations cannot redefine grants,
signed operations or the machine journal.

#### Backend ceilings

Each backend owns the numbers its declarations are checked against; a declaration above its
backend's ceiling is refused as `limit-above-backend`. The plan's canonical bytes and digest carry
only the declared limits, never these ceilings. The Local row is the machine journal's
(`OBJECT_PAYLOAD_BYTES`, `OBJECT_NAMESPACE_BYTES`, `OBJECT_NAMESPACE_ENTRIES`), which an object
stored in a cloud backend must also fit when a second machine imports it.

| Declared limit                      | Local (journal)    | Firestore                                                                  | Cloudflare (advisory)      |
| ----------------------------------- | ------------------ | -------------------------------------------------------------------------- | -------------------------- |
| `maxObjectBytes`, unchunked         | 12 MiB             | 778,680 B: one document, bounded by the extension's Rules, not verified by Remote | 12 MiB             |
| `chunkBytes`                        | -                  | 778,680 B per document, bounded by the extension's Rules, not verified by Remote | none beyond `maxObjectBytes` |
| `maxObjectBytes`, chunked           | 12 MiB             | 12 MiB (the Local value, referenced; advisory)                             | 12 MiB                     |
| `maxNamespaceBytes`                 | 64 MiB             | 1 GiB (advisory sanity ceiling)                                            | 64 MiB                     |
| `maxEntries`                        | 1,024              | 1,048,576 (advisory sanity ceiling)                                        | 1,024                      |

Firestore's 778,680 B is a 1 MiB document less 10,336 B reserved (a 6,144 B document name, 64 field
names of at most 64 B plus 1 B each, and 32 B of overhead), with base64 storing three bytes in
four. "Advisory" means Remote checks the declared number at planning but does not meter use: Firestore
has no per-collection byte or document limit, and only Rules bound a single document. Cloudflare's
numbers stay advisory until its backend ticket states its own. The deploy command's human plan
marks each advisory number `(advisory)`; the JSON plan and its digest are unchanged and carry no mark.

Admission is `{artifact,digest,entryPoint}`: an installed relative artifact path, its lowercase
SHA-256 hex digest (checked against the artifact's bytes, at most 64 KiB) and its namespace entry point; `local` names the extension's existing admission
hook. Remote composes each artifact only with namespace-scoped resource/context capabilities;
operation credentials, projections and response publication are unavailable to fragments. Reject
cross-root reads/writes, reserved operation routes, catch-all grants, arbitrary IAM roles,
overlapping resources or a composition that cannot enforce this boundary. The same
[principal and relay rules](#extension-channel-api) apply; a fragment cannot confer owner-device
context. Unsupported resource kinds, admission requirements, quotas or provider limits fail the
whole plan before provisioning; a target without a physical expiry (TTL) policy instead records a non-null `ttlField` as
`not-provisioned`, with expiry left to Rules, and does not fail the plan. On Firestore a resource path names a collection: an odd number of segments, at most 7, since `x/<extension>` is a document. Routes/assets may be mounted only through the declared namespace,
never an extension-selected public operation URL. Extensions requiring changes submit a new
declaration, not a second backend/sign-in/deploy owner.

A Hosting declaration is `{version:1,files}` with a nonempty, strictly path-sorted inventory of
`{path,sha256,length,contentType}` with optional `csp` last. CSP is 1 to 2048 bytes of printable
ASCII (0x20–0x7e), without CR/LF, and is omitted when absent. Paths are absolute, case-sensitive
ASCII file paths with no empty, dot or hidden segment, escape, query or fragment; `/__` and its
descendants are reserved.
The inventory has at most 256 files, 4 MiB per raw file and 16 MiB total; each digest is lowercase
SHA-256 of the exact raw bytes. Content types are `text/html`, `text/plain`, `text/css`,
`text/javascript`,
`application/javascript`, `application/json`, `image/svg+xml`, `image/png`, `image/jpeg`,
`image/webp`, `image/x-icon`, `font/woff` or `font/woff2`. The declaration digest covers this inventory.

For a declared inventory, Remote captures the fixed public `<extension> hosting-bundle --json`
command once, with no digest argument, in the same neutral cwd and environment as declaration
capture. The strict reply is exactly `{version:1,manifestDigest,files}`: `manifestDigest` is SHA-256
of compact JSON with keys ordered `version,files` and file keys `path,sha256,length,contentType,csp`
(the last omitted when absent). Each reply file is `{path,bytesBase64}` in inventory order, with
canonical padded Base64. All paths, raw lengths and raw digests must match
before provider setup; missing release-embedded bytes are unavailable, never an empty site or
fixture substitute. The reply is bounded to 24 MiB and the invocation to 30 s, with waited cleanup.
Remote adds each Hosting-declaring extension's exact admitted declaration bytes at
`/tmt/declarations/<extension>.json`, outside the extension manifest to avoid a self-digest cycle.
The raw file SHA-256 is the same `declarationDigest` the native sealed declaration returns;
no parsing, normalization or second declaration form is introduced. This public JSON grants no
access or operations authority. Its standard headers have no inherited CSP, and its bytes count
against the same composed file/byte bounds and composition/plan digests, upload and read-back.
The `/tmt/declarations` prefix and descendants are Remote-reserved; manifests refuse them.
Remote composes disjoint inventories, requires `/index.html` as the HTML shell, and freezes gzip
bytes (level 6, timestamp zero, OS 255) with separate raw and transport digests. Its ServingConfig
pins content types, `nosniff`, `no-cache` and fixed `Referrer-Policy: no-referrer` on every file,
and emits its declared CSP verbatim as `Content-Security-Policy`. Remote appends `; charset=utf-8`
to HTML, CSS, JavaScript, JSON and plain-text headers (both JavaScript MIME types), not SVG/binary;
declarations keep bare types and the config digest covers these headers. Only `/colab`, `/colab/`,
`/p/<id>`, `/read/<id>` and `/pair` rewrite to that shell; IDs use the public short-route grammar. Header
rules for these routes and `/` (directory index, no rewrite) copy the shell's exact set because
Firebase matches headers against the original
request path, as documented by the [REST schema](https://firebase.google.com/docs/reference/hosting/rest/v1beta1/sites.versions)
and [Hosting header guide](https://firebase.google.com/docs/hosting/full-config#headers), not
verified live-project acceptance. Reserved Firebase paths are never rewritten. Public bytes and
configuration confer no device authority.

The Hosting plan names the same-project default site and `web.app` entry, any site/web-app
creation, exact content/config digests and a captured foreign-release fingerprint. Remote selects
its recorded ACTIVE same-project app ID; disappearance or a nonempty conflicting Site association
refuses. Otherwise a nonempty `Site.appId` selects its ACTIVE same-project app only when its public
`getConfig` agrees, including among several apps. Without that association, exactly one ACTIVE
app named `tmt Remote (<deployment UUID>)` is used, or creation is planned; duplicate names refuse.
An unassociated site (fresh or existing) gets an explicit `hosting-site:configure` step after app/site
creation: an appId-only PATCH, never a repoint of a nonempty association. Display name is a lookup
aid, not ownership proof; an arbitrary sole foreign app is never selected. The Site association
supports same-origin Firebase SDK configuration; reserved `/__/` paths remain unrewritten.
Ownership requires Remote's deployment/plan labels and full observed gzip-file/config identity;
labels alone cannot adopt a release. Rust `hosting` alone hashes compact UTF-8 JSON with
`files` then `config`: path-sorted rows with fields `path`, `hash`, `status` in that order,
plus config with recursively sorted object keys (array order retained). The first 32 lowercase
hex characters of its SHA-256 are the content label. The adapter captures the digest with prepared
content and supplies it to the private helper for labels/read-back; the helper never recomputes it.
This adds no frozen plan or record field. Old helper sorted-JSON labels remain foreign and are
not migrated or adopted.
The whole plan digest covers foreign Rules and Hosting replacement fingerprints; changed foreign
state invalidates consent.

Declared Hosting refuses before login when the lazy provider factory does not support the
complete lifecycle. Executable steps create only the named app/site, associate the site when listed,
finish database/sign-in/index prerequisites, register this machine as owner, stage a Version (create, populate, required-hash uploads,
finalize), switch Rules, release Hosting, then verify both. Pending app/stage work blocks subsequent steps. Rules and
Hosting publication are not atomic: a single writer is required, switches re-read captured state,
and ambiguity stays partial/unknown without rollback or a usable binding. Provider-generated
resource handles and the original authorized envelope are retained in the versioned deployment
checkpoint; v1/v2 readers remain and reads never convert them. Recovery observes the original
before any further effect; uncertain absence cannot justify a duplicate web-app or Version create.
Other Hosting steps reapply only after observation proves their idempotent work absent. App
creation observes its Operation within the apply budget; if still pending, the owner reruns the same
command. Hosting preparation/observation, a mutation reply and one read-only reconciliation each
have a separate 30-second budget; the apply child is bounded to their combined 90 seconds,
including process I/O and exit. After an ambiguous mutation the adapter reconciles once in the
same run, accepts only exact original evidence, and never sends a second mutation. Partial or
mismatching file inventory remains unknown, never permission to re-populate. File inventories
union explicit `status=EXPECTED` and `status=ACTIVE` bounded listings, including live inventory;
an identical path/hash transitioning between them counts once as ACTIVE. After validating every
row, the helper excludes exactly `/__/firebase/init.js` and `/__/firebase/init.json` from the user-file
union, including live fingerprints, counts and samples. The raw union permits only the 256-user-file
cap plus those two entries; the user union still has the 256-file cap. No other path or hash is exempt.
Conflicting hashes, duplicate paths within one status and an incomplete union refuse. A retained
foreign replacement fingerprint is never rewritten: a changed observed fingerprint refuses before
release unless exact read-back already proves the intended release. Populate compares the union's
paths/hashes; upload/finalize/release also require the appropriate ACTIVE evidence. Omitted
`uploadRequiredHashes` means none required; a supplied list must contain only planned hashes.

Each unknown Hosting step adds optional `cause:{code,phase,httpStatusClass?,files?,fileSamples?}` to its existing
step record and JSON; the human Cause line is unchanged. `phase` is `credentials`, `list`, `mutation`, `body` or
`child-exit`; the closed cause codes are `deadline`, `transport`, `invalid-response`, `body-invalid`,
`body-too-large`, `inventory-incomplete`, `files-mismatch`, `http-rejected`, `child-failed`,
`output-invalid`, `call-incomplete` and `unconfirmed-create`. HTTP classes, when known, are
`1xx` through `5xx`. Optional `files:{planned,expected,active,listed,missing,extra}` reports only
integer counts (0–256): planned files, union members by EXPECTED/ACTIVE status, union total,
and missing/extra path+hash pairs. Unknown/incomplete enumeration does not claim exact counts.
Optional `fileSamples` contains at most four deterministic rows, mismatches first then path order:
`{path,status,hashPrefix,relation}`. Each path is a validated public Hosting-relative path of at
most 128 bytes or `<redacted>`; selected account/project/app/site/version names, resource names,
URLs, queries and unsafe paths are redacted. Status is `EXPECTED` or `ACTIVE`, the hash prefix
is eight lowercase hex digits, and relation is `path-missing`, `hash-different` or `exact`.
Samples total at most 1 KiB, never contain file content, provider bodies or full hashes, and
are also shown beneath the human Cause line. Both detail fields are bounded and validated on
read. Diagnostics default absent, are omitted when absent and clear on a definitive outcome; old records read unchanged
and resume without a version bump. Diagnostics are outside the authorized plan grammar.
An unconfirmed create reports that it will not be repeated: rerun the same command to observe
its result and inspect the recorded cause/provider state if still unknown; retain the saved
record. A Remote-only helper update preserves its plan digest when captured declarations,
Hosting content and plan grammar are unchanged; use `tmt extension upgrade remote --yes --json`,
not a whole-product upgrade that may replace Colab content. An unfinalized Version retained
for at least 12 hours requires a newly displayed and explicitly authorized plan.

Only final joint read-back produces a typed verified publication, saved atomically with Complete:
exact Rules, live Hosting Version/files/config, Site.appId equal to the selected same-project app,
its public config and the `web.app` entry.
Its public config contains exactly `apiKey`, `authDomain`, `projectId` and `appId`. It contains no
credentials and is the output consumed by the separate remote-link descriptor producer. Verification
here is control-plane evidence, not live-project/browser acceptance. Rules and declarations change additively first: client N−1
works with Rules N. Tightening waits until the client no longer needing the old shape has been
the deployed bundle for one release. Colab owns the whole hosted bundle, including an
unmodified, digest-checked copy of Remote's built SDK; Remote adds no hosted pages.

The verified Hosting release name is retained exactly as received: only
`sites/<site>/releases/<id>` or `sites/<site>/channels/live/releases/<id>` for the selected site,
with a nonempty single-segment ID within the existing resource bound, is admitted. Other sites,
channels, empty IDs and extra segments refuse. POST/read-back identity comparisons use the
validated site and release ID, not the path spelling; this does not add a provider read or change
the authorized plan, digest, handles, fields or record version. Existing site-qualified records
read unchanged. A record with the live-channel-qualified name is forward-only: older Remote
versions refuse it, so do not downgrade that record.

**Remote link.** The public carrier is derived only from a Complete record's typed verified
Hosting publication. No link metadata is persisted and deployment documents stay at version 3.
An authorized Complete prints the derived link; `tmt remote link [--json]` derives the same bytes
from one bounded lock-free owner-home snapshot, without provider calls, deploying, opening a
browser, converting or repairing state. No record, partial/unknown or unverified publication is
`REMOTE_LINK_UNAVAILABLE`; unsafe records retain the existing state refusal. JSON is
`{version:1,remoteLink:<URL>}`. The link can be shared again and grants no access.

The receiver is exactly `https://<project>.web.app/pair#<descriptor>`, derived from the verified
public config's project. The complete URL is bounded to 8 KiB. The fragment is unpadded base64url
of canonical UTF-8 JSON, with recursively sorted object keys and exactly `version:1`,
`kind:"firestore"`, `publicWebConfig`, and `checksum`. Public config has exactly `apiKey`,
`authDomain`, `projectId`, `appId`; strings are printable ASCII, with consistent same-project
`firebaseapp.com` auth domain and HTTPS `web.app` receiver. Unknown/duplicate fields, malformed
text, unsafe or inconsistent URLs and changed checksums refuse. Checksum is lowercase SHA-256
of the canonical descriptor JSON with the checksum field omitted; it detects corruption only,
not authority. A receiver treats the link as untrusted configuration and uses it only when its
projectId, appId and authDomain equal the same-origin Hosting config at /__/firebase/init.json;
on mismatch it refuses, so a link cannot point sign-in at another project. No credential,
membership, uid, device grant, deployment identity or label is carried. Pairing/admission remains separate; real tenant/browser acceptance still requires the
hosted receiver and separately authorized provider proof.

The composed plan is deterministic JSON (extensions and resources sorted by name) addressed by its SHA-256 digest,
which also covers each declaration's own digest, the target backend and whether physical TTL is provisioned. Its
`profile` is `sharing`: extension resources and admission fragments without an operation root. A later layer-2 profile
is an additive variant and never changes the bytes or digest of a sharing plan. An enabled extension with no
declaration for the target is listed as unavailable there and does not fail the rest of the plan.

The developer-only `compose_firestore` Cargo example validates the exact declaration reply and
composes Rules, indexes and a plan through the same pure owners as deployment. Its `plan.json`
includes the plan digest and both exact input digests. Colab owns
`extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.json` and its composed golden trio;
Remote's mirror reads those files and requires every golden when the vector exists. Until Colab
supplies that vector, the mirror reports pending and only Remote-authored fixtures qualify the
entry; those fixtures are not Colab declaration or routed acceptance evidence.

### Hosted anonymous bootstrap

The served SDK's explicit `firebaseRemote.connect` verifies the canonical Remote link and
checks that `projectId`, `appId` and `authDomain` equal same-origin `/__/firebase/init.json`
before lazily loading the hosted Firebase asset.
`firebaseRemote.originLink()` derives the canonical public pair link from the origin's bounded
init.json response using only `apiKey`, `appId`, `authDomain` and `projectId`, validates the
exact `https://<projectId>.web.app` origin, and refuses with `REMOTE_INPUT_INVALID`; callers
pass it through the unchanged `connect` checks, gaining no admission or operations authority.
The caller supplies the frozen extension declaration bytes and their SHA-256; digest or
resource-grammar disagreement refuses before provider access. The table is data, with no
extension-specific resource names in Remote. Configuration and authentication grant neither
space admission nor operations authority. Colab owns its admission protocol.

One Firebase App/Auth is shared by owners of the same verified public configuration. V1
supports anonymous sign-in only, initialized without a popup/redirect resolver; no GAPI or
Auth iframe is required. An anonymous uid persists per browser origin/profile; clearing site
data requires Firebase sign-in and extension admission again, not local pairing. Bootstrap and Auth observation are bounded to ten
seconds. Closing releases that owner and suppresses late identity publication; the last owner
closes the app. Explicit sign-out affects the shared identity. Provider-disabled and rate
refusals use Remote's typed vocabulary, not a machine-signed local-session receipt or proof
of device revocation. Missing lazy code is `REMOTE_STATE_UNAVAILABLE`, with no CDN fallback.

Hosting copies `/sdk/remote-v1.js`, `/sdk/remote-firebase-v1.js` and
`/sdk/THIRD-PARTY-NOTICES-remote.txt` byte-for-byte; the generated asset/notices have reviewed
build pins. Scripts stay same-origin. A hosted page that calls `connect` must allow
`https://identitytoolkit.googleapis.com` and `https://securetoken.googleapis.com` in its
`connect-src`; `https://firestore.googleapis.com` is required for sealed-document transport.
Colab owns the hosted shell policy and its composed manifest headers.
The local door's pairing and session path never implicitly loads Firebase.
Cloud agent operations and a machine connector are not shipped. This bootstrap's Firestore
access is the browser sealed-document transport described below.

### Anonymous connect guide

Colab's hosted `/pair` entry presents the public Remote link as an anonymous connect guide,
before its page-link fragment parser. The original public link and the installed declaration's
exact bytes/digest pass through the existing `firebaseRemote.connect` checks; the guide adds no
link parser, pairing, device proof or Rules/admission gate. Colab owns that entry and its view;
Remote owns the public link, CLI help and bounded Auth implementation.

Browser progress comes only from bounded `connect` and an explicit `signInAnonymous` result.
A confirmed browser sign-in proves neither page access nor machine readiness. Closing the guide
closes its Auth reference and suppresses late progress. Failure uses fixed copy, never provider
bodies, and causes no automatic sign-in retry; the same action permits an explicit retry after failure.
Only a validated public link enables command copying.

The machine section provides instructions to add the public reference and sign in with an explicit
remote selector, with no progress labels, local probes or default changes. Copying does not execute
them. Opening the owner's page link is a browser-only instruction: Colab's existing token self-join
admits that browser uid with the link's role. It does not admit the machine's native sync uid.
An active native sign-in reuses its uid; after it ends, signing in creates a new identity.
Page access through an imported remote is not shipped: there is neither native share-link self-join
nor an owner-to-Firebase-uid enrollment command. Imported machine page access remains stage 5 of #3861;
#3868 only integrates local browser opening.
The existing CLI grammar, JSON shape and public link bytes remain unchanged; fixed sign-in
diagnostics state this limitation only for imported references. The selector carries its owned/imported
provenance into presentation without changing authority or JSON. The serving owner uses its own
project and publishes its bound uid; this does not rebind an existing space to a new uid. Help that
cannot know the selection scopes the limitation to remotes added with `tmt remote add`.

### Native sealed-document carrier

**Status:** the neutral `tmt-extension-objects::sync` DTOs and Remote's injected adapter
extend the existing object channel. Serve registers the native factory for a verified selected
Firestore project, an existing active sign-in and enabled release declarations. Missing or
unsafe inputs leave native activation typed `REMOTE_STATE_UNAVAILABLE`, with no provider I/O
or project fallback; the ordinary door stays usable. v3 remains explicitly opt-in.
Serve resolves the selected native project once at startup; changing `settings.json`'s
`defaultRemote` requires restarting serve and does not retarget the running factory.
Colab's accepting adapter (PR-B) must ship before Remote
enables version 3 by default. This carrier does not implement the cloud ObjectBackend
adapter or change LocalFs startup: accounting still opens fail-closed.

**Activation and compatibility.** A capable installed Colab invokes the hidden host protocol
`tmt remote __activate --extension <name> --deadline-ms <absolute Unix milliseconds> --json`
through its existing supervisor subprocess owner and its own timeout. It is absent from help,
completions and the handbook. Success is exactly `{"generation":"<lowercase UUIDv4>","project":"<project id>"}`;
the validated project is the registered native factory's startup selection, not a deploy-record
projection or another status read.
failures exit nonzero with `{"error":{"code":"<code>","message":"<message>","state":"deadline"}}`
(the state member appears only for deadline). Codes reuse `REMOTE_NOT_RUNNING` for absent/refused
socket, `REMOTE_STATE_UNSAFE` for an unsafe/foreign-owned path, `REMOTE_STATE_UNAVAILABLE` with
state `deadline`, and `REMOTE_SERVE_OUTDATED` for unsupported, the exact legacy unknown-op
refusal or a valid old three-member activation success: restart the door, never guess a project. Other typed refusals pass through, including `REMOTE_SCOPE_DENIED` for an undeclared
extension. An already-spent deadline causes no I/O. Root discovery and the single control
exchange share the caller's bound; no scan, state creation, restart or fallback occurs.

Only Remote discovers/adopts its private control address. Colab neither derives that path
nor reads Remote records or imports Remote types. The CLI sends exactly
`{"op":"sync.activate","extension":"colab","version":3}` internally, with success
`{"activated":true,"version":3,"generation":"<lowercase UUIDv4>","project":"<project id>"}`. An unsupported version
is `REMOTE_CONTROL_UNSUPPORTED`; malformed/extra input is `REMOTE_INPUT_INVALID`.
The extension must be in the existing enabled Local declaration list and have a registered
factory. Activation uses the existing bounded single-flight setup owner; a failed candidate
leaves the prior channel available. Status, channel end and reads do not activate sync.
A Remote restart loses registrations, bindings and interests: the project owner registers
again and Colab explicitly re-activates, rebinds and queries its own saved positions.

Installing a newer binary does not change an already-running older door. That door refuses
the unknown control operation with typed `REMOTE_CONTROL_UNSUPPORTED` (earliest legacy doors
use `REMOTE_INPUT_INVALID`, message `Unknown control operation.`); ordinary Local objects
remain available. The door needs a restart to gain this activation surface. Refusal is not
permission to retry in a loop or invent a second socket.

The explicitly requested native connection offers its highest version, 3, exactly once
using the same seven request headers and endpoint as the legacy handshake. A successful
reply has the existing 101 headers plus `tmt-object-channel: <1, 2 or 3>` naming the
accepted version. Native activation requires 3; a selected older version returns typed
unsupported and leaves the previous channel intact. A supported strict refusal is exactly:

```http
HTTP/1.1 426 Upgrade Required
tmt-object-channel-error: unsupported-version
tmt-object-generation: <the offered generation>
Content-Length: 0
```

There is no second connection, repeated offer or fallback after a v3 EOF, timeout,
malformed reply or typed refusal. The new acceptor answers 3 to a v3 offer, and typed
unsupported to a syntactically valid higher version. A current v1/v2-only Colab silently
closes a v3 offer; it cannot supply the new selected-version reply. Such a peer continues
to connect through ordinary Local activation, which offers 2 and retains exactly the
existing single fresh v1 attempt after failure before a complete reply head. V1/v2 heads,
framing, methods and outcomes stay byte-identical. This distinction is the rollout gate,
not a multi-round version negotiation.

**Bounds.** V3 adds `sync-request`, `sync-result` and `sync-notice`; existing object frames
remain valid with their original bounds. The four-byte length prefix is excluded below.
The negotiated limit is checked before body allocation; sync frames retain strict UTF-8,
duplicate-member, unknown-member, depth-eight and unsigned JSON-safe-number validation.

| Bound                                                                           | Value                              |
| ------------------------------------------------------------------------------- | ---------------------------------- |
| One sync frame body                                                             | 397,312 bytes                      |
| Canonical REST fields in one document                                           | 393,216 bytes                      |
| Non-fields metadata in one frame, including received whitespace/escape spelling | 4,096 bytes                        |
| One query/discovery result                                                      | One frame, at most 256 documents   |
| Stateless discovery continuation                                                | At most 1,024 graphic ASCII bytes  |
| Bound spaces in one channel generation                                          | 8                                  |
| Page interests                                                                  | 3 per channel, 12 per installation |
| Shared object/sync requests                                                     | 8 per channel, 32 per installation |
| Existing callbacks                                                              | 8 per channel, 32 per installation |
| Existing receive queue                                                          | 8 frames and 524,288 body bytes    |

There are no parts, staging handles, batches or retained pages for later draining. A
provider page must already fit one carrier result, charging actual reads/re-reads for
rows omitted from a bounded query. `query_page_bytes(context,page)` measures the actual
encoded result with the longest RPC ID reserved. It refuses an oversized page without
trimming rows or advancing its cursor. Native object chunks remain 32,768 decoded bytes;
the larger sync frame does not enable a new attachment transport. Colab's canonical
REST fields preserve its 354,304-byte sealed update and 43,692-byte padded-base64 checkpoint
envelopes. Authority/checkpoint mapping, retained unpublished originals and rebootstrap
on missing history remain Colab decisions; discovery does not prove retained first payloads.

**DTOs.** The leaf owns these wire types; consumers do not import Remote implementation
types. Sync frame `version` is 1, independently of negotiated carrier version 3.
Generation/binding IDs are lowercase UUIDv4, RPC/notice counters positive canonical
u64 decimal strings, and cursor `seq` unsigned JSON-safe integers. The types `BoundContext`, `StreamPosition` and `InterestNotice` carry the bound identity,
consumer position and wake/terminal status respectively. A `Target` is
`{resource,id}`; a `Document` is `{target,fields,digest}`, with the SHA-256 of the canonical
Firestore REST `Document.fields` map. No additional ciphertext wrapper or Auth token is
carried. Bound public context is `{id,project,extension,declarationDigest,space,uid}`;
identifiers and digests confer no admission.

Requests are `{kind:"sync-request",version:1,generation,requestId,call}`. `call` has exactly
`{method,input}` with the following inputs:

| Method           | Input                                         |
| ---------------- | --------------------------------------------- |
| `sync.bind`      | `{project,extension,declarationDigest,space}` |
| `sync.read`      | `{binding,target}`                            |
| `sync.query`     | `{binding,query}`                             |
| `sync.subscribe` | `{binding,interest}`                          |
| `sync.cancel`    | `{binding,subscriptionId}`                    |
| `sync.publish`   | `{binding,mutation}`                          |

`query` is either `{shape:"stream",resource,scope,stream,cursor}` or
`{shape:"discovery",resource,name,scope,continuation}`. Optional cursor/continuation is
encoded as null when absent. The discovery `name` must select the declared optional
shape; discovery has no stream field or logical cursor. `interest` is exactly
`{resource,scope}`, the regular query scope minus its stream field; no discovery shape
is required. It has no seed, sentinel, cursor, continuation or delivery progress.
`mutation` is `{mode:"create",document}`, `{mode:"put",document}` or `{mode:"delete",target}`.

Results are `{kind:"sync-result",version:1,generation,requestId,method,outcome}`. `outcome`
is tagged by `result`: `binding` with `context`, `read` with nullable `document`, `stream`
with `documents,cursor`, `discovery` with `documents,continuation`, `subscribed` or
`cancelled` with `id`, `published` with `status`, or `error` with `error`. Publish status
is `confirmed`, `already-present` or `unknown`. Create uses the declared conditional
create semantics; put/delete use declared whole-field/unconditional semantics. The
carrier neither prepares a read nor retries, changes the target or reseals original bytes.
A possible effect remains unknown across cancellation, deadline or identity loss.

Error is exactly `{code,state,resetAtMs,retryAfterMs}`, with absent optional values encoded
as null. Codes are `REMOTE_INPUT_INVALID`, `REMOTE_SCOPE_DENIED`, `REMOTE_STATE_UNAVAILABLE`,
`REMOTE_BUDGET_EXHAUSTED` and `REMOTE_RATE_LIMITED`. Only state-unavailable may carry a
state: `cancelled`, `deadline`, `catch-up-required`, `conflict`, `unsupported`, `unenrolled`,
`expired`, `sign-in-needed` or `revoking`. Budget exhaustion carries both millisecond
fields; other codes carry neither. The Remote budget guard stays in force and typed
refusals reach Colab unchanged; this adds no Colab uid/space counter outcome.

**Registration, wake and cleanup.** The subscribe RPC request ID is also its subscription
ID, fenced by the shared object/sync request high-water mark. A successful registration
is written before notices are enabled. Notices are
`{kind:"sync-notice",version:1,generation,subscriptionId,sequence,notice}`, where `notice`
is `{notice:"changed"}` or `{notice:"terminal",error}`. Notice sequence increases within
one interest. Only a pending Changed flag and a reserved terminal refusal are retained;
there is no document queue. Changed is level-triggered, may be spurious and is never lost.
The provider owns suppression under the [Subscribe rule](#subscribe). Colab performs
ordinary queries from its own positions and alone advances them; empty finite results
preserve the input cursor. Gaps/compaction/oversized pages return catch-up-required.

The injected subscription owns polling/listeners. The carrier only awaits `next` with a
fresh bounded deadline. Idle/backoff deadline is not positive absence: it continues that
same interest without carrier sleeps or provider queries. If a probe deadline closes the
provider interest, its next Unavailable is terminal. Cancellation removes the interest,
stops and joins its awaiter before acknowledgment; generation loss/stop closes all ports,
wakes workers and joins awaiters, dispatcher and bus reader. A taken notice overtaken by
cancel cannot satisfy a successor. A terminal interest keeps its ledger slot until cancel or
generation close; consumers cancel it before registering a successor. No registration
survives restart or reconnect.

**Injected construction.** `Registration` accepts trusted public configuration, exact
Firestore declaration bytes/digest, Auth and this constructor-only factory:

```rust
Arc<dyn for<'a> Fn(BindInput<'a>)
    -> Result<Arc<dyn SealedSyncPort>, SyncError> + Send + Sync>
```

`BindInput` contains `public_config: serde_json::Value`, `declaration: &'a [u8]`,
`declaration_digest: &'a str`, `space: &'a str`, and `auth: Arc<dyn AuthPort>`.
Wrong-project configuration/Auth is refused before construction/provider I/O; the returned
context must match every frozen input. Repeated binding of one space reuses its context.
The factory captures the selected project's shared runtime/model; it does not create a
runtime, Auth owner, budget pool or timer per consumer. Serve obtains exact enabled release
declarations through InstalledDeclarations and verifies their declaration/admission digests
without deploying. One selected-project Auth owner loads an existing sign-in; it never signs
up implicitly. The shared wakeable minute checkpoint runs while workers idle/back off.
Checkpoint failure fences every project binding and Auth refresh. Normal stop fences new
work, cancels Auth and wakes the tick, closes/joins carrier/provider workers, joins the tick,
then calls finish before releasing the budget writer and Serving lease. Early setup failure
uses the same owned cleanup. Budget refusal performs no provider I/O or allowance reset.

### Extension-scoped sealed-document client

Browser transport is shipped. Native credentials and REST effects belong to `firebase_auth`
and `native_sync`; serve composes their shared project owner behind the opt-in v3 carrier.

The SDK and native types define one bound client port. Provider implementations are separate
from this pure type boundary. The browser factory `HostedAuth.sealedSync(space, guard)` binds
it to the existing verified anonymous Auth owner; the
[native carrier](#native-sealed-document-carrier) defines the injected factory seam.
Its context contains verified public Firebase configuration, extension, exact declaration digest,
declared resources, space and authenticated uid. Binding validates the entire declaration before
accepting any resource; digest or context disagreement refuses before provider access. The public
configuration is verified by the bootstrap above (native callers use their explicitly selected
project); pure binding cannot attest an origin or grant admission. Colab owns membership, page/epoch
and writer admission in its [sync/admission contract](../extensions/tmt-colab/contracts/colab-v1.md#sync-and-backend-admission).
There is no agent-operation, owner-device or Admin authority and no fallback to another remote.

`SyncAuthPort` (native `AuthPort`) supplies an already authenticated `{projectId,uid}` identity
and a cancellation/deadline-bound token request. A token result carries that same identity,
its transient secret and `expiresAtMs`; the transport must recheck identity and expiry before
I/O and must not log, persist or place the token in an original ID, receipt or argv. An auth
failure is `REMOTE_STATE_UNAVAILABLE` with state `expired`, `sign-in-needed`, `unavailable` or
`cancelled`. Offline does not prove expiry; a refused refresh cannot silently mint a new uid.
The port defines neither credential storage nor refresh policy. Browser implementation shares
one bootstrap App/Auth, refuses binding before anonymous sign-in, and rechecks token aud/project,
sub/uid, anonymous provider and expiry before I/O. Sign-out/close invalidate its bound clients;
no implicit sign-in replaces an admitted uid. Clearing site data requires fresh Firebase sign-in
and extension admission, not local pairing.

Browser document data uses fixed-project default-database REST get/runQuery/single-write commit,
with redirects refused and no mutation retry or offline queue. An acknowledged create collision
reads only the same target to distinguish identical bytes from conflict; this is not a preparation
read. Before every REST attempt and listener start, the caller-supplied `guard.admit(cost)` reserves
cost or refuses with `REMOTE_BUDGET_EXHAUSTED {resetAtMs,retryAfterMs}` or a sanitized
`REMOTE_SCOPE_DENIED`. Each commit cost carries frozen, SDK-validated
`mutation:{mode,resource,target}` metadata; `target` is the declared document ID. It contains no
extension role or record kind. The caller owns the shipped
[budget model](#free-plan-budget-guard) and durable reservations; this client adds no quota ledger.
Cost metadata bounds REST reads, uses the budget model's writer lookups for commits and three Rules lookups per delivered document, and marks the
listener's full matching initial tail, all deliveries including the writer's own, and additional
consumer REST catch-up. Memory-only listener reconnects cost a fresh query
([Firebase pricing](https://firebase.google.com/docs/firestore/pricing#listening_to_query_results));
the caller reserves that cost too. Listener tail size is not reported as zero or as measured project usage;
conservative active-hour estimates include both push delivery and REST catch-up, not plain twice
the append model, which excludes the writer's own listener.

An optional caller-owned `guard.received(documents)` receives the complete initial server
snapshot count, then every document-change batch before its coalesced hint; the transport
stores no counter. A guard refusal closes the listener and preserves the typed budget reset
for pending and later `next` calls. The consumer joins its outstanding guard work at teardown.

Browser wake uses one Web SDK onSnapshot per page interest, with only its declared equality scope,
memoryLocalCache and no persistent cache. Metadata-only changes after the initial server snapshot
are ignored. Snapshots carry coalesced hints, never document delivery or cursor progress; listeners
cover empty pages, new writers and later writes in existing streams. Cancellation, unsubscribe,
sign-out and close fence late callbacks. An idle `next` deadline rejects with
`REMOTE_STATE_UNAVAILABLE {state: 'deadline'}` and keeps the interest alive; call `next` again.
An aborted call rejects with `state: 'cancelled'`. Provider permission/authentication refusals
reject with `REMOTE_SCOPE_DENIED`, resource exhaustion with `REMOTE_RATE_LIMITED`, and other
listener failures or ordinary ended/closed interests with bare `REMOTE_STATE_UNAVAILABLE`;
pending and every later `next` retain a terminal refusal. Any
rejection other than `state: 'deadline'` is terminal for the consumer, never a device-revocation proof. Firestore is confined to the lazy Firebase asset; the loopback door does not import it.

The port offers bounded read, query, subscribe, publish and close. A document target names one
resource and an opaque ID, resolved only within that declared collection under `x/<extension>`.
Space/uid IDs use ASCII letters, digits, `_`, `+` or `-`, 1..128 bytes; document IDs use the same
alphabet, 1..256 bytes. Document.bytes is canonical UTF-8 JSON of the Firestore REST `Document.fields` map,
not one opaque ciphertext field or a second envelope. Each field has exactly one of
`stringValue`, `integerValue`, `booleanValue`, `nullValue`, `timestampValue` or `bytesValue`;
other Value kinds are refused before I/O. Integers are canonical decimal strings within
2^53-1, nullValue is null, and bytesValue is canonical padded standard base64. Timestamps
are valid RFC 3339 UTC with Z and the shortest exact 0/3/6-digit fraction: other precision
is refused, never rounded. Firestore stores only microseconds; the SDK adapter must preserve
these bytes unchanged through REST; Web SDK listener filters preserve the same exact values. Keys are
sorted, whitespace omitted and every field is declared by sync.fields. Remote checks the
space binding on field-bearing reads/create/put, publisher uid on create/put, exact scope filters and SHA-256;
it never decrypts the sealed envelope field. Payload fields are otherwise extension-owned.
Ports capture frozen document bytes before asynchronous token acquisition. One publish targets one
declared document: create uses exists=false, put replaces the whole field object unconditionally,
and delete is unconditional. Declared writeModes do not grant admission; ordinary Auth and Rules,
including extension-owned version fields, remain decisive. There is no Admin authority, updateTime
precondition, preparation read or accounting companion. Create of identical frozen bytes returns
already-present; conflicting bytes refuse with `REMOTE_STATE_UNAVAILABLE`, state `conflict`.
Undeclared modes refuse with that code and state `unsupported`. An acknowledged effect is confirmed;
a possibly applied effect is unknown, including cancellation. Recovery reads the SAME target: create
and put compare frozen desired fields, delete checks absence, without claiming which writer applied
it. No hidden retry, replacement ID or resealing is permitted. Admission refusal uses
`REMOTE_SCOPE_DENIED`; provider exhaustion follows the [budget guard](#free-plan-budget-guard),
including unknown writes. A complete commit HTTP 403 with `error.code:403` and
`error.status:PERMISSION_DENIED` proves an admission refusal. A malformed or ambiguous error,
transport loss or possibly applied resource-exhausted commit stays unknown; none is resent.

Catch-up cursors are declared per-stream sequence positions (`{stream,seq}`), with stream
identifying the extension's writer stream (not the Auth uid), and seq the last observed contiguous nonnegative integer
below 2^53. Colab Rules enforce contiguous seq within a stream owned by its initial uid. Neither a global clock nor lexical document
ID orders catch-up. A finite query gap/compaction refuses with `REMOTE_STATE_UNAVAILABLE` and
state `catch-up-required` (native SyncError::CatchUpRequired); no partial finite page is exposed.
Ordinary queries require the exact stream filter. The optional declaration-named discovery shape
has no stream filter or logical cursor, only its exact equality scope and declared seq equality.
A bounded run returns actual documents, paged by an opaque stateless continuation (at most 1 KiB,
printable ASCII without spaces), containing the last document name and bound to scope, shape,
generation and deadline. A mismatched token or continuation after close/deadline refuses typed
before I/O. No continuation means the run is complete; otherwise it is incomplete. There is no
run ledger. Repeated runs discover arrivals missed by a prior run; pruning requires rebootstrap.
Native poll interval/read costs are defined by the native transport, not this type boundary.

Native Auth and sealed-document acceptance is an explicitly selected Auth+Firestore
emulator scenario over the checkout's Colab declaration, Rules and indexes, recording
those hashes. It exercises ordinary anonymous signup/refresh, same-target create/read,
stream-cursor catch-up, page COUNT wakes and typed write/read/query/wake refusals after
admission changes, retaining the uid. Production endpoints are unchanged; only a private
lib-test HTTP adapter routes fixed demo endpoints to loopback. Plain crate runs ignore
this scenario. This proof does not establish live-project acceptance or composite-index
enforcement (the emulator does not enforce those indexes).

#### Subscribe

Subscribe takes a page-level `SyncInterest {resource,scope}`: exact query-mapping scope minus
the stream field, without requiring discovery. It has no seed stream, cursor, continuation or
provider-held delivery progress; one page interest counts once toward the listener caps.
It produces only coalesced Changed wake hints or CatchUpRequired. Changed is level-triggered:
it may be spurious but never lost. Native polling suppresses a wake only when a successful
COUNT over the exact interest scope equals the last in-memory count and the ten-minute
heartbeat is not due. The first probe and every count difference emit Changed; the heartbeat
bounds equal-count delete/create swaps. Browser listeners are unchanged and cover later writes,
not only first-sequence documents;
terminal status is a typed refusal, not a document queue. Consumers fetch with ordinary read/query
and alone advance their logical cursors. A gap, compaction or overflow is never a silent drop.
Each call has caller cancellation and an absolute deadline; closing suppresses late publication.
There is no offline mutation queue. Envelope/checkpoint bounds are 354,304/43,692 encoded bytes.
Provider pages hold at most 256 documents and fit one 397,312-byte carrier result, including
metadata; never retain a larger page for later draining. Listener caps remain 3 per open page and
12 per tab, one in-flight write per stream and 4 per tab. These are transport bounds, not a new
per-user or per-space project quota; implementation slices pin deadlines/backoff without adding
operations authority.

#### Proposed cloud object profile

A thin ObjectBackend adapter uses this same client; no second Firebase stack or direct attachment
provider exists. Chunked resources explicitly declare decoded `chunkBytes`: attachments use
262,144, checkpoints 32,768, with at most 513 chunks. Attachment serialized bytes stay capped at
12 MiB; checkpoint serialized bytes can reach 16,779,264 (513 chunks). Each admitted resource
provides its own lower object bound; the shared validator ceiling is 16,779,264.
The object-channel's existing 32 KiB framing is unchanged; the later adapter assembles transport
parts into declared cloud chunks. Immutable chunk IDs are `<original>+<index>` (zero-based decimal)
and the last-written create-only manifest is `<original>+manifest`. The logical manifest data is exactly
`{original,count,bytes,digest,chunkBytes}`; canonical UTF-8 JSON sorts these keys. Its transport
document also includes declared admission metadata and uses the restricted REST fields encoding
above; the logical manifest is not an extra on-wire envelope. Count equals
ceil(bytes/chunkBytes); nonfinal chunks are full, the final chunk is bounded, and an empty object
has no chunks. Readers require that manifest and every chunk, exact count/length and full-object
SHA-256 matching the admitted descriptor: a complete verified object or absence/unavailability,
never truncated data. Original identity, chunks and manifest remain unchanged across loss of
acknowledgement or restart. No upload reseals or invents a second original on recovery.

This profile promises original-ID/digest idempotency and complete-or-absent publication, not
LocalFs's durable cross-machine reservations, namespace fences or exact accounting. New chunks
are denied when the page is archived/deleted or loses admission; Colab owns those Rules and
metadata resources. Owner namespace deletion is best-effort; `Removed` requires verified absence,
otherwise cleanup is pending/unknown, not freed storage. Spark tables and attachment cost examples
are dated hints, not aggregate enforcement; the shipped page-append budget guard remains in force.
There is no physical TTL setting or cleanup runner on Spark; rows remain until owner deletion.
Remote-authored frozen vectors qualify pure grammar/validation only, not Colab admission or
provider acceptance. Cloud adapter conformance distinguishes these every-backend guarantees from
local-only guarantees rather than silently skipping the latter.

### Hosted own-record publication guard (#3122)

Colab's hosted caller admits exactly `members` create and prepared `own`-namespace `log`
create. This is caller policy over the SDK's validated neutral mutation metadata, never a
Remote role/record validator. Every other mode/resource shape refuses with
`REMOTE_SCOPE_DENIED` before Auth or provider I/O. Colab owns fresh signed author admission,
own-record preparation and sealing; the browser never seals content or checkpoints.

The durable cross-tab origin ledger shares 10% of `READS_PER_DAY`: 5,000 modeled reads,
warning at 3,500 and refusal before a reservation reaches 4,500, with the SDK's Pacific reset.
Writes are tracked, not separately refused. Members-create reserves four lookup reads and one
write. Each own-log create reserves `BudgetModel.create(4,4).readsPerAppend()` reads and one
write: 13 + 1 under the four-person/all-writers assumption, pinned to the native vector.
A prepared batch is at most four documents (52 reads + 4 tracked writes); the Colab edit
request transport prepares one document. REST catch-up/readback and listener deliveries add
their ordinary costs. A refused reservation spends nothing; uncertainty is never refunded.
These are advisory reservations, not measured project usage or a provider billing claim.

The sender freezes exact IDs/bytes before any create and serializes each own stream across
tabs through creation and readback. It sends each original once. A commit acknowledgment or
unknown gets one bounded exact-ID readback; `already-present` already includes the SDK's
comparison and gets no second read. Only matching canonical bytes and digest are sent.
Missing/unreadable observation stays unconfirmed; an occupied immutable ID with different
bytes is a definitive loss. A partial batch never reports full success. Neither uncertainty,
Rules/budget/rate refusal nor definitive loss triggers resealing, a new ID or another create.
The distinct `writer-in-use` result covers definitive stream/ID occupancy; a hosted uid may
continue a stream whose predecessor another uid wrote only when both uids were enrolled by the same
editor link (Colab Rules compare the two members link fields); every other uid is refused. Colab owns
the device derivation and that Rules clause. Publication confirmation is not owner application:
the owner applies edit requests while Colab is running. Page change/close cancels and joins
sender, reader and guard work before releasing keys or opening a successor.

### Authorized deploy

**Shipped:** the Firestore deployment described under **Current implementation** below includes
extension-sharing Rules and the native agent-talk carrier/enrollment policy. Publishing that
policy does not enroll a peer or grant agent access. The superseded admission-service and
Cloudflare provisioning in the following profile remain **proposed**.

`tmt remote deploy <backend>` composes Remote's resources and all enabled extension declarations
into one deployment in the owner's account. Before effects it shows the exact account/project,
deployment ID, region, enabled extensions and declaration digests, resources, roles/bindings,
Rules/Worker changes, quotas/expiry/index policy and destructive changes. The owner explicitly
authorizes that plan for that target account at deploy time. Existing pairing, start, account login
or extension installation is not authorization to create or change real account resources.

#### Deployment input manifest

This backend-neutral contract is implemented by Remote's shared input selection and
refusal model. Backend command integration is separate; the current Firestore command
below does not yet consume it (#4879). The following requirements govern that integration
and every later `tmt remote deploy <backend>`.

Each backend owns one ordered input manifest. Each entry names the input and its long
flag, its kind (`select-existing` or `create-or-adopt`), the value `--defaults` chooses
(or `none: must be given`), what would be created if absent, a cost note, and the
permissions needed. Manifest metadata is authored by Remote, never taken from provider
exception text. The same manifest supplies input admission, help and refusal display;
backend-specific validators and resource planning stay with the backend.

Before login or any provider call, an omitted required input without `--defaults`
refuses with `REMOTE_DEPLOY_INPUT_REQUIRED`, **exit 1**, through the standard error
publisher. There is no interactive missing-input prompt in v1: humans, agents, CI and
`--json` receive the same refusal. Human output lists each missing flag, the value
`--defaults` would use and its conditional creation; then one `Needs these permissions`
block and one `Would create` block for the whole manifest, deduplicated in first-occurrence
order. A concrete rerun hint preserves already supplied input flags with shell-safe values.
The final line is:
`Rerun with the flags above, or add --defaults to use the listed values.`

JSON uses the standard `{"error":{...}}` envelope. `error.code` is the code above;
`error.message` names only the missing flags. `error.missing` is an array in manifest
order of `{input,flag,default,creates}`: `flag` includes `--`, `default` is the manifest's
display string, and `creates` is its conditional creation description or null when
nothing is created. `error.permissions` and `error.creates` are ordered string arrays
for the two blocks. No raw provider response, discovered resource identifier or URL
enters the refusal; the human rerun command may repeat only values the user supplied.
Unrelated errors retain their existing shape. New flags use `tmt-cli-style` command
registration; printed rerun commands must parse back to the same flags and values.

`--defaults` fills only omitted inputs. It never replaces an explicit flag and cannot
fill an entry marked `none: must be given`; those entries still refuse. A default
requiring provider state remains a typed planning intent, not an invented resource ID.
The backend planner resolves it, then freezes the actual value into the plan and its
digest before asking for authorization. This shared input layer performs no provider
calls or mutations. `--defaults` grants no consent: exact-plan y/N or `--authorize`
is unchanged; there is no `--yes` implication or new automatic execution path.

Reruns/resumes reuse the recorded inputs, including values chosen by an older default.
An explicit value that disagrees with the record refuses through the backend's existing
recorded-input conflict code before provider setup. A new default must never silently
move an old deployment. Existing record owners supply the saved values and retain their
schema/admission and conflict mapping; this contract introduces no parallel record.
The plan states every actual selected value. Agents, CI and `--json` remain plan-only
without the existing explicit digest authorization.

**Historical cloud-operation provisioning profile:** its [stacked authentication is superseded](#superseded-cloud-authentication-proposal).
Firestore provisions the admission/token service and its scoped service role, Auth configuration,
operation admission/permit/inbox/outbox/checkpoint collections, composed Rules, indexes/TTL and only
declared extension/blob resources. Cloudflare provisions the Worker, per-machine and declared
per-namespace Durable Object bindings, scoped connector credentials and only declared blob resources.
Private device/machine encryption material is never provisioned to either provider. A backend
with missing required services/permissions is refused before publishing an active binding.

A hosted Firestore plan includes the explicit `owner:register` step after its public configuration
and sign-in setup, before Rules publication and Verify. The short summary and details disclose
`Register this machine as owner`; its step ID is in the authorized digest. It reuses ordinary
anonymous Auth (included in the displayed plan), then uses the authorized deploy credential to
admit `/owners/<uid>` and create the pinned native machine row. Clients can never write `/owners`.
On uid loss, the same authorized step may update only the original owner's uid, preserve its native
key/descriptor, and retire the previous uid's matching owner entry in one conditional commit.
A conflicting key, inviter provenance or foreign allow-list binding refuses with
`REMOTE_DEPLOY_OWNER_CONFLICT`; nothing is overwritten. The original snapshot preconditions guard
against concurrent pin replacement. Exact reruns observe before writing, including a lost commit
answer; there is no mutation retry. Verify remains read-only and checks this step too. Deploying
these gates grants no native operation permission or public Core delivery activation by itself.

Remote records deployment identity, plan/declaration digests and per-resource outcomes durably in
its own state. Validate all declarations first; publish a usable binding only after the complete
authorized plan succeeds. A partial failure reports completed/pending resources and leaves new
routes disabled. Exact-plan retry resumes owned provisioning by stable resource identity, without
duplicating resources or sending agent work. A changed plan/account needs new authorization.
Never overwrite or delete unrelated account resources; deletion/data loss requires explicit
authorization in that concrete plan. Preserve an existing deployment on failed upgrade or report
its actual partial availability; never claim provider rollback restored data.

A Remote home retains one independent deployment identity per Firebase project under
`remote/deployments/<project>/deploy.json`, with one shared `deploy.lock`. Catalog edits and
record persistence use shared Document updates with bounded admission and atomic old/new snapshots;
Remote retains strict 0600 admission, schemas and the whole-run writer. Deployment persistence
errors retain the exact selected command; catalog errors give an equivalent add/rm command.
Neither repeats a provider effect automatically. SQLite
contention waits at most five seconds without bypassing the IMMEDIATE authority fence.
Plan-only reads
persist project-local v1 drafts without binding a region. The first authorized save binds
`{project, region}` before any effect; a different region for that project refuses with
`REMOTE_DEPLOY_PROJECT_CONFLICT` before provider setup, login or inventory, without reset.
A changed unfinished intent never refuses a plan-only preview. Human output and optional
`supersededRun:{planDigest,safe,reason,next}` explain replacement without undoing anything.
Explicit `--authorize` of the current digest may replace it only without an unknown web-app/site/config
or Version creation and without a staged Version, except the explicit expired-Version abandonment
path. Unsafe replacement returns `REMOTE_DEPLOY_PREVIOUS_CREATE_UNCONFIRMED` with the existing
no-repeat/recovery guidance or `REMOTE_DEPLOY_PREVIOUS_VERSION_STAGED` with the earlier-build or
expired-Version next step. The same digest resumes unchanged; an authorization of an older digest
still refuses. Accepted replacement retains the prior run with `state:"superseded"` in optional
`record.supersededRuns`, preserving all steps and checkpoints. Missing history stays omitted and
old records read unchanged at the same version; no provider resources are deleted.
A different project has its own identity and recovery history. Readers never migrate or
infer a target. An old single `remote/deploy.json` refuses with `REMOTE_SELECTOR_LEGACY`,
names that path and gives a command to move it aside; no legacy reader, conversion or move ships.
Removing local history leaves Firebase resources untouched; any later destructive takeover
still requires authorization of the plan's observed foreign fingerprint.

### Browser remote registry

The browser SDK's `remoteRegistry` stores public references using the native catalog's
`{version:1,entries:[{name,kind,url,project}]}` fields and name/URL grammar. It has at most
16 references and 8 KiB of UTF-8 JSON. This origin's `tmt-remote` IndexedDB `device` store
keeps the public catalog under `remotes-v1`, separately from the existing opaque device key
and pairing under `device`; no database version change or key migration occurs. Add, rename,
remove and confirmed import replace only that public entry. Removing a reference neither
signs out nor revokes access. Site-data clearing removes both; changing a pairing preserves
references. Registry validation, storage and concurrent-edit failures use the existing
`REMOTE_SELECTOR_INVALID`, `_UNAVAILABLE` and `_CONFLICT` family, not signed access evidence.

Local Open validates the destination and the whole public handoff before navigation to that
origin's `/remote/` entry; the stored public reference is unchanged.
The fragment carries `tmt-remotes` (base64url compact catalog JSON in the field order above)
and optional `tmt-page` (base64url compact `{spaceId,pageId}` JSON in that order, using Colab's ID
grammar). All values count within the 8 KiB encoded-fragment bound. Unknown, duplicate or
invalid members refuse the whole packet; no truncation or partial import occurs. The receiver
strips the entire fragment before admission and confirms imports before storing anything. The
page hint grants no access and is not consumed by this landing page; Colab resolves navigation
after its own admission. No keys, grants, sessions, credentials,
drafts, original operation IDs or reader seeds are carried.

A duplicate canonical target is already added; a colliding name offers a new name or Skip,
without overwriting the existing reference. Landing rows use only this origin's existing
verified observation for current access. Foreign rows say `not checked`, carry no state mark
and offer Open, with no probes or manufactured last-seen/default status. Firebase references
reuse the validated public remote link and remain `not checked`; Open preserves that link
unchanged, with no catalog handoff, Auth, configuration fetch or Firestore transport. Hosted
catalog reception and the Colab switcher are later consumers, not shipped registry behavior.

### Native remote selection

`add <name> <remote-link>`, `ls` and `rm <name>` manage an owner-only
public catalog in `remotes.json`, not grants, private keys or deployment ownership. Local
references have an absolute HTTP(S) door-root URL; Firebase references use the validated
[remote link](#backends-and-deploy). A name cannot shadow a different owned project id. Removing a
reference preserves cloud resources and deploy history. The native catalog has no current/provider
status observation; origin-local browser references use the separate
[browser registry](#browser-remote-registry).

`ls --json` returns `{version:1,remotes:[{name,kind,project,region,default}]}` (names are strings,
`kind` is `local` or `firestore`, project/region are strings or null, and `default` is boolean);
`add --json` returns `{added:true}`, and `rm --json` returns
`{removed:true,cloudResourcesChanged:false,deployHistoryChanged:false}`.
`settings --json` retains its existing members and adds the reference string `defaultRemote`
(`local` when unset) and `defaultRemoteSource: "default"|"settings.json"`; these CLI members do not
change the signed settings shape.

The single `--remote <ref>` selector accepts `local`, a catalog name or an owned project id.
Targeting surfaces resolve explicit selection, then `settings.json`'s optional `defaultRemote`,
then this home's local fallback. The fallback selects a sole owned project; several require
an explicit choice, never last-used or alphabetical selection. A missing or unreadable default
refuses rather than falling back. `settings default-remote <ref|local>` changes that default
without changing browser active selection. `deploy firestore --project` remains the explicit
create input and overrides a targeting default; if `--remote` is also given they must agree.
Status layers and link also accept an owned `--project`; when both selectors are given they must
agree. A public Firebase reference maps to an owned file
only when its projectId, appId and authDomain agree with that file's verified publication;
importing a link alone cannot claim ownership or trigger provider setup.

Owner-host serve/stop/pair/devices/approve/cancel/settings/catalog commands and ordinary status
always act on this home and ignore the default. An explicit other Local reference refuses
`REMOTE_SELECTOR_LOCAL_REQUIRED`; Firebase administration refuses
`REMOTE_SELECTOR_KIND_UNSUPPORTED`. No foreign-host administration client ships.
`REMOTE_SELECTOR_REQUIRED`, `_INVALID`, `_NOT_FOUND`, `_NOT_OWNED`, `_STALE_DEFAULT`,
`_UNAVAILABLE`, `_CONFLICT` and `_LEGACY` distinguish selection failures and include the repair
command.
Codes are JSON-only in CLI output; human errors and hints use plain words.

`status --layers` reads one selected owned snapshot without provider calls or the writer lock,
including while serve is stopped. It retains the existing layer definitions and adds
`firestoreDeployment: {project, region, deploymentId}` only for a bound selected snapshot;
ordinary status JSON and signed settings shapes stay unchanged. The signed settings optional
projection uses the same home-selection policy, returning unknown on ambiguous or damaged
state instead of merging evidence across projects. `link [--json]` derives the selected
Complete record's public link; it never creates or repairs a project.

A Firebase project has one deployer; tenants use identical composed Rules and share the project's
quota. Colab separately grants access to its tenancy spaces (`spaceId`), not deployment ownership.
Another home sees the deployer's Rules as foreign. Its authorized destructive plan can replace
those Rules, including a newer release: a tenant must not deploy just to obtain access. Remote
cannot enforce exclusivity across homes, so the single-writer-project limitation still applies.
Remote links carry public configuration only as defined in the deployment section; registry
selection and access are separate from deployment ownership.

**Firestore sharing layer (layer 1).** This layer deploys without the admission/token service. Its
plan adds the owner's database `(default)` (edition `standard`, one location), the sign-in providers
the layer needs, the declared indexes and the composed Rules to the extension plan, and the owner
authorizes the digest of that whole envelope (account, project, deployment ID and every item above); a
digest prefix of at least 12 hex characters names it. Steps run in a fixed order and the Rules
release follows additive prerequisites; a declared Hosting release follows it. A final
read-back step passes only when Rules match and every required index is `READY`; `CREATING` stays
building, lost or invalid evidence stays unknown, and `NEEDS_REPAIR` refuses without deleting the index.
Composite creation is reconciled by bounded exact-spec inventory; its operation acknowledgement alone is
not readiness. Live plus missing planned composites are counted before effects and again before allocation,
including unrelated indexes, separately from single-field configurations. Existing records are not converted;
a new authorized plan reconciles its required indexes. Index readiness is a recorded prerequisite of sharing, as defined by the status-layer projection. Each step is observed
first and applied only when absent, so an exact-plan retry skips what exists and never repeats an
effect; a provider answer lost after a possible effect is recorded `unknown` and resolved by the next
observation, never reported as failed. Equivalent user resources are adopted and kept;
owner-access metadata follows the registration rule above. A destructive replacement of foreign live Rules names their exact fingerprint in the
plan; authorizing that plan digest consents to replacement. A changed foreign fingerprint invalidates
that authorization and is refused before switching the release.
The deployed Rules begin with a Remote marker line naming the deployment and the digest of the
composed body, so ownership of a live release is read from its bytes; a Remote release edited after
it was published is foreign and needs a newly authorized plan naming its fingerprint. A console
step only the owner can do (for example creating a sign-in provider that needs an OAuth client)
ends the run as partial with a fixed instruction code, and the retry continues after the owner acts. The binding is written
in the same save that completes the last step. From the moment the Rules call may have been made
until the run completes no binding is usable; a failure before that call leaves the earlier binding
valid; running a finished plan again is a fresh check that keeps the binding until a Rules call is made.
Preflight HTTP failures expose `error.provider: {operation, status}` in JSON and name the
operation and numeric HTTP status in fixed human text. `operation` is one of `identity`,
`project`, `database`, `owner-registration`, `fields`, `indexes`, `rules-release`, `rules-set`, `web-apps`,
`web-app-config`, `web-app-operation`, `sign-in-config`, `google-sign-in-config`,
`hosting-site`, `hosting-channel`, `hosting-version`, `hosting-files`, `hosting-release`,
`hosting-upload`; `status` is a numeric HTTP code. No URL, resource name, project ID or
provider body crosses this error boundary. Non-HTTP failures omit `provider`. Classification
and accepted absence are unchanged: database/release 404 is accepted, list/ruleset 404 refuses;
403/429 retain their dedicated codes, 408/409/5xx remain unknown (SERVICE_DISABLED retains
its API-disabled code). No automatic retry or new retryable-state inference is made.
Records carry fixed reason codes, never provider text, and Remote fixes the owner-step instruction text:

- Refusals before any effect: `REMOTE_DEPLOY_PROJECT_INVALID`, `_LOCATION_INVALID`,
  `_DEPLOYMENT_INVALID`, `_SIGN_IN_MISSING`, `_AUTHORIZATION_STALE`, `_ACCOUNT_CHANGED`,
  `_ACCOUNT_UNREADABLE`, `_PROJECT_CONFLICT`, `_HOSTING_UNAVAILABLE`.
- Step faults: `REMOTE_DEPLOY_PERMISSION_DENIED`, `_API_DISABLED`, `_QUOTA_EXCEEDED`,
  `_PROVIDER_REJECTED`, `_OWNER_CONFLICT`, `_DATABASE_MISMATCH`, `_RULES_FOREIGN`, `_VERIFY_FAILED`.
- Owner steps: `REMOTE_DEPLOY_OWNER_INITIALIZE_AUTH`, `_ENABLE_GOOGLE_SIGN_IN`.

The public `<extension> deploy-declaration --json` reply has exactly six fields:
`version: 1`, `extension` (the requested name), `backend: "firestore"`, `declaration`
(exact UTF-8 declaration text), `artifact` (exact UTF-8 admission text), and
`declarationDigest` (lowercase SHA-256 of the declaration's UTF-8 bytes). Both texts
are bounded to 64 KiB; the strict JSON envelope is bounded to 790,528 bytes to allow
escaping. Unknown/duplicate fields, mismatched identity or digest, and invalid
artifact/declaration content refuse composition. One static read has a 10 s absolute
invocation bound. A nonzero reply with structured `error.code` `USAGE_ERROR`,
`EXTENSION_NOT_INSTALLED`, or `COLAB_INPUT_INVALID` means no declaration (the last
is old Colab's unknown-command response); other failures are unavailable. Stderr
and raw child errors are never disclosed. Missing bytes never select a fixture.

**Current implementation.** `tmt remote deploy firestore` composes captured installed
extension inputs through the existing deployment owner and real provider port. It defaults
to a plan with no provider effects for agents, CI, `--json` and non-interactive input;
only an explicit digest authorization can run those invocations. An interactive terminal instead
displays a short summary of the captured account, target and Rules/website changes; `--details`
shows the full plan instead. Database presence comes from the existing budget inventory;
unknown database/sign-in/index state shows its target, never an inferred change. A completed
record is not live no-change evidence, so fallback lines still require the question:
`Deploy these changes to your Firebase project? [y/N]`. Yes authorizes precisely those captured
bytes: discovery, inventory and plan preparation are not repeated after the answer; live account
and effect guards still run. No, EOF or cancellation while waiting makes no provider changes;
decline/EOF exits zero. There is no `--yes` flag. A saved identical authorized digest continues
without another question only while unfinished; a completed run asks again. A changed unfinished intent requires one
new confirmation and retains its superseded outcomes. That explicit terminal decision may leave
a known or possibly created unreleased Hosting Version untouched and unreleased, with the
corresponding warning before confirmation; it never bypasses an unknown web-app/site/config
creation. This exception requires a pending release; an attempted or completed release retains
the ordinary supersession fences. It
does not change non-interactive exact-digest authorization, the plan grammar, cause codes or
same-digest create no-repeat/upload-origin fences. A private deployment record preserves the
draft identity and saves run outcomes under a separate writer lock, publishing complete files
by atomic rename. Readers never take that lock, so status need not wait for a deployment run.
Running `status --layers` reads recorded project, sign-in, verified Rules and composite-index outcomes without
provider I/O; damaged records report unknown evidence. An incomplete Rules attempt withdraws
the old binding and reports partial Rules. The [status-layer requirement policy](#local-cli-discovery)
satisfies sharing's tier by its no-paid-plan design and quota by the required own-modeled-share
usage guard, without observing billing or project-wide headroom. A Complete record with verified
project, sign-in, Rules and composite-index evidence therefore reports sharing enabled with the limits
note; missing/damaged factual prerequisites retain their existing unknown/refusal meaning. Status adds
no provider call, runtime quota check, scope or poller. A checked emulator artifact is byte-bound to the real deployment output;
it proves Rules behavior, not provisioning or quota. Installed declarations arrive only through
fixed public extension commands dispatched by the supplied TMT executable, as bounded strict
JSON containing exact declaration/artifact UTF-8 bytes and the declaration digest. The
artifact digest is verified from the declaration; no installed asset path or cwd module is
read. Missing declarations are unavailable, never substituted. Colab's
`tmt colab deploy-declaration --json` command shipped in #2516 with compiled-in immutable bytes;
its [declaration section](../extensions/tmt-colab/contracts/colab-v1.md#firestore-deployment-declaration)
owns that contract. Installed end-to-end acceptance and real-project proof (#2166) still require
separate owner authorization; no cloud relay or machine connector is shipped (deploy
provisions only the project, Rules, sign-in providers and Hosting). The adapter
resolves firebase-tools from
the installed `firebase` executable's realpath and Node from its supported launcher; both
that embedded helper and declaration children start in a fixed neutral cwd. It uses a
version-gated firebase-tools login, emits only fixed diagnostics and never automatically
retries mutations. Rules switching
requires a single-writer project because no atomic provider precondition exists: re-read before
the final switch and exact read-back after it detect visible drift or report unknown without
a binding; they cannot exclude an interleaving writer.

[Start and pair](#provisioning-on-start-and-pair) automatically prepare the namespaces/bridge of
already authorized local or deployed resources and publish the device admission projection. They
do not authorize first deployment, new account resources or expanded provider permissions. Adding
an enabled extension with undeployed requirements leaves that cloud extension unavailable until
an authorized deploy; local availability and existing operation authority are unchanged.

### Free-plan budget guard

The shipped Rust/SDK model and guard, `status --budget` and the published-limit projection use
the dated free-plan table. Document transport is still separate; its effect path must apply this
guard before sending. Layer 1 must run on the provider's no-cost plan, and the provider's quotas are project-wide: no client
reads them and Rules cannot. A client therefore guards its own modeled share, not the provider's counter. For one
shared page with `members` listeners and `writers` of them appending, one append costs the project `writeLookups +
(members - 1) * (1 + readLookups)` reads (the writer's Rules lookups, then for every other listener the delivered
document and its lookups again). The writer lookups use the continuing non-owner member path;
the owning free-plan guide derives it from Colab's Rules. The lookups are fixed guard constants; the page's daily append
allowance is the daily reads pool divided by that, capped by the writes pool, and a writer's share is the allowance
divided by `writers`. Below 70% of the share an append is allowed, from 70% the client is warned, from 90% the append
is refused; both are named constants in one table. The allowance also gives the minimum average interval at which an
extension must flush coalesced updates. A client counts its own appends per US Pacific calendar day, because
provider quotas reset at the next Pacific midnight. A page whose modeled share is too small for the guard to allow even one
append is one the free plan cannot host: it is refused when the model is built, never as a recoverable pause that would
end at the next reset and begin again. The model covers one page and its appends only: the quota pool is project-wide, so
several active pages, attachment chunks and deletes are outside the guard, and the provider's `resource-exhausted` answer
below is the backstop for their sum. The quota numbers and their sources live in the Remote implementation guide with
their dates, not in this contract.

A refusal by the guard is made before any effect: it is the error `REMOTE_BUDGET_EXHAUSTED` with `resetAtMs` and
`retryAfterMs`, nothing was sent, and the same sealed update or original ID can be sent unchanged after the reset.
Remote queues nothing. A provider `resource-exhausted` answer to a request that may have changed state is an
unknown outcome, never `REMOTE_BUDGET_EXHAUSTED`: the caller reads back the original ID and does not resend. The same
answer to a request that cannot have changed anything is a refusal with the reset time. Either answer is recorded as
`exhausted` evidence for the layer readiness view, and that evidence says nothing after the next reset.

### Proposal: cloud conformance

**Authentication superseded:** see the [historical proposal status](#superseded-cloud-authentication-proposal).

Before cloud agent operation implementation acceptance, independent HPKE and application byte vectors plus
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
not evidence of implemented cloud agent operations; shipped extension page sync does not satisfy them.
