# Ask agent modules (#1110)

Ask agent is browser-direct: the asker's paired device calls Remote operations through the
Remote served SDK and records the ask, its states and the reply in its own Colab stream.
There is no native bridge, ledger, schema, route or command. Local v1 is owner-only.
Wire grammar and the signed-tail vectors are in
[colab-v1](../../../../extensions/tmt-colab/contracts/colab-v1.md); this file maps the
modules in `extensions/tmt-colab/typescript/app/src` and `rust/tmt-colab/src/ask.rs`.

## Modules

- **`ask-intent.ts` (`FrozenAsk`).** Built from an `AdmittedSelection` (the trusted parent's
  admitted page/source selection; a renderer message or claimed name never is one) and a
  caller-verified `AskDestination`. It freezes the transport message
  (`Page:`, `Link:`, `Quote:`, `Comment:`; http(s) URL without credentials) and the preview-only `deliveredMessage`, which prepends Remote's
  `[remote: <deviceName>]` line. Only the unprefixed `finalBytes` are digested, signed and
  sent. `Link:` is the page's own mounted URL with the canonical `#space=&path=` fragment built
  from the selection; any other fragment, a query or a `/read` path is refused, never stripped. Explicit Enter captures and sends the frozen intent; admitted outcomes remain inline in the conversation. `signed` frames the 15-field `tmt-colab-send-v1` input (default validity one hour,
  at most 24); `escapedPreview` shows control and format characters without replacing bytes.
