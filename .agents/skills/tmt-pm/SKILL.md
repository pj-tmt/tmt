---
name: tmt-pm
description: Run the hourly project-management pass for this repository. Find blocked, stale, unstaffed, oversized and waiting-on-maintainer work, move it with the owning lead, and report to tmt-lead.
---

# TMT project management

Use this skill when you act as the project manager (`tmt-core-pm`) or audit
delivery flow. [Project tracking](../../../DEVELOPMENT.md#project-tracking)
owns the Project fields, Status meanings and the tracker rules. This skill
owns the procedure only.

## Ticket levels

- **Tracker (`Feature: <name>`)**: a product item the maintainer set. A squad
  lead may propose a new tracker to tmt-lead, but it is opened only after the
  maintainer approves it. The PM never creates a tracker.
- **Child issue**: one outcome with acceptance criteria and normally one
  reviewable PR (about 1,500 changed lines or fewer). Leads and the PM may
  open children under an existing tracker.
- **Split** a child when its progress is invisible: several PRs, several
  squads, or a long open period without movement. Propose each split to the
  owning lead first; open the children after the lead agrees.

## Hourly pass

1. **Read state in one batch.** Make one GraphQL query for Project items
   (Status, Squad, Owner, Agents, parent, updatedAt) and one for open PRs
   (head, checks, mergeable, updatedAt) and the merge queue. Read `tmt ls`
   for active members. Do not query items one by one.
2. **Detect.**
   - _Blocked without an owner_: the work waits on something that has no
     issue, no owner, or no recent movement.
   - _Stale_: In Progress or blocked for more than 6 hours with no PR
     activity or comment. A PR with a pinned head that waits on CI or the
     merge queue, or one held for a named dependency that is moving, is not
     stale.
   - _Unstaffed_: a squad with In Progress or blocked work and no active
     member.
   - _Board drift_: a squad's `tmt sq ls --squad <name>` members differ
     from the active members working for that squad.
   - _Waiting on the maintainer_: a decision marked pending in an issue or
     PR comment.
   - _Oversized_: a tracker or child whose progress the board cannot show.
3. **Act through the owning lead.** Name the item, what is missing, and the
   next action you propose: assign the blocker an owner, file the missing
   child, rebase, split, or request staff. Follow up next hour. A lead with
   blocked work and no member requests staff through tmt-lead; "no expansion
   without the maintainer's approval" never means staying silent.
4. **Report to tmt-lead.** Send at most 12 lines under these headings:
   Blocked (item, blocker, owner, age, action), Stale, Unstaffed, Waiting on
   Ben (item, one-line question, age), Splits. Send `PM: all clear` when
   nothing needs attention. tmt-lead relays maintainer questions the same
   hour.
5. **Keep the pinned "Pending owner decisions" issue current.** It holds one
   checklist line per open decision, with its link and the date it was
   asked. Remove the line when the answer is recorded.

## Limits

- **Code and merging:** the PM never merges, enqueues, pins, approves PRs or
  pushes code.
- **Members and settings:** the PM never starts or retires members and never
  changes provider or TMT settings.
- **Project fields:** when editing a single-select field, pass existing option
  IDs, and never redefine a field's options.
- **Replies:** only a received reply block carries a receipt. For a "reply
  from" notice, read it with `tmt result` and answer with `tmt talk`.
