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
the shipped defaults. Global `default` and `flushCount` are TOML-only, with no CLI flags:

```toml
default = "auto"
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

`setByIdentityId` is absent when Core cannot identify the setter. `setAtMs` is the
current override write timestamp, a nonnegative JS-safe integer; it is neither
delivery eligibility nor telemetry freshness. Inherited settings have no member
attribution and never borrow a removed setter or invent global attribution.

Writes validate the whole existing document, preserve comments and unknown fields,
serialize extension writers, and atomically replace the file with private permissions.
Invalid input, invalid files and failed Core discovery leave settings unchanged.

The extension remains unpublished. Member writes save configuration; the
[minute tick](digest-tick-v1.md) applies it and offers Core delivery opportunities.
No skill or managed installation is shipped by this settings surface.

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

Timestamps and numeric counts are nonnegative JS-safe integers. Deadline arithmetic
that cannot be represented safely is unknown. `nextSendAtMs` is an extension
deadline, never Core's `nextEligibleAtMs`. It is null for auto/off, zero held
messages, or unavailable oldest-held timing. Phase-2 auto means when idle, not a
predicted timestamp. A deadline or “Due now” label never proves the agent is idle
or authorizes delivery past Core's admission/approval guards. `flushCount` can make
an interval batch due earlier: keep the scheduled `nextSendAtMs`, but report
`timeToSendMs: 0` and “Due now” when its known threshold is reached. Neither
condition establishes Core eligibility. This matches tick scheduling; the renderer
does not infer a threshold from the held count.

Status is read-only: it does not tick, claim/deliver messages, refresh provider
transcripts, collect history or change settings. Failure of identity/settings
discovery fails the command rather than emitting an empty successful inventory.
Unavailable stats keep the known identity/settings row and use null evidence.
Consumers acquire status off their paint path; an absent/slow extension or failed
command leaves its presentation unavailable and never blocks a frame or tab.
Unknown additive JSON fields may be ignored; unsupported versions or malformed
required fields are unavailable, not guessed defaults.

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

The offline table is owned by `rust/tmt-digest/src/pricing.rs`. A successful status
document also contains additive root `pricing: {version, asOf, basis,
models:[{modelId, usdMicrosPerMillionInputTokens, source}]}` metadata, acquired with
the same status call. `basis` is `standard-base-input`; `source` is the official
source URL and `asOf` is the verification date. Version `2026-10-14` includes only
canonical `claude-opus-5-5`, `claude-sonnet-5-5`, `gpt-6-sol`, and `gpt-6.1-sol`.
There are no aliases or fallback rates. This fixed assumption excludes cached
input, cache writes, fast/regional/long-context modifiers, negotiated discounts,
subscription allowances and historical price reconstruction. Status performs no
network price lookup. Updating rates requires a new verified table version.

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
Ops owns the separate display/toggle consumer; this producer adds no UI.

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
`storage.root` discovery), schema version 2. Read-only status accepts schema 1 with null valuations and
never migrates it; the next writer atomically adds usage tables and advances the
version while preserving settlement cursors and lifetime totals. Directories are 0700, files 0600;
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
raw-input budget accounts for event input bytes and 128 bytes per usage minute;
it evicts oldest evidence with explicit recent-window coverage loss while
preserving compact lifetime settlement totals. SQLite uses at most 32 MiB, reserving another 32 MiB of the 64 MiB store
budget for its rollback journal. Compact totals/cursors remain within that bound;
capacity exhaustion fails atomically rather than resetting them. One raw event
is bounded to 1 MiB; oversized member histories retain explicit incomplete evidence.
At most four enumeration pages and 32 continuation pages per event are collected
in one opportunity, within the tick deadline. There is no background collector
or off-machine export. Status opens only a read-only snapshot and never collects,
prunes or checkpoints history.
