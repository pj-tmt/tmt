# Remote squad directory v1

Remote owns one list/send/delivery interface for other machines. The first adapter is
Firestore, reusing the existing paired native channel, encryption, operation journal
and receipt owners. This contract grants no authority and creates no transport,
pairing flow or delivery ledger. Core's prepared-message v1 contract is unchanged.

`rust/tmt-remote/src/squads` owns the types, projection and interface. `reads` owns
the bounded public Ops invocation. `cloud_inbox` and `cloud_pairing::service` own
carrier and original-operation observations. A joined publisher under the serving
Service publishes filtered summaries; this module retains verified cache in the existing private
Store and adds no public command.

## Summary bytes

One encrypted summary has this JSON shape:

```json
{
  "version": 1,
  "machineName": "Studio",
  "observedAt": 1700000000000,
  "status": "available",
  "squads": [
    {
      "squad": "infra",
      "lead": {
        "id": "00000000-0000-4000-8000-000000000001",
        "name": "lead",
        "state": "working",
        "presence": "active"
      },
      "members": [
        {
          "id": "00000000-0000-4000-8000-000000000002",
          "name": "worker",
          "state": "waiting-ci",
          "presence": "active"
        }
      ]
    }
  ]
}
```

Member UUIDs are keys; names are display text. The source is only the successful
public `tmt ops squad ls --json` document: `squad.name`, nullable `squad.lead`, and
`sections[].rows[]`. The lead is separate from the members. A null lead is omitted
in the summary. The same UUID appearing twice within a squad refuses the summary;
equal display names on different UUIDs are valid.

Only these member fields survive:

- `id` and `name`.
- `state`: `working`, `idle`, `blocked`, `review`, `testing`, `hold`, `waiting`,
  `waiting-ci`, `resting`, or `unknown`. Only the public row `state` is read.
- `presence`: `active`, `offline`, or `unknown`.

All unrecognized state/presence values become `unknown`, on both publication and
read. Unknown fields are ignored, not forwarded. No activity string, mood,
idle alert, staleness, color, attention, task, next, pending, usage, timer, pane,
path or conversation field enters the summary. There is no fallback to the broad
owner-browser Ops projection, cron, private Ops/Core state or host inspection.

A missing/failing Ops command, invalid JSON or changed required document shape
produces a publishable `{version:1,machineName,observedAt,status:"unavailable"}`
snapshot with no `squads`. It is never an empty available success and contains no
stale facts or diagnostic free text. An explicitly empty available `squads` list
is valid. `Summary::read` rejects incompatible versions,
malformed identity/name/shape and duplicate JSON keys. Names are bounded text
without controls or bidi-control characters. Named limits live in `limits.rs`;
overflow refuses instead of truncating identity. Received unlisted fields are
removed before any subsequent serialization.

`observedAt` is Unix milliseconds, sampled after the public read attempt finishes. A
viewer computes age; a future timestamp cannot prove freshness. It must not replace
observed time with its local read time or persist an online/offline flag as fact.
The unchanged-summary heartbeat is five minutes; changed allowed data is coalesced
and published promptly by the summary publisher. Its traffic shares the existing
Firestore project guard and has no separate quota allowance.

## Consumer interface

`Interface` has exactly three operations:

- `list`: return a versioned list of `{remote,status,summary?}` entries for other
  machines. The source port supplies verified retained bytes and existing local
  routing metadata. Each machine is `available`, `unavailable` or `incompatible`.
  A malformed record is unavailable; an unsupported version is incompatible;
  neither hides healthy peers or exposes squads. A published unavailable snapshot
  retains its machine name and observation time, with no squads. The adapter
  filters its own machine UUID. Only failed local sources, invalid routing keys,
  duplicate machine IDs, conflicting selectors or machine-limit overflow fail the
  entire list. Machine identity and the local `remote` selector are routing
  metadata, outside the synchronized summary.
- `send`: carry exact Core-prepared message bytes through the existing serving
  service. Preserve its current version-1 answer. In particular, `sent` is not
  proof that the receiving agent was handed the message.
- `delivery`: observe the original operation UUID through the existing carrier,
  operation journal and public request state. Never dispatch it again, reseal its
  plaintext, extend its original lifetime or create another ledger.

## Delivery evidence

Only a sender outbox row with the existing private route binding may be observed
as an original send. A metadata lookup/final row is not an original send. Missing
or unclaimed originals return `unknown` without provider work. A selected peer must
still have its existing confirmed connection; current provider and remote grant
checks continue to govern every read.

`queued` requires the exact original carrier to be observed in Firestore with no
recipient response yet, or the original request's authoritative `messageState`
being `sent`. It means durable acceptance awaiting hand-over evidence, not proof
that a machine is currently offline. A write claim, send return code, absent local
cache, timeout or failed read is insufficient.

