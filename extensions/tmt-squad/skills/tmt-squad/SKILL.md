---
name: tmt-squad
description: Lead a TMT squad - read the squad board, keep member state current after every dispatch and reply, and agree conventions with the user instead of guessing.
---

# TMT squad (for leads)

Use this skill when you lead a squad: `tmt squad ls` shows you as the
squad's `lead`. A squad is a TMT room named `squad-<name>`. Each member's board
fields are that member's identity metadata `squad.<name>.<field>`. The authoritative roster and board fields stay in TMT; optional age observations
live in a disposable Squad cache. `tmt sq` is the
same command as `tmt squad`.

## Read the board

```sh
tmt squad ls --json [--squad <name>]
```

`tmt sq ls --tab <name>` returns one configured or built-in board tab's document,
with the same rows, sections, attention, columns and lines as the board. Hidden
squads remain included. `leads` lists each squad lead; `all` lists squad summaries.
`--tab` cannot be combined with `--squad` or `--refresh-fields`. An unknown tab
returns `SQUAD_TAB_NOT_FOUND`; an empty tab has an empty rows array.

Define a cross-squad member view in `squad.toml`:

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

Run `tmt sq ls --tab needs-me --json` for that view. It includes leads and
members across all squads, including hidden squads. Names use the squad-name
rules; `leads`, `all`, `colors`, `order`, `pin` and `hide` are reserved.
`tab:<name>` distinguishes a user view from a squad in `order`, `pin` and
`hide`; unplaced views follow the default tabs in definition order. Hidden views
remain reachable through the switcher and `ls --tab`.

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
source, optional squad, and error code/message. The board marks a partial view;
text lists show warnings. Available rows remain visible, so an empty partial
view must not be treated as evidence that nobody waits on you.

With `--squad <name>` the document is that squad's; without it, it is always
`{squads: [...], you}`, one document per squad in name order (even for one
squad or none), so read `.squads[]` unless you pass `--squad`. `columns` and
`lines` are the board's row grid: each column's field, title and sizing, and
the fields each line of a row shows (`{field, span}`, field null for an empty
cell).

- A cell has optional `token`, its semantic theme role, which overrides
  threshold/provider colors for that cell. Team publishes
  `lines[1][2].token = "waiting"` for its pending second line; unstyled cells
  omit the key. This is decoration metadata, not a changed row value.
- A column's `width` is null, a cell count or a percentage string such as
  `"30%"`. Covered-track percentage widths total at most 100% and resolve against data width
  after borders/row marks on the board (gaps are additional); lists resolve after
  gaps. `min`/`max` remain cells.
- A column has optional `valueOnly: true` when no row line covers its positional
  track. It remains a value source but reserves no board width; other columns
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
  answer) and `blocked` (members in the `blocked` state). Tabs and the switcher
  reserve a two-cell leading slot: `◆ ` for waiting on you, else `✗ ` for
  blocked, else two spaces. The count follows the name (`◆ product 2`);
  both states append the blocked count (`◆ product 2 ✗1`). The marks carry
  the meaning, including without color.
- `sections`: always a list. Unless the user defined sections, it holds exactly
  one section with `title: null` containing every member except the lead. With
  user sections, members that match none follow in a final `title: null`
  section. The lead is shown first by the board and text `ls` only (below);
  the JSON never moves it into `sections`, and member counts exclude it.
- `squad.noteAnnotations` is optional: open notebook-line requests from the
  recorded user to the current lead, as `{requestId, line, quote}` with a
  zero-based source `line` and bounded sanitized `quote`. It is absent when
  none are observed; the shared bounded history also governs board markers.
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
the member reported about itself.

A request tagged `[<squad> · <member>]` from the user is an annotation: a note
about that row for you to act on. Answer it with `tmt reply` as usual; the
user's board shows it as ✎ until you do. Never edit the user's notes for it.

## Board appearance

On a squad tab the lead is the first row, in the same columns as the members, with a
dim `lead` after its name (cut first when the name cell is narrow), followed by
the dim rule `── members · N ───` (`── members · 0 · none yet ──` for a squad
with no member; the numbers exclude the lead). The rule is not a row: ↑/↓ step from
the lead straight to the first member. Opening a squad tab puts the cursor on
the lead, so Enter, `t`, `a` and the other row keys reach it. Search matches
the lead like any row. A squad without a lead keeps its member rows as they
are. Text `ls` lists the lead first in its own `LEAD` section before `MEMBERS`;
`--json` is unchanged.

The home tab is the `▚ tmt` accent block, with ◆ waiting and ✗ blocked
counts inside. Its command/config name remains `all`. With neither `tabs.order`
nor `tabs.pin` configured, home is pinned first, followed by leads and squads.
An explicit order or pin, including an empty array, keeps the existing ordering
policy; hide always applies. Opening the board writes no configuration.

