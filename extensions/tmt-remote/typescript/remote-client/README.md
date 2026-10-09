# Remote device SDK

This private module implements the device side of
[remote-channel-v1](../../../../contracts/remote-channel-v1.md):

- `src/canonical-bytes.ts`: decoded-value envelope, device enrollment, possession
  and `tmt-ext-cert-v1` signing-byte builders, the `K_response` and `serverProof`
  HMAC inputs, pairing-code decoding, strict unpadded base64url and four-word
  fingerprint indexes. Inputs are already decoded; structural checks enforce
  framing, exact UTF-8, decimal bounds, fixed profile values and kind/origin pairs.
- `src/device.ts`: a non-extractable WebCrypto Ed25519 device key, the pairing link
  parser and pairing client (retrying a pending candidate and accepting the machine
  key only after `serverProof` verifies), the `session.open` client that verifies
  the machine-signed response, and extension key certification.

- `src/operations.ts`: `operations(session, {timeoutMs?})` exposes `listAgents`,
  `send`, read-only `operation` and `result` over the verified session. Its private
  `session-channel.ts` owner shares one serialized lane across helper instances,
  signs exact payload bytes and verifies machine signatures, full correlation and
  increasing response sequences. Agent listing includes presence and optional
  core-published `delivery` unchanged; it never infers readiness.
- `src/budget.ts`: the free-plan Firestore budget guard, the twin of the Rust `firestore_budget`
  module. `BudgetModel.create(members, writers)` refuses a page the free plan cannot host,
  `decide` allows, warns or refuses an append before any effect (with the reset time), and
  `classifyProviderExhausted` keeps a provider `resource-exhausted` answer after a possible write
  an unknown outcome, never `REMOTE_BUDGET_EXHAUSTED`. Pure: callers pass the clock and keep their
  own per-day counter, which is per browser and device, so `decide` is a local estimate and the
  provider's answer is the backstop. `BudgetModel.minFlushIntervalMs()` gives the shortest average
  interval between flushed updates. The Rust tests and `test/budget.test.ts` share one set of vectors.
- `src/browser.ts`: the browser entry the door serves as `/sdk/remote-v1.js`. It
  exposes `pairingPage(link)` for the fragment-erasing `/sdk/pair.js` bootstrap and gives mounted extension pages `reopenSession`,
  `operations`, `ClientError`, `RefusalError` and `certifyKey`, whose extension comes from `/sdk/mount`, and the `budget` namespace of `src/budget.ts`.

`parseLink(link, descriptor)` validates `http://127.0.0.1:PORT/pair#CODE` with a
separately obtained public descriptor. `resolveLink(link, fetch?)` fetches the current
same-origin `/sdk/pair-offer` descriptor and validates both. Neither sends the code;
the pairing page's synchronous bootstrap captures and removes the fragment before
loading the SDK. One offer exists at a time; replaced or ended offers require a new link.

```ts
// Use Colab's existing session; constructing the helper opens nothing.
const remote = operations(session);
const agents = await remote.listAgents();
// The caller durably freezes operationId, agentId and message before sending.
try {
  const state = await remote.send({ operationId, agentId, message });
} catch (error) {
  if (!(error instanceof ClientError) || error.code === 'sequence_unavailable') throw error;
  // Recovery observes the original ID and never dispatches automatically.
  const recovered = await remote.operation(operationId);
}
```

The exported `RemoteOperations` interface has these signatures:

```ts
listAgents(): Promise<RemoteAgent[]>;
send(input: {operationId: string; agentId: string; message: string}): Promise<SendState>;
operation(operationId: string): Promise<SendState>;
result(requestId: string): Promise<ResultState>;
```

`SendState` preserves `held`, `accepted` (with `requestId`), `uncertain` (optional
`requestId`), `refused` and `cancelled` (optional `reason`), always with the original
`operationId`. `ResultState` preserves `pending`, `replied` (exact inert `message`,
including empty text) and `unavailable` (optional `reason`), always with `requestId`.
`RemoteAgent` contains `id`, `name`, `presence` and optional core-owned `delivery`.
These types and `SendInput` are exported by the browser entry.

