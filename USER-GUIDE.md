# tmux-team user guide

This guide covers the common v5 native alpha workflows. Start with the
[README](README.md) for the current installation status and verified release
URL, then use
[`skills/README.md`](skills/README.md) for provider-specific installation and
[`skills/tmux-team/SKILL.md`](skills/tmux-team/SKILL.md) for canonical agent
guidance. Optional Office workflows have canonical
[`tmt-office`](extensions/tmt-office/skills/tmt-office/SKILL.md) and
[`tmt-prop-create`](extensions/tmt-office/skills/tmt-prop-create/SKILL.md) skills.

## Install and load the skill

Use the native installer asset from a published release as described by the
README, then let the native executable detect supported agents with `tmt install`.
The installer defaults to `$HOME/.local/bin/tmt` and runs skill installation
unless `--no-skill` is supplied. Reload the agent after installation.

The native runtime requires macOS or Linux and tmux for pane operations, but no
Node.js, Rust toolchain or source checkout. Native `tmt upgrade` (also
available as `tmt update`) follows its retained stable/alpha channel; use
`--to <version>` to pin or `--unpin` to resume channel updates. Package-manager
installations from older releases are a separate legacy TypeScript runtime.
Use their original manager to remove them before switching; current repository
source is not an npm product installation. See the replacement guidance below.

After installation, load or reload the `tmux-team` skill in every agent that
will send or receive TMT work. Installation places the provider integration;
it does not reload an already running agent session.

Native `name` and `add` bindings are temporary by default. Add `-s`/`--save`
to preserve an identity, and use `tmt rm <name>` to retire a temporary identity
(`--force` is required for a saved identity). `tmt rename <old> <new>` (also
`tmt identity rename`) gives an identity a new name and keeps its UUID, and with
it the remembered session, profile, notes, metadata, rooms and history. Requests
sent before the rename still reach it; new ones must use the new name. Switching from npm or pnpm is a
fresh installation: stop old writers first; no configuration, database or
historical exchange is migrated or deleted. Native schema migrations are forward-only,
so never use the old TypeScript writer on a native database.

## Connect agent lifecycle hooks

`tmt setup` sets up every agent it finds, after one approval. It reads only the
filesystem and never starts an agent. Agents with an executable on PATH get the
TMT skills and session hooks. Agents that are only configured get the skills,
and so do installed extensions' skills. It prints every file it will change and
asks once; nothing already set up is offered again. A skill path that holds
something else is listed under KEEP and left as it is. An older TMT skill linked
from another TMT installation is listed as a change: setup backs it up next to
the skills folder (`.tmt-skill-backups`) and replaces it. Without a terminal it needs
`--yes` and otherwise changes nothing. Run `tmt setup claude` or `tmt setup codex`
to review one agent's exact settings and launcher paths and approve that plan. Noninteractive use requires `--yes`; add `--json` for a
structured result. This updates only TMT-owned SessionStart/SessionEnd entries in
`~/.claude/settings.json` or Codex's `CODEX_HOME/hooks.json` (default
`~/.codex/hooks.json`), retaining other hooks and permission settings. It does
not install the agent, approve provider hook trust, or change permission policy.

The hook uses the stable `tmt` launcher selected on PATH. Keep that launcher in
place across upgrades; rerun setup if it moves. An identical rerun makes no
changes. Add `--remove` to review removal of only unchanged TMT hooks.
Edited/conflicting hooks or invalid JSON are left untouched. Updates report a
recoverable settings backup; identity, notes and exchange data are never removed.

In a verified bound tmux pane, starts restore the small `whoami --context` summary
and record the exact independent Claude/Codex session for resume. Clear and compact do not change
the pane's identity. Unbound panes receive a binding hint, not a guessed identity;
unavailable evidence produces no context. Hook failures do not veto the agent or
grant permissions. Hooks observe only their own short lifecycle window; they do
not run a daemon. Session-only/Desktop identity binding is not supported yet.

Codex shared-server hooks require an existing exact thread mapping, recorded by
an independent session or an exact `tmt run --resume`; the server's inherited pane
never selects your identity. An unmapped shared thread receives no context or
session write. Disconnecting a shared client does not prove that its thread ended.

## Name panes and inspect presence

Give each live agent pane a global name from that pane's shell, before
launching the agent:

```bash
tmt name reviewer
# Or bind another pane from a shell that can address it:
tmt add %12 gemini
# Or mark the intended pane in tmux, then bind that explicit mark:
tmt marked claude
```

Use these commands to inspect your agents:

```bash
tmt ls                # agents first, saved then temporary
tmt ls --all          # also each offline identity with nothing to resume
tmt ls --saved        # or --temp; --here keeps this tmux session's agents
tmt ls reviewer       # one identity's full details, including its pane
tmt whoami
```

`tmt ls` prints one row per agent: a state mark, the name, where the agent
lives and its folder, with an action at the end only where one is possible:

```text
SAVED 3
  ●  astra            codex:019a2f4c   ~/dev/tmux-team
  ●  reviewer         claude:7c41e9d2  ~/dev/tmux-team
  ○  sol              claude:3f9a1c07                    ↻ tmt resume sol
    offline: gemini

TEMPORARY 1
  ◌  scratch          tmux:%31         ~/dev/scratch     shell
```