Only adjacent squad tabs sharing a prefix before the first `-` are grouped,
with at least two visible tabs (`tmt · core squad`). Built-ins and user tabs
interrupt groups. Grouping changes display only: each squad keeps its own mark
slot, count, selection, click and drag target. The prefix is not clickable.
The `s` switcher retains full names, including hidden tabs.

`tmt sq board --tabs product,infra,needs-me` picks the tabs this board shows,
using names from the tab line. Squad names take precedence over user-tab labels;
use `@tab:NAME` for an unambiguous user tab. `leads` selects leads; omit `--tabs`
or use `--tabs all` for the default set. Unknown names produce a usage error
listing valid names. `--squad NAME` must be among the picks when both flags are
given. This resolver is board-only; `ls --tab` keeps its own names.

In the switcher, Space picks or unpicks the highlighted tab. `[x]` marks
picked tabs and `[ ]` unpicked tabs. Enter opens a tab and picks it on this
board; opening a squad from home does the same. Unpicking the current tab opens
the next picked tab, or home if none remain. The named action `pick-tab` is
rebindable, for example `[bind] p = "pick-tab"`; that binding replaces Space
and opens the switcher from the board. A different action bound to Space takes
precedence.

Unpicked squads share one `N not on this board` segment, dim when quiet and
lit with ◆ waiting and ✗ blocked counts when they need attention. Click it to
open a switcher limited to those squads. Picks belong to this board process,
survive refresh and resizing, and never write `squad.toml`. `tabs.hide` remains
global: hidden tabs stay out of the tab line even when opened or picked.

The line keeps the current tab visible. Left overflow shows `‹ N`; right
overflow names hidden tabs as `+N › remote◆2 docs …`, waiting first, then
blocked, then quiet, retaining arrangement order within each tier. Names remain
full unless a visible group prefix makes them unambiguous. If configured pins
leave no room for the current tab, pins step aside from the end, except the
current pin; their stored order stays unchanged. A label wider than the available
cells is shortened with `…`. Hidden tabs opened through the switcher remain
selected and marked `(hidden)`, without a drag target. There are no number keys.

`ctrl-r` refreshes the board in squad, leads and all views, including while
searching or composing a message, without changing the entered text. The footer
and `?` help list the effective bindings. The footer shows only the row actions the
selected row allows (`r reply` needs a decision to answer, `o open` a link) and,
when the width runs out, drops whole hints from the end; `q quit` and `? more`
always stay. Rebind it in `[bind]` (or a section),
or `[tabs.all.bind]` for all. F5 has no default action; an explicit
`f5 = "refresh"` binding remains supported.

The board uses the shared TMT design tokens: `muted` for readable tabs, labels
and key hints, `accent` plus bold for focus, and `dim` for secondary values and
borders. Only a tab's leading attention mark and appended blocked `✗n` use bold
waiting/blocked colors; names and primary counts keep accent/bold when selected
and muted otherwise. The fixed mark slot keeps each name's starting column stable.
Selection uses the theme's `selection` background for rows and selected squad/pane tabs,
retaining each cell's state/provider color and each tab's foreground; a terminal without a background color uses reverse video,
including `NO_COLOR`. Colors decorate the words and marks; never infer state
from color alone. The CLI theme is `theme.base` in the global `config.json`;
`tmt config show` shows its value and file. Board themes layer that resolved
theme, then `[board.theme]`, then `[squad.<name>.theme]` in `squad.toml`.
`auto` works in both `squad.toml` theme layers and both picker scopes; the global
`config.json` theme rejects it. Use `tmt sq theme set auto` for all boards.

