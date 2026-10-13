# Board frame, rows and interaction

The [developer SKILL](../SKILL.md#board-surface-ownership) owns the surface map and
extension invariants. [Full-screen interaction](../../../../design/cli-style.md#full-screen-interaction)
owns interaction and appearance rules; the [TUI reference](../../tmt-tui/references/pipeline-and-components.md)
owns admission, geometry, painting and shared components. The shipped Squad skill owns
[board controls](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#board-appearance)
and [row configuration](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#columns-and-row-lines).

Surface references: [HOME](board-home.md), [notebooks/detail/replies](board-notebooks.md)
and [cron](board-cron.md). Acquisition belongs to [refresh-and-meter.md](refresh-and-meter.md);
configuration and effects belong to [config-and-effects.md](config-and-effects.md).

## Frame and terminal

`board::terminal` owns raw mode, mouse capture and the alternate screen through
`Screen` and `Guard`. Partial entry is undone; restoration runs on return, error,
panic and TERM/HUP. Worker shutdown follows terminal restoration.

`board::view::render_frame` resets frame-local hits, input placement and scroll
regions once. `view::panes` dispatches workspace snapshots to `workspace::render`
before independent row/pane painters. `board/mod.rs` receives input, snapshots
and deferred events on one channel; snapshots wake painting directly. Redraws
follow state/input/resize changes and changed clock text/spinner frames.

Workspace tree/detail uses shared Outline/Strip/Scrolls. Conversation and inline
input retain their WP1 owner and opaque covered-hit mask. Review/Result uses a
square shared modal. Incidental frame cells remain Dim; receiving focus and
selection stay distinct. Frozen row/HOME provenance and decoded fixture approval
are owned by the verification reference.

## Reload in place

`board::reload::Watch` lets a running board adopt a new `tmt-ops` without losing its
place. Once a second on the existing loop wake it compares the launcher fingerprint
(canonical path, device, inode, mtime of the PATH-resolved `argv[0]`) with the one
taken at start, and the `reload/request` marker under the Ops cache (written by
`ui --reload-all`) with the board's start time; a replaced launcher wins over a
request, and a missing or non-runnable launcher never triggers a reload. A due
reload runs only when `App::reload_blocked` is false and the board has had no input
for `Watch::idle` (1 s); `reload_waiting` makes the footer append a dim cue instead
of replacing its prompt, notice or hints. `session` then returns `Ok(None)` with
`App::restart` set. `run` restores the terminal, stops the worker, writes
`board::resume::Resume` to `reload/resume-<pid>.json` (0600, version, pid and
freshness checked, size-bounded, never through a symlink) and `exec`s the launcher,
so the pid is kept; a failed `exec` discards the snapshot and returns
`SQUAD_RELOAD_FAILED`. A failed `exec` also resets SIGPIPE to its default for the
whole process, so tests drive `restart_with` with an injected failure, never a real `exec`. The new process takes the snapshot once at start and
`App::carry` applies it on the first fresh view of that tab (`carry.rs`): selection
by `RowTarget` identity, never by index, a vanished row falling back to the normal
start, and the snapshot dropped if that view fails. The snapshot holds view state
only, no squad data.

## Rows, grid and identity

- `rows::Rows` owns positional tracks and prefix coverage, including empty cells
  and spans. Uncovered columns remain value sources. `Column::display` ignores
  their sizing; `ListSizing` selects natural text-list sizing unless shown columns
  opt into percent width or overflow. Text lists then use `tmt-cli-style::grid`;
  board geometry remains the separate CSS grid policy of `markup::Grid`.
- `markup::Grid` resolves configured bases/bounds and chooses priority tracks before
  one TUI grid computation. Growing tracks default to `rows::NARROWEST`; natural
  capped tracks retain content demand, while capped growing tracks expand to their
  cap. Hidden tracks keep original positions: spans shrink over surviving tracks,
  and zero-survivor cells disappear. `rows::read` rejects unknown, duplicate,
  uncovered and all-hidden masks. Full projected values remain intact.
- `markup::row_values` binds display-ready cells and typed semantic `Role` tokens,
  never source lookups or formatting. Occurrence scope is tab, authored section,
  source squad and member UUID, followed by static line/column IDs. The lead uses
  the `lead` scope; member order is not identity. UUID-free rows remain drawable
  but have no actionable IDs. `App::shown_tab` supplies the retained view's owner
  during loading, including after resize or search.
- `display_rows::project` supplies lead, nonselectable members rule and authored
  sections to `App::items`, `App::rows` and text `ls`. The rule counts distinct
  shown members excluding the lead; `none_yet` depends on the roster, not search.
  Search also filters the lead; without a lead there is no rule. `each_row` includes
  the lead in waiting, staleness, usage and color projections. `RowOrigin::Lead`
  carries no section binding, and public JSON keeps it outside `sections`.
- Default squad `members` and HOME leads use the neutral `view::member_list` boxed-list
  scene and `Derived.member_list` cache:
  lead first, the nonselectable members rule, then each member’s selectable heading and task line.
  `View.exchanges` comes from `home_leads::members` using the already acquired
  `requests::Sent` window. `App::items` applies the HOME comparator within authored
  member groups; the lead/rules/sections and public documents stay fixed. Actual
  row waiting overrides newer replies for squad ordering; HOME retains its latest
  exchange semantics. Task/state/model/observed age feed the same scene key, and
  read-only detail joins the row stream through `row_detail`.
  The mark column is reported state only (`◌` idle; `○` is the Core offline mark and no
  member row draws it). Observed `presence` fills the state cell when no state is
  reported, appends `· offline` to a reported one, and sets the bound `name_role` (dim
  offline); HOME lead headings bind `Text`. The stored-display cache carries no presence.
- `view::stable::Stable` (`App.stable`, begun per frame) owns what outlives a snapshot,
  so a refresh moves no cell. Cell widths only grow while a tab stays open at one
  terminal width (another tab starts over; a resize derives them again), and the last
  model and age read for a row id stand in, dim, for up to ten minutes when a read has
  none (an unobserved or `unknown` age; `disabled` clears it). `view::rows` takes the
  `natural` of unsized grid columns and the row-end label reserve from it;
  `member_list` takes the model cell (grow-only, at most 8) and the age cell (8 cells
  right-aligned; HOME lead headings 4) from it, and below 40 cells shows neither. The
  squad name column follows the widest filtered display name, with a 12-cell minimum
  and a 26-cell / half-row maximum; whole state text precedes optional chips. HOME
  keeps its existing name sizing. Layout
  reads no value as loaded now: placeholder, carried and fresh values share their cells
  (`view/tests/layout_shift.rs`). The stored display is a separate inert preview.
- Other named/custom views: `view::rows` prepares `row_paint::RowPaint` before replacing the immutable view's
  `Derived.grid`. Its key includes effective width, search and `Extra` (lead,
  clock-derived cron/request labels, sent feedback and input reservation). Failed
  layout/value preparation displays a muted error and leaves the prior cache intact.
  Selection changes styles without rebuilding the scene. New views start fresh.
- `RowPaint` owns cells, logical text widths, cuts, row-end labels and clipped hits;
  `paint_with` gets Squad's alignment and `Look::row_span` selection/stale/token
  policy. Missing uncolored cells remain Dim; reverse selection overrides their
  colors. Annotation marks never take selection. The row backdrop reaches its
  text or end label. The lead tag stays inside the member cell, cuts before the
  name, disappears below two cells and reserves no width on other rows. Age/cron
  room is reserved only if it hides no additional column; cron drops before age.
- Occurrence selection changes styles while preserving the existing blank prefixes:
  grid x=1 beside the attention diamond, and boxed heading indentation.
  Hit/reveal ranges, annotation, sent feedback, borders and input reservations
  do not define selected paint. Duplicate occurrences remain independent. HOME
  boxed leads select their heading only; member boxes retain their heading/task selection.
- Selection words: `Look::selected_words` is the one owner of what a real selection
  background (`tmt`, `tmt-light`) does to colors. `render_frame` runs it last, over the
  finished buffer: on a cell with the selection background, `muted`, `dim`, `accent`, `link` and the
  state colors (waiting, working, review, blocked) paint in `text` unless the cell is a
  single mark glyph (`MARKS`; a dim or muted mark still becomes `text`). Surfaces name no
  word/mark classes; `render_replica` in `frame_timing` repeats the call. The reverse
  fallback stays with `row_span`. The contrast test in `tmt-cli-style` pins text 4.5:1
  and marks 3:1 on that background.

- `board::row_detail` owns the typed entity fields/reply projection, one renderer and
  full-reply reader for member, HOME lead/member and job rows. Adapters supply data
  and row geometry plus fields shown uncut in the collapsed row (truncated or
  clamped fields remain in the detail block), never a
  separate detail look. Collapsed content stays intact; detail omits visible
  fields, including task/model in the default member box and width-dependent
  job/grid cells. `e` toggles stable row occurrences;
  several can expand. Refresh prunes disappeared rows and obsolete request bodies.
  Detail participates in the list height and parent hits, with no composer, input-band
  or selection background. Labels/gutter, three-line fields and six-line Markdown
  replies share wrapping and roles across every adapter. Existing scroll owners
  reveal the minimum row/block range, clipping tall blocks with their overflow cue.
  Full bodies arrive only through the existing cancellable worker/request cache,
  fenced by revision and exact sender/recipient/request; collapsed or removed rows
  cannot accept late results. `v` opens any available reply in the shared read-only
  modal/ScrollState reader, and clicking its generated overflow line does the same.
  Enter and `o` retain ordinary row actions. Expansion needs no recorded user or
  Rows-pane focus; sending still does.
- Row composer and footer: `Input.compose` is the current mode (answer, note, talk or status)
  and `Input.others` the rest in cycle order; Tab (`cycle_mode`) rotates them and
  refreshes the quote. `attach_row` builds the list from the row (`other_modes`), so a
  mode appears only when available; multiple answers use the same explicit request menu. `Input::header` names the recipient, its squad and the mode (`→ sol (product) · note`).
  Workspace dispatch, help and footer resolve the same Config key table. Text,
  search, completion, overlays and the conversation own input before shell keys.
  The footer fits whole ranked hints and hides its base hints under opaque modals;
  notices/error precedence remains with the existing frame owner. Theme and Token
  window actions reuse their existing Config writers, while Auto/Side/Stack writes
  only workspace preferences. Reading, preview or cancelling sends nothing.

## Composition, folds and scrolling

`workspace::render` places the tree and conversation/detail from Auto/Side/Stack
preferences and actual minimum usable sizes. Squad expansion is keyed by room
UUID, not a per-tab pane fold. Retired factory/pane/tab settings remain authored
configuration with located migration notices; they do not create a legacy UI.
The independent split/markup readers and row painters remain separately tested
for CLI projection and offline authoring contracts.

`board::scroll::Scrolls` owns pane offsets, clamping, viewport reservation and
pointer areas. Tree starts, reveal and clipped hits share that viewport. Following
reveals the selected visual range or its first line when taller than the viewport;
paging uses visual lines. Review/Result's shared ScrollState owns its read region.
Resize/model replacement invalidates controls before another action can use them.

## Workspace and retained identity

`board::shell` selects Overview/Agents/Digest without loading a second roster.
`workspace::observe` projects the same acquired room/document/identity snapshots;
`workspace::OpsWorkspace` owns UUID selection, expansion and filter/sort ordering.
The [workspace reference](board-home.md) owns geometry, count truth and modal
operation review. Public Config and `ls --tab` projections remain independent of
interactive navigation; tab/pick/reorder and factory-view bindings report
migration notices instead of providing a legacy UI.

Cold startup keeps the delayed spinner (`SPINNER_DELAY`, 100 ms) and existing
worker cancellation fences. Failed refresh retains stale evidence; it cannot
turn unknown membership into an empty pool. Historical usage loads after roster
without moving recipient authority. Effective refresh preserves search/drafts.
In-place reload carries shell/expansion/exact UUID selection; hidden/missing
anchors refuse actions and never retarget to a visible neighbour.

## Decisions and composers

- `attention::waits_on_you` owns the pending/request predicate. `view::waiting`
  prefers pending text to the oldest acquired request preview and accepts only
  positive, nonfuture request timestamps for ages. Authored pending cells keep
  their position; otherwise a continuation carries its own row hit. Header
  `time_marks` includes request ages, so text advances without another read.
- `view::footer::hints` derives one allowed action hint from effective bindings
  (Enter first), ranked by `Action::footer_rank` independently of menu `Action::order`.
  `row_allows` rejects unavailable row actions; internal verb wording uses `hint_word`. The oldest-member
  label gets space only after all hints through `FOOTER_ROW_ACTIONS_END` fit.
  `board::glyph_guard` checks these labels and HOME/cron text. Tail fitting and
  appearance rules belong to the interaction owner linked above.
- `App::input` is the shared talk/answer/annotation/status/ask-lead/HOME-audience composer.
  `RowSend` freezes the occurrence and opening sender. `composer` retains session-only
  drafts, folded paste and response quotes by saved sender/recipient UUID across
  view and membership changes. Successful delivery clears only the submitted
  revision; failed or uncertain delivery retains it. Restart persistence is not
  promised. Submission revalidates sender, occurrence, actual lead or chosen open
  request before invoking existing public send effects. Several open requests use
  an explicit picker; empty submission and cancellation send nothing.
- Message fields use `tmt_tui::components::Editor` for grapheme-safe multiline
  editing, source-preserving bracketed paste and cached cell geometry. Enter
  submits, distinct Shift+Enter or portable Ctrl+N inserts a newline, and Escape
  retains the draft. Terminals own IME preedit; committed Unicode enters the same
  editor. Large paste folds retain exact text and can be previewed or removed.
  `mentions` supplies the shared completion component with acquired agent UUIDs
  and squad IDs. Inserting a reference never changes recipients or broadcasts;
  cron stores literal text. Name, configuration and search fields do not complete.
- `board/status_update` owns the status form in that composer, measured wrapped
  lines and focus/scroll. Each wrapped field line reserves two columns for the `›`
  focus cue, independent of color and checked mutation choices. `SelectedRead::Status` acquires raw roster metadata through
  the existing cancellable worker, fenced by generation, read revision and exact
  target/actor. No display/provider value becomes an expectation. `membership/status_update`
  freezes UUID, room, namespace and full keys, validates the actor and occurrence,
  then calls unattributed public `identity.meta.apply` once for selected fields,
  including locally unchanged selections as expectation guards. A wholly unchanged
  submission refuses before applying; the announcement lists only changed fields.
  Exact old values and absence stay distinct; unsupported legacy values refuse.
  Conflict refresh retains context, clears field choices and needs a fresh submit; ambiguous output locks
  out mutation replay and sends no announcement. Typed `ActionOutcome::Status`
  keeps Refused/Conflict/Unknown/Applied state rather than flattening it to a notice.
  Applied retains only a frozen announcement for explicit notification-only retry;
  focus returns to the first field so queued submit keys cannot retry a notification.
  Unanswered requests remain separate and use the existing one-request composer.
- A visible conversation reserves its composer inside that pane. Otherwise row
  painters reserve input beneath the complete target in the same line stream used
  for starts, reveal and hits. `view::waiting` removes only hits covered by the
  current opaque band, preserving independent pointer scrolling in other panes. Headers derive from actual `Compose` recipients/subjects. Unanchored input
  keeps the footer path. Recipient/sender/source fences remain enforced at submit. Read-only detail belongs to `row_detail`, outside the composer.
- `RowFeedback` retains the anchored occurrence through refresh; its `sent` flag
  alone permits `✓ sent`. A removed workspace target remains an explicit missing anchor, with no adjacent
  recipient authority. An active status form retains its row through
  applied/unknown outcomes and notification retry without claiming a successful
  send; the acquired model and request state are unchanged. Clearing precedes any
  underlying action after leaving the form.

## Overlays and offline validation

`App::overlay_event` synchronizes help/settings/theme/view/switcher/cron/checklist controllers
and workspace forms with one caller-owned `FocusStack` before base dispatch. Controllers own saves,
rollback and worker effects; `picker_surface::State` owns component cursor/query,
admitted scenes and clipped frame maps. Refresh follows stable selected identity;
resize/model replacement invalidates hits. Query edits select the first match;
query/list and selection-only scope fields remain controller-specific.

Choice cursors are caller-projected `›` cells from the reconciled selected row ID.
View keeps saved `●` at mark cell 0 and cursor at cell 1. Theme uses zero-gap
`[10,1,1fr]` tracks; menus keep the exact key width followed by one cursor cell.
Switcher uses a fixed nonshrinking five-cell pick/cursor/attention prefix, and
cron a six-cell state/cursor/ID prefix, both with zero inner and original outer
gaps. Content starts and semantic marks stay fixed. The cron list supplies its
cursor; the shared jobs half passes no cursor. These cues appear on the first
admitted row line; Config retains its existing selected continuation marks.

A controller returns `None` only for an unconsumed event: routing may offer it again
to the overlay. Consumed moves and boundary presses return `Some`, including
`PickerInput::Captured`. Settings group headings are disabled rows, read-only entries
remain selectable, and cancel restores list selection/scroll. Help refresh replaces
projected data and clamps its shared viewport. Component routing/chrome rules are
owned by the TUI reference; preview/write contracts by the configuration reference.

`layout` checks a bounded authoring file before core/config/storage discovery, using
TUI admission and eager `squad-projected-v1` binding. Field/source validity reuses
`rows::field_name`, `OWN_FIELDS`, `ColumnSource` and `Format`; provider names are
checked syntactically, without configuration/data. It materializes nothing and the
board never loads the file. The shipped skill owns the
[schema and examples](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#offline-markup-authoring).

## Verification

[development.md](development.md#checks) owns commands and capture requirements;
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline) owns
the frozen parity approval/regeneration gate. Relevant tests live in `view/tests`
(cells, styles, hits, footer, menu and parity), `composition`, `scroll`, `app`,
`markup` and `layout`. Text-list projection is corroborated by the native Squad test.

## Checklist controller

`App::context_menu` owns the context action menu for configured Menu bindings and
`,` → Workspace settings → Actions. It adds Checklist before
member-row admission; no selected member exposes only context actions. Workspace settings exposes Actions after layout/Theme/Token window choices. Closing that menu returns directly to the board.

`checklist::{controller,forms,surface,load}` owns one centered square modal and its
room chooser, independent filters, item selection, authored fields and exact
confirmation. The shown named room freezes its UUID; other contexts require an
explicit room choice. Default scope is Open / Squad-wide / archived excluded.
The unfiltered total uses the same archive scope as the matched count. Item Enter
opens details; explicit named controls perform edits, assignment, completion,
archive, delete and full inventory reorder. Archived items expose Restore/Delete.
Delete uses Review delete / Confirm delete, with its title in text colour and
muted UUID/revision context outside the selectable choices. Details keeps IDs
muted; single-line Body previews show only the first line, followed by an ellipsis
when more follows. Delete starts on Cancel and requires all exact identities/revisions and both
controls to fit in current shared geometry. Clipped/stale hits cannot submit.

The existing refresh worker queues checklist jobs without collapsing writes. Its
lane retains Service actor/context for a controller and exact room, with a separate
shutdown cancellation token so board-read preemption cannot erase a started write.
Controller/serial/room keys fence every result. UUID allocation reuses the same
`id::new_v4` helper as explicit sends; each caller retains its own error mapping. Conflict/refusal retains intent and
fields; Refresh observes and Review explicitly adopts current expectations before
fresh Confirm. Original Unknown targets remain unknown after reads and subsequent
acknowledgements. Closing retains drafts and stable base return context; explicit
Cancel discards intent. This flow never contributes attention or changes Core
requests, member state, metadata or configuration. Storage/service invariants live
in [data-and-state.md](data-and-state.md#checklist-storage-and-service).

The jobs half and `c` list never expand merely on selection or focus. Their ListState
owns navigation, scrolling and hit geometry; `row_detail` paints into their reserved
inline detail lines. Scoped `e` toggles, `E` edits, and the single cron `KEYS` table
owns footer/help words. `E edit` appears only when a job is selected.
