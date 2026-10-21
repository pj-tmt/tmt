---
name: tmt-ops
description: Lead a TMT squad - read the squad board, keep member state current after every dispatch and reply, and agree conventions with the user instead of guessing.
---

# TMT squad (for leads)

Use this skill when you lead a squad: `tmt ops squad ls` shows you as the
squad's `lead`. A squad is a TMT room named `squad-<name>`. Each member's board
fields are that member's identity metadata `squad.<name>.<field>`. The authoritative roster and board fields stay in TMT; optional age observations
live in a disposable Ops cache. `tmt ops squad` has the alias
`tmt ops sq`. Bare `tmt ops` prints Ops help. Bare `tmt ops squad` prints the member list followed by
`tmt ops ui opens the board`; `--json` prints only the same document as
`tmt ops sq ls --json`. Run `tmt ops ui` to open the interactive board.

## Read the board

```sh
tmt ops squad ls --json [--squad <name>]
```

`tmt ops sq ls --tab <name>` returns one configured or built-in board tab's document,
with the independent CLI rows, sections, attention, columns and lines. Hidden
squads remain included. `leads` lists each squad lead; `all` lists squad summaries.
`--tab` cannot be combined with `--squad` or `--refresh-fields`. An unknown tab
returns `SQUAD_TAB_NOT_FOUND`; an empty tab has an empty rows array.

Define a cross-squad member view in `ops.toml`:

```toml
[tabs]
order = ["tab:needs-me", "leads", "all"]
pin = ["tab:needs-me"]

[tabs.needs-me]
filter = "pending or waiting_on_you"
sort = ["squad", "name"]

[tabs.needs-me.bind]
enter = "jump"

[[tabs.needs-me.section]]
title = "Blocked"
filter = "state = blocked"
sort = ["squad", "name"]

[tabs.needs-me.section.bind]
o = "tab"
```

Run `tmt ops sq ls --tab needs-me --json` for that view. It includes leads and
members across all squads, including hidden squads. Names use the squad-name
rules; `leads`, `all`, `colors`, `order`, `pin` and `hide` are reserved.
`tab:<name>` distinguishes a user view from a squad in `order`, `pin` and
`hide`; unplaced views follow the default tabs in definition order. These independent views remain reachable through `ls --tab`; they do not add
interactive shell tabs.

Filters use the section language: field presence, `=`, `!=`, `and`, `or`, `not`
and parentheses. Fields include `squad`, `name`/`member`, `state`, `pending`,
`presence`, `lifetime`, `role`, `task`, `activity`, projected custom fields, and
`waiting_on_you` (present when a member waits on the recorded user). With no
recorded user, that field is absent. Sort keys ascend by default; prefix `-` to
descend. Missing values stay last; bound numeric values retain numeric order.
The view filter selects rows before sections. Each matching section may repeat
a row, with unmatched rows following untitled; an omitted filter selects all.
Tab bindings override global bindings, and section bindings override tab bindings.
Views use the squad/member/state/task row preset; per-tab `rows` are unsupported.
At most 16 user views and 16 sections per view are allowed. Invalid definitions
fail Config reading with their setting path, including hidden views.

Failed roster or inbox reads set `partial: true` and include `failures` with
source, optional squad, and error code/message. Text lists show warnings; the workspace retains its own partial-read evidence. Available rows remain visible, so an empty partial
view must not be treated as evidence that nobody waits on you.

With `--squad <name>` the document is that squad's; without it, it is always
`{squads: [...], you}`, one document per squad in name order (even for one
squad or none), so read `.squads[]` unless you pass `--squad`. `columns` and
`lines` describe the independent CLI row projection: each column's field, title and sizing, and
the fields each line of a row shows (`{field, span}`, field null for an empty
cell). These values remain accepted and validated; they do not compose the
workspace's tree, conversation or pending-continuation geometry.

- A cell has optional `token`, its semantic theme role, which overrides
  threshold/provider colors for that cell. Team publishes
  `lines[1][2].token = "waiting"` for its pending second line; unstyled cells
  omit the key. This is decoration metadata, not a changed row value.
- A column's `width` is null, a cell count or a percentage string such as
  `"30%"`. Covered-track percentage widths total at most 100% and resolve against data width
  after the CLI grid's gaps. `min`/`max` remain cells; this geometry does not
  arrange the workspace.
- A column has optional `valueOnly: true` when no row line covers its positional
  track. It remains a value source but reserves no CLI grid width; other columns
  omit this key. See Columns and row lines below.
- A column has `overflow` only when configured: `"ellipsis"` or `"wrap"`.
  Without it, cells use ellipsis. Wrapped continuations align to the cell start.
- A wrapped column has `max_lines`, its bounded visual-line count (1–8, default 2).
  The last line uses an end ellipsis if cut, even with `truncate = "middle"`. This differs from document-level `lines`,
  which describes the configured row grid.
- Text `ls` keeps legacy natural sizing unless a shown column opts into percent
  width or overflow. Opt-in text uses the shared grid and fit rules; a pipe's
  budget is natural data widths plus gaps before priority hiding, so text may
  wrap, truncate or hide columns. JSON row values stay full.
- `squad`: `name`, `roomId`, `layout` (`crew`, `pr-queue`, `minimal` or `team`),
  `lead` (a row, or null) and `attention`: `state` (`waiting`, `blocked` or
  `normal`), `waiting` (members that owe the user a decision or wait for an
  answer) and `blocked` (members in the `blocked` state). These remain CLI/source
  facts; the Agents Usage band and Measure view own the dashboard aggregate, while the squad tree displays
  observed membership state. The common shell shows UUID-deduplicated online,
  waiting and blocked observations on every tab; tab labels have no attention badge.
- `sections`: always a list. Unless the user defined sections, it holds exactly
  one section with `title: null` containing every member except the lead. With
  user sections, members that match none follow in a final `title: null`
  section. Text `ls` shows the lead first; JSON never moves it into `sections`,
  and its source member counts exclude it. The workspace's UUID roster includes
  the lead once in the squad count rather than adding a second synthetic row.
- `squad.noteAnnotations` is optional: open notebook-line requests from the
  recorded user to the current lead, as `{requestId, line, quote}` with a
  zero-based source `line` and bounded sanitized `quote`. It is absent when
  none are observed. This remains an independent source projection; the workspace
  uses the selected agent's response annotation flow rather than notebook markers.
- `squad.idleAlert` is `{enabled, afterMs}`: the squad's effective
  [idle alert](#alert-the-lead-to-an-idle-member) setting, present for every squad.
- A row, the lead's included, has the optional `idleAlert` object only while that alert is on and
  Core reports the member `idle` for at least the threshold: `{state, sinceMs}` where `state` is
  `over` (the lead is told or due to be) or `excepted` with `reason` `pending` or `hold` (no notice).
  `sinceMs` is Core's idle start in both states. It is absent, never null, otherwise.
- Each row has `id`, `name`, `lifetime`, `presence` (`active`, `offline` or
  `unknown`), `pane`, `activity` (self-reported status, or null), `state`,
  `pending`, `fields` (the `squad.<name>.*` values except the internal
  leadership marker and retired row note, by field name,
  with the user's column sources, formats and field providers applied;
  these strings are already display text and must not be formatted again),
  `failed` (fields whose provider failed; they show `?`), `annotation` (the user's open note
  about this row, or null) and `waitingOnYou` (open requests from this member to
  the user), and `staleness` (observed task/state age).
- Every row has a separate `staleness` object, and `squad.notesStaleness`
  describes the lead's notebook: `state` (`disabled`, `unknown`, `fresh`,
  `stale`), `unchangedSinceMs`, `ageMs`, `activityAfterUpdate` and `reasons`.
  Unknown timestamps are null. Age is observed raw task/state or exact notes
  content age, not file/core modification time. First observation starts the
  clock; never infer older age from a cursor or missing evidence. Text `ls`
  labels stale rows and notes with their age. See the reminder configuration
  below for reset and evidence limits.
- A row with `pending` owes the user a decision. It is marked ◆, and the crew
  and team layouts list it first.
- A row has the optional `digest` object when Core supports the digest policy read:
  `{active, digestUntilMs, remainingMs, heldCount}`. Times are milliseconds; inactive
  windows have zero remaining time. Failed/older Core reads omit it. The board shows
  `digest 30m · 2 held`, rounding remaining minutes up (then `1h20m`); zero held is
  omitted. Complete chip units fit the current tree/conversation budget; trailing
  labels are omitted when space is short. There is no inline `e` expansion.
- Extensions can label member rows. The board asks each extension in `[labels] sources`
  in `ops.toml` (default `["digest"]`; `[]` turns it off) for `tmt <name> status --json`,
  in the background every few seconds, and shows the labels it supplies after the
  member's name, led by the extension's name in `muted` and in the roles the extension
  chose, such as `digest Auto · 3 held · Due now`. A member with supplied labels shows
  them instead of the `digest …` chip, never both. Nothing waits for it: an absent,
  slow, failing or malformed extension changes nothing, and labels from one that stops
  answering disappear after 30 seconds. Labels give way to the member's name and the
  whole state word: narrow rows drop trailing labels first, show the extension's name
  only with its first label, and show no chip when those do not fit. Read the
  extension's `status --json` for its complete current labels.
  The sources are read when the board starts; restart it after changing them.
- A label may offer a choice; its chip ends with `▾`, such as `digest Auto ▾`. Click the
  chip, press `D` (`digest`, rebindable) on a member row, or choose Digest in its
  action menu, to open that member's choices (for the digest extension: Auto, 1 minute up to
  1 hour, and `Custom…`), the current one marked `(current)`. Enter or a click runs a choice; the wheel and Up/Down move the
  highlight; Esc closes. The click and the wheel work in every action menu; a click
  outside its entries does nothing. `Custom…` asks for a duration such as `20s`, `10m` or `1h`;
  empty text goes back to the default. Only the recorded user and the squad's lead may
  change a digest, as with `tmt ops digest`; anyone else is told so and nothing runs,
  and the check is made again when the choice is applied. A member without an offered
  choice, or no extension at all, says so on `D` and has no Digest menu entry. The
  extension, not the board, decides what is offered and what runs: the board starts
  `tmt` with the extension's own arguments, the chosen text as one argument, never
  through a shell, and shows the extension's refusal as is. The chip updates at once
  after a change.
- A row has the optional `colors` key only when a cell has a color:
  `{field: theme token}`. `colors.state` holds the resolved state token; other
  keys come from the user's column thresholds or a field provider's suggestion.
  Colors only decorate; read the values.
- States come from the layout: crew and team use `working idle blocked review testing
hold`; pr-queue uses `preparing ready sent merged`; minimal has no fixed list.
  Color and order resolve through exact `[squad.<name>.states]` entries (including
  layout presets), then the first matching `[[squad.<name>.state_patterns]]`,
  then the default. Patterns require `match` and `color`; optional `sort` is
  0-999 and `ignore_case` defaults to false. `*` matches any run, `?` one Unicode
  scalar, and other characters are literal. Case-insensitive matching compares
  each scalar's Unicode lowercase form. Limits: 64 patterns per squad and 256
  UTF-8 bytes per nonempty match. An exact entry wins entirely; unspecified
  pattern sort ranks after ranked states. State text and tab attention stay the
  same. Do not change the user's vocabulary without asking.

`presence` is observed by TMT, not reported by the member. `activity` is what
the member reported about itself. CLI row marks and state vocabulary remain
independent source facts. The workspace labels observed presence as `active` or
`offline`; missing presence remains unknown.

A request tagged `[<squad> · <member>]` from the user is an annotation: a note
about that member for you to act on. Answer it with `tmt reply` as usual. Never
edit the user's notes for it.