`●` means an agent is running, `○` offline and `◌` a bound pane with only a
shell in it. The address is the agent's own session (`claude:`, `codex:`) when
TMT knows it, otherwise its tmux pane (`tmux:%N`); identifiers are shortened to
8 characters. `↻ tmt resume <name>` appears when a remembered session can
resume, and `stale` when that session is gone. `tmt ls <name>` shows the full
values, the tmux location and the remembered session. The output follows the
[CLI style](docs/cli-style.md): on a terminal, rows never wrap and folders are
shortened first; piped output keeps every value whole. Control characters in
names or folders are shown as escapes, not executed. Use `--json` for scripts:
each row adds `address` and `driver` to the existing fields.

`name` and ordinary `whoami` need a live caller pane. `add` accepts `%pane_id`,
`window.pane`, or `session:window.pane`; the current order is pane target first,
global name second. `marked` selects only the pane explicitly marked on the
tmux server selected by the invocation. It never substitutes the caller or
active pane, never searches another server, and leaves the mark unchanged.
Add `-s` to any binding command to save or promote the identity. `tmt unbind`
retires a temporary identity; a saved identity and its profile remain offline.
Neither operation kills the pane.

To recover identity context after a conversation restart or compaction, use
`tmt whoami --context` (add `--json` for tools). It reports the verified identity,
its lifetime, a short role summary, an existing saved notes path, and counts with
inspect commands for unacknowledged originated and incoming X items. It does not
include request bodies or IDs, read notebook contents, bind an identity, create
files, renew retention or mark messages read. A verified empty pane gets the hint
`TMT: this pane has no identity. If the user wants TMT messaging here, they can run: tmt name <name> (-s to save).`
Unavailable or ambiguous evidence instead returns empty human output (JSON
`status: "unavailable"`) successfully; it does not guess an identity.

Output is at most 4 KiB, with at most 500 role characters. Counts and inspect
commands remain available when role/path content is shortened; `truncated` marks
output shortening. Ordinary `whoami --json` retains its existing fields and additionally
reports `interfaceKind` and `sessionState`. Extension context is currently empty.

Global names are independent of the working directory. A durable identity can
exist without an active pane:

```bash
tmt identity create coordinator --json
tmt identity show coordinator --json
tmt identity list --json
```

`identity show <name-or-uuid>` reads an active stored identity without tmux.
An active canonical UUID takes precedence over an identical UUID-shaped display
name; otherwise selection uses the normalized name. Inside a
verified bound pane, `identity show` may omit the name to inspect its caller;
outside that context, supply the name. `identity list` still lists all stored
identities, and bare `preamble show` still lists all stored preambles.

Creation is idempotent for a canonical-equivalent name. Creation alone does not
bind a pane, authenticate a caller, queue work, or perform delivery. The active
identity can receive a later `talk` request in its Inbox while offline;
`talk --inbox` explicitly queues without attempting live delivery.

Attach exact, searchable descriptive metadata to an active identity:

```bash
tmt identity meta set --identity coordinator department engineering
tmt identity meta set --identity coordinator project tmt
tmt identity meta set --identity coordinator capability.review true
tmt identity meta list --identity coordinator --json
tmt identity meta get --identity coordinator project
tmt identity meta rm --identity coordinator project
tmt identity list --where project=tmt --where department=engineering --json
tmt identity list --has capability.review --json
```

Repeated `--where KEY=VALUE` and `--has KEY` filters are combined with AND;
`KEY=VALUE` splits at its first equals sign. Keys are case-sensitive literal
strings of 1–64 ASCII bytes matching `[a-z][a-z0-9_.-]*`. Values are exact,
case-sensitive strings of 1–1024 UTF-8 bytes without control characters
(including the literal string `true`), with at most 64 entries per identity.
Metadata does not save a temporary identity and is not authentication,
authorization, live presence, a capability grant, or a safe place for secrets or
instructions. Omit `--identity` only from a verified bound pane.

### Show an agent's pane

`tmt focus <name>` switches your own tmux client to that identity's pane, even
in another session, after verifying the binding. It never types into the pane.
The JSON result records where you were and which client moved, so you can
return:

```bash
tmt focus auth-fix --json        # {"focused":{"pane":"%5"},"from":{"pane":"%2"},"client":"/dev/ttys004"}
tmt focus %2                     # back to the pane you came from
tmt focus --client --json        # {"client":"/dev/ttys004","pane":"%5"}
```

Run it inside tmux: from a pane, a `display-popup`, or a key binding's
`run-shell` (which has no `TMUX_PANE`; `TMUX` names the session). Only the client
showing that session moves. `--client` takes no target and never switches anything: it
names the client a focus from here would move and the pane that client shows.
Outside tmux, or when no client shows your session, both fail with
`HOST_UNSUPPORTED` and change nothing.

## Launch a command with an identity

Inside a tmux pane, use `run` to bind an identity and start a foreground command:

```bash
tmt run reviewer claude --model sonnet
tmt run -s coordinator codex
tmt run coordinator
tmt resume coordinator
```

