# Identity self-reported status

Status is identity-owned, transport-neutral descriptive data. It is neither
endpoint presence, permission to dispatch, nor evidence that a request is running
or complete. There is one current record per identity UUID, outside presentation layout,
appearance profiles and free-form metadata. All callers use the same core service
and SQLite repository; no room-specific copies or status event log are required.

## Commands and values

```sh
tmt identity status set "Reviewing the renderer" --mood "focused" --for 60m
tmt identity status show --identity Alice --json
tmt identity status rm --identity Alice
```

Each operation accepts `--identity`. Omission uses the existing verified caller
resolver, never the first/last identity. An explicit identity works outside tmux
and without any extension installed or running. Saved and active temporary identities
are eligible; reporting status does not change lifetime, binding or membership.

`set` replaces the complete record: activity is 1–160 UTF-8 bytes, optional mood
is 1–32 bytes. Both must contain non-whitespace text and no control characters.
Blankness uses Unicode `White_Space`, matching the core validator; browser
decoding must not add ECMAScript `trim()`'s additional BOM restriction.
Omitting mood clears it. Text is inert; no markup, shell or executable capability.
The existing duration syntax applies (seconds, `ms`, `s`, `m`); `--for` defaults
to 60 minutes and must be 1 second through 24 hours. No non-expiring status.

The host samples the update time and atomically stores activity, mood,
`updatedAtMs` and `expiresAtMs`. A successful later set replaces an earlier set
and renews expiry, even for identical text. This is last-writer-wins descriptive
state, not the conditional document-edit protocol. A failed validation or inactive
identity check writes nothing. `clear` is idempotent. Neither operation changes
identity creation/update time, appearance profile revision, requests or replies.

`show` returns `{identityId, status}`. A present status contains `activity`, nullable
`mood`, `updatedAtMs`, `expiresAtMs`, and derived `stale`. Absence is `status: null`.
`set` returns the same shape; `clear` returns `{identityId, removed}`. Missing or
retired identities are errors, distinct from an active identity with no status.
At `now >= expiresAtMs` the record is stale. A clock earlier than the recorded
update also fails closed as stale. Reading never deletes or renews a record.

## Lifecycle and consumer projection

Promotion keeps the UUID and record. Unbinding/offline saved identities keep
their status; presence is displayed independently. Retirement hides status from
active projections and prevents updates. Reusing a retired name creates a new
UUID with no inherited status. Existing requests and content are untouched.

Consumers read this canonical record, not a status embedded in the map/profile.
Show one short activity/mood cue for fresh status, with reply-ready attention
taking precedence. Details show exact text, update/expiry times and a stale label;
stale text must not appear as current activity. Merely sending work never sets
status or creates typing/working indicators. Browser rendering must expire a
previously fresh cue even without another network read.
Directory/appearance observation must not reuse profile revision as a status
version: saving an avatar cannot restore an older status or erase a newer one.
Read status in a batch for the identity directory, not one query per room/actor.
Expiry rendering should schedule the next relevant deadline and resample when
the page becomes visible, not add a continuous animation or per-actor poll loop.

## List runtime driver

Collection `tmt ls --json` (alias `tmt list --json`) identity rows optionally
include `runningDriver`, a registered driver ID such as `"claude"` or `"codex"`.
It appears only when the command's existing fresh runtime observation is Running,
the row has the current active binding and live pane, and that binding's admitted
provider session is disclosed and matches the identity's remembered session.
The registered harness comes from those remembered coordinates. Otherwise the
key is absent, never null: this includes Ended or Unknown runtime evidence,
undisclosed or mismatched sessions, and historical, offline or unknown rows.

