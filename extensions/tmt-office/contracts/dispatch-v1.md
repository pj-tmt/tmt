# Local Office dispatch v1

This is the host capability for explicit request or announcement composition. It is not a shell
runner or second inbox. Spatial
extensions call the same capability; their artwork grants no authority.

## Endpoint and input

`POST /api/v1/local/dispatch` uses the existing local browser bearer, exact
loopback Origin and JSON content-type checks. Missing authority and malformed
input are rejected before opening storage. The caller is the local Office owner;
the endpoint accepts no caller-selected sender identity, command or file path.

```json
{
  "operationId": "11111111-1111-4111-8111-111111111111",
  "recipientIds": ["22222222-2222-4222-8222-222222222222"],
  "message": "Please review this draft."
}
```

Operation and recipient IDs are canonical, non-nil UUIDs. Select 1–64 UUIDs per
composition; 64 bounds a single SQLite transaction, not the identity directory.
Duplicate selections are deduplicated and ordered before intent comparison.
The message must contain non-whitespace text and respects the existing 1 MiB
exact UTF-8 request limit. Content is not trimmed, normalized or executed.
The wire envelope permits worst-case JSON escaping plus the bounded UUID list.
Duplicate and unknown JSON fields are rejected.

Optional `kind` is `request` (the default) or `announcement`. Null and unknown
kinds are rejected. Announcements use the same durable inbox, audience fence,
attention, retention and receipt as requests, but never wait for or accept a
response. This is a delivery policy, not a separate transport or queue. Explicit
`request` and omitted kind have the same intent digest, preserving existing
receipts. Changing kind under an accepted operation ID is an idempotency conflict.

The view must show the exact selected audience and message before explicit Send.
Drawing, opening a prop or copying a reference never submits this endpoint.
Optional `room` has explicit semantics, independent of presence or physical area:

- `{kind:"direct",roomId}` gives one recipient meeting context. The canonical
  request service checks current membership at enqueue; it does not require a
  full roster or a room revision. A missing membership returns HTTP 409
  `ROOM_RECIPIENT_NOT_MEMBER` without a request or operation receipt.
- `{kind:"roster",roomId,revision}` selects the complete
  [meeting-room roster](meeting-room-v1.md). New fan-out checks revision and exact
  effective `recipientIds` in the same transaction; mismatch is HTTP 409
  `ROOM_ROSTER_CHANGED` with no writes. The composer previews the exact audience.

Omitted/null room is unscoped explicit-identity delivery. Unknown/untagged modes
and direct context with multiple recipients are rejected. Mode and room are part
of the intent; retry cannot turn a private request into fan-out. Committed replay
returns its original receipt before checking potentially changed membership.

## Acceptance, failures and replay

```json
{
  "operationId": "11111111-1111-4111-8111-111111111111",
  "createdAtMs": 1700000000000,
  "items": [
    {
      "recipientId": "22222222-2222-4222-8222-222222222222",
      "requestId": "req_33333333-3333-4333-8333-333333333333",
      "acceptance": "queued"
    }
  ]
}
```

One operation receipt and all canonical request writes commit in one transaction.
Acceptance means a durable inbox entry, not that an agent has received, read or
completed work. For a newly accepted single-recipient request, including direct
room context, the host may attempt one advisory pane notification after commit.
The notification requires the recipient's currently active, explicitly bound
identity and verified server/socket, pane PID and identity marker. It contains
only the validated request ID and accepted recipient UUID, with an instruction
to use `tmt x show <requestId> --incoming --identity <recipientUuid> --json`;
the request body stays in the inbox. The explicit selector resolves an active
canonical UUID before normalized-name fallback. A retired UUID cannot restore
the retired identity, and `x show --incoming` checks exact recipient ownership
even when another active identity has a UUID-shaped name.
Announcements and roster sends remain queued without pane notification.
Receivers use the existing `x listen` and `x show --incoming` flow. Requests
additionally provide the `reply` flow, and `result <requestId>` reads the canonical final. An owner
composition has unknown sender identity rather than a
synthetic agent, while preserving the exact request text.