TMT options go before the name. Everything after it is the command and its exact
arguments; no `--` separator is needed. TMT does not insert a provider session ID
or store arguments, secrets or executable paths. A new identity is
temporary unless `-s`/`--save` is supplied; an existing saved identity stays saved.
The ordinary binding conflict rules still apply.
The executable word cannot start with `-`: `tmt run Alice -s` is rejected before
binding. Use `tmt run -s Alice <command>`; flags after a real command stay exact.

Registered runtime drivers recognize their executables and remember only the
harness ID. Bare `run <name>` resolves that driver's executable through PATH and
passes no arguments. Without a remembered harness, specify a command. An
unrecognized command still runs with the same binding and lifetime tracking,
without replacing a previously remembered harness.

### Resume a remembered session

```bash
tmt resume coordinator           # in the pane where it should run
tmt resume --retry coordinator   # try a session marked stale once more
tmt resume --forget coordinator  # clear the remembered session
```

`tmt resume <name>` resumes the identity's exact remembered provider session in
the current pane, with the model its provider last reported. `tmt run --resume
<name>` is the same command. It never uses a provider's "last session" shortcut,
and it never starts fresh: when nothing is remembered, or the session's driver
cannot resume it, the command reports why and ends with
`Start fresh with: tmt run <name>`. A fresh start is always explicit.

Provider hooks (`tmt setup`) record the session, runtime mode and reported model
whenever the provider starts a session, and `/clear` makes the new session the
one to resume. A model is kept only when the provider reports it. `run` alone
records nothing, and a launch under a different runtime drops the previous
runtime's session. Retiring an identity clears its remembered session.

A resume that exits with an error before the provider confirms the session
marks it stale. This happens only when the provider's TMT hooks are installed,
so a confirmation would have been seen. A Ctrl-C or other signal exit never
marks it stale. A stale session is not resumed again until you retry it, forget
it, or start fresh. If a remembered session's driver is no longer registered,
the next resume forgets it and says so. Do not combine `--resume` with an
explicit command.

While a session is remembered, `tmt identity show --json` and `tmt ls --json`
include a `resume` object with the driver, mode, session, model and `staleAtMs`.

The command inherits the terminal and foreground job control. Ctrl-C reaches the
command; Ctrl-Z suspends it together with TMT, and `fg` resumes both. TMT returns
the command's exit code (128 plus the signal number for signal termination) and
records its exit without unbinding the pane. If TMT itself is killed with SIGKILL,
it cannot guarantee child cleanup; an otherwise live child with a missing launch
owner is Unknown, not a verified delivery destination. Harness-created background
processes remain the harness's responsibility.

### Shell completion

Load completion in the current shell, or add the corresponding command to your
own shell configuration:

```bash
# bash
source <(tmt completion bash)
# zsh, after compinit
source <(tmt completion zsh)
# fish
tmt completion fish | source
```

Identity completion is storage-only: it does not inspect tmux or create a
database. Saved identities appear before temporary ones; `resume` and
`run --resume` offer identities with a remembered session. After the identity, completion belongs to
the selected command and uses that shell's installed command completions. TMT
does not install provider completion scripts. Bash integrates with bash-completion
when it is loaded and can also use already registered function completions.

## Saved identity notes

Each saved identity can own one ordinary local Markdown file:

```bash
tmt notes path --identity coordinator
tmt notes path --identity coordinator --json
```

Omit `--identity` only in a verified pane bound to a saved identity. Outside
tmux, or when caller evidence is unavailable, select an existing saved identity
explicitly. Temporary identities return `NOTES_SAVED_IDENTITY_REQUIRED`; an
unknown or retired name returns `NAME_NOT_FOUND`.

The first successful invocation creates an empty `notes.md` at
`<global-state>/notes/<identity-uuid>/notes.md`; plain output is only that
absolute path. JSON returns `identityId`, `path`, and `created`. Directories and
the file are created owner-only on supported Unix platforms. Later invocations
preserve the file's exact bytes and do not refresh, truncate, template, lock, or
watch it. Edit it with normal filesystem tools and coordinate concurrent writers
as you would for any other file.

The path follows the saved identity UUID, not its display name, pane, current
directory, role, or Office state. Retiring an identity retains its notebook; a
new identity that reuses the name receives a new UUID and path. TMT does not
garbage-collect old notebooks. This is local filesystem discovery for the same
OS user, not authentication, isolation, encryption, or a shared remote notebook.

## Optional Office guidance

Office is not required for identity or pane collaboration. After explicit
consent, `tmt office install --yes` installs the independently versioned
companion and the optional `tmt-office` and `tmt-prop-create` skills. Core
`tmt install` continues to manage only `tmux-team` and `tmt-inbox`.

```bash
tmt office
tmt office status --json
tmt office install --yes
tmt learn --skill tmt-office
tmt learn --skill tmt-prop-create
```

Bare `tmt office` inspects the installed companion and local service. It does
not install, start, pair or open a browser. Explicit `tmt office start` returns
a local browser URL; open it on the same machine and treat its access token as
private. The link belongs to that service start. A later start may return a new
link. Published Office support depends on the installed CLI/companion pair, so
check their reported versions before relying on source-only features.

