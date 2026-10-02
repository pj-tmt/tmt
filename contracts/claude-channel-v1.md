# Claude channel delivery (v1)

This document owns the shared launch policy, Claude opt-in, the channel endpoint, the enrollment
lifecycle, the delivery result mapping, the supported provider range and the
`tmt channel` recovery command shared by every driver with a channel.
[ARCHITECTURE.md](../ARCHITECTURE.md) owns module boundaries; the request
lifecycle is owned by the request service and
[contracts/request-response-v1.md](request-response-v1.md).

## Status

Tracking: #329, delivered as four stacked changes (#712 to #715). Every section below
describes shipped behavior: the outcome vocabulary and reporting, the
`RuntimeChannel` lease port with pane attribution and the foreground callback, the
Claude send classification, the Claude enrollment and stdio server, `tmt run
--channel` with the hidden `__channel-server` command, every delivery route, the
baseline-paste evidence check and the E2E scenarios. The `tmt channel` recovery
command (#764) is shipped behavior as well.

## Evidence base

The behavior below follows the #329 spike on Claude Code 2.1.285 (claude.ai Pro
login, negotiated MCP protocol `2025-11-25`). It established that a
`notifications/claude/channel` write reaches an idle or mid-tool session, that
Claude sends **no receipt** to the server, that a write can succeed while Claude
deliberately skips the channel, and that a server crash cannot prove that an
earlier write was not delivered. A correlated durable reply is the only proof of
processing.

## Opt-in and the launch lease

- The launcher selects one mode for both run and exact resume: Default when
  neither flag is present, Disabled for `--no-channel`, Required for `--channel`.
  The flags conflict in clap and go before the identity; `resume --forget`
  conflicts with either flag. Drivers advertise only their default through
  `RuntimeChannel::enabled_by_default`; the CLI contains no provider-name policy.
  Claude advertises false and stays opt-in; the
  [Codex contract](codex-channel-v1.md#default-launch-policy)
  owns its default enrollment and exact-thread attachment.
- Claude channel enrollment on exact resume remains unsupported pending
  [#783](https://github.com/pj-tmt/tmt/issues/783). Both `tmt resume --channel`
  and `tmt run --resume --channel` fail with `CHANNEL_UNSUPPORTED` before
  enrollment writes or foreground startup, with guidance to resume without
  `--channel`. Default keeps Claude resume plain, subject to the unchanged
  no-paste check for retained enrollment.
- A driver's preflight classifies an outcome as unavailable or informational.
  Informational advisories are shown and enrollment proceeds, including Claude's
  accepted-but-untested 2.x builds; its handshake decides readiness. Codex's
  unqualified-build advisory is unavailable, as its contract defines.
  Required refuses unavailable outcomes before foreground startup:
  `CHANNEL_UNSUPPORTED` for no port, `CHANNEL_PROVIDER_UNSUPPORTED` for a refused
  or unqualified build, and `CHANNEL_UNAVAILABLE` for other enrollment failures.
  Default attempts only advertised channels. An unavailable attempt may run the
  original plain command with one visible reason line,
  `warning: tmt: <name> uses paste delivery: <reason>`, only after failed-start
  cleanup and existing pane evidence permit it and binding authority still holds.
  Unknown or retained enrollment stays terminal. Disabled does not enroll and
  never bypasses the no-paste evidence check.
- The driver owns the launch plan through `runtime::channel::RuntimeChannel`.
  `enroll` receives the user's command (selected executable and argv, resume
  substitution included), the launch directory, the identity and the pane address
  of the binding (server incarnation, pane ID and pane process), the `tmt run`
  process as launch owner and the channel directory, and returns a lease
  (`ChannelEnrollment`): the foreground command to spawn verbatim, environment
  for the provider child, optionally the provider session the driver created
  before the child starts (Claude creates none), `foreground_started` and a
  consuming `withdraw`. The CLI never parses or rewrites provider arguments. The
  environment reaches the provider child, and therefore the subprocesses the
  provider itself starts (such as its MCP server); it is never written to the
  ambient or global environment, to persisted state or to logs.
- The launcher holds the lease for the child's whole lifetime and tells it, once,
  the exact child incarnation it spawned and observed (`foreground_started`, before
  the launch is admitted; a driver that cannot record it makes the launcher warn and
  leaves the enrollment unconfirmed). It retires the lease (`withdraw`) only when no
  child was ever spawned or the same child was confirmed reaped (its wait returned).
  On every path where the foreground may still run (a failed wait, a panic, an early
  return after the spawn, the launcher being killed) it drops the lease and the
  driver's record stays exactly as it is: nothing retires a record from the
  launcher's own state, an EOF or a server exiting.
- Claude's plan is the user's command with `--mcp-config <inline JSON>` and
  `--dangerously-load-development-channels server:tmt` appended. It adds no
  `--strict-mcp-config` and writes no Claude settings or MCP configuration. The
  inline config starts the hidden `__channel-server` of the invoking `tmt` (the
  absolute path of the running executable) with the binding ID, the generation
  and the absolute channel directory.
- **Enrollment is durable and written first.** After binding and before the
  provider starts, the driver writes `<global>/channels/<binding-id>.json` (owner-only
  directory, atomic 0600 replacement) with a fresh per-launch generation, the
  launch owner (PID and start identity), the identity ID, the pane the launch runs
  in (the full tmux server incarnation, the pane ID and the pane process), and no
  foreground and no Claude process yet. That is the state "opted in, channel not
  ready". `enroll` refuses a pane that cannot be identified (an empty identifier or
  a zero PID): a record that could not be matched to its pane later would be
  invisible to every guard. A session with no record never opted in.
