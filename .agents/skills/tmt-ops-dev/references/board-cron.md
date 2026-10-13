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
its frozen room UUID; a member supplies that room and prefills its owner when
creating a job. Overview and Common pool have no invented room context. The list
filters by room UUID rather than a mutable name, preserves `<room UUID>/<c-id>`
selection across refresh and uses the shared FocusStack/picker surface. Opening
forms or delete confirmation closes the list; pause/resume/send retains it.

Cron keys belong to the focused list, not the workspace tree. Enter opens the
selected job's squad; `n` creates, `e`
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

Workspace squad timer counts match room UUID; conversation member counts
match owner UUID. The
shared cron projection retains earliest active-slot and next-run detail for
job rows; the retired squad-tab jobs half owns no focus, geometry or controls.

## Verification

`cronboard` tests use disposable service fixtures; workspace App tests cover the
scoped entry/target and native Ops tests cover rename, counters and sticky pause.
[development.md](development.md#checks) owns focused commands and isolated captures;
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline) owns
fixture-change approval. Service/clock verification remains with the cron owner.