Announcements appear with incoming kind `announcement` and final status
`not_required`. Showing one provides its exact retained text but no reply receipt
or command. `result` returns terminal `not_required` with exit 0; even a correctly
correlated `reply` fails with `RESPONSE_NOT_REQUIRED` without changing attention.
Reading never acknowledges. The recipient uses the existing incoming ack/ackall;
acknowledged announcements are settled without manufacturing a response. Sender
and recipient acknowledgment remain independent. Schema 25 defaults all existing
attempts to `request` and constrains announcements to non-waiting inbox attempts.

An inactive or missing recipient is `recipientUnavailable`, with a failed request
ID and no incoming attention. It never resolves a replacement by name. Other
recipients can be queued in that same committed receipt. An unexpected storage
failure rolls back the whole transaction, including notification counters and
the receipt; it does not return fabricated per-recipient success.

Retry the same operation ID and exact message/audience after an uncertain HTTP
result. Reordered or duplicate selections mean the same audience. An accepted
operation returns its original receipt and request IDs without another enqueue.
A different message or audience under that ID returns HTTP 409
`DISPATCH_IDEMPOTENCY_CONFLICT`. A deliberate new request needs a new operation ID.

The immutable receipt records original acceptance only, never current request
status. Configuration changes, identity retirement, later replies and request
retention do not cause a replay to dispatch again. Schema 23 retains a compact
intent digest and receipt (at most 16 KiB), not another message body or response.
Receipts currently remain as replay tombstones after request expiry. No cleanup
may remove them without a policy that prevents old operation IDs creating work
again. Normal request retention remains owned by the existing settings/service.

The HTTP response may add an optional independent `wake` object for a newly
accepted single-recipient request: `{status,paneAttempted,agentProcessed}`.
`status` is `notAttempted`, `unknown`, `sent`, `unavailable` or `uncertain`;
`paneAttempted` is respectively false, null, true, false or true, and
`agentProcessed` is always null. These fields are advisory transport evidence,
not delivery, reading or processing evidence. A claimed wake survives process
loss as `unknown`; it is never automatically resent because pane input may have
occurred. Failure to wake never changes the queued request or its receipt.
Replay and `/dispatch/show` recover original acceptance only: they omit `wake`,
do not report the recorded notification outcome and make no new wake attempt.
Schema 32 stores the one-shot claim on the canonical request
attempt; it adds no second request or replay ledger.

Malformed JSON or dispatch values are HTTP 400 `DISPATCH_INVALID`. HTTP framing
errors, including a body above the endpoint budget, retain the shared HTTP 400
`BAD_REQUEST` response before dispatch admission. Invalid settings are HTTP 500
`CONFIG_ERROR`; repository or close failures are HTTP 500 `STORAGE_UNAVAILABLE`.
Close errors after commit are uncertain results: retry the same operation ID.
The composer distinguishes exact invalid-intent, membership and idempotency-conflict rejections
from uncertain outcomes. It stops offering replay after these definitive rejections,
retains the reviewed intent and requires explicit discard before a new composition.
A later rejection never proves that an earlier unconfirmed attempt was not queued.
The receipt contains no bearer, reply proof or host filesystem path.

## Retained conversations and acceptance recovery

Owner inspection uses three read-only POST operations with the same browser
bearer, exact loopback Origin and JSON admission. These are local-owner views,
not participant ACLs. They never acknowledge, enqueue or wake a pane.

- `/api/v1/local/requests/list`: `{recipientId?,roomId?,limit?,before?}` requires
  a recipient UUID, a room UUID, or both. Default limit is 20, maximum 50.
  `before` is `{preparedAtMs,requestId}` from the previous page. Scope is applied
  before newest-first keyset pagination; leaving a room does not hide old work.
- `/api/v1/local/requests/show`: `{requestId}` returns exact retained prompt and
  response text on demand. Request IDs use the native `req_<uuid>` form.
- `/api/v1/local/dispatch/show`: `{operationId}` returns the original immutable
  acceptance receipt, even after roster changes or request expiry. This recovers
  an uncertain acceptance without sending again. A missing receipt is not a
  replacement operation; retry still uses the original reviewed intent and ID.

Inspection input is capped at 4 KiB; the receipt lookup's strict envelope is
capped at 256 bytes. Unknown and duplicate fields are rejected before storage.
The list returns `{items,nextBefore}`. Items expose `requestId`, nullable `roomId`
and `recipientId`, sender `{kind,identityId}`, request `kind`, `preparedAtMs`,
`delivery`, nullable `recipientAcknowledged`, `final`, and a nullable `preview`
of at most 160 Unicode scalars. They include acknowledged requests and unknown-
sender Office requests, not only inbox attention. Pane delivery has null
acknowledgment because it cannot prove that an agent read the request.

