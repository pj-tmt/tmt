# Digest extension contract v1

Digest owns individual member settings. It uses public Core commands for identity
and configuration-path discovery; it never reads Core storage or configuration files.

## Settings

`tmt digest <member> <duration>|auto|off|default` resolves a saved identity name or
exact UUID. Values are positional. Digest imposes no squad or lead restriction.
`default` removes the override and its current attribution; the member then inherits
the global default. Explicit `auto` remains an override even when the global default
is a duration. `off` disables the member's configured digest mode.

The file is `digest.toml` beside the global configuration file reported by public
`tmt config show --json` (normally `~/.config/tmt/digest.toml`). Missing files inherit
the shipped defaults. `tmt digest global <duration>|auto|off` saves the global
`default` and records who set it; `flushCount` is TOML-only, with no CLI flag:

```toml
default = "auto"
defaultSetByIdentityId = "20000000-0000-4000-8000-000000000001"
defaultSetAtMs = 1791629000000
flushCount = 10

[members.10000000-0000-4000-8000-000000000001]
mode = "20s"
setByIdentityId = "20000000-0000-4000-8000-000000000001"
setAtMs = 1791629000000
```

`default` and member `mode` accept `auto`, `off` or a positive duration. Durations
use ms/s/m/h/d, or bare seconds; decimals have at most nine fractional digits and
must resolve exactly to at least one
millisecond and at most 9,007,199,254,740,991 milliseconds. `flushCount` is a positive
integer (zero is invalid), defaults to 10, and applies only to interval mode.
Member keys and setter IDs are canonical identity UUIDs.

`defaultSetByIdentityId` and `defaultSetAtMs` follow the same rules as a member's
attribution: the setter is absent when Core cannot identify it, and a write with no
identifiable setter removes a previous one rather than keeping a stale owner. A member
that inherits the default has the default's setter as its recorded setter; a member
override has only its own. The recorded setter is an owner whose messages pass the
member's [hold](digest-tick-v1.md#settings-and-deadlines); with none recorded the
member owns it and only an urgent send passes. A person, an identity marked with
Core's reserved `user` metadata entry, always passes every hold and needs no setter.

`setByIdentityId` is absent when Core cannot identify the setter. `setAtMs` is the
current override write timestamp, a nonnegative JS-safe integer; it is neither
delivery eligibility nor telemetry freshness. Inherited settings have no member
attribution and never borrow a removed setter or invent global attribution.

Writes validate the whole existing document, preserve comments and unknown fields,
serialize extension writers, and atomically replace the file with private permissions.
Invalid input, invalid files and failed Core discovery leave settings unchanged.

The official Digest extension is installed through `tmt extension install digest --skills`.
Member writes save configuration; the [minute tick](digest-tick-v1.md) applies it
and offers Core delivery opportunities. `tmt digest skill` prints its bundled
usage guidance; settings writes do not install or start it.

## Status document

`tmt digest status --json` produces this reader contract. The frozen
[status vectors](vectors/status-v1.json) pin its ready-made labels and actions.
Ops and insight call the extension once for all agents; they do not read Digest
TOML/history or Core storage. Digest reads public Core identities once and stats
in batches of at most 256, never one subprocess per member. Core calls share a
15-second command deadline; unavailable stats retain known rows with null evidence.

A successful document has `version: 1`, `observedAtMs` (snapshot assembly time),
and `members`, with one row per identity from public Core identity listing,
including members without overrides. Rows are sorted by canonical `identityId`;
an empty inventory is `members: []`. Names are display text, never action selectors.
Each row contains:

