## Page look

Colab gives every page the TMT design system with nothing to import: its tokens (`--tmt-ui-*`, light and
dark), fonts, the `tmt-ui-*` components and `<tmt-icon>` ([components](components.md),
`tmt colab skill components`). The starter below is only the glue on top: body type, the heading scale,
rhythm, a block look for `pre`, and the `muted` and `waiting` classes. It sits in the `tmt-starter` layer, so
the components and your own CSS win over it. A page whose source contains `tmt-starter`, a `tmt-ui-` class
or `<tmt-icon` is painted with the viewer theme's paper and ink and accent links; set both a body background
and a text colour, or neither.

Keep the look flat and shadow-free, with a mark plus a word for every state (for example, "◆ Waiting"),
never colour alone. Colour only the waiting mark, with the starter's `waiting` class
(`<span class="waiting">◆</span>`); leave `●` and `✓` neutral. On a `column` or `full` page Colab sets the width and the
gaps: read [layout](layout.md) (`tmt colab skill layout`) and do not set a
width, margin or block gap yourself. The starter's `body` rule (centred,
`max-width: 1280px`) is for `canvas` and `freeform` pages. A page keeps the starter it was created
with; a newer starter changes only the pages created after it.

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
    font: 15px/1.5 var(--tmt-ui-font-body);
    overflow-wrap: anywhere;
  }
  h1, h2, h3 {
    margin: 0 0 8px;
    font-weight: 660;
    line-height: 1.2;
  }
  h1 {
    font-size: 29px;
    font-weight: 720;
    letter-spacing: -0.95px;
  }
  h2 { font-size: 22px; }
  h3 { font-size: 19px; }
  p, ul, ol, pre, table { margin: 0 0 16px; }
  section { margin: 0 0 16px; }
  section > :last-child { margin-bottom: 0; }
  ul, ol { padding-left: 24px; }
  table { width: 100%; border-collapse: collapse; }
  th, td {
    padding: 8px;
    border-bottom: 1px solid var(--tmt-ui-surface-rule);
    text-align: left;
    vertical-align: top;
  }
  pre {
    padding: 12px 16px;
    background: var(--tmt-ui-surface-code);
    border-radius: var(--tmt-ui-radius);
    font: 13px/1.5 var(--tmt-ui-font-mono);
    white-space: pre-wrap;
  }
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
