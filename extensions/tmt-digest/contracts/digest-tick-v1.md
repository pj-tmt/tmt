# Digest tick contract v1

`tmt digest tick` is the shared entry point for Ops, launchd and cron. Invoke it
once a minute after installing the optional Digest extension. Installation does
not schedule the tick; the Ops board clock or an external scheduler invokes it.

The process discovers Core's public `storage.root`, takes a nonblocking exclusive
lock at `<dataRoot>/digest/tick.lock`, and holds it through the run. Another tick
exits successfully without reading settings or changing policies. A held lock is
released by process exit; the lock file is retained and has no lease or PID meaning.
The directory and newly created lock file use private permissions.

Each admitted run ends at the next wall-clock minute boundary. A monotonic budget
prevents a wall-clock rollback extending its lifetime. No new Core call starts
after the boundary; an already-started call can finish within its existing
15-second bound before the process exits and releases the lock. The next minute's invocation resumes from current Core
observations, without a last-sent timestamp or persistent scheduler state. Its separate
[history](digest-v1.md#history) cursor records evidence, never delivery eligibility.

## Settings and deadlines

The [settings contract](digest-v1.md#settings) owns resolution and attribution.
`off` means no Core hold, and so does every member while the [global switch](digest-v1.md#global-switch) is off. Interval and `auto` modes create or renew a Core hold each tick, so non-urgent automatic arrivals enter Core's checklist.
The hold expires at renewal time plus the greater of the interval and 120 seconds; an `auto` member has no interval, so its hold lasts 120 seconds.
An unchanged active policy is renewed once per invocation; a changed setting or
inactive policy is reconciled again. If ticks stop, the hold lapses within that
window and normal Core delivery resumes on the next admitted opportunity.
The extension is the policy owner. The owner Core pins is the one whose messages
pass the hold, so a new policy uses the recorded setter of the member's effective
setting for both IDs: a member override's own setter, or for a member that inherits,
the [global default's](digest-v1.md#settings); the member itself when none is
recorded. Existing Core owner/setter IDs are preserved, except that an ID naming
the member itself is replaced once the settings record a setter, which the next
tick writes. This policy attribution does not fill an unknown settings setter. Core separately lets a person, an identity marked with the reserved `user` metadata entry, through every hold whatever the owner.
A revision conflict is an error, never permission to repeat the same write.

Interval delivery becomes due when the oldest held item has waited its configured
interval or the held count reaches `flushCount`. Empty membership has no message
deadline. The extension reads batched public statistics, derives the oldest arrival
from `observedAtMs - oldestHeldAgeMs`, and never treats Core's `nextEligibleAtMs`
as the configured deadline. Settings and arrivals are refreshed at most one second
apart between work, with sleeps shortened for an earlier in-minute deadline.
Subprocess and serialized delivery time can delay observations.

Immediately before an opportunity, settings and statistics are read again. An
increased interval or a changed flush count can postpone delivery; `off` or `auto`
clears the hold and offers normal Core delivery. No last-sent state changes empty
intervals or restart deadlines.

## Auto delivery

An `auto` member's hold lets Core keep its automatic messages while the agent is busy.
Three parties deliver them; only the cap has settings (`maxWait`, `maxCount`).

- **Turn end: Core and the provider.** At an admitted turn boundary Core claims
  everything held and the provider hook delivers it once as one checklist. This
  path does not depend on the tick or on an active policy.
- **Idle offer: the tick.** Idle delivery claims only the range marked due, so for
  an `auto` member holding messages the tick calls `digest.checklist.dueNow` and
  then `digest.checklist.flush` when something new is held (a different held count
  or oldest arrival) and again once 15 seconds have passed with the same backlog.
  Core answers `not_idle` for a busy member, which stays held for its turn end, or
  `delivered`. A message that arrives while the agent is idle therefore waits for the
  next once-a-second look rather than for a turn end that will not come.
- **Cap: the tick.** When the member's `maxCount` messages are held (default 5), or
  the oldest has waited its `maxWait` (default 5 minutes), the same offer is made with `whenBusy: true`. Core then skips only its
  live-idle evidence ([flush](../../../contracts/extension-api.md#digest-policy-and-checklist));
  the verified live binding and session, approval and host checks, claim, seal and
  settlement all stay. The cap is what keeps a member that never reports idle, or
  has no turn-end hook, from being held for ever; its worst case is an ordinary
  message today. The two caps are `digest.toml` settings of `auto` only
  ([digest-v1](digest-v1.md#auto-caps)); `flushCount` stays interval-only.

Urgent sends and the policy owner bypass the hold as for interval mode. An uncertain
or failed offer is reported after the minute and that member is not offered again
within the invocation; the next tick reads current state.

## Delivery and failure

For a due interval, `digest.checklist.dueNow` captures the currently held range;
`digest.checklist.flush` offers Core an idle delivery opportunity pinned to that
UUID, with no name resolution or captured text. Core owns live readiness, channel selection,
claiming, uncertainty and settlement. Busy or approval-blocked members stay held;
a due deadline does not authorize input. Provider-admitted turn boundaries retain
their existing behavior. Later arrivals need another due-range capture on the idle
path. Savings and status projections are separate consumers.

Core flush errors or uncertain delivery are reported after the minute and that member is not retried
within the invocation. Other members can still proceed. An observation or policy
failure is reported; no hidden fallback or direct transport is attempted. A new
tick independently reads current policy/checklist state; an uncertain sealed
checklist remains Core-owned and never becomes a replay lease.

## History collection

The tick reconciles retained settlements once when each member is first observed
in the invocation, and after its own flush opportunities. Core provider/talk/check
settlements between these observations are collected by the next tick. Status
exposes the last reconciled watermark and flags newly observed uncollected
successful deliveries. Collection runs within the same minute deadline, including a bounded public
`consumption.history` read for the private minute snapshots and latest completed
model used by [valuation](digest-v1.md#valuation). Neither status nor tick reads
provider files or changes Core usage retention. Collection errors
are reported without marking the member delivery-failed or retrying delivery.

## Decision records

When the history store is available, the tick records why each member was or was
not offered delivery, so a day of decisions can be listed (`tmt digest decisions`,
[contract](digest-v1.md#decision-list)) and later recomputed under another rule. A
record is content-free: time, member UUID, mode and interval, flush count, `rule`,
held count, oldest held age and `outcome`. It holds no message text, subject or
sender, and changes no setting, policy or delivery admission.

The rule is decided by the same reading that decides delivery. A member holding
nothing has no decision. An `off` member holding messages is offered Core's normal
delivery opportunity (`off`). An `auto` member is `auto_cap` when its `maxCount` messages are
held or the oldest has waited its `maxWait`, and `auto_hold` otherwise; `outcome` is
Core's answer to the offer, which for `auto_cap` never says `not_idle` for a live
member. `auto_idle_point` is what a tick wrote before `auto` held messages: stored
rows keep it, and no tick writes it now. An interval member
is `flush_count` when the held count reached `flushCount` (also when the interval
has passed), `interval_deadline` when only the oldest held message's interval has
passed, and `not_due` otherwise, including when the fresh reading taken just before
an opportunity postpones what the earlier reading found due. `outcome` is Core's
`digest.checklist.flush` state, which carries the readiness reading (`not_idle`
means the member was busy); a failed or malformed answer is `error` and an uncertain
claim `uncertain`. `not_due` has no outcome because no opportunity was offered.

A record is written only when a member's `(rule, outcome)` or oldest held message
differs from its newest stored record, read once per member and invocation. The
loop re-reads about once a second and ticks run every minute, so a state that lasts
(waiting on a deadline, a busy agent) is one record that holds until the member's
next one, and a new holding episode or a different answer is another. Records are
capped at 30,000 rows (about 4.8 MB, a fifth of the shared raw budget), oldest first,
besides the 30-day retention, so a flapping member cannot push out settlement
evidence. Retention and the byte budget are otherwise the
[history store](digest-v1.md#history)'s. A record that cannot be written is reported
after the minute like a collection failure and never blocks or retries delivery.

## Tick liveness

An admitted run records its start time in `<dataRoot>/digest/tick.json`
(`{"version":1,"lastAtMs":N}`, private permissions, replaced atomically) as soon as it
holds the tick lock, so [status](digest-v1.md) can show whether ticks are running at all.
A failed write leaves the previous record and never fails the tick. The record is
Digest's own state: Ops, the scheduler and Core neither write nor read it, and it grants
no delivery eligibility.

Tick success produces no stdout. Invalid settings, unavailable Core, lock errors
and incomplete Core responses return a human error and nonzero status. There is
no long-lived service or board dependency.
