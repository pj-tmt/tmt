# Codex native channel contract

Status: unregistered Codex groundwork under #719/#329. The #785 foundations
extend #736–#738 with pane/foreground record state, exact takeover/prune/withdraw,
startup cleanup certainty and permission planning. Lease composition, consumer
routing and user-facing registration remain later slices. This slice neither
registers a channel nor invokes a native delivery path.

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

The future enrollment owner must establish ownership of the exact endpoint
process and capability before constructing a client. A loopback address or an
arbitrary endpoint's self-report is insufficient authority. The client accepts
only explicit loopback IPv4/nonzero ports and a bounded capability token, sent
in the HTTP Authorization header. It initializes that owned endpoint and reads
the leading provider build version from `userAgent`, not the trailing client
version. The bounded supported set is 0.159.2 and 0.159.3; other builds fail closed.
The initialize format is source-backed at the pinned revision above, in
`request_processors/initialize_processor.rs` and
`login/src/auth/default_client.rs`. Patch compatibility still requires the final
slice's real-provider verification; the allowlist alone is not runtime evidence.

One client connection has one absolute deadline, recalculated before every
underlying read and write, including library-internal handshake/fragment reads.
Input is at most 64 KiB, response/message at most 1 MiB, frame at most 256 KiB,
and at most 256 interleaved events are inspected per call. Only notifications
may be skipped; malformed envelopes, requests and wrong response IDs terminate
the attempt. Delivery consumes the initialized client. There is no reconnect,
retry worker or resend after any outcome.

Tests use owned loopback peers and synthetic frames; they prove transport
classification, bounds and absence of a second write/connection. They do not
prove provider processing, same-live-turn attachment or production ownership.
The final consumer slice must prove those properties and remeasure the fully
linked CLI; groundwork can be removed from an unused binary by the linker.

## Private enrollment records

`drivers/codex/record` stores one bounded, private per-binding opt-in record under
the caller-supplied channel directory's `codex` subdirectory. It is not an identity registry and
performs no binding write. Version, binding UUID, generation UUID and exact
launch-owner PID/start identity are mandatory. Readiness additionally carries
the owned endpoint process incarnation, loopback port and exact provider thread
UUID. Tokens are absent from this record. Record input is capped at 8 KiB;
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
The final launcher/lease slice must exercise this guard, later admission failure
and withdrawal together before activation; record unit tests alone do not prove
full crash recovery or cleanup of a former endpoint process.

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

The resource owner signals only its directly owned, unreaped process group,
then waits up to two seconds before deleting created files. Startup failure,
explicit stop and drop use the same ordering. Unverified process cleanup reports
failure and retains capability/diagnostic files; it is not successful cleanup.
File cleanup compares created regular-file and directory device/inode identities,
never recursively removes a directory, and preserves replacements. This is
in-process cleanup; launcher crash/orphan recovery remains a final integration
gate, not a guarantee provided by destructors.

`drivers/codex/attachment` accepts a bounded option surface before launch. It
resolves relative `-C` against the original working directory once, emits the
absolute directory for foreground resume, and exposes that same directory to
server spawn and later thread creation. It rejects initial prompts, resume/fork
subcommands and implicit/default remote selection. The foreground command uses
`resume --remote ... --remote-auth-token-env TMT_CODEX_ENDPOINT_TOKEN` with the
exact supplied provider thread. Planning checks endpoint shape; ownership comes
from the enrollment resource, not from a user-supplied endpoint string.

If Codex asks to trust the launch folder, the user must answer in its TUI before
attachment can proceed; the channel never answers that prompt or changes trust
configuration, and accepted queue input may wait for attachment.

Channel permission options are owned by the new thread, not its attached TUI.
`-s`/`--sandbox` accepts `read-only`, `workspace-write` or `danger-full-access`;
`-a`/`--ask-for-approval` accepts `untrusted`, `on-request` or `never`. They set
`thread/start.sandbox` and `thread/start.approvalPolicy`, respectively, and the
same app-server defaults; they are never passed to `resume --remote`. With no
explicit flags, thread creation leaves these fields absent and preserves the
provider defaults. Unsupported values are refused before any process starts.

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
provider turn. The final lease/consumer slice must establish that continuity,
thread admission, channel foreground identity, and terminal routing for talk and
reply notifications before user-facing opt-in is enabled.

## Foreground and cleanup foundations (#785)

The private record stores its claimed pane address before any foreground spawn.
Unknown foreground state stays terminal for that exact pane and is never pruned;
server disappearance is not evidence that an unconfirmed foreground ended. An
explicit same-binding, same-pane relaunch may take over Unknown only after all
recorded processes are conclusively gone. A known foreground must also be gone.
Every mutation compares the exact lease under the existing lock. Records without
attribution require named recovery and cannot establish authority for a pane.

Failed endpoint startup returns both the startup error and cleanup certainty.
When cleanup cannot be confirmed, created files are retained. Later lease code
must preserve enrollment as well; this foundation does not yet compose that owner
or register a channel. Native tests prove record transitions and permission/cwd
planning, not user-facing delivery or launcher-crash cleanup.
