# TMT native alpha installation

The verified public pair is TMT CLI `5.0.0-alpha.6` with optional TMT Office
`0.1.0-alpha.3`. Download the current immutable CLI installer from the
[README][public-install]. After installing the CLI, opt into Office with:

```sh
tmt office install --yes --channel alpha
tmt office start
```

For an existing public alpha installation, update the CLI and its complete core
skill set before updating an installed Office companion:

```sh
tmt upgrade --channel alpha
tmt install
tmt office upgrade --channel alpha
```

Use `tmt office install --yes --channel alpha` instead of the last command when
Office is not installed. Installation and upgrade do not start Office; run
`tmt office start` explicitly. Reload or restart existing agent conversations
after `tmt install` or an Office install/upgrade. The old public CLI alpha.4 does
not recognize `tmt office stop`, so it is not an update prerequisite. A development
build already running the local service must be stopped with that same old build
before switching writers.

For an optional local `tmt-office` archive, use an installed native CLI:
`tmt office install --yes --archive <archive.tar.gz> --manifest <dist-manifest.json>`.
Then run `tmt office status`. Add the same `--prefix <folder>` to both commands
for a custom installation. Office is independently versioned. The public
companion and source builds support compatibility probes, explicit pairing and
renewable scoped access. Installation alone does not pair, open a world or start a
service. The public companion includes `tmt office start` and `stop` for a
loopback-only embedded local UI.
The public Office install and upgrade commands also manage the optional
`tmt-office`, `tmt-prop-create` and `tmt-avatar-create` agent skills through the
CLI's existing provider and custom-root registry. Core installation remains
independent, and the hidden binary-only
`__native-install --product office` boundary does not install skills.
CLI and Office share forward-only state migrations, so the compatible CLI must be
upgraded first. The four-file Office archive inventory is unchanged because generated
SPA assets are compiled into `tmt-office`, not installed as mutable files.
The public alpha companion is acquired through the CLI's verified immutable
release path. No hosted Office service or production Firebase deployment is
provided.

The remaining instructions apply to the core `tmt` archive.

This archive contains the standalone Rust native alpha runtime. It needs no
Node.js, npm, pnpm, Rust toolchain or source checkout to run. tmux and socket
access are required for binding, sending and capturing live panes, not for
storage-only identity/profile/reply/result/X operations with explicit selection
where required. Direct pane delivery still requires a reachable live pane, but
explicit local `talk --inbox` delivery and bounded `x listen` do not; they use
the same local SQLite state without a daemon, remote service, MCP, or Office.
Use the installer asset from a published release; the README
supplies the verified version URL when one is available.

Native schema migrations are forward-only: the TypeScript runtime cannot reopen native state.
Installing or replacing a native binary does not migrate, delete or downgrade
application data. Stop older TMT writers before switching, and do not run the
native runtime against a database that the TypeScript runtime still uses.

After extracting a verified archive into a chosen directory:

```sh
tmt_preview_root=$(mktemp -d)
./tmt --version
TMUX_TEAM_HOME="$tmt_preview_root" ./tmt learn --skill
TMUX_TEAM_HOME="$tmt_preview_root" ./tmt install --dir "$tmt_preview_root/skills" --json
```

Skill installation is non-interactive and does not install agent applications.
The example keeps its managed files in a temporary directory; for normal use,
choose a directory your provider discovers and reload its skills. Native pane
bindings are temporary by default: use `-s`/`--save` with `name` or `add` to
preserve one, and use `rm <name>` to retire a temporary identity (`--force` is
required for a saved identity). Retain the included LICENSE and
THIRD-PARTY-NOTICES.txt with redistributed binaries.

Managed binary receipts, `upgrade`/`update`, and release-generated shell bootstrap
are implemented by the native runtime. Do not overwrite an npm, pnpm, Homebrew
or manual installation.
The selected executable's help is the capability authority; the shared alpha
version number alone does not distinguish native and TypeScript runtimes.

