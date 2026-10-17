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

`board::terminal` owns raw mode, mouse capture, bracketed paste, negotiated
keyboard enhancement and the alternate screen through
`Screen` and `Guard`. Partial entry is undone; restoration runs on return, error,
panic and TERM/HUP. Keyboard support is queried after the placeholder and before
input starts; only an owned enhancement push is popped. TSTP restores before
stopping and re-enters on continuation with the same draft and observed capability,
without querying the active reader. Worker shutdown follows terminal restoration. Cold launch
scope names resolve active rooms through the existing `rooms.roster` snapshot port
before terminal admission; a valid ephemeral reload handoff
uses its carried UUID scope and does not resolve the original argv names again.
Configuration and worker setup follow the admitted placeholder.
The admitted board selects structured migration notices before entry: the shared
migration decision feeds the App footer rather than writing untracked stderr
cells. CLI invocations retain their once-per-invocation warning stream.

`board::view::render_frame` resets frame-local hits, input placement and scroll
regions once. `view::panes` dispatches workspace snapshots to `workspace::render`
before independent row/pane painters. `board/mod.rs` receives input, snapshots
and deferred events on one channel; snapshots wake painting directly. Redraws
follow state/input/resize changes and changed clock text/spinner frames. Each
paint registers only visible dynamic label budgets and Overview quota rows;
clock checks reuse those projections without roster traversal or source reads.
Clipped countdown suffixes and hidden members do not wake the frame. Geometry
invalidation clears this registry, and painting sets the next comparison base.
`App::workspace_frame_rows` shares one immutable tree projection during paint;
its scope restores nested paints and unwinding. Input and snapshot updates
retain no row cache, so changed membership/filtering always resolves fresh.

