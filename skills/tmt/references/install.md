# TMT reference: Installation, updates and completion

Part of the tmt skill, printed by `tmt learn --skill tmt install`.
The rules of the skill entry (`tmt learn --skill`) still apply.

### Shell completion

`tmt completion` (see `tmt completion --help` for the startup files and zsh
placement) warns about duplicate lines and is textual configured-state evidence.
`--json` reports the shell, installed state, file and line. Public `completion`
always guides or checks, even when piped, and never edits startup files.
Top-level terminal help offers a tip when the detected shell is not configured;
piped help and inspection errors suppress the tip.
Completion offers saved-first identity candidates and command-owned completion
after `run`'s name. Discovery does not probe tmux or create storage. Provider
completion scripts still require separate installation.

### Skill installation

Install the same native skill with `tmt install` (auto-detects supported agents).
Claude uses `<CLAUDE_CONFIG_DIR>/skills` (otherwise `~/.claude/skills`); Codex, Gemini, OpenCode and Muse share
`~/.agents/skills`. Antigravity CLI (`agy`) uses
`~/.gemini/config/skills`; Pi uses `~/.pi/agent/skills`
(or `<PI_CODING_AGENT_DIR>/skills` when configured). Each selected root receives
sibling `tmt` and `tmt-inbox` skills.
All targets link the same bundled content. No plugin or separate command wrapper is needed.
Installation is non-interactive; `--json` is supported. With no detected provider,
the shared target is installed and its result omits `agent`. This does not install
an agent application. An existing `.agents` directory alone is not provider evidence.
Claude's native skill can be invoked as `/tmt`. Core bundled and verified official
release publication replaces any existing entry at a catalog skill name in the selected roots, without a prompt, flag,
backup or content check. Other names and symlink destinations are untouched.
Plugin settings are never modified.
Native `tmt upgrade` refreshes recorded managed skills. For a manual binary
replacement, run `tmt install` again. Reload or restart the agent afterwards.
For an existing conversation, run `tmt learn --skill` and read its complete output
before using remembered commands. Pi can load `/skill:tmt`; OpenCode uses
its `skill` tool. Installation does not bypass provider permissions or guarantee
that a running conversation has refreshed its instructions.

### View and install the bundled skill

`tmt learn --skill` prints the exact bundled universal skill; plain `tmt learn`
shows the guide. Both are text-only. Install default integrations with
`tmt install [AGENT]`, or choose a skills root explicitly:

```bash
tmt install --dir ./my-skills
```

This links sibling `./my-skills/tmt` and `./my-skills/tmt-inbox`; do not
also specify a provider. Choose a
folder your provider actually discovers and reload its skills if needed.
Managed links follow bundled updates at the same package path. Re-run the same
install command to refresh the target after relocation; existing entries at the
published skill names are replaced.
Automatic drift reminders inspect known default paths, not custom folders.
They do not reload an active agent, update provider-managed plugins, or track
alpha release channels. Package upgrades and skill installation are separate
from provider discovery.

### Native installation and updates

Native release installers use a versioned `tmt-installer.sh` URL supplied by the
release. Use the repository README or an actual published release; never invent a URL or
use npm `upgrade` as a native migration. The shell bootstrap defaults to
`~/.local/bin/tmt`, supports `--prefix`, `--pin` and `--no-skill`, and otherwise
runs the new absolute command's skill installer. Verify `command -v tmt` and
the new absolute `tmt --help`; never share native state with the retired
TypeScript runtime. Default Core data moves once as one directory; explicit
`TMT_HOME` stays exact. No package-manager installation is uninstalled.
If the selected installation is uncertain, check `tmt --help`; never fall back
to TypeScript against native state or delete data as a migration workaround.
A skill publication problem after a successful binary upgrade is a warning;
the binary stays active. Inspect the reported path and cause before retrying.
Read this skill again through the new executable before using remembered syntax.

