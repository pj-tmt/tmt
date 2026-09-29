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
| `accent`               | blue         | running, `hint:`, row actions         |
| `ok`                   | green        | success (`✓`)                         |
| `warn`                 | yellow       | needs attention                       |
| `error`                | red          | `error:`, failed                      |
| `dim`                  | dim          | counts, times, offline, secondary     |
| `title`                | bold         | section titles and help headings      |
| `literal`              | bold         | commands and flags a reader types     |
| driver `claude`        | magenta      | an address or name driven by Claude   |
| driver `codex`         | cyan         | an address or name driven by Codex    |
| any other driver       | dim          | including the `tmux:%N` transport     |

Help uses the same tokens through clap `Styles`. A full-screen view, such as the
Squad board, takes its colors from `Token::color`, not from its own palette.

## Marks

Each mark has one meaning everywhere (`mark::Mark`). A row's leading state mark is
`●`, `○` or `◌`:

| Mark | Meaning                          |
| ---- | -------------------------------- |
| `●`  | running or active                |
| `○`  | offline or ended                 |
| `◌`  | bound to a pane, no agent running |
| `↻`  | leads a resume action (`↻ tmt resume <name>`), never a row's state |
| `✓`  | done                             |
| `✗`  | failed                           |
| `!`  | warning                          |

## Lists

This is the list model that `tmt ls` (#434) follows first; other lists use the
same parts.

- Agent-first: each row starts with a state mark, then the name.
- A section is an UPPERCASE bold title followed by a dimmed count. Rows are
  sorted by name within a section.
- Rows are indented two spaces, with no header row and no borders. Sections with
  the same columns share one layout, so their rows line up.
- A row's trailing action appears only where an action is possible, such as
  `↻ tmt resume <name>`, `stale` or `shell`. It is accent-colored, comes after
  every column, and is never truncated.
- A section-level `hint:` line comes last, only for a next step that applies to
  the whole section.

```text
SAVED 3
  ●  astra            codex:019a2f4c   ~/dev/tmux-team
  ●  opus-tmt-peer-2  claude:7c41e9d2  ~/dev/tmux-team/worktrees/feature-branch
  ○  sol              claude:3f9a1c07  ~/dev/tmux-team                           ↻ tmt resume sol

TEMPORARY 2
  ●  mamezu-astra     codex:01a9c3b8   ~/dev/mosaic-art
  ◌  opus-1           tmux:%31         /srv/builds/nightly                       shell
hint: tmt ls --all shows offline identities
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
