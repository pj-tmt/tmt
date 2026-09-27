# Request, response and exchange contracts

The native CLI stores complete replies in SQLite. `talk` waits for a durable
reply by default; `--timeout` bounds observation and `--detach` returns after
sending. Terminal capture and `check` are diagnostics, not authoritative
completion or full-body retrieval. Explicit `talk --inbox` adds a local durable
recipient route; it does not imply a provider hook, background daemon, remote
transport or authentication.

A cooperating agent submits its complete final body successfully, then may show
a short truthful summary of work, verification and unresolved items. Submission
means delivered, not successful work; summary failure cannot undo an accepted
reply. [Architecture](ARCHITECTURE.md) owns implementation boundaries and
[the user guide](USER-GUIDE.md) provides usage examples.

## Durable final submission

`RequestService::submit_response` and `get_response` share one storage owner.
Submission supplies a request ID, recorded or compact proof of the attempt and
six-field endpoint, and an exact body. A response record carries those values, its UTF-8
`bodyBytes`, and immutable `submittedAtMs`. The body limit is 1,048,576 bytes,
inclusive. Empty text, BOM, NUL, CR/LF and valid Unicode are preserved; malformed
Unicode and oversized input fail before mutation. File/stdin decoding is an
input-adapter responsibility, not an alternate response store.

One immediate transaction validates the request, attempt and full endpoint and
accepts only `sending`, `sent` or `uncertain`. A matching retained final is an
idempotent retry with its original timestamp; different content cannot overwrite
it. `prepared` and `definitely_failed` cannot submit. If a reply wins the race
against definitely-failed settlement, settlement remains conservatively uncertain
and cannot refund cadence. A local reply is not authentication or proof of an
external transport effect. No transaction spans external work.

The submission deadline is the later of attempt expiry and seven days after
preparation. Equality is expired. Wait release and the existing one-hour minimum
attempt expiry do not end that window. Cleanup preserves terminal attempt metadata
through both this deadline and the existing 24-hour settlement retention floor.
Final bodies have independent retention after submission, using the duration
frozen on their request (90 days by default for new requests); expired bodies
are hidden by reads and deleted by opportunistic cleanup. This is not a scheduled
physical-deletion SLA. Retained retries remain idempotent past submission expiry.

Migrated historical requests and bodies retain a concrete seven-day duration,
preserving submission timestamps;
new global policy never extends those old bodies. The global config's
`exchange.retentionDays` accepts integer days 1 through 3650, default 90.
`config set exchange.retentionDays <days> --global` changes future preparation
only; local override/clear is rejected. Reply/result use stored deadlines and
do not load current configuration. No permanent alternate legacy runtime path
or migration-time rebasing is introduced.

Initial metadata expiry protects the preparation content horizon, reply
acceptance deadline and attempt-expiry-plus-24-hour settlement floor. An actual
nonexpired settlement protects its own floor; first final submission extends
metadata through that body's expiry. Reads, housekeeping, waiter release and
identical retries do not renew retention. Metadata cannot be pruned ahead of a
retained final or still-eligible reply. Original prompts, provenance and
identity-scoped attention follow the contracts below.

Service-owned request/result reads apply logical expiry independently of physical
cleanup. Opportunistic cleanup uses deterministic limited batches in a short
transaction: up to 100 expired-attempt transitions, 100 prompt scrubs, 100 final deletions and 100
metadata deletions. Batch failure rolls back and propagates through the existing
error boundary. Repeated invocations drain backlog; no invocation means no
scheduled deletion. Deadlines are fixed UTC wall-clock values. Clock rollback
can delay logical expiry while content is physically present, but never changes
the stored deadline or restores deleted data. Cleanup is not secure erasure,
file shrinkage, acknowledgement, cancellation or a change to reply eligibility.

Migration 5 adds independent `request_responses` rows, with complete endpoint
snapshots and no cascading foreign keys to attempts or identities. It also adds
`response_submitted_at_ms` to attempts, committed atomically with the final body.
That bounded completion marker prevents recreation or false cadence refunds if a
long-lived attempt outlasts its body's retention. It is not a second result body or
an unbounded tombstone store. After all retained metadata is physically removed,
an unknown request cannot be distinguished from a previously expired one.

Typed response errors distinguish invalid/oversized input, unknown request, wrong
attempt, wrong recipient, ineligible state, expiry and conflicting content.
Rejected submissions preserve attempts, cadence and responses. Storage failures
remain storage failures. There is no cancellation operation or automatic retry routing policy.

