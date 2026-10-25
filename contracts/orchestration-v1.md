# Orchestration reads (v1)

Core owns one resolver for plans and personal focus. This slice exposes reads,
preview and the decision journal. It does not enforce receiving/sending rules,
run jobs, migrate Ops cron, install timers or add a resident process. Existing
readiness, approval and delivery owners retain authority.

## Commands and API

```text
tmt orch [--range 1d|7d|30d] [--at TIME] [--plan NAME_OR_UUID] [--member TEXT] [--json]
tmt orch plan ls [--json]
tmt orch plan show NAME_OR_UUID [--json]
tmt orch preview --file PLAN_JSON [--range 1d|7d|30d] [--at TIME] [--member TEXT] [--json]
tmt orch why SUBJECT [--before EVENT_ID] [--limit 1..100] [--json]
```

The default range is one local day containing the selected instant. `TIME` is
an offset-bearing RFC3339 timestamp or local `YYYY-MM-DD HH:MM`; an omitted
instant uses one captured current time. Local ambiguous/nonexistent selections
are rejected: choose an explicit offset. `--member` matches a name prefix or
substring on canonical names, case-insensitively (prefix matches first). Plan names resolve exactly and uniquely; UUIDs are
authoritative. Reads use active canonical identity UUIDs, including offline seats.

The local API adds `orchestration.read` in the existing version-1 envelope:

```json
{
  "version": 1,
  "operation": "orchestration.read",
  "input": { "view": "overview", "rangeDays": 7, "atMs": 1793109600000, "memberQuery": "night" }
}
```

Input is a strict tagged object; unknown and duplicate fields fail admission:

| View       | Fields                                                                           |
| ---------- | -------------------------------------------------------------------------------- |
| `overview` | Optional `rangeDays` (1, 7 or 30), `atMs`, `plan`, `memberQuery`                 |
| `plans`    | Optional exact `plan` selector                                                   |
| `preview`  | Required `candidate` plan; optional overview fields                              |
| `why`      | Required `subject`; optional `before` positive event ID and `limit` (default 50) |

`subject` is `{"kind":"request"|"identity"|"job","id":"..."}`. The CLI
accepts `req_<uuid>`, a canonical identity UUID, or `job:<id>`. A journal cursor
is a committed event ID, never a timestamp or a delivery acknowledgment.

Overview returns `schemaVersion:1`, `configurationRevision`, `observedAtMs`,
`atMs`, `timeZone`, `range:{startMs,endMs}`, current `plans`, `members`, and
`timeline`. Each member includes its canonical `identityId`, current `name`,
`effective` fields, `activePlans` and `nextChangeAtMs` (nullable). Each effective
field is `{value,source}`; a source has `kind` (`base`, `plan`, `member`, `focus`),
nullable `planId`/`identityId`, revision, nullable `slot` bounds, and `defaulted`.
Evaluation uses the current configuration at the requested instant; it does not
reconstruct past plan definitions. Historical explanations come from the journal.
Timeline intervals identify the actual active plans and boundaries rather than
fixed-duration sampling. Job details are `null` in this slice, never invented
counts of existing Ops jobs.

Plans view returns the same plan documents. Preview replaces/adds the candidate
UUID in an in-memory snapshot and returns `before`, `after` and per-member
changed fields. It uses the same resolver, stores nothing and appends no journal
entry. A candidate's name/priority collision is an error, not an automatic reorder.
Why returns `items`, `nextBefore` and `coverage`; no retained evidence produces an
empty page, explicitly described as no recorded decision. CLI JSON and API share
one encoder. There is no additional transport wrapper or caller attribution.

Limits: 128 plans, 256 selected identities, 4,096 timeline transitions, 32 KiB
candidate UTF-8, 100 journal entries/page, and 30-day query ranges. Inputs and
Unix millisecond values must be safe integers; overflow/ambiguity is rejected,
never silently truncated. Human output uses the shared CLI style. Invalid CLI input
is `ORCH_INPUT_INVALID`; invalid API envelopes/query documents are `API_INPUT_INVALID`. Missing
or ambiguous plans are `ORCH_PLAN_NOT_FOUND`/`ORCH_PLAN_AMBIGUOUS`, invalid/unavailable
zones are `ORCH_TIME_ZONE`, and exceeded expansion bounds are `ORCH_LIMIT`.
CLI not-found exits 3; other failures exit 1. The local API retains version-1
fault exit 1. Storage keeps its established typed
access/refusal contract.

A fresh store shows **Base only**, configuration revision 0, and no recorded decisions.
No settings row is created until an explicit configuration write. Plan creation/editing
and personal focus writes arrive in slice 2; no missing plan is fabricated.
`orchestration.read` is additive: the API envelope stays version 1.
`--plan` filters the displayed timeline; effective member policies still include
all competing plans, and `plans` retains their source definitions.

