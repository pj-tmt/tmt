---
name: tmt-tui
description: Verify the internal TUI markup crate (`rust/crates/tmt-tui`) - XML admission, utilities, geometry, paint and components - and the Squad board parity baseline. Load when changing tmt-tui or board output. Owner - the tmt-squad squad.
---

# Internal TUI markup (`tmt-tui`)

## Drawing boundary

Squad consumes drawing primitives through `tmt-tui`. The native architecture
test's `architecture/board_widgets.rs` checks Squad's production module tree
with the existing syn collector, including qualified, imported, aliased and
macro-token widget paths. Widget traits and state are infrastructure; unrelated
CLI/TOML tables and test-only code remain outside the primitive prohibition.

The remaining raw base chrome, scroll painting and action menu are tracked by
[#1544](https://github.com/pj-tmt/tmt/issues/1544). The guard permits only exact
file/widget entries, each with a preservation reason and that follow-up link;
new widgets in listed files still fail, and stale exceptions must be removed.
The surface map belongs to [tmt-squad-dev](../tmt-squad-dev/SKILL.md).

Run `CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-cli --test architecture`
from `rust/`. It includes a clean source-tree control and a seeded raw-widget
failure through real module discovery. Keep the frozen parity gate in the
development reference below; unexplained differences block handoff.

## References

- [references/pipeline-and-components.md](references/pipeline-and-components.md): admission, binding, geometry, paint and component rules.
- [references/development.md](references/development.md): build, test and verification commands moved from DEVELOPMENT.md.

## Admission rules

- Squad is the only reviewed consumer. A new consumer, a new dependency or any
  Squad term in the crate goes to tmt-lead first. The architecture guard permits
  XML parsing, borrowed JSON, shared style, private Taffy geometry and Ratatui
  buffer painting; never core, adapters, CLI or extension behavior.
- The leaf acquires nothing: no terminal, clock, settings persistence, markdown or
  provider data. Applications own data, effects, item cursors and terminal
  lifecycle.
- Components implement the
  [full-screen interaction guideline](../../../design/cli-style.md#full-screen-interaction);
  application-owned descriptions and effective bindings supply their text.
- Tokens use `tmt-cli-style::theme::Role`. No palette is resolved or copied here.
