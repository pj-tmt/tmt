# Board internals

`tmt squad board` renders the same status document as `ls` with ratatui over crossterm.
It runs only on an interactive terminal (see the invariants in [SKILL.md](../SKILL.md)).
Paint and input perform no core reads; loading happens on workers
([refresh-and-meter.md](refresh-and-meter.md)).

## Terminal

`board::terminal` owns raw mode and the alternate screen behind a `Screen` trait and
restores on return, error, panic (panic hook) and TERM/HUP (signal-hook). Mouse capture is
part of the terminal state the `Screen` guard restores. The input loop rebuilds only after
a state/input/resize change or when displayed clock text or the delayed spinner changes.
Input, snapshots and deferred tab attention share one event channel; a snapshot wakes the
painter directly.

## Rows and grid

- `rows::Rows` is the row grid (`columns` and `lines`) and owns prefix coverage, including
  empty cells; original span positions survive hiding. A cell optionally carries a typed
  shared `Role`, read from `token` with strict semantic-name validation and published only
  when configured. `markup::row_values` admits that token on each cell and the board
  resolves it through `Look` before projected field decoration. Missing/empty values and
  failed providers without projected colors keep Dim; stale-row inheritance applies;
  `Look::row_span` still overrides cell colors and Dim for reverse selection. Team alone
  opts in, with `waiting` on its pending cell.
- `rows::Column` keeps `grid::Basis` for cell/percent configuration with cell bounds.
  Columns no line covers stay projection sources; their JSON metadata adds `valueOnly: true`
  and `Column::display` ignores their sizing, so flat text lists keep natural values.
  Metadata keeps percent strings and adds `overflow` and wrap `max_lines` only when opted
  in; full row values never change.
- `markup::Grid` compiles the covered tracks and configured spans through `tmt-tui`
  admission and one Taffy grid computation. Squad resolves configured CSS clamp bases and
  selects priority tracks before sizing (priority hiding is Squad's, not Taffy's); growing
  tracks default their minimum to `rows::NARROWEST`. The grid keeps geometry's logical text
  widths and clips for fitting. There is no arithmetic span solver or scalar
  `grid::fit/fit_lines` in the board.
- `rows::ListSizing` picks the text-list sizing policy once from the shown columns: without
  percent/overflow it keeps legacy list sizing and complete piped values. Opt-in text lists
  decode only projected display settings through `Column::display` and use the same grid
  solver and fitter; a pipe's budget is summed natural data widths plus gaps before priority
  hiding, so such lists may truncate, wrap or hide columns. CLI lists keep after-gap
  percentages, largest-remainder rounding and `grid::fit_lines` wrapping; no parallel layout
  engine exists.
- `board.hidden_columns` is carried by `Rows` as named original track positions. The reader
  rejects unknown, duplicate, uncovered and all-hidden masks. `markup::Grid` seeds its shown
  set with the mask before priority hiding and sizing; spans count surviving tracks in their
  original ranges and a cell with zero surviving tracks is omitted. `ls` text uses the same
  visibility; JSON keeps every field value and original column/line metadata and emits
  `hidden_columns` only when nonempty.
- The immutable view owns disposable derivations keyed by effective pane width (and grid
  search): the width/search cache keeps admitted projected row cells and geometry together,
  markdown wrapping caches styled lines by the active look so theme previews repaint them,
  and replacing the view invalidates them. Selection-only frames change styles without
  rebuilding templates or sizing.
- Occurrence IDs contain tab, authored section slot, source squad and member UUID followed
  by static line/column keys; member order is never identity, and UUID-free display rows have
  no actionable IDs. `App::shown_tab` supplies the retained view owner while another tab
  loads, and resize/search never substitute the requested tab.

## Decisions and ask-lead

`attention::waits_on_you` owns the shared pending/request predicate.
`board::view::waiting` selects pending text before the oldest acquired request
preview and formats only authoritative nonfuture request ages. Authored pending
cells keep their grid position; rows without one gain a continuation line with
its own row hit. `time_marks` includes request ages so they advance without reads.
The waiting hint uses `Attention::of`, matching the tab's count, and effective
bindings; narrow fitting removes the oldest-member label before the actions.
The footer reserves `? more` as its final hint before fitting whole tail hints.

