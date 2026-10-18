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
In phase 2, `auto` and `off` both mean no Core hold. Interval mode creates or renews
a Core hold each tick, so non-urgent automatic arrivals enter Core's checklist.
The hold expires at renewal time plus the greater of the interval and 120 seconds.
An unchanged active policy is renewed once per invocation; a changed setting or
inactive policy is reconciled again. If ticks stop, the hold lapses within that
window and normal Core delivery resumes on the next admitted opportunity.
The extension is the policy owner. Existing Core owner/setter IDs are preserved;
a new policy uses the recorded settings setter for both IDs when known, otherwise
the member itself. This policy attribution does not fill an unknown settings setter.
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
nothing has no decision. `auto` and `off` members holding messages are offered
Core's normal delivery opportunity (`auto_idle_point`, `off`). An interval member
is `flush_count` when the held count reached `flushCount` (also when the interval
has passed), `interval_deadline` when only the oldest held message's interval has
passed, and `not_due` otherwise, including when the fresh reading taken just before
an opportunity postpones what the earlier reading found due. `outcome` is Core's
`digest.checklist.flush` state, which carries the readiness reading (`not_idle`
means the member was busy); a failed or malformed answer is `error` and an uncertain
claim `uncertain`. `not_due` has no outcome because no opportunity was offered.

The loop re-reads about once a second, so a record is written only when a member's
`(rule, outcome)` differs from its last record in the same invocation. A later
invocation records its first decision again. Retention and the byte budget are the
[history store](digest-v1.md#history)'s. A record that cannot be written is reported
after the minute like a collection failure and never blocks or retries delivery.

Tick success produces no stdout. Invalid settings, unavailable Core, lock errors
and incomplete Core responses return a human error and nonzero status. There is
no long-lived service or board dependency.
