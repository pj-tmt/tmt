---
name: tmt-squad-dev
description: Build and verify the Squad extension (`tmt-squad`, `tmt-sq`, the board, notebook, cron library, embedded lead skill and playbooks). Load when changing extensions/tmt-squad or running its native and E2E checks. Owner - the tmt-squad squad. Not the shipped lead skill `tmt-squad`.
---

# Squad development

## Board surface ownership

`board/view.rs` owns frame orchestration and one frame-level hit/scroll-map reset.
Its surface modules under `board/view/` retain the existing painters:

| Module      | Responsibility                                                     |
| ----------- | ------------------------------------------------------------------ |
| `header`    | Summary, token meter, spinner and clock-derived invalidation text  |
| `tabs`      | Tab labels, windows and painted tab hits                           |
| `panes`     | Split/tab composition dispatch, `Outline` borders, folded titles   |
| `rows`      | Cached row scene preparation, scroll reveal and clipped row hits   |
| `row_paint` | Row scene: admitted cells, ages, waiting line, `paint_with` hits   |
| `notes`     | Shared notebook lines, lead notes selection, links and hits        |
| `detail`    | Selected row fields; member notebook                               |
| `replies`   | Safe final bodies, their derived cache and scrolling               |
| `footer`    | Effective hints, notices, link previews and unanchored input strip |
| `waiting`   | Acquired decision text, inline composer bands and docked ask-lead  |
| `overlays`  | Overlay dispatch and switcher painting                             |

`row_paint` builds the rows scene (admitted cells, solved boxes, ages, waiting line,
annotation, `✓ sent` line, reserved input lines) and paints it through
`tmt-tui::paint::paint_with`; `rows` only prepares and caches it.

Every strip, border and scroll line is painted through `tmt-tui` (`Strip`,
`Outline`, `Modal`): no board module names a ratatui widget. The action menu is a
`tmt-modal` list surface built per menu in `board/menu_surface.rs` (its title is
the row or request name); `Action::order` orders its entries, the selected row's
actions before the board's.

`App`, terminal/worker lifecycle, acquisition, `Scrolls` (position math; it paints
its lines and overflow indicator through `Strip`), home and shared TUI
components keep their separate owners. Home dispatch precedes ordinary panes;
its painter alone produces home row starts and continuation hits. Existing
`view` helper entry points remain available to those callers. Integrated renderer
tests live in `view/tests.rs`, with help, meter and frozen parity submodules.

Raw ratatui widget enforcement and its verification belong to the
[tmt-tui skill](../tmt-tui/SKILL.md). A surface split preserves captured cells,
styles, hits and list bytes; it grants no parity-regeneration permission.

## References

- [references/development.md](references/development.md): build, test and verification commands moved from DEVELOPMENT.md.

[ARCHITECTURE](../../../ARCHITECTURE.md#squad-extension) owns the seam, dependency direction and
public-contract index. The user-facing row and config reference is the embedded lead skill
(`extensions/tmt-squad/skills/tmt-squad/SKILL.md`); do not copy its field lists, state-pattern
grammar or key tables into the references below.

## Reference files

| Topic                                                                                | File                                                      |
| ------------------------------------------------------------------------------------ | --------------------------------------------------------- |
| Membership, leadership marker, `me`, field providers, staleness, reminders, cron     | [data-and-state.md](references/data-and-state.md)         |
| `squad.toml` layering and writes, themes, views, settings, link and action effects   | [config-and-effects.md](references/config-and-effects.md) |
| Board frame, row grid, composition, tabs, scrolling, composers, overlays, validation | [board.md](references/board.md)                           |
| HOME model, section scenes, cursor projection and cache invalidation                 | [board-home.md](references/board-home.md)                 |
| Lead/member notebooks, links, annotations, detail and replies                        | [board-notebooks.md](references/board-notebooks.md)       |
| Board cron acquisition, jobs half/list, forms and scoped input                       | [board-cron.md](references/board-cron.md)                 |
| Refresh worker, change detection, token meter, shutdown                              | [refresh-and-meter.md](references/refresh-and-meter.md)   |

## Invariants

- Core reachability: public `--json` commands and `tmt api` only, through
  `TMT_EXECUTABLE` (or `tmt` on PATH). `runner` maps results and errors onto
  `tmt-invoke` for bounded capture. No TMT crate depends on Squad; the
  architecture guard enforces both directions for Cargo dependencies and source
  references. Runtime TMT dependencies are the neutral leaves `tmt-cli-style`,
  `tmt-invoke` and `tmt-tui`.
- A squad is the core room `squad-<name>`. Member fields are identity metadata
  `squad.<name>.<field>`; Squad has no membership store of its own.
- Squad-owned data lives under `<dataRoot>/squad` (`storage.root` from `tmt api`),
  plus disposable caches under `$XDG_CACHE_HOME/tmt-squad/`. `squad.toml` is the
  user's file; agents never write it, and no cron data goes into it or the core
  database.
- Squad never writes `config.json`, a provider directory or tmux state except
  through core commands; `jump` and `back` use `tmt focus`.
- Board-only data (the home model and token-rate meter state, including any
  `usage.*` observation) never enters public `ls --json` or the other public
  documents. Public documents carry display-ready strings; consumers must not
  format them again.
- Paint and input perform no core reads; refresh, providers and notebook reads run
  on workers (see [refresh-and-meter.md](references/refresh-and-meter.md)).
- Command grammar, help and human output go through `tmt-cli-style`
  (`CommandSpec`, `Interaction`); `board` runs only when `Interaction::view()` is
  `Interactive`, decided once in `main`, otherwise it is `ls`. `tmt squad` with no
  command is `board`. Consent for hotkeys and playbooks is a `Consent` decided in
  `main` from `--yes` and `prompt()`.
- Squad's dependencies must not change the CLI product: prove it package-scoped
  (`cargo ... -p tmt-cli` alone), because combined workspace builds can unify
  shared-dependency features.
- Squad is versioned and released independently (`tmt-squad-v<version>` tags). Its
  archive also carries `skills/tmt-squad/`, the same source as the embedded lead
  skill; playbooks under `extensions/tmt-squad/playbooks/` are deliberately outside
  `skills/` (see [config-and-effects.md](references/config-and-effects.md#playbooks)).