`ask-lead` opens the existing input composer with the configured question.
Enter validates the opening sender, squad and current lead, then produces the
existing `Request::Talk`; no extra client or read path exists. Its docked prompt
uses an opaque full-width `tmt-tui::Modal` band and admitted text strips. `board::view::strip` also
owns the converted footer/loading/empty-detail text, leaving their callers'
resolved styles intact. User-facing action and question settings are owned by
the shipped Squad skill.

## Composition and folds

- `split` owns validated row/column trees up to `MAX_DEPTH` 3 and reading/focus order, not
  geometry. `board::composition` admits an embedded version-1 XML scaffold before raw mode,
  then instantiates named prototypes from the validated Board/Split and runtime folds. Folded
  panes reserve one stacked title line or compact side-by-side title width, and fully folded
  groups propagate that footprint. Expanded siblings share the remainder through typed
  percent/grow styles and one Taffy flex computation. Named rectangles dispatch to the rich
  pane painters (notes/replies keep Markdown, wrapping and interaction owners). Tabs reserve
  a shrinkable one-line bar above a focused pane with a one-line minimum. There is no runtime
  file loader or alternate solver.
- Nested percentages use raw fractional parents followed by cumulative edge rounding. The
  tree's reading order is the focus order, skipping folds. The immutable-view cache keys
  viewport, effective Board, folds and tab focus; row selection never rebuilds geometry.
- The configured Board/Split never changes during a toggle. `Config::board` strictly validates
  the initial `collapsed` pane list for split mode and `fold_below = { width, panes }` (width
  1–1000, panes present in the resolved layout); Team sets width 100 for detail and replies.
  `App` resolves the effective fold set from board body width and the immutable defaults;
  per-pane user overrides win at either width. The terminal draw owner supplies the full-width
  body measurement and the view only passes the set to `board::composition`. `App` keeps
  bounded per-tab session overrides: they survive unchanged refreshes and cached switches,
  reset when the board config changes, drop with removed tabs, and are not persisted.
- The `action` owner parses `toggle <pane>...` (one or more unique literal pane names). It acts
  on the named panes present, doing nothing when none are present; if any is expanded it folds
  all, otherwise it expands all, setting each session override. Both host presets bind `d` to
  `toggle detail replies` when the board holds both panes, else the one available pane, else no
  default `d` action or hint; configured and section bindings override the preset. Footer and
  help name the effective panes and state (`detail+replies ▾` when any is expanded, `▸` when all
  are folded); the footer drops the whole hint if it does not fit.
- Each render records visible title hit regions; a left press toggles before row dispatch,
  without selecting a row or joining double-click history. Folded bodies produce no row/scroll
  hits. Collapsing focus moves to visible rows, else the next expanded pane; with every pane
  folded there is no body focus, and expanding from that state focuses the expanded pane. The
  notes action expands notes before focusing it. A single expanded pane keeps its borderless
  rendering and its folded title is clickable.

## Scrolling

`board::scroll` (`Scrolls::show`) is the one scroll owner: each pane hands it lines and it keeps
a position per pane, clamps it to the content, reserves the last line for an `↑ n  ↓ m`
indicator on overflow, and records where the pane was drawn so the wheel scrolls the pane under
the pointer and a left click focuses it. Panes keep no scroll state of their own. The rows pane
only asks it to reveal the selected record's visual-line range while followed (or its first line
when taller than the viewport). Each draw records record starts and hit targets for every
continuation, and each draw records which screen lines show which row so a click selects exactly
the row drawn there. Paging moves by viewport lines (including notes and configured row lines),
by record when no positions were drawn.

## Tab line