## Reply and result commands

```text
tmt reply <request-id> --receipt <receipt> (--message <text> | --file <path> | --stdin) [--json]
tmt result <request-id> [--json]
```

Native talk emits compact v2 receipts; bounded v1 input remains supported as
defined below. Receipts are correlation, not authentication. The recipient must use the supplied receipt, not infer
the latest request from a pane. `talk` supplies it in the recipient instruction,
including detached requests. No receipt is included in routine result/ack output.

Files must resolve to regular files; symlinks are followed, and descriptors are
closed after bounded reads. Explicit stdin requires EOF within five seconds.
Both reject malformed UTF-8 and bodies beyond 1,048,576 bytes before submission,
preserving empty text, BOM, NUL, CR/LF and whitespace without normalization.

Inline `--message` passes its exact string to the same service validation.
Exactly one source is required; an explicit empty inline string is valid.
Shell quoting and OS argv size limits apply, and argv cannot contain NUL.
Use file/stdin for larger or NUL-containing bodies.

Requests group one reply command in `<tmt-reply from="…">` tags, including the
receipt once. The XML-escaped originator name is presentation, not authentication.
Brief submission/summary/error guidance stays outside. This is request
grouping, not a hidden UI promise or terminal-output boundary. HTML comments
would conflict with ASCII `!` protection, which remains unchanged. Detailed
input and retry rules remain in installed skills/help instead of every request.

Reply success returns `status: submitted`, `requestId`, `bodyBytes` and
`submittedAtMs`. Identical retries preserve the timestamp; different bodies
cannot overwrite a final. Result success returns `status: completed`, `requestId`,
exact `response`, `bodyBytes` and `submittedAtMs`. JSON is the exact-text interface;
human output adds formatting. A missing retained result is `status: unavailable`
with `RESPONSE_NOT_AVAILABLE` (exit 3), not a claim that a request is unknown,
cancelled or completed. Input deadline uses exit 4; conflicting final uses exit 5.

Storage-only commands work after pane closure and waiter exit. They reuse the
service and its retention, not another reply store. Agent guidance requires a
truthful short user summary only after successful submission. Submission means
the result was delivered, not that the requested task succeeded. Summary failure
does not undo or justify repeating an accepted final.

## Compact receipts

Encoded receipts are bounded to 8192 characters. V1 is decode-only; v2 is the
native emission format.

Native talk uses the shared codec to emit exactly 25 ASCII characters: `v2_` plus
canonical unpadded base64url of the first 16 SHA-256 digest bytes. The preimage
starts with ASCII `tmux-team/reply-receipt/v2` and NUL, followed in order by
request ID, attempt ID, server ID, socket path, server PID, server start time,
pane ID and pane PID. Each string is exact UTF-8 prefixed with its unsigned
64-bit big-endian byte length; PIDs are unsigned 64-bit big-endian integers.
Independent goldens freeze this protocol, including Unicode byte lengths and
ambiguous concatenation boundaries. Padding, nonzero trailing bits, unsupported
versions and partial tokens are rejected.

The digest binds the whole recorded association without storing another token. Its 128-bit output space is not a promise of
128-bit collision resistance, secrecy or authorization. Predictable inputs stay
predictable; native callers must generate independent random request/attempt IDs.
This local correlation format is not a remote MCP access credential.

The existing `submit_response` takes one `SubmitResponse` with a recorded or
compact `ResponseProof`. Within its existing immediate transaction it looks up
the explicit request ID, prefers a retained final, and validates the proof using
that record's attempt/endpoint. It never scans by token, looks up a current pane
or performs a preliminary housekeeping read. Identifier uniqueness still fails
atomically; there is no token registry, retry loop or collision-resolution table.
Wrong compact proofs return `RESPONSE_RECEIPT_MISMATCH`; unknown requests retain
`RESPONSE_REQUEST_NOT_FOUND`. Rejections mutate nothing. Both proof modes use the
same body, eligibility, immutable-final, retention and attention implementation.

Native v1 decoding is compatibility input only, not another emitter or service.
It preserves bounded strict UTF-8, the exact old object shape, safe positive PIDs,
canonical base64url spelling, positional request matching, and JSON whitespace,
key-order and duplicate-key-last-value behavior. Retained finals remain sufficient
for retry after attempt removal; expiry and conflicts never renew them. Tests
execute original TS-generated v1 receipts against migrated frozen schema-8
fixtures, including in-flight and orphan finals. This proves service handoff,
not simultaneous TS/schema-8 access to current native state.

