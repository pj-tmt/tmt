# Agent skill installation

Install the native alpha using the [README](../README.md) command, then run
`tmt install`. Use the installer asset from a published release. No plugin, marketplace
or separate slash-command package is required. The native executable embeds the
canonical [tmt skill](tmt/SKILL.md), focused
[tmt-inbox skill](tmt-inbox/SKILL.md), optional
[tmt-office skill](../extensions/tmt-office/skills/tmt-office/SKILL.md), and optional
[tmt-prop-create skill](../extensions/tmt-office/skills/tmt-prop-create/SKILL.md) and
[tmt-avatar-create skill](../extensions/tmt-office/skills/tmt-avatar-create/SKILL.md) in one versioned bundle. Core
install exposes only the first two; explicit Office setup manages all three Office skills.

The native runtime needs no Node.js, Rust toolchain or source checkout; tmux is
still required for pane operations. Native bindings are temporary by default:
use `-s`/`--save` to keep one, and `tmt rm <name>` to retire a temporary identity
(`--force` is required for a saved identity). Native schema migrations are forward-only;
do not use the legacy TypeScript runtime on a native database.

## Install

```bash
tmt install          # Detect installed providers
tmt install claude   # Or select one explicitly
tmt install codex
tmt install gemini
tmt install agy
tmt install pi
tmt install opencode
tmt install all      # Install for every supported provider
```

| Provider                   | Native skill root          |
| -------------------------- | -------------------------- |
| Claude Code                | `~/.claude/skills/`        |
| Codex, Gemini and OpenCode | `~/.agents/skills/`        |
| Antigravity CLI (`agy`)    | `~/.gemini/config/skills/` |
| Pi                         | `~/.pi/agent/skills/`      |

Each root receives sibling `tmt/SKILL.md` and `tmt-inbox/SKILL.md` links.

Installation is non-interactive and accepts `--json`. If no provider is detected,
the shared `~/.agents/skills/{tmt,tmt-inbox}` targets are installed without
claiming a provider was found; each JSON item has `skill`, `target` and `changed`,
but no `agent`.
This does not install the agent applications themselves. Explicit selectors work
even before the selected provider is installed.

Pi honors `PI_CODING_AGENT_DIR`: its targets are sibling directories under
`<agent-dir>/skills`.
OpenCode's configuration directory is used for detection, including
`OPENCODE_CONFIG_DIR` or `XDG_CONFIG_HOME`, but its installed skill remains in the shared home location.
Use `--dir` for a different skill discovery root; TMT does not edit provider settings.

Each skill directory is a managed link to one immutable native asset bundle.
Repeating installation is a no-op when both links are correct. Native
`tmt upgrade`/`tmt update` refreshes recorded managed skills through the newly
activated executable; use `--channel stable|alpha`, `--to <version>`, or
`--unpin` as needed. A skill refresh can fail after binary activation and is
reported as a warning with its path, cause and partial skill publication; the
binary upgrade stays successful. Fix the cause and repeat the same selection.

The legacy TypeScript `tmt upgrade` follows npm `latest` and cannot update a
native installation. Use the original manager for package-manager installations.

Load `tmt` in your agent before pane collaboration and `tmt-inbox` for an
authorized bounded inbox-processing session. Claude Code's canonical skill
can be invoked as `/tmt`; the CLI remains `tmt`. Installing files does not
guarantee an already-running agent has reloaded them. Use its skill discovery
or restart the session when necessary. The [Claude skill documentation](https://code.claude.com/docs/en/skills)
describes its native personal skill location and invocation.

Pi exposes `/skill:tmt`; OpenCode loads `tmt` through its `skill`
tool. Antigravity discovers skill metadata when starting a conversation; ask it
to load `tmt` or explicitly read `tmt learn --skill`. Provider permissions
or disabled skill discovery can still prevent loading. Installing a link is not
proof that a running session has loaded its content.

Paths follow the [Antigravity skill documentation](https://www.agy.dev/docs/skills/),
[Pi documentation](https://pi.dev/docs/latest/skills), and
[OpenCode documentation](https://opencode.ai/docs/skills).
Pi's native path also supports the locally verified 0.85.0 loader, which does not
discover the shared `.agents` path by default. Older Antigravity documentation
lists different directories; use a current CLI or explicitly select its actual
discovery root rather than installing multiple competing copies.

## Inspect or choose a folder

```bash
tmt learn --skill
tmt install --dir './project skills'
```

`learn --skill` prints the exact core skill; `learn --skill tmt-inbox`,
`learn --skill tmt-office`, `learn --skill tmt-prop-create`, and
`learn --skill tmt-avatar-create` select the other
exact embedded sources. Plain `learn` is a short guide.
Custom installation creates sibling `./project skills/tmt` and
`./project skills/tmt-inbox` links relative to the current directory. Choose a
folder your provider discovers, and do not combine `--dir`
with a provider or `all`. Custom installs do not migrate default paths or touch
unrelated siblings. Automatic drift reminders cover default locations, not
arbitrary custom folders.

## Optional Office guidance

Office is frozen: installation and upgrade refuse before acquisition. Existing
Office skills remain available for viewing and consented removal; native core
refresh skips names held by an extension owner. Publication of optional bundled
Office guidance uses the same name replacement contract as core guidance.

```bash
tmt learn --skill tmt-office
tmt learn --skill tmt-prop-create
tmt learn --skill tmt-avatar-create
tmt office install --yes
tmt office upgrade
```

Custom and provider-specific targets remain managed by the same immutable asset,
registry, drift, backup, and lock owner. There are no provider-specific Office
copies or command wrappers.

## Existing installations

Update the native CLI using a published release's installer asset, run
`tmt install` if needed, then reload or restart the agent. For an existing conversation,
ask the agent to run `tmt learn --skill`, read the complete output, and use it
instead of remembered instructions from an older version.

Replacing package-manager or manual installations does not delete their files.
Verify `command -v tmt` and the new absolute `tmt --help`. The one-shot default
data-directory cutover is owned by Core configuration; explicit `TMT_HOME` stays exact.

Core bundled and verified official release publication replaces any existing
entry at a catalog skill name in selected roots, with no prompt, flag, backup or prior ownership/content check.
Other names and symlink destinations remain untouched; immutable sources are
validated before publication. A former Core skill target is retired only after
its complete managed source digest and inventory verify and the replacement is
published. Modified or unmanaged former targets remain conflicts. Do not delete
the source package or your identity database to repair a skill link.

The old Claude `~/.claude/commands/team.md` entry is no longer installed or
updated. `tmt install claude` preserves an existing entry and warns; after the
native skill is installed successfully, `tmt install claude --force` can move
that old entry to a recoverable backup. Other commands are untouched. Local
drift checks also report retired command entries, including broken links.

## Verify

```bash
tmt --version
tmt learn --skill
tmt install claude --json   # A correct existing link reports changed: false
```

Use [Working](https://pj-tmt.github.io/tmt/working) for the first live exchange,
recovery and roles, and [Settings](https://pj-tmt.github.io/tmt/working/settings) for configuration.
