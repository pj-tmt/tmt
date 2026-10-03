---
name: tmt-tui
description: Verify the internal TUI markup crate (`rust/crates/tmt-tui`) - XML admission, utilities, geometry, paint and components - and the Squad board parity baseline. Load when changing tmt-tui or board output. Owner - the tmt-squad squad.
---

# Internal TUI markup (`tmt-tui`)

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
