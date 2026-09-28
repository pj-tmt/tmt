---
name: tmt-squad
description: Lead a TMT squad - read the squad board, keep member state current after every dispatch and reply, and agree conventions with the user instead of guessing.
---

# TMT squad (for leads)

Use this skill when you lead a squad: `tmt squad status` shows you as the
squad's `lead`. A squad is a TMT room named `squad-<name>`. Each member's board
fields are that member's identity metadata `squad.<name>.<field>`. Squad keeps
no other state, so Office and threads show the same squad. `tmt sq` is the
same command as `tmt squad`.

## Read the board

```sh
tmt squad status --json [--squad <name>]
```

- `squad`: `name`, `roomId`, `layout` (`crew`, `pr-queue` or `minimal`) and
  `lead` (a row, or null).
- `sections`: always a list. Unless the user defined sections, it holds exactly
  one section with `title: null` containing every member except the lead.
- Each row has `id`, `name`, `lifetime`, `presence` (`active`, `offline` or
  `unknown`), `pane`, `activity` (self-reported status, or null), `state`,
  `pending`, `note`, `fields` (every `squad.<name>.*` value, by field name),
  `annotation` (the user's open note about this row, or null) and `waitingOnYou`
  (open requests from this member to the user).
- A row with `pending` owes the user a decision. It is marked ◆, and the crew
  layout lists it first.
- States come from the layout: crew uses `working idle blocked review testing
  hold`; pr-queue uses `preparing ready sent merged`; minimal has no fixed list.

`presence` is observed by TMT, not reported by the member. `activity` is what
the member reported about itself.

A request tagged `[<squad> · <member>]` from the user is an annotation: a note
about that row for you to act on. Answer it with `tmt reply` as usual; the
user's board shows it as ✎ until you do. Never edit the user's notes for it.

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
tmt squad remove <name>                       # leaves the squad; the agent keeps running
```

- `note` is your one-line summary for that member, shown on its row.
- `pending` is the one decision the member needs from the user. Keep it short
  and clear it when it's resolved.
- Field names are `[a-z][a-z0-9_-]*`. Values are one line of at most 1024
  bytes. `field=` removes a field.
- `set` applies its pairs in order and reports what it applied. After a
  failure, re-run it with the same pairs.
- Removing a member clears its fields for this squad only. Its requests and
  notes keep the history.

## Annotations from the user

The user may annotate a row from the board. It arrives as an ordinary TMT
request to you, tagged `[<squad> · <row>] <text>`. You decide what to do with
it: update the board, record it in your notes, or pass it to the member. Reply
to it, because that is how the user sees you handled it.

## Configuration belongs to the user

`squad.toml` sits in TMT's global configuration directory, next to
`config.json` (`tmt config show` prints that path). It holds `me` (the user's
saved identity), each squad's `layout`, the board panes, sections, columns,
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