| Field                        | Definition                                                                                                                                          |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `identityId`, `name`         | Core canonical identity UUID and current display name.                                                                                              |
| `mode`                       | Resolved `auto`, `interval` or `off`, following [Settings](#settings).                                                                              |
| `intervalMs`                 | Resolved positive duration in milliseconds for interval mode; null otherwise.                                                                       |
| `settingSource`              | `default` for inheritance, `member` for a current override.                                                                                         |
| `settingValue`               | `default` for inheritance; otherwise `auto`, `off` or the current duration, normalized for display/action selection.                                |
| `setByIdentityId`, `setAtMs` | Current override attribution as defined by Settings; both omitted for inheritance, setter ID omitted when unknown.                                  |
| `heldCount`                  | Core held count, or null when unavailable; missing evidence is never zero.                                                                          |
| `statsObservedAtMs`          | Core observation timestamp for the held snapshot, or null when unavailable.                                                                         |
| `nextSendAtMs`               | Configured interval deadline of the oldest held message, or null without a known interval deadline.                                                 |
| `timeToSendMs`               | Zero when the interval flushCount threshold is reached; otherwise `max(0, nextSendAtMs - observedAtMs)`. Null when required inputs are unavailable. |
| `savings`                    | Null without usable history; otherwise the [savings projection](#savings). Unavailable estimates are never zero.                                    |
| `labels`                     | Ordered ready-made labels defined below.                                                                                                            |

`oldestHeldAgeMs` and `deliveredDigests` are additive nullable row fields from the same Core stats observation. The counter is cumulative Core evidence, not a selected-window total.

`ownerBypass` is an additive row field from settings alone: `true` when the member's recorded setter (its override's, or the global default's for an inheriting member) is someone else, whose messages then pass its hold; `false` when none is recorded, so the member owns its own hold and, besides a person marked `user` in Core, only an urgent send passes it; null for `off`, which holds nothing. It is a flag for a setting that needs attention, never delivery eligibility.

Timestamps and numeric counts are nonnegative JS-safe integers. Deadline arithmetic
that cannot be represented safely is unknown. `nextSendAtMs` is an extension
deadline, never Core's `nextEligibleAtMs`. It is null for auto/off, zero held
messages, or unavailable oldest-held timing. Auto means when the member is idle or its turn ends, and at the latest at the [cap](digest-tick-v1.md#auto-delivery), not a predicted timestamp. A deadline or “Due now” label never proves the agent is idle
or authorizes delivery past Core's admission/approval guards. `flushCount` can make
an interval batch due earlier: keep the scheduled `nextSendAtMs`, but report
`timeToSendMs: 0` and “Due now” when its known threshold is reached. Neither
condition establishes Core eligibility. This matches tick scheduling; the renderer
does not infer a threshold from the held count.

Status is read-only: it does not tick, claim/deliver messages, refresh provider
transcripts, collect history or change settings. Its one outside effect is the
anonymous price-file fetch and the private cache it writes ([Pricing file](#pricing-file)).
Failure of identity/settings
discovery fails the command rather than emitting an empty successful inventory.
Unavailable stats keep the known identity/settings row and use null evidence.
Consumers acquire status off their paint path; an absent/slow extension or failed
command leaves its presentation unavailable and never blocks a frame or tab.
Unknown additive JSON fields may be ignored; unsupported versions or malformed
required fields are unavailable, not guessed defaults.

## Decision list

`tmt digest decisions --json [--since 24h|7d|30d]` (default 24h) lists the
[decision records](digest-tick-v1.md#decision-records) the tick wrote in the
window ending now, oldest first, as one document and keeps no state of its own:

```json
{"version":1,"since":"24h","fromMs":0,"throughMs":0,"decisions":[
  {"atMs":0,"identityId":"<uuid>","name":"worker","mode":"interval","intervalMs":20000,
   "flushCount":10,"rule":"interval_deadline","heldCount":3,"oldestHeldAgeMs":21000,
   "outcome":"delivered"}]}
```

`mode` is `auto`, `off` or `interval`; `intervalMs` is null unless `interval`. `rule`
is one of `interval_deadline`, `flush_count`, `auto_hold`, `auto_cap`, `auto_idle_point`
(stored rows from before `auto` held messages), `off` or `not_due`;
`outcome` is Core's answer to the delivery opportunity (`delivered`, `not_idle`,
`nothing_due`, `unavailable`, `uncertain` or `error`) and is null exactly for
`not_due`. `name` is null when the roster cannot name the member (retired or Core
unreadable). A store without records, or no store, lists an empty array. An
error prints `{"version":1,"error":{"code","message"}}` and exits 1. Consumers ignore
unknown additive fields.

### Delivery report

`tmt digest decisions --report --json [--since 24h|7d|30d] [--recompute INTERVAL
[--flush-count N]]` replaces the list with the savings the delivered digests made, from
the settlements the [history](#history) already keeps (every delivered message's arrival,
the claim and settle times) and, for the rule that fired, the decision records. It
delivers nothing and changes no store. `--recompute` needs `--report`, takes an interval
(not `auto` or `off`, which would need idle times Digest does not keep) and `--flush-count`
defaults to the configured `flushCount`.

```json
{"version":1,"since":"24h","fromMs":0,"throughMs":0,
 "baseline":"each message delivered on its own; turnsSaved is an upper bound, because a busy agent may combine queued messages anyway","delayBasis":"settled minus arrival",
 "members":[{"id":"<uuid>","name":"worker","messages":6,"baseline":{"deliveries":6},
   "actual":{"deliveries":3,"delayMs":{"median":15000,"p90":40000,"max":40000},
             "byRule":{"interval_deadline":2,"unrecorded":1}},
   "turnsSaved":3,"excluded":{"notDelivered":0,"incomplete":0,"clock":0},
   "coverage":{"startsAtMs":0,"collectedThroughMs":0,"complete":true,"gap":null}}],
 "total":{"messages":6,"baseline":{"deliveries":6},"actual":{},"turnsSaved":3,"excluded":{}}}
```

Every output, with or without `--recompute`, carries the baseline (one delivery per
message, so `baseline.deliveries` equals `messages`), `turnsSaved` (`messages` minus
`actual.deliveries`) and the actual delay beside them, per member and in `total`.
`turnsSaved` is an upper bound: without a digest a busy agent may already combine
messages that queue up into one turn, so the real saving can be smaller.
`messages` and `deliveries` count only delivered checklists whose membership is
complete, so each message's arrival is known; the rest are counted in `excluded`
(`notDelivered`, `incomplete`, `clock` for a message that arrived after its settle time),
never dropped silently. `delayMs` is the nearest-rank median, 90th percentile and maximum
of settled minus arrival over those messages, or null. `byRule` counts the actual
deliveries by the decision record that matched each (a delivered record from the same
member written between its claim and two minutes after its settle, each used once);
`unrecorded` is a delivery with none (made before recording began, or by Core itself).
`coverage` shows where the member's evidence begins in the window (`startsAtMs`, the later
of the window start, collection start and the oldest retained settlement), the collection
watermark and a recorded gap; `complete` is false for a late start or a gap. A member with
no coverage row has `coverage` null. A figure is zero or null only when measured so.

With `--recompute`, each member and `total` also carry `recomputed` (`deliveries`,
`delayMs`, `turnsSaved`), and the document carries `recompute` `{method:"arrival-batching-v1",
intervalMs, flushCount, estimate:true, notes}`. The method batches each member's arrivals:
a batch starts at the first unsent arrival and is delivered when that message has waited
the interval, or at the arrival that reaches the flush count, taking every message that
arrived by then; delay is delivery minus arrival. It is an estimate from arrival times
only: Core's readiness wait is not modelled, so a recomputed delay is a lower bound and
the figures compare rules, they do not predict them. It is never billed tokens. A smart
`auto` against idle-point `auto` cannot be recomputed; it is measured actual against
actual from these reports before and after the switch.

## Usage report

`tmt digest usage report` joins Core's consumption, account limits and exchange
flow for all members; [usage-v1.md](usage-v1.md) owns it. It keeps no Digest state.

## Labels

Each label is `{text, colorClass, action}`. Text is plain display text (no markup,
ANSI or shell syntax); `colorClass` is a semantic role from the shared
[design tokens](../../../design/tokens/tokens.json), not a raw color or CSS.
This version uses only `text`, `dim`, `muted` and `waiting`. Consumers render the
supplied text and role; they do not recalculate Digest rules. `action` is null
for a display-only label. There are at most three labels, in this order:

1. Mode/interval: `Auto`, `Every 20s`, `Every 5m`, or `Off`. Auto/interval use
   `text`; off uses `dim`. This chip alone has a choose action.
2. Held count: `N held` with `muted`, only for known N greater than zero.
3. Timing: omit for off or a known zero held count; otherwise
   `Unknown` (`dim`) for unavailable timing/count, `When idle` (`muted`) for
   auto with held messages, `In 20s` (`muted`) for a future interval deadline,
   or `Due now` (`waiting`) when effective `timeToSendMs` is zero. Positive countdowns
   round upward to whole seconds before unit formatting; numeric fields retain milliseconds.

Interval display uses the largest exact unit among d/h/m/s/ms, so 300000 ms is
`5m` and 1500 ms is `1500ms`. Choices are ordered Default, Auto, Every 1m, Every 5m, Every 10m,
Every 30m, Every 1h, saved custom duration, Off. The default label exposes its resolved value:
`Default (Auto)`, `Default (Every 5m)` or `Default (Off)`; its value is always `default`.
A non-preset current custom duration is included as, for example, `Every 20s (custom)`.
Saved custom values, when available, follow the preset choices without duplicates.
The current choice is `default` for inheritance, including inherited auto; an
explicit auto override selects `auto`. There is no flushCount choice or CLI flag.

The Ops UI owns authorization, mouse/keyboard controls and its final `Custom…` entry with
Set/Cancel input (#2152). That entry is UI, not a synthetic `custom` CLI value.
It submits a duration through the same action; clearing submits `default`.
Ops applies squad defaults per member; Digest has no squad concept or UI lead rule.

## Actions

The supplying extension namespace is `digest`, known by the consumer from the
command it invoked, not trusted from a response field. Two tagged shapes exist:

- Choose: `{kind:"choose", options:[{label,value}], current, argv}`. Option
  labels are plain text, values are strings and `current` names an offered value.
  Settings values/durations follow Settings; no TOML parsing or parallel grammar.
- Run: `{kind:"run", argv}`. This executes a fixed action after explicit user
  activation. Run is supported by the seam, but adds no fourth Digest chip.

`argv` is a nonempty array of strings without NUL, never a shell command. Before execution,
consumers require `argv[0]` to equal the supplying namespace exactly; `tmt`,
`ops`, another extension, an executable path or an unknown action kind is refused.
No fallback execution is permitted. Run has no substitutions. Choose replaces
one whole argv element equal to `{value}` with the selected value; embedded,
missing or repeated placeholders are refused. Other placeholders are unsupported.
Member UUIDs are already literal argv elements, never a `{member}` substitution.

The member setting action is
`["digest","10000000-0000-4000-8000-000000000001","{value}"]`.
The consumer spawns `TMT_EXECUTABLE` directly with the resulting array, preserving
each value as one argument; it never joins the array, invokes a shell, looks up a
response-supplied executable or interprets selected text as syntax. A custom
duration from explicit UI input uses this same substitution and Settings validation.
Failures are shown to the user without claiming that a setting was saved or
automatically repeating a possibly effected command.

The vectors include inherited/explicit auto, inherited interval, custom interval,
off, empty and unavailable held evidence, unknown setter, a due deadline and early flush.
Action vectors freeze literal argv expansion, run, and namespace/shape refusals;
they are consumer examples, never commands to run during fixture verification.

## Savings

`status --json --window 24h|7d|lifetime` selects one window (default `24h`).
The additive `savings` object is `{method, window, fromMs, throughMs,
estimatedInputTokensAvoided, avoidedSeparateDeliveries, complete, gap}`.
It also contains additive nullable `cost` and `percent` fields defined under
[Valuation](#valuation). `method` is `context-input-avoided-v1`.
The [normative vectors](vectors/savings-v1.json) fix the event formula.

For a confirmed delivered sealed batch of N original hold references, the
counterfactual is N separate deliveries instead of one checklist. The estimate
sums `contextTokensAtArrival` for references 2..N, in original sequence order;
`avoidedSeparateDeliveries` is N-1. A known singleton is zero even without
context samples. Definitely-unsent and uncertain settlements contribute no
confirmed savings. Missing required member inputs, context or sealed count makes the token
estimate null. Values and sums must be JS-safe; overflow stays unknown.

This estimates gross context rereads, not billed tokens, cost or model turns.
Ordinary delivery can already coalesce notices. It excludes output, checklist
and prompt overhead, caching, compaction, overflow reads and subsequent work.
It is neither causal savings nor a guaranteed lower bound.

`throughMs` is the latest successfully reconciled Core observation, not status
assembly time. Recent windows cover `(throughMs - duration, throughMs]` (clamped
to zero); lifetime covers collection start through that watermark. Lifetime means
**since this identity's Digest collection began**, never all historical TMT use.
Compatible consumer sums require matching method/window and explicit coverage;
missing members cannot silently contribute zero. Ops owns aggregation/rendering.

`complete` is true only for fully known covered evidence. One `gap` reason
explains unavailable/incomplete coverage: `before_collection`, `missing_context`,
`incomplete_membership`, `missing_deliveries`, `uncollected_deliveries`,
`history_evicted`, `clock_discontinuity`, `clock_or_store_discontinuity`, or
`unsafe_arithmetic`. A gap nulls the token estimate; the avoided-delivery count
can remain known for missing context/membership with a known original sealed
count. Gap precedence is coverage/counter/clock before event arithmetic/context.
A covered empty window is zero. Off mode preserves history. Missing, unreadable,
unsupported or invalid stores produce `savings: null`; status never repairs them.

## Valuation

The [normative valuation vectors](vectors/valuation-v1.json) fix quotes and ratios.
Valuation never changes the parent token figure, checkpoint or completeness. Each
valuation is independently null when its required evidence is unavailable; older
producers may omit these additive fields. Consumers must label cost as an estimate
**at list price**, never as money saved or a historical bill.

`cost` is null or `{usdMicros, modelId, modelObservedAtMs, pricingVersion,
usdMicrosPerMillionInputTokens, assumption:"uncached-input"}`. It quotes the
parent `estimatedInputTokensAvoided` at the latest evidenced completed-request
model's standard base uncached input list price. The model comes from public
`consumption.history`'s `latest.consumption`, never the starting/resume model or a
provider transcript. Its timestamp records model evidence, not the savings event's
model. It may postdate the parent savings checkpoint. Missing/unsupported models,
prices or tokens, future model evidence, and unsafe arithmetic produce null;
known zero tokens produce zero only with a supported evidenced model. Quotes round
to the nearest micro-dollar, half upward, using checked wide intermediates.

A successful status document also contains additive root `pricing: {version, asOf,
basis, origin, fetchedAtMs?, models:[{modelId, usdMicrosPerMillionInputTokens,
source}]}`, acquired with the same status call. It names the one table every quote of
that document used ([Pricing file](#pricing-file)). `basis` is `standard-base-input`;
`source` is the official source URL and `asOf` is the verification date. Version
`2026-10-14` includes only canonical `claude-opus-5-5`, `claude-sonnet-5-5`,
`gpt-6-sol`, and `gpt-6.1-sol`. There are no aliases or fallback rates. This fixed
assumption excludes cached input, cache writes, fast/regional/long-context
modifiers, negotiated discounts, subscription allowances and historical price
reconstruction. Updating rates requires a new verified file `version`.

A successful status document also contains additive root `tick: {lastAtMs}`, the
start time of the last tick that was admitted ([liveness record](digest-tick-v1.md#tick-liveness)),
or null when none is recorded or the record is unsafe. It is liveness evidence only:
status never infers a hold, a delivery or an owner from it.

`percent` is null or `{basisPoints, estimatedInputTokensAvoided,
observedInputTokens, fromMs, throughMs}`. It is available only for `24h` and `7d`;
**lifetime percent is always null**. Its interval uses whole closed minutes inside
the parent window: `fromMs = ceil(parent.fromMs / 60000) * 60000`,
`throughMs = floor(parent.throughMs / 60000) * 60000`. Empty intervals are null.
The numerator includes confirmed events in `(fromMs, throughMs]`, separately
from the parent's potentially larger token interval. The denominator is observed
input in minute buckets `[fromMs, throughMs)` plus that numerator. Output is
excluded; cache read/write counts are already subsets of input and are not added.
Every minute must have complete, continuous source coverage with no gaps, and the
parent token evidence must be complete. Missing, expired or evicted minutes are
unknown, never zero. Source totals are snapshots, never cumulative seeds or
guessed deltas. A positive denominator and known zero numerator produce zero;
a zero denominator or unsafe sum produces null. Basis points round
`10000 * avoided / (observedInput + avoided)` to nearest integer, half upward.

Consumers aggregate only compatible method/window and exact interval pairs by
summing their numerators and observed inputs, then calculating the ratio. They
never sum or average percentages, and must expose partial member coverage.
Cost quotes combine only when every member quote has the same `pricingVersion`:
each quote already used its own member's evidenced model, so the total is the sum
of their `usdMicros` list-price quotes. Quotes under different `pricingVersion`s
are never summed, converted or reconciled; the total is unavailable and the
consumer says the prices differ. A sum above 9007199254740991 micro-dollars is
unavailable too. Ops implements this in `sum_costs`
(`extensions/tmt-ops/rust/tmt-ops/src/labels/digest.rs`).
Ops owns the separate display/toggle consumer; this producer adds no UI.

### Pricing file

The price table is one public file in the repository,
[`pricing/pricing-v1.json`](../pricing/pricing-v1.json), with the shape of the
status `pricing` object plus `schema`:

```json
{"schema":"tmt-digest-pricing-v1","version":"2026-10-14","asOf":"2026-10-14",
 "basis":"standard-base-input",
 "models":[{"modelId":"claude-opus-5-5","usdMicrosPerMillionInputTokens":4000000,
            "source":"https://platform.claude.com/docs/en/about-claude/pricing"}]}
```

- **Built-in table.** Digest embeds these exact bytes; they are the built-in table
  and keep no second copy in code. Each Digest release attaches the same file as
  release asset `tmt-digest-pricing-v1.json`, and the release check proves it equals
  the file at the tagged commit.
- **Validation.** A file is used only if all of this holds, else it is dropped: at
  most 16 KiB of UTF-8 JSON with exactly the keys above (none missing, extra or
  repeated); `schema` equals `tmt-digest-pricing-v1`; `version` and `asOf` are real
  `YYYY-MM-DD` dates; `basis` equals `standard-base-input`; 1–64 models with unique
  `modelId` (`[A-Za-z0-9._-]`, at most 64 bytes), integer prices in `0..=9007199254740991`
  and a `source` that is an `https://` address of at most 256 printable bytes.
- **One version, one table.** Quotes with equal `pricingVersion` are summed, so a
  version never changes its prices. A fetched or cached file whose `version` is
  already known (the built-in table's, or the cache's) is used only if it equals
  that table after parsing; a file with an older `version` than the built-in
  table's is dropped, so a table never rolls back. A price change always takes a
  new `version`.
- **Fetch.** `tmt digest status` makes one anonymous HTTPS GET of
  `https://raw.githubusercontent.com/pj-tmt/tmt/main/extensions/tmt-digest/pricing/pricing-v1.json`:
  no token, cookie, authorization header, redirect, retry or GitHub API call, and
  only `User-Agent: tmt-digest/<version>` as identification. The Ops board reads
  status when it opens or reloads, so those are the fetch moments; the tick, the
  usage report and every other command make no request.
  The request runs while Core is read, has a 2-second deadline from DNS to the last
  byte, accepts only a 200 answer of at most 16 KiB; status waits at most 2.5 seconds
  for it. Attempts are at least 30 seconds apart, and a failed one counts, so a
  dead network costs one wait per half minute.
- **Fallback order.** The fetched file, then the cache, then the built-in table;
  each is used only if it validates and follows the previous rule. A failed fetch
  leaves the cache unchanged.
- **Cache.** `digest/pricing/fetched-v1.json` under Core's data root: a private
  directory and file (owner-only, no symlinks, at most 64 KiB), replaced atomically.
  It holds the last valid fetched file, `fetchedAtMs` and the last `attemptedAtMs`.
  It is untrusted on read: the file is validated again, and any doubt means no cache.
- **Output.** `pricing.origin` is `fetched`, `cached` or `built-in`, and
  `fetchedAtMs` (the fetch that produced the file) is present unless `built-in`.
  Every quote of a document uses that document's table and says its version.
- **Privacy and opt-out.** Only the machine's address, the fixed URL and the
  User-Agent reach GitHub; no identity, model or usage data. Setting
  `TMT_DIGEST_PRICING_URL=off` turns the fetch off (the cache and the built-in table
  still serve). The variable may instead hold another `https://` address, or `http://`
  to this machine, which tests use; any other value also turns the fetch off. There
  is no `digest.toml` key for it.

## History

The existing [tick](digest-tick-v1.md) owns collection through public
`digest.checklist.history` and `consumption.history`; it never opens Core storage
or provider files.
Each identity's first successful collection atomically records the observed
sequence/counter as its baseline and start time; prior settlements are excluded.
This avoids presenting pre-collection activity as lifetime savings.

Subsequent terminal events and their cursor/lifetime totals commit in one
transaction, deduplicated by checklist ID and monotonic settlement sequence.
Counter reconciliation at the end of enumeration detects missing delivered
history; counter deltas never reconstruct missing inputs or settlement times.
Interrupted reads/writes resume from the durable cursor. Collection failure
never offers delivery or changes Core's uncertainty/admission rules.

Private evidence lives at `<dataRoot>/digest/history/history.sqlite` (public
`storage.root` discovery), schema version 3. Read-only status accepts schemas 1 and 2 (usage
valuations are null before 2, decision records absent before 3) and never migrates
them; the next writer atomically adds the missing tables and advances the version
while preserving settlement cursors and lifetime totals. Digest admits
its private file/schema and DELETE/FULL policy, then transfers the writer
connection to the shared [Store](../../../rust/crates/tmt-extension-state/src/lib.rs).
Every mutation, including initialization, uses one BEGIN IMMEDIATE with at most
five seconds of admission; callbacks run once. Rollback-journal COMMIT uses only
the remainder of that same window. A timeout keeps the existing
`DIGEST_HISTORY_IO` envelope and states whether the change was not started or may
not be saved, with `tmt digest tick` as the retry. Read-only status and reports keep their own
read-only connection and never adopt a writable Store. Directories are 0700, files 0600;
nonprivate paths, symlinks and unsupported schema versions are refused. Events
retain content-free IDs, settlement/claim/collection timestamps, original arrival
inputs, outcome/opportunity, formula version, nullable estimate and nullable
`decision` evidence for the later decision work. No bodies, previews, receipts,
transport tokens or provider locators. Current collection does not invent a
Digest decision for a Core settlement.

Each history opportunity also reads one ten-minute public usage window per member,
with at most 120 five-second buckets. Only complete whole minutes are retained;
partial edge minutes wait for a later read. Overlapping/restarted reads replace
minute snapshots rather than add them, allowing coverage to improve without
double-counting. Missing more than ten minutes of tick collection leaves gaps;
there is no catch-up reconstruction from cumulative totals. Minute rows and the
source watermark/latest model commit atomically. They store only identity,
minute/collection timestamps, nullable input totals and model provenance.

Tick maintenance expires raw events 30 days after collection and usage minutes
after 30 days of event time, including when no members remain. A shared 24 MiB
raw-input budget accounts for event input bytes, 128 bytes per usage minute and 160 bytes
per [decision record](digest-tick-v1.md#decision-records), which are also capped at 30,000 rows;
it evicts oldest evidence with explicit recent-window coverage loss while
preserving compact lifetime settlement totals. SQLite uses at most 32 MiB, reserving another 32 MiB of the 64 MiB store
budget for its rollback journal. Compact totals/cursors remain within that bound;
capacity exhaustion fails atomically rather than resetting them. One raw event
is bounded to 1 MiB; oversized member histories retain explicit incomplete evidence.
At most four enumeration pages and 32 continuation pages per event are collected
in one opportunity, within the tick deadline. There is no background collector
or off-machine export. Status opens only a read-only snapshot and never collects,
prunes or checkpoints history.