- A Claude launch without `--channel` creates no channel state, and no launcher removes
  another launch's enrollment, except that `enroll` prunes ended launches (see
  "Enrollment ownership and serialization").
- Non-enrolled sessions, other drivers and every other launch keep their current
  behavior, including tmux paste.

## Supported provider range

Claude Code **2.1.285 or newer within the 2.x line**. A launch probes the command's
`--version` (bounded process owner, 5 s, 4 KiB). The whole line must be
`<major>.<minor>.<patch> (Claude Code)` with canonical decimal numbers; it is
accepted when the major is 2 and the build is at least 2.1.285, and refused before
any spawn otherwise (older builds, another major line, a pre-release suffix or any
text that is not a version line).

Whether a newer build really speaks the channel is decided by the handshake, not by
a list: a build the channel was never exercised on launches with an advisory on
stderr that names it (`Claude Code <version> has not been tested with message
channels (tested: <list>)…`), and if its handshake does not complete the session
stays `not_ready`, which is terminal and never pasted to ("Delivery mapping").
Builds on which the channel was exercised against the real provider are recorded in
`drivers::claude::channel::TESTED_VERSIONS`:

| Build | Evidence |
| --- | --- |
| 2.1.285 | the #329 spike (live session, `notifications/claude/channel` reaches an idle or mid-tool session) |

Lowering the minimum or accepting another major line needs recorded evidence and a
reviewed change to `drivers::claude::channel::MINIMUM_VERSION`. Drift is detected by
contract tests over the frozen constants and the range rule and by the opt-in
developer check
`cargo run --locked -p tmt-adapters --example channel-contract -- /absolute/claude`,
which runs only `--version` and `--help`, fails when the version is outside the
range or `--help` no longer documents `--mcp-config`, and says when the build is
accepted but untested. Claude 2.1.285 does not list
`--dangerously-load-development-channels` in `--help`, so that preview flag is
covered only by the spike evidence and the handshake. The check passes against the
installed 2.1.286 (`--version` and `--help` only); no recorded session proof exists
for it yet, so it is accepted but not in `TESTED_VERSIONS`. This range is separate
from `runtime-contract`, whose resume pin is a different provider version.

## Channel endpoint and enrollment record

