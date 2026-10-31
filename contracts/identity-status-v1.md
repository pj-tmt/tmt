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

## Plain program

A `tmt run <name> <program>` whose program no built-in driver claims (a shell, a
build, any plain command) binds a member that runs no agent. While that binding is
active, its `tmt ls --json` row and its `identities.status` entry include
`plainProgram`: the basename of the program's first argument, at most 64
characters with control characters removed, never its arguments. The key is absent
otherwise. Such a member receives nothing: a message pasted there would run as a
command, so `tmt talk` refuses with `RECIPIENT_NOT_AGENT` and Digest checklists and
notices stay held ([request/response](request-response-v1.md)). The first AI agent
that starts in the same pane and binds through its driver hook replaces it; its
admission clears `plainProgram` in the same transaction, and the member then
receives as any other. The key is a stored launch fact, not a fresh probe.

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

## List detected panes

`tmt ls --detected` adds one read-only discovery scan after the member listing
has finished and closed storage. Without the flag, `tmt ls` does the same work
and prints the same bytes as before. The scan uses the selection, the
observation allowance and the row and source schema of
[discovery v1](discovery-v1.md), which owns them; `--host` and `--socket` are
valid only with `--detected` and select only the scan, never the member rows.
`--detected` conflicts with a target, `--room` and `--here`; `--saved`,
`--temp` and `--all` filter the member sections only.

Collection `tmt ls --detected --json` adds a top-level `detected` key, present
exactly when the flag is given and absent otherwise. A failed member listing
keeps its own error document and exit code, runs no scan and has no `detected`.
`detected` is a discovery v1 document, `{version, observedAtMs, sources,
candidates, truncated}`, with fewer candidates:

- A candidate is removed when a stored binding of a non-retired identity holds
  its exact (host, socket, pane ID), taken from the observation, never from a
  target, a title or a name. The bindings are read after reconciliation and
  before the lifetime and room filters, so an Unknown member, and a member that
  `--saved` or `--temp` hides, still removes its pane. A candidate whose
  `identity` is set is removed too. A failed read of the bindings is a member
  listing error.
- Only agent-like candidates stay: `display.agentName` is set, or
  `runtime.driver` names a registered driver. This is an advisory heuristic. A
  host can label a shell, and an empty `candidates` list does not prove that no
  other panes or agents exist; `tmt discover` stays the full inventory.
- Remaining rows keep their discovery meaning, including `stale`, `hook_only`
  and conflicting owners (`state: "duplicate"`), which are not adoptable.

A detected row is presence only. It has no `id`, `name` or `address`, never
enters member or address indexes, sender resolution or dispatch, and its agent
name is display text, never a name suggestion or a link to an identity. An
absent, offline, slow, failing or malformed host is scan doubt in
`detected.sources`; the command still exits 0.

