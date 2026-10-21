# Workspace recovery snapshots

Core's pure `workspace` values describe remembered structure. The adapters own
capture, the versioned JSON codec and private publication; `ConfigPaths` alone
chooses the location. Snapshots are recovery input, never authority for binding,
presence, delivery or launching a provider. No SQLite migration, extension
registry, Core timer, daemon or detached worker participates.

The normative document, retention and selection definitions live in the
[workspace recovery contract](../../../../contracts/workspace-v1.md). Core owns
pure generation/selection policy; adapters own immutable bytes, the index and
publication lock; CLI owns preview, consent and restore composition.

## Capture and publication owners

`ConfigPaths` selects the private socket directory. The built-in host makes one
bounded `list-panes -a -F` topology read. PID/start observations are batched, and
native ancestry is cached across panes without a per-pane subprocess fallback.
Unknown ancestry omits an optional command annotation. One read-only joined
storage projection supplies current unretired bindings and preferences without
initialization, migration, reconciliation or housekeeping. Marker, binding,
server, pane PID and recorded native pane incarnation must agree before an
identity is annotated. Mismatch never retires or repairs durable state. External
hosts without a capture port skip.

Publication keeps immutable generation bytes ahead of the atomic catalogue,
then updates the legacy projection and prunes only verified owned orphans.
Readers use the committed catalogue even if a crash left the legacy projection
behind. A valid legacy latest is adopted on the first changed publication;
named save continues to copy that legacy projection byte-for-byte. Recovery
content equality excludes capture time, so no-change publication preserves
recovery bytes and timestamps. The pure Core catalogue retains recent distinct
generations plus the protected pre-shrink baseline, without hourly buckets.
It owns the bound and shrink classification; IO callers do not duplicate them.

## Recovery commands

`workspace ls` lists generations and manual names with capture age, pane/agent
counts and protected status. `show` and `restore` accept a manual name or an
immutable `g:<sha256>` selector. Missing explicit selectors never fall back.
Restore acquires its selected bytes once; `--expect` checks that same acquisition.
Smaller choices use the existing CLI consent owner before any effect, with
noninteractive refusal and an exact pinned retry. Only a completed explicit
restore can update its observed protected choice under the publication lock;
a newer concurrent baseline wins.

Manual save/remove retain the independent 16-name cap and no-clobber publication.
Names and automatic generation selectors have disjoint grammars. Listing, preview
and acquisition perform no capture, initialization or repair. Automatic document
selection never substitutes a same-name identity for its recorded UUID.

## Events and policy

Global `workspace.snapshotEnabled` defaults to true. Set it with
`tmt config set --global workspace.snapshotEnabled false` to disable capture.
Global `workspace.snapshotIntervalMs` is an integer from 0 through 2147483647,
default 60000. After a command that changes state or talks to a pane (talk,
reply, answer, check, focus, and the mutating identity, room, preamble, role and
exchange operations) flushes its output, an advisory refresh reads the previous
snapshot timestamp once under the publication lock. A pure read (`whoami`, `ls`,
`result`, `inbox`, and every identity, room, preamble, role and exchange
listing or show) never refreshes, so its caller never waits for a capture. Only
an older snapshot (strictly beyond the interval), or a missing one, requests
bounded capture. A backwards clock keeps the prior snapshot.
Zero disables command refresh while preserving event capture. Fresh snapshots
need no host or storage reads; due captures retain native caller verification.
Listen, provider hooks, API/MCP and launch/binding event owners do not use this
command path. Optional refresh errors preserve output and exit status.
`config rm` retains its local-only contract. Unknown configuration keys survive
writes.

Committed binding changes request capture after their output. Foreground launches
capture before child spawn and after owned completion/retirement; no snapshot IO
is inserted between spawn and the launch owner's protected storage close.
Admitted provider callbacks (including prompt, Stop and activity observations)
may request optional capture after their context is written and flushed, with
strict remaining-budget headroom for acquisition, cleanup and the output reserve.
The ToolFinished storage-only no-op shortcut still performs no host probe.
Provider attempts share the publication try-lock and a 30-second native-incarnation
cadence; only the separately named ephemeral coordination record changes for an
unchanged recovery. Due attempts read topology once and publish only changed
recovery content. All errors are advisory to the admitted event.