This projection reuses the command's existing observations; it adds no probe,
subprocess, PID, session ID or address disclosure. It does not depend on optional
activity telemetry in `session.activity` and grants no dispatch authority.
The existing `address`, `driver`, `resume`, `session.activity` and human output
retain their semantics; `resume.observedModel` is defined in
[the extension API](extension-api.md#last-observed-model). In particular, `driver` follows the existing address
projection and can name a remembered harness even after its process ends or
while offline; it is not verified current runtime evidence. Named or direct-pane
inspection and storage-only `identity ls` do not gain `runningDriver`.

## List driver support

Collection `tmt ls --json` identity rows optionally include `driverSupport`, a
plain string: `"full"` or `"basic"`. It is the level of the built-in agent driver
TMT already identifies for the row, in this order: the remembered session's
harness, else the preferred harness, else, for a row with presence Active and a
live pane, the driver whose declared executable name equals the pane's foreground
command exactly (directories ignored; `pi-calc` is not Pi). The key is absent,
never null, when no built-in driver is identified, including a harness that is not
a built-in driver.

`full` drivers declare lifecycle hooks and a runtime, so `session.activity`,
model, consumption, resume and channels exist for them. `basic` drivers are
detected and given skills only (`tmt run <name> <command>` still binds a pane and
runs any command, but TMT starts no driver session): their `session.activity` stays
`unknown` and `resume` and `runningDriver` are absent, so those fields are blank
because TMT cannot observe them, not because the agent is idle. The level is a
constant of the driver's descriptor, not runtime evidence. The key is advisory
like `driver`: it does not need a fresh runtime observation, may name a
remembered harness while offline, and does not verify that the process is still
that driver. It adds no probe, subprocess, PID, session ID or address disclosure
and grants no dispatch authority.

The existing keys keep their meaning and order. Named or direct-pane inspection
and storage-only `identity ls` do not gain `driverSupport`. Human `ls` shows a
dimmed `basic` note after the row of a running agent whose identified driver is
`basic`, where no action (`stale`, `shell`, a resume hint) takes the slot.

## List pane location

Collection `tmt ls --json` identity rows with a live pane optionally include
`location`, and the `pane` document of `tmt ls <target>` carries the same object:
`{"host": "tmux", "session": "work", "window": "2"}`. `host` is the host token
(`tmux`, or an external host's name). `session` and `window` are the host's own
session name and window index (decimal text), present only when the host reports
them; external hosts report only `host`. The object is absent, never null, when the
row has no live observation or the observation carries no location. `target` keeps
its text and meaning as a display string: consumers read `location`, never parse
`target`, for coordinates. The projection reuses the command's existing observation;
it adds no probe and grants no dispatch authority.

## List agent name

Collection `tmt ls --json` identity rows with a live pane optionally include
`agentName`, and the `pane` document of `tmt ls <target>` carries the same text:
the name the pane's host gives the agent it recognizes there (Herdr's agent
name). It is absent, never null, when the host has no agent names (tmux), the
row has no live observation, or the host reports no name. TMT sets that name
from the identity's name, after an explicit bind or rename and, as best effort,
after `tmt run` or `tmt resume` starts an agent: silently, except for one
warning when a driver process it started could not be cleaned up
([`name-agent`](driver-protocol-v1.md#name-agent)). It never imports that name
into the identity, so `agentName` can differ from `name`; human `tmt ls` then
shows the difference with `tmt this` to set TMT's name. The projection reuses
the command's existing observation; it adds no probe and grants no dispatch
authority.

A bind (`tmt name`, `this`, `add`, `marked`) or a `tmt mv` of a binding on an
external host asks that host for the agent name. A definite answer adds
`agentName` to its JSON result: `{"status": "set", "name"}` when the host
confirmed it, `{"status": "not_linked"}` when its driver does not offer agent
names, and `{"status": "refused", "name", "message"}` when the host refused the
name and the agent keeps its earlier name. Any other answer adds no `agentName`
and one entry to `warnings`, `[{"code", "message"}]`: `AGENT_NAME_NO_AGENT` or
`AGENT_NAME_NOT_READY` when there is no agent to name yet, and
`AGENT_NAME_UNCONFIRMED` when the call failed, possibly after the host set the
name. With `--json` the warning is only in the JSON document and stderr stays
empty; human output gives the same message as one warning on stderr. Every
answer leaves the binding as it is. An explicit bind repeated on the pane that
already holds the binding asks again. A tmux binding, a rename to the same name,
and a command that binds nothing ask nothing.

## Verification

Verify byte/control/duration boundaries, exact expiry and clock rollback, atomic
replacement and clear, restart persistence, temporary-to-saved promotion,
retirement/name reuse, explicit and inferred caller resolution, no tmux probe for
explicit selection, JSON/human/help output, and invalid-input preservation.
Consumer projection tests must prove one durable record drives both views, expiration
removes only the current cue, and reply attention wins without modifying status.

## Session binding observation

Each `tmt ls --json` row and bound `tmt whoami --json` result includes
`session.unbound`: `true` when the binding's
latest refused SessionStart is newer than its admitted session, `false` when an
exact current session has been admitted, and `null` when neither is known. The
field is a stored observation, not liveness or delivery authority. It adds no
read-time probe. A missing activity sample does not imply unbound.

A refused SessionStart in an existing bound pane records only the driver, provider
session ID, bounded reason code and observation time, fenced to the exact binding.
An admitted start clears it in the same CAS; rebind and retirement clear it with
the binding. It never creates an identity, changes remembered preferences or
permits adopting a nested, shared, pane-process or unverified provider. A hand
resume may complete that existing binding only for the same recorded driver (or
none), after the old session ended or its exact process is conclusively gone,
and with the new private main process verified under that pane by PID/start.
Unknown evidence refuses adoption. Hooks still require the user's installed,
trusted setup; missing hooks cannot supply an observation.

Explicit `this`, `name` and `add` perform the same admission after the pane bind
commits. A verified caller-session locator applies only to that exact target;
`add` from another pane cannot use its invoker's session. When a provider is
present but its exact session cannot be admitted, the bind succeeds with additive
`warnings:[{code:"SESSION_NOT_RECORDED",reason,fix}]` and `session.unbound`.
Warnings merge with existing warnings. Human output prints the same reason and
fix on stderr. With remembered coordinates, the fix names the actual identity and
tells the user to exit the agent in that pane, then run `tmt resume -- <name>`.
Without remembered coordinates, it names `tmt setup` and the next prompt needed
to learn a session through installed hooks. No hook installation, process
signal, automatic relaunch or guessed session is implied.
