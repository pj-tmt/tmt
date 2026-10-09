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
  sent. New `Link:` values use the short owner URL from admitted page/catalog composition
  (`short-links.ts`); the full scope stays in the signed record. Source admission still requires
  the canonical full owner fragment (or none); other fragments, queries and `/read` paths are
  refused, never stripped. Older captured full-fragment links remain valid. Explicit Enter captures and sends the frozen intent; admitted outcomes remain inline in the conversation. `signed` frames the 15-field `tmt-colab-send-v1` input (default validity one hour,
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
  refused with `REMOTE_SESSION_ENDED` before admission stays `refused`. The SDK also
  resolves signed eviction Send/operation refusals as states, carrying the positive
  limit and optional settings URL. `sessionEviction` derives the typed page fault
  without replacing a Send's known `refused` outcome. The adapter retains the first
  verified eviction for its old Session; later ENDED reads or opaque socket close
  cannot authorize reopening. Other refusals use the reviewed
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
  Explicit recipient replacement uses the existing own ledger: `checkRetry` blocks on
  any non-provably-unsent record for that thread/message/machine/agent, and `adopt`
  repeats the check under a pair-scoped Web Lock. One durable IndexedDB pair marker
  holds only the latest replacement UUID, written before adoption; an absent or
  non-provably-unsent admitted record blocks stale tabs. Only authoritative
  `adopted: false` releases it; storage errors fail closed. `ask-remote.ts` owns the predicate.
- **`ask-attempt.ts` (`AskController`).** `prepare` captures synchronously against the
  current Remote context. `send` is the only Remote write: recheck context, sign, adopt
  (durable), record `dispatching`, recheck expiry and context again, then `remote.send`; a
  failure after adoption records `uncertain` if the send started, else `failed`.
  `recover` only calls `operation` and `result`, publishing state and the final as records;
  an interrupted `dispatching` ask becomes `uncertain` (`OBSERVATION_INTERRUPTED`) first, and
  a missing operation stays `uncertain` and can be abandoned. A refused read is an ephemeral
  `ReadRefusedError`, never a ledger state; the observer keeps backing off, while a
  session-ending refusal stops it. Each completed permitted observation cycle
  reports one aggregate unavailable boolean through `LiveAsk` to Live's existing
  projection. The warning stays visible across ordinary page publications and
  clears on the next successful cycle; closed/replaced/hidden observation cannot
  publish a late status. This changes no ledger state or retry cadence.
  `observe` makes one sequential activation pass
  over the 256 newest unresolved owned asks on page open and visible-again, including
  old intents. Continued visible polling backs off from 2 s up to 30 s and stops
  at the two-hour operation horizon; failed older reads are not retried.
  It never sends on reload or reconnect. `abandon` applies only to
  `uncertain`, records `MAY_HAVE_BEEN_DELIVERED` and cancels nothing. On a Session fault
  an adopted send ends `uncertain` (a typed sequence failure too) and an `accepted` ask's
  records stay unchanged; pre-admission session-end or eviction Send refusals stay `refused`.
  An eviction operation read leaves the prior ledger unchanged. Both returned
  eviction paths carry the typed limit/settings notice and stop further work.
  The controller then calls `sessionEnded` once, after publication, refuses further work and
  stops observing.
- **`writer.ts` and the fold Worker.** `Writer.submitOwn` is generic over the own roots
  (`threads`, `intents`, `messages`, `replies`) and imports nothing from Ask. It has the
  Worker `prepare-own` a candidate update (immutable per key, size-bounded, not committed),
  then submits it through the same `submit(update, 'own')` path as content, so sequence,
  Web Lock and exact-envelope staging are shared. The decoder state commits only after the
  append is admitted.
- **`ask-again.tsx`.** One trusted action serves composer-local pre-adoption failures and
  own signed refusals from `AskPanel`. It refreshes the exact UUID pair, captures the
  original comment and calls `LiveAsk.prepare` with `retryOf`; `LiveAsk` checks the own
  ledger and re-admits the comment again before Send. The [discussion contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#inline-annotation-conversations-1587)
  owns eligibility, capture/reload lifetime and one-action-per-recipient behavior.
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
- **Read-only agent status.** `AskController` owns one current-context/signed-directory
  read shared by Ask destination admission and `LiveAsk.observeDestinations`. The latter
  returns `AgentDirectoryObservation`: a local check time and admitted presence rows,
  or a session/directory phase with bounded session-end, verified eviction or refusal codes;
  unexpected failures expose no raw diagnostics. Observation does not populate the Ask
  preview cache, publish, dispatch or invoke Live's session recovery callback. It checks
  current page admission and the same active connection before and after the read; a
  closed/replaced facade refuses late results. Existing Ask destinations retain their
  separate fail-closed session lifecycle response. No writer admission is required for
  this read, and observing status never grants a send capability.
- **Reading asks.** `pageAsks`/`readAskViews` run per admitted writer; other writers' asks
  verify with `Objects.ownSigningKey`, a display-only key captured from an authenticated,
  cut-admitted own envelope (revoked history stays inert and grants no authority). Slow
  verification keeps one active and the latest pending snapshot, so source edit and export
  keep reading the committed document.
- **UI.** `annotation-input.tsx` owns one trusted Send across Chat, thread and annotation:
  exact visible mentions choose UUID pairs, one comment is recorded, and each distinct
  recipient gets one frozen Ask under current admission. Plain text records a comment.
  The [discussion contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#inline-annotation-conversations-1587)
  defines creator defaults, binding, fan-out limits and per-recipient failures.
  No confirmation or sent-bytes disclosure is offered. `thread-panel.tsx`
  renders verified replies inline and puts Edit (own annotation comments) and Delete
  (own) in the square `⋯` menu (`components/action-menu.tsx`); held/recheck/uncertainty keep the existing ledger.
  `chat-panel.tsx` replaces standalone Ask with one bottom input and page-visible
  null-anchor threads. `ask-panel.tsx` displays verified ledger outcomes/replies through the shared
  `components/conversation-turn.tsx` and
  owns trusted recheck/abandon actions; it has no composer or dispatch button. Test
  IDs remain `ask-entry`, `ask-state`, `ask-reply` and `ask-reply-attribution` for
  pending delivery and admitted replies; `ask-entry` retains the admitted ledger
  state independently of the disappearing delivery status. `chat-toggle` opens the pane; `chat-panel` scopes its shared
  Message combobox. The shared plaintext/history Lexical editing boundary and
  reset/recipient ownership are defined in [architecture-state](architecture-state.md#message-editing-boundary).

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
