# Refresh worker and token meter

## Refresh worker

- One refresh thread loads snapshots off the input loop, collapsing queued requests, so keys act
  on painted data. Results carry their generation and cannot replace a newer view. The worker
  owns one never-reset stop flag per generation; preemption and shutdown set it while the
  generation counter still fences events.
- Cancellation kills and reaps the child process group in the shared `tmt-invoke` bounded process
  owner, without changing ordinary command deadlines or output bounds. Shutdown cancels the
  in-flight core read, disconnects requests and joins after terminal restoration.
- Timer: the input loop requests a reload at the shown squad's `refresh` interval, which each
  snapshot carries, so a squad that failed to load retries at the default. The timer always runs.
- Change detection: between requests the worker checks `board::changes` every `CHECK_EVERY` (1 s)
  using core's `changes.cursor` (public extension API method) and the invocation-selected config file's modification time (`ops.toml` after cutover)
  and length. When either moved since the stamp taken just before the last load, it reloads that
  squad early unless its `refresh` is off. A failed read is never a change, and
  `API_INPUT_INVALID` (a core without the method) stops cursor reads for the session, leaving the
  file check and the interval. A field-provider save moves the cache directory's stamp, which
  `board::changes` also watches.
- Providers: the board hands each load's members to one fetcher thread that runs due provider
  work off the paint path and again at the shortest `every`.
- Startup: `board::run` enters the terminal and paints a placeholder (`startup.placeholder`)
  before it runs any command. It then spawns the worker, so its setup reads
  (`worker.caller`, `worker.Config::locate`) overlap the config read and the
  background-colour query. The first tab is requested only after
  `look::configure_background`, because loads build looks from that process-wide signal.
  The query is bounded terminal I/O; the config read is one Core call. A startup error
  drops the terminal guard, which restores the screen before the error is reported.
- Input and render thread: `board::session` holds `runner::forbid_commands()`, so a
  command started on it panics in debug builds and tests. Keys, clicks and frames never
  wait on a command or a lock. A user action (send, jump, open, copy, tab order, cron
  control) and the config read behind a picker go to the single `board::lane` thread as
  a `Job`; the board shows `Sending…`-style progress at once and applies the returned
  `Acted`, `Opened` or `TokenWindowSaved` event. Jobs run in order, and a composer
  refuses a new send while one is in flight so no text is lost. Picker and overlay
  saves are bounded local file writes through the compare-and-set config writer and stay
  on the session thread; they start no command.
- <a id="extension-labels"></a>Extension labels: a dedicated `labels::Reader` thread, started by the
  board next to the refresh worker (never inside it, so a slow extension cannot hold up a
  reload), asks each `[labels] sources` entry `tmt <name> status --json` through `Core` with
  a 5 s limit, one read in flight, every 5 s. A document that is not version 1, has no
  member list or has a malformed row leaves that source or row unavailable; unknown additive
  fields are ignored; a label's `choose` action is checked by `labels/action.rs` and carried with the label. A failing source backs off to 60 s; its
  last good rows stand in for 30 s, then drop. The reader sends `BoardEvent::Labels` only
  when the combined result changed, `App` replaces `labels` wholesale, and painters read
  it through `board/row_chips.rs`. Supplied text never depends on the clock, so only the
  digest policy chip feeds `header::time_marks`. A `Refresh` handle (`Reader::refresher`) makes the thread read at once and restart its wait;
  the lane uses it after a successful digest change. Dropping the reader cancels the read in
  flight and joins the thread.
- Squad enrichment: a squad tab publishes without its digest-policy read and reply-body
  reads. It applies the digest rows and bodies this worker already read (`Known`), so a
  reload never blinks them off. The first deferred job (`EnrichJob`) then makes one
  `digest.policy.show` read and the missing `requests.show` reads, and sends `Enriched`.
  `App::apply_enriched` sets digest exactly as read (a failed read removes it, as before)
  and fills only replies still without a body. It acts only on the shown, settled squad.
  A fully enriched view equals the former synchronous load; a body-read failure no
  longer fails the view.