`__channel-server` is a stdio MCP server for one Claude process. It exposes no
tools, declares `capabilities.experimental["claude/channel"] = {}` and writes
one-way `notifications/claude/channel` messages with `params.content` set to the
prepared request payload. Replies use the existing `tmt reply` receipt path, not
a channel tool. It cannot create an enrollment: it refuses to start unless the
launch's record exists with its own generation and no Claude process yet.

The record is
`{"version":1,"bindingId","identityId","generation","launchOwner":{"pid","start"},"pane":{"host","serverId","socketPath","serverPid","serverStartTime","paneId","panePid"},"foreground":{"pid","start"},"claude":null|{"pid","start"}}`
in `<global>/channels/<binding-id>.json`. `identityId`, `pane` and `foreground` are
optional on read: a record written before pane attribution has none of them and
cannot be attributed to any pane. The launcher publishes `foreground` through
`foreground_started` once it has spawned and observed the child; the server
publishes `claude` after the handshake. The two are independent, so every check
that compares a record with an earlier read ignores both. That file name, `<binding-id>.sock` and
`.lock` are this driver's namespace in the shared channel directory; another
driver keeps its own files under different names or its own subdirectory, and
neither driver opens the other's files (a record that is not this driver's is never
parsed as one).
After Claude completes the MCP handshake (`notifications/initialized`) the server
observes its parent (Claude), rewrites the record with that process in `claude`
and only then accepts frames; this is "ready". Frames before that are refused
(`not_ready`). The record is discovery and evidence only and grants nothing:
`send` trusts it only where it agrees with the stored binding, below.

The Unix socket is `<global>/channels/<binding-id>.sock` (0600), never a network
listener. One ingress connection carries one frame,
`{"version":1,"generation","content"}`, and receives `{"written":true}` after the
notification was flushed to Claude, or `{"refused":"<reason>"}`. Writing the frame
and reading the reply share one absolute 2 s deadline, enforced at each underlying
read and write, so a slow or trickling endpoint cannot extend it; running out of
time after the connection exists is `Uncertain`. Only `{"written":true}` alone is a
write and only `{"refused":…}` without `written` is a refusal: a reply claiming
both, or neither, is `Uncertain`. The server keeps
no request-ID set: a request has at most one wake claim in the request service,
and the driver never retries. Each ingress connection has one absolute 2 s deadline
covering the frame read, the wait for the write and the answer, enforced at each
underlying read and write, so a trickling sender cannot hold the single acceptor.

## Enrollment ownership and serialization

A launch is identified by its generation (random per `enroll`) together with its
launch owner. One lock file, `<global>/channels/.lock`, taken through the existing
`file_lock::exclusive` with a bounded retry that fails closed, is held across
every mutation of the record or the socket, so no check-then-change race exists:

| Mutation | Under the lock, it proceeds only if |
| --- | --- |
| `enroll` writes the record | the old record is absent, this same launch's, or positively over: its launch owner and every process it recorded (foreground, Claude) are conclusively gone. When nothing but the owner was ever recorded (the launcher died before it published the foreground) nothing proves where the agent went, and only a relaunch in the very pane the record names replaces it. Anything alive, unverifiable or unreadable, or such a record naming another pane or none, refuses the enrollment (`Occupied`) and leaves it untouched |
| Lease `foreground_started` records the child | the record still carries exactly the lease's generation and launch owner; otherwise it writes nothing and the enrollment stays unconfirmed |
| Server binds the socket | the record is this server's pending enrollment (its generation, no Claude yet); it adopts the launch owner the record names, which every later step must find unchanged. A live second server is refused, a socket that cannot be opened is left alone and fails the start, and only a conclusively absent or refusing socket is replaced |
| Server publishes readiness (`claude`) | the record still carries the server's generation and the launch owner it adopted |
| Server removes its socket on exit | the record still carries the server's generation and the launch owner it adopted |
| Lease `withdraw` removes the record and socket | the record carries exactly the lease's generation and launch owner, and names no Claude process that is not conclusively gone; otherwise it removes nothing |
| `enroll` prunes other launches | the other record recorded a foreground or Claude, and its launch owner and every recorded process are absent from one `ps -A -o pid=,ppid=` snapshot, so the launch is over in every recorded respect; a record that recorded neither (the unconfirmed case), has a live or unobservable process, is unreadable or is not this driver's is left alone, at most 64 records are examined, and a snapshot that cannot be taken prunes nothing |
| `tmt channel recover` removes the record and socket | the user named this record's generation, every process it recorded (launch owner, foreground, Claude) was observed conclusively gone by exact incarnation, and the record is still exactly the one observed; a socket path that is not a socket is left and reported (see "Recovery") |