Each signed transport attempt defaults to a 40-second timeout. `send` and `operation` return
`{state: 'refused', operationId, reason}` for verified pre-effect refusals:
`REMOTE_SCOPE_DENIED`, `REMOTE_INPUT_INVALID`, `REMOTE_RATE_LIMITED`,
`REMOTE_INTENT_CONFLICT` and `REMOTE_CLOSED`. The generic pre-admission HTTP 404
maps to `REMOTE_SESSION_ENDED`; it is a session-ended signal, not a signed response.
`listAgents` and `result` throw the exported `RefusalError` with a typed `code`
and optional bounded `retryAfterMs`; callers branch on `instanceof` and `code`.
The exported `RemoteRefusalCode` union includes those six codes plus existing
`REMOTE_INPUT_TOO_LARGE`, `REMOTE_STATE_UNAVAILABLE` and `REMOTE_CORE_UNAVAILABLE`.
These other signed refusals also throw `RefusalError` on `send`/`operation`.
Raw server messages are never exposed.

Unknown outcomes throw the exported `ClientError`, with `code` from the exported
`ClientErrorCode` union: `transport_failure`, `timeout`, `unverifiable_response` or
`sequence_unavailable`. `send`/`operation` errors retain `operationId`. Transport
status never proves acceptance. After a timeout, lost or unverifiable response at
sequence n, read `operation(originalId)` on the existing session. Before the next
call of any kind, the helper synchronizes internally using scope-free read-only
`capabilities` at n+1 and, only after a verified `REMOTE_REPLAY` refusal, retries
once at n. It then performs the caller's call. Signed refusals also mark the
sequence ambiguous, since refusal may precede or follow sequence consumption.
No third sequence guess is allowed. A lost recovery response or two replay refusals produce
`sequence_unavailable`; that helper session can no longer be used. The caller then
explicitly reopens and observes the original ID. `REMOTE_CLOSED` and
`REMOTE_SESSION_ENDED` also require a caller-owned reopen. Reopening adds a fresh
session and leaves existing sessions and tunnels live. Normal recovery never reopens it.

The SDK never resends automatically or generates a replacement dispatch ID. The
caller owns durable IDs and exact intent; the SDK stores no dispatch payload in
IndexedDB. An explicit identical resend reconstructs the same payload bytes;
recovery observations themselves never send. Local invalid input or signing
failure raises `TypeError` before publishing. A plain copied or fabricated
`Session` contains no signer and cannot construct an operations helper.

`pnpm build` bundles the browser entry with Vite+ on the aliased Vite core (library mode, unminified) into
`../../rust/tmt-remote/assets/remote-v1.js`, which the door embeds; commit the
result. `pnpm test:browser` uses `vp exec playwright test` to run the Playwright Chromium pairing smoke against
`rust/target/debug/tmt-remote` (or `TMT_REMOTE_BINARY`).

Network access goes through an injected fetch. The caller persists the device
key's opaque `CryptoKey` (the browser page uses IndexedDB structured clone); the
private key is never exported. Browser `reopenSession` adopts the current same-origin
route path after verifying the machine identity, so schema 6's one-time prefix replacement
retains pairing. Other SDK callers must discover and adopt the current address explicitly;
old path links and cookies stop working. Byte construction and signatures establish no
authority: live grants, timestamps and replay are checked by remote.

Production uses TextEncoder and native WebCrypto; no Node imports or third-party
crypto. Payload bytes are copied before asynchronous hashing and are never parsed,
normalized or reserialized. Fingerprint indexes point into the pinned BIP-39
English list at `../../rust/tmt-remote/assets/bip39-english.txt`; callers map
indexes to words.

From the repository's `typescript` directory with Node 22.12.0 or later, pinned pnpm and Python 3:

```sh
pnpm --filter @tmt/remote-client install --frozen-lockfile --ignore-scripts
pnpm --filter @tmt/remote-client --fail-if-no-match check
pnpm --filter @tmt/remote-client --fail-if-no-match test
```

The test command checks the independent Python 3 oracle before running the
workspace-pinned Vite+ test runner with explicit `vitest.config.ts`. Oracle failures stop the command before the test runner;
assertion failures and missing tests also fail the command. Type checking remains
in the separate `check` command.
The existing unconditional Code quality CI job runs these commands using the
repository default Node 22.23.2, including for changes confined to this directory.
Run the same test command with Node 24 for the contract conformance target. No
release or distribution entry is added.

