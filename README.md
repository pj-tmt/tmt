# tmt

Formerly tmux-team; the project has grown beyond tmux.

Your coding agents, working together. Send work to a tmux pane by name, get
complete replies, and recover outstanding tasks without digging through terminal
history. A standalone native CLI—no Node.js, Rust toolchain, or daemon required.

## Install

Native alpha for macOS and Linux, arm64 and x64. No Node, npm, pnpm or Rust
toolchain needed.

[Download the installer](https://github.com/wkh237/tmt/releases/download/v5.0.0-alpha.6/tmt-installer.sh),
then run it from the download folder:

```sh
sh tmt-installer.sh
```

Installs into `~/.local/bin` and sets up the agent skill non-interactively.
If `tmt` is not found, complete the [one-time PATH setup](docs/NATIVE-INSTALL.md#one-time-path-setup).
Reload your agent's skills. For an update, run `tmt upgrade --channel alpha`
followed by `tmt install`, then reload the agent; no binary reinstall or repeated
PATH setup is needed.

Prefer curl, a custom location, or replacing an older installation? See
[installation options](docs/NATIVE-INSTALL.md). The installer never uninstalls old
packages or deletes application data.

Office is optional. After installing the CLI, install and start the independently
versioned local companion with:

```sh
tmt office install --yes --channel alpha
tmt office start
```

Open the printed URL to see your office. Choose **Enter room**, then **Arrange** to furnish an
agent's space, then **Save layout**. No layout file or Firebase setup is needed.

For an existing Office installation, update the CLI first as above, then run
`tmt office upgrade --channel alpha` and start Office explicitly.

tmux is needed for live pane operations: binding, direct pane messaging and
inspection. Explicit local identity/profile access, stored results, and the
opt-in `talk --inbox` queue with bounded `x listen` work without it. Inbox
delivery stores work locally; it does not wake or run an agent, add a background
service, or provide remote transport.

## Quick start

For the receiving agent, start a tmux session if needed:

```bash
tmux new -s tmt
```

Name the target pane from its shell, then launch your agent:

```bash
tmt name reviewer
gemini
```

When the target shell cannot run TMT, mark that pane in tmux and run
`tmt marked reviewer` from a shell connected to the same tmux server. TMT uses
the explicit mark and leaves it unchanged.

From another terminal or tmux pane on the same machine (the sender need not be
inside tmux):

```bash
tmt talk reviewer "Review the current changes and report concrete risks."
```

The receiving agent's skill explains how to submit its complete reply. `talk`
waits for that stored reply, not terminal markers or idle output. Use
`--detach` to return immediately and `tmt result <request-id>` to collect it later.

Names are global, not folder-scoped. Pane identities are temporary by default;
use `tmt name reviewer -s` to save one. `tmt ls` shows lifetime and presence.
`tmt rm reviewer` removes a temporary identity; saved removal requires `--force`.
Neither removal nor unbinding kills the pane.

## Recover outstanding work

A saved originator identity lets you recover requests after timeout, detach or
pane loss, including when the caller is outside tmux:

```bash
tmt identity create coordinator
tmt talk reviewer "Run the agreed checks." --identity coordinator --detach --json
tmt x --identity coordinator --json
tmt x show <request-id> --identity coordinator --json
tmt x ackall --identity coordinator --json
```

Reads never acknowledge results. `ackall` marks the current snapshot handled;
a later reply appears again. Acknowledgment neither cancels work nor asserts success.

## Keep local notes for a saved identity

Initialize and print an identity-owned Markdown path with:

```bash
tmt notes path --identity coordinator
```

Inside a verified saved-identity pane, `--identity` may be omitted. The file is
kept under TMT's local state by identity UUID, so pane loss, renaming a tmux
window, or working offline does not move it. Temporary identities are rejected.
TMT creates an empty private file on first use and never overwrites later edits.

## Skills, no plugin

`tmt install` supports Claude Code, Codex, Gemini, agy, Pi and OpenCode. If none
is detected, it installs the shared core skills. Use `tmt install --dir <skills-root>`
for another discovery folder. No plugin or separate `/team` command is needed;
Claude Code can invoke the skill as `/tmux-team`.

For an existing conversation, ask the agent to read `tmt learn --skill` after
updating. Installation does not reload a running agent. See the
[provider guide](skills/README.md) and
[canonical collaboration skill](skills/tmux-team/SKILL.md). Core installation
manages the `tmux-team` and `tmt-inbox` skills. Explicit Office setup separately
manages the optional [`tmt-office`](extensions/tmt-office/skills/tmt-office/SKILL.md) and
[`tmt-prop-create`](extensions/tmt-office/skills/tmt-prop-create/SKILL.md) skills, plus
[`tmt-avatar-create`](extensions/tmt-office/skills/tmt-avatar-create/SKILL.md); core installation does
not add them.

## Boundaries worth knowing

Direct delivery routes to live panes on the current tmux server and remains the
primary path when the recipient is reachable. For explicit asynchronous local
delivery, `tmt talk <identity> "message" --inbox` queues to an existing
non-retired identity in local SQLite; the recipient uses bounded `tmt x listen`
and `tmt x show --incoming` commands to discover and inspect it. This does not
add a daemon, remote routing, shared memory, authentication, MCP connectivity,
or an Office dependency.

Your tmux titles and border layout stay untouched. Badges are off by default;
see [optional pane badges](USER-GUIDE.md#optional-pane-badge). Inspect settings
with `tmt config show --json`.

ASCII `!` becomes fullwidth `！` in delivered messages to protect coding-agent
shell shortcuts. If delivery becomes uncertain, inspect before retrying; a
timeout alone is not permission to resend.

See the [user guide](USER-GUIDE.md) for notes, roles, preambles, configuration
and failure handling, or `tmt help` for command options.

## Development

Office is an optional, independently versioned public alpha companion. It provides
pairing and remote agent block decoration with scoped, renewable access, plus an
offline local Office through `tmt office start`. The local UI is loopback-only,
uses the same SQLite repository as one-shot `--local` commands and needs no Node or
Firebase at runtime; no hosted Office service is provided. See
[Office commands](docs/office/commands.md); ordinary TMT use does not require it.

Contributor-only requirements and checks are in [development](DEVELOPMENT.md).
See [architecture](ARCHITECTURE.md) for runtime and test ownership.

## License

MIT