- **`ask-remote.ts`.** `RemoteClient` port and `createRemoteClient`, which wraps the exact
  verified registration Session in the served SDK's `operations` helper. It never reopens
  the session (that would end Live's tunnels); the SDK owns sequence resync and same-ID
  reads. `context()` combines `api/session` (device, name, grant revision) with
  `/sdk/mount` (machine). A failed `send` or `operation` becomes `uncertain`, never a
  retry. `listAgents` keeps id, name and presence only; the port has no delivery field and
  no `check`. A Session fault (SDK `RefusalError` `REMOTE_SESSION_ENDED`, `ClientError`
  `sequence_unavailable`, an expired Session or a changed grant revision) is normalized to
  `SessionEndedError` or an `uncertain` state with that reason. A verified send that Remote
  refused with `REMOTE_SESSION_ENDED` before admission stays `refused`. Other refusals use the nine reviewed
  `REMOTE_REFUSAL_CODES`; anything else is `REMOTE_REFUSED`. Registration must rebuild the client and its
  controllers when it replaces the Session; an old client never adopts a new one.
- **`ask-records.ts`.** Record types `ask`, `ask-state`, `ask-reply`, the ledger states and
  `canTransition`. `readAskRecords` (alias `readAskViews`) reads only the admitted
  per-writer projection and verifies each ask's signature with that writer's key; a
  writer, request ID or agent claimed inside a body never selects another stream. An expired
  ask stays readable; effect checks happen in the controller.
- **`ask-record-store.ts` (`AskRecordStore`).** The own stream is the ledger. `adopt` verifies
  the signed ask and its scope, then stores the local draft (`storeAskDraft`, in the same
  module; signed input and signature only) and publishes the ask with two display labels,
  `agentName` and `deviceName` (publisher-asserted, outside the signed input, at most
  128 UTF-8 bytes each, validated by the browser codec and by `ask.rs`). UUIDs keep
  authority and routing; `readAskViews` exposes the names and an empty label falls back.
  The same ID with other bytes is `INTENT_CONFLICT`. A stored draft never authorizes another effect. `state` and `reply`
  write immutable revisioned records, validated here, not in the Writer. All writes for one
  ask run under the Web Lock `ask-ledger:<space>:<page>:<device>:<id>` (`exclusive`).
- **`ask-attempt.ts` (`AskController`).** `prepare` captures synchronously against the
  current Remote context. `send` is the only Remote write: recheck context, sign, adopt
  (durable), record `dispatching`, recheck expiry and context again, then `remote.send`; a
  failure after adoption records `uncertain` if the send started, else `failed`.
  `recover` only calls `operation` and `result`, publishing state and the final as records;
  an interrupted `dispatching` ask becomes `uncertain` (`OBSERVATION_INTERRUPTED`) first, and
  a missing operation stays `uncertain` and can be abandoned. A refused read is an ephemeral
  `ReadRefusedError`, never a ledger state; the observer keeps backing off, while a
  session-ending refusal stops it. `observe` runs while the page is visible, backs off from 2 s up to 30 s, stops
  after two hours and never sends on reload or reconnect. `abandon` applies only to
  `uncertain`, records `MAY_HAVE_BEEN_DELIVERED` and cancels nothing. On a Session fault
  an adopted send ends `uncertain` (a typed sequence failure too) and an `accepted` ask's
  records stay unchanged; a pre-admission `REMOTE_SESSION_ENDED` refusal stays `refused`. The
  controller then calls `sessionEnded` once, after publication, refuses further work and
  stops observing.
- **`writer.ts` and the fold Worker.** `Writer.submitOwn` is generic over the own roots
  (`threads`, `intents`, `messages`, `replies`) and imports nothing from Ask. It has the
  Worker `prepare-own` a candidate update (immutable per key, size-bounded, not committed),
  then submits it through the same `submit(update, 'own')` path as content, so sequence,
  Web Lock and exact-envelope staging are shared. The decoder state commits only after the
  append is admitted.
- **Native.** `ask.rs` decodes and verifies a `SignedAsk` (strict framing, canonical ID
  list, window of at most 24 hours, digest of the final bytes, operation and sender
  matching) and the decoder validates the `intents`, `messages` and `replies` roots with it.
  Nothing there dispatches or persists.

## Page wiring

- **Session and client.** `registration.ts` keeps the exact Session returned by
  `sdk.reopenSession()` as `remoteSession`. `mounted.ts` builds one shared `RemoteClient`
  from it before any sync `Connection` opens; an SDK without `operations` leaves Ask
  unavailable and source usable. On reconnect it coalesces re-registration, owner and
  same-device verification and a new client into one shared replacement.
- **`live-ask.ts` (`LiveAsk`) and `live.ts`.** `LiveAsk` composes the controller and store for
  a page: constructing or reconnecting it sends nothing, every effect starts from an explicit
  trusted action. `Live#replaceAsk` closes the old facade and builds the new one, binding the
  store to the registration keys, the admitted own state and `Writer.submitOwn`. The
  observer runs only while the page is visible and has subscribers, and a Session end fails
  the Live connection.
- **Reading asks.** `pageAsks`/`readAskViews` run per admitted writer; other writers' asks
  verify with `Objects.ownSigningKey`, a display-only key captured from an authenticated,
  cut-admitted own envelope (revoked history stays inert and grants no authority). Slow
  verification keeps one active and the latest pending snapshot, so source edit and export
  keep reading the committed document.
- **UI.** `annotation-input.tsx` uses the same Ask binding for direct explicit Enter
  sends; it freezes the current text and captured conversation references without a
  confirmation screen; no surface offers a "show what was sent" view. In Chat it
  prefills `@agent` (last replier, else publisher, else the only reachable agent) and
  Enter without a recipient shows an error and opens the list. `thread-panel.tsx`
  renders verified replies inline and puts Edit (own annotation comments) and Delete
  (own) in the square `⋯` menu (`components/action-menu.tsx`); held/recheck/uncertainty keep the existing ledger.
  `chat-panel.tsx` replaces standalone Ask with one bottom input and page-visible
  null-anchor threads. `ask-panel.tsx` displays verified ledger outcomes/replies and
  owns trusted recheck/abandon actions; it has no composer or dispatch button. Test
  IDs remain `ask-entry`, `ask-state`, `ask-reply` and `ask-reply-attribution` for
  admitted history. `chat-toggle` opens the pane; `chat-panel` scopes its shared
  Message to agent combobox and exact-byte disclosure.

## Invariants and gotchas

- Reload, reconnect, a timer or an observer never dispatches. A resend, even with the same
  operation ID, is not offered: an `uncertain` ask can only be re-checked or abandoned.
- Preview and signed bytes differ by exactly the `[remote: <device name>]` line; the
  recipient sees the prefixed text, so acceptance asserts that text verbatim.
- `agents.list` reports presence only in v1, so the UI shows presence and no delivery state.
  Absent mode or expiry evidence shows as unavailable.
- Tests: `test/ask-ledger.test.ts` (own record before effect, concurrent same-ID sends,
  absent recovery and abandon, rename or revision change before publication, per-writer
  attribution, shared-session SDK use, visible bounded observation), `test/own-fold.test.ts`
  (prepared own records) and `test/ask.test.ts`. Real-binary cases are in
  [acceptance.md](acceptance.md); doubles do not replace them. Check the vectors with
  `contracts/vectors/send-preview-reference.py` in a throwaway virtualenv with
  `cryptography`; `--write` only for a reviewed regeneration.