- Tab attention: the refresh computes attention for the shown squad from its document and
  publishes that view first. The same worker then computes every other squad's attention from a
  roster-only document (one `rooms.roster` read each plus one `inbox` read shared by all, no `ls`).
  Previous tab attention stays visible until that generation's update arrives, and a newer switch
  preempts this lower-priority work. The cross-squad leads/all views read the rosters needed for
  their own rows before publication.
- HOME exchanges: after publishing HOME, `board::home_leads` reads one bounded
  `requests.list` originator results page and one bounded recipient-history page per
  distinct current lead UUID (50 items each). Other tabs schedule no exchange reads.
  Inbox questions reuse the acquired `waitingOnYou` projection; acknowledged finals
  remain results. Ordering uses submission time for replies and preparation time for
  asks. The selected `Kind::Question` is also HOME's ◆ mark source: those questions
  come first, oldest first. Other exchanges follow, newest first. Undated exchanges
  follow dated ones within each group; both use stable lead UUID/squad ties.
  Leads without any exchange follow both groups
  in name/squad/UUID order. Read, reconcile and replacement share this ordering.
  Truncated pages and read failures remain
  explicit evidence; no room scan or unbounded pagination fills gaps. Observations
  belong to the user's UUID and current lead occurrence, and share the refresh
  generation's cancellation. Preview text is sanitized separately from retained bodies.
  The same results page supplies HOME member reply metadata. Expanded row replies
  and the full-reply reader schedule `requests.show` on that same worker through
  the selected-read queue. The user/recipient/request/kind key and selection
  revision fence delivery; detail verifies both participants before displaying
  the retained body. `board::row_detail` drops late bodies for collapsed or removed
  targets and prunes expansion/cache entries when their rows disappear.
- Priorities: full loads outrank selection jobs (detail notebook or row reply) and usage-only reads.

## Display snapshot cache

`board::snapshot_cache` owns the disposable last fresh base display per tab at
`<dataRoot>/ops/cache/board/<filename>`; the root comes from the existing public
`storage.root` resolver. It adds no Core state or action authority. The refresh
worker publishes its event before root discovery, serialization and file I/O,
then writes before deferred enrichment. Failed/partial views, failed publication
and cancelled generations preserve the last good file.

The same worker offers each tab's stored display once per board, right after the
squad inventory read supplies the room fence and before the tab's views load
(`BoardEvent::Cached`). `App` keeps it only for the current tab while that tab has
no fresh view (opening, or a switch with no visited view) and drops it on any
result for the tab or on leaving it. `view::cached` paints it as inert text with
the shared `tmt-tui` status slot (`<tab> · cached <age> ago`, spinner, steady
`[busy]` under NO_COLOR). It is never a `View`: no row is selectable and
`App::perform` refuses every live action with the loading notice until fresh data
lands. A same-tab refresh keeps the cursor on the same member id when rows reorder.

Writes require current migration paths and an existing owned `ops` directory.
Legacy/deferred paths or a missing `ops` directory silently skip caching. The
cache creates only its `cache/board` children; atomic publication cannot recreate
missing parents or change the migration cutover decision.

Version 1 explicitly projects tab order/pinning, squad-key/room-UUID inventory,
waiting/blocked tab counts, section titles/order and basic member/lead headings
(name, squad, state, task, and a boolean decision mark). HOME
instead retains summary/squad/member counts and blocked rows with names,
squad labels and optional observed age. It omits grid/configuration, arbitrary
fields/providers, presence/role, model/usage history, digest, action bindings/targets, identity
UUIDs, panes/ttys, requests/receipts/bodies/previews, annotations, notes, checklists
and deferred exchanges/cron. Fresh acquisition supplies omitted details.

HOME `@all` maps to `home.json`, `@leads` to `leads.json`, named squads to
`squad-<name>.json`, and user `@tab:<name>` keys to `tab-<name>.json`. The existing
1–24-byte name grammar and reserved user-tab names bound disjoint path components.
The envelope has `version`, `writtenAtMs`, `tabKey` and the full sorted squad/room
map. Read-only load returns a miss on an old/future schema, unknown fields, wrong
tab, any squad/room-map change, missing/corrupt/oversized content or unsafe paths.
Age does not invalidate a display: every loaded value is labelled stale and never
supplies action authority, routing, sender identity or live observation evidence.

