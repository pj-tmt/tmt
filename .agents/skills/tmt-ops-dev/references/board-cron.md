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
room UUID. Overview owner counts open the same owner's all-room list. Counts derive
once from the admitted job snapshot, deduplicating room/job keys; on with a valid
owner is active independently of clock state. A failed read makes counts unknown,
while retaining prior jobs with the failure. Empty global owner lists cannot invent
a room for creation. The list filters UUIDs rather than mutable names, preserves `<room UUID>/<c-id>`
selection across refresh and uses the shared FocusStack/picker surface. Its title
uses the already acquired name for that exact room UUID, even with no jobs. An
unobserved room is labelled by UUID; only an unscoped list says `all squads`. No
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

Overview, Agents and conversation reuse `overview::cron_counts::Summary`; room
context narrows owner counts in Agents and is never inferred from a display name.
Count clicks freeze the painted UUID scope and reject superseded geometry.
The shared cron projection retains earliest active-slot and next-run detail for
job rows; the retired squad-tab jobs half owns no focus, geometry or controls.

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
apply. A generation-fenced review retains the exact request; cancellation or
replacement cannot reopen it when a late result arrives. Apply uses the existing
service's actor, room and revision revalidation, never the preview as authority.