## Startup tab

`tmt ops ui --tab agents|digest` opens a fixed destination; `1` and `2` are
aliases. The retired `overview` opens Agents. The flag wins over the global
`[board] startup_tab` preference, which accepts `last` (default), `agents` or
`digest`. `last` keeps the
selected view layer's saved destination. Edit **startup tab** in Ops Config or
run `tmt ops sq config set board.startup_tab digest`. Invalid authored values are
ignored with a notice and left untouched; an invalid flag or edit is rejected.
In-place reload keeps its current destination rather than reapplying this cold
startup choice. Without an interactive terminal, the flag validates and the
existing listing behavior is unchanged.

## Retained view layers

`tmt ops ui --view popup --popup` opens a separate retained view; omit `--view`
or use `--view default` to share the everyday board's layer. Names are 1–32
lowercase ASCII letters, digits or hyphens. Boards with the same name share their
last saved view; different names keep independent destination, UUID selection,
expanded squads, Agents sort and filters, committed search and pane scroll.
Only nondefault names appear in the header: `OPS / UI · popup`. Global attention
counts take priority: the layer name shortens with an ellipsis and disappears
when there is no room. The shared top bar also shows held provider limits and
selected-member context; see the [TUI guide](../../../../design/tui-board.md#frame-and-view-composition)
for fitting and unknown-value rules.

`tmt ops ui --view popup --reset-view` clears that layer before opening cold.
Reset requires an interactive terminal and cannot be combined with `--reload-all`;
in-place reload carries the current layer without resetting again. A layer is
created on first saved view. The default uses the existing Ops cache
`board/view.json`; named layers use `board/views/<name>.json`. These are disposable
hints, restored only against the first fresh complete roster. Pins, hidden squads,
panels, keys, theme and startup-tab preferences remain shared. Authored Config sort
provides cold defaults; interactive sorting does not change Config. Launch squad
scope, hidden-room reveal and composer drafts are not retained across ordinary
restarts. Active search drafts are not saved. Without an interactive terminal,
`--view` validates its name and leaves the existing listing output unchanged.

## Resizing panes

Drag a visible divider or section rule to resize adjacent panes. Hover brightens
its line and shows ↔ or ↕ in the footer. Each pane keeps its usable minimum;
double-click the same divider to restore its default. A hidden panel has no
resize target. Ratios are saved in the current view layer, reapplied on terminal
resize, and carried through in-place reload. Temporary minimum-size clamping
never overwrites the saved ratio.

Keyboard resizing has no default keys. An authored binding can use
`resize-pane <boundary> <direction>`, with `left`/`right` for columns,
`up`/`down` for rows, or `reset`. Each step moves about 5% of the shared space.
Boundaries are `agents-columns`, `agents-rows`, `agents-usage` (the Usage band's
height), `digest-columns` and `digest-rows`. The retired `overview-columns` and
`overview-usage` still parse as `agents-columns` and `agents-usage`; the other
`overview-*` names are dropped. Only a currently visible,
resizable boundary accepts the action. For example,
`[bind]` with `r = "resize-pane agents-columns right"` widens the Agents list.
Tab and other authored bindings retain their ordinary meanings.

## Member links

Write `#tmt:<canonical-name>` or `#tmt:<identity-UUID>` in message text to refer to a
member. Known references display the member's name as an underlined link; unknown
references stay literal. Links do not broadcast or change the message recipient.
Conversation responses, outgoing messages, inactive composer previews and Recent
exchange previews use the same acquired identity inventory. Copy and annotation
keep the original markup and exact source.

Click a link, or use left/right in the conversation's read focus and press Enter,
to open its adjacent menu. The first arrow selects the first visible mention. Tab
and BackTab keep switching panes; authored bindings take precedence. `j` uses that
member's configured host action; unavailable bindings explain why it is disabled. Enter shows the member in Agents,
expanding its squad and keeping the current draft. This explicit navigation clears
session search/scope and reveals hidden squads without changing shared preferences.
Esc or a click outside closes the menu. The menu flips above a mention near the
bottom edge and never covers it. NO_COLOR keeps the underline.

## Reloading open boards

An open board reloads itself in place, keeping its pid, shell section, exact selection, scroll,
expanded room UUIDs, pool expansion and search, when the `tmt-ops` it was started
from is replaced (for example by `tmt extension upgrade ops` or `tmt upgrade`) or
when `tmt ops ui --reload-all` asks every open board to. The board waits until it
has been idle for a second and nothing is open: a composer, search, overlay, reader,
pending request or loading view holds the reload, and the footer adds a dim
`reload waiting`. After the reload the footer says `Board reloaded · tmt-ops X`.
Boards started before this behavior shipped need one manual restart.

## Board appearance

The board opens one `OPS / UI` shell: `1 Agents`, `2 Digest`.
Shortcut caps precede tab and filter labels and show effective configured keys;
unbound actions have no cap. Selected tabs use a brighter neutral cap.
Agents holds the whole workspace: a full-width Usage band (global usage and provider
limits) above two columns. The left column lists squads, agents or pinned ones; the
right column shows the selected target as Talk (the conversation) or Measure (TOK in the saved range,
STATE, DIGEST and TIMERS cells, the soonest timers, the Digest table and who talks to
whom). `f` cycles the left chips Squads / Agents / Pinned and `t` toggles Talk / Measure;
both controls sit on their pane's top rule. The shell attention bar stays global.
Digest keeps the same identity context and existing Core-owned policy controls. A
squad's theme applies to Agents/Digest.
Member conversation status, selected-squad usage and the Measure Digest table show `Est. ctx avoided`: Digest's estimate of context input tokens avoided by batching,
not billed tokens, cost or turns saved. In workspace settings (`,`), choose the shared
Digest estimate window: 24h (default), 7d or all time. The saved preference is
`workspace.digest_window` (`24h`, `7d`, `lifetime`). `—` means unavailable; known zero
is `0`. Partial sums show `(N of M)` included members. All time begins separately
for each member when Digest started recording; the detail names those local dates.
The Measure Digest table sums the agents it counts and follows with the estimate only
when it fits whole (the short `Est. avoided` form first); it is absent while
Digest status is unavailable.
Click any estimate to cycle its unit on every view at once: tokens (default), percent
(`12% of input`) and cost (`≈ $12.40 at list price`). The saved preference is
`workspace.savings_unit` (`tokens`, `percent`, `cost`), next to the window.
Percent has no all-time form, so all time shows tokens and the cycle skips it.
Cost is an estimate at list price for uncached input, never money saved or a bill;
the Digest tab's Details list the price table, where it came from (fetched, cached or
built-in) and each member's quote and share.
A sum of percents needs one shared interval and a sum of costs one price table;
otherwise it reads `— (intervals differ)` or `— (prices differ)`. The `savings-unit`
action has no default key: bind one under `[bind]` and its hint appears beside a
painted estimate, or choose `Savings unit` from the Actions menu.
The effective destination keys work from any non-typing pane or overlay; an
overlay closes through its own back path before switching. Active text, search
and completion keep `1`/`2` as input.

The left list's rule carries the Squads / Agents / Pinned chips and the shared search
field; the totals sit at the bottom right of the list. Squads is the tree: a squad row
shows its lead-first TEAM strip in STATE, TOK 1h from existing HOUR readings, and its
now text (a task, or "waiting on you" when a request is pending, never the preview) in MODEL.
Agents is the flat list of AGENT, SQUAD, STATE, TOK 1h and MODEL. Unread agents come first, then waiting agents, ordered
by oldest proven request, then unknown age and name/UUID; no waiting duration is
shown. Blocked state alone does not imply waiting. A squad selection messages its
observed lead; an agent selection uses that exact member. Missing evidence refuses
rather than choosing another recipient. Search and refresh retain hidden UUID
anchors. `e` opens full observed member values. Authored sort changes the current
layer within unread/waiting/other groups. Retired summaries, Recent exchanges and
the separate Needs you section are absent. Unreported data stays unknown.
Usage sits above the body panes and shows
the selected window and total. Independent Range and Each dropdowns use Enter to
open/pick, arrows to navigate and Esc to close; the current choice is checked.
Range offers 1h, 6h, 24h, 7d and 30d. Each offers Auto, 5m, 15m, 30m, 1h, 2h,
6h, 12h and 1d, hiding intervals longer than the range. Choices exceeding 720 bars
stay visible but disabled, with the minimum interval explained in the menu footer.
Auto picks the finest interval fitting the plot width and 720; fixed choices
survive resize. Range changes keep a valid fixed interval or choose the closest
admitted one. These authored choices are saved per view layer, separately from
configured token-meter windows. Long ranges use retained Core observations;
history before collection is unknown, and gaps are never filled with zero.
Muted TOK and % of all headings label the numeric tracks; quota retains its
independent 7d/5h chips. The share track is eight cells wide.
Each acquired driver has a neutral-grey trend row, observed total and share;
unknown provenance has its own muted row. Shares include unknown tokens in the
denominator. Trends start immediately after the names; unused cells follow the
last exact bucket and time labels follow the drawn width. Fixed intervals that do
not fit use one-cell bars and horizontal scroll over the already-loaded series.
Arrows and wheel move the window; Home/End reach oldest/newest. Muted ◂/▸ marks
show off-screen history. Hover or keyboard selection replaces the title with the
bucket span/total and the numeric columns with its provider values, with ~ for
partial counts. A neutral column and axis ▲ mark the selection. Click or Enter
pins it; click again or Esc unpins. There is no Usage Details window. Provider quota occupies the same row; its header chips select
one remaining-balance window for all providers. That choice is saved per view layer. The Subscription quota toggle controls these inline cells;
there is no separate Subscriptions block. Usage has a fixed header position. With no observed token bucket, provider names and available quota remain beside one muted empty-state line and em-dash totals; no axis or invented unknown row appears. A measured zero stays zero.
The default `c` with no squad or agent selected uses `all-timers` and opens all-room Timers, matching the
Measure preview. An authored `timers` binding keeps the selected-scope behavior;
hidden or missing selected scopes still refuse to widen.

Workspace settings → Panels previews each dashboard's visibility and its separate
Conversation/Details switch. Agents sections have fixed positions; its picker
exposes Usage, quota inside Usage, the list, and the side column (Talk / Measure). Digest uses fixed sections and keeps chart/list/details visibility. Save, Cancel
or Reset through the same picker. Keep an interactive surface visible. Cancel
discards preview; stale Save retains it with an error. Closing a pending save
does not cancel the admitted write. A configured-off Agents side column opens
on explicit message focus; Escape returns to rows and retains the draft.
Retired Digest status/usage/report switches and ordering are reported without
rewriting authored config.

Digest has a range summary, Agents by TOK, Exchanges and selection details.
The shared `workspace.tok_range` is `1h` (default), `3h`, `6h`, `1d` or `7d`;
Workspace settings selects it, and clicking the Digest Range control or an
explicit `cycle-tok-range` binding cycles the same setting. Agents TOK headings
and values follow it. Chart Each/Range controls remain their independent viewport.
`all` sums the admitted visible identities and never sends a message or changes a
member's mode. Real members keep UUID selection as usage sorting changes.
Activity shows 24 hourly cells, scaled to each row's busiest hour; unknown is `?`,
known zero is `·`. Whole optional columns yield on narrow terminals; details carry
the hourly strip at 60 columns. Tab exposes Exchanges in the lower pane.
Exchanges counts only talk and reply messages, with sender rows and recipient
columns; `─` is self, `·` observed none and `—` unavailable.
Enter opens the selected member's Agents conversation. D uses the existing
Digest chooser; e toggles details, u opens usage controls and w stays Snapshots.
Hints follow configured bindings. The `≈ $ in` column requires full input-model
attribution and held quotes: input tokens at list price; not a bill. Avoided
estimates remain supplier-owned and show unavailable when their window differs
from the selected TOK range. Source windows and prices remain in details.
Usage and traffic come from bounded background Core reads, even without Digest;
held and avoided values then show unavailable, with `tmt extension install digest`.
Missing, partial and failed/stale observations never become measured zeros.

