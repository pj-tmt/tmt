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

- `squad`: `name`, `roomId`, `layout` (`crew`, `pr-queue` or `minimal`),
  `lead` (a row, or null) and `attention`: `state` (`waiting`, `blocked` or
  `normal`), `waiting` (members that owe the user a decision or wait for an
  answer) and `blocked` (members in the `blocked` state). The board colors the
  squad's tab by it.
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
  layout lists it first.
- A row has the optional `colors` key only when a cell has a color:
  `{field: theme token}`. `colors.state` holds the resolved state token; other
  keys come from the user's column thresholds or a field provider's suggestion.
  Colors only decorate; read the values.
- States come from the layout: crew uses `working idle blocked review testing
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

The board uses the shared TMT design tokens: `muted` for readable tabs, labels
and key hints, `accent` plus bold for focus, and `dim` for secondary values and
borders. Attention tabs keep their waiting/blocked color and counts. Selection
uses the theme's `selection` background while retaining each cell's state or
provider color; a terminal without a background color uses reverse video,
including `NO_COLOR`. Colors decorate the words and marks; never infer state
from color alone. The global theme belongs in `config.json`; per-squad theme
bases and overrides belong in `[squad.<name>.theme]` in `squad.toml`.

The detail pane shows full projected board-column values not already shown by its header, task, note, activity or links, in column order; values wrap without grid truncation, with `?` for failed providers and `–` for missing values.

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

## Optional observed age

The user can enable observation per squad; defaults are disabled and 30 minutes:

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
open/merged, or a submitted member final after the task/state update. Cold
provider data and bounded room history can miss transitions. Idle is never
guessed from silence or offline presence. This slice reports age in `ls`;
board marks, settings controls and a reminder in the lead's next-turn context
are planned follow-ups. Enabling these keys installs no provider hook.