A launcher never removes a record because its owner is gone; a stale launcher has
no authority over a replacement enrollment. Stale takeover happens only in `enroll`
(replacing the same binding's record, and the prune above) and in the server's
bind. Records are therefore removed where the launcher knows the launch is over
(`withdraw`: no child was spawned, or the child was confirmed reaped) and replaced
by the next enrollment of the same binding. A launch that crashed, or whose
launcher was killed, leaves one behind. Once the foreground it recorded is gone
too it has ended, no longer protects its pane, and the next enrollment of any
binding removes it; one that never recorded a foreground stays unknown until the
user recovers it with `tmt channel recover` or the same pane relaunches (see
"Recovery" and "The baseline-paste check"). The lock file is never removed (replacing its inode would
split lock domains), and a lock that cannot be taken within 2 s fails the
operation closed. A socket path therefore belongs to the generation in the
record or is a leftover that only the next validated bind replaces. `send` is
read-only.

## Delivery mapping

**Paste is only for a session that never opted in.** The driver's `send` reads
the stored binding and the record, then classifies in this order. Every
opted-in outcome is terminal: the driver never returns `NotSent` and never falls
back, so "no byte moved" is not a reason to paste. Post-write uncertainty is
never resent or pasted either.

| Observation | Driver result | Paste |
| --- | --- | --- |
| No record for the binding (or no binding) | `Unsupported`: never opted in | yes, after the baseline-paste check |
| Provider configuration cannot be discovered | `Denied` (unverifiable) | no |
| Record names a different launch than the binding's current one, and that current launch is positively proven (its stored launch owner is observed live with a matching start identity) | `Unsupported`: the enrollment is stale for this launch, and the record is left untouched | yes, after the baseline-paste check |
| Record unreadable or not a regular file, directory not owner-only, unknown version or other binding, the binding's current launch not verifiable or ambiguous, Claude process differs from the stored runtime observation | `Denied` | no |
| Record names the binding's current launch, that launch owner is conclusively gone, and the record names a valid Claude and the runtime now observed for the binding is positively alive and is a different process (a plain relaunch outside `tmt run`) | `Unsupported`: the enrollment is stale for that runtime, and the record is left untouched | yes, after the baseline-paste check |
| Record names the binding's current launch and that launch owner is conclusively gone, otherwise (the record names no valid Claude, or the observed runtime is the one it names, is not alive, or none is observed) | `Denied(stale)`: the enrollment belongs to an ended launch; the message says to relaunch with `tmt run` | no |
| Opted in, not ready: waits up to 3 s polling the record, and it becomes ready | continues below | n/a |
| Opted in, still not ready after the wait (failed handshake, never started, or Claude at its own prompt) | `Denied(not_ready)` | no |
| Enrollment removed or its generation replaced during the wait | `Denied` | no |
| Ready, socket absent or connection refused | `Denied(unreachable)`, zero bytes moved | no |
| Ready, other connection error, or the endpoint answers `refused` | `Denied` | no |
| Connected, then any error, timeout, EOF or ambiguous answer | `Uncertain` | no, no resend |
| Endpoint answers `written` | `Completed(Unacknowledged)` | no |
| Payload above the frame bound | `Denied` (before connecting) | no |

"Paste: yes" always means "through the baseline-paste check", which can still
refuse when another enrollment is live in the pane. An enrollment applies only to
the exact launch that created it. A different
launch that is positively proven current, a plain relaunch included (through
`tmt run`, or directly while the binding still names the old launch owner),
treats an old record as non-applicable and gets the baseline delivery; nothing
removes the old record. An ended owner alone is never enough, because it does not prove that a
different launch is current: while that is unknown, ambiguous or unverifiable the
send stays terminal and nothing is pasted. A record is never read as "never
opted in" merely because its launch ended.

`Unacknowledged` is a `DeliveryAcceptance`: the message was handed to a one-way
channel and no provider receipt exists. It is never `Submitted` or `Queued` and
never claims the model saw it. A provider denial or a pending tool approval is
not observable on this transport, so neither is ever reported or inferred.
`not_ready` is reported as exactly that: readiness is unknown, and it is not
labelled as an approval that was proven pending. The ordering of Claude's own
trust and development-channel consent prompts relative to its MCP handshake was
not observed in the spike, which is why an unready session is never pasted to.

## Every delivery route

An opted-in session is never pasted to, whichever command delivers. All routes
share `delivery::send`, which prefers the driver and falls back only after
`Unsupported` or `NotSent`. A record under the binding names its driver before any
preferred harness does (the preference is written only after the launch is
admitted), so the enrollment decides the route; the preference chooses only among
sessions that never opted in. A channel directory that cannot be discovered is
unverifiable, never "no enrollment":

- `talk` to an identity name.
- Originator notifications (reply and timeout hints, from `talk`, `answer`,
  `reply` and the request observer) all go through `delivery::notify`, which has
  no paste of its own and claims each hint once. A notification that is
  unavailable or uncertain is never resent, and the durable reply stays accepted
  regardless.
- `talk` to a raw pane address. The pane's current binding is resolved through
  the existing `target::resolve`; when it names an identity the send goes through
  `delivery::send` for that identity. When it names none, `talk` pastes directly,
  behind the baseline-paste check below.

### The baseline-paste check

"No binding" is not proof that a pane never opted in: observation deletes the
binding of a pane that lost its marker (`tmt ls`, `whoami`, a name talk and every
other reconciling command do), and naming the pane again creates a new binding with
no record under its own ID. So evidence that survives those is read at the paste
itself, from the drivers' own records matched to the pane by the address each
persisted at enroll, never from the stored binding. The same check
(`delivery::pane_channel_evidence`) runs immediately before both places that can
paste: the fallback of `delivery::send` (every identity-based route, including
notifications and dispatch, however the identity was resolved), which also names the
binding it is delivering to, and the direct paste of a raw pane with no identity.

`RuntimeChannel::enrolled_in_pane(directory, pane, binding_id, deadline)` returns
`PaneEvidence` or an `EvidenceError`, asked of every registered channel. For Claude,
each `<uuid>.json` of the channel directory (never a socket, the lock or another
driver's file) is classified:

| Record | Result for this pane |
| --- | --- |
| Attributed to the pane (server incarnation, pane ID and pane process all equal), and its launch owner, foreground or Claude process is observed as exactly the recorded incarnation (PID and start identity; a stopped process counts) | enrolled: terminal, nothing is pasted |
| Attributed to the pane, a foreground or Claude was recorded, and every recorded process is conclusively gone (a reused PID is another incarnation) | ended: not evidence, the pane is ordinary again |
| Attributed to the pane, neither was recorded and the owner is gone (the launcher died before it published the foreground) | unknown: terminal for this pane only, naming the exact `tmt channel recover` command |
| Attributed to the pane, and a process of it cannot be observed | unknown: terminal for this pane; the message says to retry and names `tmt channel inspect` |
| Attributed to another pane, server incarnation or socket | not evidence, never observed |
| Unreadable, naming no pane (written before attribution) or naming another binding | skipped: blocks nothing and is named in a warning; the one exception is a record whose file is named for the binding being delivered to, which is that binding's own invalid evidence and terminal |

Only a recorded process that is itself the foreground can stand in for a published
foreground (for Claude, the Claude process, whose child the channel server is); a
helper or server process never does.

Terminal means the paste does not happen and `talk` fails with
`DELIVERY_PREPARATION_FAILED`. The message names the record and the pane. For an
unknown enrollment it names the exact recovery, `tmt channel recover --binding
<binding-id> --generation <generation>`, to be run only after confirming that no agent
of that launch still runs in the pane; for a process that cannot be observed it says
to retry and names `tmt channel inspect --binding <binding-id>`. Only a record of the
pane's own binding that cannot be read keeps a manual `rm -- '<record>' '<socket>'`,
since recovery refuses what it cannot read. Both paste sites report it identically:
`Delivery::ChannelUnavailable` carries the driver's `EvidenceError` (fault, file,
recovery) to `talk`, and `delivery::send` also returns the skipped records
(`Attempt::unattributed`) so that `talk` names them at either site with a stderr
warning (`Skipped channel record <path>: it names no pane, so it cannot protect or
block this one.`). Evidence about this pane that cannot be told (an unreadable
directory, an observation that fails, running out of the one 3 s deadline) is
terminal too, and records unrelated to the pane never block it.

The unknown state clears through `tmt channel recover` or an explicit relaunch of the
same binding in the very pane the record names (`enroll` replaces it); nothing
infers an end from the launcher, a server or an EOF. A pane whose opted-in launch
ended with its foreground confirmed (even by a crash that left its record) is an
ordinary pane again, and a new plain launch there keeps the baseline paste.

Cost: with no record in the channel directory (a user who never opted in) the check
is one directory read. With records it adds one small read per record and one
`ps -p <pid> -o lstart=` observation per process of each record attributed to the
pane; no process table is listed. There is no record-count cap; the single deadline
bounds the whole check. `enroll`'s prune takes one `ps -A -o pid=,ppid=` snapshot.

Limits: the pane's process tree and tty are not used as evidence (the tree is
rewritten when a launcher dies, and a tty is not portably observable), so the
launcher's published foreground and the persisted pane address replace them. A launch
killed between its spawn and `foreground_started` leaves an unknown record that
protects its pane until it is recovered or the same pane relaunches; records written before
attribution can never be matched and are only named.