After a real identity creation or local Office start, human commands may print
one short next-step hint on stderr, including when output is redirected. A
repeat that changes nothing does not repeat the hint. Set `TMT_HINTS=off` to
disable optional discovery. JSON and raw stdout remain unchanged; error
recovery guidance and terminal-only managed-skill drift still appear when
applicable.

Office setup uses the same provider roots and includes custom roots that still
contain an owned core skill. It preserves unmanaged `tmt-office` or
`tmt-prop-create` paths unless the user explicitly repeats install or upgrade with
`--force`; replacements are backed up outside the discovery root. Companion
activation and guidance publication are separate outcomes, so a reported skill conflict can leave the
verified companion installed. Office uninstall retains managed guidance,
release files, and application data.

Reload the agent before using the new skills. See
[`docs/office/commands.md`](docs/office/commands.md) for the human command
contract; the installed skill owns agent safety, selective notes, decoration,
pairing, and board behavior.

## Extension hooks

Extensions found on `PATH` run only when you invoke them. To let an extension
react to identity and room changes, enable its hooks explicitly:

```sh
tmt extension hooks enable office   # trust tmt-office on PATH
tmt extension hooks list
tmt extension hooks disable office
```

Enabling requires an executable you own that no one else can modify, and records
exactly that file; after an upgrade or any change, enable it again. Hooks receive
only UUIDs and states, never names or messages, and cannot block or change a
command. For Office, enabling hooks keeps retired identities and rooms marked
without waiting for the next Office command. An extension that offers context
adds one labelled, informational line to `tmt whoami --context` and to the
context agents receive; Office describes the identity's desk.

## Optional squad extension

Squad organizes agents into squads: a lead that dispatches work and keeps the
board current, and members that do the work. It adds no daemon and no store. A
squad is the TMT room `squad-<name>`, and member fields are identity metadata
`squad.<name>.<field>`. It is not distributed yet: build it from source and put
it on PATH together with its `tmt-sq` alias link.

```bash
(cd rust && cargo build --locked -p tmt-squad)
ln -s "$PWD/rust/target/debug/tmt-squad" ~/.local/bin/tmt-squad
ln -s tmt-squad ~/.local/bin/tmt-sq
```

```bash
tmt squad init product --me <your saved identity>   # room squad-product
tmt squad lead sol                                  # a saved identity
tmt squad add auth-fix docs-sweep                   # agents already running
tmt squad set auth-fix state=blocked pending="approve the plan" note="needs a call"
tmt squad status                                    # --json for scripts
tmt squad remove auth-fix                           # the agent keeps running
tmt squad help set                                  # or `set -h`: help with examples, for every command
```

`me` (your saved identity) is recorded in `squad.toml`, next to TMT's global
`config.json`, with its UUID as `me_id`; the first interactive `init` asks for
it, and non-interactive use requires `--me`. Re-running `init` changes nothing.
When you rename your identity (`tmt rename`), squad follows it: at once if you
enabled its hooks (`tmt extension hooks enable squad`), otherwise on the next
command that acts as you. The UUID decides who you are, so editing `me` by hand
to another identity only prints a warning; change who you are with `tmt squad
init <squad> --me <name>`. With one squad, commands
select it; with several, pass `--squad <name>`. `status` lists members, one per
row: a leading mark (◆ when the member waits on you with `pending`, otherwise
● active, ◌ unverified or ○ offline), the name, the state, and what you need to
know first (what it waits on you for, its note, your open annotation). `set
field=` clears a field. `remove` clears only that squad's fields.

`status` shows one list unless you define sections in `squad.toml`. Each section
has a title, an optional filter and optional sort keys; a member appears in every
section whose filter it matches:

```toml
[[squad.product.section]]
title  = "Needs me"
filter = "pending or state = blocked"   # and, or, not, (), =, !=; quote values with spaces

[[squad.product.section]]
title = "Everyone"
sort  = ["state", "-name"]              # "-" sorts descending; state follows the layout
```

Filters compare text fields of a row: `name`, `presence`, `lifetime`,
`activity` and every squad field such as `state`, `pending`, `note` or
`pr_link`. A bare field name means "present and non-empty".

`tmt squad board` opens the terminal board: squad tabs (←/→), one searchable
list (`/`), the ◆ rows that wait on you first in the crew layout, and each
member's note under its row. It refreshes in the background every few seconds
and re-reads `squad.toml`, so edits apply on the next refresh; `q` or Esc
closes it. Without a terminal, or with `--json`, it prints `status`. Columns
and state colors are configurable:

```toml
[squad.product.columns]
show = ["member", "state", "task", "pr_link"]   # member is the name
task = { width = 32, title = "WORK" }

[squad.product.states]
blocked = { color = "red", sort = 0 }   # colors: default, dim, red, amber, green,
                                        # cyan, blue, magenta; sort 0-999 orders states
```

The board is made of panes: `rows`, `notes` (the lead's own notebook, the same
file as `tmt notes`, read-only), `detail` (the selected row) and `replies`
(answers to what you sent the squad). Choose them and how they sit:

```toml
[squad.product.board]
mode      = "split"            # split or tabs
direction = "left-right"       # or top-bottom
panes     = ["rows", "notes"]  # also detail, replies; rows is required
sizes     = [60, 40]           # split only: one percentage per pane, total 100
```

Tab moves between panes (or tabs); ↑/↓ scroll the notes pane when it has focus.
In tabs mode the lead's notes always get their own tab. Notes render as light
Markdown: headings, lists, bold, italic, inline code and links (shown as text);
tables, HTML, images, code blocks and quotes appear as written. Terminal
escapes, control characters and hidden bidi/format characters are removed
first. Set `[squad.<name>.notes] render = "plain"` to show the text unformatted. The crew
layout shows rows and notes side by side, pr-queue shows rows over detail, and
minimal shows rows only.

Keys act on the selected row. Inside tmux, Enter jumps to the member's pane and
Backspace goes back; in a plain terminal, where the board cannot show another
pane, Enter opens a menu of the row's actions instead. `o` opens the row's link,
`y` copies it, `t` talks to the member, `r` replies to it, `a` annotates the
row for the lead, `n` focuses the notes pane and Tab moves to the next pane;
`?` lists every key. Rebind keys in `squad.toml`, for all squads or for one section's rows:

```toml
[bind]                               # over the host preset, for every squad
enter = "open {pr_link}"
f5    = "refresh"
y     = "copy - [{name}]({pr_link})"

[[squad.product.section]]
title = "Needs me"
filter = "pending"
[squad.product.section.bind]         # over [bind], for this section's rows only
enter = "copy {name}: {pending}"
```

A binding is `event = "action [argument]"`. Events are `enter`, `backspace`,
`tab`, `space`, `delete`, `home`, `end`, `pageup`, `pagedown`, `f1`–`f12`,
`ctrl-<letter>` (except `ctrl-c`), `click`, `double-click` or one printable
character other than the board's own `q`, `j`, `k`, `/` and `?`. Actions are
`jump`, `back`, `open [{field}]`, `copy [template]`, `run <program> [arguments]`,
`notes`, `refresh`, `next-pane`, `menu`, `talk`, `reply` and
`annotate [lead|member]`. An unknown action, event or field syntax makes the
board report the configuration error; a field that is empty for the selected
row refuses the action with a notice and runs nothing.

A click selects the row under the pointer, and a double-click runs Enter's
action; bind `click` to act on a single click. While the board is open it
captures the mouse, so select terminal text with your terminal's override
(usually Shift or Option while dragging).

`run` starts a program for the selected row without a shell, with no terminal
input or output, in its own process group so it outlives the board:

```toml
[bind]
e = "run code --reuse-window -- {cwd}"
```

The program is a name on `PATH` or an absolute path, written literally. Each
argument is split once when `squad.toml` loads (double quotes group words), and
a `{field}` value fills exactly one argument however it is spelled, so it never
becomes several words or shell syntax. A value can still begin with `-`; when
the program accepts it, put `--` before field arguments, as above, so such a
value is read as a file or name rather than an option.

Some row actions also work as commands, for scripts, tmux key bindings and
terminals without the board:

```sh
tmt squad jump auth-fix                     # show its pane in your tmux client
tmt squad back                              # return to where the last jump came from
tmt squad open auth-fix                     # pr_link, else link, else another *_link
tmt squad open auth-fix --link issue_link
tmt squad copy auth-fix                     # "auth-fix: <task> (<state>)"
tmt squad copy auth-fix --format '- [{name}]({pr_link})'
tmt squad annotate auth-fix "split this job"   # to the lead; --to member for the member
```