`tmt sq theme ls` (or bare `tmt sq theme`) lists built-in bases, marking the
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
tmt sq theme set auto                      # match the terminal on all boards
tmt sq theme set tmt-light                 # all boards
tmt sq theme set mono --squad product      # this squad
tmt sq theme rm --squad product            # remove only its base override
```

Set and remove keep token overrides and the rest of the user's TOML. They
refuse if the file changed since it was read. On the board, `T` opens the
theme picker (`theme` is a bindable action). Arrow keys or j/k preview in
memory; Tab switches all-boards/this-squad scope, Enter saves, and Esc cancels.
The leads/all tabs offer all-boards scope only. A squad's own base still wins
over an all-boards preview; the picker names that masking setting. A failed
save stays open with a notice; cancel and reopen to read a changed file.
Agents change the user's appearance only when the user requests it.

When the squad tab's lead row is selected, detail shows its name with a dim `lead`
tag, state/model/cap, task, nonempty pending as `◆ waits on you`, and links.
Missing values are omitted. With no row fields set, it shows
`no row fields set · tmt sq set <lead> task=…`.
The dim `notes below · replies at right` line points to the separate panes;
lead detail reads and displays neither the notebook nor reply bodies.

For members, the detail pane shows full projected board-column values not already shown by its header, task, activity or links, in column order; values wrap without grid truncation, with `?` for failed providers and `–` for missing values.

The replies pane shows full available replies to your squad requests as safe
Markdown, using the notes pane's styles. Reply bodies are indented; prompts wrap,
and recipient/age headers stay on one line. Fenced code and unsupported Markdown
constructs appear as source text. Focus replies to scroll with arrows or j/k,
PgUp/PgDn and Home/End, or use the wheel over the pane. The scroll marks show
remaining content. Older replies without a loaded body retain `tmt result <id>`
hints; reading and scrolling acknowledge nothing.

Rows that wait on you show a single decision line: `pending` when set,
otherwise the oldest unanswered request preview. Text ends in `…` when it does
not fit; detail retains the full available text and the effective reply/jump
keys. Request age comes from the inbox timestamp; pending-only rows have no
request age. The hint uses the tab's member count and drops its oldest-member
label first when space is short.

Press `A` (`ask-lead`, rebindable) on a squad tab to open the docked prompt
with recipient-first header `→ lead <name>`.
The prompt starts with "List what waits on me: one line each with who, the
decision, your suggestion and what happens if I wait." Edit before Enter sends;
Esc cancels. Missing or changed lead/sender/squad refuses without sending.
It uses ordinary detached `tmt talk`; replies and ▚ notices follow the normal
request path.

Set the question with `[board] ask_lead`, overridden by
`[squad.<name>.board] ask_lead`. It must be a nonempty single line of at most
4000 characters. The settings editor and `tmt sq config set board.ask_lead`
use the same validation and concurrent-edit refusal as other board settings.

```sh
tmt sq config set board.ask_lead "What needs my decision?"
tmt sq config set board.ask_lead "Summarize our pending decisions." --squad product
```

Press `a` on a home, squad member or leads row to answer an open request,
otherwise to send the squad's lead a note. The input opens directly beneath the
complete selected row in an opaque full-width band, shifting rows below it.
The header names the actual recipient first: `→ docs-sweep (tmt-product)` for
an answer, or `✎ note → sol · about docs-sweep` for a note to the lead about
that member. When the row itself receives the note, the header is simply
`✎ note → docs-sweep`. The chosen waiting question is quoted above the answer input.
Several open requests require an explicit
choice before composing. `r` still opens the answer path on member/leads rows;
on a row that waits only on a `pending` decision, with no open request to
answer, it opens a note to that member instead (nothing is sent, cleared or
acknowledged until you press Enter on a non-empty note, and the pending text
stays until its owner clears it). `t` still composes a direct message. Explicit member-note bindings retain that
recipient.

Inside the band, Enter sends and Esc cancels; Tab switches answer/note when
both apply, preserving your text. The band has a visible rule in both themes
and NO_COLOR. At 80 columns the quoted question truncates before the recipient
or input, which retains at least 30 columns. A successful send closes the band
and shows `✓ sent` on the row until the next key; the cursor stays with it.
Opening, cancelling, empty input or a changed target sends nothing. A failed
send shows its error and does not show `✓ sent` or retry automatically.

## Home dashboard

The built-in `all` board shows counts, needs-you members and a blocked subgroup,
then boxed leads, cron and squads. Squads occupy one full-width column at every width, with one compact table row
per squad. Member counts align within their table column rather than at the terminal edge. A row shows squad attention, lead/model and non-lead member marks in urgency
order (◆ ✗ ◐ ● ○) and a member count. Each mark has a trailing space. Members
with unknown/custom states appear as `N other`. Sampled token windows and the lead's share follow the member count.
Selection covers the whole row, including padding.

At 100 columns and wider, observed HOME usage adds a header row below the counts:
`tok 5m ~N · 1h ~N · share 1h: <member> P%`. At 140 columns it also
shows the first window, `models sol P%, opus P%` and `N members without data`
(`1 member without data` for one). The labels use global
`[board] tok`; the top and model shares use its longest window. Squad rows and
member grids retain their own overrides. UUIDs in several shown sampling squads
count once, using the better-covered observation. Missing readings are excluded
from totals and counted once as members without data; incomplete totals and
shares carry `~`. A zero total has no share. Model attribution uses the current observed model
and is best effort. The row hides below 100 columns and when no shown sampling
squad has observed readings. Search limits it to the squads currently shown.

Tiles use the board's observed usage (see below). Missing values show `–`, measured
zero shows `0`, and partial totals/share carry `~`; a zero squad total has no share.
A sampling lead with no observed totals shows one dim `–` in the first token column. When a squad's token sampling
is off, its tile hides token cells. Token/share columns appear only when observations admit them, and the squads
heading hides the token legend when no rendered column has data. Below 100 columns
only the last two windows are eligible; wider tables also show the first window
and share when observed. Observed lead models remain visible with sampling off; without a model
observation, the model cell is omitted. Home and crew use the same short family names, such as `opus`,
`sonnet` and `sol`; unfamiliar names truncate to the available column width.
The squads heading names shared windows once. Mixed `tok` settings label each
tile's actual windows. Attention rows show only member, squad and available relative
age; blocked ages say `observed` to identify the task/state observation. Questions
appear in the inline composer after `a`. Quiet needs-you takes one line, and empty
blocked disappears. Public `tmt sq ls --tab all --json` and text retain the aggregate document.

Leads show the latest exchange with you, newest first, in one full-width square
box. Each header has a bold name, its squad from 100 columns, and an event age
at the right. The second line previews the message: ◆ means the lead asks you,
… means you asked and no reply has been submitted, and ✓ means the lead replied.
Reply wording does not change that mark. A lead with no exchange has a blank mark
and `–` age, with just one line and no blank separator around it. Partial, expired
or unavailable reads remain explicit.

`t` hides previews and the blank separators between leads, and saves the global
`board.home_replies` choice (default `true`). `e` on a lead expands its complete
wrapped message directly below its header in the same inline band used for answers,
replacing that row’s preview while open. Arrows/j/k and
PgUp/PgDn scroll that body; `e` or Esc collapses it, and `a` opens an answer or
note to that lead in the same place. Reading sends and acknowledges nothing.
Only one band can be open.

The `→ all leads` footer sits outside the box. `A` writes to all current leads;
`@` picks one lead. Both use the ordinary composer. The recipient identities
are frozen when it opens, deduplicated for dispatch and checked again before
sending. A changed sender or lead audience sends nothing. Feedback reports
each recipient's queued or unavailable acceptance; uncertain acceptance keeps
its operation ID and saved intent for inspection, with no automatic resend.

One cursor spans attention rows, leads, their footer, cron and squads. Arrows or j/k move it; Tab
keeps its board-wide pane-focus behavior and does not jump between home sections. Open on the first decision, otherwise the first
squad. Enter jumps to a member/lead, opens the squad, or composes on the all-leads footer. `a` answers an open request
through public `tmt answer`, otherwise annotates for that squad's actual lead.
The composer refuses changed targets/requests/leads and missing sender/lead;
Esc cancels and empty text sends nothing. Left/right switch tabs, `s` opens the
switcher, and `/` searches. Home has no r/R reply shortcut or numeric navigation.

## Cron on the board

The home tab shows one line, `cron · N jobs · next <time> <owner> · clock on/off/checking · c list`: the job count
and earliest active slot. It omits the job prompt, clock location and paths. Up/Down
reaches it like any row. Enter on it, or `c` anywhere, lists every squad's
jobs, hidden squads included; Enter opens the job's squad and Esc closes. The
job list retains full prompts and clock status (`no clock` means due slots are
not sent; `clock: checking…` is the first read). A running clock identifies its
holder as `session:window` inside tmux when available, otherwise by pane id.

A squad tab is split in two: members above, that squad's jobs below (as tall as its jobs, at
most two fifths of the body). The `c` list is as tall as its jobs too. Tab moves into the
jobs after the last pane and back to the first. Members who own an active job show
`cron <next>` at the row end (the first thing to drop when narrow) and in their detail. The selected job expands in
place with its full message, its next three runs and its time zone.

While the jobs (or the `c` list) have focus these keys are job keys, and `?` lists them:
Enter go to the owner (open the squad, in the list), `n` new, `e` edit, `p` pause or resume,
`x` send now, `o` reassign and `d` delete (after a confirmation). A key you bound in `[bind]`
keeps its binding. New, edit and reassign use the input line one step at a time: owner
(member name), message, then schedule (`every 3h from 09:00`, `daily 09:00`, `weekdays 09:00`,
`mon,thu 10:00` or five cron fields); Esc cancels and nothing is written. The message is sent as
typed, and an edit leaves untouched fields as stored; a message with several lines is kept and
only its owner and schedule can be changed there (use `tmt sq cron edit --message`). Changes
use the same permission and revision checks as the commands: only the recorded user or the
squad's lead can change jobs, and a job that changed since you looked is refused, not
overwritten. Failures are shown and never retried. `x` sends once, like `tmt sq cron send`.

## Manage recurring jobs

`tmt sq cron ls [--squad NAME]` lists jobs across all active squads, including
hidden boards. `show <squad> <id>` shows the exact message, pause attribution and
up to three future slots. All management commands take `--json`.

```sh
tmt sq cron add product worker --every 30m "Review the queue"
tmt sq cron add product worker --at 09:00 --on weekdays "Review the queue"
tmt sq cron add product worker --cron "0 */3 * * *" "Review the queue"
tmt sq cron edit product c1 --message "Review pending changes"
tmt sq cron pause product c1
tmt sq cron resume product c1
tmt sq cron reassign product c1 reviewer
tmt sq cron rm product c1
```

Writes require the recorded user or that squad's current lead. `--identity`
selects an explicit actor; otherwise the verified caller wins, then the recorded
user only when no caller exists. An ordinary identified member cannot fall back
to the user. Add accepts `--paused`; edit accepts the same schedule flags or
`--message`. Reassignment clears no owner but retains its pause; resume separately.

Messages must be nonempty and are retained exactly, without substitution. Jobs
live under the extension's own storage, separate from `squad.toml`. IDs are never
reused. Fixed local times skip daylight-saving gaps and use the first repeated
time; intervals keep elapsed duration. Schedule edits retain the stored zone.
Change notices are best-effort announcements to the owner; reassignment notifies
old/new owners, and the actor's own notice is suppressed. A committed change can
report notification warnings. Owner retirement pauses the job as no owner and
notifies the lead; each management invocation handles at most 16 pending hooks.
An open board keeps the clock running independently of its refresh setting.
Outside a board, `tmt sq cron run` keeps it in the current pane; `clock` shows
its pane, PID and how long it has held the lease. Only one clock holds the lease. Stop `run` with
Ctrl-C; stale leases can be taken over. Startup and takeover begin at now, so
slots missed while no clock was running are not caught up. A running clock
admits slots since its previous tick, capped at five minutes. `tick` makes one
pass over the last 60 seconds and refuses an active clock. Scheduled messages
are anonymous requests; the same room/job/slot operation is accepted once.

`tmt sq cron send product c1` sends once now using the recorded or explicit
user/lead actor. It can send a paused job with a current owner and changes no
schedule. Each explicit send is a separate action. Output confirms acceptance,
not delivery or completion; Squad stores no run results. If acceptance is
uncertain, retain the reported operation ID and recover it with `dispatch.show`
through `tmt api` before deciding on another action. Board cron controls (above) are
separate from the clock lifecycle.

## Inspect board settings

Press `,` to open settings for the shown squad or tab; `settings` is bindable.
Select with arrows/j/k, PgUp/PgDn or Home/End; the wheel scrolls. Press Enter on
an editable entry to open its value. Type a scalar or JSON array, or use Ctrl-U
to clear it. Valid input previews live behind the prompt; Enter saves, and Esc
cancels the edit without writing. A second Esc closes settings. Entries marked
`*`, including provider argv and run bindings, are read-only. Each value shows its preset/default or configuration setting source
and the path of `squad.toml`. Configured provider argv and run bindings are
shown without executing them. Close and reopen to read later config edits.

`tmt sq config show` inspects board defaults. Use `--squad product` for one
squad or `--tab all` (also `leads` or a configured tab name) for an aggregate
view, and `--json` for full values and source paths. These scope flags are
exclusive. Inspection changes no configuration or member state. JSON marks the
settings supported by `config set`; the board uses the same validation and writer.

Use `tmt sq config set KEY VALUE [--squad NAME]` for validated edits. Squad scope
is required for `layout`, `board.panes`, `board.direction`, `board.sizes`,
`board.hidden_columns`, `notes.render`, `states.STATE.color`, `reminders.enabled`
and `reminders.stale_after`. `board.refresh` and `board.ask_lead`
use the squad layer with `--squad`, otherwise the global Squad board layer.
`board.home_replies` is a global boolean (default `true`) controlling HOME's
lead-message previews and separators; it rejects `--squad`. `t` on HOME saves
this setting through the same validated writer. `tabs.order` and `tabs.hide`
always edit global Squad tab policy. Arrays use JSON;
other values are unquoted scalar arguments. Examples:

```sh
tmt sq config set notes.render plain --squad product
tmt sq config set board.refresh 10s --squad product
tmt sq config set board.hidden_columns '["pr_link"]' --squad product
tmt sq config set tabs.hide '["leads"]'
```

Editing `board.direction`, `board.sizes` or `board.panes` pins the effective workflow
layout and full flat split (direction, panes and sizes) in `squad.toml`, preserving
the untouched geometry. Future preset changes no longer replace these values.
Nested split trees are read-only and must be edited in `squad.toml`. Existing validators reject invalid values
before writing. The writer preserves unrelated keys and comments and refuses a
file changed since reading it. A conflicting board save keeps the prompt and
explains the refusal; cancel, close and reopen to load the newer file. Refreshes
retain the current valid preview and the newest roster data. Provider/run commands,
patterns and core/provider configuration remain read-only.

For the selected squad, set `reminders.enabled` to `true` or `false`, and
`reminders.stale_after` to whole s/m/h units from 1m to 24h (for example `30m`).
The settings view shows effective defaults and their sources. Preview reclassifies
known observed ages without observing or sending reminders; unknown stays unknown.
After saving, ordinary reload uses the new policy. Disabled reminders produce no
stale marks or reminder work. These edits install no provider hook and grant no
extension consent.

## Choose a board view

`tmt sq view ls` (or bare `tmt sq view`) lists factory pane arrangements:
`team`, `focus`, `notes`, `detail` and `wide`. Views change only pane positions
and fold defaults. Workflow states, rows, providers, reminders, the token meter
and theme keep their settings; crew, pr-queue and minimal remain workflow layouts.

```sh
tmt sq view set notes                      # all boards
tmt sq view set wide --squad product       # this squad
tmt sq view rm --squad product             # inherit the arrangement
```

The effective arrangement comes from a hand-written per-squad `board.layout`
or `panes`, then `[squad.<name>.board] view`, then `[board] view`, then the
workflow layout's own arrangement. `team` keeps today's responsive arrangement;
`focus` starts detail/replies/notes folded, `notes` gives lead notes most space,
`detail` places detail/replies below rows with notes folded, and `wide` uses
three columns. Team folds detail/replies below 100 cells, detail folds replies
below 100, and wide folds detail/replies below 180. Manual folds retain their
existing session rules.

Set and reset write only the selected layer's `view` key through the existing
format-preserving writer and refuse a concurrently changed file. Scoped set
refuses a custom layout with a manual-removal hint. Reset removes only `view`,
retaining custom layout and fold keys; all-boards settings remain masked by
custom and scoped arrangements. Reset drops an emptied table only when its header
has no comments; existing empty tables remain. On the board, `l` opens the view
picker (`view` is bindable). Arrow keys or j/k preview only in memory, Tab
switches all-boards/this-squad scope, Enter saves once, and Esc restores the
opening arrangement and runtime folds without writing. Data keeps refreshing.
The leads/home tabs offer all-boards scope only and retain their fixed composition
during preview, save and cancel; views apply to squad tabs. The default entry
removes only the chosen layer's view key. A custom entry identifies hand-written layout;
this-squad preview works, but scoped save is refused with a manual-removal hint.
An all-boards choice saves while this squad keeps its custom layout; the picker
names that masking setting.
A failed or stale save stays open; cancel and reopen to read the changed file.
`l` (view) and `T` (theme) appear together in help. No preset binds `jump lead`:
the lead is the first row of its tab. Bind it yourself (`[bind] L = "jump lead"`)
to go to the lead's pane from any row.
Agents change views only when requested.

## Fold board panes

In split mode, press `d` to fold or expand detail and replies together when
both panes exist; otherwise it toggles whichever exists. With neither it does
nothing silently. Footer and help show `d detail+replies ▾` when any is open, or
`d detail+replies ▸` when both are folded; single-pane labels use that pane's
title. The footer drops the whole hint when space is short. Click a pane's title
to toggle it alone.
A folded title reads `▸ detail` and stays in place. Stacked panes reserve one
line; side-by-side panes reserve a compact title-width column. Expanded neighbours
share the freed space, and expanding restores the configured proportions.
Nested percentages use the raw fractional parent, then round cumulative boundaries
to terminal cells. For example, Team at body height 21 gives detail/replies 6/7
cells rather than halving an already rounded parent into 7/6.
Tab skips folded panes. Folding a focused pane moves focus to rows when visible,
otherwise the next expanded pane; unfolding keeps an existing focus. With all
panes folded, only titles and bindings act;
`n` expands and focuses notes. A single expanded pane stays borderless; bind
`toggle rows` to fold it, then click its folded title to expand.

Set the initial state or override a binding in `squad.toml`:

```toml
[squad.product.board]
panes = ["rows", "detail"]
collapsed = ["detail"]