A scan child that is not cleaned up makes a **partial list**, the only
non-zero outcome the scan can add. The command writes exactly one document and
exits 1: the shared error envelope `error: {code: "CLEANUP_ERROR", message}`,
`status: "partial"`, the complete `identities`, and `detected` with that source
`unknown` (`cleanup_failed`), `candidates: []` and `truncated: true`. When an
implicit selection fails before it chooses a host, that source has
`host: null` (text: `scan: unknown (cleanup_failed)`); an explicit `--host` or
`--socket` names its host. `status` is absent otherwise. This partial list is not the guarded add `partial`
([discovery v1](discovery-v1.md#guarded-existing-binder)).

Human `ls --detected` adds a `DETECTED` section after `SAVED` and `TEMPORARY`
(see the [CLI style](../design/cli-style.md#lists)). Each adoptable row gets one
`hint: tmt add <pane> <name> --expect <digest> --socket <socket>` line, with the
pane and socket shell-quoted and `<name>` left for the user. JSON adds no argv:
consumers build it from `pane`, `socket` and `expect`. A partial list prints the
member sections, the failed source under `DETECTED`, and the error on stderr.

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
permits adopting a nested, shared, pane-process or unverified provider. Existing
exact-session shared-host routing is unchanged. A hand
resume may complete that existing binding only for the same recorded driver (or
none), after the old session ended or its exact process is conclusively gone,
and with the new private main process verified under that pane by PID/start.
Unknown evidence refuses adoption. Hooks still require the user's installed,
trusted setup; missing hooks cannot supply an observation.

### Managed launch session learning

A managed launch records a bounded observation before spawning its provider.
Without a confirmed native owner incarnation, it refuses with `RUNTIME_CONFLICT`
before spawn or marking a resume pending, says no new agent was launched and
gives a retry step. Non-provider commands are unchanged.
One binding-owned row per exact launch owner PID/start holds both the intent and
any early SessionStart. It grants no presence, delivery or resume authority.
Only the already verified private main provider may supply that event; the
ordinary binding, driver, native child/owner and session admission fences remain.
Foreground admission consumes a matching early event and remembers its session
in the same transaction. A matching intervening End stays ended. A concurrent
binding or preference change wins; no event replaces another session's key.
An admitted start commits the session before optional context work.

For a recognized Codex npm launcher, the driver may select its unique immediate
native child within the existing foreground observation budget. The native and
launcher incarnations and their exact original launch-owner edges must all be
proved; uncertainty grants no session authority. The original owned launcher
retains process control and its exit status. Its exit alone cannot end the native
session or stale a resume: those settlements require conclusive evidence that
the exact native incarnation ended, under the existing binding and owner fences.

The row retains only driver/session coordinates, bounded driver state, fixed
diagnostic codes and times. It is settled at launch exit, removed with its
binding on rebind/retirement, and retained for at most 24 hours (at most eight
launches per binding; expired rows are removed on the next launch). It never
stores a transcript, context body or a second copy of preferences.

`tmt ls --json` rows expose additive `session.learning`: `null` without an
unresolved launch observation, an admitted launch's missing-start Stop diagnostic,
or its `digest_stop:` detail,
otherwise `{step,reason,nextStep}` plus `detail` when the step recorded what it
stopped on.
`tmt identity show --json` and named `tmt ls <name> --json` add `session` only
when such an observation or detail exists. Steps are `input`, `decode`, `driver_admission`,
`host_discovery`, `pane_snapshot`, `provider_verification`, `storage`, `context`.
Reasons are `in_progress`, `not_admitted`, `worker_error`, `budget_timeout`,
`spawn_failed`, `launch_ended`, `conflict`. `detail` is one fixed token, never a
payload body, path or identifier: `refused:<decoder>:<event>` when no decoder
accepted a hook payload (decoder `turn`, `prompt`, `activity_session` or
`lifecycle`, the family that owns the first-party event `SessionStart`,
`SessionEnd`, `UserPromptSubmit`, `Stop`, `Interrupt`, `Notification`,
`PostToolUse` or `PostToolUseFailure`; `refused:unknown` for any other or missing
event name), `stopped:<decoder>:<event>:<reason>` (`stopped:unknown:<reason>` for an
event outside that list) when a decoded turn, prompt or activity event that cannot bind
admitted nothing, with `<reason>` one of `activity_missing` (an activity-only event with
no activity), `not_waiting` (a finished tool call with no activity waiting),
`no_admitted_start` (only `stopped:turn:Stop:no_admitted_start`: no start was admitted
in the end's exact session/process/turn),
`observer_owns_stop` (an installed answer observer owns the turn end), `unbound_session`
(no bound launch is waiting for the provider session) or `not_current` (the bound
session or process is not the one the payload names), `host_unsupported` when the
driver found no provider process among the hook's ancestors, or `digest_stop:<reason>`
when a Digest Stop hook on an admitted launch ended with no output because a check
refused (#4871). Its `<reason>` is one of `worker_spawn`, `worker_timeout`,
`worker_error` (legacy or unclassified), `worker_stderr`, `capture_failed`,
`launch:argument`, `launch:host`, `launch:caller` (the caller is not a verified bound pane),
`launch:caller_probe` (legacy unclassified caller verification), `launch:binding`,
`launch:process`, `launch:session`, `launch:remembered`, `launch:endpoint`,
`launch:owner` (the owner is live with a start identity other than the recorded one),
`launch:owner_probe` (the owner's process probe gave no answer), `launch:owner_not_live`,
`launch:deadline` (the hook deadline passed, including during a probe),
`notice_unavailable` (the restart notice's private directory is unusable),
`handoff_invalid` or `checklist_changed`. The bounded operation details additionally distinguish:

- `worker_exit` (an unlabelled failed exit), `worker_signal`, `worker_io`,
  `worker_output_limit`, `worker:config`, `worker:serialize` and `worker:output`.
- `worker:<operation>:<cause>`, where operation is `open | ingest | fitting | claim | render | close`
  and cause is `busy | error`. Busy names the storage error class; error is another returned
  storage/request failure, never its backend message.
- `launch:caller:config`, `launch:caller:host`, `launch:caller:pane`,
  `launch:caller:storage`, `launch:caller:storage_busy` and `launch:caller:process`
  for the initial caller observation's failed boundary.
- `launch:continuation:owner_probe` or `launch:continuation:<operation>:<cause>`,
  where operation is `open | write | close | read` and cause is `busy | error`,
  for the separate awaiting-session continuation.

These labels change no hook budget or admission decision. Every `launch:*` detail uses
step `provider_verification`, `capture_failed` uses `storage`, and other names use `context`.
Its `reason` is `spawn_failed` for `worker_spawn`,
`budget_timeout` for `worker_timeout` and `launch:deadline`, `conflict` for
`notice_unavailable` and `checklist_changed`, and `worker_error` otherwise. A Stop with no
Digest due, a claim that finds nothing, or a restart notice an earlier Stop already
showed is not a refusal and stores nothing. A later step of the same
run drops it. A run that cannot bind (a turn, prompt or activity event, or a payload of
no known event) never replaces the step, reason or detail an earlier run stored for a
launch that is not admitted yet; it names its own reason only when no detail is stored.
A lifecycle event (`SessionStart`, `SessionEnd`) is the launch's own run and replaces
them. On an admitted launch, `stopped:turn:Stop:no_admitted_start` remains visible
through that hook's successful completion (step `storage`, reason `not_admitted`),
without projecting `session.unbound=true`; its next step says to check the provider's
start/stop hook ordering. It retains no final and settles no bound input. A later hook
stage may clear or replace it under the ordinary rules; a pre-existing Digest refusal
keeps its own priority.

An admitted session otherwise suppresses an older failure, except a `digest_stop:` detail:
it stays visible on the admitted launch until the next Digest Stop of that launch
delivers, a later refusal replaces it, or a later launch starts its own row. No other
hook run replaces or clears it, and it never projects `session.unbound=true`; its
`nextStep` says that the next turn end tries the Digest again. The Digest Stop stores it
best effort within its own budget, selecting only the pre-existing admitted row its
launch coordinates name, and never changes its own output for it. These reads are
stored-only; human listing/show gives the same
recovery step. An unresolved observation projects `session.unbound=true` and
the existing message-state `session_not_bound` reason without authorizing or
blocking delivery itself. `nextStep` never suggests resume without remembered
coordinates: it asks the user to exit the agent and either resume the retained
session or start through `tmt run`.

Hook diagnostics share the original work budget; they never extend it, retry a
hook or leave an unbounded writer. A failed optional context step cannot erase
an already committed session. If even the bounded diagnostic cannot be saved,
the provider-facing hook reports that fact without claiming a stored result.

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