A complete serialized snapshot is at most 1 MiB. An oversized candidate skips
publication rather than truncating it; reads acquire at most the bound plus one
byte. Cache directories are 0700 and files 0600, with owned real-path admission;
the trusted root and existing Ops directory permissions are preserved. Existing
unsafe cache paths are skipped without repair. Publication reuses `cache::replace`
with exclusive process/sequence temporary names in the destination directory,
then rename: the last successful rename wins and readers see complete files.
Failures clean only that attempt's temporary. All cache failures are silent to
the user and cannot change the board's result. This is disposable storage, not a
crash-durable transaction or a lock/merge store. Back/provider/staleness cache
paths, document shapes and existing-directory modes retain their own behavior.

## Load timing trace

`TMT_OPS_TIMING_TRACE` enables board-only diagnostic JSON Lines. Unset, empty or
`0` disables it; `1` writes to stderr; any other value names an append-only regular
file (created with mode 0600, symlinks refused). An unusable sink reports once and
disables tracing; it does not fail the board. Trace records never go to stdout.
Disabled tracing performs no timing clock reads, record formatting, sink operations
or extra Core reads; the runtime opt-in requires one startup env check and branches.

Names below are stable for baseline comparisons, including #2125. Version 1 fields
are `version`, `pid`, `load_id`, `generation`, `tab`, `event`, `stage`, `duration_us`,
`elapsed_us`, `status` and `deferred_pending`. Times are monotonic microseconds;
`elapsed_us` starts at board entry before initial config/selection/terminal setup.
`status` is `ok`, `error` or `partial`; no error bodies, requests, receipts, notes or
row contents are recorded. `load_id` is process-local, increasing from 1;
startup uses 0 and null generation. Generation is the existing refresh fence.
A default-tab acquisition can have null `tab` on early stages; its total/milestone
records identify the resolved tab. `deferred_pending` on the total/milestone records
means existing enrichment, attention, history, cron or HOME exchanges remain scheduled.

| Event          | Stage names / boundary                                                                                             |
| -------------- | ------------------------------------------------------------------------------------------------------------------ |
| `stage`        | `startup.placeholder`: the placeholder frame, drawn before any command                                             |
| `stage`        | `startup.Config::load`: initial config in `board::run`                                                             |
| `stage`        | `worker.caller`, `worker.Config::locate`: refresh-worker setup, overlapping terminal entry                         |
| `stage`        | `Squad::list`, `Config::load`, `load`: `board::refresh::load` acquisition and total                                |
| `stage`        | `observe::observe`, `observation.document`, `squad_view`: named squad acquisition (digest and bodies are deferred) |
| `stage`        | `snapshot_cache_read`: the stored display offer after the inventory read                                           |
| `stage`        | `snapshot_publish`: fresh event send; `snapshot_cache`: subsequent best-effort root/serialization/write            |
| `stage`        | `all_view`: complete HOME base acquisition/projection                                                              |
| `first_frame`  | `draw`: first successful terminal draw, including a loading screen                                                 |
| `cached_board` | `draw`: first successful draw showing a stored display, before any fresh board                                     |
| `fresh_board`  | `draw`: first successful draw after an accepted current-tab snapshot                                               |

Stage durations include their called work and stop before trace serialization;
outer stages include enabled inner trace emission overhead. Failed Result stages
emit `error` before returning the original error. Digest enrichment remains best
effort, so its successful timing does not attest available digest evidence.
HOME uses its own roster projection; named-squad stage records are not synthesized
for it. Both milestones are emitted once per UI process, after the normal draw
returns successfully. Failed, cancelled, wrong-tab and retained/cache-only views
cannot produce `fresh_board`. A partial snapshot is explicitly labelled.
Freshness describes the base board; existing deferred reads are excluded, and
`deferred_pending` distinguishes it from a fully enriched view. These are UI-entry
measurements, excluding the shell/core extension dispatch before board entry.

