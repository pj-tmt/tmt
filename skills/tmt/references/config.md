# TMT reference: Configuration

Part of the tmt skill, printed by `tmt learn --skill tmt config`.
The rules of the skill entry (`tmt learn --skill`) still apply.

## Configuration

Use `tmt config show --json` to inspect resolved settings and file paths.
Human `config show` also labels actual value source, accepted values and whether
each key is CLI-editable locally/globally or global-file-only. The three
`defaults.timeout`, `defaults.pollInterval` and `defaults.captureLines` keys
are global-file-only and cannot be changed by `config set` or `config rm`.
`config set` supports `preambleMode`, `preambleEvery`, and
`pasteEnterDelayMs`; without `--global` it writes a local override. Numeric writes require decimal digits only: no suffixes,
fractions, signs, or whitespace. Zero disables preamble injection or removes
the paste-to-Enter delay. Preamble frequency is bounded to a safe integer;
paste delay is at most 2147483647 milliseconds.
The default paste-to-Enter delay is 500 milliseconds; `config show` reports
the effective value after global and local overrides.

`tmt config set notifications.replyBatchWindowMs 5000 --global` sets the fixed
reply-notice window (`0..60000` integer milliseconds); `0` disables grouping.
`tmt config set notifications.typingQuietMs 2000 --global` sets the key debounce
quiet period (`0..30000` integer milliseconds). Both are global-only; new notices
never reset the batch deadline. tmux activity has second resolution, so quiet
detection can take an extra second. It does not prove the input buffer is empty.

`tmt config set exchange.retentionDays 90 --global` sets the duration for new
requests, from 1 through 3650 integer days (the agent's answers keep their own 7 days).
Local overrides and local `config rm` are not supported for this global-only key.
Changing it never extends existing data or changes the reply acceptance window
or observer timeout. Results remain available without reading current configuration.

Expired content is unavailable at its stored UTC deadline. Request/result
operations perform bounded opportunistic cleanup; without an invocation there
is no punctual physical deletion. A late accepted final has its own duration
from submission, so metadata can outlive the original request horizon. Reads
never acknowledge a result. Cleanup is not file shrinkage or secure erasure;
wall-clock rollback can delay logical expiry while data remains stored.

A cosmetic pane-local `@tmt.badge` label is on by default;
`tmt config set ui.paneBadge off --global` turns it off. Recorded Running displays
`● alice (tmt)` with a green dot, Ended displays a dim `○ alice (tmt)`, and
Unknown displays plain `alice (tmt)`. This is recorded session state, not work
activity or permission to type; without lifecycle updates it can become stale.
It is global-only and uses `ui.paneBadge` in the same global file. Settings
changes do not scan panes; the next binding, launch/exit, provider hook or
recovered session applies the setting to that pane. `unbind` clears its badge regardless
of the setting.
With `off`, the next successful binding clears a previously published badge.
Badge writes are bounded and best-effort; a display failure is not a reason to
retry a successful identity mutation. Binding commands validate loaded settings
before mutation.

`tmt config set user.name <name> --global` sets the name of the person's own
identity (1 to 64 characters, not a pane target); without it the OS login name is
used. Surfaces that send as the person resolve it with the `user.identity` API
operation, which creates and marks that identity. A name an agent already holds is
never taken over: the identity becomes `<name>-user`. Changing the name starts a
new identity; the old one keeps its history. The key is global-only and has no
`config rm`.

On a badge refresh, TMT prefixes the conditional fragment
`#{?@tmt.badge, [#{@tmt.badge}],}` to the pane's inherited
`pane-border-format`, using a pane-local override only. The inherited content,
title, colors and border position stay unchanged; no global, session or window
option or `~/.tmux.conf` is written. A user-owned pane override or a format already
referencing the badge is preserved. The pane-local `@tmt.border` records the
exact installed format. Unbind or the next refresh with `off` removes only an
unchanged TMT-owned override; later user edits remain intact. The override snapshots
the inherited format when installed. After a theme reload, unbind and bind again
to inherit the updated format.

The label is visible when the window already has pane borders enabled. If
`pane-border-status` is `off`, an explicit `this`/`name`/`add` binding or `run`
launch prints one stderr hint with a command to enable it, and leaves it off.
Delivery, provider hooks and automatic refreshes stay silent. The hint may
accompany successful JSON output. An optional custom theme may insert the fragment elsewhere or choose its
own narrow-pane policy. Display labels neutralize `#` and control characters and
cap names at 48 Unicode code points; identity names are unchanged. Previously
overwritten titles/layouts require restoration from the user's saved theme; do
not guess or overwrite them as a migration.

For a badge hidden below 80 columns:
`#{?#{&&:#{@tmt.badge},#{e|>=:#{pane_width},80}}, [#{@tmt.badge}],}`.
The width threshold is theme-specific, not automatic fitting. Generated session
styles restore surrounding attributes with `push-default`/`default`/`pop-default`.
Do not place this inside another `push-default` span: tmux has no nested saved
defaults. Place it outside that span and use the theme's explicit restoration.

Invalid known fields in a loaded config return `CONFIG_ERROR` (exit 1) before
talk/check effects, even when another layer would override them. Unknown and
retired fields remain opaque and are not migrated. A rejected settings update
leaves the file unchanged. Correct the reported field; do not delete the whole
configuration as a workaround. Reply acceptance and `result` do not load
unrelated settings, so malformed config does not prevent durable submission
or retrieval.
