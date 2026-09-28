# tmux-team user guide

This guide covers the common v5 native alpha workflows. Start with the
[README](README.md) for the current installation status and verified release
URL, then use
[`skills/README.md`](skills/README.md) for provider-specific installation and
[`skills/tmux-team/SKILL.md`](skills/tmux-team/SKILL.md) for canonical agent
guidance. Optional Office workflows have canonical
[`tmt-office`](skills/tmt-office/SKILL.md) and
[`tmt-prop-create`](skills/tmt-prop-create/SKILL.md) skills.

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
(`--force` is required for a saved identity). Switching from npm or pnpm is a
fresh installation: stop old writers first; no configuration, database or
historical exchange is migrated or deleted. Native schema migrations are forward-only,
so never use the old TypeScript writer on a native database.

## Connect agent lifecycle hooks

`tmt setup` is read-only: it shows detected providers and Claude/Codex hook status.
Run `tmt setup claude` or `tmt setup codex` to review the exact settings and launcher
paths and approve one plan. Noninteractive use requires `--yes`; add `--json` for a
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

Use these commands to inspect the current tmux server:

```bash
tmt list
tmt list reviewer
tmt whoami
```

Human tables align columns using Unicode display widths and show complete values.
Long rows may wrap in narrow terminals. Control characters in table metadata are
shown as escapes, not executed. Use `--json` for scripts and exact metadata;
human spacing is presentation, not a machine-readable format.

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

`identity show <name>` reads that exact stored identity without tmux. Inside a
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

## Launch a command with an identity

Inside a tmux pane, use `run` to bind an identity and start a foreground command:

```bash
tmt run reviewer claude --model sonnet
tmt run -s coordinator codex
tmt run coordinator
tmt run --resume coordinator
```

TMT options go before the name. Everything after it is the command and its exact
arguments; no `--` separator is needed. TMT does not insert a provider session ID
or store arguments, model choices, secrets or executable paths. A new identity is
temporary unless `-s`/`--save` is supplied; an existing saved identity stays saved.
The ordinary binding conflict rules still apply.
The executable word cannot start with `-`: `tmt run Alice -s` is rejected before
binding. Use `tmt run -s Alice <command>`; flags after a real command stay exact.

Registered runtime drivers recognize their executables and remember only the
harness ID. Bare `run <name>` resolves that driver's executable through PATH and
passes no arguments. Without a remembered harness, specify a command. An
unrecognized command still runs with the same binding and lifetime tracking,
without replacing a previously remembered harness.

`--resume` uses an exact remembered provider session and runtime mode, not a
provider's "last session" shortcut. Session capture belongs to provider hooks;
`run` alone does not capture it or inject initial context. Without a supported
remembered session, it reports that fact and starts the remembered harness bare.
This fallback occurs only before launch. A failed resume process is never
automatically replaced by a fresh session. Do not combine `--resume` with an
explicit command.

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
database. Saved identities appear before temporary ones; `run --resume` offers
identities with a remembered session. After the identity, completion belongs to
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

TMT never changes `pane_title`, `pane-border-format`, border position, or colors.
The badge is **off by default**. To opt in:

```bash
tmt config set ui.paneBadge on --global
tmt name alice
```

This publishes `alice (tmt)` in the pane-local `@tmux-team.badge` option.
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

Your theme controls color, alignment, and narrow-pane behavior. TMT does not
reserve space or move the existing right-hand segment. For black text on a light
blue background, hidden below 80 columns, an optional fragment is:

```text
#{?#{&&:#{@tmux-team.badge},#{e|>=:#{pane_width},80}},#[push-default]#[fg=black bg=colour153] #{@tmux-team.badge} #[default]#[pop-default],}
```

Adjust the width threshold for your theme; it is not an automatic fit calculation.
The style save/restore assumes your surrounding theme does not already use
`push-default`: tmux only supports one saved default, not nested style stacks.
If it does, integrate the colors using that theme's own restoration mechanism.
See the [tmux styles reference](https://man.openbsd.org/tmux#STYLES).

Display labels replace
`#` and control characters with non-executable text and truncate names after
48 Unicode code points; the stored identity name remains unchanged.

Configuration changes do not scan or rewrite panes. They apply on the next
successful `name`, `this`, `add`, or `marked` for that pane. To disable the
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