The Measure view shows up to three soonest timers, with a more
control and the effective all-room shortcut, beside Digest key/value observations.
Paused timers are dim. Empty, stale and unavailable reads stay distinct; a missing
clock never becomes a promised countdown. Timers remain global and say so when
the roster is filtered or scoped. Digest counts and estimates use the visible
scoped roster, retain unknown values as `—`, and show list-price quotes separately
from avoided context tokens. These fixed token/cost rows do not change the saved
savings-unit preference. Small widths stack the lower modules. Wide screens put the selected waiting
agent's exchange and a filled three-row reply field above Timers and a Digest
table: a bold all row for scoped totals, then the agents with the highest observed
TOK 1h, one per row as height allows. Metric labels are muted and numbers align
right without truncation. Unreported cells stay `—`. The fixed 24h estimate columns require matching
supplier observations. The idle field shows its effective start key; factory Enter
replies to the selected incoming request, and authored bindings retain precedence.
The active field uses Enter to send and Escape to retain its draft. Its fill is
the boundary; there is no frame or heading rule. The Talk matrix is hidden until its data ships.

A submitted reply you have not seen shows `● N unread` in STATE and squad team
strips, before working/idle/waiting state. Only the unread glyph is blue; NO_COLOR
keeps the mark and words. The top bar shows your global unread count beside
waiting. `N+` is a lower bound from the existing 50-result page, not an all-history
count; zero observed unread never invents an unread state. Opening the agent's
acquired conversation or successfully jumping with `j` advances your shared read
marker. It survives restart, named views and reset-view. A save failure keeps the
unread count and names the cause; reopen the conversation or jump to retry. These
Ops markers never acknowledge Core results.

The Agents tree keeps SQUAD / AGENT, STATE, TOK 1h, ◷ and MODEL. A Muted `*`
directly before the name marks a lead. Model cells show the observed completed-turn
model, falling back to the session-start model, only with a running driver; a leading
`claude-` is omitted and unknown is `—`. Narrow tables drop the whole model column
before TOK 1h; long models end in `…`. Member token values and squad sums use
the same observed hour. ◷ is the total **timers owned** by that member in the row's
room, paused included: known zero is `0`, unavailable is `—`. Digest/runtime
appear in selection-driven focus. Only attention glyphs carry state colour; a member task state is red only when blocked;
selected rows use a full neutral background at normal weight, or the existing
NO_COLOR reverse cue. The All/Pinned chips and search remain above the list;
ordinary totals belong to the Squads rule.
All keeps Common pool first, then pinned squads and the selected sort order.
`f` cycles the layer-local All / Pinned filter; explicit bindings win. Pinned shows
pinned squads and individually pinned agents with their containing group. `p`
pins or unpins a visible agent or squad in Agents. `v` opens the reply reader; `f` cycles
the Squads / Agents / Pinned chips from either pane.
`/` edits the retained query and filters live; Up/Down move between matches.
Enter keeps the filter and returns keyboard control to the list. The committed
field shows the query, matching/total count and a clickable ×. Esc clears and
closes search in one press; × does the same. Both keep the current exact row
and squad occurrence, expanding its group and scrolling it into view when needed.
A second Esc uses the normal list behavior. Backspace in an empty field closes it.
During editing, match counts remain on the section rule; committing moves the
count into the field. Search reveals matching children of visible squads temporarily;
clearing expands only the selected occurrence's group when needed. Select a squad and use `H` Hide squad (or its Actions entry) to
hide it from Agents. The `Hidden: N` header's show control (`h` by
default) reveals hidden squads only for this session. Before reveal, the line
shows waiting/blocked counts from hidden squads in their state colours. Select a revealed squad and
use `S` Show squad to remove its saved hide. Common pool cannot be hidden. Pins
are retained independently, and hidden squads still contribute to global shell
attention. Explicit Hide moves the selection only after the save succeeds;
search and missing targets retain their UUID anchors.
Expand a squad to see its members; the lead is marked and included once in
its count. A collapsed squad shows its admitted member state strip, lead first;
attention ordering uses its worst observed state. An identity
in several squads keeps each squad's state and one conversation/draft by UUID.
Common pool means no Ops squad membership, not no Core room membership. Partial
or failed reads never prove that an identity belongs in the pool.

With tree focus, ↑/↓ move, Right expands and Left collapses. From either pane,
factory Enter replies to an observed question or messages the selected member or
squad's observed current lead; `j` uses that recipient's guarded host action.
A missing lead or Common pool grants no substitute recipient. The focus shows
member identity/stat tiles or squad Board/Usage/Timers, then `Exchanges with you`
(the currently observed actor-only history). Its filled input follows the last
group with one blank row, and pins to the bottom only on overflow. Key hints stay
in the effective footer, and the link line shows only its resolved target.
These defaults are the same on every host (tmux, Herdr or unknown); a
`[bind.<host>]` table only overrides keys for rows on that host and never changes
the default of a host without one.
Tab moves between the tree and conversation. `1`/`2`/`3` switch
sections only when an editor, search, completion, overlay or conversation has not
claimed the key. The compatible `p` action retains saved identity pins
without changing waiting order. Keyboard sort is opt-in: bind `sort` or
`reverse-sort` in the usual binding table.
In Agents, `p` pins/unpins a squad. `n`
creates a squad, `m` manages a squad or assigns a member, `x` removes only the
selected membership, `D` opens Digest and `c` opens Timers. The compact footer omits Digest, Manage, Create, Remove and Details;
these actions remain in Help and the Actions menu with their effective keys.
`r` reviews Rename for
the selected visible agent by UUID; compatible explicit bindings keep their meaning.
Outside an editor or modal, `w` opens saved workspace snapshots on every tab;
compatible explicit bindings retain their configured meaning. The top bar offers
the same Snapshots action. The default `f` cycles Squads / Agents / Pinned and `t` toggles
Talk / Measure. An authored `overview`, `overview-view` or `overview-filter` binding gets a
located retirement notice and is not applied.
Default `j` jumps through the observed tmux binding only from a tmux invoker; Herdr
focus is unavailable. Standalone `j` opens the shared conversation and Esc returns.
The header names the target and effective action; custom bindings retain their
actual argv. `j` is not tree navigation.
Comma Settings → **Host action (j)** offers Focus agent, directional Swap beside
board, or Custom command. A swap keeps the board focused and exchanges the
neighbour into the selected agent's old slot; select the displaced agent to
reverse it. An agent already beside the board stays put. Swap refuses unsupported
hosts and missing neighbours. Custom commands pass `{pane}` and the verified
`{board_pane}` as single argv values without a shell; `{{` and `}}` quote literal
braces. Explicit bindings retain their meaning in the list and conversation.

`ctrl-r` refreshes without discarding entered text. `/` searches. `?` help and the
footer use the same effective binding table as dispatch: defaults, global `[bind]`,
host overrides, then section overrides. Compatible explicit bindings keep their
meaning, including an explicit `f5 = "refresh"` binding. Whole optional hints drop
when space is short; help remains available.
Menus, completion and editors own their keys and consume close without replay.
Their Esc back action cannot be rebound. At the base list, Esc clears an active
launch scope through the effective `clear-scope` binding; otherwise it stays put.
An explicit base-list `esc` binding overrides that default.
Default `q` quits the board body, closes or backs out of an overlay, and types in
an editor; Ctrl-C quits globally. In Member and conversation focus, default `v`
opens the full response reader; compatible explicit bindings still win.
Pointer controls use only their current visible rectangles.

With Agents tree focus, `s` opens Add agent; configured bindings retain
precedence. An empty Common pool offers the same visible action. Opening offers
Scan or Close and performs no discovery or binding itself. Scan reads the
current tmux environment; review a candidate, choose a new name or exact
existing identity UUID, and choose Temporary or Saved before confirming.
Existing-name text never selects an identity; known choices are advisory
previously observed evidence, and another/offline UUID can be entered directly.

