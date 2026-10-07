# tmt-tui pipeline and components

Maintained module reference. Commands and admitted spellings are in [development.md](development.md).

## Pipeline

1. **Structural admission** (`lib`, `style`): bounded XML (`MAX_BYTES` 256 KiB,
   `MAX_DEPTH` 32, `MAX_NODES` 20,000) becomes a template with source locations,
   not a renderable scene. Declarations and excessive depth are refused before
   tree allocation. Static classes, wrap and literal tokens
   compile into `style::CellStyle`, including every repeat template.
2. **Binding** (`binding::compile`, `materialize`): `compile` checks an explicit
   application schema, lexical dotted paths (root `$` and repeat aliases), stable
   IDs and application-owned source/format handles, including empty repeat
   bodies. `materialize` borrows `serde_json::Value` data and checks referenced
   value kinds. Missing required paths are errors; null scalar text is absent.
   Direct binds retain display text; only the application's source adapter applies
   formats. Expansion is bounded to 20,000 nodes, 20,000 repeat iterations and
   8 MiB of aggregate text/ID bytes (including the duplicate-ID registry).
   Borrowed text is charged before copying. Stable IDs are nonempty, nonnumeric
   strings of at most 256 bytes and use scoped components, never collection
   positions; semantic row IDs stay separate.
3. **Geometry** (`geometry::layout`): materialized styles map into one private Taffy
   flex/grid computation. It borrows node identity/style, takes scalar intrinsic
   and wrap metrics, and returns whole-cell rectangles, content, ancestor clips
   and overflow/cut intent. Fractional spare cells are styled blanks inside hits.
   A cut grid cell keeps its logical width/height, shows at least four cells or
   hides whole. Percentages are CSS content-box shares with gaps added; CLI lists
   keep their own after-gap base and rounding in `tmt-cli-style::grid`, and no
   adapter joins the two policies.
   **Width-conditional layout** (`tmt-switch`): `<tmt-switch>` holds ordered
   `<tmt-case min="lg|md|sm">` branches and a final `<tmt-default>`; geometry lays out
   exactly one. The first case whose `min` is at most the measured width wins, so
   boundaries are inclusive and there are no gaps or overlaps. The width is the
   container's (the content width of the switch's parent box), or the viewport's with
   `of="terminal"`. A parent is measured with its unresolved switches absent, so a
   box never takes its width from the branch chosen inside it; nested switches resolve
   one level per pass and a tree without a switch is laid out once. A case or default
   is not a box: its children join the parent in place, so grid items stay grid items
   and the result equals the hand-written markup of that branch. Admission accepts
   only breakpoint names (`tmt_cli_style::breakpoint`), never numbers or `max`, and
   rejects cases that do not descend, duplicate `min`, a missing or non-final default,
   a switch without a case and a branch outside a switch. `hide-below="md"` on
   `tmt-row`, `tmt-col`, `tmt-cell` or `tmt-text` is the one-element form: the element
   is the only case and the default is empty. Binding compiles and materializes every
   branch, so a schema or data error shows at every width; IDs may repeat across the
   branches of one switch (a cell present at several widths keeps its identity over a
   resize) and stay unique everywhere else. Node and byte budgets count all branches.
   Hits and selection follow stable IDs, never the branch. Fit-driven decisions (a
   column that drops until the tracks fit, a hint list cut by measured text) are not
   steps; they stay with the application that measures.
4. **Text and paint** (`text`, `paint`): `text` owns grapheme measurement and fitting
   for markup; `paint` consumes geometry in preorder into a caller-owned Ratatui
   buffer. Both use `Cell::text_width`, never the rounded spare cell. Cuts
   ellipsize already measured lines without rewrapping (end or middle); a wide
   grapheme crossing a clip edge leaves styled blanks. Theme and Depth are
   injected; roles inherit and resolve through the shared screen adapter. The
   caller supplies the complete selected-role style, or `paint_with`'s preorder-indexed
   callback for the full style and text alignment; selection policy stays with the
   application.
5. **Hits**: hits borrow scoped IDs and semantic row IDs, inherit identity,
   intersect visible buffer clips, omit zero areas and resolve in reverse paint
   order.

## Application layer and component behavior

- `app` owns base focus, one replaceable modal and top-first event routing:
  unhandled modal keys and mouse are captured, closing events never replay into the
  base, and Ctrl-C returns a quit effect.
- `components` owns opaque modal chrome, square by default, with titles only when nonempty,
  fixed footer/status/position slots, visual-line scroll/clamp/reveal and typed key-help
  sections. Wrapped fixed text is measured by the
  text owner before the scroll viewport is reserved; nowrap slots keep their one-line default.
  Authored modal cell height controls demand within placement bounds, while body references
  still fill the application body. Its surface
  compiler lowers component markup into the same bounded binding and geometry
  pipeline, so generated templates obey the same depth/node limits. Component IDs
  are static scoped IDs outside repeats. One list or table is supported per scroll
  surface.
- Modal sheets: a `Center` or `Docked` modal is a full-width sheet below the `md`
  breakpoint and nine tenths of the body from it (never wider than its authored
  demand). Breakpoint names and cell values come from `design/tokens/tokens.json`
  through `tmt-cli-style::breakpoint`; callers ask the modal for its width instead of
  repeating the rule.
- Key help measures one display-cell label column across all sections and stacks
  descriptions when fewer than 20 cells remain. Heading and spacing properties let
  a caller choose bold headings and one blank line between sections.
- `ListState` reconciles stable row identity across refresh and reorder, picks the
  nearest enabled survivor after removal and reveals the whole wrapped row.
  List/table admission requires a row template and typed `id: StableId` and
  `disabled: Boolean` fields; table cells use existing grid tracks. Disabled and
  empty rows cannot activate. Ordinary panes use `collection::compile/render`;
  modal lists and picker slots use `surface::compile/render_list`. Clipped row
  maps retain the painted model and scroll offset, so stale mouse geometry cannot
  activate. Ordinary panes reserve a dim `N more ↓` line while content overflows.
- `strip` paints one line of styled spans (left or right aligned) through the
  text owner's fitting, with no template, binding or geometry pass: a strip is
  one text cell per span, so each span is clipped to the room left, filled with
  its style and fitted by `text::fit_line`. The caller supplies the resolved span
  styles, so it owns no role or selection policy and takes no Theme or Depth
  decision. Control characters are shown
  escaped, and the continuation cell of a wide grapheme carries its span's style.
  A strip test keeps the former layout pipeline as an oracle and compares buffers
  cell for cell over generated lines, areas and buffer origins. `Outline` is the
  single-line frame with a styled title for panes of a base layer; unlike
  `Modal` it neither clears nor fills what is inside. Both default to square
  borders. Their explicit `paint_flat` choice shares horizontal rules and blank
  side slots, retaining every edge reservation, title policy and inner rectangle;
  Modal also retains its full Clear/opaque mask and fixed slots.
- `Picker` owns bounded grapheme query editing and returns query changes,
  selection changes, confirmation or cancellation. The application filters
  projected data, routes focused fields and owns previews, saves and rollback.
- Rendering injects Theme/Depth and the selection policy, keeps semantic roles
  under caller-owned selection styling (reverse/bold fallback) and returns
  current-frame clipped hits.
