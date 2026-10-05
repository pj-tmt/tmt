# Board frame, rows and interaction

The [developer SKILL](../SKILL.md#board-surface-ownership) owns the surface map and
extension invariants. [Full-screen interaction](../../../../design/cli-style.md#full-screen-interaction)
owns interaction and appearance rules; the [TUI reference](../../tmt-tui/references/pipeline-and-components.md)
owns admission, geometry, painting and shared components. The shipped Squad skill owns
[board controls](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#board-appearance)
and [row configuration](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#columns-and-row-lines).

Surface references: [HOME](board-home.md), [notebooks/detail/replies](board-notebooks.md)
and [cron](board-cron.md). Acquisition belongs to [refresh-and-meter.md](refresh-and-meter.md);
configuration and effects belong to [config-and-effects.md](config-and-effects.md).

## Frame and terminal

`board::terminal` owns raw mode, mouse capture and the alternate screen through
`Screen` and `Guard`. Partial entry is undone; restoration runs on return, error,
panic and TERM/HUP. Worker shutdown follows terminal restoration.

`board::view::render_frame` resets frame-local hits, row starts, input placement
and drawn scroll regions once. `view::panes` dispatches HOME before ordinary pane
composition. `board/mod.rs` receives input, snapshots and deferred events on one
channel; snapshots wake painting directly. Redraws follow state/input/resize changes
and changed clock text or spinner frames, rather than periodic full repainting.

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
  lead first, the nonselectable members rule, then two selectable lines per member.
  `View.exchanges` comes from `home_leads::members` using the already acquired
  `requests::Sent` window. `App::items` applies the HOME comparator within authored
  member groups; the lead/rules/sections and public documents stay fixed. Actual
  row waiting overrides newer replies for squad ordering; HOME retains its latest
  exchange semantics. Task/state/model/observed age feed the same scene key, and
  read-only expansion replaces the task line with the shared reservation.
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
- Selection words: `Look::selected_words` is the one owner of what a real selection
  background (`tmt`, `tmt-light`) does to colors. `render_frame` runs it last, over the
  finished buffer: on a cell with the selection background, `muted`, `dim`, `accent`, `link` and the
  state colors (waiting, working, review, blocked) paint in `text` unless the cell is a
  single mark glyph (`MARKS`; a dim or muted mark still becomes `text`). Surfaces name no
  word/mark classes; `render_replica` in `frame_timing` repeats the call. The reverse
  fallback stays with `row_span`. The contrast test in `tmt-cli-style` pins text 4.5:1
  and marks 3:1 on that background.

- `Compose::ReadRow` anchors fields/latest reply to the same row occurrence and
  shared `App.input` as HOME/read/send modes. `home::controller` reuses cached bodies
  or the existing fenced request read; `view::waiting::read_lines` supplies field
  order, height, paint and scroll limits. Read-only expansion needs no recorded user;
  writing does. Effective bindings control collapse/write/open. Writing from a
  read band dispatches the same row action as the list, so default `a` answers first
  and explicit note/talk/reply bindings keep their recipient and mode. Covered hits
  are removed by the existing band owner.
- Row composer and footer: `Input.compose` is the current mode (answer, note, talk or status)
  and `Input.others` the rest in cycle order; Tab (`cycle_mode`) rotates them and
  refreshes the quote. `attach_row` builds the list from the row (`other_modes`), so a
  mode appears only when available; multiple answers use the same explicit request menu. `Input::header` names the recipient, its squad and the mode (`→ sol (product) · note`).
  The footer (`view/footer.rs`) is derived from the effective bindings through
  `Action::footer_rank` (`None` keeps an action out of the footer, in `?` help only) with a
  fixed `↑↓ move` first; `s switch` shows only while `App::tabs_overflow` (set by the tab
  painter) is true. HOME's key line (`home/bar.rs`) follows the same list. `,` settings
  rows (Theme, View, Token window) are `settings::Pick`s beside the config entries, never
  config keys. After a successful settings save, their config-backed values are
  resolved again through the same Config readers as opening; the component keeps
  selection and the token-window row keeps its live session value. Failed saves
  publish no new quick-row values. Presets bind `t` to `home-replies` and no `r`, `T`, `l`, `w` or `talk`;
  fixtures that exercise those actions use `action::with_action_keys`.

## Composition, folds and scrolling

- `split` validates trees and reading/focus order (`MAX_DEPTH` 3); `board::composition`
  owns geometry. It admits its embedded XML scaffold before raw mode, instantiates
  named prototypes from Board/Split and effective folds, and dispatches rectangles
  to existing rich painters. There is no runtime layout-file loader. Folded groups
  propagate title footprints; expanded siblings share the remainder through one
  TUI flex computation. Nested percentages retain fractional parents until
  cumulative edge rounding. Tabs mode reserves a shrinkable bar and a one-line body
  minimum. The view cache keys viewport, effective Board, folds and tab focus.
- `App` resolves configured folds from the full-width body measurement supplied by
  the draw owner. Per-tab session overrides win over defaults at either width,
  survive unchanged refreshes/switches, reset on changed Board and drop with removed
  tabs. Toggling never changes the configured tree or persists folds. The `action`
  owner parses named-pane toggles; binding precedence and factory arrangements
  belong to [config-and-effects.md](config-and-effects.md#views-pane-arrangements).
- Title presses toggle before row dispatch, without row selection or double-click
  history. Folded bodies publish no row/scroll hits. Collapsing focus chooses rows,
  else the next expanded pane; all folded means no body focus, and expanding then
  focuses that pane. Notes focus expands notes first. A sole expanded pane is
  borderless, while its folded title remains clickable.
- `board::scroll::Scrolls` owns pane offsets, clamping, viewport reservation and
  overflow painting. Each frame records drawn pane areas for pointer focus/wheel
  routing. Rows supply every continuation's start/hit and reveal the complete
  selected visual range while followed, or its first line when taller than the
  viewport. Paging uses visual lines, falling back to records before positions
  have been drawn; painters keep no competing pane scroll state.

## Tabs and retained views

- `view::tabs` measures styled `tab_label` widths for windowing, overflow and hits;
  selection adds no characters. `tabs::arrange` owns order/pins. Window admission
  uses measured group/overflow widths and preserves the current tab even when other
  pins must step aside. Adjacent squad-prefix groups are display-only; prefixes
  have no hit, suffixes retain canonical keys/indices. An opened globally hidden
  tab has no movable `TabHit`. Tab moves persist through `Config::write`.
- `board::pick::Picks` owns process-local admission separately from full arranged
  and hidden inventories. Startup resolves inventory before terminal admission;
  opening picks a tab, unpicking the current tab requests the next pick or HOME
  through the ordinary load path. Refresh prunes removed keys but does not erase
  picks after a failed empty inventory read. Config and `ls --tab` are unchanged.
- `view::tabs` measures the unpicked-squad fold before the picked window and
  recomputes groups over that sequence. Fold attention includes only visible
  unpicked squads, excluding aggregates and global hide. Its separate rectangle
  opens a restricted switcher and cannot be dragged. Switcher filtering belongs
  to `tabs::matching` (prefix, substring, then ordered letters). Its adapter consumes
  effective `pick-tab` bindings before query editing; explicit bindings replace
  local Space, and a non-pick Space binding wins.
- `App` retains visited views. Cached switches display immediately; uncached ones
  retain the shown view marked stale, refusing row actions until replacement.
  The requested tab is underlined without taking shown-view selection. Failure
  restores the shown tab and clears that cue. `view::header` delays the uncached
  spinner by `SPINNER_DELAY` (100 ms), with 80 ms frames; cached switches omit it.
  Effective refresh dispatch precedes text inputs, preserving search/drafts.
  `App::lead` resolves the shown squad/aggregate/HOME lead before ordinary jump.

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
  `RowSend` retains occurrence and opening sender; a chosen request, quoted preview,
  note recipient and draft survive mode changes; the status draft keeps its own field choices and reason. Several requests use
  an explicit picker. Submission revalidates sender, occurrence, actual lead or
  chosen open request against acquired data before the existing public send effect.
  Pending-only Reply opens an annotation to that member (`note_member`), never
  clears pending or acknowledges it. Empty submission and cancellation send nothing.
- `board/status_update` owns the status form in that composer, measured wrapped
  lines and focus/scroll. `SelectedRead::Status` acquires raw roster metadata through
  the existing cancellable worker, fenced by generation, read revision and exact
  target/actor. No display/provider value becomes an expectation. `membership/status_update`
  freezes UUID, room, namespace and full keys, validates the actor and occurrence,
  then calls unattributed public `identity.meta.apply` once for selected fields.
  Exact old values and absence stay distinct; unsupported legacy values refuse.
  Conflict refresh retains context, clears field choices and needs a fresh submit; ambiguous output locks
  out mutation replay and sends no announcement. Typed `ActionOutcome::Status`
  keeps Refused/Conflict/Unknown/Applied state rather than flattening it to a notice.
  Applied retains only a frozen announcement for explicit notification-only retry;
  focus returns to the first field so queued submit keys cannot retry a notification.
  Unanswered requests remain separate and use the existing one-request composer.
- Row painters reserve input beneath the complete target in the same line stream
  used for starts, reveal and hits. `view::waiting` places the current-frame opaque
  band across the body, or the HOME lead box's inner width, and removes covered
  hits. Headers derive from actual `Compose` recipients/subjects. Unanchored input
  keeps the footer path; ask-lead keeps a docked band and opening sender/squad/lead
  fences. `Compose::ReadLead` is read-only; wrapping, reservation and scrolling share
  `home_leads::message_lines`, and transition to answer/note uses the normal owner.
- `RowFeedback` retains the anchored occurrence through refresh; its `sent` flag
  alone permits `✓ sent`. A removed HOME row stays in the transient display
  projection until the next key. An active status form retains its row through
  applied/unknown outcomes and notification retry without claiming a successful
  send; the acquired model and request state are unchanged. Clearing precedes any
  underlying action after leaving the form.

## Overlays and offline validation

`App::overlay_event` synchronizes help/settings/theme/view/switcher/cron controllers
with one caller-owned `FocusStack` before base dispatch. Controllers own saves,
rollback and worker effects; `picker_surface::State` owns component cursor/query,
admitted scenes and clipped frame maps. Refresh follows stable selected identity;
resize/model replacement invalidates hits. Query edits select the first match;
query/list and selection-only scope fields remain controller-specific.

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
[schema and examples](../../../../extensions/tmt-squad/skills/tmt-squad/SKILL.md#offline-markup-authoring).

## Verification

[development.md](development.md#checks) owns commands and capture requirements;
[TUI development](../../tmt-tui/references/development.md#board-parity-baseline) owns
the frozen parity approval/regeneration gate. Relevant tests live in `view/tests`
(cells, styles, hits, footer, menu and parity), `composition`, `scroll`, `app`,
`markup` and `layout`. Text-list projection is corroborated by the native Squad test.