`workspace.capture` is the public, recorded-socket-only backstop port; its strict
input, aggregate budget, bounded counts and native fences live in the
[contract](../../../../contracts/workspace-v1.md#capture-triggers-and-public-backstop).
An optional Ops minute consumer uses its own existing cancellation lifecycle;
Core installs no tmux hooks, scheduled jobs or background owner. Pure reads still
perform no automatic capture.

## External foreground dispatch

The generic external-command boundary first checks the caller's controlling tty
and foreground group without subprocess or config discovery. Non-foreground
tool calls return immediately. Eligible dispatch reads only the selected pane's
tty, observes the caller's native PID/start incarnation once and writes literal
`tmt <extension> ...` argv into `@tmt.workspace-command` before exec, sharing one
finite budget. It never captures or publishes a snapshot before exec; the next
eligible event or command refresh admits the marker through native evidence.
There is no board-specific grammar or Ops knowledge in Core. The process survives
exec; its exact incarnation must remain in the pane's ancestry and foreground
terminal group to be included by later captures. Ended, background, reused or
uncertain owners are excluded. Failed exec clears only its exact marker through a
server-side comparison and refreshes the failed dispatch's snapshot. Literal argv
is data, never a shell command. Help/completion do not install markers.
Linux cross-pane foreground evidence reads only the exact owner's bounded
`/proc/<pid>/stat` tty and process-group fields, agreeing with the selected pane's
native tty device. It does not require the observing command to own that terminal.
Darwin uses the existing selected-process BSD information boundary for the same
tty and foreground-group agreement when the direct terminal query is unavailable.

## Restore preview

`tmt workspace show [<name>] [--socket PATH] [--json]` reads the exact selected socket's
snapshot without capture, publication, database initialization, reconciliation or
provider observation. Outside the original tmux environment, select its absolute
socket path explicitly; no default server or snapshot-directory search occurs.
Malformed, unsupported, oversized or symlinked snapshots are refused unchanged.

The version 1 plan contains the existing `snapshot` topology plus `sessions` and
`panes` actions. A bounded read of current session names marks matching names
`skip_existing`, absent names `create`, and uncertain observation `unknown`. A
missing socket has no sessions. The tmux reader flags control-bearing names before
newline framing and refuses malformed or unsupported output as uncertain; it never
splits one name into multiple sessions. Literal Unicode names remain unchanged under
a C locale through the shared UTF-8 client flag. Refusal preserves snapshot and
durable state. Every effectful restore must recheck live state;
this preview grants no authority to overwrite or launch.
Human output shows snapshot age with the CLI's relative-time formatter; JSON
retains the exact `capturedAtMs`. A stale conversation is labeled as stale without
advertising restore flags that this read-only command does not implement.

Pane actions are `shell`, `relaunch_command` (literal external argv),
`identity_missing`, `no_remembered_session`, `resumable`, or
`stale_requires_retry`. Exact current unretired UUIDs and durable preferences take
precedence over snapshot names and conversation annotations, including unbound
saved identities. A same-name replacement is missing, never resurrected. Pending
resume marks remain informational; stale marks require explicit retry. Public
resume coordinates exclude opaque driver state. No command settles a pending
resume, repairs storage or starts a fresh conversation during preview.
`resumable` means current durable coordinates exist without a stale mark;
provider capability, availability and binding admission are rechecked at launch.

## Layout-only restore

`tmt workspace restore [<name>] --layout-only [--socket PATH] [--json]` reads the selected
snapshot for the exact selected socket through the same bounded, no-follow reader.
It bypasses identity learning, provider observation, SQLite and advisory capture.
Core validates every saved layout's checksum, bounded tree, dimensions, leaf
correspondence, selection and zoom before the adapter permits host effects.
Historical IDs are correspondence keys, never live targets.

The built-in tmux adapter owns a finite 30-second creation budget and native
server-incarnation fencing. Matching live session names are skipped whole,
including their windows and panes. New sessions use only returned native IDs;
shared saved windows link only among invocation-created sessions. User panes
pass no command and inherit tmux's `default-command`/`default-shell`, entering
recorded directories checked before creation and read back afterwards. No saved
identity, provider session or external command is replayed or bound. Window-local
geometry and border settings affect only created windows; existing server options and pre-existing resources are preserved.

Outside tmux, select the socket explicitly. The first `new-session` starts an
absent server and initializes a fresh TMT server UUID; snapshot server identity
is never adopted. A remaining socket is stale only after a bounded nonblocking
OS connect proves `ECONNREFUSED`; tmux then owns normal socket replacement.
Core never unlinks it. Live, denied, timed-out or otherwise uncertain sockets
remain refused unchanged.
The command prints one created/skipped/partial summary, or one version 1 JSON
result with session actions, recorded/native window and pane mappings, failures
and retained bootstrap IDs. Partial failure returns exit 1, retains created
resources and stops further effects; reruns skip the surviving sessions.

A session containing only windows already created by this invocation needs a
bootstrap `/bin/sh -i` shell. Its exact startup command is rechecked, and it may
be removed only after all required links succeed and fresh native PID/start, pane,
ordinary-shell and foreground evidence agree.
A server-side guard also requires the same server/pane/PID, an unattached session,
one unshared bootstrap pane, another linked window and no TMT runtime markers.
Failed linking or uncertain verification retains and reports the bootstrap.
No pre-existing pane, window or session is removed or overwritten.

## Full restore

`tmt workspace restore [<name>] [--socket PATH] [--json]` runs the same layout phase,
then starts eligible panes through this executable's ordinary `tmt resume -- <name>`
or generic external dispatch with the recorded literal TMT argv. Core adds no
provider launch path or board grammar. A partial layout starts nothing; existing
sessions are skipped whole. Shared panes and repeated identities start at most once.

Restore checks the snapshot name still names its original unretired UUID before
starting resume. Renamed, replaced, missing, stale and sessionless identities need
the user; no fresh conversation or automatic retry is substituted. Resume retains
its own binding, live-agent, session, preset and channel admission rules. Current
durable coordinates take precedence over snapshot conversation annotations.

Only panes selected for revival initially run an invocation-owned `/bin/sh -i`.
The adapter replaces that shell through direct tmux startup after
checking it still owns the pane and foreground; ordinary and pre-existing panes
remain untouched. No paste or send-keys participates. The existing native server
fence and literal tmux quoting apply. Launches use the restore-selected Core data
directory. The user's tmux `default-shell` runs the strictly POSIX-quoted command
and returns to that shell when the agent or board exits, preserving the pane.
Startup errors retain resources and never authorize replay.

Human output has one short line per pane; JSON adds `revival` rows with recorded
and native pane IDs, name, status and message. Started means process startup was
requested, not that the provider confirmed its conversation. Missing or stale
sessions are reported without launching; a startup failure yields partial/exit 1.
`--layout-only` preserves its output and storage-free behavior.
