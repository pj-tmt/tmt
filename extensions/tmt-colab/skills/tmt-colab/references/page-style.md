## Page look

Colab gives every page the TMT design system with nothing to import: its tokens (`--tmt-ui-*`, light and
dark), fonts, the `tmt-ui-*` components and `<tmt-icon>` ([components](components.md),
`tmt colab skill components`). A page whose source contains `tmt-starter`, a `tmt-ui-` class or `<tmt-icon`
also gets the shared editorial layer on its plain elements: the type ramp for `h1` to `h4` and body text,
paragraph and list rhythm, links, `blockquote`, tables, `figure` with `figcaption`, inline `code`, wrapping
`pre`, and `<span class="tmt-ui-tag">` labels. That page is painted with the viewer theme's paper and ink;
set both a body background and a text colour, or neither. The starter below is only the glue on top: the
page column, section spacing, and the `muted` and `waiting` classes. The layer and the starter sit in the
`tmt-starter` layer, so the components and your own CSS win over both.

Keep the look flat and shadow-free, with a mark plus a word for every state (for example, "◆ Waiting"),
never colour alone. Colour only the waiting mark, with the starter's `waiting` class
(`<span class="waiting">◆</span>`); leave `●` and `✓` neutral. On a `column` or `full` page Colab sets the width and the
gaps: read [layout](layout.md) (`tmt colab skill layout`) and do not set a
width, margin or block gap yourself. The starter's `body` rule (centred,
`max-width: 1280px`) is for `canvas` and `freeform` pages. A page keeps the starter it was created
with, but the editorial layer is the frame's and follows the current design system.

Colab already shows the brand, page title and actions. Do not add a site header,
navigation, product mark or wordmark, or any sticky or fixed bar. Begin with the
content; the user sees one Colab header.

Colab sets root data-theme (light or dark) before scripts and on live changes,
overwriting any author-pinned value, and the design tokens follow it: colour with the `--tmt-ui-*`
variables. CSS keyed only on prefers-color-scheme follows the OS.

For a fixed look, paint the page yourself: set the body background and text colour, and the components take
the scheme you paint (a page painted light stays light under a dark viewer). Author CSS owns colours and
backgrounds. A downloaded `page.standalone.html` carries the design system and follows the OS.

Use [answer-block markup](questions.md) for questions: one topic/section per
`tmt-questions` group, with long explanatory prose outside the group. Colab draws
the tabs, state, choice tiles and Send controls; do not hand-build forms or import
the React browser leaf into authored HTML. The group heading inherits page ink;
its muted `Q:` prefix and the pinned light answer card are supplied by Colab.

<!-- BEGIN generated page style -->

```html
<style>
@layer tmt-starter {
  * { box-sizing: border-box; }
  body {
    max-width: 1280px;
    margin: 0 auto;
    padding: 16px;
  }
  section { margin: 0 0 16px; }
  section > :last-child { margin-bottom: 0; }
  .muted { color: var(--tmt-ui-color-secondary); }
  .waiting { color: var(--tmt-ui-color-waiting); }
}
</style>
<!-- Examples to copy with fresh UUIDs (this comment is inert); see tmt colab skill, "Ask and flag"
<tmt-questions data-id="20000000-0000-4000-8000-000000000001">
  <p data-title>One decision</p>
  <tmt-question data-id="10000000-0000-4000-8000-000000000001" data-kind="single" data-agent="Your agent name">
    <p data-question>Which option?</p>
    <span data-choice>First</span><span data-choice>Second</span>
  </tmt-question>
</tmt-questions>
<tmt-read data-id="10000000-0000-4000-8000-000000000002" data-agent="Your agent name" data-why="Read before choosing">
  <h2 data-title>Requirements</h2>
  <p>The passage.</p>
</tmt-read>
-->
```

<!-- END generated page style -->
