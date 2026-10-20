# Page layout

Colab owns a page's frame: the column width, the side gutters and the vertical gap between
top-level blocks. You own the content: colours, type and the layout inside each block.

## Layouts

| Layout     | What Colab does                                                      | Use for                                              |
| ---------- | -------------------------------------------------------------------- | ---------------------------------------------------- |
| `column`   | Frames the page in one centred column (below).                       | Reports, plans, reviews, Q&A. Most pages.            |
| `full`     | The same frame as `column`; built around edge-to-edge blocks.        | Wide images, boards, timelines.                      |
| `canvas`   | Sets nothing. The page lays itself out.                              | Interactive charts, games, tools that draw the page. |
| `freeform` | Sets nothing. A page that existed before layouts is `freeform`.      | Nothing new.                                         |

## Choose at creation

`page create` requires a choice and refuses without one:

```sh
tmt colab page create --title "Weekly plan" --layout column --file page.html --json
tmt colab page create --title "Board" --layout canvas --reason "Draws its own cards" --file page.html --json
```

`canvas` needs `--reason`, one line saying why the page draws its own frame; Colab shows it on the
page's About panel. A page that has no layout yet (`freeform`) keeps its look until you give it one:
`tmt colab page layout PAGE column` (or `full`, or `canvas --reason TEXT`) changes only the layout.
Writes and patches keep it. `tmt colab show PAGE --json` reports `layout`. There is no way back
to `freeform`.

## The frame

A top-level block is an element child of `<body>`, other than `script`, `style`, `template`,
`noscript`, `link`, `meta` and `[hidden]`. All blocks share one column:

- width `min(1280px, 100% - 32px)`: at most 1280 px, 16 px from each side of a narrower window;
- 16 px between blocks, and none above the first;
- text wraps at the column width. There is no narrower measure.

Each top-level element is a block with its own gap, so group related text in one `<section>` or
`<div>` instead of leaving headings and paragraphs loose under `<body>`. A wrapper that holds the
whole page is one block: its inside is yours, its width is the column.

Questions, review controls and proposal cards that Colab draws sit on the same column.

## Two declarations

Both go on a top-level block. They are the only way to change the frame.

| Declaration                     | Effect                                                            | Use for                                             |
| ------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------- |
| `data-width="full"`             | The block runs edge to edge; its text still starts on the column. | A large image, a board, a banner. Also in `column`. |
| `data-gap="tight"` or `"loose"` | The gap above the block becomes 8 px or 40 px (default 16 px).    | Keep two related blocks close; open a new part.     |

```html
<section>
  <h2>Release status</h2>
  <p>Two checks are still running.</p>
</section>
<section data-gap="tight">
  <table>
    <tr><td>Build</td><td>✓ Done</td></tr>
  </table>
</section>
<div data-width="full">
  <img src="colab-attachment:9c01aa3e" alt="Timeline" width="1600" height="400" style="width: 100%; height: auto">
</div>
```

A `data-width="full"` block gets its horizontal padding from Colab, which puts its text on the
column. Put your own horizontal padding on a child element.

## What CSS may change

Colab overrides these, even with `!important` in a stylesheet. On a top-level block: `width`,
`min-width`, `max-width`, margins and the gap between blocks, plus the horizontal padding and
borders of a `full` block. On `<html>` and `<body>`: horizontal margin, padding and borders, and
the `body`'s `display` and width. Do not set them; they have no effect.

Everything else stays yours: colours, backgrounds, type, a column block's padding and borders,
vertical `<body>` spacing, and any layout inside a block (grid, flex, tables).

An inline `style` attribute with `!important` can still win, and then the block no longer sits on
the column or lines up with Colab's own controls. The frame is a cosmetic projection, so use the
two declarations instead.

The `body` rule in the [page-style](page-style.md) starter (`max-width`, centred) is for `canvas`
and `freeform` pages; a framed page ignores it.

For the markup that fits inside a block, read [components](components.md)
(`tmt colab skill components`).