Use one binary/profile, terminal size and starting data/cache per comparison, with
three fresh UI processes on HOME and a representative squad. Record rows, all runs
and ranges alongside stage durations; do not use timings as CI thresholds. The UI
starts an automatic cron clock. A safe copied-data measurement holds the copy's
`ops/cron/clock.lock` exclusive flock until UI exit, uses private roots and a private
tmux server, and disables executable providers/hooks in the copy. It must verify
no lease publication, sends or owned process leaks. Label copied-data measurements
and their presence/host-probe limitations. Compare with three live non-UI
`tmt ops sq ls --squad <name> --json` and `tmt ops sq ls --json` invocations, which
start no clock or sends; Core list reads retain their ordinary presence reconciliation.
Do not subtract unlike UI/list totals as an exact host-probe cost.

## Token window meter

The meter shows completed-request token totals for the visited squad. Input plus
output counts cached input once. Mixed providers sum reported token units, not
cost or text volume. `board::rate` owns evidence, `board::meter` presentation and
`App` the effective configured window.

- Acquisition: `board::rate::Input` captures roster UUIDs and public `resume`
  values before section shaping. On named/HOME entry, the existing cancellable
  worker publishes the usable roster before acquiring history. It batches
  `consumption.history` for at most 32 UUIDs per call, requesting one longest
  window capped at the API's 1 h limit. The worker may reuse validated seeds
  only with the same successful core change cursor, closed five-second range
  and requested window; missing UUIDs are fetched, failures are not cached, and
  any unknown/changed cursor or time range discards reuse. This bounded worker
  cache does not replace interval freshness for panes or notebooks.
  A new generation reacquires history even when queued tab switches coalesce
  back to the previous name; ordinary same-generation reloads do not acquire it.
- Publication: the same worker serializes roster, history and a fresh public
  usage observation. History never replays the opening snapshot's older counters.
  One global `ls` supplies the post-history observation, replacing the first
  scheduled usage poll. Delivery checks cancellation, displayed owner, room UUID,
  settings and exact member UUIDs before seeding the existing Rate rings. A failed
  observation preserves historical evidence with a gap; it cannot imply zero or
  continuous coverage. The usable roster remains interactive while usage updates.
- Seeding: closed deltas replace the authoritative recent region on re-entry,
  preserving older board observations in the same owner. The included `latest`
  driver/session/epoch/sequence and cumulative counters, rather than `throughMs`,
  anchor live subtraction. Shorter windows overlap and are never added again.
  Whole rollups crossing a shorter window boundary are excluded rather than
  prorated. Missing or failed acquisition means unavailable evidence, not zero,
  and cannot bridge current cumulative counters.
- Live sampling: a named-squad load carries public counters; idle reads use only
  `ls --room --json` for those UUIDs on separate 5–10 second deadlines, without
  providers, notes or staleness publication. Full loads take priority, but
  sampling keeps its own due time (`serve`): a reload of the same meter and
  interval leaves it unchanged, and an idle check runs a due sample before it
  decides to reload, so a team whose Core changes every second still samples;
  usage shares generation cancellation and shutdown ownership. `App` accepts counter
  receipts at the configured cadence and never treats cached tabs as fresh
  evidence. It retains meters for visited squads, pruned against visible/hidden
  tabs; leaving a tab closes continuity. Meter state is separate from folds/panes.
- Evidence: each reporting UUID owns a bounded ring of 5 s buckets sized by the
  longest configured window, up to 24 h. Receipt time advances monotonically from
  a UTC anchor. `Rate` validates cumulative input/output/cache-subset,
  driver/session/epoch and sequence/time order. Missing, invalid, gap, decrease,
  new-session or recovery evidence establishes a baseline without invented
  tokens. Removing a roster UUID drops its history. Failed reads close continuity
  and mark gaps after two sampling periods. Provider `observedAtMs` is order
  evidence, not a heartbeat. Retained model attribution uses the current observed
  session model and is best effort.
- Configuration: team preset, global `[board.token_rate]` and per-squad keys layer
  through `config::TokenRate`; Team alone defaults on. `[board] tok` and per-squad
  `board.tok` select three distinct ascending whole m/h durations from 1m through
  24h, default 1m/5m/60m. Both layers validate even when masked. The config reader
  reports the winning setting path and preserves explicit column titles.
