# Claude channel delivery (v1)

This document owns the launch opt-in, the channel endpoint, the enrollment
lifecycle, the delivery result mapping and the supported provider range.
[ARCHITECTURE.md](../ARCHITECTURE.md) owns module boundaries; the request
lifecycle is owned by the request service and
[REQUEST-RESPONSE.md](../REQUEST-RESPONSE.md).

## Status

Tracking: #329, delivered as four stacked changes (#712 to #715).

- **Shipped:** the outcome vocabulary and reporting (below), the `RuntimeChannel`
  lease port with its registry slot, and the Claude send classification
  (`ClaudeRuntime::send`, "Delivery mapping"). Nothing writes an enrollment yet, so
  no session can opt in and every Claude session still gets the baseline delivery.
- **Not shipped yet:** the enrollment and the stdio server (#714), `tmt run
  --channel`, notification and raw-pane routing and the E2E scenarios (#715).

Every section other than Status and the two shipped items describes the target
contract of the unshipped changes, not current behavior. Each change updates this
status in the same pull request.

## Evidence base

The behavior below follows the #329 spike on Claude Code 2.1.285 (claude.ai Pro
login, negotiated MCP protocol `2025-11-25`). It established that a
`notifications/claude/channel` write reaches an idle or mid-tool session, that
Claude sends **no receipt** to the server, that a write can succeed while Claude
deliberately skips the channel, and that a server crash cannot prove that an
earlier write was not delivered. A correlated durable reply is the only proof of
processing.

## Opt-in and the launch lease

- Only `tmt run --channel <identity> <claude command…>` enrolls a session.
  `--channel` is a plain flag before the identity; it is rejected with `resume`,
  and for a command whose driver has no channel, a provider outside the supported
  range, a command line the driver cannot plan around, or a launch whose own
  process cannot be observed. Rejection happens before any spawn; TMT never
  silently launches without the channel.
- The driver owns the launch plan through `runtime::channel::RuntimeChannel`.
  `enroll` receives the user's command (selected executable and argv, resume
  substitution included), the launch directory, the binding, the `tmt run`
  process as launch owner and the channel directory, and returns a lease
  (`ChannelEnrollment`): the foreground command to spawn verbatim, environment
  for the provider child, and a consuming `withdraw`. The CLI never parses or
  rewrites provider arguments. The environment reaches the provider child, and
  therefore the subprocesses the provider itself starts (such as its MCP server);
  it is never written to the ambient or global environment, to persisted state or
  to logs. The launcher holds the lease for the child's whole lifetime and
  withdraws it on every path: spawn failure, normal exit, interrupted wait and
  early return.
- Claude's plan is the user's command with `--mcp-config <inline JSON>` and
  `--dangerously-load-development-channels server:tmt` appended. It adds no
  `--strict-mcp-config` and writes no Claude settings or MCP configuration. The
  inline config starts the hidden `__channel-server` of the invoking `tmt` (the
  absolute path of the running executable) with the binding ID, the generation
  and the absolute channel directory.
- **Enrollment is durable and written first.** After binding and before the
  provider starts, the driver writes `<global>/channels/<binding-id>.json` (owner-only
  directory, atomic 0600 replacement) with a fresh per-launch generation, the
  launch owner (PID and start identity) and no Claude process yet. That is the
  state "opted in, channel not ready". A session with no record never opted in.
- A launch without `--channel` touches no channel state, and no launcher ever
  removes another launch's enrollment.
- Non-enrolled sessions, other drivers and every other launch keep their current
  behavior, including tmux paste.

## Supported provider range

Claude Code **2.1.285** only. A launch probes the command's `--version` (bounded
process owner, 5 s, 4 KiB) and refuses any other version. Widening the range
needs recorded evidence for the new version and a reviewed change to
`drivers::claude::channel::SUPPORTED_VERSIONS`. Drift is detected by contract
tests over the frozen constants and by the opt-in developer check
`cargo run --locked -p tmt-adapters --example channel-contract -- /absolute/claude`,
which runs only `--version` and `--help` and fails when the version leaves the
range or `--help` no longer documents `--mcp-config`. Claude 2.1.285 does not
list `--dangerously-load-development-channels` in `--help`, so that preview flag
is covered only by the exact-version pin and the spike evidence. It is separate
from `runtime-contract`, whose resume pin is a different provider version.

## Channel endpoint and enrollment record

`__channel-server` is a stdio MCP server for one Claude process. It exposes no
tools, declares `capabilities.experimental["claude/channel"] = {}` and writes
one-way `notifications/claude/channel` messages with `params.content` set to the
prepared request payload. Replies use the existing `tmt reply` receipt path, not
a channel tool. It cannot create an enrollment: it refuses to start unless the
launch's record exists with its own generation and no Claude process yet.

The record is
`{"version":1,"bindingId","generation","launchOwner":{"pid","start"},"claude":null|{"pid","start"}}`.
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
and the driver never retries.

## Enrollment ownership and serialization

A launch is identified by its generation (random per `enroll`) together with its
launch owner. One lock file, `<global>/channels/.lock`, taken through the existing
`file_lock::exclusive` with a bounded retry that fails closed, is held across
every mutation of the record or the socket, so no check-then-change race exists:

| Mutation | Under the lock, it proceeds only if |
| --- | --- |
| `enroll` writes the record | always: the newest launch of a binding replaces any earlier record |
| Server publishes readiness (`claude`) | the record still carries the server's generation and launch owner |
| Server binds the socket (and replaces an unreachable one) | the record carries the server's generation; a live second server is refused |
| Server removes its socket on exit | the record still carries the server's generation |
| Lease `withdraw` removes the record and socket | the record carries exactly the lease's generation and launch owner; otherwise it removes nothing |

A launcher never removes a record because its owner is gone; a stale launcher has
no authority over a replacement enrollment. Stale takeover happens only in `enroll`
and in the server's bind. A socket path therefore belongs to the generation in the
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
| No record for the binding (or no binding) | `Unsupported`: never opted in | yes, the normal path |
| Provider configuration cannot be discovered | `Denied` (unverifiable) | no |
| Record names a different launch than the binding's current one, and that current launch is positively proven (its stored launch owner is observed live with a matching start identity) | `Unsupported`: the enrollment is stale for this launch, and the record is left untouched | yes, the normal path |
| Record unreadable or not a regular file, directory not owner-only, unknown version or other binding, the binding's current launch not verifiable or ambiguous, Claude process differs from the stored runtime observation | `Denied` | no |
| Record names the binding's current launch, that launch owner is conclusively gone, and the runtime now observed for the binding is positively alive and is not the Claude the record names (a plain relaunch outside `tmt run`) | `Unsupported`: the enrollment is stale for that runtime, and the record is left untouched | yes, the normal path |
| Record names the binding's current launch and that launch owner is conclusively gone, otherwise (the observed runtime is the one the record names, is not alive, or none is observed) | `Denied(stale)`: the enrollment belongs to an ended launch; the message says to relaunch with `tmt run` | no |
| Opted in, not ready: waits up to 3 s polling the record, and it becomes ready | continues below | n/a |
| Opted in, still not ready after the wait (failed handshake, never started, or Claude at its own prompt) | `Denied(not_ready)` | no |
| Enrollment removed or its generation replaced during the wait | `Denied` | no |
| Ready, socket absent or connection refused | `Denied(unreachable)`, zero bytes moved | no |
| Ready, other connection error, or the endpoint answers `refused` | `Denied` | no |
| Connected, then any error, timeout, EOF or ambiguous answer | `Uncertain` | no, no resend |
| Endpoint answers `written` | `Completed(Unacknowledged)` | no |
| Payload above the frame bound | `Denied` (before connecting) | no |

An enrollment applies only to the exact launch that created it. A different
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
`Unsupported` or `NotSent`:

- `talk` to an identity name.
- Originator notifications (reply and timeout hints, from `talk`, `answer`,
  `reply` and the request observer) all go through `delivery::notify`, which has
  no paste of its own and claims each hint once. A notification that is
  unavailable or uncertain is never resent, and the durable reply stays accepted
  regardless.
- `talk` to a raw pane address. The pane's current binding is resolved through
  the existing `target::resolve`; when it names an identity the send goes through
  `delivery::send` for that identity, after re-verifying that the stored binding
  still names that pane. No bound identity is not proof that the pane never opted
  in (a non-active binding resolves to none as well): existing channel evidence for
  the pane blocks the baseline paste, and ambiguous or unverifiable ownership or
  enrollment fails terminally. Only a pane with no channel evidence keeps today's
  paste.

## Talk behavior

`delivery::send` maps `Unacknowledged` to the request's `uncertain` wake state and
keeps waiting for the durable reply. A timed-out wait reports `TIMEOUT` and that
delivery was uncertain. It never resends and never pastes. A paste that fails
mid-transport keeps its existing `DELIVERY_UNCERTAIN` failure and stops.

When the opted-in channel cannot carry the request, `talk` fails with
`CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE` or `CHANNEL_ENROLLMENT_ENDED` (any
other terminal channel outcome uses `DELIVERY_PREPARATION_FAILED`). The message
says nothing was sent and nothing was pasted, and the hint tells the caller to
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

## Isolation

Tests and manual runs use a disposable `TMUX_TEAM_HOME`, a private tmux socket and
the existing Claude login. No global Claude, MCP or settings file is edited, and
TMT never copies credentials.