## Recovery

`tmt channel` is the supported recovery for an enrollment that a crashed launch left
behind, for every driver with a channel. This section owns the command; each
driver's contract owns what its records name and which files it removes (for Codex,
see its "Consumer lifecycle").

- `tmt channel inspect <target> | --binding <binding-id> [--json]` is read-only. A
  target is an identity name, UUID or pane resolved through the shared resolver to
  its current binding; `--binding` takes the exact binding ID a talk error printed,
  which may no longer be current (observation deletes the binding of a pane that lost
  its marker, and the enrollment outlives it). For each driver that has a record of
  the binding it reports the record, identity, generation, the pane the record names,
  every recorded process with an exact observation (`running`, `gone` or
  `unobservable`), whether the foreground was recorded, what recovery would remove
  and keep, what the user must verify, and the exact recover command when recovery
  would proceed. It changes no file and signals no process.
- `tmt channel recover <target> | --binding <binding-id> --generation <generation>
  [--json]` names one exact enrollment by its generation. A driver removes it only
  when every process it recorded is conclusively gone (exact PID and start
  identity; a reused PID is another process), and only while, under the driver's
  lock, the record is still exactly the one it observed. A gone incarnation never
  returns, so an unchanged record keeps that observation valid.
- An enrollment whose foreground was never recorded is recovered on this explicit
  request: nothing TMT recorded can prove that agent gone, so the command shows the
  pane and launch owner to check, and the user's request after checking it is the
  proof. An enrollment whose recorded processes are all gone and whose foreground
  was recorded has ended; recovering it does what the next `enroll`'s prune would.
