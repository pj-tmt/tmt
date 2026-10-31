# Orchestration (v1)

Core owns one resolver for plans and personal focus. Plan/focus writes share an
effect preview and explicit consent. Receiving uses existing Digest admission,
claims and transport; sending rules and talk limits use existing request
admission; jobs remain a separate slice.
No timer is installed or resident process added. Existing readiness, approval,
person and delivery owners retain authority.

## Commands and API

```text
tmt orch [--range 1d|7d|30d] [--at TIME] [--plan NAME_OR_UUID] [--member TEXT] [--json]
tmt orch plan ls [--json]
tmt orch plan show NAME_OR_UUID [--json]
tmt orch plan add --file PLAN_JSON [--yes] [--json]
tmt orch plan set NAME_OR_UUID --file PLAN_JSON [--yes] [--json]
tmt orch plan rm NAME_OR_UUID [--yes] [--json]
tmt orch focus MEMBER DURATION|off [--yes] [--json]
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

Overview returns `schemaVersion:1`, `configurationRevision`, `receivingRevision`, `observedAtMs`,
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

Receiving also reports `available`. Before the first configured Digest publication,
it is false, `value` is null, and `proposedValue` explains the resolver's proposed
receiving value. Receiving-changing plan/focus apply fails with
`ORCH_RECEIVING_UNAVAILABLE`: “The Digest tick has not published its settings yet;
run tmt digest tick or install Digest.” Changes confined to sending, talk limits
or routine still preview and apply. A published member input identifies that
identity and publication revision in its Base source; the global master switch
uses the publication revision without a member identity.

API version 1 adds `digest.receiving.sync` with strict fields `expectedRevision`,
`enabled`, `defaultReceiving` and `members`. Receiving is `{mode:"direct"}`,
`{mode:"auto"}` or `{mode:"batch",intervalMs:N}` with positive safe integer
milliseconds. `members` contains at most 256 unique `{identityId,receiving}`
overrides. Absent identities inherit the published default, including identities
created later. CAS starts at revision zero; the first publication is revision
one, and an identical publication at the current revision stores nothing.
The result contains `revision` and a current overview `projection`, whose
`receivingRevision` identifies its observed input. Core stores this typed input;
Digest alone reads its TOML and the tick consumes Core's resolved receiving.

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
No settings row is created until an explicit configuration write. No missing plan
or job is fabricated; job dispatch and catch-up-once belong to slice 3 (#4891).
`orchestration.read` is additive: the API envelope stays version 1.
`--plan` filters the displayed timeline; effective member policies still include
all competing plans, and `plans` retains their source definitions.

## Configuration writes

`plan add` consumes the same bounded full plan document as preview, with
`revision: 1`. `plan set` replaces the selected plan, requires its current revision
and keeps its UUID; `rm` selects that exact plan. Name/priority collisions refuse,
never reorder. Focus takes one canonical active identity (offline allowed), a
positive `ms`/`s`/`m`/`h`/`d` duration of at most 30 days in whole milliseconds, or
`off`. Bare duration numbers mean seconds. The normalized focus intent has
explicit start/end instants; approval does not silently extend the captured end.

Each command prints an effect preview before saving. Interactive use requires
an explicit affirmative answer. Noninteractive/JSON without `--yes` returns a
preview with `requiresConsent:true` and saves nothing. With `--yes`, JSON emits
one result containing the preview and the committed configuration revision.
Running work is never interrupted and no agent is stopped.

`orchestration.write` is additive in API version 1. Input is strict at every
nested object; unknown/duplicate fields, unsafe integers, invalid schedules,
sub-millisecond durations and oversized candidates fail admission.

```json
{
  "version": 1,
  "operation": "orchestration.write",
  "input": {
    "phase": "preview",
    "mutation": {
      "kind": "focus.set",
      "identityId": "10000000-0000-4000-8000-000000000001",
      "startMs": 1793109600000,
      "endMs": 1793116800000
    }
  }
}
```

Mutation tags are `plan.add` (`candidate`), `plan.set` (`plan`, `candidate`),
`plan.rm` (`plan`), `focus.set` (`identityId`, `startMs`, `endMs`) and
`focus.clear` (`identityId`). Preview normalizes selectors to UUIDs and returns
`schemaVersion:1`, `configurationRevision`, `previewAtMs`, `timeZone`, `mutation`,
`before`, `after`, `changes`, nullable `validUntilMs`, `requiresConsent:true` and
`previewDigest`. Apply input copies the normalized mutation and requires
`phase:"apply"`, `expectedConfigurationRevision`, `previewAtMs`, `previewDigest`
and `consent:true`. The digest fences this preview; it authenticates no caller.
As with metadata writes, the same-user envelope names no originator.

The writer acquires one IMMEDIATE transaction, reads the current snapshot and
revalidates the preview. Configuration, selected UUID, roster/name, zone or
relevant time-boundary changes refuse with `ORCH_CONFLICT` (CLI exit 5) and require
a new preview. Missing identities are `ORCH_MEMBER_NOT_FOUND` (CLI exit 3).
Successful actual changes increment configuration revision once plus the affected
row revision; exact no-ops increment neither. Declined/failed previews and rolled
back writes change no configuration or journal rows. A commit/close error may be
uncertain: inspect the current revision before retrying, never replay silently.
Apply returns `schemaVersion:1`, `changed`, `configurationRevision`,
`requiresConsent:false` and the frozen `preview`. CLI and API use one encoder.

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
Plan/member batch intervals are positive whole minutes, at most 24 hours.
Base retains existing Digest interval precision; it is not rounded to this plan-field bound.

`effective(snapshot, identityId, atMs, calendar)` is the sole rules owner. Base is
always present: Awake, configured Digest receiving (direct when absent), anyone
sending, routine run, unlimited. Digest publishes its typed configuration through
the public Core boundary; Core never reads the extension's TOML or guesses a
mode from an expiring hold deadline. Base receiving retains its published revision
in provenance. Plans, member overrides and focus override Base only here.
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

Digest's global-off master switch is above scheduled receiving; its false value
forces direct receiving while leaving state, sending and routine unchanged.
The existing `user` sender/recipient immunity remains an admission rule. Focus
has no lead/setter/owner-name exemption. Sleep holds urgent automatic agent
traffic too; urgency is preserved on the canonical request, never rewritten to
fake an ordinary hold.

## Receiving

Receiving admission asks the resolver at the transaction's captured instant; a
plan change does not wait for a cached tick decision. Ordinary direct, auto and
interval modes reuse Digest's current opportunities, caps and claims. Focus
withholds non-urgent automatic traffic at idle and provider turn-boundary
opportunities; Sleep withholds urgent traffic too. No cap or `whenBusy` flag
bypasses these restrictions, readiness, approval, membership or launch evidence.
The existing same-user person/passthrough authority remains separate.
Holding still requires the existing finite Digest window: if Digest is removed
and its tick stops, a stale settings publication creates no continuing hold.

### Incoming summaries and wake spacing

The transaction that actually holds Incoming under effective Urgent/Hold
creates or coalesces one pending class marker for the canonical identity UUID.
It stays pending while current receiving blocks those classes; release does not
reconstruct historical policy. Base Auto never creates a marker, and holds from
before the upgrade keep the ordinary Digest lifecycle. The marker keeps no range
history or second copy of held membership. Current policy and held items determine the
released classes; Sleep changing to Focus releases urgent Incoming separately
while ordinary Incoming remains held. Other simultaneous class releases coalesce.
An overlapping effective restriction still prevents an inadmissible wake.

The existing finite Digest tick calls additive API-1 `digest.wake.step` with
strict `{budgetMs,limit}` input: 1..60,000 whole milliseconds and 1..256
candidates. At most one attempt is admitted per step. Due identities are scanned
in canonical UUID order; offline, busy or approval-blocked members remain pending
without consuming a transport admission or blocking later ready recipients.
A singleton durable clock separates admitted attempts by at least 1,000 ms,
including across restart and wall-clock rollback. Budget exhaustion leaves work
pending. Per-identity flushes and provider hooks retain their pinned recipient;
they may defer but never deliver another identity's wake.

Only the claim freezes the exact cutoff, eligible Incoming membership and counts
in the existing checklist, tagged `incoming_summary`. N counts all eligible
Incoming still held for that released class; K is its stored urgent subset.
Canonical open-Inbox/first-final eligibility excludes expired, withdrawn,
answered and otherwise non-answerable requests. Results/timeouts keep their
ordinary Digest lifecycle. Later arrivals cannot join a claimed summary.
If no eligible Incoming remains, no notification is sent.

The existing Digest transport sends exactly
`you have N messages, K urgent: tmt inbox`. This projection carries no bodies,
member request IDs or receipts, reserves no captured input, and never enters
request-ID prompt confirmation or reoffer. Notification does not pull,
acknowledge, answer or complete its requests. Delivered or uncertain is terminal;
the claimed opportunity closes with settlement and sealed membership grants no
replay. Only other unsealed eligible work keeps the marker pending.
Only a proven-unsent preflight permits a later opportunity. Possible transport
is uncertainty, not automatic retry permission.

The step returns `schemaVersion:1`, a typed delivery outcome or
`nothing_due`/`unavailable`, nullable `identityId`,
`projection:"incoming_summary"`, frozen `incomingCount`/`urgentCount` when
claimed, and nullable `nextAtMs`. It uses the existing claim/transport/settlement
owner, also reached by `digest.checklist.flush`; there is no separate wake
delivery path. Digest selects the step from its existing once-per-run capability
read. Explicit absence retains an older Core's existing tick behavior; an
advertised-operation error never falls back after a possible effect.

Real wake admissions and outcomes use the existing transaction-local journal and
frozen claim provenance. Reads, config saves, marker scans, blocked/no-op
observations and rollback append no decisions. No new command, daemon or timer
is added. Jobs, catch-up once and the merged `tmt tick` remain slice 3.

## Sending and talk limits

Sending rules and talk limits apply only to a new request between two agents:
the originator and the recipient are both identities and neither is the `user`
person. Anonymous senders, a person's passthrough text, reply/answer finals,
result notices and Digest checklists are never refused, counted or held. Request
admission decides at its transaction's captured instant, inside that same
transaction, with the same resolver; there is no other entry point, counter or clock.

Leads are an explicit list of canonical identity UUIDs in the orchestration
configuration, replaced by `orchestration.write` mutation `leads.set`
(`identityIds`, at most 256, unique) or `tmt orch leads set NAME...`. Core never
reads another component's squad metadata. A retired identity stays stored and
is dropped from the effective list at read time.

The sender's effective `sending` decides the route: `anyone` allows every
recipient, `leads` only current leads, `paused` no agent. A restricted route is
refused before anything is stored: `ORCH_ROUTE_RESTRICTED` (CLI exit 5), no
request, no paste. The refusal is journaled (subject = sender, action `block`,
reason `route_restricted`, source = the sending field) in its own transaction
after the preparation rolls back. `--urgent` does not bypass it.
`route.allow` (`fromId`, `toId`, `untilMs`, optional `actorId`) or
`tmt orch route allow FROM TO [--until TIME]` lets that ordered pair pass its rule
until `untilMs`, at most 24 hours ahead; a later allow replaces it.

The pair limit is the smaller effective `talkLimit` of the two members; `null`
is unlimited, the default. The count is the new requests between them, both
directions, prepared in the last 60 minutes or since the pair's last resume,
whichever is later; requests from a current lead and held requests are not
counted, and a lead's own requests are never held. At the limit the request is
stored and held, never dropped: no wake and no Digest item. Talk returns
`status:"held"`, `waitingFor:"person"`, and `x show` reports the same. The hold
is journaled (action `delay`, outcome `held`, reason `talk_limit`).
A held attempt's expiry becomes the earlier of the hold instant plus 7 days and
its prompt-retention horizon; it is never extended past the retained text.
`talks.resume` or `tmt orch talks resume A B` releases every live held request
of the pair through normal delivery, gives each a fresh normal expiry from the
release instant, and restarts the pair's window at that instant. A held request
left unresumed past its expiry expires unsent (never sent, never retried) with
one journal entry; `talks` and `x show` show it as expired-unsent and resume
skips it.
`--urgent` does not bypass the limit.

`route.allow` and `talks.resume` use the same preview, consent and CAS as other
writes. A caller attributed as a verified agent that is not a current lead is
refused with `ORCH_NOT_PERMITTED` (exit 1); a person at a terminal (no verified
agent caller) may always. This is local attribution, not authentication.

`orchestration.read` adds `talks` and `routes` views, both with optional
`memberQuery` and a page cursor (`limit` 1..256, opaque `after`). `talks` lists
pairs with counted or held requests in the window: members with their effective
`talkLimit`, `count`, `limit`, `lastAtMs`, `windowStartMs`, `state` (`open` or
`held`), `heldCount` and up to 20 `held` entries (request ID, sender/recipient
UUIDs and names, time; no bodies or receipts), plus `leads` (with `retired`) and
live `routeGrants`. `routes` returns, for each ordered pair among at most 256
selected members, the verdict `allowed`, `blocked` or `needs_person` and its
source (`base`, `plan`, `member` or `grant`). An unknown or missing view is not
available, so an older Core degrades cleanly.

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

Receiving writes record real newly created holds by request and admitted checklist
claims/settlements by identity. Claim provenance is retained with the checklist;
settlement uses that frozen evidence even after a plan edit. Unchanged holds and
idempotent settlement append nothing. Job and sending producers arrive with their
own later slices. Reads, preview and configuration saves never fabricate entries,
acknowledge, renew retention or prune. Writers prune within a bounded budget; reads filter
expiry and expose journal coverage. Normal storage-open/schema admission remains
with its existing owner. No Ops files or provider configuration are read.
