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

| Token            | Rendering | Use                                   |
| ---------------- | --------- | ------------------------------------- |
| `accent`         | blue      | running, `hint:`, row actions         |
| `ok`             | green     | success (`✓`)                         |
| `warn`           | yellow    | needs attention                       |
| `error`          | red       | `error:`, failed                      |
| `dim`            | SGR dim   | counts, times, offline, secondary     |
| `title`          | bold      | section titles and help headings      |
| `literal`        | bold      | commands and flags a reader types     |
| driver `claude`  | magenta   | `review`: an address driven by Claude |
| driver `codex`   | cyan      | `link`: an address driven by Codex    |
| any other driver | dim       | including the `tmux:%N` transport     |

The `dim` token uses the SGR dim effect with the terminal's own foreground:
without a theme, with the `terminal` base, and in every 16-color fallback.
Truecolor themes keep their contrast-tested RGB dim values; explicit overrides
and `mono` retain their configured rendering.

Help uses the same tokens through clap `Styles`. A full-screen view, such as the
Squad board, draws only design tokens, through `theme::screen::style`. This crate
is the only place a color is named: elsewhere, production code writes no
`Color::Red`, `Color::Rgb`, `AnsiColor` or `RgbColor`, and the native
architecture test enforces it for the CLIs and the Rust extensions.

## Themes

`theme` owns what the design tokens (`design/tokens/tokens.json`) look like in
a terminal. Its roles are the tokens: `text`, `muted`, `dim`, `accent`,
`waiting`, `working`, `review`, `blocked`, `link` and `selection` (a
background). A `Theme` is a built-in base plus per-role overrides:

| Base        | Rendering                                                                      |
| ----------- | ------------------------------------------------------------------------------ |
| `auto`      | resolves a supplied terminal background to `tmt` or `tmt-light`; dark fallback |
| `tmt`       | the tokens' dark values in 24-bit color; the default                           |
| `tmt-light` | the tokens' light values in 24-bit color                                       |
| `terminal`  | the tokens' terminal column: the terminal's own 16 colors                      |
| `mono`      | bold (`accent`, `waiting`, `review`, `blocked`) and dim (`muted`, `dim`)       |

`auto` is valid in Squad's `squad.toml` `[board.theme]` and
`[squad.<name>.theme]` layers, including both picker scopes. The global
`config.json` theme rejects `auto` with a board-only hint; CLI colors and defaults
are unchanged. `Base::Auto` is selectable by board callers; `Theme::default()`
remains `tmt`. Callers resolve
an automatic base with `Theme::resolve`, retaining token overrides. Rendering
never reads the environment or queries a terminal. `theme::background` owns
COLORFGBG and OSC 11 interpretation, linear luminance classification, and a
bounded reader with an injected monotonic clock. The caller owns query eligibility,
terminal I/O and received input. Its total OSC budget is 100 ms, including the
write, and at most 256 received bytes; replies at or after the deadline are not
accepted.

The Squad board defaults to `auto`; a concrete configured base wins. It resolves
once before its input and refresh workers start, using COLORFGBG first and then
OSC 11 only for an interactive colored full-screen view. Squad's existing
terminal guard owns the query after entering raw mode. No query runs for lists,
help, JSON, pipes, `TERM=dumb`, `NO_COLOR` or disabled color. Startup input read
during the query (at most 100 ms) is discarded; late OSC replies are filtered
before board actions. An unavailable signal uses `tmt`. Picker previews reuse the
same signal and save the requested `auto` base, without changing CLI defaults.

`tmt sq theme ls` lists `auto` first. With COLORFGBG it shows `auto (tmt-light)`
or `auto (tmt)` and source `detected`. Without a measurement it shows `auto`
with detail `matches the terminal when the board opens`, the configured source,
and JSON `resolvedBase: null`. An auto result retains its configuration layer in
`baseSource`; concrete bases keep their usual source.

