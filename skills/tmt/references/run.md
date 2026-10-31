# TMT reference: Run, resume, channels and the built-in host

Part of the tmt skill, printed by `tmt learn --skill tmt run`.
The rules of the skill entry (`tmt learn --skill`) still apply.

### Foreground identity launch

`tmt run <registered-agent> [args...]` starts immediately under a generated
temporary name such as `claude-12ab34cd56ef`, with one naming hint and no prompt.
Inside the agent, `tmt this <name>` (or `name`) renames only a verified auto-named
identity, preserving its UUID, binding, session and any opted-in channel.
Naming does not save: use `tmt this --save <name>` or `tmt run --save <agent>`.
Normal exit keeps the binding; temporary retirement still follows unbind or
conclusive pane loss. A failed spawn retires only the new temporary auto-name.
An existing destination name refuses without merging identities. Ordinary named
panes retain their conflict rules; intentional later renaming uses `tmt mv`.
If an identity holds the executable name, shorthand refuses: use
`tmt run claude claude` for that identity or `tmt run <new-name> claude`.
TMT options precede the first operand; shorthand provider arguments stay verbatim.
Resume still needs the hook-recorded session; naming preserves it but never guesses
one. Save to keep resume coordinates across pane loss. Naming an enrolled launch
neither restarts its channel nor enables paste fallback.

With `experimental.channel` off (the default), explicit `--channel` refuses; flagless resume
uses paste without changing the remembered channel choice. Re-enabling the setting
restores that choice. `--no-channel` remains accepted and remembers plain delivery.
A still-running channel session must end before another launch for that identity;
dead prior enrollment records need no manual cleanup for paste resume.

With the setting on, fresh launches use the driver's channel default: Codex and Claude remain opt-in.
An admitted fresh launch remembers its channel/plain choice. Exact resume
reuses it; explicit `--channel`/`--no-channel` on resume updates the remembered
choice after successful admission. Opt in once with `tmt resume --channel <name>`;
failed Required enrollment leaves the choice unchanged. Resume without flags
preserves it. While the setting is on, a remembered channel
requires successful enrollment and never silently falls back to paste.
`--no-channel` chooses and remembers plain paste delivery, while `--channel`
requires enrollment or fails before recording an enabled choice. The flags conflict
and go before the name for both run and exact resume. A fresh default launch uses
the original plain command without channel enrollment. Live or unconfirmed
enrollment evidence remains terminal; never paste or resend into an enrolled
pane after a refused or uncertain channel outcome. Codex owns an extra private
app-server and supervisor per enrolled foreground, cleaned on exit and Ctrl-C.
Folder trust is answered by the user in Codex's TUI; accepted queued input may
wait for attachment. Enrollment never changes global provider config, auth or hooks.

TMT options, including `-s`/`--save` and `--resume`, go before the name; every
argument after it belongs to the command, without requiring a `--` separator.
The executable itself cannot start with `-`; misplaced TMT flags such as
`tmt run Alice -s` fail before binding. Flags after the executable stay verbatim.
For a known agent, `--model <model>` and `--effort <effort>` before the name choose
its model and reasoning effort; its driver turns them into the provider's own flags,
the way resume applies them. A command no driver claims, or a driver that cannot apply
the value (agy takes no effort), is refused with `RESUME_SETTINGS_INVALID` before
anything starts.

A new identity is temporary unless `-s` is supplied; existing saved identities
never downgrade. Registered runtime drivers remember their harness ID; provider
hooks record the session and observed model. A bounded launch preset separately records the exact
executable, extracted model/effort and only `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` and
`CLAUDE_MINI_PCT` when present. No full argv or other environment is stored.
An admitted provider session associates a wrapper with that exact launch; a wrapper
name alone never identifies a provider. Generic commands leave the prior harness
preference intact.
Bare `tmt run <name>` launches the remembered registered executable through PATH
with no arguments; without one, supply an explicit command.

Outside tmux and Herdr, `tmt run` and named `tmt resume` create a built-in-host
member. They report the actual name, host and pane after managed startup is
confirmed. A fresh `tmt run` from an interactive terminal then attaches to the
member; `tmt resume`, or a run whose stdin or stdout is not a terminal or whose
`TERM` is `dumb`, returns after confirmation and leaves it in the background.
Closing this terminal leaves it running; `tmt ls`, `talk`, `reply`, `check`,
`unbind` and `rm` use the same identity and exchange contracts.
`check` reads only the current screen, with no scrollback. Use
`--host tmux|herdr|pty` before the name to choose; an unavailable explicit host
refuses and never falls back. `tmt attach <name-or-UUID>` enters a built-in
member's terminal; plain `tmt focus <name-or-UUID>` does the same. It requires a
terminal and refuses JSON. Attachment first paints the current screen (no
scrollback), then follows live output. Ctrl-\
detaches and leaves the member running; Ctrl-C goes to the program. `attach -d`
takes sole use and disconnects other viewers. Terminal sizes above 1000 columns
or rows are clamped; another viewer's resize never resizes your physical window.
After a program exits, attachment restores your terminal and prints its resume
command. A lost connection does not prove the program ended; inspect `tmt ls --json`
and `tmt check <name>` first. Once a saved member is offline after supervisor loss,
`tmt resume <name>` starts its recorded session in a new terminal; it never resends
old requests. `tmt stop --close <name>` stops the verified agent and waits for its
supervisor to end. `rm` and `unbind` change identity bindings; they are not stop commands.

