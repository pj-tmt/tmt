# Internal TUI markup (`tmt-tui`)

Architecture and the style guideline own roles and sizing; this skill holds the
commands, the admitted spellings and the parity gate.

```bash
(cd rust && cargo test --locked -p tmt-tui)
(cd rust && cargo +1.95.0 test --locked -p tmt-tui)
(cd rust && cargo test --locked -p tmt-cli --test architecture)   # component and dependency changes
(cd rust && cargo test --locked -p tmt-ops)                     # source adapter, board/list parity fixture
(cd rust && cargo test --locked -p tmt-ops board::composition)  # composition changes
```

## Utility spellings

One spelling per value kind; anything else fails admission with a located error:

- Cells, weights, counts: `w-N`, `h-N`, `basis-N`, `min-w-N`, `max-w-N`, `grow-N`,
  `shrink-N`, `gap-N`, `gap-x-N`, `gap-y-N`, `p-N`, `px-N`, `py-N`, `col-span-N`,
  `line-clamp-N`; also `grow`, `shrink`, `w-full`, `h-full`. `N` is ASCII decimal
  0..4096 terminal cells (`w-4` is four cells, not a rem scale); spans and clamps
  are positive.
- Percentages and tracks: `w-[N%]`, `h-[N%]`, `basis-[N%]`, `grid-cols-[tracks]`.
  Percentages are integers 0..100; tracks are underscore-separated cells,
  percentages, integer `Nfr`, `auto`, or `minmax(a,b)` (no fr minimum; `auto` only
  as the minimum). Bracket integers fail with a bare-form hint.
- Layout: `flex`, `flex-row`, `flex-col`, `grid` (grid conflicts with an explicit flex
  direction). Text: `truncate`, `truncate-middle`, `line-clamp-N`; leaf
  `wrap="true"|"false"` conflicts with all text-flow utilities.
- Unknown or malformed utilities, duplicate/overlapping properties, variants,
  fractional numbers and arbitrary CSS values fail admission. Padding is symmetric.
  View and col default to column, other elements to row; sizes default to auto,
  gaps/padding/grow to zero, shrink to one, text to clipping. `token` names a shared
  `Role`; omission inherits.

- Width steps: `<tmt-switch of="container|terminal">`, `<tmt-case min="sm|md|lg">` and
  `<tmt-default>`; `hide-below="sm|md|lg"` on row, col, cell and text. The names come
  from `design/tokens/tokens.json`; no other attribute (`max`, numbers, style, id,
  token) is admitted on a branch.

## Components

- `components::surface::compile` lowers `tmt-modal`, `tmt-scroll` and `tmt-key-help`
  into primitive templates and checks depth/node budgets and schemas eagerly. A
  surface has one literal-ID modal, one literal-ID scroll body and optional
  `tmt-text slot="footer"|"status"`; components cannot occur in repeats.
- `tmt-list`/`tmt-table` bind a root collection whose rows declare
  `id: StableId`, `disabled: Boolean` and display fields, and contain exactly one
  `tmt-row` template (`as` alias, default `row`); one list or table per scroll
  surface. `tmt-picker` supplies its own modal and scroll body.
- Callers perform effects: route input through `app::route` before base handlers,
  discard row maps on resize or model replacement, and call `Picker::reconcile`
  after `QueryChanged`. Rendering never runs effects.
- Authored `h-N` sets modal height demand within placement bounds (body references still fill
  the body); wrapped footer/status text reserves its measured lines before the scroll
  viewport, while nowrap slots keep their one-line default.
- `text::measure` is a capped upper bound, not the widest wrapped line; measurement
  and fitting share the recorded text width, and paint reuses it.

## Board parity baseline

The parity harness captures the three explicit presets and the team default at
120×30, 80×30 and 120×30 again, including every cell's style and state, hits, row
starts and list text/JSON. A board PR that intentionally changes captured output
sends the decoded cell/style/hit diff to the Squad lead and gets approval **before**
regenerating the baseline. The regeneration is its own commit in that PR.

The read-only inspector decodes cell runs and styles, reports hit/tab/row-start changes,
and requires list text/JSON to stay byte-identical:

```bash
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-ops inspect_markup_parity_diff -- --ignored --nocapture
```

After approval, regenerate with:

```bash
CARGO_BUILD_JOBS=2 cargo test --locked -p tmt-ops regenerate_markup_parity_fixture -- --ignored
```

Attribute every change to the PR's approved behavior and review hit identities and list
bytes; unexplained changes block handoff. Normal tests never write the fixture.
