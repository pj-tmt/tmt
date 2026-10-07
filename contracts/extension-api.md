# Local extension API

Use the invoking executable supplied by extension dispatch:

```sh
printf '%s\n' '{"version":1,"operation":"capabilities","input":{}}' |
  "$TMT_EXECUTABLE" api
```

No Office installation is required. Capabilities list supported operations and
byte bounds without creating state. Close stdin within five seconds. Exit 0
means the operation returned a resource; exit 1 returns
`{"error":{"code":"...","message":"..."}}`. This is a local same-user
interface, not remote authorization. The [architecture contract](../ARCHITECTURE.md#local-extension-api-v1)
owns compatibility, transport and persistence rules.

Official executable installation is a local CLI operation (`tmt extension
install|upgrade|rm`), separate from this process API and its `skills.install`
operation. Installer registration may precede a product's first published
archive: `tmt extension ls` reports registered products, while install requires a
verified release. Remote and Colab use that shared installer; installation does
not start either extension, pair a device, grant remote operation authority or
access their private `<dataRoot>/remote/` and `<dataRoot>/colab/` state. Colab's
settled package contract embeds its app in the executable, with no separate
installer data tree; build-time embedding and publication remain separate owner
gates. The [native installation
contract](../ARCHITECTURE.md#managed-skills-and-native-installation) owns archive,
receipt, consent and unavailable-release behavior.

Every request has `version`, `operation` and `input`. Writes (`dispatch.create`,
`rooms.write`, `rooms.retire`) additionally name exactly one originator: `identity`, an active
identity UUID or name, or `"originator":"anonymous"`, which stores no writer identity,
exactly like the CLI without `--identity`. Anonymous is not an authenticated owner and
grants nothing beyond what same-user CLI calls without an identity can already do; both
or neither is `API_INPUT_INVALID`. Operations without originator attribution, including
`identity.meta.apply`, name neither. The metadata target is its input `identityId`. Unknown request fields
are rejected. Responses reuse existing resource shapes, without a second wrapper.
Clients must tolerate additive response fields. Public `tmt api` storage opens,
including `notes.read`, return `STORAGE_NOT_WRITABLE` (exit 1) only for a confirmed
OS-denied or read-only data directory. The message names that directory; no API
operation was performed. Other storage-open failures retain `API_UNAVAILABLE`
or the resource's existing code. `capabilities` and `storage.root` remain
independent of storage access.

| Operation                | Input                                                                                              | Result                                                                                                                                                            |
| ------------------------ | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capabilities`           | `{}`                                                                                               | Protocol range, operations, byte limits and ordinary commands                                                                                                     |
| `storage.root`           | `{}`                                                                                               | `dataRoot`: absolute selected TMT data directory; no directory creation or storage/config reads                                                                   |
| `changes.cursor`         | `{}`                                                                                               | `cursor`: an opaque non-negative integer that changes whenever core's durable records change (see below)                                                          |
| `requests.list`          | `recipientId` and/or `roomId`, or `originatorId` + `view:"results"`; optional `limit` and `before` | `items`, `nextBefore`                                                                                                                                             |
| `requests.show`          | `requestId`                                                                                        | Request detail including retained prompt/final state                                                                                                              |
| `dispatch.show`          | `operationId`                                                                                      | Immutable acceptance receipt                                                                                                                                      |
| `dispatch.create`        | `operationId`, `recipientIds`, `message`, optional `kind`, `room`                                  | Acceptance receipt; optional independent `wake` on first direct request                                                                                           |
| `rooms.write`            | `roomId`, `room: {expectedRevision, name, memberIds}`                                              | Room resource                                                                                                                                                     |
| `rooms.retire`           | `roomId`, `expectedRevision` (positive)                                                            | Room resource (same shape as `rooms.write`); a repeat at the pre- or post-retirement revision is idempotent, any other stale revision is `ROOM_REVISION_CONFLICT` |
| `rooms.roster`           | `room` (UUID or unique exact name), optional `metadataPrefix`                                      | `room` resource and `members` with metadata and status                                                                                                            |
| `notes.read`             | `identityId`                                                                                       | Saved identity's `identityId`, `name`, `content`                                                                                                                  |
| `identityHooks.register` | `consumer`, `identityId`, `reference`                                                              | `state`: `registered`, `pending` or `delivered`                                                                                                                   |
| `identityHooks.pending`  | `consumer`, `limit` (1–16)                                                                         | This consumer's `hooks` (`identityId`, `reference`, `attemptCount`) and `pending` count                                                                           |
| `identityHooks.attempt`  | `consumer`, `identityId`, `reference`                                                              | `recorded`                                                                                                                                                        |
| `identityHooks.ack`      | `consumer`, `identityId`, `reference`                                                              | `acknowledged`                                                                                                                                                    |
| `skills.install`         | `owner`, `consent: true`, `skills`, optional `force`                                               | `owner`, `published` targets                                                                                                                                      |
| `skills.remove`          | `owner`, `consent: true`, optional `skills` (names)                                                | `owner`, `removed` and `kept` targets                                                                                                                             |
| `references.resolve`     | optional `identityIds`, `roomIds` (canonical UUIDs, at most 256 in total)                          | `identities` (`id`, `found`, `name`, `lifetime`, `retired`) and `rooms` (`id`, `found`, `retired`)                                                                |
| `consumption.history`    | `identityIds` (1–32 UUIDs), `windowsMs` (1–3), optional `maxBuckets`                               | Closed timestamped deltas, coverage and included cumulative seed watermark (see below)                                                                            |
| `identity.meta.apply`    | `identityId` (active canonical UUID), `changes`                                                    | `identityId`, `changed`; atomic conditional metadata update (see below)                                                                                           |
| `identities.status`      | `identityIds` (canonical UUIDs, at most 256)                                                       | `identities`: `{id, found}` and, when found, `status`: the `tmt identity status` value or `null`                                                                  |
| `extensions.uses`        | `extension`, `feature`                                                                             | `available`, `installed`, `reason`, `hint` for a declared optional use (see [Optional cross-extension uses](#optional-cross-extension-uses))                      |

`storage.root` reports the data directory selected by the invoking core, including
its normal explicit-home/XDG/legacy selection. Extensions MUST use this operation
rather than infer configuration paths, and keep their files under
`<dataRoot>/<extension>/`. The extension owns creation and lifecycle of its
subtree, with 0700 directories and 0600 secret/state files; it MUST NOT open or
modify core's database/configuration or provider settings. Discovery does not
create the root or read its files. `capabilities` remains a constant document.

## Conditional identity metadata

`identity.meta.apply` updates descriptive, untrusted metadata for one exact active
identity UUID. It grants no ownership, availability or prompt authority and does
not promote temporary identities. An unknown or retired UUID is `NAME_NOT_FOUND`;
it never falls back to a display name or a same-name successor. No originator
fields are accepted for this operation.

```json
{
  "version": 1,
  "operation": "identity.meta.apply",
  "input": {
    "identityId": "11111111-1111-4111-8111-111111111111",
    "changes": [
      { "key": "pending", "expect": { "value": "review" }, "then": "remove" },
      { "key": "state", "expect": "absent", "then": { "set": "working" } }
    ]
  }
}
```

`changes` contains 1–64 entries with distinct keys and no unknown fields. Each
entry has exactly `key`, `expect` and `then`. Expectations are `{"value":"exact"}`
(exact string equality, including whitespace), `"absent"` (no row), or `"any"`
(present or absent). Actions are `{"set":"value"}` or `"remove"`; removing an
absent key is a no-op. Object forms keep literal values such as `"absent"`,
`"any"` and `"remove"` unambiguous. Keys use `[a-z][a-z0-9_.-]*`, 1–64 ASCII
bytes. Exact expectations and set values use the existing 1–1024 UTF-8 byte,
nonempty, no-control-character metadata validation. The changes JSON is bounded
by 815,104 bytes. Invalid API fields or changes return `API_INPUT_INVALID`.

One SQLite writer transaction rechecks active UUID membership, checks every
expectation against the same current rows, and only then applies all effects.
Any mismatch writes nothing and returns exit **5** with:

```json
{
  "error": {
    "code": "METADATA_CONFLICT",
    "message": "Metadata expectations did not match current values; no changes were applied.",
    "current": { "pending": null, "state": "blocked" }
  }
}
```

`current` contains every conflicting key, with its current string or `null` when
absent; matching keys are omitted. It is the transaction's conflict snapshot,
not a promise that another writer cannot subsequently change it. A client must
refresh its preview and obtain a fresh submit before overwriting intervening
values. Competing writers with the same exact expectations serialize: one
applies, the other conflicts. `"any"` deliberately permits unconditional writes.
The final metadata set must contain at most 64 entries; a removal and addition
can exchange an entry regardless of list order. An entry-limit refusal is
`IDENTITY_METADATA_INVALID` and writes nothing. A later storage failure rolls
back all effects. Success is `{identityId, changed}`, where `changed` is false
when every action is already satisfied. No-ops and conflicts leave the durable
change cursor unchanged. Existing metadata `set` and `rm` remain unconditional.

The CLI uses the identical changes schema on stdin:
`tmt identity meta apply --identity <name-or-uuid> --json < changes.json`.
Without `--identity`, it uses the existing verified-caller selector. UUID
selection is exact, without display-name fallback. Stdin must be non-terminal
and close within five seconds; invalid input is `IDENTITY_METADATA_INVALID`.
The CLI shares the success/conflict documents and conflict exit 5 with the API.
This mutation sends no notification; callers own any subsequent notification and
must not repeat a confirmed mutation to retry it.

## Consumption history

`consumption.history` v1 is a storage-only batch read. It accepts 1–32 distinct
canonical `identityIds`, 1–3 distinct `windowsMs` (multiples of 5,000 from 5,000
through 3,600,000), and `maxBuckets` (1–120, default 120). Other values or fields
return `API_INPUT_INVALID`. Batch larger rosters. The normative shared vector is
[consumption-history-v1.json](consumption-history-v1.json); it includes an open
sample beyond the returned seed, so clients can verify the first live delta.

```json
{
  "version": 1,
  "operation": "consumption.history",
  "input": {
    "identityIds": ["11111111-1111-4111-8111-111111111111"],
    "windowsMs": [60000, 300000, 3600000],
    "maxBuckets": 120
  }
}
```

The result is one SQLite read snapshot. `asOfMs` is core's UTC Unix millisecond
clock at admission; `throughMs` is that time rounded down to 5 seconds.
`resolutionMs` is 5,000 and `retainedFromMs` is `throughMs - 7,200,000`, clamped
to zero. Only closed buckets in `[retainedFromMs, throughMs)` are included.
The open bucket is excluded. Counter `observedAtMs` records acceptance of new
provider evidence and does not advance on a reread; `lastSampleAtMs` records an
accepted source read, including an unchanged counter. Neither grants a heartbeat
or permission to send input. Clock rollback fails closed rather than inventing
an interval.

`identities` preserves request order. Unknown or retired UUIDs return
`{id,found:false}`. A found identity additionally returns `reporting`,
`availableFromMs`, `lastSampleAtMs`, `latest` and `windows`:

- `reporting` means retained closed history contains a successful normalized
  counter observation. False is unavailable evidence, not a measured zero.
- `availableFromMs` is the earliest successful observation time in retained
  closed history, or null. It is not a promise of continuous coverage.
- `latest` is `{driver,session,consumption}` from the last accepted source read
  represented in retained closed history. `consumption` has the public
  `resume.consumption` shape: cumulative `inputTokens`, `outputTokens`,
  `cachedInputTokens`, `epoch`, `sequence`, `observedAtMs`, `complete`, `gap`.
  A failed last read makes `latest` and `lastSampleAtMs` null. This watermark
  excludes newer observations in the open bucket, even when ordinary `ls`
  already exposes them. It is never substituted with a newer live counter.
- Each window is `{windowMs,fromMs,toMs,bucketMs,buckets}` with
  `toMs=throughMs` and `fromMs=max(0,throughMs-windowMs)`. `bucketMs` is
  `ceil((windowMs/5000)/maxBuckets)*5000`; aggregation starts at `fromMs` and
  the final bucket may be shorter. Each window returns at most `maxBuckets`
  buckets, including uncovered intervals.
- Each bucket is `{fromMs,toMs,inputTokens,outputTokens,cachedInputTokens,
coveredMs,complete,gap,discontinuous}`. Token fields are normalized **deltas**,
  attributed to the accepted observation's base bucket, never counters or
  prorated estimates. Cached input is already part of input; total is input
  plus output. `coveredMs` is the union of verified contiguous successful read
  intervals in that bucket, bounded by its duration. Reads more than two sampling
  cadences apart do not establish coverage between them. An incomplete source
  may contribute a known lower-bound delta without establishing coverage.
  `complete` requires full duration coverage and complete evidence without a
  gap; `gap` marks missing evidence/coverage, and `discontinuous` marks an
  observed driver, session or epoch boundary. An unavailable read, decreasing
  counter or boundary never bridges unknown counters. A gap observation sets
  the new baseline; a later same-epoch gap-free read may contribute a known
  delta from it without claiming coverage across the gap. Zero tokens with full
  coverage is a measured zero; zero tokens without coverage is unknown.

Core retains 5-second buckets for two hours: at most 1,440 closed buckets plus
one open bucket per identity. Expired history is excluded immediately on reads;
accepted writes prune the identity and opportunistically prune expired inactive
rows. Reads never renew history. Migration does not backfill history and core
never replays old transcript prefixes. Provider formats are unofficial; missing
records, bounded reads and source loss can leave partial history. Counter
arithmetic is checked against JavaScript's safe integer bound.

Closed buckets can gain coverage when a later accepted read verifies an interval;
responses are snapshots, not an append-only delta feed. Use the included
`latest` driver/session/epoch/sequence and cumulative counters as the live seed
watermark. `throughMs` alone cannot identify which counters were included. The
shared fixture demonstrates an open read completing a closed bucket's coverage
without changing that bucket's included cumulative counter.

A meter seeds its existing Rate owner from the **longest returned window once**;
shorter windows overlap and must not be added again. It uses the included
`latest` snapshot as its live `ls` baseline. For example, the shared fixture's
closed seed includes 14 input and 7 output tokens and `latest` is 114/57;
the newer live 120/60 counter adds exactly 6/3, not the full cumulative total.
An epoch/session/driver change, null seed, gap or decreasing counter requires a
new baseline. On reentry, rebuild the covered recent range and preserve older
observed intervals in that same owner. Do not add core deltas and live deltas
for overlapping intervals. If overlap or evidence cannot be resolved exactly,
retain the known total as partial; do not prorate or double count. A configured
window longer than the query/coverage cap can use older observations in that
owner and otherwise remains partial (`~`); it is unavailable only with no
coverage. Core does not promise 24-hour history.

Collection runs every five seconds in the `tmt run`/`resume` foreground child
wait, using a bounded supervised worker and the existing consented usage hooks.
There are no listing-time provider reads or detached sampling services. Old
wrappers and hook-only launches remain Stop-only until relaunched through the
new foreground owner. `--no-usage` suppresses sampling and Stop collection;
legacy lifecycle-only installations remain disabled until explicitly enabled.
Exactly-once accounting covers sampling/Stop races and supported contiguous
provider groups; Claude's existing noncontiguous repeated-message limitation
remains. No new per-request deduplication store is introduced. A cursor outside
the latest 1 MiB is rebaselined at EOF with a gap rather than scanning the old
prefix. Extensions never access private source locators, provider files or
opaque driver state.

## Dispatch readiness and input safety

This section describes shipped local dispatch behavior and its limits. It does not
add a readiness operation or authorize remote callers; Remote's proposed grants
and operation admission belong to the [remote channel contract](remote-channel-v1.md).
The [request contract](request-response-v1.md) owns request lifecycle and durable
replies. The native [Claude](claude-channel-v1.md) and [Codex](codex-channel-v1.md)
contracts own channel readiness, receipts and recovery.

IDs are canonical UUIDs, except request IDs, which use TMT's `req_...` format.
`dispatch.create.kind` defaults to `request`; `announcement` does not expect a
reply. Optional room scope is `{kind:"direct",roomId}` or
`{kind:"roster",roomId,revision}`. Roster dispatch checks exact membership and
revision. Persist a new operation UUID before sending; retries must retain that
UUID and the exact normalized intent and originator. Changed intent conflicts.
An acceptance receipt is not proof of delivery or processing. An absent `wake`
on replay is intentional; an offline request remains queued without automatic
re-wake. Recover with `dispatch.show` after uncertain process completion.

### Admission, wake and completion

`dispatch.create` commits immutable acceptance before attempting an advisory wake
for a newly created, queued, single-recipient request outside roster dispatch.
The wake carries an optional bounded preview and a command to retrieve the
retained request. The notice grammar is owned by
[request-response-v1.md](request-response-v1.md); the full body is in the inbox.
Failure to wake does not undo acceptance. A queued request is durable inbox work;
it does not promise a live transport, a claimed wake or agent processing.

The optional `wake` is separate from the stored receipt. Its existing JSON shape
is `{status,paneAttempted,agentProcessed}`:

| `status`       | `paneAttempted` | Meaning                                                              |
| -------------- | --------------- | -------------------------------------------------------------------- |
| `notAttempted` | `false`         | No wake attempt reported.                                            |
| `unknown`      | `null`          | Claim or settlement could not be confirmed; input may have happened. |
| `sent`         | `true`          | Transport submission or native queue acceptance reported.            |
| `unavailable`  | `false`         | Wake could not obtain an accepted delivery.                          |
| `uncertain`    | `true`          | Delivery may have occurred without confirmed acceptance.             |

`agentProcessed` is always `null`: even `sent` proves no processing or reply.
`paneAttempted` is the existing coarse transport report; `true` does not imply
paste, and `false` is not permission to retry. Claude's successful one-way write
is unacknowledged and maps to `uncertain`; Codex's correlated queue acceptance
maps to `sent`. Neither completes the request.

Core claims a wake before input. A retained claim after an interrupted caller or
failed settlement is not an unattempted wake and MUST NOT be replayed. Neither
`dispatch.show` nor replaying `dispatch.create` attempts another wake or changes
the stored acceptance receipt; both omit `wake`. Observe request completion
through `requests.show` or the ordinary `tmt result --json` command.

A timeout is not a cancel. If process completion or output is uncertain, keep the
operation ID, normalized intent and returned request IDs; recover the original
acceptance with `dispatch.show`. A missing receipt while the original invocation
may still run is not proof that nothing was accepted. Stop/confirm cleanup of the
owned invocation before treating a definitive `DISPATCH_NOT_FOUND` as absence;
any retry retains the same operation ID and intent. Never manufacture a new ID,
resend a claimed wake or paste around uncertainty. Remote's durable journal and
authority revalidation remain the remote channel contract's responsibility.

### Readiness authority and observation races

No shipped public API operation grants a positive input-readiness lease or fences
a later dispatch. `tmt ls --json` presence, `session.activity`, self-reported
`identities.status`, elapsed time and `changes.cursor` are descriptive observations,
not permission to inject input. Diagnostic `tmt check --json` capture is not a
readiness test. Extensions invoke the supplied `TMT_EXECUTABLE` through documented
API operations and ordinary JSON commands; they do not import core host adapters,
inspect private driver records or scrape pane buffers to decide readiness.

Core owns fresh binding/runtime verification and native generation checks during
the send. A preceding observation can become stale before those checks or before
input; there is no atomic lease over a person's typing, prompt contents or provider
approval state. Native channel readiness establishes the owned transport and
foreground evidence in the channel contracts. It does not establish idle,
approval-free or processed input. Codex can accept queued input while busy or
awaiting user-owned attachment. Claude provides no provider processing receipt.

The shared driver outcomes remain uniform: completed submission, queue acceptance
and unacknowledged writes are terminal; denied, awaiting-approval and uncertain
outcomes are terminal too. Only unsupported or proven-not-sent outcomes can select
the ordinary fallback. Enrolled native drivers do not use that fallback for
not-ready, unreachable, refused or uncertain sends. Core checks pane-attributed
enrollment evidence before baseline input, including retained or unknown evidence
after rebinding. Native recovery is local; no remote call approves a provider
dialog or clears enrollment evidence. An unavailable channel is not proof of an
approval dialog.

### Current limits and deferred readiness work

For ordinary pane input, a recorded runtime that is ended or cannot be verified
blocks input unless core independently verifies a replacement runtime. Legacy
bindings without recorded runtime evidence can still use paste. Direct request
wake has no universal empty-prompt or person-typing gate; reply-notice activity
debouncing is not such a guarantee. Unknown native enrollment evidence blocks
paste, but this does not make every legacy unknown runtime fail closed.

A positive public readiness fence and the strict no-input criteria for bare
shells, person-typing and unknown approval dialogs remain open in
[#600](https://github.com/pj-tmt/tmt/issues/600), with their real disposable-tmux
acceptance evidence. Remote's reserved delivery projection is proposed until core
publishes it and would be advisory, not a dispatch lease. This work does not
reinstate mandatory hold: the remote channel's newer direct-by-default grants and
opt-in hold govern [#1055](https://github.com/pj-tmt/tmt/issues/1055), which owns the
remote operation implementation and integration tests. Remote application
operations currently remain deny-all; pairing or presence alone does not enable
them. The retargeted channel API work in [#597](https://github.com/pj-tmt/tmt/issues/597)
does not supply a readiness fence. A busy native channel's valid queue acceptance
is not itself an unsafe-input refusal.

## Other operation details

`skills.install` publishes an extension's agent skills into the user's provider
skill directories, so send `consent: true` only after asking the user, as
`tmt extension install` does. `owner` is the extension name; each skill is
`{name, files: [{path, content}]}` with UTF-8 `content`, a top-level
`SKILL.md`, canonical `/`-separated relative paths (no empty or
`.`-prefixed segments, no trailing `/`), at most 64 files of 1 MiB each and
16 skills per call. Core's `tmux-team` and `tmt-inbox` cannot be claimed; the
first owner of any other name keeps it. Office links core published before
owners existed belong to core too, and only owner `office` takes them over
without force. `force: true` transfers a name and backs up an unmanaged path
in the way. Errors are
`SKILL_INVALID`, `SKILL_OWNED_ELSEWHERE`, `SKILL_CONFLICT` (an unmanaged path),
`API_CONSENT_REQUIRED` and `SKILL_INSTALL_FAILED`. Repeating identical content
changes nothing. `skills.remove` removes only links that still point at the
owner's content and reports anything else at a recorded target as `kept`. By default it
removes all of the owner's skills; `skills` limits it to 1 to 16 distinct names, and a
name the owner does not hold (or another owner does) is nothing to remove, like a
repeat. An empty, repeated or non-canonical selection is `SKILL_INVALID` and changes
nothing.
Ownership is bookkeeping between cooperating installers of one user, not
authentication: this local API cannot prove which extension is calling.

`references.resolve` answers batch reference lookups in one call. Unknown IDs
return `{id, found:false}` entries rather than errors, so an absent ID is never
confused with a failed lookup; more than 256 IDs or a non-canonical UUID is
`API_INPUT_INVALID`. Retired identities and rooms are `found` with `retired:true`.

`identities.status` returns self-reported status for many identities in one call,
in input order. Core applies expiry: `status.stale` is computed at read time, and an
expired status is still returned (stale) so a client can show it as such. Unknown IDs
are `{id, found:false}` entries, not errors. It never reports presence; join it with
`tmt ls --json`, which verifies tmux endpoints.

`changes.cursor` tells a client cheaply whether anything it may read has
changed, so it can reload only then. The cursor advances with every committed
insert, update or delete of core's durable records in this data root:
identities and their bindings, metadata, status, profiles and session state;
requests, their
attempts, responses, attention and notifications; rooms and their members;
dispatch receipts; hooks; host servers. It is opaque: compare it with the value
you last saw for equality only, never for order or distance, and never persist
it across data roots. It is not a subscription and carries no description of
what changed. It does not cover what core does not store as records: a pane's
live presence (a binding's verification time is an observation, not a change,
and a write that leaves every value the same is not a change either),
notebook files, configuration files, and any extension's own storage. A client
that reloads on a changed cursor must still reload on its own interval for
those, and must treat `API_INPUT_INVALID` for this operation (an older core) as
"no cursor" and keep its interval.

For room creation use a new UUID and `expectedRevision:0`; updates use the current
revision. Refresh rather than blindly retrying a stale write. The returned resource
matches the `room` member of `tmt room show <id> --json`.

Ordinary recipient/room history lists default to 20 items (maximum 50). Pass `nextBefore`
unchanged as the next request's `before`. Concurrent new requests above that cursor
will appear on a fresh first page; final-state changes can appear when detail is
reread. This is not a live change feed. Reads never mark incoming work as read.
Ordinary history and detail include `final:{status:"withdrawn",reason,withdrawnAtMs}`
after [originator withdrawal](request-response-v1.md#originator-withdrawal).
This terminal state is not a recipient final or approval; consumers waiting on
a person exclude it. Original prompt/history and delivery status remain intact.
For submitted replies across rooms and recipients, call `requests.list` with
`{"originatorId":"<canonical UUID>","view":"results"}`. Both fields are required
and cannot be combined with `recipientId` or `roomId`. The default limit is 8;
`limit` accepts 1 through 50. This view includes acknowledged submitted finals,
newest first by `(submittedAtMs, requestId)` descending. `nextBefore` is null
at the end or `{submittedAtMs, requestId}`; pass it unchanged as `before` with the
same scope. A preparation-time history cursor is invalid for results and vice
versa. Refresh the first page to discover newly submitted replies, including
late finals on older requests; continuation does not include newer submissions.

Results retain the ordinary history item fields: `requestId`, `roomId`,
`recipientId`, `sender`, request `kind`, `preparedAtMs`, `delivery`,
`recipientAcknowledged`, `final`, and the unchanged prompt `preview`. They add
`responsePreview` (string or null) and `previewTruncated` (boolean). A retained
response preview is its first line, at most 160 Unicode scalar values / 640 UTF-8
bytes, with no appended ellipsis. CRLF, CR, LF and Unicode line/paragraph separators
end the line; other controls use the shared
[display normalization policy](request-response-v1.md#display-normalization).
Leading spaces remain.
`previewTruncated` is true when a line ending or the cap omits any original
content. Storage reads at most 644 response bytes per row, without loading full
bodies. An expired or unavailable final retains its honest submission/expiry
header with null `responsePreview` and false `previewTruncated`; expired request
metadata is omitted. The view uses one observation snapshot without housekeeping,
acknowledgment, retention renewal or other durable row changes.

A submitted final means a reply exists, not that the work succeeded. Core does
not classify historical replies as question or blocked, or infer those categories
from body text or delivery failures. Extensions own those labels from their inbox
and status, and may join recipient/room UUIDs to their roster for member/squad
names. `requests.show` and `tmt result` remain the full-text reads.

Use `tmt x` and its revision cursor for attention, and the ordinary JSON commands
for identity, presence, room ls/show/retire, reply and result. Notes accepts a
saved identity UUID, never a caller-selected path, and does not initialize a file.

Identity hooks are durable identity-retirement subscriptions. A consumer
(lowercase letters, digits, `-` or `_`, starting with a letter, at most 64 bytes)
registers an opaque `reference` for one identity UUID; registering after
retirement is pending at once, and repeating a registration returns the current
state. Every operation is scoped to the named consumer: pages, attempts and
acknowledgments never see another consumer's hooks, and an unknown hook returns
`HOOK_NOT_FOUND`. Record an attempt before acting, act idempotently, and
acknowledge only after the action is durable; a failure leaves the hook pending.
`HOOK_NOT_PENDING` means the identity has not retired. Delivered is terminal:
repeated attempts and acknowledgments return `false`. These operations are not
identity-attributed writes and take no `identity`. An unknown identity UUID on
registration returns `IDENTITY_NOT_FOUND`.

`rooms.roster` reads an active room's non-retired members in the room's member
order. Each member is the identity summary from `tmt identity show --json`, plus
`metadata` (only keys starting with `metadataPrefix`, a literal prefix using the
metadata key grammar) and `status` as in `tmt identity status show --json`
(`null` when absent; expired status is returned with `stale: true`). One response
comes from a single consistent snapshot. It does not include presence: join it
with `tmt ls --room <roomId> --json`. An unknown or retired room returns
`ROOM_NOT_FOUND`, and a shared name returns `ROOM_AMBIGUOUS`; select by UUID.

Example conditional room write (replace the UUIDs with actual identities):

```json
{
  "version": 1,
  "operation": "rooms.write",
  "identity": "Alice",
  "input": {
    "roomId": "00000000-0000-4000-8000-000000000001",
    "room": { "expectedRevision": 0, "name": "Review", "memberIds": [] }
  }
}
```

## Optional cross-extension uses

An extension can depend on another one for a single feature and still work without it. Core does not
install such a dependency; the feature checks for it when it is used, and core only answers whether the
other extension is installed and recent enough.

**Declaration.** An extension release may carry `TMT-USES.json` at the archive root (added through the
package's cargo-dist `include`). It is optional: no file means no uses, and every release without it stays
valid. It is covered by the archive checksum like the other release files; a release that carries a
malformed one is rejected before publication, and an installed one is inspected with the rest of the
release. At most 4 KiB of UTF-8 JSON:

```json
{
  "version": 1,
  "uses": [
    {
      "feature": "browser-access",
      "label": "Browser access",
      "extension": "remote",
      "requires": ">=0.1.0-alpha.1"
    }
  ]
}
```

- `version` is `1`. `uses` has at most 8 entries. Unknown keys and duplicate `feature` values are rejected.
- `feature` matches `[a-z][a-z0-9-]{0,31}`. `label` is 1 to 48 printable characters: no control,
  bidirectional-override or line-separator characters, so it is safe on one terminal line (the same rule
  as notice display fields).
- `extension` names an installable official extension other than the declaring one.
- `requires` is an exact minimum, `>=X.Y.Z` or `>=X.Y.Z-pre`, at most 64 bytes, compared with semantic
  versioning precedence (a prerelease sorts below its release). No other operator or range exists.

Core never interprets a feature; `label` and `feature` are display and lookup data from the declarer.

**`extensions.uses`** answers from the installed receipts of the calling and the named extension. It reads
no network, application storage or extension process, and never installs or changes anything. `extension`
is the caller's own name and `feature` one it declared. The result:

| Field                                       | Meaning                                                                                                                                                                                                                                                                                                                                |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `feature`, `label`, `extension`, `requires` | The declaration, as written.                                                                                                                                                                                                                                                                                                           |
| `available`                                 | `true` when the named extension is installed, verified and at least `requires`.                                                                                                                                                                                                                                                        |
| `installed`                                 | The installed version of the named extension, or `null`.                                                                                                                                                                                                                                                                               |
| `reason`                                    | `null` when available, else `missing`, `tooOld` or `damaged` (installed but failing verification).                                                                                                                                                                                                                                     |
| `hint`                                      | One actionable line, empty when available: `<label> needs the <Extension> extension: tmt extension install <extension> --yes`; for `tooOld`, `... tmt extension upgrade <extension> --yes`; for `damaged`, a pointer to `tmt extension ls`, which lists the damaged entry with its path and the exact repair (it does not fail on it). |

A caller shows `hint` as its failure and keeps the rest of its features working. Errors are
`API_INPUT_INVALID` (a non-canonical name) and `EXTENSION_USE_UNDECLARED` (the caller has no managed
installation, or declares no such feature); a caller treats an error as "cannot check" and fails closed
with its message. The operation resolves the default installation prefix, the one `tmt extension` uses
without `--prefix`; an installation under another prefix is not visible to it. Same-user bookkeeping like
`skills.install`: the API cannot prove which extension is calling.

**Older core.** A core without `extensions.uses` does not list it in `capabilities`, and an unknown
operation is an error. A caller reads `capabilities` first (or treats the error the same way): the use
counts as "cannot check", and its message is to update tmt (`tmt upgrade --channel alpha --yes`). A release
that carries `TMT-USES.json` also needs a CLI that accepts the file: an older installer rejects an unknown
archive path, so the CLI release that supports it must publish before any extension release that carries it
(the same rule as skills trees). A caller checks a use when the feature starts (for example once at server
start) rather than on every request.

**Visibility and removal.** `tmt extension ls` lists each installed extension's `uses` (`feature`, `label`,
`extension`, `requires`, `available`, `installed`, `reason`) and prints one dim line per use. `tmt extension rm`
and an exact `tmt extension upgrade --to` name the installed extensions whose declared features stop working
because of the change, in the consent question and as `affects` (`extension`, `feature`, `label`) in JSON.
This is a warning only: the consent gate is unchanged and nothing is blocked.

## Command-line style

An extension CLI looks like core TMT by depending on `tmt-cli-style` (its only
permitted TMT dependency) and following the [CLI style](../design/cli-style.md). Build each
command with `tmt_cli_style::command`, resolve `<cli> help <command>` with
`tmt_cli_style::route` so it prints what `<command> -h` prints, and print
through its list, message and table renderers.

An action that only makes sense in a full-screen surface is not a command, and
a hidden subcommand needs an allowlist entry with a reason that
`tmt_cli_style::audit::hidden_report` checks in the extension's own tests: see
[Hidden commands](../design/cli-style.md#hidden-commands). Agents never call a
hidden or `__` command, including the `__tmt-hooks` entry below, which only core
invokes.

## Lifecycle hooks

An extension can receive best-effort observations after core commits, once the
user runs `tmt extension hooks enable <name>`. Core then invokes the resolved
`tmt-<name>` executable with `TMT_EXECUTABLE` and `TMT_HOOK_DELIVERY=1` set:

- `__tmt-hooks 1 capabilities`: print `TMT-HOOKS/1`, then one capability token per
  line (for example `lifecycle_observations_v1`), within one second and 1 KiB.
- `__tmt-hooks 1 observe`: read `{"version":1,"events":[...]}` from stdin. Events
  are `identity.created`, `identity.renamed` and `identity.retired`
  (`identityId`, `lifetime`, `retired`) and `room.created`, `room.updated` and
  `room.retired` (`roomId`, `revision`, `retired`). A rename carries no names;
  read the current name with `tmt identity show <identityId>`.

- `__tmt-hooks 1 context` (capability `context_v1`): read
  `{"version":1,"identityId":"<uuid>"}` and print `{"summary":"<text>"}` (at most
  240 characters) or `{"summary":null}` within the shared 300 ms deadline. It is
  asked only for a verified, bound identity. Do not mutate TMT state, migrate
  databases or start services. Bounded bookkeeping in the extension's own
  private cache is allowed within the same deadline.

The public `ls --json` identity rows expose `session.activity`:
`{"state":"unknown","sinceMs":null,"lastActivityMs":null,"providers":{}}`.
Working/idle are the last admitted main-turn start/end from TMT's synchronous
setup-written provider hooks, while core process evidence can establish ended.
Missing proof gives unknown. Timestamps are local accepted observation times;
`lastActivityMs` is not a heartbeat and has no stalled threshold. Provider-only
extras are currently empty. The Stop hook included in consented `tmt setup`
supplies end events. `--no-usage` disables collection and preserves that choice;
`--usage` re-enables it. Legacy recorded lifecycle-only installs remain disabled
until explicitly enabled. `tmt setup [provider] --status` reports the installed
collection state without changing it. Absent end events never cause an inferred
idle transition. Extensions
must use this public projection rather than inspect core state.

Public `ls --json` also exposes the remembered driver's optional
`resume.consumption`: cumulative completed-request input/output, cached input as
a subset, epoch/sequence, observation time and explicit completeness/gap.
It is separate from `resume.usage` (context size). Consumers baseline on first,
epoch-change, gap or decreasing-counter observations; absent evidence is
unavailable. Counter times are not heartbeats. Foreground sampling can expose accepted
completed-request evidence before the main turn ends; it does not estimate
unreported in-flight usage. See [consumption history](#consumption-history) for
the bounded public seed and coverage contract.
See [the runtime contract](../ARCHITECTURE.md#identity-names-and-bindings) for exact fields, provider
normalization and bounded-source limitations. Extensions read these public
projections, never provider transcripts or private driver state.

An admitted Claude/Codex Compacted SessionStart adds a core-owned reminder to
re-read and update the saved identity's notes. Global boolean
`notes.compactionReminder` defaults to `true`; `false` suppresses it. Temporary,
unbound and stale sessions receive none. The reminder quotes an existing notebook
path as data, or supplies `tmt notes path --identity '<UUID>'` for a missing,
unsafe or excessively long path. Hooks never initialize or read/write notebook
content. The complete reminder shares SessionStart's 4096-byte aggregate limit;
optional extension summaries and role data are trimmed before it. This changes
core context only; extension Compacted observations retain their existing contract.

Provider prompt submission also requests this context for an already verified
current session. The provider receives only extension summaries in
`UserPromptSubmit.additionalContext`; startup identity context remains at
SessionStart. Installing the prompt hook requires a consented `tmt setup` run.
Both paths share the aggregate 300 ms callback deadline inside the provider
hook's existing supervised deadline. Unknown, stale or unbound sessions receive
no extension context. Stop does not request context. Truncated prompt context
includes the same shortened-output notice as startup context.

Summaries are untrusted informational text. TMT attributes them by extension name,
escapes them and labels them `(informational)` when they reach an agent's
context; never phrase a summary as an instruction.

Observations may be dropped, repeated or delivered after later changes; treat
them as a prompt to reconcile, not as a log. Output is ignored, and the exit
status never affects the command. All observers of one command share a 500 ms
deadline. Calls to `tmt` made while `TMT_HOOK_DELIVERY` is set emit no further
observations. Replacing or re-permissioning the executable suspends delivery
until it is enabled again.

## Focus policy and checklist

All operations below use version 1 and name neither envelope `identity` nor
`originator`. This is a trusted same-user process seam, not authentication.
Squad owns user/lead authorization and duration syntax; it passes the recorded
owner UUID. Provider adapters must admit their exact launch's turn boundary before
claiming. Core never reads Squad configuration or installs provider-global hooks.

| Operation                | Input                                                                                                 | Result                                                         |
| ------------------------ | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `focus.policy.set`       | `identityId`, `ownerIdentityId`, `setterIdentityId`, `expectedRevision`, `untilMs`                    | policy view                                                    |
| `focus.policy.clear`     | same UUID/revision fields, without `untilMs`                                                          | cleared policy view                                            |
| `focus.policy.show`      | `identities`: 1–256 active UUIDs                                                                      | `policies`: views in input order                               |
| `focus.checklist.read`   | `identityId`, optional `checklistId`, `limit` (default 32, 1–128), `after` (default 0)                | bounded ordered page                                           |
| `focus.checklist.claim`  | `identityId`, `opportunity`: `turn_boundary` or `idle`                                                | `claimed:false` or a sealed checklist, page and bounded `text` |
| `focus.checklist.settle` | `identityId`, `checklistId`, `attemptToken`, `outcome`: `delivered`, `definitely_unsent`, `uncertain` | `changed`, `state`                                             |

UUIDs must be canonical and active; revision/expiry/cursors are nonnegative JS-safe
integers, a set expiry is strictly future, absent revision is 0. A clear advances
the revision and stores expiry 0. Unknown fields (including `everyMs`) are refused.
Views contain `identityId`, `revision`, `active`, `focusUntilMs`, `remainingMs`,
`heldCount`, pinned `ownerIdentityId` and `setterIdentityId`, and `activeChecklist`
(null or an unsettled claim). Held count includes that active sealed membership.
Policy conflicts return `FOCUS_REVISION_CONFLICT`; refresh before another write.

A read without a checklist ID shows unclaimed references. A sealed read includes
its retained membership regardless of settlement; later arrivals stay outside it.
Each item includes `sequence`, `requestId`, `kind`, `source`, `createdAtMs`, sender
UUID/name, `urgent`, bounded `preview`, optional `replyCommand`/`inspectCommand`,
and a bounded `resultPreview` for finals. Page fields are `identityId`,
`checklistId`, `activeChecklist`, `items`, `total` (remaining from this cursor),
`remaining`, `nextAfter` (null at the end). Continue with the same scope and the
returned sequence cursor. Canonical retention still applies.

A successful claim returns `{claimed:true,checklist,page,text}`; the checklist
contains `checklistId`, `identityId`, `attemptToken`, `throughSequence`, `state`,
`createdAtMs`. A competing or empty claim returns no transport permission. The
`idle` path verifies actual live idle evidence and refuses while Focus is active;
`turn_boundary` relies on the admitted provider adapter. No core scheduler exists.
Settlement is exact-token and idempotent; an opposite terminal outcome returns
`FOCUS_STATE_INVALID`, a wrong scope/token `FOCUS_ATTEMPT_MISMATCH`.
A crash after claim remains unknown until inspected; no elapsed time reopens it.
See the [delivery contract](request-response-v1.md#focus-delivery-windows).