When a recipient response exists but its in-memory HPKE context is gone, use the
existing signed `operation.show` recovery for that original ID. Its `accepted`
receipt identifies the original Core request. Read that request through the
existing signed `requests.show` operation; do not parse pane output or another
agent's conversation history. Returned free text and final bodies are not exposed
by the delivery projection. A delivery observation can issue two signed remote
reads (`operation.show`, then `requests.show`), each a Firestore round-trip charged
to the shared project guard. Consumers observe on demand or on change, never in a
polling loop.

| Observed evidence                                         | Interface state |
| --------------------------------------------------------- | --------------- |
| Exact stored carrier, no stored recipient response        | `queued`        |
| Original Core `messageState` is `sent`                    | `queued`        |
| Journal is `held`, or Core is `batched`                   | `held`          |
| Core is `delivered` or `working`                          | `delivered`     |
| Core is `answered`, regardless of final retention         | `answered`      |
| Original journal refused/cancelled, or Core is `not_sent` | `failed`        |
| Journal is `uncertain`, or Core is `unconfirmed`          | `unconfirmed`   |
| No eligible original, or an unrecognized/missing state    | `unknown`       |

Failed observation/auth/transport reads remain errors; they do not make the
original operation `failed`. Only bounded fixed reason codes may accompany the
projection. There is no automatic resend. A final retains Core's receipt-bound
acceptance and retention; the directory interface never settles a request itself.

## Verification

Tests exercise strict field exclusion (including free-text activity), renamed
members, null leads, unknown fields/values, malformed/changed Ops shapes and
missing/failing commands. Carrier adapter tests use existing isolated service and
public-Core fixtures to verify original-ID observations, current receipt/final
states and no additional dispatch. Private Store and fake ordinary-user provider
tests cover filtering, coalescing, heartbeat, budget holds, immutable publication
recovery, cache restart, out-of-order/tampered input and revocation. Each storage
fixture asserts its private `TMT_HOME`. No test uses a real account or provider;
two-machine public CLI acceptance belongs to the separate integration consumer.

## Publisher and retained cache

The native-only `squads.summary` operation uses the existing signed HPKE inbox,
current grant/member fences and transport attempt store. It needs `agents.read`,
not sending permission. Each recipient gets only its allowed agent UUIDs, including
lead filtering; a hidden-only squad is omitted. Unknown fields are discarded before
cache persistence. A signed per-peer sequence prevents older arrivals replacing
newer cache entries, independently of `observedAt`.

Before publishing, the Service uses the existing signed `capabilities` operation.
Only a native peer advertising `squads.summary` under its current `agents.read` grant
receives metadata. Capability observations are cached for one day, scoped to the
native key and grant revision; failed checks back off from one minute to one day.
Absent support is not inferred from labels or another operation. Completed local
capability observation rows are retired without retaining response contexts.

The separate publisher samples public Ops at five-second intervals only with
eligible, capability-confirmed native read peers. The existing inbox worker remains
free to answer capability exchanges. Both workers share the Service, Store, project
guard and stop owner; stop joins both before releasing the serve lease. The publisher
coalesces content changes for five seconds; observation time alone
does not publish. The unchanged heartbeat is five minutes. Missing/failed Ops reads
publish the explicit unavailable snapshot. Pending intent and normalized verified
cache survive restart. Unconfirmed attempted carrier IDs are observed, never
created again. If original-ID recovery observes no carrier, that snapshot remains
unconfirmed and its attempted row stays immutable. The latest desired state is held
for five minutes, then a fresh observation may use a newer sequence and ID; it is not
a delivery claim or replay of the absent snapshot. Existing carrier-retention expiry
retires its active snapshot atomically;
the retained desired state can produce a fresh observation with a newer sequence,
never a recreation of the expired ID. Budget/Auth failure holds pending state for at
least the existing idle interval, in addition to the shared project's own guard and backoff.

After verified cache and sequence-watermark commit, the existing driver promptly
deletes a validated summary's inbox document through the project guard. It writes
no provider response and does not delete a nonexistent response document. A failed
or budget-refused delete retains the cleanup row and verified cache. The source
compacts completed summary outboxes while preserving the latest completion and
every active attempt. Generic messages and finals retain their existing 30-day
recovery horizon. Sequence watermarks survive summary carrier compaction. A paired
machine without cached bytes is unavailable, with no fabricated observation time.

A quiet directed peer costs 288 heartbeat creates plus 288 inbox cleanup deletes,
about 576 writes per day, before capability exchanges, descriptor reads,
post-create observations, inbox scans, Rules lookups and checkpoints. Fanout
multiplies that work. All traffic shares the existing guard, not a new free-plan
allowance. Provider storage proves publication, never receiver delivery.