- Recovery never signals a process, sends, resends or pastes, never touches a pane,
  a binding or a request, and never removes a directory recursively or the lock
  file. For Claude it removes `<binding-id>.json` and the socket when that path is a
  socket; anything else at the socket path is left and reported.
- It is safe to repeat: a binding with no record succeeds with `recovered: false`.

| Outcome | Exit | JSON / code |
| --- | --- | --- |
| Inspected | 0 | `{"bindingId","enrollments":[{"driver","record","identityId","generation","state","pane","processes":[{"role","pid","start","state"}],"foregroundRecorded","verification","removes","keeps","recover"}]}`; `state` is `running`, `unverifiable`, `unconfirmed` or `ended`, `recover` is `null` unless recovery would proceed |
| Recovered | 0 | `{"bindingId","generation","driver","recovered":true,"removed","kept"}` |
| Nothing on record | 0 | `{"bindingId","generation","recovered":false}` |
| A recorded process is running | 1 | `CHANNEL_ENROLLMENT_LIVE` |
| A recorded process cannot be observed | 1 | `CHANNEL_ENROLLMENT_UNVERIFIABLE` |
| Another generation is on record, or the record changed while it was checked | 1 | `CHANNEL_ENROLLMENT_CHANGED` |
| The record cannot be read (manual removal only, as named in the message) | 1 | `CHANNEL_ENROLLMENT_INVALID` |
| A removal or the lock failed (rerunning is safe) | 1 | `CHANNEL_RECOVERY_FAILED` |
| `--binding` or `--generation` is not a UUID | 1 | `USAGE_ERROR` |