- HOME projection: global `board.tok` windows live in the retained HOME model;
  tiles and member grids keep squad overrides. `App::home_header_usage` joins only
  shown sampling squads from existing roster names and meter UUIDs, with no reads.
  It queries actual durations, so a shorter retained ring remains partial in a
  longer global window. Duplicate UUIDs count once per window: prefer more verified
  bucket evidence, then complete readings and longer spans; equal evidence keeps
  displayed squad order. The global longest-window total is the shared denominator
  for the top member and current-model shares. Missing readings contribute no
  tokens, increment unreported once, and make totals/shares partial; a measured
  zero stays numeric but has no share denominator. Current model attribution is
  best effort. The header painter consumes this typed projection only.
  `home::paint::usage` formats the global windows and shares, using shared model
  family names and grapheme-safe fitting. `view::render_frame` reserves one row
  below counts at the shared MD breakpoint (100) only when observations admit it,
  painted through Strip. The MD row keeps w2/w3 and the top member's share,
  labeled `share <window>:`. LG (140) adds w1, a comma-separated `models` group
  and `N member(s) without data`. The share window is stated once and the top
  member's model is not repeated. No readings means no usage row, including
  warmup; measured zero admits it without a share denominator. Narrow widths,
  disabled sampling and search with no shown sampling squads keep their height.
