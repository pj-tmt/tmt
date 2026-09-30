# Claude channel delivery (v1)

Status: implemented by #329 (first Claude transport). This document owns the
launch opt-in, the channel endpoint, the delivery result mapping and the
supported provider range. [ARCHITECTURE.md](../ARCHITECTURE.md) owns module
boundaries; the request lifecycle is owned by the request service and
[REQUEST-RESPONSE.md](../REQUEST-RESPONSE.md).

## Evidence base

The behavior below follows the #329 spike on Claude Code 2.1.285 (claude.ai Pro
login, negotiated MCP protocol `2025-11-25`). It established that a
`notifications/claude/channel` write reaches an idle or mid-tool session, that
Claude sends **no receipt** to the server, that a write can succeed while Claude
deliberately skips the channel, and that a server crash cannot prove that an
earlier write was not delivered. A correlated durable reply is the only proof of
processing.

## Opt-in

- Only `tmt run --channel <identity> <claude command…>` enrolls a session.
  `--channel` is a plain flag before the identity; it is rejected with `resume`,
  and for a command whose driver has no channel support, a provider outside the
  supported range, or a launch whose own process cannot be observed. Rejection
  happens before any spawn; TMT never silently launches without the channel.
- **Enrollment is durable and written first.** After binding and before the
  provider starts, `run` writes `<global>/channels/<binding-id>.json` (owner-only
  directory, atomic 0600 replacement) with a fresh per-launch generation UUID, the
  launch owner (the `tmt run` process: PID and start identity) and no Claude
  process yet. That is the state "opted in, channel not ready". A session with no
  record never opted in. `run` removes the record and socket when the launch
  ends, and a launch without `--channel` removes any earlier record of the same
  binding before it starts.
- The driver appends its own arguments after the user's argv (an explicit
  exception to argv preservation): `--mcp-config <inline JSON>` and
  `--dangerously-load-development-channels server:tmt`. It does not add
  `--strict-mcp-config`, and it writes no Claude settings or MCP configuration.
- The inline config starts the hidden `__channel-server` of the invoking `tmt`
  (`TMT_EXECUTABLE`, the absolute path of the running executable) with the
  binding ID, the generation and the absolute channel directory.
- Non-enrolled sessions, other drivers and every other launch keep their
  current behavior, including tmux paste.

## Supported provider range

Claude Code **2.1.285** only. A launch probes the command's `--version` (bounded
process owner, 5 s, 4 KiB) and refuses any other version. Widening the range
needs recorded evidence for the new version and a reviewed change to
`drivers::claude::channel::SUPPORTED_VERSIONS`. Drift is detected by contract
tests over the frozen constants below and by the opt-in developer check
`cargo run --locked -p tmt-adapters --example channel-contract -- /absolute/claude`,
which runs only `--version` and `--help` and fails when the version leaves the
range or `--help` no longer documents `--mcp-config`. Claude 2.1.285 does not
list `--dangerously-load-development-channels` in `--help`, so that preview flag
is covered only by the exact-version pin and the spike evidence. It is separate from
`runtime-contract`, whose resume pin is a different provider version.

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
(`not_ready`). On exit the server removes its socket. The record stays with the
launch. It is discovery and evidence only and grants nothing: `send` trusts it
only where it agrees with the stored binding, below.

The Unix socket is `<global>/channels/<binding-id>.sock` (0600), never a network
listener. One ingress connection carries one frame,
`{"version":1,"generation","content"}`, and receives `{"written":true}` after the
notification was flushed to Claude, or `{"refused":"<reason>"}`. The server keeps
no request-ID set: a request has at most one wake claim in the request service,
and the driver never retries.

## Delivery mapping

**Paste is only for a session that never opted in.** The driver's `send` reads
the stored binding and the record, then classifies in this order. Every
opted-in outcome is terminal: the driver never returns `NotSent` and never falls
back, so "no byte moved" is not a reason to paste. Post-write uncertainty is
never resent or pasted either.

| Observation | Driver result | Paste |
| --- | --- | --- |
| No record for the binding (or no binding) | `Unsupported`: never opted in | yes, the normal path |
| Record unreadable or not a regular file, directory not owner-only, unknown version or other binding, launch owner not verifiable, stored launch owner differs, Claude process differs from the stored runtime observation | `Denied` | no |
| Record's launch owner conclusively gone | `Denied(stale)`: the enrollment belongs to an ended launch | no |
| Opted in, not ready: waits up to 3 s polling the record, and it becomes ready | continues below | n/a |
| Opted in, still not ready after the wait (failed handshake, never started, or Claude at its own prompt) | `Denied(not_ready)` | no |
| Enrollment removed or its generation replaced during the wait | `Denied` | no |
| Ready, socket absent or connection refused | `Denied(unreachable)`, zero bytes moved | no |
| Ready, other connection error, or the endpoint answers `refused` | `Denied` | no |
| Connected, then any error, timeout or EOF without an answer | `Uncertain` | no, no resend |
| Endpoint answers `written` | `Completed(Unacknowledged)` | no |
| Payload above the frame bound | `Denied` (before connecting) | no |

A new launch supersedes an enrollment only through explicit evidence:
`tmt run --channel` writes a new generation, a launch without `--channel` removes
the binding's record before it starts, and the launch that wrote a record removes
it when it ends. A record left by a launch that died without cleaning up
therefore keeps that session's sends terminal until a new `tmt run` replaces it;
it is never read as "never opted in".

`Unacknowledged` is a new `DeliveryAcceptance`: the message was handed to a
one-way channel and no provider receipt exists. It is never `Submitted` or
`Queued` and never claims the model saw it. A provider denial or a pending tool
approval is not observable on this transport, so neither is ever reported or
inferred. `not_ready` is reported as exactly that: readiness is unknown, and it
is not labelled as an approval that was proven pending. The ordering of Claude's
own trust and development-channel consent prompts relative to its MCP handshake
was not observed in the spike, which is why an unready session is never pasted to.

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
  receipt (present only then).
- Talk failure JSON (`Failure` document): top-level `deliveryState: "uncertain"`
  next to `requestId` when the failure happened after such a write (for example
  `TIMEOUT`). It is set only on a request-correlated failure.
- Human `TIMEOUT` text gains a clause that delivery was uncertain, only then.
- New error codes `CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE` and `CHANNEL_ENROLLMENT_ENDED`,
  reachable only for a session that ran `tmt run --channel`.

## Isolation

Tests and manual runs use a disposable `TMUX_TEAM_HOME`, a private tmux socket and
the existing Claude login. No global Claude, MCP or settings file is edited, and
TMT never copies credentials.
