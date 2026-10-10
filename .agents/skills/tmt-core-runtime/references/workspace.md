# Workspace recovery snapshots

Core's pure `workspace` values describe remembered structure. The adapters own
capture, the versioned JSON codec and private publication; `ConfigPaths` alone
chooses the location. Snapshots are recovery input, never authority for binding,
presence, delivery or launching a provider. No SQLite migration, extension
registry, timer, daemon or detached worker participates.

## Location and format

Each selected host socket has
`<global_dir>/workspace/<lowercase-sha256(socket-bytes)>/latest.json`. The directory
is private and owned, publication uses a stable nonblocking advisory lock, and a
staged 0600 file is synced and atomically renamed. Readers get a complete old or
new document. Contention coalesces overlapping events by skipping their optional
work; the last successful publication wins. Malformed or unsupported previous
data, failed acquisition, oversized topology or uncertain evidence preserves the
previous snapshot. No foreign files or lock inodes are removed.

Version 1 records `capturedAtMs`, `server` (socket, native PID/start incarnation,
optional TMT server UUID), `sessions`, `windows` and `panes`. Sessions retain
indexed window links and which link is active; linked windows are deduplicated by
ID. Windows retain names, full and visible layout strings, dimensions and active
pane. Panes retain their window/index, geometry and cwd. Unbound panes are ordinary
shells. Optional identity annotations retain exact UUID/lifetime/binding,
remembered harness/session/mode and channel choice; no transcript, provider
environment, credentials or agent command line is copied.

The built-in host makes one bounded `list-panes -a -F` topology read. Native
process evidence fences the selected server, pane and external-command owners.
One read-only joined storage projection supplies current unretired bindings and
preferences without initialization, migration, reconciliation or housekeeping.
Marker, binding, server, pane PID and recorded native pane incarnation must agree
before an identity is annotated. Mismatch leaves an ordinary pane and never
retires or repairs durable state. External hosts without a capture port skip.

## Events and policy

Global `workspace.snapshotEnabled` defaults to true. Set it with
`tmt config set --global workspace.snapshotEnabled false` to disable capture.
Global `workspace.snapshotIntervalMs` is an integer from 0 through 2147483647,
default 60000. After finite communication and inspection commands flush their
output, an advisory refresh reads the previous snapshot timestamp once under the
publication lock. Only an older snapshot (strictly beyond the interval), or a
missing one, requests bounded capture. A backwards clock keeps the prior snapshot.
Zero disables command refresh while preserving event capture. Fresh snapshots
need no host or storage reads; due captures retain native caller verification.
Listen, provider hooks, API/MCP and launch/binding event owners do not use this
command path. Optional refresh errors preserve output and exit status.
`config rm` retains its local-only contract. Unknown configuration keys survive
writes.

Committed binding changes request capture after their output. Foreground launches
capture before child spawn and after owned completion/retirement; no snapshot IO
is inserted between spawn and the launch owner's protected storage close.
Admitted SessionStart/SessionEnd callbacks may capture only after their context is
written and flushed, with strict remaining-budget headroom for acquisition,
subprocess cleanup and the existing output reserve. Stop and UserPromptSubmit
never capture. All errors are advisory and cannot change trigger output, error or
exit status. Manual layout edits appear at the next eligible event.

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

`tmt workspace show [--socket PATH] [--json]` reads the exact selected socket's
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

`tmt workspace restore --layout-only [--socket PATH] [--json]` reads the latest
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