For a verified managed native installation, `tmt upgrade` retains its channel;
`tmt upgrade --channel alpha` selects alpha, `--to 5.0.0-alpha.6` pins an exact
version, and `--unpin` resumes channel updates. `tmt update` is the same command.
Downgrades are rejected. An ordinary pinned invocation does not access the
network. Use `--json` for a single structured result. The update refreshes only
previously managed skills through the new binary; missing integrations stay
missing and user modifications are preserved. Reload the agent after changes.
A skill failure can occur after binary activation and is reported as partial
completion, not rollback; resolve reported conflicts and repeat the original
selection, including `--to <version>` when pinned. Ordinary pinned updates do
not retry skill work.

The distribution manifest supplies archive names, target triples and SHA-256
checksums. Checksums detect corruption, not compromise of the download origin.
No local test artifact is authenticated by a published GitHub attestation.
For a published immutable release, `gh release verify <tag> --repo
wkh237/tmt` verifies GitHub's release attestation; `gh release verify-asset
<tag> <downloaded-file> --repo wkh237/tmt` also checks a local asset.
This optional independent check requires GitHub CLI, not the installed TMT
runtime. Do not describe locally generated checksums as signatures.

Release targets are macOS x64/arm64 (build deployment target 11.0) and Linux
x64/arm64 with a static musl runtime. Refer to the release's verification evidence
for tested host OS versions; a deployment target is not testing on every OS.

## Official extensions

`tmt extension` installs, updates and removes the official extensions (`office`
and `squad`) through the same verified native installer as the CLI:

```sh
tmt extension install squad --yes --channel alpha
tmt extension list            # local state only; add --check to look for updates
tmt extension upgrade squad --yes
tmt extension uninstall squad --yes
```

Install, upgrade and uninstall change your installation, so they ask first, or
take `--yes`. A non-interactive run without `--yes` changes nothing. `install`
also accepts `--archive <archive.tar.gz> --manifest <dist-manifest.json>` for a
local artifact, and every command takes `--prefix <folder>` for a custom
installation. `upgrade` takes `--to <version>` to pin an exact version and
`--unpin` to follow the channel again. Squad installs two commands, `tmt-squad`
and its short form `tmt-sq`. Each extension has its own version and release tags
(`tmt-office-v<version>`, `tmt-squad-v<version>`); Squad's releases are still
alpha, hence `--channel alpha`.

`tmt extension list` never uses the network unless you add `--check`. An
unreachable release then shows as `unknown`. The listing also reports any other
`tmt-<name>` command on your PATH that would shadow the installed one; it never
runs that command.

An extension release may bundle agent skills, such as Squad's `tmt-squad` lead
skill. `install --skills` publishes them into your agents' skill folders. An
interactive install without `--skills` asks first; `--yes` alone installs the
extension and prints the skills it offers and how to publish them. Updates keep
published skills current and remove any the new release no longer ships;
skills you did not publish stay unpublished. A folder you manage yourself at a
skill's name is never replaced: publishing reports it and the extension stays
installed.

`uninstall` removes the extension's command links and every agent skill the
extension installed, from every agent's skill folder; the prompt lists them. It
keeps the retained releases, its lifecycle hook consent and, for Office,
`office.db` and its backups, and prints where each is and how to remove it
yourself. For Office, `tmt office install` still manages its skills.

An extension release may bundle agent skills. If a newer `tmt` installed such a
release and you then downgrade the CLI, the older `tmt` reports "Installed release
inventory has changed" for that extension. It refuses to inspect, update or
remove it, and it changes nothing. Upgrade `tmt` back to fix this.

## Curl bootstrap

The one-line install in the [README][public-install] pipes the release's
`install.sh` into `sh`. To inspect it first, download it with
`--proto '=https' --proto-redir '=https'`, then run `sh install.sh`; a failed or
truncated download is not installation success.

The script fixes the initial version and channel, not a mutable alpha tag.
Later native `tmt upgrade` follows that channel. Check the installed absolute
command afterwards.

On a terminal, the installer ends by running `tmt setup` with the new `tmt`
first on PATH; setup reads its one approval from the terminal. Without a
terminal, in CI, or with `--no-setup`, it runs the non-interactive
`tmt install` (skills only) instead and asks you to run `tmt setup` later.

Default prefix: `$HOME/.local`, with commands in `~/.local/bin`. Options after
`sh -s --` (or the downloaded script):

- `--prefix /absolute/directory`: another prefix, including paths with spaces.
- `--pin`: pin this release; `tmt upgrade --unpin` resumes channel updates.
- `--no-setup`: skip guided setup; skills are installed non-interactively.
- `--no-skill`: binary only: neither setup nor skills. No provider application
  is ever installed.

Requires POSIX shell, curl, tar/gzip, standard filesystem utilities and
`sha256sum` or `shasum`. No Node, Rust, jq or sudo. The script verifies manifest
and archive sizes/digests before executing the temporary binary, then delegates
permanent writes to the native installer. It does not edit shell profiles or
touch SQLite. Initial receipts record local-archive verification, not independent
attestation provenance. HTTPS and hashes do not protect a compromised origin.

## One-time PATH setup

If `command -v tmt` already selects `~/.local/bin/tmt`, no setup is needed.
Otherwise, for Bash or Zsh, add this block **once** to the startup file you
actually use (`~/.bashrc` or `~/.zshrc` for interactive shells):

```sh
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) export PATH="$HOME/.local/bin:$PATH" ;;
esac
```

Open a new terminal, then check `command -v tmt` and `tmt --version`.
The block avoids adding the directory again when a shell inherits it. Do not
append it on every installation/update or blindly edit both startup files.
Other shells use their own persistent PATH configuration. For a custom prefix,
substitute its absolute `bin` directory.

Running `export` in a terminal changes only that shell; `curl ... | sh` cannot
change its parent's PATH. The installer therefore reports path problems and
does not modify shell profiles. You can immediately run `~/.local/bin/tmt`
directly without changing PATH. If PATH already contains that directory but
another installation wins, follow the replacement guidance below rather than
repeatedly adding directories.

## Uninstall

```sh
tmt uninstall
```

This lists what it will remove and asks before changing anything; pass `--yes`
to approve without a prompt. It first stops a running local Office service,
then removes, in this order:

1. the lifecycle hooks `tmt setup` added to agent settings files;
2. TMT's skill links;
3. official extensions;
4. the `tmt` command and its releases in the prefix.

Anything that no longer matches what TMT wrote (an edited hook, a replaced
skill, another program's `tmt` command) is kept and reported, as are the
backups of your agent settings files. Your identities,
messages and notes stay in TMT's data directory
(`~/.config/tmux-team` by default). `tmt uninstall --purge` deletes that
directory too; `--yes` alone never does.

If a step fails, fix the cause and run `tmt uninstall` again: completed steps
are not repeated. Use `--prefix <folder>` for an installation in another
prefix.

## Replacing npm or pnpm

This is a fresh installation, not a data-transfer or compatibility tool. Stop old
TMT commands/agents before switching. Installation neither migrates nor deletes
old configuration/databases or uninstalls another manager's files.

For an npm installation, use `npm uninstall -g tmux-team`; for pnpm, use
`pnpm remove -g tmux-team`. Do not run both blindly. A collision in the selected
prefix is refused: choose another prefix or remove the old package through its
actual owner. Homebrew/manual installations likewise retain their own lifecycle.

Put the new prefix's `bin` first in PATH, then run `hash -r` or open a new shell.
Check `command -v tmt`, `command -v tmux-team` and the new absolute `tmt --help`.
The installer warns about PATH shadowing and never runs the old command. An
installed binary is not proof your shell selects it; aliases/functions and other
open shells may need separate correction.

Old npm-linked skills may conflict. Inspect them, then explicitly run the new
absolute `tmt install --force` if replacement is intended. The shared installer
makes recoverable backups; bootstrap never silently forces. Skill failure returns
nonzero after binary installation, without claiming rollback. Reload the agent
or ask it to read the complete `tmt learn --skill`. No historical-session
continuity or SQLite downgrade is promised; never use the old TypeScript writer
on a native database.

[public-install]: https://github.com/wkh237/tmt#install
