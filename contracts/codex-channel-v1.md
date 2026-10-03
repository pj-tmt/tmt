# Codex native channel contract

Status: Codex launches and exact `tmt resume` use plain delivery by default.
`--channel` opts into the native channel and requires enrollment; `--no-channel`
explicitly chooses a plain launch. Fresh channel launches let the remote TUI
create its own thread; they never resume a supervisor-created zero-turn thread.
The shared launcher policy and Codex-specific boundaries are below.
Queue acceptance is a delivery receipt; durable request completion remains separate. The pinned 0.159.3
attachment/active-turn proof and 0.160.0 attach/queue/durable-reply proof are
accepted; product routing and lifecycle gates are independent evidence described
below.

The [public dispatch readiness and input-safety contract](extension-api.md#dispatch-readiness-and-input-safety)
owns the extension-facing admission/wake boundary. Native readiness and queue acceptance here
do not grant an input-readiness lease or prove request completion.

## Default launch policy

The [shared launcher policy](claude-channel-v1.md#opt-in-and-the-launch-lease)
selects one mode for both `tmt run` (including `--resume`) and
`tmt resume`: Default when neither flag is present, Disabled for `--no-channel`,
or Required for `--channel`. The flags conflict in clap, before launch. Each
driver advertises only whether its channel is enabled by default through the
shared channel port. Codex and Claude advertise disabled and remain opt-in.
The CLI
contains no provider-name test for this policy. Changing a driver's advertised
default and its contract suffices to change its default behavior.

Default attempts enrollment only for a driver that advertises it. Disabled uses
the original plain command without enrollment. Required attempts enrollment
regardless of the advertised default and fails if unavailable. The run and
resume CommandSpec examples include both flags so the grammar audit covers them.

For Default, any reason enrollment cannot happen before channel foreground
startup, including an unqualified build, a preflight advisory, no supported
app-server, unsupported arguments or a failed exact resume, selects the original
plain command and prints exactly one visible notice line saying that the session
uses paste delivery and naming the reason:
`warning: tmt: <name> uses paste delivery: <reason>`. Codex's preflight classifies
its unqualified-build advisory as unavailable for enrollment, even if a later handshake might otherwise succeed. Required
reports the same reasons as errors with stable codes: `CHANNEL_UNSUPPORTED` for
an absent channel port, `CHANNEL_PROVIDER_UNSUPPORTED` for a refused build or
qualification advisory, and `CHANNEL_UNAVAILABLE` for other enrollment failures.

Plain fallback after an enrollment attempt requires confirmed owned-endpoint
cleanup and the existing pane-enrollment evidence to establish that no live or
unconfirmed enrollment remains. Unknown, unreadable or retained enrollment
evidence stays terminal. The launcher must still hold its binding admission
authority; fallback never bypasses a binding conflict or performs recovery.
No command is launched if these prerequisites cannot be established. Once
enrolled, the no-paste rule is unchanged: not-ready, unreachable, refused and
uncertain delivery remain terminal, with no resend or paste fallback.

Exact resume carries the launcher's selected, typed provider session through
the shared channel plan and private supervisor startup request. The Codex driver
uses `thread/resume` on its owned endpoint and validates that the returned thread
UUID equals that session before attaching the foreground. It never substitutes
a new thread or guesses a remembered session from arbitrary user argv. A Default
resume that cannot enroll before startup may run only the original exact-resume
command, with the visible paste notice and the same cleanup/evidence guards;
it never silently downgrades an enrolled thread. Disabled explicitly chooses
plain exact resume without enrollment; retained enrollment evidence still blocks
paste delivery into that pane. Required fails instead of launching plainly.

An enrolled Codex launch owns one extra app-server process, plus its existing
supervisor, for that foreground's lifetime. Normal exit and Ctrl-C use the same
confirmed-child cleanup path. Codex's folder-trust prompt remains user-owned:
the channel neither answers it nor changes trust configuration, and queued input
may wait until the user completes attachment. Plain `codex` and global provider
config, authentication and hooks are unchanged.

Acceptance covers plain default launch without enrollment, explicit opt-in,
explicit opt-out and its positive paste control, generic advertised-default
unavailable/advisory fallback with one reason line, strict stable
errors, terminal enrolled-but-not-ready routing, exact opt-in resume without
thread substitution, cleanup on exit and Ctrl-C, and a test that toggles a
driver's advertised default without a CLI provider-name change. Native fixture
and product-routing evidence remain separate from live provider qualification.

## Delivery receipt

One delivery creates one immutable `thread/queue/add` request for the enrolled
thread. A correlated response with the exact caller ID and input echo records
queue acceptance, not processing or completion. Only a durable `tmt reply`
completes the TMT request. Caller IDs are not provider deduplication keys.

A correlated error alone cannot prove absence of queue side effects. The adapter
recognizes only these exact, thread-specific pre-enqueue rejection signatures:

- Code -32600: ``session THREAD is archived. Run `codex unarchive THREAD` to unarchive it first.``
- Code -32603: `failed to read thread: invalid thread-store request: no rollout found for thread id THREAD`

All other errors, including generic correlated internal errors, remain uncertain.
Missing, malformed, mismatched or ambiguous receipts also remain uncertain.
Neither refusal nor uncertainty authorizes resend or paste fallback. A proven
pre-write transport refusal remains distinct from a provider rejection received
after writing a request.

The source boundary is OpenAI Codex revision
[33aea33b39a5e35c78de75c332106479b16c6994](https://github.com/openai/codex/blob/33aea33b39a5e35c78de75c332106479b16c6994/codex-rs/app-server/src/request_processors/thread_queue_processor.rs):
`add` calls `require_thread` before `enqueue`; archived and failed thread reads
return from that prerequisite. Conversion to an API submission occurs after
`enqueue` and can itself fail. These are source observations, not a claim that
every internal error is pre-enqueue. Exact archived/deleted responses were also
observed in the isolated 0.159.2 spike. Retained evidence contains selected native
events; full raw logs were removed. The source revision and retained runtime
observations are distinct provenance, not a complete event stream or proof of
all future provider versions. Message drift fails closed to uncertainty.

## Transport and qualification

The Codex adapter alone uses synchronous tungstenite 0.30.0 with default features
disabled and only `handshake` enabled. No TLS, async runtime or handwritten
WebSocket/SHA1 implementation is added. Core and shared driver ports do not
reference this dependency.

The enrollment owner establishes ownership of the exact endpoint
process and capability before constructing a client. A loopback address or an
arbitrary endpoint's self-report is insufficient authority. The client accepts
only explicit loopback IPv4/nonzero ports and a bounded capability token, sent
in the HTTP Authorization header. It initializes that owned endpoint and reads
the leading provider build version from `userAgent`, not the trailing client
version. The bounded supported set is 0.159.2, 0.159.3 and 0.160.0; other builds
fail closed. Binary preflight requires parseable `codex-cli major.minor.patch`
and accepts only those qualified builds. Older, malformed or other-line/build
output is refused; 0.159 patches above 3 return an unavailable unqualified-build
advisory. This does not qualify them: the owned initialize
handshake remains authoritative and accepts only the exact supported builds.
Untested 0.160.x patches are refused at both boundaries.

| Provider build                              | Binary preflight                        | Owned initialize | Live qualification                                                                                         |
| ------------------------------------------- | --------------------------------------- | ---------------- | ---------------------------------------------------------------------------------------------------------- |
| 0.159.2                                     | Accepted                                | Accepted         | Native fresh-thread binding and completion proof in #1198; earlier queue observations in #329.             |
| 0.159.3                                     | Accepted                                | Accepted         | Native fresh-thread binding and completion proof in #1198; active-turn continuity in #739.                 |
| 0.160.0                                     | Accepted                                | Accepted         | Fresh-thread binding proof in #1198; active-turn attachment, queue correlation and durable reply in #1043. |
| Later 0.159 patches                         | Unavailable: unqualified-build advisory | Refused          | Unqualified.                                                                                               |
| Other builds, including later 0.160 patches | Refused                                 | Refused          | Unqualified.                                                                                               |

The initialize format is source-backed at the pinned revision above, in
`request_processors/initialize_processor.rs` and
`login/src/auth/default_client.rs`. The accepted 0.159.3 attachment proof qualifies the remote foreground behavior
for that build. The accepted 0.160.0 proof separately qualifies that build; the
allowlist alone is not runtime evidence for future versions.

Native delivery has two bounded stages on the same client connection. Preparation
has one three-second absolute deadline for process qualification, connect,
handshake/initialize and the final enrollment/process recheck. Expired or
unverifiable preparation sends no queue frame. After a successful final recheck,
the consuming queue attempt receives one fresh three-second absolute delivery
deadline for its write and receipt. These stages can together consume six seconds;
process inspection cannot spend the delivery receipt budget. The delivery deadline
is set once on the client and stream, then recalculated before every underlying
read and write, including library-internal fragment reads; it is never renewed by
fragments or events. Enrollment-only calls retain their caller's absolute deadline.
Input is at most 64 KiB, response/message at most 1 MiB, frame at most 256 KiB,
and at most 256 interleaved events are inspected per call. Only notifications
may be skipped; malformed envelopes, requests and wrong response IDs terminate
the attempt. Delivery consumes the initialized client. There is no reconnect,
retry worker or resend after any outcome.

Tests use owned loopback peers and synthetic frames; they prove transport
classification, bounds and absence of a second write/connection. They do not
prove provider processing, same-live-turn attachment or production ownership.
The consumer tests and live-provider evidence below cover separate properties.
Release-size evidence measures the fully linked CLI against the matching baseline;
groundwork alone may be removed from an unused binary by the linker.

## Private enrollment records

`drivers/codex/record` stores one bounded, private per-binding opt-in record under
the caller-supplied channel directory's `codex` subdirectory. It is not an identity registry and
performs no binding write. Version, binding UUID, generation UUID and exact
launch-owner PID/start identity are mandatory. Fresh startup additionally records the owned endpoint incarnation, loopback
port and canonical cwd before any thread exists. Discovery fills its immutable
thread candidate; this remains unready until launcher admission. Readiness
carries that same endpoint and exact provider thread UUID. Tokens are absent from this record. Record input is capped at 8 KiB;
nonregular, symlink, public, foreign-owned, malformed and oversized records fail
closed. An absent record alone means no record, not a synthesized ready state.

Creation records opt-in before readiness. Every mutation uses the existing
stable per-binding `file_lock`; the lock inode is never unlinked. Ready publication
and withdrawal compare generation plus the whole expected launch incarnation,
including PID and start identity, while holding that lock. Withdrawal leaves a
different replacement unchanged even when its owner has exited.

A new explicit channel launch must first validate its claimed binding/session
snapshot and its own live incarnation through the existing launcher admission
authority. This caller guard is a prerequisite, not implemented by this record
module and not permission to write a second binding registry. Under the record
lock, creation rechecks that new owner is alive, and admits takeover only when
every prior recorded process is conclusively gone. A known foreground is
compared by PID and start identity. An Unknown foreground permits takeover only
for an explicit relaunch of the same binding with the same persisted pane
address; a historical ready server must also be conclusively gone. Any live or
unverifiable recorded process, unreadable record, or unverifiable new owner is
terminal and leaves prior state untouched. Old-owner absence alone never grants new authority.
The launcher/lease and product tests exercise these boundaries separately; record
unit tests alone do not prove full crash recovery or former endpoint cleanup.

Tests cover opt-in/ready transitions, serialized takeover, unchanged records
on denied authority and stale withdrawal preserving changed generation, owner
start and owner PID independently. Endpoint/client lifetime and final routing
remain separate concerns; no channel is registered by this module.

## Owned endpoint and foreground attachment

`drivers/codex/server` launches the selected executable's app-server in its own
process group, using an ephemeral IPv4 loopback listener. Startup reads the
provider's announced bound address from a private log, with a 64 KiB scan bound
and the caller's deadline. It does not reserve/release a port or discover a shared
daemon. A fresh per-launch capability is written only to the owned generation
directory (0700) and file (0600), supplied through `--ws-token-file`. The caller
passes the capability to the foreground through the named child-only environment
variable, not an argv token. This module does not copy login credentials or edit
provider configuration.

The resource owner sends SIGKILL only to its directly owned, unreaped process
group, then waits up to two seconds before deleting created files. Startup failure,
explicit stop and drop use the same ordering. Unverified process cleanup reports
failure and retains capability/diagnostic files; it is not successful cleanup.
File cleanup compares created regular-file and directory device/inode identities,
never recursively removes a directory, and preserves replacements. This is
in-process cleanup; the supervisor described below owns launcher crash handling,
which is not a guarantee provided by destructors.

`drivers/codex/attachment` accepts a bounded option surface before launch. It
resolves relative `-C` against the original working directory once, emits the
absolute directory for the foreground, and exposes that same directory to
server spawn and thread metadata validation. It rejects initial prompts, user-supplied resume/fork
subcommands and implicit/default remote selection; only the launcher-selected
typed exact resume uses its bounded generated grammar. Exact resume uses
`resume --remote ... --remote-auth-token-env TMT_CODEX_ENDPOINT_TOKEN` with the
exact supplied provider thread. Fresh startup uses `--remote` with the same
capability environment and no resume subcommand or supplied thread. Planning checks endpoint shape; ownership comes
from the enrollment resource, not from a user-supplied endpoint string.

If Codex asks to trust the launch folder, the user must answer in its TUI before
attachment can proceed; the channel never answers that prompt or changes trust
configuration, and accepted queue input may wait for attachment.

Fresh permission options go to the TUI that creates the new thread and to the
app-server defaults. `-s`/`--sandbox` accepts `read-only`, `workspace-write` or
`danger-full-access`; `-a`/`--ask-for-approval` accepts `untrusted`, `on-request`
or `never`. Exact remote resume never receives permission overrides. Absent
flags preserve provider defaults. Unsupported values are refused before spawn.

Generic `-c`/`--config` permission roots (`approval_policy`, `approvals_reviewer`,
`sandbox_mode`, `default_permissions`, `permissions`, `network`, and
`sandbox_workspace_write`) are refused; use the supported typed flags where
applicable. Other bare dotted config keys retain their server and foreground
routing. Quoted or ambiguous config keys are refused rather than allowing a
permission override to evade classification. Refusal is `UnsupportedArguments`
in enrollment, before provider/supervisor spawn or enrollment record creation;
the launcher's binding may already exist. No argument is silently discarded.

This mapping follows Codex 0.159.3 commit
[01fc69f4026735edfdf6789820549727a4867b11](https://github.com/openai/codex/blob/01fc69f4026735edfdf6789820549727a4867b11/codex-rs/app-server/src/request_processors/thread_processor.rs#L1670):
the thread-start processor maps typed sandbox and approval fields into its
configuration. The TUI's `app/config_persistence.rs` permission detector and
`app/startup.rs` remote-resume check reject foreground permission overrides.

Tests use owned shell stand-ins and temporary files to observe cwd, process exit,
startup timeout, private capability and replacement-preserving cleanup. They do
not invoke a model or prove that a real foreground client preserves an active
provider turn. The live proof below establishes provider continuity separately from the real
CLI tests for thread admission, foreground identity and terminal routing.

## Consumer lifecycle

A fresh channel's first app-server lifecycle hook may establish remembered
session/model without an existing remembered preference. Its
`LifecycleObservation::verified_binding` locator comes only from the parsed
private enrollment record, with the matching generation, exact ready thread
and known foreground. The shared hook caller uses this locator only to select a
read-only stored binding snapshot. All existing active host, exact app-server,
live owner/foreground/thread and transactional checks remain required before
context is returned or preferences change. A missing, stale, mismatched or
unconfirmed record grants no locator authority. The hook still owns session and
reported model persistence; admission never invents remembered history or model.
Ordinary shared hooks and prompt/turn hooks keep their exact remembered-session
lookup. A guard confines the locator to this driver and binding selection to
the shared `verified_caller` owner.

The provider record persists the claimed identity and pane address before spawn:
host/server UUID, socket path, server PID/start, pane ID and pane PID. The launcher
supplies this evidence from its existing binding, without another binding write.
Missing or mismatched attribution refuses enrollment. Older/unreadable records
without attributable pane evidence are named in diagnostics; they cannot block
unrelated panes. Corruption of the current binding's own record remains terminal.

Foreground state starts Unknown. The launcher's single `foreground_started`
callback records only the original owned child's observed incarnation. For a
fresh launch it also discovers the TUI-created thread on the owned endpoint:
the initial loaded list must be empty, then the one-time pre-first-turn gate
requires exactly one loaded thread and `thread/read` metadata with the same ID,
canonical cwd and explicit `ephemeral: false`. A missing flag is not false.
Discovery performs no model turn, `thread/start`, resume, subscription or queue.
A timed-out early hook degrades only its own activity/model update; it never
authorizes a different session and never causes paste or resend. No owner-side
failure latch, spool or replay is added: later authoritative admission may
publish Ready. A subsequent same-session starting hook can record the session
and model through the unchanged verification path; a different session cannot.
If the first starting hook was lost, remembered model/activity remain unknown
until a verified starting hook supplies them. Prompt/Stop do not create missing
remembered preferences, and thread/read model metadata is not persisted as hook
history.
The eager first-hook gate is read-only and generation-scoped. Claude, plain
hooks, unrelated generations, known auxiliary sessions and already-Ready records
return immediately without sleeping or probing. Before the candidate is known,
a matching fresh generation may wait; after discovery, only its exact session
may wait. The driver caps this wait at one second within the hook owner's work
deadline. Discovery itself has a separate five-second foreground startup bound;
TUI creation precedes SessionStart and does not consume its admission gate.

TMT installs a three-second command-hook timeout (`HOOK_TIMEOUT_SECONDS`). Codex
0.159.2, 0.159.3 and 0.160.0 honor that explicit timeout and include stdin writes
and process completion in it: see their pinned command runners
([0.159.2](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/hooks/src/engine/command_runner.rs#L276),
[0.159.3](https://github.com/openai/codex/blob/01fc69f4026735edfdf6789820549727a4867b11/codex-rs/hooks/src/engine/command_runner.rs#L276),
[0.160.0](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/hooks/src/engine/command_runner.rs#L276)).
The Codex channel hook's total bound is the installed timeout minus a named
500 ms margin: currently 2.5 seconds, including the process owner's one-second
cleanup reserve. Input, gate and worker share the remaining 1.5-second absolute
work deadline. The gate never restarts that deadline. The private worker receives
only the remaining milliseconds, capped at its existing two-second maximum;
payload bytes remain unchanged. The parent's monotonic deadline remains
authoritative. Claude and plain hooks retain the existing two-second worker
protocol. No provider timeout migration or global settings change is required.

After binding, the thread is immutable: never recheck loaded-list uniqueness.
Auxiliary `ephemeral: true` threads are excluded from binding, activity and
routing by their distinct session IDs and the non-ephemeral binding gate;
notification counts never establish authority.

The launcher's default no-op `ChannelEnrollment::foreground_admitted` callback
runs once after a committed Running admission of the original observed child,
with storage closed. Codex then rechecks its generation, candidate, original
foreground, owner and endpoint under the record lock before publishing Ready.
Failure warns, leaves the admitted child and unready evidence intact, and never
undoes admission, pastes or silently downgrades. Claude and plain launches take
the no-op path. Cleaning an endpoint never proves an Unknown foreground ended.
Known foreground, owner and any recorded endpoint must all be conclusively gone
before an ended record is pruned; read-only delivery never prunes.

`drivers/codex/supervisor` owns the original app-server child and its process
group. A private, close-on-exec launcher socket controls its lifetime; neither
the provider nor the attached foreground inherits the launcher endpoint. Launcher
EOF, including SIGKILL, reaps the server and removes only its owned capability
files while retaining enrollment. Explicit withdrawal is separate: it may retire
the exact record only after no child was spawned or the same foreground child
was confirmed reaped. Wait errors, panic and early return do not supply that
proof. The supervisor never signals a later observed PID. Before a usable Ready handoff, initialization or publication failure explicitly retires the no-child enrollment after confirmed server cleanup. A failed control write is not retirement acknowledgement; cleanup uncertainty preserves evidence. After a complete Ready frame is written, a flush failure cannot prove that no foreground exists and retains enrollment.

Supervisor failure itself is a limitation: killing the supervisor can prevent
its owned-child cleanup. Timeout or unconfirmed cleanup retains evidence and
reports failure, rather than claiming that the endpoint or foreground ended.

Recovery uses `tmt channel`, which the
[Claude channel contract](claude-channel-v1.md#recovery) owns. Pane diagnostics
for an unknown or unobservable Codex enrollment name the pane, the foreground state
and the exact `tmt channel recover --binding <binding-id> --generation
<generation>`, to be run only after verifying that the named pane's original
foreground, the launch owner and the owned app-server are gone. Under the record's
per-binding lock and only while the record is exactly the one observed, with every
recorded process (launch owner, known foreground, ready or pending app-server) conclusively
gone, recovery removes `codex/<binding-id>.json`. It cleans the generation
directory only when the app-server was recorded and is gone: it removes the
launch's own `capability` and `server.log` when each is a regular file of this
user, then the directory with a non-recursive `rmdir`; any other entry, a link,
or the directory itself when it is not empty is left and reported. If one of those
known files cannot be removed, the record stays and recovery reports
`CHANNEL_RECOVERY_FAILED`, so running the same command again can finish it. When no
app-server was ever recorded, nothing proves it gone, so the directory is left and
reported. `tmt channel` refuses an unreadable record and names a manual
`rm -- '<record>'` with the path quoted, including embedded apostrophes. Recovery
is separate from delivery and never signals, sends or pastes.

The native send path requires the exact record, launch owner, foreground,
endpoint and thread, qualifies the owned endpoint once, and consumes one queue
attempt. Channel-originated hooks preserve the admitted foreground incarnation;
ordinary non-channel resume remains separate. Mock supervisor tests exercise
startup failure, launcher SIGKILL with a surviving foreground, the publication
window and explicit confirmed withdrawal. The registered driver participates in the shared `enrolled_harness` route selection
and `enrolled_in_pane` guards; it does not create a second paste policy. This
includes originator reply notifications, identity sends and identity-less panes.

`RuntimeLifecycle::activity_process` maps only activity attribution and never
rewrites a binding. Its default returns the observed incarnation without reads
or probes, retaining Claude/plain hook behavior. Codex channel prompt/Stop hooks
execute in the app-server while the binding retains the original foreground.
The driver verifies the exact private generation, Ready session, observed
server, recorded/current foreground, launch owner and their live incarnations
under the existing hook deadline before returning that foreground. Missing,
malformed, replaced, unready or auxiliary proof yields no activity/context write.
The mapping runs before `turn_state`, so usage scans receive the actual remainder.
It applies equally to fresh and exact-resume channel launches; the source-confirmed
alpha.41 server/foreground equality gap is recorded on #1198.

Channel lifecycle/activity hooks are expected to execute in the owned app-server.
A channel locator on an independently classified foreground hook does not grant
that server proof and produces no channel activity/context update. Plain hooks
without a channel locator retain the observed process through the default path.
UserPromptSubmit emits context only for incoming attention or extension
contributions; a verified prompt with neither produces empty stdout.

A verified main-thread Stop hook means common Idle activity while the runtime
remains Running. A queue receipt means queued only; only a durable `tmt reply`
settles a TMT request. A timeout never cancels, resends, pastes or infers
completion. TMT does not add an observer thread subscription or use an
unsubscribed observer's missing `turn/completed` as completion evidence.

The provider can perform ancillary inference after the main turn, including an
ephemeral `thread_title` thread on a different model. This spends provider
budget despite a single requested main response; qualification records those
logical model requests separately and does not claim an HTTP-attempt count.

## Verification boundaries

The accepted [0.159.3 proof](https://github.com/wkh237/tmt/issues/739#issuecomment-5925294420)
used a fresh 128-bit nonce rendered by the foreground while the native same-thread,
same-turn synthetic barrier remained held, plus two exact owned endpoint
connections. One barrier release was followed by successful native completion
of that turn and a separately rendered assistant reply. Cleanup and shared-file
hash preservation were independently checked. Retained native events are selected
evidence, not a complete stream. No product-router claim is inferred from this
provider proof.

The accepted [0.160.0 proof](https://github.com/pj-tmt/tmt/issues/1043#issuecomment-5953191549)
qualifies foreground attachment to the owned thread during an active-turn
barrier, idle/busy queue request/caller/input correlation, native processing and
a nonce-correlated durable TMT reply. It used isolated HOME/CODEX_HOME, guarded
read-only authentication, private tmux and an owned loopback app-server, with
shared-file hash preservation and independent cleanup verification. It did not
exercise fresh zero-turn attachment, which fails on 0.160.0 (#1198). Post-idle
foreground rendering was not retained and is not qualified by this evidence.
The issue records the runner's output-oracle limitation; no production-router
coverage is inferred from this provider proof.

`codex-channel.e2e.test.ts` uses the real CLI, shared delivery path, private tmux
and the deterministic `codex-channel-fixture` peer. Native queue frames, durable
request/response rows and per-pane tmux write traces are separate oracles. A
never-enrolled plain session is the positive paste control. The fixture is an
E2E-only Rust example using the existing WebSocket library, never a provider,
model, release artifact or production fallback. These scenarios complement the
native provider-local ownership tests; neither replaces the live attachment
proof or the shared launcher's wait-error tests.
The opt-in first-hook scenario invokes the real lifecycle command from its
fixture-owned app-server, asserts remembered session/model and preserved
foreground incarnation, and verifies queued delivery without paste plus owned
app-server cleanup. It uses no SQL preference seeding or provider credentials.

The [#1198 fresh-bootstrap evidence](https://github.com/pj-tmt/tmt/issues/1198#issuecomment-5965074559)
qualifies the one-time binding gate on 0.159.2, 0.159.3 and 0.160.0. The older
build runs also captured native Stop/TUI completion and auxiliary title threads.
This is native provider evidence, separate from TMT's hook-to-Idle product path
and durable request completion.
