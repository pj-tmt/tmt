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
missing socket has no sessions. Every effectful restore must recheck live state;
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
