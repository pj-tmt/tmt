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
default 60000. It is reserved for the separate command-refresh implementation;
this store/event layer does not consume it, and zero does not disable events.
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

The generic external-command boundary remembers literal `tmt <extension> ...`
argv and its native PID/start incarnation in an owned pane option before exec.
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