The installer does not edit shell profiles or change the parent shell's PATH.
If `tmt` is missing, use the installed absolute path and complete one-time shell
PATH setup; do not repeatedly append exports on every upgrade. If another
installation is selected, inspect its owner before changing or removing it.

The selected Rust executable embeds this exact skill plus the focused
`tmt-inbox` skill; viewing and installation
work after moving the binary, without Node or a checkout. Native installs link
an immutable digest-addressed source under TMT's global directory
(`skill-assets/<sha256>/{tmt,tmt-inbox}`). Re-run the intended `install` command after
replacing a manual binary to refresh valid managed links. An edited current
bundled source blocks installation even with force; inspect it before repair.
Replacing links to older sources never changes the source content. Old source
directories remain available for recovery, not automatic cache deletion.

Publication across providers is not all-or-nothing; a warning retains completed
skill targets. Custom target intents are recorded in `skill-installations.json`
for future refresh at those published names. Refresh alone creates no new provider scope.
Native passive reminders cover known defaults only during interactive human
commands; JSON and piped automation do not perform that scan.

For a managed native installation, `tmt upgrade` (alias `tmt update`) retains the
receipt's channel (`--channel`, `--to` and `--unpin` are in `tmt upgrade --help`);
do not combine `--to` and `--unpin`. Downgrades are rejected. Pinned products stay
pinned while upgrade can offer other components. Package-manager or unmanaged binaries refuse native updates;
use their original manager, not an overwrite workaround.

Upgrade also offers missing ops, digest, remote and colab with their bundled skills,
using setup's inventory and the upgrading CLI's prefix. Colab visibly needs Remote.
A human terminal can select missing items and approve the complete plan once;
decline/EOF changes nothing, and deselections are not remembered. User-authorized
`--yes` installs missing official extensions and bundled skills even with JSON or
no terminal. Without it, machine runs report missing items and exact commands.
New extensions use the displayed alpha release without a pin; installed channels
and pins are retained. Damaged or unmanaged entries require their recovery command.

Completed items stay installed after an independent failure. Rerunning retries
unfinished work. Ctrl-C reaches the apply child, which stops at checkpoints,
keeps completed outcomes and returns 130. A second Ctrl-C shortens the wait;
forced cleanup or a deadline leaves an unconfirmed outcome. Inspect before retrying.
JSON adds
`providerSetup` outcomes (`changed`, `consentRequired`, `needsApproval`, `skipped`
or `failed`) beside existing `integrations` and per-product results. A target
that cannot apply the new plan reports its exact follow-up; no apply is replayed.

Successful native updates use the new executable to refresh existing recorded
skill targets, replacing entries at the published names. With `--json`, inspect
`changed`, `version`, `pinnedVersion`, each product's activation result and
`skills.status`; skill problems use `skills.warning` with the path and cause,
while partial publication remains in the skill report. They do not fail an
otherwise successful binary update. Fix the cause and repeat the original update
selection (including `--to <version>` when pinned); an ordinary pinned invocation
will not retry skill work. Check `pathWarning` before assuming the
shell selects the updated binary. Reload/restart the agent, or read the complete
`tmt learn --skill` output in an existing conversation.

`tmt storage` (or `--json`) shows what each local store occupies: the data root,
`tmt.db` with its free pages and auto-vacuum mode, notes, and Colab `space.db`
with its backups. It changes nothing. When it reports the one-time `tmt.db`
compaction as pending, `tmt upgrade` finishes it; the upgrade reports the
outcome as `databaseCompaction` and never fails because of it.

### Installing a missing official extension

A person in an interactive terminal can accept a single installation question.
The verified installer includes bundled agent skills and shows download,
verification and activation progress. Metadata preview failure shows an unknown
version and size and still allows the question. Declining changes nothing.
After successful installation, TMT prints the original command to run again;
it never runs that command. Root help shows which official extensions are
missing; completion continues to list only installed commands.

Office is no longer shipped.