Discovery supports tmux; unsupported hosts, permission/offline failures and
incomplete observations carry their explanations. Runtime hints and recorded
state do not prove readiness. A binding does not install or start a runtime,
extension or hook. Follow the existing [tmux
guide](https://github.com/pj-tmt/tmt/blob/main/site/src/chapters/drv-tmux.mdx)
and [runtime/setup
guide](https://github.com/pj-tmt/tmt/blob/main/site/src/chapters/drivers.mdx)
for prerequisites.

Confirmation identifies one exact pane/socket, identity choice and lifetime.
Cancel, rescan, changed selection or resize revokes it; enlarge a clipped
terminal before confirming. Results retain each Bound, Already bound, Not
applied, Needs you or Unknown outcome across close/rescan. Nothing replays or
rolls back automatically. A successful public refresh reveals the returned UUID
in Agents; uncertainty needs inspection before a new explicit attempt.

### Saved workspace snapshots

Workspace snapshots support tmux only. The list reads Core's Latest capture and
named saves for the original socket. Selecting or opening a preview is read-only.
Preview includes capture time, digest, topology, pane directories and literal
recorded argv. Long detail is scrollable and paged; text never becomes a command.
Save latest capture copies the published capture under an explicit name; it does
not capture the current layout. Delete removes only a confirmed named save.

Restore defaults to Layout only. Full mode can request eligible agent resume and
recorded TMT command startup. Review shows the exact socket, name, digest and mode;
Confirm submits that selection once using Core's digest guard. Existing sessions
skip. A changed or missing snapshot requires fresh review. Enlarge the terminal
when the exact target and Cancel/Confirm cannot fit readably.

The result preserves session/window/pane mappings and each startup outcome.
Startup requested does not prove a conversation resumed. Partial or Unknown
results retain their explanation; created resources remain, with no automatic
retry or rollback. Closing retains submitted work and its result. Inspect the
original socket and live resources before deciding a further action. An open
snapshot view, pending command or retained Unknown result defers board reload.

Old squad tabs, Left/Right tab navigation, tab reorder/pick/hide, factory views and
custom pane geometry do not select a legacy UI. The board reports located
migration notices and preserves authored TOML. `--tabs` is refused. Launch with
`tmt ops ui --squad product` or repeat the option, for example
`tmt ops ui --squad product --squad infra`, to show only those squads for this
session, opening Agents on the first named squad. Scope overrides saved hides,
excludes Common pool, and appears as `Scope: product,infra · Esc clears` with
whole names fitted to the width. Escape clears scope outside an editor, search
or overlay; explicit bindings win. Duplicate room IDs collapse and an unknown
or retired name fails before terminal admission with active valid squad names. `all` is an ordinary
squad name. Neither scope nor session reveal changes the saved hidden list;
in-place reload carries both, including a cleared scope. Noninteractive `ui`
reuses the existing listing documents: one unique requested squad gives its
single document, several give `{squads:[...],you}`. Independent `ls --tab`
projections remain available.
See [the migration table](../../docs/ui-migration.md) for old-to-new controls.

Ordinary canvas, content and chrome inherit the terminal background. The board
uses shared TMT tokens: `text` for shown tab names, primary counts and
partial/unavailable explanations, `muted` for context and key hints, and `dim`
for separators and incidental metadata. HOME summary state glyphs keep their
semantic roles; their words and counts use `text`. Selection uses the theme's
background and readable text, with words and marks independent of colour. A terminal without a selection
background uses reverse video, including `NO_COLOR`. On a real selection
background the final word policy uses readable `text`, while semantic marks
retain their colors. User theme and tab-color overrides still apply. Colors
reinforce words and marks; never infer state from color alone. The CLI theme is `theme.base` in the global `config.json`;
`tmt config show` shows its value and file. Board themes layer that resolved
theme, then `[board.theme]`, then `[squad.<name>.theme]` in `ops.toml`.
`auto` works in both `ops.toml` theme layers and both picker scopes; the global
`config.json` theme rejects it. Use `tmt ops sq theme set auto` for all boards.

`tmt ops sq theme ls` (or bare `tmt ops sq theme`) lists built-in bases, marking the
current base and its source: `default`, `cli`, `board`, `squad` or `detected`.
`auto` is first and is the board default when no layer sets a base. It chooses
`tmt` or `tmt-light` from COLORFGBG, then an OSC 11 query only when opening an
interactive colored board, with a 100 ms limit and dark fallback. Concrete
configured bases win. Startup keys received during the query are discarded;
late replies never become board actions. Lists never query: without COLORFGBG,
`auto` says “matches the terminal when the board opens”, with JSON
`resolvedBase: null`; a measured result says `auto (tmt-light)` or `auto (tmt)`
and `detected`, retaining its configuration layer in `baseSource`. Add
`--squad <name>` to inspect that squad. These choices affect the board only;
CLI colors stay unchanged.

```sh
tmt ops sq theme set auto                      # match the terminal on all boards
tmt ops sq theme set tmt-light                 # all boards
tmt ops sq theme set mono --squad product      # this squad
tmt ops sq theme rm --squad product            # remove only its base override
```

Set and remove keep token overrides and the rest of the user's TOML. They
refuse if the file changed since it was read. On the board, the `,` settings menu's
Theme row opens the theme picker (`theme` is also a bindable action, with no default key). Arrow keys or j/k preview in
memory; Tab switches all-boards/this-squad scope, Enter saves, and Esc cancels.
Common pool offers all-boards scope only. A squad's own base still wins
over an all-boards preview; the picker names that masking setting. A failed
save stays open with a notice; cancel and reopen to read a changed file.
Agents change the user's appearance only when the user requests it.

The selected agent's conversation header shows its name, state, observed presence,
runtime/model, supplied Digest labels, timer observations and resolved link when
available. Missing observations stay unavailable. Read the independent `sq ls`
JSON or `sq status` for full custom fields and domain metadata; those CLI fields
do not create a second detail grid or notebook pane.

The response region shows the selected agent's latest available response as safe
Markdown. Scrolling, copying and source selection send and acknowledge nothing.
A response that is unavailable or still loading is labelled; a withdrawal is not
a recipient reply or approval and does not clear manual pending/state. The shell
owns global attention; selected-target context never substitutes a guessed
recipient for a hidden, removed or unknown target.

Conversation Markdown includes filled, unwrapped code blocks, muted quote gutters
and boxed tables (literal source when the grid is too wide). Links and member
references use neutral underlined text; GitHub issue URLs shorten to
`owner/repo#N`. Conversation bodies always render in full; the pane scrolls.

Default Enter focuses Talk for the selected agent through the shared conversation
composer; clicking it also focuses. The composer stays visible while idle and
after Send. Factory `t` is unassigned; compatible explicit bindings retain their
meaning. Writing requires a recorded sender. Common pool supports Talk without
inventing
a squad; notes and manual squad status require a real membership.
On a selected workspace member, `y` copies `{name}: {task} ({state})`, and `a`
opens a note to its observed squad lead with the member named, even when a request
is pending. Both actions follow the selection while the conversation is focused.
A missing lead or Common pool membership shows a notice instead of opening a
composer to another recipient. Authored bindings retain precedence.
`reply` focuses the same member message composer; open requests are resolved at
submission as described below. Notes retain the actual lead or explicit member
recipient, with the recipient and source rechecked at submission. Esc returns to
the list with the message draft kept; empty input sends nothing.
Every workspace message input completes acquired agent and squad references with
the shared fuzzy matcher. Matching characters are bold; accepting a reference
inserts text only and never changes the recipient. Colab pages are not candidates.
Member Rename defaults to `r` in the Agents list only;
its reviewed UUID and exact current name must still match before Core changes the name.

Choose **Update status** from the selected lead or member's Actions menu. The
shared band shows exact raw manual `pending` and `state` values separately from
**Requires a reply**. Up/down selects a field or action; Enter toggles **Clear
pending** or **Replace state**. Type the explicit replacement on its row and a
reason on **Reason**, then choose **Apply and notify**. Neither change is
preselected. Clearing pending never infers a new state. PgUp/PgDn scroll long
previews; old/new values stack at narrow widths. The status form leaves the
member's message draft intact; Esc cancels.

Every selected field participates in the atomic apply with its exact raw value
expectation, including locally unchanged selections. The previewed identity UUID,
room, namespace and metadata keys remain fixed. Only actual changes are announced.
A conflict changes nothing,
refreshes the preview, clears field choices and requires another explicit submit. Legacy empty or invalid
metadata refuses with an unsupported-value notice. An unknown apply outcome sends
no notification and cannot be replayed from the form: inspect current metadata
before reopening. Confirmed updates queue an attributed announcement to that UUID;
it does not require a reply or prove delivery. **Status updated; notification
failed** offers **Retry notification only** when acceptance has not settled; select
that action and press Enter. The
retry retains the same operation and message and never repeats the metadata update.

**Message member** opens the member's message composer. Enter resolves the newest
currently observed open request from that member and answers it; older requests
remain open. If none is open, Enter sends a new message to the same member.
A request settling while you type does not block sending. A definite settled
refusal permits one fallback new message with the same text; uncertain acceptance
never triggers a resend. Status changes remain separate and acknowledge nothing. Open **Update status**
from the row's Actions menu (or an authored `update-status` binding); the form
reuses the existing preview, permission and conflict checks.

Inside the active message editor, Enter sends and Esc returns to the list with
text kept. The member composer never copies the lead or cycles modes with Tab;
use the explicit Note to lead action for a lead note. Its placeholder names the
member receiving the message.
Narrow or low windows retain an editor row and explain the compact placement in
its title. After Send, the composer stays ready for the next message. The sent
turn appears below the retained response, followed by its delivery label and
independent acquired agent state, then an exactly correlated reply. Follow is on.
Delivery labels report `sending`, `delivered` (written to the pane), `queued`
(in the inbox), `offline · queued`, `held · digest`, or `unknown · do not resend`.
Delivered does not mean the agent saw or read it. Unknown acquired state or stale
observations never become idle by inference. Acceptance clears only the submitted
draft, including an uncertain delivery or an error carrying a retained request;
the board never resends automatically. A refusal without a request keeps the
submitted draft without overwriting newer edits. Opening, cancelling, empty input
or an unavailable member sends nothing and retains the draft.

## Home dashboard

The Agents Usage band and Measure view are the shell's dashboard. The flat agent rows and Digest counts follow
saved hiding and launch scope; shell attention remains global. A complete read
can say `Nothing needs you`, but a partial/stale read
shows observed counts and says totals are unknown. The shell keeps online/waiting/
blocked observations visible across tabs; missing evidence is qualified rather
than presented as zero. Search, pinning, sorting, expansion,
refresh and reload retain exact room/identity selection. A hidden or removed
selected target cannot redirect an operation to an adjacent row.

The longest-window usage line uses the configured global token window. Identities
in several sampling squads count once with the better-covered observation.
Missing differs from measured zero; incomplete totals/shares carry partial
coverage, and a zero total has no share. Sampling off supplies no token values.
Model attribution is best effort. Provider limits and sampled history reuse the
existing bounded observations; reading sends and acknowledges nothing.
Public `tmt ops sq ls --tab all --json` retains its aggregate document.

Select a squad to inspect its lead-first MEMBER / STATE / TASK / NEXT ACTION roster,
supplied usage facts and timers. Agents uses a 47% list in Side layout; Auto stacks
below 120 columns and uses a single focused pane when the usable budgets do not fit.
The table reserves one range/total/more row. Add a member, set or
clear its lead, rename it, pin it or open Timers through the named controls or
Manage. `n` creates from a name, optional lead and explicit members. Assign offers
add (keep every membership) or move (remove the selected source only after the
destination add is confirmed). Common pool offers add only.

Management reviews freeze exact room/identity UUIDs and revisions, show timer
impact and start on Cancel. Tab reaches the scrollable read region and Apply;
clipped or stale Apply controls cannot submit. Pending prevents repeated applies.
Results retain per-item success/failure and explicit repair guidance, with no
automatic retry. `x` removes membership, not an identity, process, draft or history.
Source timers pause before leave; re-add/reassign never resumes them. Explicit
Resume is separate.

Squad rename keeps its room UUID, moves Ops metadata/config and the cron counter
namespace/jobs' squad field, and preserves c-ids and pause. A second-step failure
leaves the confirmed room name intact and reports a repair command. Use that
exact command to repair; do not roll the room name back or blindly retry.

## Digest time

`tmt ops sq digest <member> [duration|off] [--squad <name>] [--json]` shows, sets or clears
one active member or lead policy. Showing, setting and clearing all require the
recorded user or the current squad lead; a lead needs the saved owner recorded with
`tmt ops sq me <owner>`. Human output names the member. JSON includes `member` (the
resolved name) and `identityId` (the UUID), alongside the policy fields.
Use whole `s`/`m`/`h` segments from **1s through 24h**, such as `30m` or `1h30m`.
Zero, negative and larger windows are refused. Omit duration to inspect; `off` clears.
Running it again replaces the window against the observed revision. A revision
conflict says to reload and retry; there is no automatic retry or recurring cadence.
Core owns digest and held items; Squad stores no policy. The command passes through
Core errors, including when an older Core has no digest operation. Digest delays
ordinary deliveries; owner and urgent messages can still interrupt.

## Cron on the board

The Agents section's Timers control opens all observed rooms without retargeting
selection. `c` or the selected squad's Timers control opens the shared job list, scoped by
room UUID. Member counts show active/total jobs: the flat list uses that owner across
rooms, the tree uses the current room, and squad counts include orphan jobs. Counts
remain unknown after a failed read. Clicking a complete count freezes its room
and owner UUIDs; the list filters to that exact scope. Its title names the scope
even when empty; a removed room is not treated as all rooms. A selected member
prefills its owner when creating a job. The flat list and
Common pool do not invent a room; choose a real squad for a new job. Read failures
retain prior jobs and their reason. `no clock` means due slots are not sent;
`clock: checking…` is the first read. A running clock shows its holder's
`session:window` when available, otherwise its pane ID.

The focused list owns Enter (open the job's squad), `n` new, `e` expand/collapse, `E` edit, `p`
pause/resume, `x` send now, `o` reassign and `d` delete after confirmation. The
workspace entry follows explicit bindings; the modal owns its scoped keys.
Forms retain exact owner/message/schedule; Esc cancels. Before create, changed
schedule or reassignment applies, Review shows the frozen operation, up to three
next occurrences and scheduled invocations in the next 24h. Tab reaches reading,
Cancel and Apply; a pending or failed preview cannot apply. Dense schedules say
at least 4096 (preview capped). This is not tokens or dollars, and depends on the
owner and clock. Apply rechecks the original actor, room and revision. Multiline messages use the shared editor; oversized stored messages
remain intact when editing only owner or schedule.

Changes use the same permission/revision checks as the CLI: only the recorded
user or squad lead can change jobs; changed jobs are refused, never overwritten.
Failures remain visible with no automatic retry. `x` sends once through the same
manual service as `tmt ops sq cron send`. A missing owner is shown as
`paused · owner missing`. Re-add/reassign never resumes a paused job; only explicit
Resume clears pause.

## Manage checklists

`tmt ops sq checklist ls --room ROOM_UUID` reads a manually authored room checklist;
`--json` returns one complete document. `list` remains an accepted hidden alias;
help and examples use `ls`. These commands are also available as `tmt ops squad checklist`.
The board's Checklist actions use the same admitted service. Every command requires the exact room UUID,
never a squad name; writes freeze supplied checklist/item UUIDs and revisions.
No command accepts an actor override or uses a member filter as assignment.

| Action                                                 | Explicit inputs besides `--room`                                                                                                                                |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ls`                                                   | Optional `--include-archived`, `--completion open\|complete`, `--assignee UUID`; reads create nothing                                                           |
| `show`                                                 | `--checklist UUID --item UUID`                                                                                                                                  |
| `create TITLE`                                         | `--checklist UUID --item UUID --expect-inventory absent\|POSITIVE`; optional `--body TEXT --reference HTTP(S) --assignee UUID`                                  |
| `edit`                                                 | `--checklist UUID --item UUID --expect-revision POSITIVE`; at least one `--title TEXT`, `--body TEXT`/`--clear-body`, `--reference HTTP(S)`/`--clear-reference` |
| `assign`                                               | Item/checklist UUIDs, exact `--expect-revision`, explicit `--assignee UUID`                                                                                     |
| `unassign`, `complete`, `reopen`, `archive`, `restore` | Item/checklist UUIDs and exact `--expect-revision`                                                                                                              |
| `delete`                                               | Item/checklist UUIDs, `--expect-revision`, `--expect-inventory`, matching `--confirm-item UUID --confirm-revision POSITIVE`                                     |
| `reorder`                                              | `--checklist UUID --expect-inventory POSITIVE --order JSON_UUID_ARRAY`; all nondeleted items including archived items                                           |

Generate new checklist/item UUIDs before the first Create and keep them fixed for
that submission. First Create expects `absent`; subsequent Create expects the
reviewed positive inventory revision. No command invents new UUIDs, adopts a
latest revision, retries or substitutes a same-name target after refusal.
Set/clear pairs conflict. Empty body normalizes to absent; empty reference is
invalid and clearing it is explicit. Title is nonblank, without controls, at most
1024 UTF-8 bytes; body is at most 65536 bytes with only line break/tab controls.
A single reference is inert HTTP(S), at most 4096 bytes, never fetched or opened.
Authored strings remain data.

The verified invoking caller decides authority. Only when no caller is bound may
the recorded active user be used; ambiguous or invalid callers refuse. Active
recorded users and room leads are managers. Collaborators can create unassigned
items, edit content, complete and reopen unarchived items. Initial assignment
requires manager permission, including self-assignment; only managers assign,
unassign, archive, restore, delete or reorder. Assignment chooses an exact active
same-room member UUID. Departure retains its UUID/last label as unavailable;
Unassign needs no live former assignee. Completion changes no pending/state/request
or dispatch/notification data.

Each actual item change increments only its revision; Create/Delete/actual Reorder
also increment inventory once. A no-op changes neither revision but still admits
permission and exact expectations. Archive retains content, completion and order;
Restore changes only visibility. Archived items require explicit Restore before
editing, assigning or completion changes. Delete removes authored content and
retains a minimal tombstone; neither deleted nor live UUIDs may be reused.

Success exits 0, checklist operation errors 1 and grammar errors 2. JSON successes
have `action` and `current`; listing uses semantic action `list`, `totalCount` before
filtering and `matchedCount` after filtering. Mutations add `changed`, plus `itemId`
and `itemRevision` where applicable. `current` contains `room:{id,name,available,manager}`,
nullable `checklistId`/`inventoryRevision`, ordered `items` and nullable
`deletion:{itemId,deletionRevision}`. Items expose `{id,revision,title,body,reference,
assignee,completion,archived}`; absent optionals are null and assignment is null or
`{id,label,available}`. Completion is `open` or `complete`.

Errors retain `{error:{code,message,current?}}`. Only authorized typed conflict/
deletion data supplies `current`; denial exposes no current projection. Human
errors go to stderr and print no success line. Missing checklist, empty inventory,
filtered-empty inventory and storage/read failure are distinct; corrupt data is
never treated as empty or repaired. Exact UUID reads preserve a renamed room;
a same-name successor is distinct. Only the recorded active user may inspect an
unavailable room's retained data read-only, and all orphan writes refuse.

After `CHECKLIST_CONFLICT`, review the supplied current revisions and make a new
explicit submission. `CHECKLIST_OUTCOME_UNKNOWN` never claims success or rollback
and never permits blind replay: later matching fields, revision or tombstone only
confirm observed current state. Current state refreshed; original update outcome
remains unknown. No operation ledger is introduced.

## Manage recurring jobs

`tmt ops sq cron ls [--squad NAME]` lists jobs across all active squads, including
hidden boards. `show <squad> <id>` shows the exact message, pause attribution and
up to three future slots. All management commands take `--json`.

```sh
tmt ops sq cron add product worker --every 30m "Review the queue"
tmt ops sq cron add product worker --at 09:00 --on weekdays "Review the queue"
tmt ops sq cron add product worker --cron "0 */3 * * *" "Review the queue"
tmt ops sq cron edit product c1 --message "Review pending changes"
tmt ops sq cron pause product c1
tmt ops sq cron resume product c1
tmt ops sq cron reassign product c1 reviewer
tmt ops sq cron rm product c1
```

Writes require the recorded user or that squad's current lead. `--identity`
selects an explicit actor; otherwise the verified caller wins, then the recorded
user only when no caller exists. An ordinary identified member cannot fall back
to the user. Add accepts `--paused`; edit accepts the same schedule flags or
`--message`. Reassignment clears no owner but retains its pause; resume separately.

Messages must be nonempty and are retained exactly, without substitution. Jobs
live in the Ops state database in the extension's own storage, separate from
`ops.toml`. IDs are never
reused. Fixed local times skip daylight-saving gaps and use the first repeated
time; intervals keep elapsed duration. Schedule edits retain the stored zone.
Change notices are best-effort announcements to the owner; reassignment notifies
old/new owners, and the actor's own notice is suppressed. A committed change can
report notification warnings. Owner retirement pauses the job as no owner and
notifies the lead; each management invocation handles at most 16 pending hooks.
An open board keeps the clock running independently of its refresh setting.
Outside a board, `tmt ops sq cron run` keeps it in the current pane; `clock` shows
its pane, PID and how long it has held the lease. Only one clock holds the lease. Stop `run` with
Ctrl-C; stale leases can be taken over, and a running clock (a board or `run`) replaces
a live lease held by an older Ops version, or by an Ops that predates the version field,
whose holder then stops at its next renewal; a one-shot `tick` never displaces a clock.
Startup and takeover begin at now, so
slots missed while no clock was running are not caught up. A running clock
admits slots since its previous tick, capped at five minutes. `tick` makes one
pass over the last 60 seconds and refuses an active clock. Scheduled messages
are anonymous requests recorded with origin `automation`, so they are told apart from
typed messages; the same room/job/slot operation is accepted once.
The same tick runs the [idle alert](#alert-the-lead-to-an-idle-member) scan.

`tmt ops sq cron send product c1` sends once now using the recorded or explicit
user/lead actor. It can send a paused job with a current owner and changes no
schedule. Each explicit send is a separate action. Output confirms acceptance,
not delivery or completion; Squad stores no run results. If acceptance is
uncertain, retain the reported operation ID and recover it with `dispatch.show`
through `tmt api` before deciding on another action. Board cron controls (above) are
separate from the clock lifecycle.

## Alert the lead to an idle member

A squad can opt in to one notice when a member sits idle too long. The user switches it
on; a lead changes it only when the user asked.

```sh
tmt ops sq config set idle_alert.enabled true --squad product
tmt ops sq config set idle_alert.after 20m --squad product
```

`idle_alert.enabled` is `false` in every layout until set. `idle_alert.after` takes whole
`s`/`m`/`h` units from 5m to 24h and defaults to `30m`. The keys live in
`[squad.product.idle_alert]` of `ops.toml`, change through the validated writer of the
reminder settings, and `config show --squad product` reports their values and sources.
There is no per-member setting.

A member is over when Core reports its activity `idle` (the admitted end of a turn, from the
hooks `tmt setup` or `tmt run` install) for at least `after`. Working, ended, unknown, stale
or missing activity never counts, so a member without hooks never alerts. An over member is
excepted, and not announced, while its row has a non-empty `pending` (it waits on the user)
or `state` `hold`, in any letter case (a resting seat). The lead is never announced to
itself, and a squad without exactly one lead sends nothing until it has one.

At the first scan where a member is over and not excepted, the clock sends the lead one
announcement (no reply expected, origin `automation`, not urgent, so the lead's Digest rules
apply). One idle phase is one episode, named by Core's idle start: no repeat, no escalation,
no "no longer idle" notice. The member's next idle phase is a new episode, and a new lead is
told once. The body:

```text
Idle alert · squad product
worker-3 has been idle since 2026-10-19 14:32 +09:00 (alert after 30m).
Row (when sent): state=review · task="rotate session tokens" · pending=none
Look: tmt focus 3f2a9c1e-5b7d-4e08-9a6c-0d1e2f3a4b5c · Talk: tmt talk 3f2a9c1e-5b7d-4e08-9a6c-0d1e2f3a4b5c "<message>" --detach
```

The two commands carry the member's identity UUID, never its display name, which may hold
spaces and shell characters. The row line quotes the member's `state`, `task` and `pending` as
data, never instructions:
one line each, cut at 120 characters, at most 600 bytes together, read when the notice was
first attempted. The dispatch operation is derived from the room, member, lead and idle
start, so a retry, a restart or a second clock cannot duplicate it. When a retry's row text
differs, Core refuses it as a changed intent and the clock treats that as already accepted.
Nothing about delivery is stored; the notice is in the lead's inbox.

The scan is a second duty of the cron clock. A clock must run, with a board open or
`tmt ops sq cron run` in a pane (`cron clock` shows it); with no clock there is no alert.

The board shows the same facts without a new call. Press `,` with a squad selected and choose
`Squad settings: <squad>` (last in the menu); Esc returns to the menu. There, Space on
`idle_alert.enabled` flips it through the same writer as `config set`, and `idle_alert.after` edits
with Enter. A
squad row shows `idle alert 30m` (`alert 30m` in a narrow cell) only while the alert is on, and
`◷ N idle` after its team marks while members are over. An over member shows `◷ idle 32m` in its
state cell; an excepted member keeps the plain `◌ idle 32m`. `tmt ops sq ls` appends
`idle alert 30m` to the squad header and `◷ idle 32m` to the over row. A UUID in the notice that
names a known member opens that member's menu. With the alert off, none of these appear.
It reads one roster and one `ls` per enabled squad, at least 60 seconds apart; a squad with
the alert off costs nothing. A clock's first look, and its first look after a pause of more
than five minutes (a sleeping machine), only record the time: members are judged a minute
later.

## Use the board checklist

Press `,`, choose **Actions** after the layout/Theme/Token window choices, then
**Checklist**. A configured
`menu` binding opens the same action menu. A visible selected squad or its member
in Agents or Digest uses its exact room; the flat list, Common pool and hidden/missing targets
ask you to choose a room explicitly. Opening
Checklist does not select or assign a member. Esc from the action menu returns
directly to the board.

The checklist starts with Open items, Squad-wide assignment scope and archived
items excluded. Tab/Shift-Tab reaches **Filter** and **Actions**; Enter opens their
choices. Filters include Complete, All completion, Unassigned, an explicit member
and archived inclusion. Counts show visible items and all nondeleted items in the
same archive scope. Arrows/j/k, Home/End and PgUp/PgDn select or scroll items;
Enter opens details without completing an item. In details, arrows and paging keys
read the content; Tab switches between reading and the named action choices.

Actions offers Create and Refresh, and Reorder for a current manager. Create starts
unassigned. Details offers Edit, Complete/Reopen and an explicit Open reference;
current managers can Assign/Unassign, Archive/Restore and Delete. Archived items
expose Restore/Delete. Reorder includes the full inventory, including archived
items, and uses Move up/Move down followed by a separate preview and Confirm.
Forms accept typing or paste; Tab moves fields and Ctrl-U clears a field.

Every update previews exact actor, room, checklist, item and required revisions.
Delete starts on Cancel; enlarge the terminal when its exact target and controls
cannot fit readably. Conflicts and refusals retain the draft and target. Refresh
reads current state, then Review explicitly adopts its expectations before a fresh
Confirm. An original unknown outcome stays unknown after observation or later
updates. Esc from a child screen retains intent and returns to the list; Esc from
the list returns to the board. Resume retained work through Actions, or explicitly
Cancel to discard it. Checklist completion changes no member state, requests or
attention, and opens references only when you choose Open reference.

## Inspect board settings

Press `,` for Workspace settings: Auto/Side/Stack, Theme, Token window and
Actions. The menu reports current choices; cancel writes nothing. Layout, pins and hidden squads are shared per-user Ops preferences; interactive
sort belongs to the current view layer. Config saves compare the opening values
and file bytes, preserving comments and unrelated keys; a conflict keeps the
current effective choice. Theme can use all-boards or selected-squad scope;
Common pool has only the shared scope.

`tmt ops sq config show` inspects board defaults. Use `--squad product` for one
squad or `--tab all` (also `leads` or a configured tab name) for an aggregate
view, and `--json` for full values and source paths. These scope flags are
exclusive. Inspection changes no configuration or member state. JSON marks the
settings supported by `config set`; the board uses the same validation and writer.

Use `tmt ops sq config set KEY VALUE [--squad NAME]` for validated edits. Squad scope
is required for `layout`, `board.panes`, `board.direction`, `board.sizes`,
`board.hidden_columns`, `notes.render`, `states.STATE.color`, `reminders.enabled`,
`reminders.stale_after`, `idle_alert.enabled` and `idle_alert.after`. `board.refresh`, `board.ask_lead`, `board.view` and `board.token_rate.window`
use the squad layer with `--squad`, otherwise the global Squad board layer.
The obsolete `board.home_replies` key is ignored with a one-line deprecation
notice; authored TOML stays unchanged. It is no longer editable. `tabs.order` and `tabs.hide`
always edit global Squad tab policy. Arrays use JSON;
other values are unquoted scalar arguments. Examples:

```sh
tmt ops sq config set notes.render plain --squad product
tmt ops sq config set board.refresh 10s --squad product
tmt ops sq config set board.hidden_columns '["pr_link"]' --squad product
tmt ops sq config set tabs.hide '["leads"]'
```

Legacy geometry, factory view and tab policy remain authored CLI configuration;
the workspace reports migration notices instead of applying those arrangements.
The validated writer preserves unrelated keys and comments and refuses a file
changed since reading. Provider/run commands and Core/provider configuration
remain read-only to settings inspection and are never executed by inspection.

For the selected squad, set `reminders.enabled` to `true` or `false`, and
`reminders.stale_after` to whole s/m/h units from 1m to 24h (for example `30m`).
`config show --squad <name>` reports effective defaults and their sources.
After saving, ordinary reload uses the new policy; unknown observed ages remain
unknown. Disabled reminders produce no
stale marks or reminder work. These edits install no provider hook and grant no
extension consent.

## Choose a board view

The settings menu offers Auto, Side and Stack for the workspace/conversation
placement, alongside Actions, Theme and Token window. These per-user Ops
preferences persist through Config's compare-and-set writer; stale-file refusal
keeps current state and preserves comments and unrelated values. They change
presentation only, not workflow state, membership, provider or sampling policy.
Saved squad hiding uses optional `workspace.hidden`, a unique array of at most
1,024 canonical lowercase room UUIDs. Empty saves remove the key; missing means
none. Unknown/deleted IDs remain stored without pruning and count nowhere in the
UI. Scope and reveal are session state, separate from these preferences and from
retained cold-start view hints.

Factory `sq view` configuration and custom pane keys no longer choose the UI.
Their authored bytes remain intact and located migration notices identify them.
There is no legacy mode. Independent CLI list/layout configuration retains its
own reader and validation; it is not a second interactive editor or shell.

## Squad member list

Agents shows one expandable squad tree and Common pool, with explicit room and
identity targets. Lead badges and member counts include the lead once.

A member's STATE shows core's provider activity first and the squad task state
second, such as `● working 12m · review`. The first word is `waiting` when a
request waits on you; otherwise it is core's `working` or `idle` with how long
that phase has run, or `ended`. Unknown activity has no word: the cell is a
muted `?` and then the task state, such as `? idle · member`. When the board's last read failed, or
is older than its refresh interval (at least 5 s) plus 10 s, the word is `stale`
with the time of core's last event, such as `? stale 4m ago`; with refresh off,
rows go stale and stay stale until you refresh. Stale and unknown never mean
idle, and the age is never made up when core gives no time. A narrow cell clips
the task first, then drops the age. Unread replies from an agent replace the
activity words: the cell is `● N unread` and then the task state, such as
`● 2 unread · review`, with only the `●` in the unread colour. The working and idle counts follow core
activity, each agent once, and count a stale agent as unknown. In a squad row each TEAM glyph shows a waiting request (◆) first, otherwise core
activity (`?` when stale or unknown); a task state never overrides it. Attention and
presence are scoped observations, and failed observations remain partial/stale
rather than becoming idle or zero.
A collapsed squad shows its worst observed state. Display names never provide
operation authority.

Enter focuses the selected agent's composer in the shared conversation; the
configured Inspect action remains available. Factory `t` is unassigned, and `j`
jumps to the available host. Source-provided labels retain their semantic roles
and choice actions; a choice click opens that exact control rather than sending.
Digest uses the same UUID context and existing Core authorization. Refresh and
resize retain the target and clear old hit rectangles. Reading requires no sender
and sends/acknowledges nothing; writing requires `tmt ops sq me`.

## Conversation and message editing

The workspace shows the selected agent's conversation in its Auto/Side/Stack
placement. All sections share response, reading position and composer by agent UUID, so renaming an agent or changing its membership does not
create another draft. Runtime is the observed runtime name, not the model's vendor;
missing runtime/model evidence is shown as unavailable. For a basic driver (Core's
`driverSupport`: detection and skills only) the Runtime column and the header's
runtime line say `basic` instead, because its activity, model and runtime are not
observable and a blank must not read as idle.
The model shown is the last completed turn's model (`consumption.modelId`), falling back to
the session-start model, so a mid-session `/model` switch appears after the next completed turn.

A two-pane `rows`/`replies` workspace uses terminal cells without blank row padding.
Its automatic layout is side by side when wide, stacked at medium widths, and focused
single-pane when narrow or low. An explicit side/stack preference degrades safely.
The wheel scrolls the pane under the pointer without changing the message recipient.
PgUp/PgDn/Home/End act on the focused pane. Reading older response text pauses follow;
new output is labelled until End or intentional downward scrolling to the bottom
resumes follow. Resize and new output alone keep it paused. A replaced response
stays visible while paused. A vanished source is labelled separately, not presented as new
output.

Each acquired settled reply is a separate message with its own author and age,
correlated to its request. Single line breaks remain visible in replies and pane
output. Pane output stays in a separate labelled block, even when it repeats a
reply's text; reading older messages holds position while later replies arrive.

The composer stays at the conversation bottom when an agent is shown: a box of at
least three rows, a blank padding row above and below the text, growing upward
without moving its info line. Focus changes only the fill, never the height.
Terminals under 28 rows give the padding up and keep the one-row box. The line below
shows the recipient, acquired unread/open-request counts, optional running-session
context usage and `⏎ send  Alt+⏎ new line  @ mention`; narrow widths shorten the
words before dropping keys. Unknown context is omitted, not estimated.
Default Enter or a click focuses it; list actions remain available while idle.
At eight terminal rows, the Agents composer fills the body and member rows are
hidden. Esc leaves typing with the draft kept; ↑/↓ still change the selected target.
With list focus, default Home selects Common pool and restores the group tree.
Select a member to return to its retained draft, or enlarge the terminal to show
more of the list.
Talk, reply and annotation use a multiline editor. Left/Right and Home/End move the
caret; Shift with movement selects; Backspace/Delete remove selected text or a
grapheme. Click places the caret and drag selects. Enter submits. Shift+Enter inserts
a newline when enhanced keyboard reporting is active; Alt+Enter and Ctrl+J work
without it, and Ctrl+N remains available. The input box grows with its content up to
six text rows (plus the padding rows), then scrolls; the info line and footer stay in place. The info line and the
footer name Shift+Enter when the terminal reports it and Alt+Enter otherwise
(Alt+Enter and Ctrl+J always work). Shift+Enter inside tmux needs
`set -g extended-keys on` (or `always`) and `set -g extended-keys-format csi-u`
(tmux 3.5 or later); Ops reads these two options once at start, asks the terminal for
the extended keys only when both are set, and releases them on exit. Ops never
changes tmux settings.
Esc leaves input without sending. Direct-message drafts and their quote/paste
attachments are session-only: they survive view/layout/resize and membership changes,
but do not survive restarting the TUI. An accepted request clears only the submitted
draft and leaves the composer ready, even if delivery is uncertain; do not resend
that request. Refusal without a retained request keeps the draft. Sent turns and
exactly correlated replies are bounded session feedback, not durable history.

Bracketed paste never submits. Message text is limited to 1 MiB, with exact text and
newlines preserved. In direct messages, paste over 500 Unicode characters or
10 logical lines becomes an inline chip; exactly 500 characters or 10 lines stays
ordinary text. Click its body or press Ctrl-P at the chip to open a read-only,
scrollable preview above it (below when there is no room above). Esc or an outside
click closes the preview and keeps the draft. Click ×, or use Backspace/Delete at
the chip, to remove the whole pasted range. Multiple chips can sit between typed
text; sending uses every original byte. With ASCII glyphs or NO_COLOR, the chip
reads `[Pasted … x]`. Other message forms retain their inline fold preview with
Ctrl-P for paste over 500 characters or eight lines.
Name/configuration/search fields keep their smaller bounds and do not gain message
completion. The status reason keeps its 4,000-byte bound and explicit Apply step.

Type `@` in a message, status reason or cron message body to search acquired agents
and squads. Arrows select a candidate; Enter/Tab inserts it, Esc dismisses, and a
click selects it. Mentions are plain text only. The displayed direct recipient stays
unchanged; `@squad` never broadcasts or chooses a lead. Cron stores the literal text,
and later renames do not rewrite it. Colab addressing has its own semantics; a page
link in the conversation does not authorize native page/thread writes.

Dragging rendered response text opens a labelled exact source view; it does not
guess raw byte ranges from styled text. Select on the freshly painted raw view,
then use its annotation control to attach the exact source without sending. Esc
leaves that source view first, then follows the normal return path. In the full
response reader, `s` opens exact source selection. Shift with movement or mouse
drag selects a range; `y` copies and `a` attaches it to a draft without sending.
Common pool quotations use Talk to the same UUID; squad notes keep their actual
recipient. The quote carries the response/request identity and source byte
range. Selecting text never acknowledges a request. If the source changes or the
agent disappears, the retained text can still be read/copied but cannot be attached
to another response. The composer shows a readable author/excerpt quote caption;
its remove control removes the most recent quote while keeping message text.
Alt-Backspace deletes the word before the draft cursor. Details shows bounded, frozen provenance only for quotes matching
both the selected source UUID and the draft's actual recipient UUID, including
full request/identity and exact byte range.
For terminal-native selection, use the terminal's mouse-capture bypass modifier
(commonly Shift); the exact modifier belongs to the terminal.

## Fold board panes

The new workspace uses squad expansion and Auto/Side/Stack placement. Left/Right
expand/collapse the selected tree group; a hidden or removed selection remains
anchored and cannot confer actions on a neighbour. Old pane toggles, fold keys
and tab-reorder bindings are retired with located migration notices, preserving
user configuration. There is no factory-view or legacy-mode fallback.

## Keep it current

A stale board is worse than none. Update the board as part of every dispatch
and every reply you receive, not later.

```sh
tmt ops squad add <name>...                       # agents that are already running
tmt ops squad set <member> state=review task="rotate session tokens"
tmt ops squad set <member> pending="approve the token rotation plan"
tmt ops squad set <member> pending=               # clear it once answered
tmt ops squad set <member> pr_link=https://github.com/acme/app/pull/412
tmt ops squad rm <name>                           # leaves the squad; the agent keeps running
```

- `task` describes what the member is doing; `pending` describes what it waits
  on the user for. Keep member context in that member's own saved-identity
  notebook (`tmt notes path --identity <member>`).
- The per-member `note` field is retired: nonempty `note=` fails before any
  writes. Empty `note=` still clears an old value. Reads preserve stored legacy
  notes but exclude them from rows and row `fields`; `note` remains reserved
  and cannot be reused by a field provider or bound column.
- `pending` is the one decision the member needs from the user. Keep it short
  and clear it when it's resolved.
- Field names are `[a-z][a-z0-9_-]*`. Values are one line of at most 1024
  bytes. `field=` removes a field.
- `set` applies its pairs in order and reports what it applied. After a
  failure, re-run it with the same pairs.
- `tmt ops squad lead <name>` selects the lead independently of free-text `role`
  and `lead` fields. Setting or clearing either field never changes leadership,
  and selecting a new lead preserves every member's role text and membership.
  `tmt ops squad lead --none` clears leadership. Former leads remain members; use
  `tmt ops squad rm <name>` separately when they should leave.
- Repeating `tmt ops squad add <name>` reports that the member is already in the
  squad and preserves its state and task. A missing state receives the configured
  initial value.
- Legacy members with only `role=lead` still appear as lead until a role write
  would change leadership or `squad lead` records their separate marker. Listing
  and opening the board never perform that conversion. The reserved metadata suffix `lead.marker` is not a
  user field and never appears in row `fields`; leadership is shown through
  `squad.lead` and the section partition.
- Removing a member clears its fields for this squad only. Its requests and
  notes keep the history.
- Squad has no `talk`, `reply`, `replies` or `annotate` command. To talk to a
  member use `tmt talk <member> "…" --detach`; to answer what someone is waiting
  on you for use `tmt inbox` and `tmt answer` (or `tmt reply --receipt` when you
  were given a receipt). Adding a note to a row is the user's board key, not a command, and
  agents never call hidden or `__` commands (TMT's CLI style, "Hidden commands").
  Without a lead, select one with `tmt ops squad lead <name> --squad <squad>`.

## Keep your notebook current

Each saved member owns its notebook; there is no separate squad notebook.
Find it with `tmt notes path --identity <name>` and edit that file with ordinary
filesystem tools. The board never creates another member's notebook. The
workspace conversation replaces the former Notes-pane navigation; notebook
reading/links/line-annotation controls are not workspace defaults.

Notebook content remains owned by each member. A board projection never makes
another member's notebook writable; use the explicit notebook commands below.

Every member should keep a short **Current state** section at the top of their own notebook, with
**Now / Next / Blocked** in a few lines, because the user reads it on the board.
Update those lines when the working state changes; keep history below them.
User annotations remain requests about a row or a notebook line.

## Annotations from the user

The user may annotate a row from the board. It arrives as an ordinary TMT
request to you, tagged `[<squad> · <row>] <text>`. You decide what to do with
it: update the board, record it in your notes, or pass it to the member. Reply
to it, because that is how the user sees you handled it.

## Configuration belongs to the user

`ops.toml` sits in TMT's global configuration directory, next to
`config.json` (`tmt config show` prints that path). It may hold `me` (the
user's saved identity, recorded with `tmt ops squad me <name>`) and `me_id` (its
UUID, which lets `me` follow a rename; squad maintains it), each squad's `layout`, the board panes, sections, columns,
states and key bindings. Bindings and actions are the user's. Never edit them
silently. If a change would help, propose the exact lines and let the user
apply them.

Recording the user (`tmt ops squad me <name>`, `init --me`, and each board start)
also gives that identity Core's reserved `user = true` metadata entry, so Digest
never holds their messages or replies. Only an absent entry is written; an
existing value, even `false`, is yours, and the entry is never removed. No other
identity is marked. Without Ops, `tmt identity meta set user true --identity <name>`
does the same.

On first config/state use, Squad migrates `squad.toml` to `ops.toml` and its
`<dataRoot>/squad/` state to `<dataRoot>/ops/` under a lock. Config comments and
state bytes are preserved; originals become `.migrated-<timestamp>` backups and
user `.bak-*` files stay untouched. If both names exist, Ops wins and a notice
names the ignored legacy paths. An old clock with a live lease defers the entire
migration: config and jobs stay visible on legacy paths, and the notice names the
holder and how to stop it. Invoke again after stopping the clock to migrate.
Restart boards started before the upgrade before editing settings. A legacy config
that reappears is reported but never read or merged; move wanted settings into
`ops.toml`, then delete the old file yourself. Commands use `tmt ops` and the
binary is `tmt-ops`; disposable observations use the `tmt-ops` cache namespace.
Refresh former generated hotkeys with consent using `tmt ops hotkeys install`.
The former generated file and immutable skill sources remain available.

Cron jobs and checklists are kept in one Ops database (`<dataRoot>/ops/ops.db`). The
first use after an upgrade imports the earlier `cron/jobs.json` and
`checklist/<room-uuid>/items.json`, then renames each to `<name>.migrated-<date>`;
nothing is deleted, and an old file that reappears is archived the same way and never
read. Concurrent commands queue for up to five seconds; a longer wait reports
`SQUAD_CRON_STORE_BUSY` (or a checklist storage error) with nothing saved, and the same
command can simply be run again. A database written by a newer Ops is refused with an
upgrade hint (`tmt extension upgrade ops` or `tmt upgrade`) and never changed.

## Observed token usage

The selected squad's usage disclosure shows completed-request tokens from public
Core history and acquired observations in its configured windows (default
**1m / 5m / 1h**). The workspace uses those observations independently of the CLI
row grid. CLI presets and custom columns can name `usage.w1`, `usage.w2` and
`usage.w3`; one-shot `sq ls` has no window history, so JSON keeps those column
descriptors without usage values and text omits the board-only sources. Changing
the active window affects the acquired summary, not a second member grid.

Input and output count once; cached input is already included in input, and
normalized reasoning in output. Mixed providers sum reported token units, not
cost or interchangeable text volume. Model attribution is best effort: a
mid-session model change attributes retained observations to the current model.
Counters update at request completion and are observed every 5–10 seconds,
not while a model writes. On entry, the board reads closed core history for up to
one hour and continues from its included counter watermark. It stores no separate
usage history and computes no money estimate.

Team enables observation; crew, pr-queue and minimal keep it off by default.
Common pool omits this named-squad meter. The settings menu's Token window row cycles the summary's windows, as does
the bindable `token-window` action (no default key), or clicking any part of the meter.
All three paths save the next window to the shared layer; a squad override remains
in effect until reset in settings. Selected squad usage detail can disclose each configured window.
The live label names the configured window and the live number is a token total. Configure exactly three distinct ascending whole `m`/`h`
durations, from 1m through 24h:

```toml
[board]
tok = "1m/5m/60m" # for example, "5m/60m/24h"

[board.token_rate]
enabled = false
every = "5s" # 5s through 10s; independent of board.refresh
window = "1m" # persisted summary window; falls back to the first configured window
reduced_motion = true

[squad.checkout.board]
tok = "5m/60m/24h" # overrides the global windows

[squad.checkout.board.token_rate]
enabled = true # individual keys override global policy and layout preset

[bind]
u = "token-window"
```

`–` means no usable observed interval for that member; a baseline alone is not
measured zero. Covered readings, including measured zero, are numeric. Known nonzero history
deltas also show as partial lower bounds even without continuous coverage;
zero without coverage stays unavailable. `~` marks
a window longer than available coverage or with missing evidence. Windows beyond
one hour include retained board observations when available; partial history
shows the covered total. Core rollups crossing a shorter window boundary are
excluded whole rather than prorated. Unreported members are excluded from
totals and make the total approximate; `?` lists never-reporting members.
Resets, new sessions, invalid counters, gaps and failed reads rebaseline without
inventing tokens. Refreshing the retained usage source keeps older board observations without
bridging uncovered intervals.
Changing the observation policy starts fresh observations; core history seeds again
on the next admitted source entry.

While the usage meter is on, Agents shows the selected provider quota on each
Usage row. Click 7d/5h in its heading, or use the existing `u` controls (arrows,
Enter and Tab fields); changing quota never changes the token chart or rereads
Core. Bars and percentages are neutral. The rate uses the last 2h of observed
quota changes, divided by actual source elapsed time; its arrow compares with
the preceding 2h. Token Each/Range controls do not affect either horizon. A reset
or percentage drop starts a new suffix. Fewer than two observations or less than
30 minutes of spacing means `—`; an unknown prior rate shows no arrow.
Only a positive known rate projects depletion. ◆ marks a projected runout before
reset; otherwise a known projection says `on pace`. Only ◆ is colored. Missing,
stale or reset windows remain unknown, and small widths omit whole fields rather
than clipping numbers. Quota Details retain full source, reset and projection
values. A footer-only balance does not claim an API-backed two-hour rate.

Digest keeps its existing account meters: fill follows remaining percent and
warnings remain independent of fill. Its held weekly pace compares recent use
with the six-hour baseline; a short window never borrows weekly pace. Numbers use
provider session reports or captured statusline footers through Core
`check --capture-only`; reads send nothing. A sample older than 2h is unavailable,
never an old percentage. The board samples only while open.

Digits count with cubic ease-out for at most 600 ms; reduced motion and summary
window switches show the exact value immediately. Eight bucket-aligned bars show
observed totals by slice: blank is no evidence, ▁ is measured zero and ▂–█ scale
nonzero values. Narrow boards drop the trend, then shorten the unit. The active
window label stays next to the meter values; lead/attention text clips if needed.
Only a terminal too narrow for the compact meter hides it. Without a
covered reading it shows `–` and a dim `no usage reported yet` line. Cycling the window still
saves the shared choice, updates the effective label and posts the window in the board notice. `?`
explains the totals, best-effort coverage, switch order and `tok` configuration.
Whole-hour labels use `h`, so 60m displays as `1h`.
Hover highlights the whole meter group. Hovering one bar temporarily replaces the
number and label with that slice's token total and age, e.g. `18k tok · 3m ago`;
observed zero reads `0 tok`, and no evidence reads `–`. Ages measure from the
slice's end: `now` below one minute, minutes below one hour, then hours. The readout keeps
its fixed width and the hovered bar uses text colour. Leaving restores live text;
there is no popup. Keyboard operation remains available without mouse motion.

## Columns and row lines

Use `[squad.<name>.rows]`; `columns` defines positional tracks and value
sources, and `lines` places cells from track zero. A string names a field,
`""` is an empty cell, and `{ field = "pending", span = 3 }` covers three
tracks. A cell can add `token = "waiting"` (or another semantic theme role),
which overrides its projected field color without changing the value. Literal
colors and legacy color aliases are refused. Missing/empty values and failed
providers without projected colors stay dim; stale-row inheritance and reverse
selection still apply. Spanned cells use the first track's fitting settings. The legacy
`[squad.<name>.columns]` form remains supported; do not set both forms.

`[squad.<name>.board] hidden_columns = ["pr_link"]` hides named original tracks
for the board and `ls` text without deleting columns, field values or authored
lines/spans. Only covered tracks can be hidden and at least one must remain.
A spanning cell shrinks to the surviving tracks in its original range; hiding
one track can shrink a different field's cell rather than remove that field.
Set the mask to `[]` to restore the original grid. JSON retains all field values
and lists the mask when nonempty.

| Setting                 | Current behavior                                                                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`, `title`         | Field name and optional column heading.                                                                                                                |
| `width`                 | Cells (1–200) or a quoted percentage (1–100%, supported since Squad alpha.8).                                                                          |
| `min`, `max`            | Cell bounds, including percentages; growing tracks default to a four-cell minimum.                                                                     |
| `grow`                  | Weight (0–100), default 0. On the board, grow with max: max wins; the weight has no effect.                                                            |
| `align`                 | `left` (default), `right` or `center`.                                                                                                                 |
| `truncate`              | `end` (default) or `middle`.                                                                                                                           |
| `overflow`, `max_lines` | `ellipsis` (default) or `wrap`; wrapped visual lines are bounded to 1–8, default 2, with a final end ellipsis.                                         |
| `priority`              | 1–100; higher values hide first when minimum widths cannot fit. Without it, a track never hides through priority selection.                            |
| `from`, `format`        | Bind a column to a supported public source (listed below); format as `text` (default), `tokens`, `age` or `count`. Squad-owned fields cannot be bound. |

Supported `from` paths are `member`, `presence`, `cwd`, `target`,
`session.driver`, `session.model`, `session.usage.tokens`,
`session.usage.remaining`, `meta.<key>`, `meta.squad.<field>` and
`fields.<configured-provider>`. Without `from`, a format reads the column's
squad field; `member`, `role`, `state`, `pending` and `note` cannot use either.

On the board, `%` is a share of the **whole content width** after borders and
row marks, before fixed columns are deducted; gaps are additional, as in CSS.
Covered percentages total at most 100%. A configured width is clamped to the
cell bounds. Without a width, the base is `min`, defaulting to four cells for a
growing track and natural content otherwise. `grow` without `max` shares the
remainder as CSS `fr`; capped growing tracks reach `max` before fr tracks share
what remains. Priority hides optional tracks whole before sizing when their
minimums cannot fit. Non-priority overflow keeps earlier sizes; a right cut
needs four visible cells, otherwise that cell hides whole. CLI lists retain
their after-gap percentage base, bounds, weighted growth and scalar fitter.
The board and `tmt ops sq ls` can therefore differ by a cell or two in a column
with a percentage width.

A column's own width/min/max/grow apply only if some line covers its positional
track. Empty cells and spans count as coverage. Uncovered trailing columns
are value-only: their fields can appear on another track without reserving
an extra column. `ls --json` adds **`valueOnly: true`** only to these column
entries; ordinary columns omit the key. Their source/format metadata and full
row values remain available. Text `ls` lists their values naturally and ignores
their width/min/max/grow settings.

This checkout example splits the remainder with task/PR weights **62:26**;
`ctx` and `model` supply footer-line values on existing tracks, not extra widths:

```toml
[squad.checkout.rows]
columns = [
  { name = "member", width = 30, truncate = "middle" },
  { name = "state", width = 10, overflow = "wrap", max_lines = 4 },
  { name = "task", grow = 62, min = 20, title = "WORK", overflow = "wrap", max_lines = 3 },
  { name = "pr_state", grow = 26, min = 10, title = "PR", priority = 2 },
  { name = "ctx", from = "session.usage.tokens", format = "tokens", width = 6, title = "" },
  { name = "model", from = "session.model", width = 14, title = "" },
]
lines = [
  ["member", "state", "task", "pr_state"],
  ["", { field = "pending", span = 3 }],
  ["", { field = "ctx" }, { field = "model", span = 2 }],
]
```

## When a rule is unclear, ask

Don't guess, and don't invent conventions. Ask the user and record what you
agree on. Examples:

- which states to use and what each one means;
- what counts as pending;
- who writes which fields;
- how members are started (worktrees, windows, sessions).

Record the agreement in your notes (`tmt notes path` prints your notebook's
path), or propose an `ops.toml` change for the user to apply. Squad never
starts members, worktrees or windows; that is yours to arrange with the user.

## Team workflow and previous board view

Workflow presets remain independent of the new shell. Squads with no layout key
use team unless their simple board form keeps crew; explicit crew, pr-queue and
minimal keep their workflow settings. Team uses crew states, pending-first order,
the existing task/PR/model projections and a 30-minute observed-age default.
A missing PR link never invokes `gh`; unavailable or failed provider values keep
the normal missing/unknown distinction. Authored row/provider tables and reminder
overrides retain their validation. They do not revive a factory board view.

## Optional observed age

The user can configure observation per squad; the threshold defaults to 30
minutes. Team enables it by default; the other layouts disable it:

```toml
[squad.product.reminders]
enabled = true
stale_after = "30m"
```

The threshold accepts whole `s`/`m`/`h` durations from 1 minute to 24 hours.
The first observation starts a grace period; existing work is never backdated.
Missing or unreadable notes, unavailable cache and rollback clocks are unknown.
Cache loss/corruption starts a new period; config edits do not reset age.
Disabling stops observation; after re-enabling, surviving fingerprint matches
keep their first-observed time. Disabled observation does no cache work and
never creates a notebook. Observation belongs to the unchanged task/state fingerprint, not an authoritative
blocked-start time. Disabled or unavailable evidence supplies no age. Request
ages use the real inbox timestamp; pending-only rows have no request age. The
workspace retains these data distinctions without the former Notes-pane or
lead/HOME-tab placement.

The row's age changes only when its raw task/state changes; links, notes and
provider refreshes do not renew it. `activityAfterUpdate` records relevant
observed PR link changes, successful current `github-pr` state transitions to
open/merged, a submitted member final, or an authoritative idle transition in
public `session.activity` after the task/state update. Ordinary `ls` and board
reads retain that idle evidence; the reminder never probes live presence or
uses self-reported activity. Cold provider data and bounded room history can
miss transitions. Idle is never guessed from silence or offline presence.

With Ops extension hooks enabled (`tmt extension hooks enable ops`) and
the provider hook installed through consented `tmt setup`, Squad may add one
informational line to the lead's next turn, including SessionStart context.
It never emits at Stop. Enabling reminder settings installs no hook. Start
observations with `tmt ops sq ls` or the board: a cold cache stays silent. Disabled,
fresh, already-claimed and non-lead cache checks call no core and take no room
lock; warm candidates revalidate the current config, room and sole lead. The
best-effort preflight examines at most 128 cache-directory entries per call.

A reminder names stale notes or counts/names stale rows with relevant activity.
The line is sanitized and at most 240 characters; one invocation has an aggregate
300 ms budget including child cleanup, capped by the host's earlier deadline.
No provider or network runs in the hook. The host isolates the hook's process
group, and nested public reads remain in it so timeout cleanup reaches them.
Context calls require an isolated process group owned by the extension.

One claim bit per content generation is atomically published before handoff.
Concurrent calls share the nonblocking room lock. A lost handoff, crash or host
cutoff after publication can lose a reminder; it is never blindly retried.
At-most-once applies while the cache survives: loss/corruption restarts grace,
and changed content starts a new generation. This is best-effort context, not a
notification queue. Board reminder-setting controls are a separate slice.

## Offline markup authoring

`tmt ops sq layout validate board.xml` checks an authoring file only; add `--json`
for a structured result. Success exits 0, and invalid/unreadable files and invalid
command syntax exit 1. It does not discover core, read Squad config, run
providers or open a terminal. **The board does not load these files.** The
interactive tree and conversation use workspace preferences; authored row and
factory values remain independent CLI projections.

The `squad-projected-v1` schema declares `$.squad.name` and `$.rows` (a collection).
With `each="$.rows" as="row"`, each row declares stable `row.id`, scalar `name`,
`presence`, `pending`, and `fields`/`colors` with valid Squad row field names.
Field names and sources reuse the row/config owners; IDs must bind stable IDs.
Binding checks repeat bodies even without data. Sources reuse column `from`/`format`
in lexical `row` scope. Provider field names are checked syntactically offline;
provider configuration/data availability is not checked. Direct binds retain display text.

```xml
<tmt-view version="1" class="flex-col">
  <tmt-repeat each="$.rows" as="row">
    <tmt-row id-bind="row.id" row-bind="row.id" class="grid grid-cols-[12_1fr] gap-1">
      <tmt-cell bind="row.name"/>
      <tmt-cell bind="row.fields.task" wrap="true"/>
    </tmt-row>
  </tmt-repeat>
</tmt-view>
```

Width steps are markup, not class prefixes. `<tmt-switch>` holds ordered
`<tmt-case min="lg">` / `<tmt-case min="md">` branches and a final `<tmt-default>`;
exactly one branch is laid out, the first whose `min` fits the container's width
(`of="terminal"` measures the whole terminal). `min` and `hide-below` take a step
name (`sm` 80, `md` 100, `lg` 140 cells), never a number, and a width exactly at a
step takes it. `hide-below="md"` on a row, column, cell or text is the one-element
form. Every branch is checked offline, so an error shows at every width.

```xml
<tmt-view version="1" class="flex-col">
  <tmt-repeat each="$.rows" as="row">
    <tmt-row id-bind="row.id" row-bind="row.id" class="flex-row gap-1">
      <tmt-cell bind="row.name"/>
      <tmt-switch>
        <tmt-case min="lg"><tmt-cell bind="row.fields.task" wrap="true"/></tmt-case>
        <tmt-default><tmt-cell bind="row.fields.task" class="truncate"/></tmt-default>
      </tmt-switch>
      <tmt-cell bind="row.presence" hide-below="md"/>
    </tmt-row>
  </tmt-repeat>
</tmt-view>
```

Integers are terminal cells: `w-4` means 4 cells, unlike Tailwind's rem scale.
Use `flex`/`flex-col`, `grid`, `grid-cols-[12_30%_1fr]`,
`grid-cols-[minmax(4,1fr)_8]`, `col-span-N`, `gap-N`, cell padding and bounds.
A cut column keeps at least four cells or hides whole; Squad chooses priority tracks before sizing.
Admission bounds: 256 KiB XML, depth 32, 20,000 parser/expanded nodes, 20,000
repeat iterations and 8 MiB bound text/ID bytes. The offline check validates the
template; runtime expansion limits require data and are not simulated here.
