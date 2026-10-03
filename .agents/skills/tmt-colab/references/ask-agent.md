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
  (`Page:`, `Link:` without fragment, `Quote:`, `Comment:`; http(s) URL without
  credentials) and the preview-only `deliveredMessage`, which prepends Remote's
  `[remote: <deviceName>]` line. Only the unprefixed `finalBytes` are digested, signed and
  sent. `signed` frames the 15-field `tmt-colab-send-v1` input (default validity one hour,
  at most 24); `escapedPreview` shows control and format characters without replacing bytes.
- **`ask-remote.ts`.** `RemoteClient` port and `createRemoteClient`, which wraps the exact
  verified registration Session in the served SDK's `operations` helper. It never reopens
  the session (that would end Live's tunnels); the SDK owns sequence resync and same-ID
  reads. `context()` combines `api/session` (device, name, grant revision) with
  `/sdk/mount` (machine). A failed `send` or `operation` becomes `uncertain`, never a
  retry. `listAgents` keeps id, name and presence only; the port has no delivery field and
  no `check`. A Session fault (SDK `RefusalError` `REMOTE_SESSION_ENDED`, `ClientError`
  `sequence_unavailable`, a verified refused-`REMOTE_SESSION_ENDED` state, an expired
  Session or a changed grant revision) is normalized to `SessionEndedError` or an
  `uncertain` state with that reason. Registration must rebuild the client and its
  controllers when it replaces the Session; an old client never adopts a new one.
- **`ask-records.ts`.** Record types `ask`, `ask-state`, `ask-reply`, the ledger states and
  `canTransition`. `readAskRecords` (alias `readAskViews`) reads only the admitted
  per-writer projection and verifies each ask's signature with that writer's key; a
  writer, request ID or agent claimed inside a body never selects another stream. An expired
  ask stays readable; effect checks happen in the controller.
- **`ask-record-store.ts` (`AskRecordStore`).** The own stream is the ledger. `adopt` verifies
  the signed ask and its scope, then stores the local draft (`storeAskDraft`, in
  the same module; signed input and signature only) and publishes the ask; the same ID with
  other bytes is
  `INTENT_CONFLICT`. A stored draft never authorizes another effect. `state` and `reply`
  write immutable revisioned records, validated here, not in the Writer. All writes for one
  ask run under the Web Lock `ask-ledger:<space>:<page>:<device>:<id>` (`exclusive`).
- **`ask-attempt.ts` (`AskController`).** `prepare` captures synchronously against the
  current Remote context. `send` is the only Remote write: recheck context, sign, adopt
  (durable), record `dispatching`, recheck expiry and context again, then `remote.send`; a
  failure after adoption records `uncertain` if the send started, else `failed`.
  `recover` only calls `operation` and `result`, publishing state and the final
  as records. `observe` runs while the page is visible, backs off from 2 s up to 30 s, stops
  after two hours and never sends on reload or reconnect. `abandon` applies only to
  `uncertain`, records `MAY_HAVE_BEEN_DELIVERED` and cancels nothing. On a Session fault
  the controller publishes `uncertain` (an `accepted` ask's records stay unchanged), then
  calls `sessionEnded` once, refuses further work and stops observing.
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