- Provider limits: `board::limits` owns the HOME limits row's data and
  `home::limits` its strip. A provider is a driver ID from the listing; no Ops
  code spells a driver (the #440 guard). The HOME usage receipt's one `ls` also
  carries each session's `resume.consumption.rateLimits` when Core has them
  (percent used, so the board shows percent left). For a provider whose
  sessions report usage but no limits (Claude today), the windows exist only in
  a pane's statusline footer: at most once a minute per provider, and only when
  a listed active session has newer usage than the last reading or attempt, the
  worker reads up to three panes with `tmt check <name> --capture-only --lines
12 --json` and parses the exact `5h P% (reset)  7d P% (reset)` form
  (otherwise no reading). Never a plain `check`: it may hand a retained Digest
  checklist to an idle pane, and a Core without the flag leaves those providers
  unread. A reading is dated by its source's own time (the limits' own
  observation; a session's usage observation for a footer), never by the read.
  Samples persist in `$XDG_CACHE_HOME/tmt-ops/limits/samples.json` (10-minute buckets, 8
  days, saved at most every 5 minutes), a disposable cache. The burn is the
  weekly percent left lost per hour over the 6 h before the newest sample inside
  the current cycle (a rise in percent left, or a reset time later by more than
  30 minutes, starts a new cycle) and needs 1 h of samples. A reading older than 2
  h, or whose week has already reset, is shown only as `no reading for <age>`.
  Only windows of about 7 days and 5 hours are shown, so the labels are exact.
  The strip is reserved from the first HOME frame at MD and wider while the usage
  meter is on (`Updating limits…` until the first sample), so a sample moves no
  other row; with no provider listed or ever sampled it gives the row back.
- Coverage: covered readings stay numeric, including measured zero. Known nonzero
  history deltas also remain numeric lower bounds without continuous coverage;
  zero without coverage stays unavailable. Partial coverage, windows longer than available evidence and unreported members prefix
  the known total with `~`. Unreported identities contribute no tokens. A baseline
  alone is not measured zero. Without usable interval evidence, the meter retains its active
  window label and `–`, plus one dim `no usage reported yet` line; member cells
  also show `–`. Built-in all/leads tabs omit the named-squad summary meter.
- Window selection: comma settings > Token window cycles configured windows;
  `token-window` remains bindable with no default key. Both paths post the label in
  the existing board notice, including without data or when the summary cannot fit. Whole hours use
  `h`, so 60m displays as 1h. These paths and clicks on the painted meter group
  persist the shared `board.token_rate.window` through the existing Config writer.
  Explicit squad overrides still win; failed writes leave the displayed window
  unchanged. Reload/reopen resolves config rather than a session precedence flag.
- Pointer presentation: the existing frame hit collection includes the meter group
  and individual bars. Moved events repaint only on target changes; leaving,
  keyboard operation, resize and modal input clear hover. The whole group uses
  selection background. A hovered bar uses text colour and replaces the fixed
  number/label slot with its total and bucket-end age (`now` below 1m, minutes below 1h, then hours).
  Readouts reuse the token formatter and already retained Rate evidence, with no
  acquisition in paint/input; absent evidence stays distinct from measured zero.
- Row projection: `App::project_usage` derives a board-only document from immutable
  public status using the accepted meters for model and three totals. Repeated
  section rows share one UUID history; changed values invalidate only existing
  grid/cell derivations. Retained views keep their owning values while another
  squad loads. TEAM and crew preset TOML own column placement/priorities;
  `ColumnSource` resolves `usage.w1`–`usage.w3` by window index. One-shot
  `ls` has no history: JSON keeps descriptors without usage values and text skips
  board-only columns. Policy changes start fresh observations. The default grid
  reserves a 20-cell TASK minimum. Sampling off hides usage tracks through
  `Rows.hidden_columns` while retaining MODEL. With sampling on, authored
  window-column priority ranks keep the active window longest: inactive windows
  step aside before PR, then model, then the active window. `Rows.select_window`
  reorders those ranks; App applies it and invalidates the grid immediately on
  selection, even when usage values are unchanged. Model width follows content
  up to 8 cells; paint has no meter-specific fitting branch.
- HOME: separate typed `View.home_rate` templates supply each squad roster. While
  HOME is open and any squad enables observation, one global public `ls --json`
  per sampling cycle is indexed once and joined into the same retained meters.
  Other tabs never schedule that read. Switching cancels the existing generation
  and closes continuity. `App::home_usage` exposes the current lead model,
  configured windows, raw lead/squad readings and the lead's share of the longest
  window (normally 1h). Missing stays absent; zero stays zero; partial flags
  propagate to share; a zero denominator has no share. Consumers format this
  projection without sampling or recomputing totals.
- HOME presentation: `home::counters` retains disposable digits keyed to the
  recorded user, exact squad room/lead UUID or header roster, policy and window.
  It reuses `meter::Counter` easing; `App::home_usage` and `home_header_usage`
  remain raw evidence for aggregation, shares, top/model attribution and coverage.
  First usable evidence, unavailable transitions and reduced motion settle;
  owner/window changes get fresh digits. Tab handoff and failed HOME reads discard
  presentation. HOME painters supply admitted token cells after their existing
  column budget and scroll clipping. The header budgets each numeric cell from
  the accepted reading, right-aligning animated digits without moving later
  labels, shares, models or unreported text. A displayed value that cannot fit
  that admitted budget settles immediately; settled formatting stays unchanged.
  Covered composer cells and hidden body
  overlays stop their motion. The existing session tick/wait wakes only for
  changed fitted visible digits, with no public read, independent timer or
  renderer. Missing `–`, measured zero and `~` labels keep their meaning; hidden
  counters settle and re-enter without counting from stale digits.
- Presentation: cubic digits count for at most 600 ms at 250 ms frame spacing;
  retargeting starts from displayed digits, and window switches/reduced motion
  settle immediately. Eight bucket-aligned bars derive from the same rings: blank
  means no evidence, ▁ measured zero, and ▂–█ nonzero. A right-aligned
  number/unit/window/trend group reserves a fixed 24-cell readout slot when bars
  fit, accommodating both live totals and slice total/age without shifting. It
  drops the trend, then shortens the unit. It preserves the selected label by
  clipping lead/attention text when necessary; it hides only when the terminal
  cannot fit the compact meter itself. Its status row stays reserved while enabled
  so coverage transitions do not move member rows. The new unavailable-state
  text uses the admitted `tmt-tui` strip painter. Help explains completed-request
  window totals, best-effort/excluded/partial semantics, effective switch keys and
  cycle order, and the global/per-squad `tok` setting. The normal cached renderer and ratatui
  diff own output; meter-only ticks stay in the meter band. No parallel paint,
  worker, core dependency, provider-file read or Squad persistence is introduced.