Talking and answering are TMT's own commands, not Squad's: `tmt talk <member>
"…" --detach` sends, and `tmt answer <member> "…"` answers what a member is
waiting on you for (see [Inbox and answer](REQUEST-RESPONSE.md#inbox-and-answer)).
The old `tmt squad talk`, `reply` and `replies` now refuse and name these
commands. On the board, `t`, `r` and `a` open a one-line composer: Enter sends,
Esc cancels, and empty text sends nothing. They send as your saved identity
(`me` in `squad.toml`) and never wait. `t` is a detached `tmt talk` in the
squad's room, so its answer shows on the board. `r` is `tmt answer`: with
several open requests you choose one (the board lists them, oldest first), and
none is assumed. Answering acknowledges nothing. `annotate` sends `[<squad> · <member>] <text>` to the lead
(or, with `member`, to the member) and never edits anyone's notes; until it is
answered, the row shows `✎ sent to <name>: <text>`, rebuilt from request history
on every refresh. `status --json` reports it as each row's `annotation` and the
member's open requests to you as `waitingOnYou`, from `tmt inbox`: a request
stays there until it has a final or its answer deadline passes. Squad reads at
most 200 requests for each; when more exist it says `older requests not
shown`.

The `replies` pane lists the answers to your talks and annotations in the
squad, newest first: who answered, how long ago, what you asked and the reply.
It reads the squad room's request history, so an answer stays after you
acknowledge it. The newest eight show their text, with terminal escapes removed
and at most six lines; older ones point to `tmt result <request-id>`, which
prints the whole reply exactly. Reading replies acknowledges nothing, so
`tmt x list` still shows them until you acknowledge them there.

`jump` is `tmt focus` for a squad member or the lead, run inside tmux. Each
jump, from the board or the command, records where your tmux client came from;
`back` (or Backspace on the board) returns that client there. Run `back` from
where your client is now: the member's pane, a new popup, or a key binding such
as `bind B run-shell "tmt squad back"`. A board left behind by its own jump no
longer shows your client, so its Backspace cannot return it. With nothing
recorded, `back` says so and changes nothing. The record is disposable, kept per
tmux server and client under `$XDG_CACHE_HOME/tmt-squad` (or
`~/.cache/tmt-squad`), at most 32 entries.

In tmux, hotkeys open the board: `prefix S` as a popup that closes when you
jump, and `prefix B` as a pane beside the current one that stays open. They are
added to your tmux configuration only with your OK:

```sh
tmt squad hotkeys install --print   # what would be written; changes nothing
tmt squad hotkeys install           # shows the plan, asks, then installs
tmt squad hotkeys show              # installed? which keys? is tmt still there?
tmt squad hotkeys remove            # takes out only squad's line and keys
```

The bindings live in `squad.tmux.conf` beside `squad.toml`, which squad
regenerates. Your tmux configuration gets one line, `source-file -q
'<…>/squad.tmux.conf' # tmt squad hotkeys`, in the first of `~/.tmux.conf`,
`$XDG_CONFIG_HOME/tmux/tmux.conf` or `~/.config/tmux/tmux.conf` that exists
(`~/.tmux.conf` is created if none does; `--config <path>` picks another).
If that file is a link, as dotfile managers make it, squad edits the file it
points to and keeps the link; a link to a missing file is refused.
Before writing, install rereads the file, keeps a byte-exact backup beside it
(`<name>.tmt-squad-backup-<time>`) and replaces it in one step; running it
again changes nothing. Inside tmux it also loads the bindings into the running
server, and `remove` unbinds only keys still bound to squad's commands. Without
a terminal to ask on, pass `--yes`. If a chosen key is already bound, in the
running server or your configuration, install lists it and changes nothing;
choose other keys in `squad.toml`:

```toml
[tmux]
popup = "S"      # the defaults; a single key, C-x, M-x or F1-F12
pane  = "B"
back  = "b"      # optional: prefix b runs `tmt squad back`
```

The bindings run the `tmt` found on your PATH (for example `~/.local/bin/tmt`),
not a versioned release path, so upgrades keep them working; `show` says when
that command no longer exists. Squad needs tmux 3.2 or later.

`open` opens only http and https links, with `open` on macOS and `xdg-open`
elsewhere. `--format` fills `{field}` placeholders from the row:
`name`, `state`, `pending`, `note`, `presence`, `lifetime`, `activity`, `pane`,
`target`, `cwd` or any squad field. An empty or missing field refuses the action
instead of copying a gap. Inside tmux, `copy` loads a buffer on your tmux server
(`tmux load-buffer -w`, tmux 3.2 or later), which tmux passes on to your
terminal's clipboard; when the server's `set-clipboard` is `off`, the text stays
a tmux buffer and the message says so. Outside tmux, the text goes to the
terminal as OSC 52, which some terminals must be told to allow. A configured
program replaces either route. It runs directly, never through a shell: the
opener gets the link as its last argument and the clipboard program gets the
text on stdin.

```toml
opener    = ["firefox", "--new-tab"]    # top level of squad.toml
clipboard = ["pbcopy"]                  # or ["wl-copy"], ["xclip", "-selection", "clipboard"]
```

The lead's skill ships with the extension. Publish it into your agents' skill
folders with `tmt extension install squad --skills` (an interactive install
asks); updates keep it current and `tmt extension uninstall squad` removes it.
`tmt sq skill show` prints the same skill.

Playbooks are optional guidance your lead agent can follow to lay a squad out on a
host. The first, `tmux-squad`, suggests a `leads` session, a `crew` session with one
window and git worktree per member, and the board on its hotkey. Squad never runs a
playbook; the agent proposes the commands and you decide. A playbook is a skill that
is not installed with the extension:

```sh
tmt squad playbook list                       # names and descriptions
tmt squad playbook show tmux-squad            # the exact text, nothing installed
tmt squad playbook install tmux-squad --print # the plan; changes nothing
tmt squad playbook install tmux-squad         # shows the plan, asks, then publishes
tmt squad playbook remove tmux-squad          # removes only this playbook's skill
```

Installing publishes the skill into your agents' skill directories through TMT's
managed skill installation, owned by `squad`, and only with your OK (`--yes` when
there is no terminal). A skill of that name that TMT does not manage is never
replaced: the install is refused unless you pass `--force`, which keeps a backup.
Removing takes out only the playbook; the `tmt-squad` skill stays, and a copy you
replaced or edited is kept.

## Talk and receive a complete reply

Send a request by global name or direct pane target:

```bash
tmt talk reviewer "Review this patch and report concrete risks."
tmt talk %12 "Run the focused checks." --timeout 300
```

The receiver must have the skill loaded. TMT gives it a request ID and receipt
inside the delivered instructions; the receiver submits one complete final
reply with that receipt. `talk` waits for the durable final by default.

An ended or unbound identified recipient stays queued in Inbox, without pane
input. A verified pane without runtime hooks retains legacy delivery; agent
readiness is unverified, and tmux cannot detect provider approval prompts.

For work that should continue after the caller returns:

```bash
tmt talk reviewer "Run the agreed checks." --detach --json
tmt result <request-id> --json
```

Use exactly the request ID and receipt supplied by TMT. Do not invent a receipt,
select the latest request, or infer a pane. A successful submission means a
body was stored; it does not prove that the requested work succeeded.

Agents can submit a final explicitly when TMT supplies the receipt:

```bash
tmt reply <request-id> --receipt <receipt> --message 'Review complete.'
tmt reply <request-id> --receipt <receipt> --file response.md
tmt reply <request-id> --receipt <receipt> --stdin < response.md
```

Choose exactly one input source. Identical retries are safe while the body is
retained; a different body conflicts and cannot replace the stored final.

To answer requests addressed to you without a receipt, for example as a person
at a shell, list what is waiting and answer by the sender's name:

```bash
tmt inbox --identity ben
tmt answer reviewer "Yes, ship it." --identity ben
```

`tmt inbox` lists each open request with its sender, age, request ID and first
line, oldest first. A request stays there until it has a final or its
acceptance deadline passes; acknowledging it does not remove it. When the
sender has several open requests, `tmt answer` sends nothing and lists them;
choose one with `--request <request-id>`. Inside your own bound pane,
`--identity` can be omitted. The [contract](REQUEST-RESPONSE.md#inbox-and-answer)
defines selection, errors and JSON.

## Attribute and recover requests

When the caller is not in a verified bound pane, select an existing durable
identity explicitly. The option belongs after `talk`:

```bash
tmt talk reviewer "Review the release notes." --identity coordinator --detach --json
```

This is local attribution, not authentication. An offline recipient keeps the
request in Inbox; TMT reports it immediately and never pastes into an ended
session or automatically sends again when it returns. Live delivery keeps the
usual sent/completed output and does not leave duplicate incoming attention.
Detached or interrupted callers may receive a one-line reply hint at their
current verified binding; a live blocking waiter gets only the full reply.
Notification failure never invalidates a stored final. Use `tmt result <id>`
from a hint, rather than re-sending. Explicit `--inbox` remains queue-only.
To recover requests after timeout, detach, process restart, or
pane loss:

```bash
tmt x --identity coordinator --json
tmt x show <request-id> --identity coordinator --json
tmt x ack <request-id> --revision <revision> --identity coordinator --json
tmt x ackall --identity coordinator --json
```

Bare `x` means `x list`: it returns unacknowledged retained metadata. `x show`
reads the retained original prompt and final when available. `ack` requires the
revision observed by list/show; `ackall` acknowledges the current transaction
snapshot without enumerating or claiming that every body was read. Reads and
acknowledgements do not cancel work or renew retention.

## Roles and preambles

An optional role profile is stored with an identity and is not automatically
injected into messages:

```bash
tmt role set "Review correctness before style." --identity reviewer
tmt role show --identity reviewer
tmt role clear --identity reviewer
```

Preambles are separate and are included in messages for the selected identity:

```bash
tmt preamble set reviewer "Be concise and cite concrete evidence."
tmt preamble show reviewer
tmt preamble clear reviewer
```

Use notes for deliberate working context, `role` for durable profile data, and
`preamble` for message context. All three survive pane loss and rebinding.
Explicit names work outside tmux; unknown names are not created implicitly.

## Important delivery behavior

TMT is CLI-only. Each command exits after its operation, and direct pane routing
on the current tmux server remains the primary delivery path when the recipient
is reachable. For explicit asynchronous local delivery,
`tmt talk <identity> "message" --inbox` queues to an existing non-retired
identity in local SQLite. The recipient can run
`tmt x listen --identity <name>` with bounded `--timeout` and `--debounce`
values, then inspect with
`tmt x show <request-id> --incoming --identity <name>`. This is not a background
listener or daemon, and it adds no remote service, cross-machine routing, MCP
transport, or Office dependency. Durable identities and retained request/reply
bodies remain local.

Line breaks are preserved. ASCII `!` is converted to fullwidth `！` to protect
coding-agent shell/bash shortcuts, so code such as `if (!ready)` is not delivered
byte-for-byte. If tmux input may have reached the pane and TMT reports
`DELIVERY_UNCERTAIN`, inspect the pane before deciding whether to retry; a
timeout or missing visible output is not proof that nothing ran.

For command grammar and edge cases, use `tmt help`. For the complete durable
request/response contract, see
[`REQUEST-RESPONSE.md`](REQUEST-RESPONSE.md). For provider-specific skill
installation, custom skill roots, and retiring an older Claude integration, see
[`skills/README.md`](skills/README.md).

## Configuration and troubleshooting

If `STORAGE_NOT_WRITABLE` names the TMT data directory, an agent sandbox may
be denying SQLite or WAL access. Allow that directory or use the provider's
escalation for the authorized command. An existing data directory without owner
write permission is reported, not repaired. An identical `reply` retry is safe when
the error says nothing was stored; keep its request ID, receipt and body
unchanged. `TMUX_PERMISSION_DENIED` means the sandbox may instead be blocking
the tmux socket. Do not delete storage or change tmux identity evidence to work
around either error. A user may optionally add the data directory to Codex
`writable_roots`; an agent should not change that setting without consent.

Inspect resolved settings before changing them:

```bash
tmt config show --json
tmt config set pasteEnterDelayMs 500
tmt config set preambleEvery 3
tmt config set exchange.retentionDays 90 --global
```

| Setting                  | Default  | Scope                                                |
| ------------------------ | -------- | ---------------------------------------------------- |
| `preambleMode`           | `always` | Local override or `--global`; `always` / `disabled`  |
| `preambleEvery`          | `3`      | Local override or `--global`; `0` disables injection |
| `pasteEnterDelayMs`      | `500`    | Local override or `--global`; `0` removes the delay  |
| `exchange.retentionDays` | `90`     | Global only; new requests, integer days `1..3650`    |
| `ui.paneBadge`           | `off`    | Global only; `on` / `off`                            |

Human `config show` identifies each value's actual source, accepted values and
whether the setting is CLI-editable locally/globally or global-file-only.
`defaults.timeout`, `defaults.pollInterval` and `defaults.captureLines` are
global-file-only; `config set` and `config clear` do not edit them. Numeric CLI
writes use unsigned decimal integer tokens. `config show --json` retains the
resolved values, sources and actual file paths.
Global settings normally live in `~/.config/tmux-team/config.json`; local
overrides live in `./tmux-team.json`. Use the reported paths when a custom home
or configuration root is in use. Global-only settings cannot be set or cleared
locally. Unknown fields are preserved; invalid known fields should be repaired,
not worked around by deleting the file.

### Optional pane badge

TMT never changes `pane_title`, `pane-border-format`, border position, or theme options.
The badge is **off by default**. To opt in:

```bash
tmt config set ui.paneBadge on --global
tmt name alice
```

This publishes a label in the pane-local `@tmux-team.badge` option:

- Recorded running session: `● alice (tmt)`, with a green dot.
- Recorded ended session: `○ alice (tmt)`, with the whole badge dimmed.
- Unknown session: `alice (tmt)`, with no dot or added styling.

The dot reports recorded session state, not activity, readiness or permission to
send input. Without a lifecycle event, a stale observation can remain visible;
the badge never polls or controls routing.
It does not display anything until you explicitly insert this fragment at the
desired position in your own tmux `pane-border-format`:

```text
#{?@tmux-team.badge, [#{@tmux-team.badge}],}
```

For a theme showing the pane number on the left and `repo/branch` on the right,
place the fragment after the pane number, before your right-aligned segment.
Keep the existing expressions and styles; do not replace the whole theme with
this fragment. The result is conceptually:

```text
---10.0 [alice (tmt)]----------------------------repo/branch---
```

Your theme controls alignment, background and narrow-pane behavior. TMT does not
reserve space or move the existing right-hand segment. To hide the badge below
80 columns, use:

```text
#{?#{&&:#{@tmux-team.badge},#{e|>=:#{pane_width},80}}, [#{@tmux-team.badge}],}
```

Adjust the width threshold for your theme; it is not an automatic fit calculation.
Running and ended labels use `push-default`/`default`/`pop-default` to restore
the surrounding colors and attributes. Do not wrap the badge in another
`push-default`: tmux only supports one saved default, not nested style stacks.
Place it outside any such span and use the theme's explicit restoration after it.
See the [tmux styles reference](https://man.openbsd.org/tmux#STYLES).

Display labels replace
`#` and control characters with non-executable text and truncate names after
48 Unicode code points; the stored identity name remains unchanged.

Configuration changes do not scan or rewrite panes. They apply on the next
successful binding, recorded launch/exit, provider lifecycle hook or recovered
session for that pane. Cosmetic failures leave durable state unchanged. To disable the
current badge:

```bash
tmt config set ui.paneBadge off --global
tmt this alice
```

`unbind` also clears the badge, even when it is disabled. Failed bindings leave
it unchanged. Badge writes are bounded and best-effort: display failures do not
undo a successful identity change. If an older TMT version already replaced
your title or border format, restore it from your saved tmux configuration;
TMT cannot reconstruct an overwritten theme.

### Installation troubleshooting

If native `tmt` is not found after installation, put `~/.local/bin` (or your
selected prefix's `bin`) first in PATH, then open a new shell or run `hash -r`.
Check `command -v tmt`, `command -v tmux-team` and the new absolute `tmt --help`;
an older npm command may still shadow the native installation. Use a user-owned
prefix instead of adding `sudo` blindly. See [npm/pnpm replacement](docs/NATIVE-INSTALL.md#replacing-npm-or-pnpm)
before switching package managers or touching their files.

If an integration path conflicts with unmanaged files, inspect the
target first; `tmt install <provider> --force` creates a recoverable backup
outside the skills root and reports its path.
After changing skills or provider setup, reload or restart the agent session.
