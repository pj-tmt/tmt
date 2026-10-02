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

With `--squad <name>` the document is that squad's; without it, it is always
`{squads: [...], you}`, one document per squad in name order (even for one
squad or none), so read `.squads[]` unless you pass `--squad`. `columns` and
`lines` are the board's row grid: each column's field, title and sizing, and
the fields each line of a row shows (`{field, span}`, field null for an empty
cell).

- A column's `width` is null, a cell count or a percentage string such as
  `"30%"`. Covered-track percentage widths total at most 100% and resolve against data width
  after row marks and gaps; `min`/`max` remain cells.
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
  answer) and `blocked` (members in the `blocked` state). The board colors the
  squad's tab by it; tab and switcher counts use `◆n` for waiting on you
  and `✗n` for blocked members, including without color.
- `sections`: always a list. Unless the user defined sections, it holds exactly
  one section with `title: null` containing every member except the lead. With
  user sections, members that match none follow in a final `title: null`
  section.
- Each row has `id`, `name`, `lifetime`, `presence` (`active`, `offline` or
  `unknown`), `pane`, `activity` (self-reported status, or null), `state`,
  `pending`, `note`, `fields` (the `squad.<name>.*` values except the internal
  leadership marker, by field name,
  with the user's column sources and field providers applied), `failed` (fields
  whose provider failed; they show `?`), `annotation` (the user's open note
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

`ctrl-r` refreshes the board in squad, leads and all views, including while
searching or composing a message, without changing the entered text. The footer
and `?` help list the effective bindings. Rebind it in `[bind]` (or a section),
or `[tabs.all.bind]` for all. F5 has no default action; an explicit
`f5 = "refresh"` binding remains supported.

The board uses the shared TMT design tokens: `muted` for readable tabs, labels
and key hints, `accent` plus bold for focus, and `dim` for secondary values and
borders. Attention tabs keep their waiting/blocked color and counts. Selection
uses the theme's `selection` background for rows and selected squad/pane tabs,
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

The detail pane shows full projected board-column values not already shown by its header, task, note, activity or links, in column order; values wrap without grid truncation, with `?` for failed providers and `–` for missing values.

## Fold board panes

In split mode, press `d` to fold or expand detail, or click a pane's title.
A folded title reads `▸ detail` and stays in place. Stacked panes reserve one
line; side-by-side panes reserve a compact title-width column. Expanded neighbours
share the freed space, and expanding restores the configured proportions.
Tab skips folded panes. With all panes folded, only titles and bindings act;
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
It applies only to split mode. `toggle <pane>` uses the same literal names;
a missing pane or tabs mode gives a notice. User and section bindings keep their
usual precedence. Runtime folds survive unchanged refreshes and squad switches
within the board session. Changed board configuration resets them; restarting
uses the configured initial state. Toggling writes no config or member state.

## Keep it current

A stale board is worse than none. Update the board as part of every dispatch
and every reply you receive, not later.

```sh
tmt squad add <name>...                       # agents that are already running
tmt squad set <member> state=review task="rotate session tokens"
tmt squad set <member> pending="approve the token rotation plan"
tmt squad set <member> pending=               # clear it once answered
tmt squad set <member> note="needs a login-vs-sweep call"
tmt squad set <member> pr_link=https://github.com/acme/app/pull/412
tmt squad rm <name>                           # leaves the squad; the agent keeps running
```

- `note` is your one-line summary for that member, shown on its row.
- `pending` is the one decision the member needs from the user. Keep it short
  and clear it when it's resolved.
- Field names are `[a-z][a-z0-9_-]*`. Values are one line of at most 1024
  bytes. `field=` removes a field.
- `set` applies its pairs in order and reports what it applied. After a
  failure, re-run it with the same pairs.
- `tmt squad lead <name>` selects the lead independently of free-text `role`
  and `lead` fields. Setting or clearing either field never changes leadership,
  and selecting a new lead preserves every member's role text.
- Legacy members with only `role=lead` still appear as lead until a role write
  would change leadership or `squad lead` records their separate marker. Listing
  and opening the board never perform that conversion. The reserved metadata suffix `lead.marker` is not a
  user field and never appears in row `fields`; leadership is shown through
  `squad.lead` and the section partition.
- Removing a member clears its fields for this squad only. Its requests and
  notes keep the history.
- `tmt squad annotate` acts as you: the identity of the pane you run in (or
  `--identity <name>`), never as the user. To talk to a member use
  `tmt talk <member> "…" --detach`; to answer what someone is waiting on you
  for use `tmt inbox` and `tmt answer` (or `tmt reply --receipt` when you were
  given a receipt). `tmt squad talk`, `reply` and `replies` were removed and
  only refuse.

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

## Columns and row lines

Use `[squad.<name>.rows]`; `columns` defines positional tracks and value
sources, and `lines` places cells from track zero. A string names a field,
`""` is an empty cell, and `{ field = "pending", span = 3 }` covers three
tracks. Spanned cells use the first track's fitting settings. The legacy
`[squad.<name>.columns]` form remains supported; do not set both forms.

| Setting                 | Current behavior                                                                                                                                       |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `name`, `title`         | Field name and optional column heading.                                                                                                                |
| `width`                 | Cells (1–200) or a quoted percentage (1–100%, supported since Squad alpha.8).                                                                          |
| `min`, `max`            | Cell bounds, including for percentage widths.                                                                                                          |
| `grow`                  | Weight (0–100) for distributing remaining space after bases and bounds; default 0 in the full rows form.                                               |
| `align`                 | `left` (default), `right` or `center`.                                                                                                                 |
| `truncate`              | `end` (default) or `middle`.                                                                                                                           |
| `overflow`, `max_lines` | `ellipsis` (default) or `wrap`; wrapped visual lines are bounded to 1–8, default 2, with a final end ellipsis.                                         |
| `priority`              | 1–100; higher values hide first when minimum widths cannot fit. Without it, a track does not hide.                                                     |
| `from`, `format`        | Bind a column to a supported public source (listed below); format as `text` (default), `tokens`, `age` or `count`. Squad-owned fields cannot be bound. |

Supported `from` paths are `member`, `presence`, `cwd`, `target`,
`session.driver`, `session.model`, `session.usage.tokens`,
`session.usage.remaining`, `meta.<key>`, `meta.squad.<field>` and
`fields.<configured-provider>`. Without `from`, a format reads the column's
squad field; `member`, `role`, `state`, `pending` and `note` cannot use either.

`%` is a share of the **whole data width**, like CSS `width: …%`: after
borders, row marks and gaps, before fixed columns are deducted. For example,
with member/state widths 30/10, task `width = "62%"` and PR `width = "26%"`
still share the whole data width, not what those fixed columns leave. Covered
percentages total at most 100%; bounds and fitting can reduce the final widths.
To split the remainder instead, use `grow`, like CSS `fr`: weights distribute
the space left after fixed widths and other bases, respecting cell bounds.

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
  ["", { field = "note", span = 3 }],
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
`d` toggles detail; click either title to toggle its pane. User toggles win at
both narrow and wide widths until the board configuration changes or the session
restarts. Widening restores automatic panes without moving focus. Custom split
boards can set `fold_below` with width 1–1000 and panes present in their layout.

Member, state, PR and model use percentage widths (22%, 14%, 24%, 16%);
task grows into the remaining space. Model yields first when space is short,
then PR; member/state/task remain. Values truncate with the existing ellipsis.

Team uses crew states and pending-first ordering. Rows show member, state,
task, PR and model (`session.model` from the existing presence read); pending
text has its own line under task. Its `pr` field uses `preset = "github-pr"`
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
row's end, and puts the notes' age on the notes pane title; the leads and all
tabs show no ages.

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