[bind]
d = "toggle detail"
```

`collapsed` accepts unique configured pane names: rows, notes, detail or replies.
It applies only to split mode. `toggle <pane>...` uses the same literal names
and accepts one or more unique panes, for example `toggle detail replies`. If any present pane is expanded, it
folds all present panes; otherwise it expands all. Duplicate or unknown names are configuration
errors; the action toggles the named panes present on the board and does nothing
silently if none are present.
Tabs mode gives a notice before any change. User and section bindings keep their
usual precedence. Runtime folds survive unchanged refreshes and squad switches
within the board session. Changed board configuration resets them; restarting
uses the configured initial state. Manual toggles win over automatic width-based
folds until the board config changes, including after resizing in either
direction. Toggling writes no config or member state.

## Keep it current

A stale board is worse than none. Update the board as part of every dispatch
and every reply you receive, not later.

```sh
tmt squad add <name>...                       # agents that are already running
tmt squad set <member> state=review task="rotate session tokens"
tmt squad set <member> pending="approve the token rotation plan"
tmt squad set <member> pending=               # clear it once answered
tmt squad set <member> pr_link=https://github.com/acme/app/pull/412
tmt squad rm <name>                           # leaves the squad; the agent keeps running
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
- `tmt squad lead <name>` selects the lead independently of free-text `role`
  and `lead` fields. Setting or clearing either field never changes leadership,
  and selecting a new lead preserves every member's role text and membership.
  `tmt squad lead --none` clears leadership. Former leads remain members; use
  `tmt squad rm <name>` separately when they should leave.
