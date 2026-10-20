# Board cron surface

`board::cronboard` owns board-side acquisition, projection, controllers, forms,
modal list painting and scoped hints. It reaches jobs through `cron_service`, manual
sends through `cron_clock::send_now`, and clock status through `cron::Clock::status`.
[data-and-state.md](data-and-state.md#cron) owns storage, actor/revision admission,
dispatch and the independent clock lifecycle. The shipped Squad skill owns
[board job controls](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#cron-on-the-board).

## Acquisition and placement

The refresh worker queues `Deferred::Cron` through the existing generation and
cancellation lane and publishes `BoardEvent::Cron` to `App.cron`. One read includes
active squads, next slots, clock status and actor. Failed reads retain prior jobs
with their reason; paint and input acquire no job data. Clock location resolution
uses the invoker's socket and caches failures, falling back to pane ID.

The workspace's Timers action opens `Overlay::CronList`. A selected squad supplies
its frozen room UUID; a member supplies its owner UUID and, in Agents, the observed
room UUID. The flat list's owner counts open the same owner's all-room list. Counts derive
once from the admitted job snapshot, deduplicating room/job keys; on with a valid
owner is active independently of clock state. A failed read makes counts unknown,
while retaining prior jobs with the failure. Empty global owner lists cannot invent
a room for creation. The list filters UUIDs rather than mutable names, preserves `<room UUID>/<c-id>`
selection across refresh and uses the shared FocusStack/picker surface. Its title
uses the already acquired name for that exact room UUID, even with no jobs. An
unobserved room is labelled by UUID; no room scope says `all squads`, with any
owner filter named separately. No
name becomes filtering or operation authority. Opening forms or delete confirmation closes the list; pause/resume/send retains it.

Cron keys belong to the focused list, not the workspace tree. Enter selects the
selected job's observed room UUID in Agents without a named-tab reload. A room
hidden by filtering or no longer observed refuses navigation and keeps the list;
a reused squad name cannot redirect it. `n` creates, `e`
expands, `E` edits, `p` pauses/resumes, `x` sends, `o` reassigns and `d` deletes after
confirmation. The workspace entry uses effective user bindings; the modal owns
its scoped keys. `cronboard::hints::KEYS` supplies its footer and hides edit without a
selected job.

Ops membership removal pauses source-owned jobs durably before leaving the room.
Move pauses before its dependent source leave; failed destination admission does
not remove the source membership. Positive owner absence also persists a pause;
failed observations remain unknown. Re-add/reassign never resumes a paused job.
Only explicit Resume clears pause. Human list/show and board state distinguish
`paused · owner missing`; JSON retains `ownerMissing` and `stateLabel`.

Squad rename keeps the room UUID and c-ids, moves jobs' squad field and the counter
namespace under the store lock, and preserves pause and counters after all jobs
were deleted. A failed second step is an honest repairable partial result; there
is no room-name rollback or automatic retry.

## Controls and row projection

`CronRequest` retains the acquired actor, job key and viewed revision for execution
through the existing effect path; locked service revalidation owns refusal and no
retry. `forms::Draft` preserves exact message text and omits unchanged message/
schedule fields from edits; owner and schedule input are trimmed at submission.
Stored multiline/control-bearing or oversized messages remain intact. Submission
resolves owner names through `identity show`.

The flat list, tree and conversation reuse `overview::cron_counts::Summary`; room
context narrows owner counts in Agents and is never inferred from a display name.
Count clicks freeze the painted UUID scope and reject superseded geometry.
The shared cron projection retains earliest active-slot and next-run detail for
job rows; the retired squad-tab jobs half owns no focus, geometry or controls.

## Idle alert scan

`idle_alert` is the clock tick's second duty: `Watch::advance` runs after `digest.advance` and the
cron pass in `cron_clock::serve`, under the same lease, and is rebuilt when the lease is acquired.
`Watch::due` allows one scan a minute. A clock's first look, a look after more than five minutes
without a tick and a look after the clock moved back only record the time, so members are judged a
minute after a wake, never at it. A scan re-reads `ops.toml` (settings change while a clock runs),
lists the squads whose table sets `idle_alert.enabled`, and per enabled squad reads the roster and
one `ls` through `Squad::members`: nothing for a squad with the alert off, no daemon, model call or
stored state. `keep` renews the lease and reports cancellation between squads and before each send.

`assess` is the one rule, shared with `ls --json` (`Observation::document` overlays `squad.idleAlert`
and each row's `idleAlert`): the typed `board::activity` projection of Core's `session.activity`
must be `idle` for at least `after` from its `sinceMs`. Pending text or a case-insensitive `hold`
state excepts a member; the lead is marked but never announced to. `notices` needs exactly one
lead. The operation UUID hashes a fixed domain, the room, member and lead UUIDs and `sinceMs`
(`operation::uuid_v8`, shared with `cron::tick::operation_id`). Core compares the recipient, kind,
origin and body; the body names the squad, member, idle start and threshold, never the scan time,
and quotes the row line (cut per value and to 600 bytes) as of the first attempt. A retry whose row
differs returns `DISPATCH_IDEMPOTENCY_CONFLICT`, which `send` reads as accepted. `Core::dispatch_queued`
is the one owner of the rest, shared with the scheduled send: an uncertain create recovers with
`dispatch.show`, and the receipt must be the single queued item for the one recipient. The suggested
`tmt focus`/`tmt talk` lines carry the member's identity UUID, never its display name. The only memory is the in-clock set of accepted operation IDs that
spares a repeat call each minute; a new clock repeats one call and Core makes it a no-op. Tests run
the real scan against the fake Core of `cron_service::test_support`.

## Verification

`cronboard` tests use disposable service fixtures; workspace App tests cover the
scoped entry/target and native Ops tests cover rename, counters and sticky pause.
[development.md](development.md#checks) owns focused commands and isolated captures;
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline) owns
fixture-change approval. Service/clock verification remains with the cron owner.

## Save preview

The existing action lane computes `cronboard::preview` before New/Edit/Reassign
can apply. It uses the production schedule parser and `next_after`, with an injected
instant and at most 4,097 next-slot calls: the first 4,096 invocations can be counted
and one further slot determines the explicit capped result. The next three
occurrences and rolling 24-hour scheduled-invocation count are not token or money
estimates and remain subject to owner/clock state. Unavailable previews cannot
apply; neither can a clipped or unpainted Apply control. A failed acquisition
labels the clock unknown rather than presenting a retained clock as current.
A generation-fenced review retains the exact request; cancellation or
replacement cannot reopen it when a late result arrives. Apply uses the existing
service's actor, room and revision revalidation, never the preview as authority.
