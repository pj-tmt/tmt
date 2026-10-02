# TMT release archive

This archive holds one TMT product: the `tmt` CLI, or the `tmt-office` or
`tmt-squad` extension. It runs without Node.js, npm, a Rust toolchain or a source
checkout. Keep the included `LICENSE` and `THIRD-PARTY-NOTICES.txt` with any copy
you redistribute.

The README at <https://github.com/pj-tmt/tmt> has the current one-line installer.
The handbook at <https://pj-tmt.github.io/tmt/> is the full user guide.

## Check the download

The release manifest lists archive names, target triples and SHA-256 checksums.
A checksum detects corruption, not a compromised download origin. For a published
release, GitHub CLI can verify the release attestation and a downloaded file:

```sh
gh release verify <tag> --repo pj-tmt/tmt
gh release verify-asset <tag> <downloaded-file> --repo pj-tmt/tmt
```

## Try the CLI without installing it

For the `tmt` archive, extract it into a directory you choose and run it from
there. This example keeps all its state in a temporary directory:

```sh
tmt_preview_root=$(mktemp -d)
./tmt --version
TMUX_TEAM_HOME="$tmt_preview_root" ./tmt learn --skill
TMUX_TEAM_HOME="$tmt_preview_root" ./tmt install --dir "$tmt_preview_root/skills" --json
```

Installing skills never installs agent applications. For normal use, run the
installer from the README: it verifies the release and then runs `tmt setup`.
Installing or replacing a `tmt` binary never migrates, deletes or downgrades your
data. State migrations only move forward, so stop older TMT commands before you
switch.

## Install an extension archive

An installed `tmt` installs an extension from a local archive and its manifest:

```sh
tmt extension install squad --yes --archive <archive.tar.gz> --manifest <dist-manifest.json>
tmt extension ls
```

Use `office` instead of `squad` for the Office companion. Both commands accept
`--prefix <folder>` for a custom installation. Install the compatible `tmt` first:
an older CLI may refuse state that a newer extension has upgraded.

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

Open a new terminal, then check `command -v tmt` and `tmt --version`. The block
avoids adding the directory again when a shell inherits it. Other shells use their
own persistent PATH configuration; for a custom prefix, substitute its absolute
`bin` directory. You can also run `~/.local/bin/tmt` directly without changing
PATH. The installer reports PATH problems and never edits shell profiles.