- Repeating `tmt squad add <name>` reports that the member is already in the
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
  Without a lead, select one with `tmt squad lead <name> --squad <squad>`.

## Keep your notebook current

The notes pane shows the squad lead's own saved-identity notebook, read-only.
There is no separate squad notebook. Find your notebook with
`tmt notes path --identity <lead>` and edit that file with ordinary filesystem
tools. The board never creates it: a saved lead without a notebook shows
`(no notes yet)`, while a temporary lead shows the
`NOTEBOOK_SAVED_IDENTITY_REQUIRED` failure text.

Click a notebook line in the lead notes pane to focus it and place the cursor.
Arrow keys or j/k move between displayed lines; PgUp/PgDn page, and
Home/End or g/G select the first/last line. The cursor follows unchanged source
text when notes refresh (nearest match for duplicates, clamped after deletion).
The wheel scrolls independently; moving the cursor brings it back into view.
Every displayed continuation of the selected source line uses the full-width
selection appearance, including reverse video with `NO_COLOR`. A fixed two-cell
gutter holds the sent marker or blanks, so notebook text stays aligned.

In Markdown notes, Tab/Shift-Tab select links; the footer previews kind and target.
Enter or clicking the selected link activates it; the first click selects only.
Esc clears link selection, and configured bindings take precedence. With no links,
Tab moves to the next pane. Plain notes and undefined schemes stay inert.
Built-ins are `tmt:jump/back/talk/answer/open/copy/annotate`; except `back`, append
`/<current-member-name-or-id>`. Talk/answer/annotate open the existing prompt,
optionally prefilled by bounded percent-encoded `?text=`; Enter submits, Esc cancels.
Custom programs require your own `[links]` entries such as
`gh = "run gh issue view {path}"`: argv only, one argument per template, no shell.
Bare #N remains plain; full GitHub issue/PR URLs are selectable. Absolute file
links reveal after resolving symlinks; configured openers get only the containing
directory. Relative paths stay plain. Opening files requires a custom scheme.

