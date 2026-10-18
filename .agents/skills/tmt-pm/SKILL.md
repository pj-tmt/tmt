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
   - _Urgent_: tell the owning lead immediately; list every open urgent item in
     each status update until closed. Tell the maintainer if one has no movement
     for two passes.
   - _Red_: an armed PR has a failing required check.
   - _Idle_: a Codex seat is idle or review-blocked for 30 minutes with a startable
     Todo; ask its lead for the next ticket. Fixed seats keep two or three queued.
   - _Stagnant_: the same ticket and epic counts across two consecutive hourly
     checks while a seat is working.
3. **Act through the owning lead.** Send one line naming the item, missing fact
   and proposed action; request no status reply. Nudge only for a real problem,
   then follow up on the next scheduled pass. Leads staff within recorded limits
   and ask the maintainer beyond them; approval limits never mean staying silent.
4. **Read usage from tmt.** Run `tmt digest usage report --since 24h --json` once per
   pass (`--since 7d|30d` on request, `--csv usage|exchange` for a spreadsheet). It is
   the only source: no pane capture, no provider files, no sampling database or
   other store of PM's own, and no usage data in repository artifacts. Read it by the
   [usage report contract](../../../extensions/tmt-digest/contracts/usage-v1.md):
   - _Missing evidence is unavailable, never zero._ Print `usage.unavailable[]`
     identities and `limitsUnavailable[]` lines (Claude is `no_source`: Core has no
     Claude allowance source) with their `reason`; a seat whose `coverage.complete`
     is false is a lower bound; `window.toMs` is stated and excludes the newest five
     minutes.
   - _Allowance._ For each `windowMinutes` (300 and 10080) take the `limits[]` entry
     that is not `stale`: left is 100 minus `usedPercent`, `paceDeltaPoints` is the
     points over (+) or under (−) an even pace. The weekly pace is 100 points per
     168 hours (about 0.6 points/hour). Average burn is `usedPercent` divided by the
     hours elapsed (`elapsedPercent` of the window length), and projected empty is
     (100 − `usedPercent`) divided by that burn. A trend over the last hours lives
     only in PM's own pass notes. A `stale` window is printed as reset, not observed since.
   - _Seats and models._ Rank `usage.seats[]` by weighted k tokens, the same for both
     providers because Core's `input` already contains `cached`:
     (input − cached + 0.1·cached + 5·output)/1000. Claude's `input − cached` also
     holds cache creation, which Core records (`cacheWriteTokens`) but the report
     does not expose, so a Claude ranking is approximate. Split by provider through `lines`; a seat on `other` or
     `unattributed` is listed under that name. A `series[].serviceTier` is the tier in
     force at the bucket's newest token-moving read (absent is unknown, never
     `default`); rank without a tier multiplier. Model share sums `byModel` and prints
     model ids only: say `effort/fast split: unavailable (Core has model ids only)`.
     A seat's `byModel` rows may cover only part of its totals: the rest is its
     `unattributed` field (the whole total when `byModel` is null) and sits on the
     `unattributed` line. Those tokens are counted but Core could not attribute them
     to a model (Codex seats often), so print them as `model not attributed`, never
     as unknown usage, and leave per-model cost for them empty. Core has no compaction data: print the compaction
     table as `unavailable (no Core source)`.
   - _Sessions outside the seats._ A seat's totals are the tokens Core sampled from the
     seat's own admitted session. Codex subagent sessions and `codex-auto-review` run
     as separate provider sessions with no identity, and Core has no source for them
     (the 10-18 parallel run found provider-side OpenAI input 21% above the seats'
     sum). Some subagent usage does appear inside a seat's totals, so do not read a
     seat as including or excluding its subagents. Print
     `other sessions: unavailable (no Core source)` beside the seat ranking and do not
     reconcile with provider files or a second store.
   - _Credits._ `credits.points[]` is the Codex credit balance at the series resolution
     (hourly for 24h): `balance` is the provider's decimal text in its credit unit,
     the newest point is the current balance, and seats on one account give one series
     (never sum). A bucket with no point was not observed, which is not zero; `credits`
     null means no seat reported a balance (Claude has none). Draw per hour is the
     difference of consecutive points that are one bucket apart; `serviceTiers` names
     the tiers seen in the bucket (empty is unknown).
   - _Exchange._ `exchange.bySender[]` and `byRecipient[]` give who messaged whom
     (content-free counts and bytes); `pairs[]` has each sender, recipient and kind.

   Alert the maintainer in chat if projected exhaustion precedes the window's reset,
   when `paceDeltaPoints` becomes positive (the pace buffer is negative), and again
   at +7 points with options. Alert once near 5% OpenAI allowance left so the
   maintainer can decide on the Codex reset. Never change seats or models.

5. **Update Project and report.** On the owning guide's status-update schedule,
   summarize progress, merged/released work, seats, usage, epic progress and
   decisions under `Owner action pending`. Post the daily usage report on #1518
   at the guide's daily tick. Leads send event-driven updates to the PM; send
   tmt-lead only decisions about core paths/contracts or cross-squad seams.
   Broadcast team-wide API, disk and CI alerts to leads.

6. **Maintain [Pending owner decisions (#1053)](https://github.com/pj-tmt/tmt/issues/1053).**
   Keep one checklist line per decision waiting on the maintainer, with its link
   and date asked; remove it after the answer is recorded in the linked issue/PR.
   Fetch the live body before every edit. The status update's `Owner action pending`
   summarizes this durable list; it does not replace it.

## Limits

- **Code and merging:** never merge, enqueue, pin, approve PRs or push code.
- **Members and settings:** never start/retire members or change provider/TMT settings.
- **Project fields:** use existing single-select option IDs; never redefine options.
  Follow the owning guide's Priority authority and leave Owner empty.
- **Replies:** only a received reply block carries a receipt. Read a "reply from"
  notice with `tmt result` and answer with `tmt talk`.