An override is `#rrggbb`, a color name (`blue`, `bright black`, …), `default`,
`bold`, `dim` or `reverse`; `Theme::parse` reports a mistake with its setting's
place. A stream renders at one `Depth`, decided once: none (no color, as for
`NO_COLOR`, pipes and `--json`), 16 colors, or 24-bit when `COLORTERM` is
`truecolor` or `24bit`. At 16 colors, `tmt` and `tmt-light` use the terminal
column rather than the nearest shade, and a hex override uses the nearest of the
16 colors. `Token::role` names the design token each command-line token shows
as (`ok` is `working`, `warn` is `waiting`, `error` is `blocked`; a driver token
carries its design token, which the CLI picks from the descriptor's hue). Full-screen views get the same
styles for ratatui from `theme::screen::style`, behind the crate's `ratatui`
feature, so core links no ratatui. A test keeps the built-in values equal to the
design tokens. A contrast test reads the same file and enforces 4.5:1 for
body text and semantic foregrounds, and 3:1 for `muted`/`dim`, on the designed
background, the selection background and representative terminal backgrounds.

The Squad board uses `muted` for inactive tabs, summaries, column headers,
pane titles and footer hints; `dim` remains for borders, empty values, times,
staleness and scroll marks. Focus uses `accent` and bold. Each squad tab and
quick-switcher entry reserves a two-cell leading mark slot: `◆ ` for waiting,
else `✗ ` for blocked, else two spaces. The dominant count follows the name;
both states append blocked `✗n` (`◆ product 2 ✗1`). Only the leading mark
and appended `✗n` use bold configured attention colors. Tab names and primary
counts keep selected accent/bold or inactive muted; the switcher keeps its own
selected-row style. Selection spans the whole tab, including the slot and all
counts. Overflow counters retain their aggregate attention colors. The selected
row uses the `selection` background and keeps its text/state/provider foregrounds. Without a background color
(`terminal`, `mono`, 16 colors or `NO_COLOR`), selection uses reverse video
with one common foreground across the grid row, including empty and wrapped
cells and its age label. Per-cell colors and dim are dropped in that fallback;
state and attention text/marks use bold.
Unselected body text keeps the terminal's default foreground. Selected squad and
pane tabs keep their foreground and width, adding the same selection background
or reverse fallback.

A `Terminal` carries the stream's theme and depth; `paint` and table cells use
`Token::themed`, and a stream without a theme renders exactly the 16-color
output above. The executable sets the process theme once at startup
(`theme::configure`; a second call is a bug), and `Terminal::stdout` and
`Terminal::stderr` apply it to colored streams only. Help keeps the 16-color
styles: clap builds it before any configuration is read. Tests pass a theme in
the `Terminal` they build rather than configuring the process.

## Marks

Each mark has one meaning everywhere (`mark::Mark` for command-line marks; the
fold and meter marks below are board-only). `Mark::description` owns the canonical meaning;
a test checks every shared mark's symbol and description against the design
tokens. Additional marks stay labelled board only. A row's leading state mark is
`●`, `○` or `◌`:

| Mark       | Meaning                                                                                 |
| ---------- | --------------------------------------------------------------------------------------- |
| `●`        | running or active                                                                       |
| `○`        | offline or ended                                                                        |
| `◌`        | bound to a pane, no agent running                                                       |
| `↻`        | leads a resume action (`↻ tmt resume <name>`), never a row's state                      |
| `✓`        | done                                                                                    |
| `◐`        | in review, waiting on someone else (board only)                                         |
| `✗`        | failed or blocked                                                                       |
| `!`        | warning                                                                                 |
| `◆`        | waits on your decision                                                                  |
| `▾`        | an open foldable pane in a toggle hint (board only)                                     |
| `▸`        | folded Squad board pane (board only)                                                    |
| `≥`        | lower bound from missing member token coverage (board only)                             |
| `▁▂▃▄▅▆▇█` | completed-request trend: ▁ measured zero, ▂–█ relative rate, blank no data (board only) |

## Lists

This is the list model that `tmt ls` (#434) follows first; other lists use the
same parts.

- Agent-first: a row with a state starts with its mark, then the name; other
  rows start with the name.
- A section is an UPPERCASE bold title followed by a dimmed count. Rows are
  sorted by name within a section.
- Rows are indented two spaces, with no header row and no borders. Sections with
  the same columns share one layout, so their rows line up.
- A row's trailing action appears only where an action is possible, such as
  `↻ tmt resume <name>`, `stale` or `shell`. It is accent-colored, comes after
  every column, and is never truncated.
- Rows with nothing to show, such as offline identities with nothing to
  resume, may fold into one dimmed note under the section's rows
  (`offline: a · b`); a flag such as `--all` expands them.
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
    offline: gemini-helper · night-owl
hint: tmt ls --all shows offline identities
```

One record is a detail view (`detail::write`): a bold title, then dimmed keys
with full values that are never truncated.

```text
human-exchange
  delivery      sent
  final         retained
  revision      2
```

## Values

Human output shows readable forms (`value`). `--json` always keeps the full values.

- Paths under the home directory are shown as `~/…`.
- Addresses are `driver:identifier`, with identifiers shortened to 8 characters.
- Other identifiers are shortened the same way (`value::short_id`), except
  those a reader types into a follow-up command (request ids, receipts),
  which stay whole.
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

## Command names

Use `ls` for listing, `rm` for removal/reset, `mv` for identity renaming, and
`show` for displaying a record. Root `uninstall` retains its distinct whole-product
meaning; descriptive domain verbs remain when a shell verb would mislead. Old
long spellings stay accepted as hidden aliases: help, docs and examples show the
primary names; completion may offer both. Removal help must state exactly what
is removed or reset and what is retained. The recursive `list_spelling_report`
guard checks `ls` with a hidden `list` alias in every nested listing command.

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
- `-h`, `--help` and `tmt help <command>` print the same text (`help_text`). A CLI
  built with `command` has no `help` subcommand of its own; `route` resolves
  `help <command>` to the command whose help to print (an unknown word is
  reported, and nothing runs). `<command> -h` stays with clap, so an operand
  that is data, such as a message after `--`, is never taken for help.
- Each example is a comment line naming what it does, followed by the full
  command. Show the common use first. Examples must parse through the real
  grammar (`Example::argv`), so a renamed flag or missing operand fails a test.

```text
Examples:
  # Send a message to one agent
  tmt talk worker "Run the tests"
```

## Full-screen interaction

This section owns how a full-screen view, such as the Squad board, behaves:
what has focus, which keys work where, and how overlays, states and narrow
screens look. The `tmt-tui` components implement it (#1465), and the board's
surfaces move onto them. **Status:** the rules are the target for that work. A
surface that has not migrated may still differ; a new surface follows them from
the start.

### Layers and focus

A view is a stack of layers. The **base** is the tab line, the panes and the
footer. An **overlay** (help, settings, a picker, a prompt) sits on top of it,
and a **notice** takes the footer line until the next key. Only one overlay is open at a
time; opening another replaces it.

- The top layer gets every key first. An overlay is modal: it handles its own
  keys and swallows the rest, so nothing reaches the board underneath. The one
  exception is Ctrl-C, which always quits the view. Quitting from a prompt
  discards what was typed there; only Enter saves.
- In the base, one pane has focus. Tab and Shift-Tab move focus between panes in
  reading order; on the Squad home tab they move between sections. A pane may use Tab for its own items, such as links in the notes, and passes it on when it has none or the user bound Tab. The focused
  pane's title is `accent` and bold; other titles are `muted`.
- One cursor per pane. Moving between panes keeps each pane's cursor and scroll
  position. A refresh never moves the cursor or the scroll position; the cursor follows its item, or the nearest one if the item is gone. When content shrinks or the width changes how lines wrap, the scroll position clamps to the new range and keeps the cursor's item in view.

### Keys

The same key means the same thing in every view and overlay.

| Key            | Base                                 | Overlay                         |
| -------------- | ------------------------------------ | ------------------------------- |
| ↑↓, j/k        | move the cursor                      | move or scroll                  |
| PgUp/PgDn      | page                                 | page                            |
| Home/End, g/G  | first or last item                   | first or last line              |
| Enter          | the row's main action (jump)         | confirm or save                 |
| Esc            | clear search or selection, else quit | close without changing anything |
| `?`            | open help                            | close help, or text in a field  |
| `q`            | quit                                 | close, or text in a field       |
| ←→             | previous or next tab                 | not used (edit cursor in input) |
| `/`            | search                               | not used                        |
| Tab, Shift-Tab | next or previous pane                | next or previous field          |
| Ctrl-C         | quit                                 | quit                            |

A view may add its own keys, but never reuses one of these for something else.
User bindings can change a key; help and footers always show the effective key.

### Footer and key help

- The base footer lists the focused pane's most useful keys, as `key word`
  pairs separated by two spaces (`⏎ jump  o open  y copy`). When the width runs
  out, whole hints drop, lowest priority first; a hint is never cut mid-word.
  `? more` and `q quit` always stay.
- An overlay shows its own keys on its last inside line, separated by `·`
  (`↑↓ scroll · PgUp/PgDn page · Esc close`), and the base footer stays as it is.
- Help lists every key in one table: one key column, as wide as the widest key,
  and a description in plain words. A description never shows an internal
  action name (`next-pane`, `token-window`); one map from action to description
  owns the wording, shared by help and the settings view. Names are shown as
  member names, never as IDs. When the description column would be narrower than 20 cells, each key goes on its own line with its description indented below it; no key is ever cut.
- At very small widths the footer keeps `? more` first, then `q quit`, then the rest by priority. `?` closes help; other overlays ignore it unless they list it.

### Overlays

- An overlay is a box with square corners and a single-line `dim` border. Its
  title sits in the top border: the overlay's name in `muted`, then any mode,
  joined by `·` (`settings · Enter edit · * read-only`).
- Nothing from the base shows through: the overlay clears its area first.
- Content is inset one cell from the left and right borders. The key line, a
  status line and the scroll position (`1–23 of 74`, `muted`) share that inset.
- Size: help and other reference overlays fill the whole body between the tab line and the footer, which stay as they are. Small overlays (pickers, confirmations) are as wide as their content, at most 90% of the view and 80% of its height, centered. Below 100 columns every overlay takes the full body width.
  Content that does not fit scrolls; the overlay never grows past the view.
- A prompt is a short overlay docked above the footer, so the board stays
  visible as a live preview of the value being typed.

### Lists and selection

- Selection is the whole row, every wrapped line included, in the `selection`
  background, edge to edge in the pane. Without a background color it is reverse
  video (see Themes).
- Wrapped lines start under the cell's text; a fixed gutter keeps marks and
  text aligned while the cursor moves.
- Text that does not fit ends in `…`; paths and links keep both ends
  (`squad.settin…board.panes`). Keys and marks are never cut.
- Content below the fold shows `N more ↓` in `dim` on the last line.

### Empty, loading and error states

| State   | Looks like                                                    | Role               |
| ------- | ------------------------------------------------------------- | ------------------ |
| empty   | a short phrase in parentheses: `(no notes yet)`               | `muted`            |
| loading | `⠋ loading` on the summary line, only after a moment          | `dim`              |
| partial | the data plus one line saying what is missing and why         | `waiting` with `!` |
| error   | `✗` and one sentence: what failed and what to do              | `blocked`          |
| success | `✓` and a short confirmation, cleared on the next key         | `working`          |
| caution | `!` and one sentence, such as a setting that now stays pinned | `waiting`          |

Messages use the short name the user sees on screen (`stale_after`), never a
full config path or raw syntax such as backticks. A failed provider or a value
the view cannot read shows the empty mark `–`, not an error, unless the user
asked for that value.

### Motion

Only live data and progress move: the token meter, the trend sparkline, the loading spinner and refreshed counts. Nothing blinks, slides or animates for decoration. With
`reduced_motion`, live marks update in place without stepping animations. A
refresh repaints in place and never scrolls or flashes the view.

### Narrow widths

The full layout is designed for 80 columns and wider.

- Columns step aside by priority before anything wraps (see Lists).
- Below 100 columns, side-by-side panes may stack or fold to their title line
  (`▸`); overlays take the full body width.
- The tab line keeps the current tab and the home block visible and folds the
  rest into named overflow (`‹ 2 … +7 ›`).
- Below 60 columns the view shows the focused pane only, with its title.

### Roles by component state

| Component        | Normal             | Focused or selected            | Disabled or empty |
| ---------------- | ------------------ | ------------------------------ | ----------------- |
| pane title       | `muted`            | `accent`, bold                 | `dim`             |
| border           | `dim`              | `dim`                          | `dim`             |
| tab              | `muted`            | `accent`, bold, `selection` bg | `dim`             |
| list row         | `text`             | `selection` background         | `muted`           |
| key in help      | `accent`           | `accent`, `selection` bg       | `dim`             |
| description      | `text`             | `text`, `selection` bg         | `muted`           |
| editable setting | `accent` key       | `selection` background, `›`    | `muted` key, `*`  |
| link             | `link`, underlined | `link`, `selection` bg         | `text`, plain     |
| input value      | `text`, cursor `▏` | —                              | `muted`           |
| scroll position  | `muted`            | —                              | —                 |

Attention marks (`◆`, `✗`) keep their own roles in every state, including selection with a background. Without a background color, selection follows Themes: one common foreground in reverse video, with attention marks in bold.

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

  A `Stream::new` over a buffer is never interactive.

- Whether a person takes part is decided once per invocation by
  `tmt_cli_style::Interaction::detect(json)` and passed to what needs it:
  `view()` for a full-screen view (stdin and stdout are terminals, no `--json`,
  `TERM` is not `dumb`) and `prompt()` for a question on stderr (stdin and
  stderr are terminals, no `--json`), each a `Mode` of `Interactive` or
  `Plain`. `Plain` prints the plain result and never asks: a view falls back to
  its text or JSON output, and a question needs its flag (`--yes`). Commands
  never test a handle themselves.
- A command whose main view is interactive may run as the CLI's bare command
  (`tmt squad`): it opens the view when `view()` is `Interactive` and prints its
  list otherwise, so scripts and pipes get the same list as its `ls`.
- Text that must reach the reader unchanged (a stored response, a prompt,
  captured pane text, a path a script reads) is written through the stream by a
  function of its own that neither styles nor escapes it. Only output that
  cannot go through a stream at all is an exact-body exemption (see
  Enforcement).

- On a terminal whose width is known, rows never wrap. Detail columns (paths,
  previews) are truncated with `…` first, then names. Marks and fixed columns
  never truncate. Piped output is never truncated.
- CLI lists and tables take their column widths from `grid::solve` and fit
  cells with `grid::fit`. The Squad board compiles row tracks into admitted
  markup and sizes them with Taffy after its priority pre-step; its shared
  grapheme fitter preserves the resulting logical text widths and clips.
  Board percentages use the content box before gaps, while CLI lists use the
  remaining width after gaps. Widths are display cells (wide characters count
  two). A column has a basis (its width or widest content), `min`/`max`, a
  `grow` share of what is left, a shrink tier (lower tiers shrink first, widest first) and an
  optional `priority` (the highest steps aside first once minimums do not
  fit). Equal inputs always give equal widths; ties go by column order. A cell
  cut short ends in `…`, or keeps both ends for paths and links
  (`Truncate::Middle`).
- Control and line-separator characters in user data are shown escaped
  (`table::escape`). This is a trust boundary: user data never reaches the terminal
  as control sequences.

## Enforcement

Three tests enforce this document. Each keeps a migration list of what does not
follow it yet. A list must equal what still fails: a command or file that now
follows the style fails the test until its entry is removed, and anything new
that breaks a rule fails at once. Migrating a command means deleting its
entries. The first two lists are empty when #436 closes, the interaction list when #485 does.

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

- **Printed command guard** (`tmt-cli`'s `cli_style_tests`, #1079). Core help
  examples use the same rendered-help walk. Presentation sites own small,
  test-only `HintSpec` lists: each records the actual source template, command
  boundaries and representative substitutions. A source scan rejects missing
  or stale templates, including literals inside formatting macros. Dynamic
  context and channel hints also supply samples from their real formatters.
  Commands are shell-tokenized and passed to `parser::parse_core`; config set
  and clear commands additionally use the pure `Setting::edit` and
  `LocalClear::parse` policies. Nothing is dispatched or written. Invalid
  commands cannot use the help-style migration allowlist. External extension
  commands have explicit skip reasons and remain their owner's responsibility;
  core does not import an extension grammar. Non-command prose and bundled
  `learn` guidance are labeled separately, rather than treated as hints.

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

- **Interaction guard** (the architecture test, #485). In the same crates,
  production code may not call `is_terminal` or name `IsTerminal`; it receives
  an `Interaction` decision instead. The files that still decide for themselves
  are listed in `rust/crates/tmt-cli/tests/architecture/interaction.rs`.

## Migrating a command

Each step leaves the command's tests passing; delete the command's entries from
the migration lists in the same change.

1. **Help.** Build the command from a `CommandSpec`: a one-line summary and one
   to three examples, the common use first. A CLI that parses help itself (core)
   calls `apply` on its own `Command`; any other CLI calls `command`, which also
   adds `-h`/`--help` and `--json`, and answers `help <command>` through
   `route`. `command` turns clap's version flag off: a root that reports its
   version adds `version_arg`, the hidden `-V`/`--version`
   (`ArgAction::Version` prints `<name> <version>`; core passes `SetTrue` and
   answers itself). Each example is the full command a user types. The grammar walk parses it through the real parser, so run the walk
   until the command leaves its help list.
2. **Streams.** Replace `io::stdout()`/`io::stderr()` and print macros with
   `stream::stdout(json)` and `stream::stderr()`. Pass `stream.terminal()` to
   every renderer.
3. **Output.** Write outcomes with `message::success`, `message::error` and
   `message::hint`, lists with `list::Section`, one record with
   `detail::write`, and values with `value`. Text that must stay exact (a
   stored response, a prompt, captured pane text) moves into a function of its
   own that writes it through the stream unchanged. Only output that cannot use
   a stream (a protocol line on a raw handle) becomes an exact-body exemption
   with a reason.
4. **JSON proof.** `--json` never changes. Capture the command's `--json` output
   before and after the change in the same sandbox, and show the byte-equal
   comparison in the pull request.