Every refusal leaves every file in place and says that nothing was removed.

## Talk behavior

`delivery::send` maps `Unacknowledged` to the request's `uncertain` wake state and
keeps waiting for the durable reply. A timed-out wait reports `TIMEOUT` and that
delivery was uncertain. It never resends and never pastes. A paste that fails
mid-transport keeps its existing `DELIVERY_UNCERTAIN` failure and stops.

When the opted-in channel cannot carry the request, `talk` fails with
`CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE` or `CHANNEL_ENROLLMENT_ENDED` (any
other terminal channel outcome uses `DELIVERY_PREPARATION_FAILED`). The message
(with the record and recovery when the evidence has them) says nothing was sent and
nothing was pasted, and the hint tells the caller to
check the session and retry later; the request stays queued in the recipient's
inbox and is inspectable with `tmt result`. Exit status is 1.

### Output additions

All additions are additive; no existing field, code, exit status or default output changes.

- Talk success JSON: `deliveryState: "uncertain"` when a channel write gave no
  receipt (present only then). `status: "sent"` keeps its meaning as the attempt's
  status, that the send attempt completed; it never states that delivery was
  confirmed. Only `status: "completed"` does, and `deliveryState: "uncertain"`
  always accompanies a `sent` attempt that gave no receipt.
- Human talk output for that attempt reads `warning: Handed request <id> to <target>
  (<pane>); delivery is unconfirmed` with a hint not to resend, instead of
  `Sent request …`.
- Talk failure JSON (`Failure` document): top-level `deliveryState: "uncertain"`
  next to `requestId` when the failure happened after such a write (for example
  `TIMEOUT`). It is set only on a request-correlated failure.
- Human `TIMEOUT` text gains a clause that delivery was uncertain, only then.
- New error codes `CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE` and `CHANNEL_ENROLLMENT_ENDED`,
  reachable only for a session whose driver registered a channel and that opted in.
- A stderr warning `Skipped channel record <path>: …` for each channel record that
  names no pane, at both paste sites, when the paste otherwise proceeds.

## Isolation

Tests and manual runs use a disposable `TMUX_TEAM_HOME`, a private tmux socket and
the existing Claude login. No global Claude, MCP or settings file is edited, and
TMT never copies credentials.
