# Local extension API

Use the invoking executable supplied by extension dispatch:

```sh
printf '%s\n' '{"version":1,"operation":"capabilities","input":{}}' |
  "$TMT_EXECUTABLE" api
```

No extension installation is required. Capabilities list supported operations and
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
`identity.meta.apply` and `identity.rename`, name neither. Their target is input `identityId`. Unknown request fields
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
| `responses.recent`       | `identityId` (canonical UUID), optional `limit` (1–10, default 10)                                 | `identityId`, `items`: locally retained provider-final pane output (see below)                                                                                    |
| `dispatch.show`          | `operationId`                                                                                      | Immutable acceptance receipt                                                                                                                                      |
| `dispatch.create`        | `operationId`, `recipientIds`, `message`, optional `kind`, `origin`, `room`                        | Acceptance receipt; optional independent `wake` on first direct request                                                                                           |
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
| `consumption.history`    | `identityIds` (1–32 UUIDs), `windowsMs` (1–3), optional `maxBuckets`, `includeRetired`             | Closed timestamped deltas, coverage and included cumulative seed watermark; windows above an hour read the 30-day rollup (see below)                              |
| `identity.meta.apply`    | `identityId` (active canonical UUID), `changes`                                                    | `identityId`, `changed`; atomic conditional metadata update (see below)                                                                                           |
| `identity.rename`        | `identityId` (canonical UUID), `expectedName`, `name`                                              | `outcome`: `applied`, `conflict` or `not-found`; atomic conditional rename (see below)                                                                            |
| `identities.status`      | `identityIds` (canonical UUIDs, at most 256)                                                       | `identities`: `{id, found}` and, when found, `status`: the `tmt identity status` value or `null`                                                                  |
| `extensions.uses`        | `extension`, `feature`                                                                             | `available`, `installed`, `reason`, `hint` for a declared optional use (see [Optional cross-extension uses](#optional-cross-extension-uses))                      |

`storage.root` reports the data directory selected by the invoking core, including
its normal explicit-home/XDG/legacy selection. Extensions MUST use this operation
rather than infer configuration paths, and keep their files under
`<dataRoot>/<extension>/`. The extension owns creation and lifecycle of its
subtree, with 0700 directories and 0600 secret/state files; it MUST NOT open or
modify core's database/configuration or provider settings. Discovery does not
create the root or read its files. `capabilities` remains a constant document.

## Recent agent responses

`responses.recent` reads one exact identity's locally retained provider-final
text. Input is `{identityId, limit?}`: a required canonical identity UUID and an
optional integer from 1 through 10, default 10. Names, unknown fields and envelope
originator attribution are not accepted. Invalid input, including an explicit
out-of-range limit, is `API_INPUT_INVALID`.
The resource is `{identityId, items:[...]}`, with no pagination. A valid UUID
with no retained entries, including an unknown or retired UUID, returns an empty
`items` array; a same-name successor never inherits the previous UUID's entries.

Each item is `{text, endedMs, provider, truncated, sessionId?}`:

- `text` is the provider's final assistant text, preserving its text and line
  endings. It contains at most 8,192 UTF-8 bytes, cut on a Unicode scalar boundary.
  `truncated` is always a boolean and is true exactly when the byte cap omitted
  text. Core appends no in-band marker and applies no display normalization.
- `endedMs` is Core's observed turn-end time in Unix milliseconds, not a provider
  timestamp or a delivery/completion proof. Newest means capture order; an
  internal committed sequence breaks equal timestamps and wall-clock rollback.
- `provider` is `claude` or `codex`. `sessionId`, when available, is the admitted
  provider session identifier; otherwise the field is omitted, not null.

Capture uses only the turn-end hook TMT already installs: Claude `Stop` and
Codex `Stop`/notify. It requires the existing verified current binding/session
admission and enabled usage collection (`tmt setup --usage`, rather than
`--no-usage`). There is no new setting. Disabled capture stores no new entries;
previously captured entries retain their original expiry. A stale session,
subagent event, malformed payload or missing admissible final text creates no
entry, not an empty replacement. Hooks keep their existing finite budgets and
best-effort, non-veto behavior.

Claude extracts the final main-conversation assistant text blocks, in their
original order joined by a newline, from the Stop handler's existing bounded
transcript read. It performs no additional transcript read for this feature.
User text, tools and thinking are excluded. Codex takes `last_assistant_message`
from its admitted turn-end payload. Neither driver scrapes pane text or follows
an unverified transcript path. This is bounded turn-end observation, not a
complete transcript backup or a guarantee that an identity has prior text.

Core's SQLite store retains at most the newest 10 captures per identity. Capture
occurs only when the existing Stop admission commits a turn end, in that same
CAS transaction; a duplicate Stop treated as a no-op captures nothing. Text is
not deduplicated: two distinct committed turns with identical text are retained
separately. The transaction appends the capture and removes overflow. Each
capture freezes the current global `exchange.retentionDays` policy (default 90,
allowed 1–3650) and expires at `endedMs + retentionDays * 86,400,000`.
At equality it is expired: reads omit it even before opportunistic cleanup.
Reads and config changes never renew an existing entry's deadline.
Identity removal/retirement deletes these entries; retained exchanges are
unaffected. Insertions and removals advance `changes.cursor` in the same
transaction. No daemon or physical-deletion deadline is introduced.

This same-user process API is local-only, like `requests.list`; extensions use
`tmt api` and never open Core's database. Reads acquire one bounded observation
snapshot and never refresh provider state, acknowledge work, prune rows or
renew retention. Confirmed storage-access refusals retain `STORAGE_NOT_WRITABLE`;
other unavailable storage reads use `API_UNAVAILABLE`. Existing API input/output
bounds apply after JSON encoding.

These entries are pane output, never correlated to a request, never a final
submission or durable result, and never satisfy `talk` or release its waiter.
Captured text is stored only in this table; it is not copied into driver-state
JSON, logs, notices, Digest, exchange bodies or a network path. Consumer display
must distinguish pane output from a reply and treat the text as untrusted data.

Callers read `capabilities` first. If `responses.recent` is absent, they report
that recent responses cannot be read; they do not show an empty-success result.
An older Core's unknown-operation `API_INPUT_INVALID` is also unavailable,
without a scrape or private-storage fallback. Additive response fields remain
compatible. [Shared vectors](recent-responses-v1.json) pin bounds, ordering, expiry and error shapes.

## Conditional identity rename

`identity.rename` renames one exact active UUID only while its display name equals
`expectedName` byte-for-byte. It accepts `{identityId, expectedName, name}`, with
no originator or unknown fields. `identityId` must be a canonical UUID. `expectedName` is literal, without trimming
or canonicalization; the destination uses existing Core name validation. Malformed fields
or UUIDs are `API_INPUT_INVALID`; an invalid destination is `INVALID_NAME`.

One IMMEDIATE binding transaction checks the active UUID, compares its current
name and applies the rename. It serializes with rename, bind and retirement. UUID,
lifetime, binding and all UUID-keyed state remain unchanged; a changed name consumes
auto-name provenance, as ordinary `identity mv` does. The CLI uses this same
transaction-local rename owner, retaining its name selector and output contract.

Results use exit 0 and one typed resource:

- `{outcome:"applied", identityId, name, changed}` contains the resulting display
  name; `changed:false` is a no-op, leaving timestamps and the change cursor intact.
- `{outcome:"conflict", identityId, name}` contains the current display name from
  the transaction's snapshot. No write occurs; refresh before submitting again.
- `{outcome:"not-found", identityId}` means unknown or retired. It never resolves
  a display name or a same-name successor, even if that name is UUID-shaped.

A destination held by another active identity returns `NAME_ALREADY_ACTIVE`
(exit 1, the ordinary API fault envelope), with no mutation. The CLI preserves its
existing exit 5 collision and exit 3 `NAME_NOT_FOUND` behavior. Unconfirmed storage
failures return `API_UNAVAILABLE`; inspect current state before retrying. Pane
markers are cosmetic UUID-bound records: their old display name grants no routing
or lifetime authority. The CLI retains its best-effort post-commit refresh; the
API rename needs no host process or endpoint observation.

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
through 3,600,000, or multiples of 300,000 above that through 2,592,000,000, which
read the [30-day rollup](#thirty-day-rollup)), `maxBuckets` (1–720, default 120) and
`includeRetired` (default false). Other values or fields return `API_INPUT_INVALID`.
One response carries at most 23,040 buckets: per identity, each window contributes
`min(maxBuckets, windowMs / resolutionMs)` and the sum over identities and windows
is bounded (today's largest request, 32 x 3 x 120, is well inside it). Batch larger
rosters. A retired identity is `{id,found:false}` unless `includeRetired` is true,
which answers it as found with `retired:true` (its name is `references.resolve`'s). The normative shared vector is
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
`availableFromMs`, `lastSampleAtMs`, `latest` and `windows`, and, while `reporting`
is false, `cause`:

- `reporting` means retained closed history contains a successful normalized
  counter observation. False is unavailable evidence, not a measured zero.
- `cause` says why nothing is being sampled, read from the identity's binding in
  the same snapshot (no provider read). It is present only when `found` is true and
  `reporting` is false. `no_binding`: no active binding (offline or never bound).
  `no_agent`: a bound pane with no admitted agent session (a shell, a plain tmux or
  Herdr pane, or an agent whose start hook never admitted its session). `no_sampler`:
  an admitted running agent session with no foreground owner, so only its Stop hook
  collects (launched outside `tmt run`/`resume`). `not_sampled`: admitted and owned
  but no accepted sample: the usage hook is not installed or consented, the
  foreground wrapper predates sampling (relaunch it through `tmt run`/`resume`), or
  the seat just started. A consumer must tolerate a `cause` it does not know: later
  hosts add values additively. The cause describes the state at the read; it is not
  a prediction and never turns missing evidence into a zero.
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

Optional attribution extends these existing observations; it is not a separate
ledger. Missing (or null) new fields mean unknown, never a measured zero:

- `consumption.cacheWriteTokens` is a cumulative cache-write counter in the
  same token units and epoch as input. Cache-write and cache-read are subsets
  of the existing input count; neither is added to input or total again.
- `consumption.modelId` is the model on the last accepted completed request or
  turn, not the starting-hook `resume.model` used to relaunch a conversation.
  Claude reads `message.model`; Codex reads the preceding `turn_context.model`.
  A new Codex turn marker requires its own matching or subsequent context;
  an older turn's model is never carried into a turn with missing evidence.
  Missing/changed provider shapes, a missing bounded turn context or a model
  alias in rate-limit metadata cannot establish a model.
- `consumption.deltaByModel` groups the increments accepted since the preceding
  sequence as `{modelId,inputTokens,outputTokens,cachedInputTokens,
cacheWriteTokens?}`. It preserves model changes within a single read; it is
  **not cumulative** and must not be applied twice on a repeated sequence.
  Historical requests are not attributed on initial baseline. At most four
  distinct model IDs are retained per increment; unavailable evidence or the
  existing opaque-state size bound omits attribution while retaining totals.
- `consumption.rateLimits` is `{observedAtMs,windows}` from the same newest accepted
  Codex source line as the counters. It is absent for Claude, when that line carries
  no limits and when any field is invalid (never partial). `observedAtMs` is that
  line's own time, not the read time, so an idle session reports old limits as old.
  `windows` has one or two `{windowMinutes,usedPercent,resetsAtMs}` entries in
  strictly ascending `windowMinutes`: `usedPercent` (0–100) of the account's
  allowance used in that window and `resetsAtMs` its UTC Unix millisecond reset.
  The contract names no window; consumers choose one by length. Limits describe the
  account, so seats on one account report the same windows: they are not a session
  counter and are never summed. They are the first optional evidence omitted under
  the existing opaque-state size bound. `latest.consumption` carries them; buckets
  never do.
- `consumption.credits` is `{observedAtMs,balance,hasCredits,unlimited}` from the
  `rate_limits.credits` object of the same newest accepted Codex source line as the
  counters. It is absent for Claude, when that line carries none and when any field is
  invalid (never partial). `balance` is the provider's decimal text in its credit unit
  (`[0-9]{1,15}` with an optional fraction of 1–12 digits). It stays text because it can
  exceed what a JavaScript number holds exactly, and Core does no arithmetic on it.
  `observedAtMs` is that line's own time; `hasCredits` and `unlimited` are the
  provider's booleans. Credits describe the account, so seats on one account repeat
  them: a consumer takes the newest `observedAtMs` and never sums them.
- `consumption.serviceTier` is the provider's tier id (Codex reports `default` or
  `priority`; lowercase letters and `_`, at most 24 bytes). Codex writes it in a
  `thread_settings_applied` event, not on the counter line, so Core reads the newest
  such event in the bounded tail it already reads for model attribution. It is the
  setting in force at the accepted read, not a per-token attribution: an earlier
  request may have run under another setting. It is absent for Claude, when the tail
  holds no settings event and when the newest value is not a plain id; **absent means
  unknown, never `default`**.
- `credits` and `serviceTier` are optional evidence omitted before `rateLimits` under
  the existing opaque-state size bound. `latest.consumption` and `resume.consumption`
  carry them; the 5-second history buckets never do, and a rollup bucket carries the
  [per-bucket](#thirty-day-rollup) values.
- History buckets optionally add `cacheWriteTokens` and `byModel` with the same
  entry shape, containing **deltas for that bucket**, including all accepted
  model changes. The rows sum to **at most** the bucket's tokens: an unknown
  portion or a skipped sequence leaves its share unattributed, and an absent or
  empty `byModel` attributes nothing. Aggregating beyond four models drops the
  split built so far, so an aggregate may carry only a partial one. A bucket is
  never relabelled with its latest model. Unknown cache-write evidence omits that counter without altering legacy
  totals, coverage, gaps or retention. Existing stored rows are not backfilled.
- History buckets optionally add `byDriver`: `{driver,inputTokens,outputTokens,
cachedInputTokens}` rows in ascending `driver` order, holding **deltas for that
  bucket** counted between two reads of that driver. Core records the driver of the
  read, so a bucket keeps each share under its own driver across a restart, a gap or a
  driver switch, so a consumer need not infer it from `latest.driver`. The rows sum to **at
  most** the bucket's tokens, and the remainder (a row stored before this field, an
  uncovered interval) is unknown. A missing `byDriver` means unknown, never one driver.
  Aggregating beyond four drivers drops the split built so far, so an aggregate may
  carry only a partial one; the at-most rule always holds. Existing stored rows are
  not backfilled.
- A Codex session's consumption includes the sessions it started. Codex writes a
  subagent, and an auto-review (`source.subagent.other = "guardian"`), as a rollout of
  its own whose header names its parent. A started session counts toward the seat that
  owns the **root** of its parent chain, so a review of a subagent counts toward the same
  seat; one that is, or later becomes, another seat's bound session counts only toward
  that seat, with everything it started (what it already added here stays). To place a
  new rollout, Core reads its first line (the session header) and keeps only its id and
  parent; it reads token counters only of rollouts whose chain ends at a bound session,
  never the conversation, and no counters or conversation of a rollout whose root is not
  a seat's session. A started session nobody wrote to for 24 hours is folded into a closed
  sum and forgotten: if it wakes later, it and what it then starts are not counted (a
  bounded undercount; the seat's own session is unaffected). Counting starts when Core first scans a seat's started sessions: what
  they had already consumed is not counted, so the step is not new usage and a seat's
  history shows no spike when the rule ships; a session started later counts from its
  first token. While Core is still finding or following them, `complete` is false.
  `resume.consumption` and `latest` carry the sum; the split by model follows the
  at-most rule above, so a started session that cannot be attributed adds none. An
  unread family never turns into a zero: the seat's own counters carry the last sum
  unchanged until it is read again. A seat's session consumed before this rule is
  unchanged; a family that is lost starts a new baseline like any decreasing counter.

Core retains 5-second buckets for two hours: at most 1,440 closed buckets plus
one open bucket per identity; the [rollup](#thirty-day-rollup) keeps coarser sums for 30 days. Expired history is excluded immediately on reads;
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

### Thirty-day rollup

A `windowsMs` above 3,600,000 (a multiple of 300,000 through 30 days) reads the
rollup instead of the 5-second buckets; shorter windows are unchanged byte for byte.
Core folds every accepted read into 5-minute buckets in the same transaction that
writes the 5-second bucket, with one rule for both, so a rollup bucket is the sum of
the fine buckets it covers: tokens, per-model and per-driver attribution, covered time and the
`gap`, `discontinuous` and incomplete flags. Retention is 30 days by time alone
(a write prunes its identity's expired rows and a few of anyone's, so a retired
identity ages out too); there is no row cap. Recording starts at the migration, with
no backfill, and a gap in sampling is a gap, never a measured zero.

A rollup window is `{windowMs,fromMs,toMs,bucketMs,buckets}` as above with these
differences. `toMs` is the last closed 5-minute boundary, so up to five minutes of
the newest activity is not yet in it. `bucketMs` is
`ceil((windowMs/300000)/maxBuckets)*300000`. The window adds `resolutionMs` (300,000),
`availableFromMs` (start of the identity's first rollup bucket with a successful read,
or null) and `retainedFromMs` (`max(toMs - 30 days, availableFromMs)`; earlier intervals
are uncovered). A bucket adds `rateLimits` (the public `resume.consumption` shape): the
newest limits **observed inside the bucket**, absent when no provider line in it carried
any. Account windows repeat across seats on one account and across buckets, so a reader
dedupes by `(windowMinutes, resetsAtMs)` and never sums them. A bucket also adds the
Codex `credits` (the public `consumption.credits` shape), the newest **observed inside the
bucket** by the same rule as `rateLimits`, and `serviceTier`, the tier id of the newest
read **that moved tokens** in the bucket (an idle read claims none; a moving read whose
tier is unknown leaves the known one). Both are absent when the bucket saw none, never
carried forward from an earlier bucket, and an absent `serviceTier` is unknown, never
`default`. When `maxBuckets` merges several 5-minute rows into one bucket, `credits` is
the newest by `observedAtMs` and `serviceTier` the tier of the newest row that has one, so a tier change shows
at the bucket resolution. Credits repeat across seats on one account: a reader dedupes by
the newest `observedAtMs` and never sums them. Buckets from before schema 56 carry
neither. The identity-level `reporting`, `availableFromMs`, `lastSampleAtMs` and `latest`
still describe the 5-second tier.

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

`dispatch.create.origin` is optional and only `"automation"` is accepted; absent
means a person made the request, and any other value is `API_INPUT_INVALID`. An
extension that composes traffic no person typed, such as a scheduler or a notice,
declares it once per operation. It is recorded on every request the operation
creates, never changes, and is part of the intent, so replaying an operation id
with the other value conflicts while an absent value keeps every earlier digest.
`tmt talk`, `send`, `room send` and MCP always record a person. Like `sender`, the
declaration is local attribution, not authentication.

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
readiness test. Plain `check` may hand one retained Digest checklist to an idle
pane, so a tool that reads pane text only for display data, such as a provider
statusline, uses `tmt check --capture-only --json`, which captures with no
side effect and is still not a readiness test. Extensions invoke the supplied
`TMT_EXECUTABLE` through documented API operations and ordinary JSON commands; they
do not import core host adapters, inspect private driver records or scrape pane
buffers to decide readiness.

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
skill directories, so send `consent: true` only after obtaining user consent.
Native extension installation publishes its release skills after binary consent.
`owner` is the extension name; each skill is
`{name, files: [{path, content}]}` with UTF-8 `content`, a top-level
`SKILL.md`, canonical `/`-separated relative paths (no empty or
`.`-prefixed segments, no trailing `/`), at most 64 files of 1 MiB each and
16 skills per call. Core's `tmt` and `tmt-inbox` cannot be claimed; the
first recorded owner of any other name keeps it unless `force: true` transfers
the name claim and backs up unmanaged target entries outside discovery in the
sibling `.tmt-skill-backups` directory. API-supplied names refuse unmanaged targets
with `SKILL_CONFLICT`, even if a name or owner resembles an official product.
Only core bundled and verified official release skills replace catalog names
without a prompt, flag, backup or target ownership/content check. Other names and
symlink destinations remain untouched. Immutable sources are validated before
publication. Errors are `SKILL_INVALID`, `SKILL_OWNED_ELSEWHERE`, `SKILL_CONFLICT`
(unmanaged target, invalid source or retired migration path), `API_CONSENT_REQUIRED`
and `SKILL_INSTALL_FAILED`. Repeating identical content at current links
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
Every history item carries `origin`: `"person"`, `"automation"`, or `"unknown"` for
a request created before origins were recorded. A consumer treats `unknown` as a
person's request. Core records the value at creation and derives it from nothing
else, so neither a sender's name nor the message text can change it.
For submitted replies across rooms and recipients, call `requests.list` with
`{"originatorId":"<canonical UUID>","view":"results"}`. Both fields are required
and the view cannot be combined with `roomId`. Adding `recipientId` lists only that
recipient's finals with the same order, cursor and limits, so one agent's older
replies stay reachable past the newest page; a page then walks the originator's
submitted finals newest first until it has `limit` + 1 matches, which in the worst
case reads every retained final of that originator, and an older core answers
`API_INPUT_INVALID` for this form. The default limit is 8;
`limit` accepts 1 through 50. This view includes acknowledged submitted finals,
newest first by `(submittedAtMs, requestId)` descending. `nextBefore` is null
at the end or `{submittedAtMs, requestId}`; pass it unchanged as `before` with the
same scope. A preparation-time history cursor is invalid for results and vice
versa. Refresh the first page to discover newly submitted replies, including
late finals on older requests; continuation does not include newer submissions.

Results retain the ordinary history item fields: `requestId`, `roomId`,
`recipientId`, `sender`, request `kind`, `origin`, `preparedAtMs`, `delivery`,
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

- `version` is `1`. `uses` has at most 8 entries. The only optional top-level key is `requiresCli` below; other unknown keys and duplicate `feature` values are rejected.
- `feature` matches `[a-z][a-z0-9-]{0,31}`. `label` is 1 to 48 printable characters: no control,
  bidirectional-override or line-separator characters, so it is safe on one terminal line (the same rule
  as notice display fields).
- `extension` names an installable official extension other than the declaring one.
- `requires` is an exact minimum, `>=X.Y.Z` or `>=X.Y.Z-pre`, at most 64 bytes, compared with semantic
  versioning precedence (a prerelease sorts below its release). No other operator or range exists.

Core never interprets a feature; `label` and `feature` are display and lookup data from the declarer.

**Minimum CLI.** Version 1 may additionally declare one optional `requiresCli` string using the same
grammar, bound and Semantic Versioning precedence as `requires`. For example,
`{"version":1,"requiresCli":">=5.0.0-alpha.255","uses":[]}`. No declaration means no CLI minimum.
The native installer freezes the minimum as optional `requires_cli` in the installed receipt;
older receipts without it remain valid. The receipt value must match its verified declaration.

Before publishing a candidate extension, install and upgrade compare that minimum with the running
CLI build. A too-old CLI refuses before prefix, release, activation, receipt, link or skill changes,
retaining the existing command error envelope and one suggestion: `tmt upgrade` (the CLI keeps
its own channel and consent flow). There is no start-time or network compatibility check and no
automatic CLI upgrade. This comparison uses ordinary SemVer precedence: `5.0.0-dev` is newer than
`5.0.0-alpha.N`; release tooling's preparation-version ordering is separate.
Damaged repair checks the acquired, verified release's minimum before republishing and preserves the
prefix on refusal; healthy repair remains a no-op.

`extension ls` determines CLI compatibility from the verified installed receipt only. A declared minimum adds
`cli: {requires, running, compatible}` to that extension's JSON row. An incompatible minimum adds one
dim human warning alongside the existing optional-use lines, without marking the installation damaged.
No declaration adds no `cli` field or warning. The supporting CLI must publish before extensions carry
`requiresCli`: older installers reject the unknown declaration key rather than ignoring the requirement.

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

The public `ls --json` identity rows optionally expose
[`runningDriver`](identity-status-v1.md#list-runtime-driver) for a freshly observed
Running runtime with a matching admitted session on the current active binding.
Use that projection for current driver evidence; the existing `driver` may name a
remembered harness. The linked contract owns presence and omission semantics.

The public `ls --json` identity rows expose `session.activity`:
`{"state":"unknown","sinceMs":null,"lastActivityMs":null,"providers":{}}`.
Working/idle are the last admitted main-turn start/end from TMT's synchronous
provider hooks, written by `tmt setup` or composed by a `tmt run` launch, while core
process evidence can establish ended. `waiting` records Claude's exact
`Notification` with `notification_type: "permission_prompt"`, observed through the
same consented hooks. Claude may delay that notification (normally about six
seconds); TMT does not infer a prompt from pane text or a pre-decision hook.
`PostToolUse`, `PostToolUseFailure`, `UserPromptSubmit` and `Stop` clear waiting
through the existing admitted-event transition. Manual denial emits no installed
clear event: waiting remains until the next admitted event, without a timer.
A tool completion cannot reopen an idle turn. Duplicate phases do not renew
timestamps. These events record the last admitted observation, not a count of
parallel pending approvals, and never decide or answer a permission request.
Claude reports waiting; Codex does not report it in this contract. Consumers show
waiting only when `state` says `waiting`; no per-session capability is exposed.
Missing proof gives unknown. Timestamps are local accepted observation times;
`lastActivityMs` is not a heartbeat and has no stalled threshold. Provider-only
extras are currently empty. The Stop hook included in consented `tmt setup`
supplies end events. For Codex, an interrupted turn ends through its turn-correlated
`Interrupt` hook, which is activity only and installed with the lifecycle hooks. It runs
as Codex's hook trust policy allows: an ordinary unmanaged definition runs only after the
user trusts it in `/hooks`, and TMT never grants or bypasses that trust. While Codex skips
it, the last admitted activity stays unchanged. `--no-usage` disables usage and
response collection (not this activity observer) and preserves that choice;
`--usage` re-enables it. Legacy recorded lifecycle-only installs remain disabled
until explicitly enabled. `tmt setup [provider] --status` reports the installed
collection state without changing it. Absent end events never cause an inferred
idle transition. Extensions
must use this public projection rather than inspect core state.

Public `ls --json` also exposes the remembered driver's optional
`resume.consumption`: cumulative completed-request input/output, cached input as
a subset, optional cache-write and request/turn model attribution, optional Codex
`rateLimits`, `credits`, `serviceTier`, epoch/sequence, observation time and explicit
completeness/gap.
It is separate from `resume.usage` (context size). Consumers baseline on first,
epoch-change, gap or decreasing-counter observations; absent evidence is
unavailable. Counter times are not heartbeats. Foreground sampling can expose accepted
completed-request evidence before the main turn ends; it does not estimate
unreported in-flight usage. See [consumption history](#consumption-history) for
the bounded public seed and coverage contract.

A row's live context use is `resume.usage` (`{tokens, windowTokens?, observedAtMs}`) while
`runningDriver` is present; without `runningDriver` show none. An absent `windowTokens` (Claude)
means no window, and unknown is never 0. `observedAtMs` is the local time of the turn-end or start
observation that stored it, so a value seen mid-turn belongs to the previous turn. Stored usage
carries no process incarnation; `runningDriver` and the driver's start clearing it are the fence.

<a id="last-observed-model"></a>
`resume.observedModel` is `{model, observedAtMs}`: the **last observed** model, which an
admitted provider event reported, and its local accepted observation time. It is a recorded
fact, not the current model and not liveness evidence. A Codex `/model` change shows only at
the next turn event, with no time bound.

- Codex prompts, Stops and main-agent interrupts carry it; a subagent event never supplies it.
- A start of either provider sets it from the start's own reported model, or clears it.
- Claude turn events carry none, so a Claude session shows its start's model.
- A rejected or duplicate event never changes it. An admitted event without a valid model keeps
  the earlier observation and its time.
- The key is absent, never null, while nothing is recorded. It can also be lost when the
  bounded driver state evicts it, or when an older `tmt` replaces the state.
- It never changes `resume.model` (the starting-hook model that a resume launch replays) or
  `resume.consumption.modelId` (the model of the last accepted completed request or turn).

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
current session. `UserPromptSubmit.additionalContext` carries a nonzero count of
incoming unacknowledged X items with the `tmt inbox` command that pulls them
(omitted at zero; it is attention, not unanswered or unsent work, and the hook
neither retrieves nor acknowledges anything), then optional extension summaries,
which truncation removes first; startup identity context remains at SessionStart. Installing the prompt hook requires a consented `tmt setup` run.
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

## Digest policy and checklist

All operations below use version 1 and name neither envelope `identity` nor
`originator`. This is a trusted same-user process seam, not authentication.
Squad owns user/lead authorization and duration syntax; it passes the recorded
owner UUID. Provider adapters must admit their exact launch's turn boundary before
claiming. Core never reads Squad configuration or installs provider-global hooks.

| Operation                  | Input                                                                                                 | Result                                                         |
| -------------------------- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `digest.policy.set`        | `identityId`, `ownerIdentityId`, `setterIdentityId`, `expectedRevision`, `untilMs`                    | policy view                                                    |
| `digest.policy.clear`      | same UUID/revision fields, without `untilMs`                                                          | cleared policy view                                            |
| `digest.policy.show`       | `identities`: 1–256 active UUIDs                                                                      | `policies`: views in input order                               |
| `digest.checklist.dueNow`  | `identityId`                                                                                          | `identityId`, `heldCount`, `throughSequence`                   |
| `digest.checklist.flush`   | `identityId`, optional `whenBusy` (boolean)                                                           | `identityId`, `state`                                          |
| `digest.stats.show`        | `identities`: 1–256 active UUIDs                                                                      | `stats`: views in input order                                  |
| `digest.checklist.read`    | `identityId`, optional `checklistId`, `limit` (default 32, 1–128), `after` (default 0)                | bounded ordered page                                           |
| `digest.checklist.history` | `identityId`, optional `checklistId`, `limit` (default 32, 1–128), `after` (default 0)                | content-free settled checklist evidence                        |
| `digest.checklist.claim`   | `identityId`, `opportunity`: `turn_boundary` or `idle`                                                | `claimed:false` or a sealed checklist, page and bounded `text` |
| `digest.checklist.settle`  | `identityId`, `checklistId`, `attemptToken`, `outcome`: `delivered`, `definitely_unsent`, `uncertain` | `changed`, `state`                                             |

UUIDs must be canonical and active; revision/expiry/cursors are nonnegative JS-safe
integers, a set expiry is strictly future, absent revision is 0. A clear advances
the revision and stores expiry 0. Unknown fields (including `everyMs`) are refused.
Views contain `identityId`, `revision`, `active`, `digestUntilMs`, `remainingMs`,
`heldCount`, pinned `ownerIdentityId` and `setterIdentityId`, and `activeChecklist`
(null or an unsettled claim). Held count includes that active sealed membership.
Policy conflicts return `DIGEST_REVISION_CONFLICT`; refresh before another write.

A read without a checklist ID shows unclaimed references. A sealed read includes
its retained membership regardless of settlement; later arrivals stay outside it.
Each item includes `sequence`, `requestId`, `kind`, `source`, `createdAtMs`, sender
UUID/name, `urgent`, bounded `preview`, optional `replyCommand`/`inspectCommand`,
and a bounded `resultPreview` for finals. Optional `contextTokensAtArrival` and
`contextObservedAtMs` preserve the receiver's latest recorded, readable driver
usage when the reference first entered hold. They are absent when evidence is
unknown, stale, pending resume, unsupported or from a future observation. The
snapshot does not refresh the provider or change on retry. Page fields are `identityId`,
`checklistId`, `activeChecklist`, `items`, `total` (remaining from this cursor),
`remaining`, `nextAfter` (null at the end). Continue with the same scope and the
returned sequence cursor. Canonical retention still applies.

A successful claim returns `{claimed:true,checklist,page,text}`; the checklist
contains `checklistId`, `identityId`, `attemptToken`, `throughSequence`, `state`,
`createdAtMs`. A competing or empty claim returns no transport permission. The
`idle` path verifies actual live idle evidence. While Digest is active it may claim
only the range previously marked by `digest.checklist.dueNow`; that operation
captures the current retained, unclaimed range atomically without changing policy
revision/expiry. Later arrivals stay held on the idle path until another due-now
call or policy expiry/clear. An empty call returns count/sequence 0. Duplicate calls
with no new arrivals keep the same range; definitely-unsent membership remains
eligible for a later independently admitted opportunity. `turn_boundary` retains
its existing broader eligibility and relies on the admitted provider adapter.
Due-now grants no transport permission and starts no worker. No core scheduler exists.

`digest.checklist.flush` offers one existing idle-delivery opportunity for the exact
active identity UUID, without name or pane resolution. Core reuses its ordinary
live-session, readiness, claim, transport and settlement checks; it is not a
readiness grant. The one option, `whenBusy` (a boolean, false by default; any other
type or an unknown field is refused), replaces only the live-idle evidence with the
exact verified live binding and session, so an extension's cap can offer a held
backlog to an agent that is busy or never reports its activity. The approval and
host guards, the claim, the seal and settlement are unchanged, `nothing_due` still
means no range was marked, and `not_idle` then means no live session. It delivers
what an ordinary message already delivers to a busy agent, never more. The operation reads the configured paste-to-Enter delay, starts
no timer and returns no message or captured pane text. `state` is `nothing_due`
when no unclaimed range is mechanically eligible, `not_idle` when no idle claim
(or, with `whenBusy`, no live session) is admitted, `delivered` after successful settlement, `uncertain` after uncertain
transport, or `unavailable` after definitely-unsent settlement. Errors remain
ordinary API errors. A competing or uncertain sealed checklist is never replayed;
the result is not a retry lease.

Stats contain `identityId`, `heldCount` (including active claimed membership),
`oldestHeldAgeMs` (null with no held references), `deliveredDigests`, `dueCount`,
`nextEligibleAtMs` and `observedAtMs`. Ages use retained timestamps, never content.
`dueCount` counts unclaimed mechanically eligible references and is 0 during an
active claim. `nextEligibleAtMs` is observation time for a due range, policy expiry
for held future eligibility, or null with no pending work/an active claim; it is
neither a live-readiness guarantee nor the extension's configured send deadline.
The delivered counter increments once on the first successful `delivered`
settlement, never on claim, definitely-unsent or uncertain. It survives checklist
pruning/restart; migration seeds only successful retained historical checklists,
because already-pruned history cannot be reconstructed.
Settlement is exact-token and idempotent; an opposite terminal outcome returns
`DIGEST_STATE_INVALID`, a wrong scope/token `DIGEST_ATTEMPT_MISMATCH`.
Existing request housekeeping prunes empty delivered/definitely-unsent checklist
records after the metadata age floor; retained member links follow canonical
request retention. Settlement idempotency lasts while the record is retained.
Claimed and uncertain records remain discoverable even after members expire.
A crash after claim remains unknown until inspected; no elapsed time reopens it.
See the [delivery contract](request-response-v1.md#digest-delivery-windows).

### Digest checklist history

`digest.checklist.history` v1 is a content-free, storage-only observation over
existing retained checklist records, not a complete archive. It never claims,
delivers, acknowledges, refreshes provider evidence, prunes or renews retention.
The normative [history vectors](digest-checklist-history-v1.json) pin its shape.

Input is a canonical active `identityId`, optional canonical `checklistId`,
`after` (default 0, nonnegative JS-safe integer) and `limit` (default 32, 1–128).
Envelope identity/originator and unknown fields are refused. No name resolution
or transport authorization is performed.

Without `checklistId`, `after` is a **settlement sequence**. Each checklist receives
one sequence from Core's existing durable change cursor in its first terminal
transition transaction, independent of timestamps; retries preserve it. Only
settled records with a known sequence greater than `after` are listed, ordered
by that sequence. Claimed-but-unsettled records do not appear. Late settlement
receives a newer sequence even if its claim predates the collector's last read
or the wall clock moved backwards. Unrelated Core changes can leave sequence gaps.

The result contains `identityId`, `observedAtMs`, `throughSequence` (the snapshot's
durable change-cursor watermark), `retainedFromSequence` (oldest surviving known
settlement sequence, or null), `legacySettledCount` (surviving terminal records
without an assigned sequence), `deliveredDigests` (the existing cumulative counter),
`checklists` and `nextAfter` (last returned settlement sequence when more records
remain, otherwise null). At end of enumeration, collect through `throughSequence`;
future terminal transitions receive higher sequences. Finish any member-input
continuations before committing that collector watermark.

Each checklist has `checklistId`, `settlementSequence`, `claimedAtMs`,
`settledAtMs`, `outcome`, `opportunity`, `sealedCount`, `retainedCount`,
`membershipComplete`, `items` and `nextItemAfter`. `outcome` is `delivered`,
`definitely_unsent` or `uncertain`; `opportunity` is `idle`, `turn_boundary` or
null for unknown historical claims. Claim captures original `sealedCount` and
opportunity; the first terminal transition records settlement time/order atomically.
An identical settlement changes neither. Legacy facts remain null, and terminal
records predating sequence collection are counted as legacy rather than assigned
an invented historical order.

`retainedCount` counts the complete currently visible sealed membership, independent
of the returned input page. `membershipComplete` is true only when a known original
sealed count equals it. It does not imply complete context evidence or a complete
history window. Definitely-unsent settlement releases membership; its count remains
known but inputs are unavailable. Expired request metadata hides member inputs
immediately, even before physical cleanup. Existing checklist/request pruning stays
unchanged. The oldest surviving sequence is not a coverage guarantee, and absent
records cannot be reconstructed from the cumulative delivered count.

Each input is only `sequence`, `createdAtMs`, `contextTokensAtArrival` and
`contextObservedAtMs`. Context fields are nullable immutable hold-time observations;
missing context is unknown, never zero. No body, preview, request ID, receipt,
reply/inspection command, attempt token or provider locator is returned.

An enumeration page returns at most `limit` checklists and **128 member inputs in
total**. Ordinary checklists stay inline: the page ends before a subsequent batch
would exceed the input budget. If the first batch itself has more than 128 visible
inputs, its first 128 are inline and `nextItemAfter` names the last input sequence.
Resume through the **same operation** with `checklistId` and `after` equal to that
input cursor; in this scope `limit` bounds inputs, `nextAfter` is null, and the
checklist's settlement sequence stays unchanged. A scope mismatch is refused;
unsettled or legacy terminal records without a sequence return no checklist.
`nextItemAfter` is null at input-page end. Both scopes read one SQLite snapshot;
membership may expire between pages and clients must check completeness on each.
The fixed numeric/UUID fields and these bounds keep encoded pages below 128 KiB,
within Digest's existing 1 MiB Core subprocess capture limit.
