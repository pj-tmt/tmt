---
name: tmt-tui
description: Verify the internal TUI markup crate (`rust/crates/tmt-tui`) - XML admission, utilities, geometry, paint and components - and the Squad board parity baseline. Load when changing tmt-tui or board output. Owner - tmt-ux.
---

# Internal TUI markup (`tmt-tui`)

## Drawing boundary

Squad consumes drawing primitives through `tmt-tui`. The native architecture
test's `architecture/board_widgets.rs` checks Squad's production module tree
with the existing syn collector, including qualified, imported, aliased and
macro-token widget paths. Widget traits and state are infrastructure; unrelated
CLI/TOML tables and test-only code remain outside the primitive prohibition.

The guard's exception list is empty: any raw widget in Squad's production tree
fails, and an exception needs a documented boundary decision. The surface map
belongs to [tmt-ops-dev](../tmt-ops-dev/SKILL.md).

Run `CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli --test architecture`
from `rust/`. It includes a clean source-tree control and a seeded raw-widget
failure through real module discovery. Keep the frozen parity gate in the
development reference below; unexplained differences block handoff.

## Terminal screen

`components::terminal` shows another program's screen: `Screen` takes bytes and a size and
publishes a `Frame` of cells (colours, attributes, wide characters, cursor, input modes; `caret` is
the cursor cell even while the program hides it); `key_bytes`,
`paste_bytes`, `wheel_bytes` and `is_leave` (Ctrl-\) are the bytes a key, a paste or a wheel turn
sends back. It parses with `vt100` at the workspace pin (the parser that formats a built-in host's
snapshot), keeps no scrollback and does no I/O, and it knows nothing of the stream that carries the
bytes or of `tmt-pty-protocol`.

## Dialogs

A new overlay that asks, edits, chooses or shows is a `components::Dialog`; the
rule it implements is [Dialogs](../../../design/cli-style.md#dialogs). Ops overlays move onto it
one group at a time (#5647); until then each keeps its own keys and close path.

- Pick the kind by what the user does: `Confirm` for a yes/no with text (delete
  timer?), `Form` for fields (rename squad), `List` for choosing a row
  (snapshots), `ReadOnly` for reading (agent details).
- `DialogState::key` and `DialogState::mouse` are the only key and mouse rule:
  the application acts on the `DialogInput` they return (`Field`, `Content`,
  `Choose`, `Cancel`) and never reads Tab, Esc or an outside press itself.
- `Dialog::areas` keeps the modal size rule; never compute a dialog rectangle by
  hand. `Dialog::paint` frames it and paints the `Button` row; the application
  paints the content and adds its field rectangles to `DialogHits.fields`.
- The application keeps drafts across a close and keeps a running operation going
  when its dialog hides.
- Tests: the crate's dialog tests own the rule; a new Ops dialog adds its scene to
  the HOME oracle and a test that Esc and an outside press close it and that a
  draft survives reopen.

## References

- [references/pipeline-and-components.md](references/pipeline-and-components.md): admission, binding, geometry, paint and component rules.
- [references/development.md](references/development.md): build, test and verification commands moved from DEVELOPMENT.md.

## Admission rules

- Squad is the only reviewed consumer. A new consumer, a new dependency, an
  app-neutral API change or any Squad term in the crate still needs tmt-lead review.
  The architecture guard permits
  XML parsing, borrowed JSON, shared style, private Taffy geometry and Ratatui
  buffer painting; never core, adapters, CLI or extension behavior.
- The leaf acquires nothing: no terminal, clock, settings persistence, markdown or
  provider data. Applications own data, effects, item cursors and terminal
  lifecycle.
- Components implement the
  [full-screen interaction guideline](../../../design/cli-style.md#full-screen-interaction);
  application-owned descriptions and effective bindings supply their text.
- Tokens use `tmt-cli-style::theme::Role`. No palette is resolved or copied here.
