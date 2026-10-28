# Remote squad directory v1

Remote owns one list/send/delivery interface for other machines. The first adapter is
Firestore, reusing the existing paired native channel, encryption, operation journal
and receipt owners. This contract grants no authority and creates no transport,
pairing flow or delivery ledger. Core's prepared-message v1 contract is unchanged.

`rust/tmt-remote/src/squads` owns the types, projection and interface. `reads` owns
the bounded public Ops invocation. `cloud_inbox` and `cloud_pairing::service` own
carrier and original-operation observations. This module starts no publisher or public
command; consumers provide verified retained summaries.

## Summary bytes

One encrypted summary has this JSON shape:

```json
{
  "version": 1,
  "machineName": "Studio",
  "observedAt": 1700000000000,
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
returns `REMOTE_SUMMARY_UNAVAILABLE`; it is never an empty success. An explicitly
empty `squads` list is valid. `Summary::read` rejects incompatible versions,
malformed identity/name/shape and duplicate JSON keys. Names are bounded text
without controls or bidi-control characters. Named limits live in `limits.rs`;
overflow refuses instead of truncating identity. Received unlisted fields are
removed before any subsequent serialization.

`observedAt` is Unix milliseconds, sampled after the public read succeeds. A
viewer computes age; a future timestamp cannot prove freshness. It must not replace
observed time with its local read time or persist an online/offline flag as fact.
The unchanged-summary heartbeat is five minutes; changed allowed data is coalesced
and published promptly by the summary publisher. Its traffic shares the existing
Firestore project guard and has no separate quota allowance.

## Consumer interface

`Interface` has exactly three operations:

- `list`: return a versioned list of `{remote, summary}` entries for other machines.
  The source port supplies verified retained summaries and existing local routing
  metadata. The Firestore adapter filters its own machine UUID, bounds the list,
  validates summaries and refuses conflicting routing keys. A failed source is
  unavailable, not an empty list. Machine identity and the local `remote` selector
  are transport/routing metadata; they are not added to the synchronized summary.
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
by the delivery projection.

| Observed evidence                                         | Interface state |
| --------------------------------------------------------- | --------------- |
| Exact stored carrier, no stored recipient response        | `queued`        |
| Original Core `messageState` is `sent`                    | `queued`        |
| Journal is `held`, or Core is `batched`                   | `held`          |
| Core is `delivered` or `working`                          | `delivered`     |
| Core is `answered` and its final is retained              | `answered`      |
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
states and no additional dispatch. Publisher lifecycle, quota measurements and
two-machine public CLI acceptance are outside this interface qualification.
