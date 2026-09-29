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
  next step. The error code belongs to `--json`, and exit codes are unchanged.
  A multi-line message keeps its further lines, such as a usage block.
- Warning: `warning: <what>` on stderr for a non-fatal problem, when the command
  still did its work, optionally followed by `hint:`.
- Line messages use these labels, lowercase everywhere; marks are for list rows.
- One-line messages drop a single final period. The stored message, and
  therefore `--json`, keeps it.

## Help

Every command is built from a `CommandSpec`: summary, examples, output modes
(`Human`, `Json`, `HumanAndJson`, where the last adds `--json`) and optional
details. `tmt_cli_style::command` builds a whole command; `apply` puts the same
help on a command whose CLI parses help and `--json` itself, as core does.
Registration panics without a summary, or with fewer than one or more than three
examples.

- Sections, in order: summary, `Usage`, `Commands`, `Arguments`, `Options` (the
  order clap renders them), any discovered sections (such as the root's
  `Extensions`), an optional `Details`, then `Examples`.
- `Details` (`CommandSpec::details`) holds safety and boundary facts that must
  be visible in help, such as which identities a command accepts or what it
  never creates. It sits just before `Examples`. It is rare by design: it is
  never a place for a longer description.
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
- Commands write through `stream::stdout(json)` and `stream::stderr()`. Each
  returns a locked `Stream` that implements `Write` and carries its decision
  (`Stream::terminal`), so a renderer gets both from one place:

  ```rust
  let mut out = tmt_cli_style::stream::stdout(mode.json);
  let terminal = out.terminal();
  section.write(&mut out, terminal)?;
  ```

- On a terminal whose width is known, rows never wrap. Detail columns (paths,
  previews) are truncated with `…` first, then names. Marks and fixed columns
  never truncate. Piped output is never truncated.
- Control and line-separator characters in user data are shown escaped
  (`table::escape`). This is a trust boundary: user data never reaches the terminal
  as control sequences.

## Enforcement

Two tests enforce this document. Each keeps a migration list of what does not
follow it yet. A list must equal what still fails: a command or file that now
follows the style fails the test until its entry is removed, and anything new
that breaks a rule fails at once. Migrating a command means deleting its
entries. Both lists are empty when #436 closes.

- **Grammar walk** (`tmt_cli_style::audit`). For every visible command,
  extension trees included, it checks that the command:
  - has a summary;
  - prints the same text for `-h`, `--help` and `help <command>`;
  - follows the section order above;
  - has one to three examples.

  It reads the examples back from the help a user sees (`help::examples`, the
  inverse of what `command` writes). Each example must invoke its own command
  and parse through the CLI's real parser without running. Core's walk and its
  list are in `rust/crates/tmt-cli/src/cli_style_{tests,allowlist}.rs`;
  Squad's are the same files in `extensions/tmt-squad/rust/tmt-squad/src/`.

- **Output guard** (the architecture test). In `tmt-cli`, `tmt-office-command`
  and `tmt-squad`, production code may not:
  - call `print!`, `println!`, `eprint!` or `eprintln!`;
  - reach `std::io::stdout` or `std::io::stderr` in any form, including an
    import;
  - write an escape character in a literal.

  It writes through `stream` instead. A function whose output is an exact byte
  stream or a terminal protocol (`tmt api`, `tmt learn`, provider hooks, the
  Squad board) is exempt by name, with a reason. The rest of its file is still
  checked. The lists are in
  `rust/crates/tmt-cli/tests/architecture/output_allowlist.rs`.

## Migrating a command

Each step leaves the command's tests passing; delete the command's entries from
the migration lists in the same change.

1. **Help.** Build the command from a `CommandSpec`: a one-line summary and one
   to three examples, the common use first. A CLI that parses help itself (core)
   calls `apply` on its own `Command`; any other CLI calls `command`, which also
   adds `-h`/`--help` and `--json`. Each example is the full command a user
   types. The grammar walk parses it through the real parser, so run the walk
   until the command leaves its help list.
2. **Streams.** Replace `io::stdout()`/`io::stderr()` and print macros with
   `stream::stdout(json)` and `stream::stderr()`. Pass `stream.terminal()` to
   every renderer.
3. **Output.** Write outcomes with `message::success`, `message::error` and
   `message::hint`, lists with `list::Section`, and values with `value`. Output
   that must be an exact byte stream (a stored response, a script, a protocol)
   moves into a function of its own and becomes an exact-body exemption with a
   reason.
4. **JSON proof.** `--json` never changes. Capture the command's `--json` output
   before and after the change in the same sandbox, and show the byte-equal
   comparison in the pull request.