In focused notes, the annotate binding (`a` by default) opens a composer addressed
to the lead, quoting the line number and a bounded excerpt. Enter sends only
nonempty text; Esc cancels. The line shows `✎` while your request to the current
lead is open, clearing after the lead answers and the board refreshes. Notes remain
read-only. The marker uses the nearest matching quoted excerpt after an edit;
requests outside the bounded room-history window may not be shown.

The detail pane shows the selected non-lead member's own notebook after its fields,
using the same read-only Markdown/plain rendering as lead notes. A saved member
without a notebook shows `(no notes yet)`; temporary members show
`(temporary identity: no notebook)`. Only the visible selected detail is read,
on selection and board refresh. The leads/home tabs never show member detail
notebooks.

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

`squad.toml` sits in TMT's global configuration directory, next to
`config.json` (`tmt config show` prints that path). It may hold `me` (the
user's saved identity, recorded with `tmt squad me <name>`) and `me_id` (its
UUID, which lets `me` follow a rename; squad maintains it), each squad's `layout`, the board panes, sections, columns,
states and key bindings. Bindings and actions are the user's. Never edit them
silently. If a change would help, propose the exact lines and let the user
apply them.

## Observed token usage

The selected named squad shows completed-request tokens from public core history
and this board's observations, in **1m / 5m / 1h windows**. The default member grid adds the current
session model and those three totals, declared in the TEAM/crew preset rows.
Custom grids opt in with `from = "usage.w1"`, `"usage.w2"`, or `"usage.w3"` on a
column. Default headers follow `tok`; an explicit `title` stays as configured.
One-shot `tmt sq ls` has no window history. JSON keeps column descriptors without
usage values; text omits columns whose source is board-only.
When token sampling is off, usage columns hide and MODEL remains. Default
member grids keep at least 20 cells for TASK when space permits. On narrow
boards, inactive window columns step aside first, then PR, then MODEL, and
finally the active window. Pressing `w` also changes which window stays visible
longest. MODEL follows its content up to 8 cells and truncates longer names.

