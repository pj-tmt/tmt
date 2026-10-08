# Board cron surface

`board::cronboard` owns board-side acquisition, projection, controllers, forms,
half/list painting and scoped hints. It reaches jobs through `cron_service`, manual
sends through `cron_clock::send_now`, and clock status through `cron::Clock::status`.
[data-and-state.md](data-and-state.md#cron) owns storage, actor/revision admission,
dispatch and the independent clock lifecycle. The shipped Squad skill owns
[board job controls](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#cron-on-the-board).

## Acquisition and placement

- The refresh worker queues `Deferred::Cron` after full reloads of any tab, with
  the existing generation/cancellation lane, and publishes `BoardEvent::Cron` to
  `App.cron`. One read includes jobs of every active squad (hidden included), next
  slots, clock status and actor. Before its first result nothing is drawn; failures
  preserve previous jobs alongside the reason. Paint/input acquire no job data.
  `list_jobs` resolves owners through one public `references.resolve` per job.
- `cronboard::line` supplies HOME's single `home::CRON` target and pure summary;
  prompts, paths and clock location stay in job UI. Initial failure is blocked,
  later failure retains summary with its reason. `effects::pane_place` resolves
  clock `session:window` once per pane on the invoker's socket; `Places` caches
  failures too, falling back to pane ID. Lease age uses the shared relative-time
  formatter; public clock JSON keeps its original pane ID.
- `composition::halves` places the squad's configured composition above its jobs
  in one flex computation. The jobs half grows to content demand, capped at two
  fifths of body height; below 12 body lines it keeps only its rule. Its ordinary
  TUI list retains per-room `ListState` across tab switches. Selection and focus
  leave jobs collapsed; `e` toggles an explicit stable job expansion in
  `board::row_detail`, which owns the shared block rendering and reconciliation.
  Tab/pointer focus enters through `App`; `focused_pane()`
  is then `None`, and member-row actions refuse.
- `Overlay::CronList` uses the shared `FocusStack`, `app::route` and docked
  `picker_surface::State`. Rows use `<room UUID>/<c-id>` identity, preserved on
  refresh. Opening forms/delete confirmation closes the list; pause/resume/send
  retain it. Job keys route through base field `cron-jobs` before board dispatch,
  while configured `[bind]` keys win. `e` expands/collapses and `E` edits in both
  surfaces. `cronboard::hints::KEYS` supplies help and footers; `E edit` appears
  only with a selected job.

## Controls and row projection

`CronRequest` retains the acquired actor, job key and viewed revision for execution
through the existing effect path; locked service revalidation owns refusal and no
retry. `forms::Draft` preserves exact message text and omits unchanged message/
schedule fields from edits; owner and schedule input are trimmed at submission.
Stored multiline/control-bearing or oversized messages remain intact. Submission
resolves owner names through `identity show`.

Member row-end cron labels and detail match owner UUID, selecting the earliest
active slot. `view::rows` reserves the label only if no additional column hides,
drops cron before age, and includes labels in the cached row scene key.

## Verification

`cronboard` tests use disposable service fixtures; `view/tests/cron` verifies split
placement, focus/scoped keys and forms; `home/tests/cron` verifies the HOME target.
[development.md](development.md#checks) owns focused commands and isolated captures;
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline) owns
fixture-change approval. Service/clock verification remains with the cron owner.