- Tabs are the same width selected or not: selection is a style, never extra characters.
  `board::view::tabs::tab_label` owns the styled tab label: a fixed two-cell mark slot
  (`◆ ` waiting, `✗ ` blocked, else two spaces) precedes each name, the dominant count follows,
  and with both states a blocked `✗n` follows. Only the marks (and the appended blocked count)
  use the bold attention styles; names and primary counts are selected accent/bold or inactive
  muted. Selection covers the whole tab with the background or a reverse fallback; the switcher
  keeps its own selected-row style. Rendered `Line::width` drives tab scrolling, hidden
  reservation, hit geometry and switcher fitting; overflow counters keep their aggregate
  attention styling.
- Moving a tab (Shift+←/→ or a drag on the tab line) saves `[tabs] order` through
  `Config::write`. `board::view::tabs` owns tab-line display after `tabs::arrange`: a pure
  window computation admits pins and a contiguous scrolling range from measured label, group and
  overflow widths, and the painter applies the same decisions to spans and exact `TabHit`
  geometry. Only adjacent squad keys sharing a nonempty prefix before the first `-` (two or more
  drawn tabs; built-ins and user tabs interrupt a group) are grouped display-only; the prefix and
  separator have no hit, and each suffix keeps its mark slot and original key/index for clicks,
  drags and switching. Left overflow counts skipped tabs; right overflow names the remaining
  tabs, waiting first, then blocked, then quiet, in arrangement order within a tier, using full
  labels unless the visible prefix makes a suffix unambiguous. The current tab is always visible,
  even when pins exceed the width, so other pins step aside from the end without changing the
  stored order; otherwise pinned tabs (`[tabs] pin`) precede the scrolled window, and a move never
  moves or passes a pin.
- `Config::tabs` defaults to pinned home (`@all`) then leads only when neither order nor pin is
  configured (explicit empty arrays count as configured; hide stays authoritative), so a board
  opened without a squad name starts on home and `--squad NAME` opens that squad. Home renders as
  an accent `▚ tmt` block with both attention counts inside, using existing roles and a reverse
  fallback; its public and config keys stay `all`.
- The switcher (`s`, unless rebound) filters tab-line and hidden tabs with `tabs::matching`: a
  prefix match first, then a substring, then letters in order. A shown squad that is not on the
  tab line is drawn first, selected, with no `TabHit`, so it cannot be moved.
- `board::pick::Picks` owns the transient admission policy: omitted/`all` follows inventory,
  while explicit names and switcher toggles retain canonical keys. Startup resolves only board
  inventory before terminal admission; `ls --tab` is unchanged. `App` retains full arranged and
  hidden inventories for caches, ordering and switcher selection; navigation and paint filter
  them without rewriting config. Opening an excluded tab admits it. Removing the current pick
  requests the next picked tab or home through the ordinary cached/uncached load path. Refresh
  prunes removed keys but preserves picks during a failed empty inventory read.
- `view::tabs` measures the folded unpicked-squad segment first, then applies the existing
  admission/window policy to picked tabs and recomputes groups over that sequence. Drawn hits
  retain canonical indices. The fold sums `Attention::of`-derived attention from visible,
  unpicked squads only, excluding aggregates and global hide; its separate rectangle opens a
  restricted switcher and never participates in dragging. `view.rs` resets both hit maps.
- The switcher adapter consumes the named `pick-tab` action before shared picker text handling
  in both query/list fields. Default Space is local to this adapter; explicit bindings replace
  it and a non-pick Space binding takes precedence. The shared picker stays unchanged. Pick
  markers and the effective key appear in its markup/footer and board help.
- `App` keeps the view of each visited squad. A switch shows a cached view at once; otherwise it
  keeps the current frame (marked stale, so row actions refuse) until the new snapshot swaps in
  whole. An uncached switch lasting at least `SPINNER_DELAY` (100 ms) shows a spinner in the
  fixed summary header ticking every 80 ms; cached switches show none.
- `ctrl-r` defaults to refresh in squad, leads and all views; squad/leads bindings can rebind it
  through `[bind]` and all keeps its own `[tabs.all.bind]`. The effective refresh binding is
  dispatched before text inputs, preserving search and composed messages. F5 has no default but
  is configurable.