Both receipt versions work without tmux or current config. Old schema-8 inputs
may migrate through native reply; simultaneous old writers after native
migration are unsupported.

## Talk completion

`talk <target> <message> [--timeout <time> | --detach] [--json]` waits for the
shared service's complete final by default. It no longer captures or cleans
terminal output to determine completion. `send` follows the same semantics.
The recipient must cooperate by invoking reply; idle output, fake markers,
process exit and human summaries do not complete a request.

Timeout defaults to 180 seconds unless configured, accepts finite positive
seconds or ms/s suffixes, and is bounded to 24 hours. Explicit timeout and
detach are mutually exclusive. The monotonic deadline starts immediately before
beginSend/transport, after delay/preparation/receipt encoding. Checks before and
after each synchronous response read treat equality or crossing as timeout;
there is no final post-deadline read. Poll sleeps are bounded by remaining time.
Synchronous transport/Enter time counts, but cannot be cancelled mid-operation.
Timeout/interruption only releases the observer; late results remain retrievable.

Detached JSON is `{status:"sent",requestId,target,pane,identity?}`. Completed
talk returns `status:"completed"`, the same correlation and exact `response`,
`bodyBytes`, `submittedAtMs`. Timeout uses `status:"timeout"`, request/target/pane
correlation and `error:{code:"TIMEOUT",message}` (exit 4), without partialResponse,
nonce, endMarker or truncated. Delivery/state uncertainty remains nonzero and
retains inspection correlation, never automatic resend. Failure during receipt
construction before beginSend refunds a proven-unsent reservation.

`--wait` is rejected; talk rejects `--lines` while check
retains it. Stored mode/maxCaptureLines values are inert, not automatically
rewritten; explicit local `config clear mode` deletes only that obsolete key.
Historical migrations/nonce columns stay unchanged; new attempts omit nonce.

The Docker peer submits through the real public reply CLI, logs causal
request/submitted/summary or failure events, and retains a full-body oracle.
Virtualized output exposes only its tail; acceptance requires exact complete
talk/result equality, not a missing-interior characterization. Same-pane input
serialization, exactly-once processing, inbox and remote authentication are
still outside scope. User-installed skills teach full submission first, then
a truthful work/tests/blockers summary; failed submission is never success.

## TMT Exchange and attention

The same identity commands work inside and outside tmux, without implicit
selection or automatic binding. Explicit talk attribution may use an existing
identity even while another pane is bound to it; this is not authentication.

X is a logical collaboration record that relates an originator's request, its
delivery attempts, the recipient's one immutable final reply, and per-identity
attention state. It uses the existing `RequestService` and its narrow records port over one
invocation-owned SQLite connection. It does not
mandate a second `XService`, a second database, a new table name, or a new ID
format: existing request IDs and receipt/attempt fencing remain reusable.
Attention revisions/high-water marks are not response-body revisions. A final
reply remains one immutable body; an identical retry remains idempotent and a
conflicting body remains a conflict. A late accepted reply can create the final
body or reopen an unacknowledged attention revision; it never overwrites an
existing final body.

Migration 7 adds independent originator selection kind (unknown, explicit or
verified), optional originator UUID and recipient UUID. Existing `identity_id`
remains target preamble/cadence state, while public `talk.identity` remains
recipient presentation. Historical rows keep unknown provenance and NULL
original context; no pane, cadence, name, receipt or old-file backfill occurs.

New preparation requires exact original message text, before preamble, receipt
instruction or `!` protection. The existing service validates well-formed Unicode
and a 1,048,576 UTF-8 byte inclusive limit through a primitive shared with response
validation. Empty, BOM, CR/LF, NUL and Unicode are preserved, without role/preamble
normalization. Request wrappers retain `REQUEST_INPUT_INVALID` and
`REQUEST_INPUT_TOO_LARGE` (exit 1). CLI argument limits still apply; no talk
file/stdin option is added. Validation follows config/timing checks and precedes
target effects, then originator selection, cadence and transport.