`tmt resume` uses this pane's verified bound identity; an unbound pane refuses and
lists saved names. `tmt resume <name>` (alias `tmt run --resume <name>`) resumes the exact
hook-recorded session in the current verified host, or a new built-in terminal outside one. It replays the associated executable,
last observed model and launch effort. `--model` and `--effort` before the name
explicitly override those values; present allowed environment keys override stored
ones, even when empty. `--show` displays the preset without launching and
`--forget-launch` clears it while retaining the session. Without an associated preset, resume reports that it uses the registered driver.
Before it
launches, resume checks that a Claude or Codex session still has its transcript.
A session that is gone or expired, or whose file TMT may not read, is refused
with `RESUME_UNAVAILABLE` and `error.reason` `session_missing` or
`session_unreadable`; nothing is marked or forgotten. A resume
that fails before the provider confirms the session marks it stale
(`session_stale`); a stale session needs `tmt resume --retry <name>` or
`tmt resume --forget <name>`. `--retry` also tries a session the check refused,
and `--move` skips the check, as does a resume that replays a wrapper (a launch
preset whose executable file name is not the driver's own). A wrapper installed
under the driver's own name, such as `~/bin/claude` that sets `CLAUDE_CONFIG_DIR`,
is still checked; use `--retry` for it. The gone, unreadable and stale refusals
end with `Start fresh from its notes: tmt resume --new -- <name>`. `--new` keeps the
identity and starts its driver (the remembered one, else the preferred one) with
a fresh session. Bounded notes and open-request metadata are its first message,
as in a driver switch; nothing is stopped or asked of an old agent. It replays
the old session's launch preset. The old session stays remembered until the new
session starts, so a failed start keeps it for `--retry`. `--new` cannot be
combined with `--retry`, `--move`, `--driver`, `--show` or `--forget*`.

To move a member to this pane on the same computer, use `tmt resume <name> --move`.
TMT verifies the old agent, sends it one TERM, and waits up to three seconds before
moving the binding and resuming that exact session here. TMT never closes either
pane; the identity and its notes, role and requests stay the same. A pane process
or an unverified/shared agent cannot be stopped this way. TMT never stops a
shared Codex channel server. If the old agent does not exit, nothing new starts and the
binding stays put. Exit the old agent in its pane, then rerun the exact command
printed in the error. TMT never force-kills it or falls back to a fresh session.

To end a member's agent without moving it, run `tmt stop <name>`: the same verified
one-TERM stop, no KILL or retry. The identity, binding and remembered session stay, so
`tmt resume <name>` brings it back. `--close` also closes the pane, only when its host says
TMT opened it; a pane you created, or one whose origin is unknown, stays open and the
command says so. `resume`, `mv`, `rm` and `stop` also take the saved identity UUID.

`tmt rm <name>` refuses `AGENT_RUNNING` (exit 5) while the member's agent is alive or
not proven ended, and removes nothing. Run `tmt stop <name>` first; an agent TMT cannot
prove (a live pane with no recorded agent) is exited in its pane, or unbound with
`tmt unbind <name>`. When the host cannot be observed, the message says so instead.
`--force` only confirms removing a saved identity and never overrides this check.

To switch the agent behind the same identity, run
`tmt resume <name> --driver claude --model claude-sonnet-5-5` (or `--driver codex`).
This includes the stop/move path and launches in this command's pane; no extra
`--move` is needed. TMT asks the old agent once to save a handoff in its notes,
waits up to 30 seconds for its reply, then uses the same verified one-TERM stop.
If it has gone or does not answer, bounded notes and open-request metadata supply
the handoff. The new agent receives that context as its first message; it is not
proof that any request was answered. UUID, name, role, board row, notes and open
requests stay. The old driver's exact session is retained, so switching back
resumes it; a driver's first use starts a new session. Provider settings stay
untouched. Unsupported/shared runtimes refuse before stopping anything. If the
selected message channel cannot take initial handoff input, use the error's
`--no-channel` command. Inspect both panes before retrying an uncertain launch.

TMT records the owned command's exit and keeps its remembered session. A foreground
launch returns the command's status, or 128 plus a terminating signal number; a
built-in launch returns startup confirmation, or ends with the attachment that
follows it. Its supervisor ends when the owned launcher exits, so its binding can become offline. A supervisor that is
killed takes its program with it: `tmt ls` shows the member offline, `check` refuses
it as not active, and a new `talk` queues for its inbox instead of pasting anywhere;
other built-in members are unaffected. It reaps only
its direct child; harness-created background processes are the harness's
responsibility. Killing TMT with SIGKILL cannot guarantee child cleanup. A live
child whose launch owner is lost is Unknown and is not a verified input target.
Do not treat successful spawn, runtime presence or an Ended observation as a
durable task reply.

### Workspace recovery

Use `tmt workspace ls --socket <original-path> --json` to inspect retained
recovery generations when the original pane is gone, then
`tmt workspace show <generation> --socket <original-path> --json` to preview.
A generation is selected as `g:<sha256>`; manual names still work. Automatic
restore prefers protected recovery that records agents. A smaller choice
needs the user's confirmation, or an explicit selector and matching `--expect`
digest in noninteractive use. Snapshots never prove an agent is live or permit
replacing existing panes; a full restore still resumes agents only through the
normal `tmt run --resume` checks. See the
[workspace contract](https://github.com/tmt-ninja/tmt/blob/main/contracts/workspace-v1.md).