- `jump lead` (`L` in the tmux preset) resolves a lead name in `App::lead`: the document's
  `squad.lead` on a squad tab, the selected row on the leads tab, the selected entry's lead on
  home. It then takes the ordinary jump request, so the popup closes and `back` returns.

## Home

- `board::home` keeps a board-only summary, shared-filter attention sections and a compact
  squad-line model as `View.home: Option<home::Home>`; other views carry none. It reuses
  `tab_view` acquisition and the user-tab section pipeline. Optional observed ages come from the
  staleness observer: the home tab starts one for every squad before its roster read and records
  afterward, writing the cache under the held per-squad lock when enabled and available, and it
  follows the reminders policy without extra core commands. Request ages use shared-inbox
  timestamps; pending-only rows have no age. The source aggregate document and `ls --tab all`
  stay unchanged.
- The home painter uses the summary band and a flat body, bypassing ordinary pane composition
  for the shown immutable home view. It keeps one `App.selected` cursor reconciled by
  section/squad/member identity across refresh and search; attention precedes squads. Hits,
  paging and overflow reuse `Scrolls`. Enter jumps to a member or opens a squad; Tab traverses
  attention/squads, and `a` opens the real request picker or an annotation to the selected
  squad's lead. The composer keeps and revalidates sender, target, lead and open request before
  the public `tmt answer` or annotation dispatch, and questions stay inside the picker. Home
  synthesizes no tiles, replies feed or model/token totals; its ⑤ cron line is the
  [cron board](#cron-on-the-board).
- New home sections add pure line builders that return lines and local
  entry/x/width/start/end placements; home translates them into the shared cursor, paging,
  reveal and clipped hits. Their acquisition and lifecycle owners stay outside paint.

## Cron on the board

`board::cronboard` owns every board-side cron concern; all reads and writes go through
`cron_service`, `cron_clock::send_now` and `cron::Clock::status` (the
[cron section](data-and-state.md#cron) owns their contracts). Squad stays an extension: nothing
here reads the store or core directly.

- **Acquisition.** The refresh worker queues one `Deferred::Cron` read after every full reload
  (any tab, same lane and generation cancellation as attention), published as `BoardEvent::Cron`
  into `App.cron` (`State { cron, failure }`): jobs of every active squad including hidden ones
  (`list_jobs`, one next slot each), the clock status and the actor resolved once per read. A
  failed refresh keeps the previous jobs next to its reason; before the first read nothing is
  drawn. Paint and input never read it from core. `list_jobs` resolves each owner through one
  public `references.resolve`, so a read costs one core call per job.
- **Home ⑤.** One cursor target between attention and squads (`home::CRON`); Enter or `c` opens
  the list. `cronboard::line` is pure: preview, then owner, step aside before the count, time,
  clock and `c list`; below that the line compacts. The clock reads `checking…` until the second
  read and names its holder `session:window`: the refresh worker asks tmux once per pane id
  (`effects::pane_place`, one bounded `display-message` on the invoker's socket, cached in
  `cronboard::Places`, failures too) and stores it on the read; outside tmux or on any failure the
  pane id shows. `clock --json` keeps the pane id. The read's failure shows as a blocked line.
- **`c` list.** `Overlay::CronList` routed through the shared `FocusStack` and `app::route`,
  painted by a `picker_surface::State` list modal docked at its content height (like the
  prompt band, so nine tenths wide from 100 columns). Row IDs are `<room uuid>/<c-id>`. Enter opens
  the job's squad; refresh keeps the selection by identity. It closes for forms and the delete
  confirmation and stays open for pause/resume and send.
- **Jobs half.** On a squad tab (`document.squad.roomId`) `composition::halves` places the
  configured composition above and the half below from one flex computation; the half takes its
  content height up to two fifths of the body, and below 12 body lines keeps only its rule line. Its list is
  an ordinary `tmt-list` with a per-room `ListState` (selection survives tab switches); the
  selected job expands in place while the half has focus. Tab enters the half after the last
  visible pane and leaves it for the first; a pointer press inside focuses it. `focused_pane()`
  is `None` while it has focus, and `App::perform` refuses member-row actions then.
- **Scoped keys.** While the half or the list has focus, `n e p x o d` and Enter are job keys,
  routed through the `FocusStack` base field `cron-jobs` before board dispatch; a key the user
  bound in `[bind]` still wins. Footer, list footer and help share one table (`cronboard::hints`); the jobs footer ends with `? more`.
- **Controls.** Pause, resume, send and delete build a `CronRequest` with the actor from the read,
  the job key and the viewed revision, executed on the existing `execute` path; apply revalidates
  actor, room, owner and revision under the jobs lock, so stale, unauthorized or invalid requests
  write nothing and are shown, never retried (the next reload shows the truth). New, edit and
  reassign are a typed `Draft` on the input line (owner name, message, schedule text): no trim, an
  untouched message or schedule is not submitted, and a message with control characters or over the
  line limit is kept as stored. Owner names resolve through `identity show` at submission.
- **Members.** A row's `⏱ <next>` joins the row-end label after the age mark and is the first to
  drop; the grid reserves its room only when no column would hide, and the cached grid is keyed on
  the labels. The member detail repeats it as `cron: ⏱ <time> · <id> <message>`.

## Notes pane

- The notes pane shows the squad lead's own saved-identity notebook, read-only; there is no
  separate squad notebook. `observe` selects the member with `Member::is_lead` and reads its
  UUID through public `tmt api notes.read` (bounded, never creating a file), the notebook
  `tmt notes path --identity <lead>` discovers. `board::refresh::lead_notes` maps a missing
  notebook to `(no notes yet)`; a temporary lead's `NOTEBOOK_SAVED_IDENTITY_REQUIRED` is shown
  as failure text.
- `board::notes` strips every escape sequence, control character and hidden bidi/format
  character before display, since notes are agent-written. `board::markdown` is a thin
  pulldown-cmark view over that sanitized text: headings, lists, emphasis, inline code and
  links are styled and every other construct shows as source.
- `links` classifies explicit Markdown destinations as web, GitHub issue/PR, local path,
  built-in `tmt:` or user-configured scheme, and keeps destination occurrences with wrapped
  display-cell ranges. Admitted labels use the Link role and underline; kind and full
  destination show in the footer before activation. Tab/Shift-Tab select links in focused notes
  (Tab keeps pane traversal when none; explicit bindings win). A first click selects/previews, a
  click on the selected occurrence activates, Escape clears. Plain mode is inert.
- Only `tmt:jump/back/talk/answer/open/copy/annotate` are admitted. Except `back`,
  `/<member-name-or-id>` must resolve to a current row or the separately projected lead; an
  optional `?text=` is bounded percent-decoded composer text for talk/answer/annotate only.
  Those verbs reuse existing prompts/request pickers; submission revalidates sender, squad,
  member, lead or open request after refresh, and answer uses the public core answer adapter.
  Undefined or invalid schemes are plain text and cannot dispatch.
- A custom program comes only from a user-file `[links] scheme = "run program {path}"`:
  validated literal executable, one argv element per template, no shell or option injection;
  reload replaces that authority. The detached spawn/reaper owns programs. Absolute local paths
  reveal after canonicalization (macOS `open -R`; configured/Linux openers receive only the
  containing directory); relative paths are inert, and opening files needs a user-defined
  scheme. Neither parsing nor paint opens files, fetches URLs or invokes commands.
- Painted lines keep their notebook source line without a second Markdown parser. `App` keeps one
  notes cursor per visible/hidden squad, anchored to the complete sanitized source line (nearest
  match for duplicates, clamped after deletion) with a continuation offset for wrapped lines.
  Cursor movement and click placement reveal the line through `Scrolls`; wheel scrolling
  suspends following until the cursor moves. Every painted continuation of the selected source
  line uses the selection background (reverse fallback) across the pane width; only visible
  lines are decorated, and a fixed two-cell gutter holds the sent marker or blanks before
  wrapping.
- Annotations reuse the ordinary composer and sender, addressed to the current lead and tagged
  `[<squad> · notes L<one-based line> <JSON quote>] ` with a bounded quoted excerpt; that tag is
  the contract between the sender and request projection (display quotes are separate). Opening,
  canceling or submitting an empty composer sends nothing. `requests::apply` projects the
  user's open notes annotations as `squad.noteAnnotations` (`requestId`, zero-based `line`,
  `quote`) from the existing bounded room history; the painter marks the nearest matching quoted
  line with `✎` and answered requests disappear on the next refresh. No extra core read,
  notebook mutation or acknowledgement exists.

## Detail and replies panes

- The detail pane appends full projected `row.fields` values for board columns not already shown
  by its header, task, activity or links, in column order, escaped and wrapped without grid
  fitting, source lookups or provider calls. It then appends the selected member's saved-identity
  notebook: only a visible, expanded selected detail requests it (accounting for effective Board
  previews, tab focus and the last painted viewport); temporary identities show
  `(temporary identity: no notebook)` without a read; leads/home never show member notebooks.
  `board::refresh::Deferred::Notebook` runs public `notes.read` with the same bounded cancellable
  reader and 1 MiB API limit as lead notes, never creating a file. Full reloads take priority and
  queued selection jobs collapse to the latest. Events keep the generation cancellation plus a
  session selection/refresh revision, so obsolete results cannot update the cache; each snapshot
  revalidates the visible selection and hidden detail does not read.
- `App` keeps the last eight identities' sanitized notebooks, preserving the rendered body for
  unchanged content and invalidating it on width, look or render-mode change. Both notebook panes
  share safe Markdown/plain rendering and the missing placeholder; failures replace the selected
  cache entry.
- Replies: bodies come from `requests.show` for the newest eight only, and the refresh worker
  caches them by request ID, since a submitted final never changes. Bodies are agent-written and
  use the notes sanitizer and Markdown renderer with full wrapped content and a two-cell indent;
  prompts wrap with a hanging indent and recipient/age headers stay single-line. The immutable
  view's `Derived` caches rendered bodies by request ID, effective width and look; headers and
  prompts are assembled each frame so ages stay current without reparsing, and view replacement
  discards the cache. Replies use the shared `Scrolls` owner.

## Overlays: help, settings, pickers and switcher

- `board::overlay_event` (in `board/app.rs`) is the shared modal input adapter for help,
  settings, the theme and view pickers and the tab switcher. It synchronizes their controller
  identities with one caller-owned `FocusStack` and routes key and mouse events through
  `tmt-tui::app::route` before base dispatch. Controllers keep save, rollback and worker
  effects. Close is consumed, unhandled modal events stay captured and Ctrl-C returns Quit;
  pane cursors and scrolls stay with their existing owners.
- `board::picker_surface` keeps the caller-owned shared `Picker` state, admitted scenes and the
  current clipped frame maps for settings, theme/view previews and the switcher. Theme and view
  controllers derive the selected choice from stable component identity and keep scope, opening
  Config, preview and persistence; their selection-only field keeps Tab's scope action. The
  switcher registers query and list fields: printable navigation/close keys stay query text, Tab
  moves between the two fields and query edits reset to the first match. Refresh follows the
  selected complete tab key, and resize or model replacement invalidates hits. Its semantic
  attention spans use shared hit geometry and Squad's tab-color/selection policy.
- All of these use shared modal chrome, wrapping, scrolling and inside footers. Settings use
  grouped stable-key list rows for the reference and an admitted docked prompt for edits; the
  Config controller keeps raw edit text, validation, the disposable preview, stale-file refusal
  and persistence, and cancelling an edit restores the retained list selection and scroll. Group
  headings are disabled rows; read-only settings stay selectable so Enter can explain the
  restriction.
- Help (`board::help`) is a body-placed modal with one all-section key column and a fixed inside
  footer. Its scroll state and the common App focus adapter route keys and mouse before board
  actions. Refresh replaces help data and clamps the shared viewport without reads or actions in
  paint.

Theme and view picker lifecycles are in [config-and-effects.md](config-and-effects.md); shared
component rules are in the [tmt-tui skill](../../tmt-tui/SKILL.md).