`talk <target> <message> --identity <existing-name>` (and `send`) selects an
existing originator identity even when offline, overriding implicit caller
selection. The option is command-local, not a global flag or recipient selector.
Omission records a verified caller when present, otherwise unknown; anonymous
talk/result still work. Unknown explicit selection is NAME_NOT_FOUND (exit 3);
ambiguous or reconciliation failures stop before sending (exit 1).
The internal target projection retains the independently verified recipient UUID
even with no preamble, and rechecks identity/binding markers and full server/pane
evidence against the fresh pre-preparation snapshot. A changed observation rejects
before persistence/cadence/send; later rebinding never rewrites recorded IDs.
This does not authenticate authorship or guarantee which identity later processes input.

Original messages are retained locally, always on for new preparation; avoid secrets. There is no upload, indexing, redaction, encryption or
secure-erasure claim. Their fixed expiry is preparation plus the frozen policy,
not the extendable metadata horizon. Final submission cannot renew the prompt.
General metadata/list queries exclude prompt text/bytes. A focused internal
request-context read returns one retained attempt and a prompt
status: retained with exact message, byte count and expiry; expired with expiry;
or unavailable for historical content. Unknown/expired metadata returns no
record. Equality is expired even outside the bounded physical scrub batch.
The shared cleanup transaction scrubs at most 100 ordered indexed expired
prompts, retaining their expiry markers. It neither renews nor acknowledges.
`x show` reuses that projection for identity-scoped public context.

`x` commands select the originator's data with
`--identity <name>` through the shared durable selector. Omission requires a
verified caller identity; outside that context it fails rather than guessing.
Selection is local attribution, not authentication. This required
identity for `x` does not change talk's deliberate anonymous-caller exception.

The attention contract is identity-scoped and explicit:

- A read, including `result`, `x show`, or an unacknowledged listing, never
  mutates attention state or acknowledges a revision. Service-owned reads may
  perform logical-expiry checks and bounded housekeeping; `check` remains a
  diagnostic command whose existing reconciliation behavior is separate.
- The default unacknowledged view is scoped to the originator. The
  recipient's responsibility to submit a result is distinct from the
  originator's responsibility to acknowledge it; a recipient view must not be
  inferred from the current result command.
- `tmt x ack <request-id> --revision <revision>` acknowledges the exact observed
  attention revision for the selected identity. If the X is still pending,
  this acknowledges only the progress observed at that point; it does not
  claim a successful task or cancel delivery.
- A reply that arrives after that acknowledgement creates a newer attention
  revision and reopens the X for that identity. A single ack must use a
  revision/high-water compare-and-swap so it cannot acknowledge a newer result
  that was committed after the caller's observation.
- `tmt x ackall` takes one bounded, atomic snapshot of the
  selected identity's eligible revisions. Results committed concurrently after
  that snapshot remain unacknowledged and cannot be hidden by the batch.
- An X is settled only when it has a final reply and its current attention
  revision is explicitly acknowledged. A proven failed delivery may be
  acknowledged as an exception for attention management, but that is not
  successful work or a delivered result.

The command surface is `tmt x list`, `tmt x show <request-id>`,
`tmt x ack <request-id> --revision <revision>`, and `tmt x ackall`.
`tmt x` is equivalent to `tmt x list`. All accept command-local `--identity`
and `--json`; only list accepts `--limit` and `--after`, and only ack accepts
the mandatory `--revision`. Old `ack --all` and batch tokens are rejected.

Migration 8 assigns deterministic initial revisions to already known v7 originators
ordered by preparation time and request ID. Unknown/anonymous provenance stays
outside identity attention. Per-originator counters survive request cleanup;
revision exhaustion rolls back the whole request/final write. Preparation and
the first accepted final are the only revision events. Delivery transitions,
waiter release, cleanup, reads and identical retries do not advance them.

Ackall needs no previous list: inside the existing immediate transaction it
advances one identity's acknowledged-through watermark to its latest revision.
It neither enumerates nor counts nor updates individual requests. A writer
committed before that snapshot is included; a later writer allocates a higher
revision. Each invocation takes a new snapshot. Single ack compares the supplied
revision inside the transaction and never advances the identity watermark.

List returns `{identity,items,nextAfter}` with at most 50 items by default
(`--limit` 1..200). `--after` is a nonnegative decimal safe integer (default 0).
Rows are ordered by increasing revision and only unacknowledged retained metadata
is returned. One extra metadata-only row detects continuation: `nextAfter` is
the last returned revision when more exist, otherwise null. This is a live view;
deduplicate by request ID and restart at 0 to refresh. No body is loaded by list.

