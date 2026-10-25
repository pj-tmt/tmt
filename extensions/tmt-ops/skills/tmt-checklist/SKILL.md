---
name: tmt-checklist
description: Add, list and complete items on your own conversation checklist, the list the Ops board shows next to you. Use when asked to "add to the checklist", to track a follow-up for the person you work with, or when a message references an item as #Title (3f2a). Not tmt notes and not your provider's todo tool.
---

# Your conversation checklist

Each agent has one conversation checklist per squad: the list its Checklist button
opens on the Ops board. The person you work with reads it there, adds items from
chat, and sees who added or completed each one. Use it for follow-ups they should
see; it is shared and attributed.

It is not `tmt notes`, which are private notes for yourself and never shown on the
board, and it is not your provider's todo tool, which is scratch for one session.

## Commands

`tmt ops sq checklist` acts as you, on your own list. No UUIDs are needed; the room
is your squad. If you are on several squads the command lists them: add
`--room ROOM_UUID` to choose one.

```sh
tmt ops sq checklist add 'Rerun P02 on the final head'
tmt ops sq checklist add 'Read the design' --reference https://example.org/design --body 'Before Friday'
tmt ops sq checklist ls
tmt ops sq checklist done 3f2a
```

`add` prints the item the way chat names it, `#Rerun P02 on the final head (3f2a)`:
a title and the shortest unique prefix of its UUID (at least 4 hex characters).
Use that form when you mention the item to the person.

`done` takes that short id, any other unique UUID prefix of at least 4 hex
characters, or the full UUID. When a message references `#Title (3f2a)`, the short
id in the brackets is what you pass. `tmt ops sq checklist show --room ROOM_UUID
--member YOUR_NAME --item 3f2a` reads one item; an ambiguous prefix is refused with
its candidates and a missing one says so.

## Rules

- Never mark someone else's item done unless you were asked to, or you did exactly
  that work. In particular, items Ben added are his: leave them open unless he
  asks you to complete them.
- Keep titles short and put detail in `--body`. Items are visible on the board, so
  never put secrets in them.
- You cannot edit, archive, reorder or delete from these commands; those stay with
  the board. `ls` shows what is open and what is done.
- `done` on an item that is already done changes nothing and says so.
