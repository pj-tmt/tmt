# Digest tick contract v1

`tmt digest tick` is the shared entry point for Ops, launchd and cron. Invoke it
once a minute. The extension remains unpublished until activation.

The process discovers Core's public `storage.root`, takes a nonblocking exclusive
lock at `<dataRoot>/digest/tick.lock`, and holds it through the run. Another tick
exits successfully without reading settings or changing policies. A held lock is
released by process exit; the lock file is retained and has no lease or PID meaning.
The directory and newly created lock file use private permissions.

Each admitted run ends at the next wall-clock minute boundary. A monotonic budget
prevents a wall-clock rollback extending its lifetime. No new Core call starts
after the boundary; an already-started call can finish within its existing
15-second bound before the process exits and releases the lock. The next minute's invocation resumes from current Core
observations, without a last-sent timestamp or persistent scheduler state.

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

Tick success produces no stdout. Invalid settings, unavailable Core, lock errors
and incomplete Core responses return a human error and nonzero status. There is
no long-lived service or board dependency.