Final states reuse the canonical request interpretation: `not_submitted`,
`not_required`, `retained`, `expired`, or `unavailable`. Retained metadata includes
`submittedAtMs`, `bodyBytes` and `expiresAtMs`; expired/unavailable include the two
timestamps. Detail replaces `preview` with a `prompt` tagged as retained (exact
`message`, `messageBytes`, `expiresAtMs`), expired (`expiresAtMs`), or unavailable.
A retained detail final additionally contains exact `response`. No reply proof,
attempt secret or pane/socket path is returned. An observation timeout does not
change durable delivery or final state.

Missing/expired request metadata is HTTP 404 `REQUEST_NOT_FOUND`; absent operation
receipts are HTTP 404 `DISPATCH_NOT_FOUND`. Invalid inspection values are HTTP 400
`REQUEST_HISTORY_INVALID`; storage failures retain `STORAGE_UNAVAILABLE`. Reads
apply the existing bounded request-retention cleanup. Acknowledgment, body expiry
and metadata expiry remain independent; reads never resurrect content.

Schema 27 adds only recipient, recipient/room and room history indexes to canonical
attempts. It introduces no message table. Lists fetch bounded previews and final
metadata, not full bodies. The browser transport caps list responses at 256 KiB,
detail at the worst-case escaped size of two 1 MiB bodies plus 8 KiB, and acceptance
receipts at 16 KiB. All share the existing abort/deadline owner.

The local direct conversation panel observes metadata every three seconds while
visible, pauses when closed/hidden, and stops after fifteen minutes or a list
failure until refresh or reopening. The chat window holds ten exchanges and
fetches exact bodies with at most two concurrent reads. Bodies are fetched on
first display, explicit refresh or summary changes; displaced bodies are released.
Queue acceptance, recipient acknowledgment and a retained reply are separate
states; observation never acknowledges or dispatches requests.

Direct chat uses message/reply bubbles and a single input. Send or Enter freezes
and submits the exact direct intent without a separate preview step; Shift+Enter
inserts a newline and composing an IME character does not send. Confirmed acceptance
clears the input. Multi-recipient and full-room composers keep audience review.
Opening an agent from a meeting retains that canonical room UUID for direct sends
and history filtering. The header names the room and the single recipient.

Before sending, the shared composer journals the exact pending direct intent in
tab session storage, keyed by recipient UUID and optional room UUID. Room-specific
pending intents cannot overwrite or replay another room's or an unscoped draft.
Failure to preserve it prevents the new send. Reopening
checks the original operation receipt without resending; an explicit retry uses
the same operation, recipient, room context and text. Confirmed acceptance removes the pending
record. Explicit discard removes only the local record, not accepted work.
The journal is not chat history and contains no bearer token. It survives reload
only in the same tab/origin; closing the tab or changing the service port can lose
unconfirmed intent, but accepted history remains in native storage. Reload needs
the authenticated URL printed by `office start`.

## Ownership and verification

Core `dispatch` owns admitted composition values and audience normalization.
Its adapter owns strict owner JSON and framed intent hashing. `storage::dispatch`
owns only the operation ledger and composition; `storage::requests` exposes a
transaction-borrowing repository so the existing `RequestService::enqueue` owns
all request transitions, attention, expiry and prompt storage. Neither opens a
nested transaction or duplicates request SQL. The loopback adapter owns authority,
settings and connection lifetime. No new dependency is required.

The CLI room sender shares this owner, selecting sender provenance through the
existing identity resolver. A known sender participates in intent hashing and
canonical request attention; unknown-owner intent bytes remain unchanged.
The public HTTP decoder always constructs an unknown sender and rejects an
`originator` member. This is not permission for extensions to impersonate agents.
The schema 23 `office_dispatch_operations` name and digest domain are retained;
there is no second CLI dispatch ledger.

Verify exact prompts, deduplication, uncertain-response replay, changed-intent
conflict, partial inactive recipients, rollback after the last write, concurrent
replay, migration rollback and ordinary inbox access. Browser integration must
also prove explicit preview/Send, draft retention and no duplicate request after
retry; lower-layer checks alone do not prove those UI behaviors.
