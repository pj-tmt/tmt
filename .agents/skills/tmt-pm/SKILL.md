---
name: tmt-pm
description: Run the project-management pass for this repository. Find blocked, stale, unstaffed, oversized and waiting-on-maintainer work, move it with the owning lead, and keep status flowing to the PM instead of the core lead.
---

# TMT project management

Use this skill as project manager (`tmt-core-pm`) or when auditing delivery flow.
[Project tracking](../../../DEVELOPMENT.md#project-tracking) owns Project fields,
Status meanings, epic approval/start rules and cadence; this skill owns procedure.

## Ticket levels

- **Tracker (`Epic: <name>`)**: a maintainer-approved product item. Follow the
  owning guide's approval and explicit-go rules; the PM never creates an epic.
- **Child issue**: one outcome, acceptance criteria and normally one reviewable
  PR (about 1,500 changed lines or fewer). Leads/PM may open direct native
  sub-issues below approved trackers.
- **Split** work whose progress is invisible across PRs, squads or a long open
  period without movement. Propose it to the owning lead; open children after agreement.

## Scheduled check and status update

1. **Read state in one batch.** Query Project items (Status, Squad, Owner, Agents,
   Priority, Released in, native parent, updatedAt), open PRs (head, checks,
   mergeStateStatus, updatedAt), merge queue and relevant timeline evidence.
   Read `tmt ls` for active members; never query every item individually.
2. **Detect real problems.**
   - _Blocked without an owner_: no issue, responsible lead or recent movement
     for the dependency. Responsibility is recorded through Squad, not Owner.
   - _Stale_: In Progress or blocked for over six hours without PR activity or
     comment. A pinned head awaiting CI/queue or a named moving dependency is not stale.
   - _Unstaffed_: active work without an active member.
   - _Board drift_: `tmt ops sq ls --squad <name>` differs from active squad members.
   - _Waiting on the maintainer_: a pending issue/PR decision.
   - _Oversized_: tracker or child progress the board cannot show.
   - _Unarmed_: green for 30 minutes. Inspect mergeStateStatus, queue and timeline
     first: DIRTY conflicts drop auto-merge, and stacked PRs may be deliberately unarmed.
   - _Red_: an armed PR has a failing required check.
   - _Idle_: a Codex seat is idle or review-blocked for 30 minutes with a startable
     Todo; ask its lead for the next ticket. Fixed seats keep two or three queued.
   - _Stagnant_: the same ticket and epic counts across two consecutive hourly
     checks while a seat is working.
3. **Act through the owning lead.** Send one line naming the item, missing fact
   and proposed action; request no status reply. Nudge only for a real problem,
   then follow up on the next scheduled pass. Leads staff within recorded limits
   and ask the maintainer beyond them; approval limits never mean staying silent.
4. **Record usage.** Read hourly provider limits sampled on the maintainer's machine
   (Claude statusline weekly allowance and Codex session rate-limit counters),
   held in a local database outside the repository. Record account/per-seat
   counters, never message content or usage data in repository artifacts.
   The weekly pace is 100 points per 168 hours (about 0.6 points/hour).
   Alert the maintainer in chat if projected exhaustion precedes reset, when the
   pace buffer becomes negative, and again at minus seven points with options.
   Alert once near 5% OpenAI allowance so he can decide on the Codex reset.
   Never change seats or models.
5. **Update Project and report.** On the owning guide's status-update schedule,
   summarize progress, merged/released work, seats, usage, epic progress and
   decisions under `Owner action pending`. Post the daily usage report on #1518
   at the guide's daily tick. Leads send event-driven updates to the PM; send
   tmt-lead only decisions about core paths/contracts or cross-squad seams.
   Broadcast team-wide API, disk and CI alerts to leads.

## Limits

- **Code and merging:** never merge, enqueue, pin, approve PRs or push code.
- **Members and settings:** never start/retire members or change provider/TMT settings.
- **Project fields:** use existing single-select option IDs; never redefine options.
  Follow the owning guide's Priority authority and leave Owner empty.
- **Replies:** only a received reply block carries a receipt. Read a "reply from"
  notice with `tmt result` and answer with `tmt talk`.