Summary fields are `requestId`, nullable `recipientIdentityId`, `preparedAtMs`,
`delivery`, `final`, `revision`, `acknowledged`, `settled`, `retentionExpiresAtMs`.
Final is `not_submitted`, `retained` (submission time, bytes and expiry), `expired`
(submission time and expiry), or `unavailable` (marker exists but eligible body
is absent). Missing final is not evidence that a task is running. Show returns
`{identity,exchange}` and adds `prompt` plus exact `final.response` when retained.
Attempt IDs, receipts and endpoint evidence are not exposed.

Ack returns `{identity,requestId,revision,acknowledged:true,changed}`; a repeat at
the current already-effective revision has changed:false. Ackall returns
`{identity,acknowledgedThrough}`, not a count or a claim that every body was read.
Stale revision is `X_REVISION_CONFLICT` (exit 5). Unknown, anonymous, wrong-owner
and metadata-expired IDs share `X_NOT_FOUND` (exit 3). Invalid runtime parameters
use `X_INPUT_INVALID` (exit 1); revision overflow uses `X_REVISION_EXHAUSTED` (1).
Unexpected failures use sanitized `X_ERROR` (1); shared identity errors remain.

`talk`, `reply`, and `result` remain the verbs for sending, submitting, and
reading. `check` remains a pane diagnostic only. Timeout and interruption
remain observer-only: they do not cancel or complete X, and the existing
180-second default remains current behavior. Explicit `talk --inbox` queues for
one existing non-retired local identity, and bounded `x listen` observes local
recipient or result attention. This adds no lease, background daemon, memory
feature, MCP state machine, remote transport or Office dependency.

The request service owns frozen policy, metadata/final horizons, prompt
validation and bounded housekeeping. Attention consumes the same retained state. Do not add a
parallel cleanup subsystem. SQLite has no autonomous TTL scheduler, so no daemon,
cron, network service or punctual physical-deletion promise is introduced.

Acknowledgement is not deletion, and unread records are not retained
indefinitely. An expired result is unavailable, not pending. A hard metadata
horizon eventually makes an old X indistinguishable from unknown, honestly and
without a permanent tombstone guarantee. Eligibility and fencing metadata must
not be deleted before their acceptance/late-reply obligations end, and an
expired final must never be resurrected. Data deletion is not database-file
shrinkage or secure erasure: do not run `VACUUM` on every command or treat
`auto_vacuum` as a TTL mechanism ([SQLite `auto_vacuum`](https://sqlite.org/pragma.html#pragma_auto_vacuum),
[SQLite serverless operation](https://www.sqlite.org/zeroconf.html)).

The implemented interpretation of the default duration is exactly 90 days,
using the existing global config file:

```json
{
  "exchange": {
    "retentionDays": 90
  }
}
```

This applies to newly prepared requests only. Older migrated records retain
seven days, without resurrecting expired or deleted bodies. Request content
uses preparation time; final content uses submission time; metadata protects
both plus settlement/acceptance obligations. A late final can therefore keep
metadata longer than 90 days from creation. Ack and reads do not renew
retention. The 180-second `talk` observer timeout and the unchanged reply
acceptance window remain separate lifetimes.

The following semantic scenarios are illustrative only, not final JSON schemas:

| Operation                                       | Observable semantic outcome                                                                                                         |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `talk` succeeds and the final reply is accepted | Delivery/result succeeds, but the originator's X view remains unacknowledged until explicit ack.                                    |
| `talk` times out, then ack, then a late reply   | Timeout releases only the observer; ack covers the current revision; the late reply creates a newer revision and reopens attention. |
| `result` after the retention boundary           | The result is unavailable/expired, not pending; opportunistic cleanup may later remove eligible physical rows.                      |

## Verification ownership

Exact body, retained retry, revision races and expiry belong to the request
service and storage tests. [Native response tests](typescript/test/native/response.test.ts)
cover public receipt/input contracts; [Docker response integrity](typescript/test/e2e/response-integrity.e2e.test.ts)
checks complete bodies against independent mock-agent events even when the
terminal renders only a tail. Terminal echo is not a completion oracle.
See [Development](DEVELOPMENT.md) for commands and scenario ownership.

The [historical research record](https://github.com/wkh237/tmt/blob/5b1e9beb6d7deeae955eba3b49ab78bd07c9df1d/REQUEST-RESPONSE.md)
retains superseded marker/JSON-state behavior and provider research. It does not
define current commands or authorize new integrations.