## Plans and resolution

Plans contain `id` (UUID), `name`, positive `revision`, unique integer `priority`,
`enabled`, `targets`, `schedule`, `fields`, and optional `members` overrides.
Higher priority wins. Targets are `{"kind":"all"}`, `{"kind":"identity", "id":UUID}`
or `{"kind":"pattern","pattern":TEXT}`. Patterns support `*` and `?`, evaluated
against **current names** at evaluation time. They grant no identity or transport
authority; historical queries do not reconstruct historical names.

Policy fields are optional: `state` (`awake`, `low`, `focus`, `sleep`), `receiving`
(`{"mode":"direct"|"auto"|"urgent"|"hold"}` or
`{"mode":"batch","intervalMs":N}`), `sending` (`anyone`, `leads`, `paused`),
`routine` (`run`, `hold`), and `talkLimit` (positive hourly count or explicit
`null` for unlimited). Omission inherits; null is accepted only for unlimited.
Batch intervals are positive whole minutes, at most 24 hours.

`effective(snapshot, identityId, atMs, calendar)` is the sole rules owner. Base is
always present: Awake, direct receiving, anyone sending, routine run, unlimited.
Apply enabled matching active plans in ascending priority. Each plan expands its
state defaults, then explicit fields, then that plan's member-specific fields;
a member state likewise expands before its explicit fields. A higher plan affects
only fields it supplies. No automatic audience-specificity or creation-time rank.

| State | Receiving default | Routine default | Sending   |
| ----- | ----------------- | --------------- | --------- |
| Awake | direct            | run             | inherited |
| Low   | batch 30m         | run             | inherited |
| Focus | urgent only       | hold            | inherited |
| Sleep | hold all          | hold            | inherited |

Thus a higher Awake state restores normal receiving/routine over a lower hold.
An active personal focus applies Focus/urgent-only/hold above every plan, without
changing sending. Focus does not exempt leads. Slots and focus are half-open;
expiry is evaluated without deleting rows. Sources preserve which explicit field
or state default won. Policy computation observes no host, provider or screen.

## Calendar

One machine IANA zone is configured; absent configuration uses the system's named
zone without writing a default. There is no silent UTC fallback. Core receives
calendar conversion through a port; adapters own Jiff zone lookup and the clock.

Schedules are `{"kind":"always"}`, `{"kind":"once","startMs":N,"endMs":N}`,
`{"kind":"daily","start":"HH:MM","end":"HH:MM","allDay":false}`, or
`{"kind":"weekly","anchorDate":"YYYY-MM-DD","intervalWeeks":1|2,
"weekdays":[1,2,3,4,5],"start":"09:00","end":"18:00","allDay":false}`.
Weekdays are Monday=1 through Sunday=7; the anchor selects the first local week
in the repeat phase. An end before start means next day; equal times require
`allDay:true`. All-day means local midnight to the next local date, not 24 hours.
Weekly overnight membership belongs to the slot's starting weekday.

Intervals are `[start,end)`. A nonexistent boundary clamps to the first valid
instant after the gap. A folded start uses the earlier occurrence, a folded end
the later occurrence. A collapsed interval is omitted. Biweekly membership uses
anchor-week distance, not ISO week-number parity. These slot rules differ from
the future job execution policy; no cron parser is copied into Core.

## Decision journal and storage

The [handwritten wire vectors](../rust/crates/tmt-adapters/src/orchestration/wire/tests/vectors.json)
freeze state defaults, inheritance, explicit unlimited and source provenance.

The journal lives in Core's existing SQLite database. Plans, settings, audience,
member overrides and personal focus have one storage owner and revision snapshot.
Raw connections never cross the adapter boundary. Settings, plans, targets,
member overrides and focus retain all-column change-cursor triggers. The
append-only decision journal has no change-cursor triggers: `why` pages it on
demand by event ID, so appends and retention pruning do not wake boards.

Journal entries have committed monotonic order, recording time, frozen 30-day
expiry, typed subject/actor/recipient IDs, action/outcome/reason, configuration
revision, resolved zone and the relevant source snapshot (`sources` is an array of unique `{field,source}` entries). No prompt/message body,
credential or pane capture is stored. An append participates in the calling
owner's transaction; rollback cannot leave an explanation for an uncommitted
decision. External uncertainty remains uncertainty. Later plan edits, identity
renames and retirement cannot rewrite an entry's explanation.

Slice 1 provides append/read/retention contracts and fixtures, **no live event
producers**. Later slices record real block/delay/deliver/job-run transitions with
their existing owners. Reads/preview never fabricate past entries, acknowledge,
renew retention or prune. Writers prune within a bounded budget; reads filter
expiry and expose journal coverage. Normal storage-open/schema admission remains
with its existing owner. No Ops files or provider configuration are read.
