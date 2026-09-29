# TMT command-line style

This document owns how TMT's command-line output and help look. Every CLI (core
`tmt`, `tmt-squad` and `tmt-office`) follows it. The only implementation is the
`rust/crates/tmt-cli-style` crate. It depends on no TMT crate, so any CLI can
depend on it. Rules are enforced by tests, not by review.

**Adoption status:** the crate implements every rule below. Commands move onto it
under #436. Until a command has migrated, its help and human output may still
differ from this document. `--json` output never changes for style.

## Palette

Colors are semantic tokens (`palette::Token`). Each maps to one of the terminal's
16 palette entries or to an effect, so the user's theme decides the shade. No RGB
or 256-color values are used.

| Token                  | Rendering    | Use                                   |
| ---------------------- | ------------ | ------------------------------------- |
| `accent`               | blue         | `hint:`, resumable state              |
| `ok`                   | green        | success, running                      |
| `warn`                 | yellow       | needs attention                       |
| `error`                | red          | `error:`, failed                      |
| `dim`                  | dim          | counts, times, offline, secondary     |
| `title`                | bold         | section titles and help headings      |
| `literal`              | bold         | commands and flags a reader types     |
| driver `claude`        | magenta      | an address or name driven by Claude   |
| driver `codex`         | cyan         | an address or name driven by Codex    |
| any other driver       | dim          |                                       |

Help uses the same tokens through clap `Styles`. A full-screen view, such as the
Squad board, takes its colors from `Token::color`, not from its own palette.

## Marks

Each mark has one meaning everywhere (`mark::Mark`):

| Mark | Meaning                          |
| ---- | -------------------------------- |
| `●`  | running or active                |
| `○`  | offline                          |
| `◌`  | bound to a pane, no agent running |
| `↻`  | a remembered session can resume  |
| `✓`  | done                             |
| `✗`  | failed                           |
| `!`  | warning                          |

## Lists

- Agent-first: the name leads each row, after its mark.
- A section is an UPPERCASE bold title followed by a dimmed count.
- Rows are indented two spaces, with no header row and no borders. Sections with
  the same columns share one layout, so their rows line up.
- A `hint:` line comes last, only when an action is possible.

```text
RUNNING 2
  ●  coordinator  claude:e1c9ab12  just now  ~/dev/tmux-team/worktrees/feature-branch
  ◌  reviewer     codex:77aa0c3d   3m ago    ~/dev/tmux-team

OFFLINE 1
  ↻  night-shift  claude:0c3d77aa  2h ago    /srv/builds/nightly
hint: tmt resume night-shift
```

## Values

Human output shows readable forms (`value`). `--json` always keeps the full values.

- Paths under the home directory are shown as `~/…`.
- Addresses are `driver:identifier`, with identifiers shortened to 8 characters.
- Times are relative: `just now`, `45s ago`, `3m ago`, `2h ago`, `5d ago`.

## Messages

- Success: `✓ <past-tense verb> <object>`, such as `✓ Named pane %3 worker`.
- Failure: `error: <what>` on stderr, then `hint: <next command>` when there is a
  next step. `error:` and `hint:` are lowercase everywhere. The error code
  belongs to `--json`, and exit codes are unchanged.
- One-line messages drop a single final period. The stored message, and
  therefore `--json`, keeps it.

## Help

Every command is built by `tmt_cli_style::command` from a `CommandSpec`: summary,
examples, output modes (`Human`, `Json`, `HumanAndJson`, where the last adds
`--json`). Registration panics without a summary, or with fewer than one or more
than three examples.

- Sections, in order: summary, `Usage`, `Arguments`, `Options`, `Commands`, any
  discovered sections (such as the root's `Extensions`), then `Examples`.
- `-h`, `--help` and `tmt help <command>` print the same text (`help_text`).
- Each example is a comment line naming what it does, followed by the full
  command. Show the common use first. Examples must parse through the real
  grammar (`Example::argv`), so a renamed flag or missing operand fails a test.

```text
Examples:
  # Send a message to one agent
  tmt talk worker "Run the tests"
```

## Degradation

- There is no color when stdout is not a terminal, when `NO_COLOR` is set or
  `CLICOLOR=0`, or with `--json`. `CLICOLOR_FORCE` forces color. The decision is
  made once per stream (`Terminal::stdout`, `Terminal::stderr`).
- On a terminal whose width is known, rows never wrap. Detail columns (paths,
  previews) are truncated with `…` first, then names. Marks and fixed columns
  never truncate. Piped output is never truncated.
- Control and line-separator characters in user data are shown escaped
  (`table::escape`). This is a trust boundary: user data never reaches the terminal
  as control sequences.
