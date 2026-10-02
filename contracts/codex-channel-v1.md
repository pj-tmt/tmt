# Codex native channel contract

Status: native Codex channel implementation in #739, following #736–#738 under
#719/#329. The final activation slice registers `tmt run --channel codex` against
the shared channel routing and launcher ports. Queue acceptance is a delivery
receipt; durable request completion remains separate. The pinned 0.159.3
attachment/active-turn proof and 0.160.0 attach/queue/durable-reply proof are
accepted; product routing and lifecycle gates are independent evidence described
below.

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
and accepts the 0.159 minor line at patch 2 or later, or exactly 0.160.0. Older,
malformed or other-line/build output is refused; 0.159 patches above 3 retain an
unqualified-build advisory. This does not qualify them: the owned initialize
handshake remains authoritative and accepts only the exact supported builds.
Untested 0.160.x patches are refused at both boundaries.

| Provider build | Binary preflight | Owned initialize | Live qualification |
| --- | --- | --- | --- |
| 0.159.2 | Accepted | Accepted | Isolated native queue observations in #329; no accepted foreground continuity claim for this build. |
| 0.159.3 | Accepted | Accepted | Accepted foreground attachment and active-turn continuity proof in #739. |
| 0.160.0 | Accepted | Accepted | Accepted foreground attachment, idle/busy queue correlation and durable reply in #1043. |
| Later 0.159 patches | Advisory | Refused | Unqualified. |
| Other builds, including later 0.160 patches | Refused | Refused | Unqualified. |

The initialize format is source-backed at the pinned revision above, in
`request_processors/initialize_processor.rs` and
`login/src/auth/default_client.rs`. The accepted 0.159.3 attachment proof qualifies the remote foreground behavior
for that build. The accepted 0.160.0 proof separately qualifies that build; the
allowlist alone is not runtime evidence for future versions.

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
The consumer tests and live-provider evidence below cover separate properties.
Release-size evidence measures the fully linked CLI against the matching baseline;
groundwork alone may be removed from an unused binary by the linker.

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

The resource owner signals only its directly owned, unreaped process group,
then waits up to two seconds before deleting created files. Startup failure,
explicit stop and drop use the same ordering. Unverified process cleanup reports
failure and retains capability/diagnostic files; it is not successful cleanup.
File cleanup compares created regular-file and directory device/inode identities,
never recursively removes a directory, and preserves replacements. This is
in-process cleanup; the supervisor described below owns launcher crash handling,
which is not a guarantee provided by destructors.

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
provider turn. The live proof below establishes provider continuity separately from the real
CLI tests for thread admission, foreground identity and terminal routing.

## Consumer lifecycle

The provider record persists the claimed identity and pane address before spawn:
host/server UUID, socket path, server PID/start, pane ID and pane PID. The launcher
supplies this evidence from its existing binding, without another binding write.
Missing or mismatched attribution refuses enrollment. Older/unreadable records
without attributable pane evidence are named in diagnostics; they cannot block
unrelated panes. Corruption of the current binding's own record remains terminal.

Foreground state starts Unknown. The launcher's single `foreground_started`
callback publishes only the original owned child's observed incarnation under
the record lock. A publication failure stays unconfirmed. A ready app-server
precedes foreground spawn, so cleaning that server never proves an Unknown
foreground ended. Unknown stays terminal for its exact pane and is never pruned.
Known foreground, owner and endpoint must all be conclusively gone before an
ended record is pruned during enrollment. Read-only delivery never prunes.

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
Manual recovery requires first verifying that the named pane's original
foreground and endpoint are gone, then removing only the exact named stale
record. Recovery is separate from delivery and never pastes a payload. Shell
commands in recovery diagnostics must quote paths, including embedded apostrophes.

The native send path requires the exact record, launch owner, foreground,
endpoint and thread, qualifies the owned endpoint once, and consumes one queue
attempt. Channel-originated hooks preserve the admitted foreground incarnation;
ordinary non-channel resume remains separate. Mock supervisor tests exercise
startup failure, launcher SIGKILL with a surviving foreground, the publication
window and explicit confirmed withdrawal. The registered driver participates in the shared `enrolled_harness` route selection
and `enrolled_in_pane` guards; it does not create a second paste policy. This
includes originator reply notifications, identity sends and identity-less panes.

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
shared-file hash preservation and independent cleanup verification. Post-idle
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