Workspace tree/detail and conversation chrome use shared Strip/Scrolls.
`pane_chrome` projects App focus into an accent heavy top rule and bold title; other titles/rules are
Muted. A retained list selection keeps its cursor but loses the active highlight
and dims while content has focus. NO_COLOR adds a title marker (`▶`, or `>` with
the board's ASCII glyph switch), independently of the rule and bold title.
Pane titles consume one rule row with one-cell side padding, without a bottom
frame or decorative blank row. App's effective table owns Tab/backtab pane
switching; text and modal owners handle their raw events first. The same
workspace Placement feeds paint and frame-title pointer focus; invalidation
revokes its authority until a new paint. Conversation reserves its WP1 composer
band before insetting the response; the shared title replaces its former header
row and is the single stacked separator. Header actions retain complete-visibility
and UUID/scope guards over painted coordinates. Conversation and inline input
retain their WP1 owner and opaque covered-hit mask. Review/Result uses a
square shared modal. Incidental frame cells remain Dim; receiving focus and
selection stay distinct. Frozen row/HOME provenance and decoded fixture approval
are owned by the verification reference.

`board/recovery` owns the saved-snapshot modal's selection, private public-JSON
projections, read coalescing, exact digest/mode confirmation and retained reports.
Core's public workspace commands own capture, storage, live checks and effects.
Typed recovery jobs use the existing ordered action lane and bounded Core client;
only restore preserves completed nonzero JSON for partial-result presentation.
The App's effective table admits snapshot actions only in Agents list focus,
after Config binding precedence. Its modal router consumes close, and frame
invalidation revokes pointer/confirmation geometry. Reload waits for an open view,
pending work or retained Unknown report.

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
`App::carry` applies it on the first fresh workspace view (`carry.rs`): shell,
tree expansion, exact UUID selection, Reader focus and scroll state, plus session
scope and hidden-room reveal. Carried visibility overrides original launch argv,
including a scope cleared before reload. Cold retained view hints cannot set
visibility; reload triggers and admission remain unchanged. A missing
workspace target remains explicit and grants no adjacent recipient authority;
a failed view drops the pending handoff. Version 3 carries no named-tab picks or pane folds;
old versions are consumed as normal-start misses. The snapshot holds session
state only, no squad data or durable preference policy.

## Source rows and workspace identity

- `rows::Rows` owns authored positional coverage, including empty cells and
  spans. Uncovered columns remain value sources and publish value-only metadata;
  `Column::display` ignores their sizing. Independent text lists use the existing
  `ListSizing` and `tmt-cli-style::grid` owners. Row metadata, source formatting and
  hidden-column validation remain Config/CLI contracts, not workspace geometry.
- Offline layout admission uses the registered source grammar. Its binding tests
  verify acquired source values, stable identity admission and refusal of missing
  members without retaining the retired row painter, Grid or projected scene
  adapter. The workspace paints admitted row text from its acquired snapshot.
- `display_rows::project` supplies the independent document order to text `ls`:
  lead first, then authored sections, preserving repeats. `each_row` includes the
  lead in source waiting, staleness, usage and color projections. `RowOrigin::Lead`
  carries no section binding, and public JSON keeps it outside `sections`.
  The retired UI-only member rule/count is absent; workspace tree rows and counts
  derive from the acquired UUID roster instead of this document projection.
- `workspace::render::tree` projects viewport UUID targets through `tree_row`.
  `workspace::columns` owns fixed tracks and whole optional-column removal;
  TOK 1h is visible with STATE, owned-timer count and a local lead marker when
  they fit. Connectors follow filtered siblings. Selection fills the whole row
  at normal weight, including when the conversation has focus; attention belongs
  to state glyphs. `agents_focus` owns selected member/squad projection through
  shared identity/stat/usage/timer/exchange/input components. Digest choices and
  timer counts publish only complete clipped current-frame hits. Unknown readings,
  membership and timer totals remain distinct from observed zero. The
  [workspace surface](board-home.md#shell-geometry-and-input) owns its layout.

- Selection words: `Look::selected_words` is the one owner of what a real selection
  background (`tmt`, `tmt-light`) does to colors. `render_frame` runs it last, over the
  finished buffer: on a cell with the selection background, `muted`, `dim`, `accent`, `link` and the
  state colors (waiting, working, review, blocked) paint in `text` unless the cell is a
  single mark glyph (`MARKS`; a dim or muted mark still becomes `text`). Surfaces name no
  word/mark classes. Agents/Overview State rectangles are frame-local exclusions
  supplied to that same pass while no blocking overlay is present; the state word
  keeps its foreground under the selection background. Resize/model replacement
  and each frame clear these rectangles. The reverse
  fallback stays with `row_span`. The contrast test in `tmt-cli-style` pins text 4.5:1
  and marks 3:1 on that background.

- `board::row_detail` owns the typed entity fields/reply projection and shared
  renderer used by Timers detail and the response reader. Labels/gutter, bounded
  fields and Markdown previews share wrapping and roles; current workspace
  response placement belongs to `conversation`, not inline member expansions.
  The retired inline row overflow click targets and their frame cache are removed.
  Full bodies arrive through the cancellable worker/request cache, fenced by exact
  sender/recipient/request. A compatible `view-reply` action opens the shared
  read-only modal/ScrollState reader. Source selection and annotation revalidate
  the exact request and raw body before producing an effect; reading alone sends
  and acknowledges nothing.
- Row composer and footer: `Input.compose` is the current mode (answer, note, talk or status)
  and `Input.others` the rest in cycle order; Tab (`cycle_mode`) rotates them and
  refreshes the quote. `attach_row` builds the list from the row (`other_modes`), so a
  mode appears only when available; multiple answers use the same explicit request menu. `Input::header` names the recipient, its squad and the mode (`→ sol (product) · note`).
  Workspace dispatch, help and footer resolve the same Config key table. Active
  text, search and completion keep their input; non-typing panes and overlays
  accept effective destination actions after the overlay's own back path closes it.
  Esc closes one level and stays at the base list; it is not bindable. Default `q`
  quits the board body, backs out of an overlay and types in an active editor.
  Footer fitting follows `view::footer::hints` below.
  Opaque modals hide the base hints;
  notices/error precedence remains with the existing frame owner. Theme and Token
  window actions reuse their existing Config writers, while Auto/Side/Stack writes
  only workspace preferences. Reading, preview or cancelling sends nothing.

## Composition, folds and scrolling

`workspace::render` places the tree and conversation/detail from Auto/Side/Stack
preferences and actual minimum usable sizes. Squad expansion is keyed by room
UUID, not a per-tab pane fold. Retired factory/pane/tab settings remain authored
configuration with located migration notices; they do not create a legacy UI.
Config bindings and direct source/menu actions share that retirement admission,
so an action cannot reopen the old surface or implicitly redirect to a lead.
The independent split/markup readers remain separately tested for CLI projection
and offline authoring contracts. The old grid/boxed-list painters, width/model
carry cache and duplicated renderer timing harness are removed; workspace
performance measurements use the actual App/render/buffer-diff path.
The former factory-pane composition/cache is removed with its frame route;
CLI factory-value tests do not invoke interactive geometry.
The factory-view picker, worker opening/save effects and overlay state are also
removed. Workspace Settings offers Auto/Side/Stack, Theme and Token window;
closing its menu restores the same selected target and Reader focus. Independent
Config setting admission/save/reset does not expose a factory-view picker.
Pane title-fold hit maps and per-tab fold overrides are removed; current clipped
workspace controls and response viewport owners alone publish pointer authority.

`board::scroll::Scrolls` owns pane offsets, clamping, viewport reservation and
pointer areas. Tree starts, reveal and clipped hits share that viewport. Following
reveals the selected visual range or its first line when taller than the viewport;
paging uses visual lines. Review/Result's shared ScrollState owns its read region.
Resize/model replacement invalidates controls before another action can use them.

`board::discovery` owns explicit
Intro/Scan/candidate/detail/form/confirmation/result presentation. Config
supplies the Agents-list-only `discover-agents` default `s`; dispatch,
footer/help/menu and the empty-pool action use the effective table. Only public
`discover` precedes confirmation; known UUID examples reuse acquired/verified
evidence, with explicit UUID input for other identities. The existing ordered
lane owns finite Scan/guarded-add tasks and typed completions. One active scan
plus one coalesced latest scan owns its generation; submitted writes and
partial/Unknown results survive close and defer reload. Shared modal/list/scroll
geometry and Editor own current hit/caret authority. Full visible confirmation
freezes the original target/socket/digest/selector/lifetime once; Core alone
validates and binds. The accepted public roster projection reveals the returned
UUID without a local roster mutation or readiness claim.

## Pane resizing

`pane_resize` owns domain-named ratios and disposable painted boundaries; existing
placement owners apply ratios before both paint and pointer focus consume their
rectangles. Only admitted visible pairs publish clipped divider/rule targets.
App routes a drag ahead of row activation, keeps its starting geometry until
release, blocks exec reload while held, and revokes it on external geometry
invalidation. Hover changes separator
strokes only. Double-click identity belongs to a boundary, independently of row
clicks. Hidden panels and space below usable minimums publish no drag target.
`view_state::Controls` carries ratios through the existing named-layer writer and
exec snapshot. Defaults keep exact original geometry; temporary resize clamps do
not replace retained ratios. Effective bindings supply optional resize actions;
no raw key interception or default binding is added.

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
Ordinary restart reads the selected disposable view layer under the existing
per-user Ops cache after the opening frame. The default keeps `board/view.json`;
`ui --view <name>` selects `board/views/<name>.json` (1–32 lowercase ASCII letters,
digits or hyphens). Each bounded 64 KiB file retains destination, expanded squad
UUIDs, pool expansion, exact UUID selection, committed search, Overview/Agents
filters and sort, authored chart Range/Each, the shared quota-window choice, and pane scroll offsets clamped
by the next draw. Missing or unknown quota choices restore the default 7d without
discarding other valid controls. Old v1 hints
without the additive controls/scroll fields still restore without a migration.
Reads create nothing; first publication creates the layer. Same-name boards use
last-writer-wins publication; different layer files never affect one another.
Interactive `--reset-view` removes only the selected hint and opens cold; it is
refused without a terminal and with `--reload-all`. Exec carry wins and does not
repeat a reset from original argv. Nondefault layers appear in the shell header. Global attention wins over the
optional layer caption, which ellipsizes first and disappears only when no name
cell fits.
For destination/selection, exec Resume wins, then an explicit startup squad,
then the hint; an explicit squad still restores compatible expansion hints.
The first fresh Complete roster applies it; partial/failed views neither apply
nor overwrite it. Complete-proven removal uses the ordinary cold default; an
existing hidden target remains hidden and refuses actions. Changed projections
coalesce on the existing worker at two-second intervals, with one exit flush
after terminal restoration; pane-kill keeps the last published hint. Active
search drafts do not publish; only committed search is retained. Composer drafts,
launch scope and hidden-room reveal stay session-only. In-place reload carries
the layer and controls through the same App seam, preserving scope and explicit
hidden/missing action refusal. Pins, hidden squads, panels, keys, theme and the
startup-tab default remain shared Config preferences. Authored Config sort is a
cold default; interactive sort changes only the current layer.

`member_links` owns the current-frame resolved-reference rectangles and conversation
read-focus cursor. It reuses the existing action menu surface within an adjacent
clipped rectangle; no separate popup/widget family or source authority is added.
See [member references and source](board-notebooks.md#inert-text-and-independent-notebooksource-utilities).

## Decisions and composers

- `attention::waits_on_you` owns the acquired pending/request predicate. The
  workspace tree does not disclose private waiting previews. Reply/annotation
  actions use the selected target's acquired row and exact source request; reading
  or annotating never clears pending data. Visible-clock admission belongs to the
  frame registry above, not unpainted request ages or retired continuation rows.
- `view::footer::hints` uses the effective scoped Config table and typed action
  ranks, independently of menu ordering. `row_allows` rejects unavailable member
  actions; `hint_word` names compatible custom actions. Core hints drop from the
  end, retaining the effective Help/quit tail or Help alone at narrow widths.
  The optional note hint stays beside Chat only when it fits without evicting a
  core hint; Help always lists the action.
  Active input and modals use their own legend; an idle persistent composer keeps
  the effective list/reader hints.
  `board::glyph_guard` checks authored HOME/cron typography.
- `App::input` means active typing in the shared Talk/Reply/Annotate/Status
  composer. `composer` projects the idle boxed editor at the conversation bottom
  without claiming input focus. Default Enter focuses a selected agent's editor;
  active Enter submits, and Esc returns to the list with the draft kept. Group
  Enter still inspects, and explicit compatible bindings retain their meaning.
  `RowSend` freezes the occurrence and opening sender. `composer` retains
  session-only drafts, folded paste and response quotes by saved sender/actual
  recipient UUID across view and membership changes. Acceptance of a request
  clears only the submitted revision, including an uncertain delivery or an
  error carrying a retained request ID; neither permits automatic resend. A
  refusal without a request restores that draft without replacing newer edits.
  The composer stays after Send; a blocking owner masks its passive band without
  losing the draft. Restart persistence is not promised. Submission
  revalidates sender, occurrence, actual lead or chosen open request before
  invoking existing public send effects. Several open requests use an explicit
  picker; empty submission and cancellation send nothing. `conversation` holds
  bounded session-only outgoing turns, truthful typed delivery labels and replies
  correlated by exact sender/recipient/request identity. Acquired agent state and
  freshness remain independent of delivery; delivered means written to the pane.
- Message fields use `tmt_tui::components::Editor` for grapheme-safe multiline
  editing, source-preserving bracketed paste and cached cell geometry. Enter
  submits; distinct Shift+Enter, Alt+Enter, Ctrl+J or retained Ctrl+N inserts a newline, and Escape
  retains the draft. The hint is `Shift+Enter new line` only with confirmed keyboard
  enhancement, otherwise `Alt+Enter new line`. Terminals own IME preedit; committed Unicode enters the same
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
- A visible conversation reserves its composer inside that pane, anchored to
  the exact selected membership. A note can be delivered to the squad lead;
  Status has no message draft key. Neither changes the conversation owning its
  input band. Compact Overview moves an explicit message to Agents.
  `view::waiting` removes only hits covered by the opaque band, preserving
  independent pointer scrolling in other panes. Headers derive from actual
  Compose recipients/subjects; sender, recipient and source fences run at submit.
  Read-only detail belongs to `row_detail`, outside the composer.
- Completion uses the observed Core outcome as one notice. It does not retain a
  second row-mark cache or invent delivery from dispatch intent. Refresh retains
  the workspace UUID anchor; a removed target stays missing without adjacent
  recipient authority. Worker in-flight state blocks reload until effects finish.
  An active status form retains its typed applied/unknown outcome and frozen
  notification retry without claiming delivery; acquired request data is unchanged.

## Overlays and offline validation

`App::overlay_event` synchronizes help/settings/theme/cron/checklist/discovery
controllers and workspace forms with one caller-owned `FocusStack` before base dispatch. Controllers own saves,
rollback and worker effects; `picker_surface::State` owns component cursor/query,
admitted scenes and clipped frame maps. Refresh follows stable selected identity;
resize/model replacement invalidates hits. Query edits select the first match;
query/list and selection-only scope fields remain controller-specific.

Choice cursors are caller-projected `›` cells from the reconciled selected row ID.
Theme uses zero-gap
`[10,1,1fr]` tracks; menus keep the exact key width followed by one cursor cell.
Cron uses a six-cell state/cursor/ID prefix with zero inner and original outer
gaps. Content starts and semantic marks stay fixed. The cron list supplies its
cursor. These cues appear on the first
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
(cells, styles, hits, footer, menu and parity), `workspace`, `scroll`, `app`,
`markup` and `layout`. Text-list projection is corroborated by the native Squad test.

## Checklist controller

`App::context_menu` owns the context action menu for configured Menu bindings and
`,` → Workspace settings → Actions. It adds Checklist before
member-row admission; no selected member exposes only context actions. Workspace settings exposes Actions after layout/Theme/Token window choices. Closing that menu returns directly to the board.

`checklist::{controller,forms,surface,load}` owns one centered square modal and its
room chooser, independent filters, item selection, authored fields and exact
confirmation. A visible selected squad or its member in Agents/Digest freezes
the acquired room UUID; Overview, Common pool and hidden/missing targets require an explicit
room choice. Closing retains workspace UUID selection and focus without legacy
row-index restoration. Default scope is Open / Squad-wide / archived excluded.
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

The scoped Timers modal never expands merely on selection or focus. Its ListState
owns navigation, scrolling and hit geometry; `row_detail` paints into its reserved
inline detail lines. Scoped `e` toggles, `E` edits, and the single cron `KEYS` table
owns footer words. `E edit` appears only when a job is selected.