`test/reference.py` independently transcribes contract field order using Python's
standard-library `struct`, UTF-8 encoder, `hashlib` and `base64`, and refuses
to run if the pinned wordlist digest differs. Committed `vectors.json`
contains literal full bytes and SHA-256 values, first established with Python
3.14.7, rather than generated from the TypeScript implementation. The Unicode
fixture data deliberately includes astral and decomposed characters. Raw example
public keys and MACs prove framing only, not valid cryptographic proofs. Regenerate
with `python3 test/reference.py` from this directory, then format `test/vectors.json`
with `corepack pnpm@10.33.0 exec vp fmt --config vite.config.ts test/vectors.json`
(the package's Vite+ formatter, also required by `check`). The reference generator's
`--check` compares parsed values, verifying fixed bytes without rewriting them.
The TypeScript tests compare these literal artifacts
and mutate one condition at a time to demonstrate refusal and exact byte binding.

Certificate signature conformance also consumes the Rust-owned fixed WebCrypto
vectors over the Python oracle's exact certificate bytes. Tests reproduce the
signatures for `sign` and `enc` and reject changed domains, LP endianness,
extension names, purposes, keys, times and signatures. The Chromium smoke tests
silent certification for both purposes and retained signatures after revocation:
the grant loses live owner context and cannot reopen, even though its old
certificate signatures still verify. Extensions enforce certificate freshness
and learn revocation from Remote's device events; a certificate alone grants no
authority.

The operations unit tests extend that same node:crypto stand-in door with real
signatures, serialized client sequences and independent machine sequence gaps. They
cover all four calls, signed refusals and states, empty finals, correlation/audience/
session/operation/sequence tampering, frozen inputs, bounded timeout/lost-response
recovery and an explicit identical resend. The Chromium smoke additionally reads
agents, sends a direct request and reads its operation/final through the real Remote
door with a deterministic public-core fixture, asserting one core dispatch. It does
not claim real-core agent execution.

Set `TMT_REMOTE_CAPTURE_DIR` to an absolute output directory when running `pnpm test:browser`
to export pairing, four-word confirmation, static landing and page-error PNGs at 1440 and 390 px
in light and dark. The browser-page test uses the real debug door and asserts local-only requests,
CSP compliance, token colors, square hard shadows and no horizontal overflow.

## Several tabs and transport lifetime

Tabs share the paired device key in IndexedDB and one HttpOnly door cookie. Each
`reopenSession()` adds its own session with independent client/server sequences and one
serialized request lane. The cookie authenticates device context; it does not select a
tab's signed request lane. Sessions default to a limit of 8. The owner may set `tmt remote
settings sessions-per-device <n>|off`; changes apply at the next open. Unset means 8;
`off` means unlimited. With a limit, opening evicts the device's most idle session
without a live transport first, falling back to the most idle attached session.
Eviction closes its transports with `REMOTE_SESSION_EVICTED`. `RefusalError.limit`
exposes the active cap; send/operation refused states also carry `limit`. Colab can
explain that more than that number of tabs were open and show `tmt remote settings
sessions-per-device <n>` to raise it. Eviction is distinct from silent reopen after
ordinary transport loss.

Use `transportUrl(session, mountedWebSocketUrl)` when constructing each mounted WebSocket.
It adds the non-secret `tmt-session` identifier; Remote checks the live cookie's device
owns that session and removes the identifier before forwarding. It cannot authorize
without the cookie. The WebSocket scheme must match the door: http uses ws, and https uses
wss. Use the resulting URL only for the transport: never navigate to it, store it in page
history, log it, or copy it into Location/Referer. Existing URLs without it use the
cookie's session.

Closing the session's last transport starts a fresh 60-second inactivity grace, including
a transport lost while the tab sleeps or is backgrounded. Every session without a live
transport uses this grace; authenticated HTTP activity renews it and reattaching within it
resumes the same session. Sessions with a live transport retain the 12-hour idle limit.
Detached sessions count against the per-device cap until they expire. Explicit end and
eviction remain immediate. All sessions/tokens end on revoke, grant expiry/revision change,
or stop. Held work survives a session end and remains approvable under the live device grant; only stop,
revoke or grant expiry/revision change cancels it. After approval, a reopened or other tab
can recover by operation ID and observe the shared journal. The existing per-device hold
bound still applies; the approval prompt stays unchanged and does not identify a tab.
Dispatching/uncertain work keeps its original ID and recovery.

For list/result calls, `RefusalError.code` is `REMOTE_SESSION_ENDED` after session
idle expiry, or `REMOTE_SESSION_EVICTED` after limit eviction. Optional
`RefusalError.settingsUrl` (also on refused states) is absent when the door omits it or
sends null. It will identify Remote's settings page once #1769 adds it; until then the
door omits it. When present, the SDK exposes a normalized URL on the door's own origin.
Colab can show that link plus the command; when absent, show the command only. For
send/operation these codes appear in `{state:'refused', operationId, reason}`. A signed
distinct end reason is available for 60 seconds, then a generic 404 still maps to
`REMOTE_SESSION_ENDED`. The caller can silently `reopenSession()` and attach its new
transport; other tabs remain live. Recover previously unknown send outcomes by observing
the original operation ID after reopening. Never retry a send automatically because its
transport closed.

## Remote management

`management(session)` uses the same verified serialized Session channel as `operations(session)`.
It exposes `settings()`, `devices({cursor,limit})`, `set`, `rename`, `revoke` and
`operation(originalOperationId)`. Every read and effect retains live-grant admission. The [owning protocol](../../../../contracts/remote-channel-v1.md#remote-management-protocol)
specifies exact shapes and bounds. Session caps are positive decimal **strings** or null, preserving
native values beyond JavaScript safe integers; default reads as `"8"` and null means unlimited.
Values/sources, malformed/default warning and management capabilities come from server admission.

Freeze a UUIDv4 and input before a mutation. A verified refusal, committed result and unknown
outcome are distinct. `ClientError` after publication retains the original operationId; no helper
resends, reopens, creates a replacement mutation ID or designates a browser automatically.
Read `operation(originalId)` explicitly, without submitting the setter again. Unknown/pending
receipts never trigger a write; current values do not establish original commit.

After self-rename, explicitly reopen a fresh verified Session using the still-live grant and read
the original receipt. After a lost self-revoke acknowledgment, make one fresh read-only admission
attempt with the same paired identity/current trusted door descriptor. If admission is refused,
show lost current access **and unknown operation outcome**, with `tmt remote devices` for local
confirmation. Do not claim revoke committed or offer a mutation retry. Transport failure or an
unverified/stale descriptor is unconfirmed access and unknown outcome. All reads require a live
grant; no historical-key reader or general recovery endpoint exists. Losing designation alone
still permits a live grant to read its own original receipt.

Verified generic management state/ownership/publication errors raise
`ClientError("outcome_unconfirmed", ..., originalId)` after publication. A valid signature
alone does not prove a pre-effect refusal. Keep that unknown outcome and read only the
original operation; never resend it. Read `RefusalError` and agent behavior are unchanged.

Management storage has cumulative limits of 1000 retained identities per caller and 4000
per installation, including expired rows. The 30-day horizon bounds outcome availability,
not physical deletion or a rolling allowance. `REMOTE_MANAGEMENT_CAPACITY` refuses new
adoption before effects; an existing original-ID read still works at capacity. Show the
local CLI management path, preserve any earlier unknown outcome, and do not retry, reset
the database or invent a replacement ID to bypass the limit. Compaction is deferred.

The Remote-owned `/settings` page uses the shared browser presentation with three sections and
server-admitted values/sources/warnings/capabilities. Its separately built `settings-v1.js`
imports the served SDK; no second channel or public page export is introduced. Checked shared CSS
owns presentation, while Remote owns layout, native selects and control state. An untouched default
cap is not saved as explicit 8; there is no invented reset setter. Draft text and original
intent remain separate, including after unknown outcome or refreshed reads.

The page explicitly calls `reopenSession(previousSession)` for original-outcome recovery.
It binds to the existing paired identity and trust pins, never a separately persisted old key.
Before any descriptor read or admission, the stored machine pin must be a 32-byte Uint8Array
matching every byte of the verified prior Session pin. Invalid or replaced pins produce
`REMOTE_SESSION_ENDED` without network access.
There is one fresh admission attempt. An HTTP refusal with an unchanged current trusted door
descriptor is current-access refusal; a changed/stale/malformed/unavailable descriptor or
transport/unverified reply is unconfirmed. Neither establishes committed revoke or permanent
grant loss. A descriptor recheck is another read, never another admission/mutation. Existing
no-argument reopen behavior is unchanged.

The settings page retains the current device page during value/effect/recovery refreshes.
First/next navigation focuses an unsent device name and refuses to leave until that name is saved
or restored to its admitted value. Only the current bounded page's forms are retained.

## Remote browser entry

The human `tmt remote serve` link opens `/`; the JSON `address` remains the signed protocol base.
`landingPage()` is the entry bootstrap, using the same saved non-extractable key and machine pins.
On load it validates local storage, then checks once through a verified Session and signed
`capabilities` read; a saved pairing alone never proves current access. Manual Check again reuses
that Session while current. An expired Session is silently reopened once within the manual check;
an ended capabilities response is not a pairing refusal. No polling, automatic re-pair, grant
renewal, inventory or work is sent.
Only a verified signed refusal means Not accepted; opaque404, transport or unverifiable replies
remain unconfirmed and keep the pairing unchanged. Details contains the short machine ID,
viewer-local checked time, protocol address, trust pin and administration note. Commands are
copyable and run on the machine running Remote. The owner ceremony is unchanged: open the full
code-bearing link in this browser, compare its words with the terminal and confirm there.