Input and output count once; cached input is already included in input, and
normalized reasoning in output. Mixed providers sum reported token units, not
cost or interchangeable text volume. Model attribution is best effort: a
mid-session model change attributes retained observations to the current model.
Counters update at request completion and are observed every 5–10 seconds,
not while a model writes. On entry, the board reads closed core history for up to
one hour and continues from its included counter watermark. It stores no separate
usage history and computes no money estimate.

Team enables observation; crew, pr-queue and minimal keep it off by default.
The all/leads tabs omit this named-squad meter. `w` cycles the summary's windows
through the bindable `token-window` action; member columns show all three at once.
The label always names the configured window; the number is a total, never a
per-second rate. Configure exactly three distinct ascending whole `m`/`h`
durations, from 1m through 24h:

```toml
[board]
tok = "1m/5m/60m" # for example, "5m/60m/24h"

[board.token_rate]
enabled = false
every = "5s" # 5s through 10s; independent of board.refresh
window = "1m" # initial summary window; falls back to the first configured window
reduced_motion = true

[squad.checkout.board]
tok = "5m/60m/24h" # overrides the global windows

[squad.checkout.board.token_rate]
enabled = true # individual keys override global policy and layout preset

[bind]
w = "token-window"
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
inventing tokens. Returning to a tab refreshes its recent range from core while
preserving older board observations, without bridging uncovered intervals.
Changing the observation policy starts fresh observations; core history seeds again
on the next tab entry.

Digits count with cubic ease-out for at most 600 ms; reduced motion and summary
window switches show the exact value immediately. Eight bucket-aligned bars show
observed totals by slice: blank is no evidence, ▁ is measured zero and ▂–█ scale
nonzero values. Narrow boards drop the trend, then shorten the unit. The active
window label stays next to the meter values; lead/attention text clips if needed.
Only a terminal too narrow for the compact meter hides it. Without a
covered reading it shows `–` and a dim `no usage reported yet` line. `w` still
switches the label immediately and posts the window in the board notice. `?`
explains the totals, best-effort coverage, switch order and `tok` configuration.
Whole-hour labels use `h`, so 60m displays as `1h`.

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
The board and `tmt sq ls` can therefore differ by a cell or two in a column
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
path), or propose a `squad.toml` change for the user to apply. Squad never
starts members, worktrees or windows; that is yours to arrange with the user.

## Team board preset

Squads with no layout key use team unless they set the simple board form, which keeps crew. Set `layout = "crew"`, `"pr-queue"`
or `"minimal"` to retain those presets. The top 60% contains rows beside a right column (62/38), with
detail above replies (50/50). The lead's notes fill the bottom 40%.

Below 100 columns of board body width, team folds detail and replies into title
bars: `board.fold_below = { width = 100, panes = ["detail", "replies"] }`.
`d` toggles detail and replies together; click either title to toggle it alone.
Widening restores automatically folded panes without moving focus; manual folds
keep the session policy described above. Custom split
boards can set `fold_below` with width 1–1000 and panes present in their layout.

Member, state and PR use percentage widths (22%, 14%, 24%); TASK grows from
a 20-cell minimum and MODEL follows its content up to 8 cells. Usage columns
follow the sampling and active-window policy under Observed token usage above.
Values truncate with the existing ellipsis.

Team uses crew states and pending-first ordering. Rows show member, state,
task, PR and model (`session.model` from the existing presence read); pending
text has its own line under task, styled with `token = "waiting"`. Other
presets keep their styles unless their cells opt in. Its `pr` field uses `preset = "github-pr"`
from `pr_link`, refreshed at most every 60 seconds per member. A missing link
never runs `gh`; unavailable or failed provider results follow the normal
missing/`?` rules. A `rows` or legacy `columns` table replaces the whole grid;
`fields.<name>` replaces that provider's whole table, other provider names add
to `pr`, and reminder keys override individually. Set a full `board.layout`
or `board.panes` to replace the nested pane arrangement; `direction` or `sizes`
alone is refused. Host bindings and theme selection are unchanged.

Team enables observed age at 30 minutes. Other layouts keep it disabled by
default; `[squad.<name>.reminders] enabled = false` disables it for team too.

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
never creates a notebook. The board dims a stale row and shows its age at the
row's end, and puts the notes' age on the notes pane title; the leads tab
shows no ages. Home shows blocked ages only
where this observation policy is enabled (Team by default; other layouts off);
disabled or unavailable observation provides no age. Request ages use the real
inbox timestamp, and pending-only rows have no age. Home labels blocked
age `observed`: unchanged task/state observation, not an authoritative blocked start.

The row's age changes only when its raw task/state changes; links, notes and
provider refreshes do not renew it. `activityAfterUpdate` records relevant
observed PR link changes, successful current `github-pr` state transitions to
open/merged, a submitted member final, or an authoritative idle transition in
public `session.activity` after the task/state update. Ordinary `ls` and board
reads retain that idle evidence; the reminder never probes live presence or
uses self-reported activity. Cold provider data and bounded room history can
miss transitions. Idle is never guessed from silence or offline presence.

With Squad's extension hooks enabled (`tmt extension hooks enable squad`) and
the provider hook installed through consented `tmt setup`, Squad may add one
informational line to the lead's next turn, including SessionStart context.
It never emits at Stop. Enabling reminder settings installs no hook. Start
observations with `tmt sq ls` or the board: a cold cache stays silent. Disabled,
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

`tmt sq layout validate board.xml` checks an authoring file only; add `--json`
for a structured result. Success exits 0, invalid/unreadable files exit 1, and
invalid command syntax exits 2. It does not discover core, read Squad config, run
providers or open a terminal. **The board does not load these files.** Board rows
and pane composition still come from the existing Squad settings.

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

Integers are terminal cells: `w-4` means 4 cells, unlike Tailwind's rem scale.
Use `flex`/`flex-col`, `grid`, `grid-cols-[12_30%_1fr]`,
`grid-cols-[minmax(4,1fr)_8]`, `col-span-N`, `gap-N`, cell padding and bounds.
A cut column keeps at least four cells or hides whole; Squad chooses priority tracks before sizing.
Admission bounds: 256 KiB XML, depth 32, 20,000 parser/expanded nodes, 20,000
repeat iterations and 8 MiB bound text/ID bytes. The offline check validates the
template; runtime expansion limits require data and are not simulated here.
