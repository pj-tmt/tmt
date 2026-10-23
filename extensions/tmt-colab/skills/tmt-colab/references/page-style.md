## Page look

Use the TMT browser style: flat, shadow-free surfaces with the one control radius;
the teal-tinted palette; system fonts; and a mark plus a word for every state (for example, "◆ Waiting"),
never colour alone. Colour only the waiting mark, with the starter's `waiting` class
(`<span class="waiting">◆</span>`); leave `●` and `✓` neutral. On a `column` or `full` page Colab sets the width and the
gaps: read [layout](layout.md) (`tmt colab skill layout`) and do not set a
width, margin or block gap yourself. The starter's `body` rule (centred,
`max-width: 1280px`) is for `canvas` and `freeform` pages. Start with the inline
content styles below, never the browser leaf's whole static.css.

Colab already shows the brand, page title and actions. Do not add a site header,
navigation, product mark or wordmark, or any sticky or fixed bar. Begin with the
content; the user sees one Colab header.

Colab sets root data-theme (light or dark) before scripts and on live changes,
overwriting any author-pinned value. Use the starter's explicit data-theme selectors to
follow Colab. CSS keyed only on prefers-color-scheme follows the OS.

For a fixed look, use one unconditional :root palette and color-scheme; remove
the starter's data-theme and prefers-color-scheme overrides. Author CSS owns
colours and backgrounds. Outside Colab, the starter follows the OS.

Use [answer-block markup](questions.md) for questions: one topic/section per
`tmt-questions` group, with long explanatory prose outside the group. Colab draws
the tabs, state, choice tiles and Send controls; do not hand-build forms or import
the React browser leaf into authored HTML. The group heading inherits page ink;
its muted `Q:` prefix and the pinned light answer card are supplied by Colab.

<!-- BEGIN generated page style -->

```html
<style>
  :root {
    --ink: #26393f;
    --muted: #596f77;
    --paper: #f3f6f5;
    --sheet: #ffffff;
    --edge: #d4dfdb;
    --waiting: #8a5a08;
    --body: Inter, -apple-system, system-ui, 'Segoe UI', sans-serif;
    --mono: ui-monospace, SFMono-Regular, Menlo, monospace;
    --size: 14px;
    --heading: 28px;
    --rule: 1px;
    --gap: 16px;
    --small-gap: 8px;
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme='light']) {
      --ink: #c9d6dd;
      --muted: #a2b3bf;
      --paper: #0e171d;
      --sheet: #17232c;
      --edge: #283a46;
      --waiting: #f7b449;
    }
  }
  :root[data-theme='light'] {
    --ink: #26393f;
    --muted: #596f77;
    --paper: #f3f6f5;
    --sheet: #ffffff;
    --edge: #d4dfdb;
    --waiting: #8a5a08;
    color-scheme: light;
  }
  :root[data-theme='dark'] {
    --ink: #c9d6dd;
    --muted: #a2b3bf;
    --paper: #0e171d;
    --sheet: #17232c;
    --edge: #283a46;
    --waiting: #f7b449;
    color-scheme: dark;
  }
  * {
    box-sizing: border-box;
  }
  body {
    max-width: 1280px;
    margin: 0 auto;
    padding: var(--gap);
    background: var(--paper);
    color: var(--ink);
    font: var(--size)/1.5 var(--body);
    overflow-wrap: anywhere;
  }
  h1,
  h2,
  h3 {
    margin: 0 0 var(--small-gap);
    line-height: 1.2;
  }
  h1 {
    font-size: var(--heading);
  }
  h2 {
    font-size: 22px;
  }
  h3 {
    font-size: 18px;
  }
  p,
  ul,
  ol,
  table,
  pre {
    margin: 0 0 var(--gap);
  }
  ul,
  ol {
    padding-left: calc(var(--gap) * 2);
  }
  a {
    color: inherit;
    text-decoration: underline;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: var(--small-gap);
    border-bottom: var(--rule) solid var(--edge);
    text-align: left;
    vertical-align: top;
  }
  code,
  pre {
    font-family: var(--mono);
  }
  pre {
    padding: var(--gap);
    background: var(--sheet);
    border: var(--rule) solid var(--edge);
    white-space: pre-wrap;
  }
  section {
    margin: 0 0 var(--gap);
    padding: var(--gap);
    background: var(--sheet);
    border: var(--rule) solid var(--edge);
  }
  section > :last-child {
    margin-bottom: 0;
  }
  .muted {
    color: var(--muted);
  }
  .waiting {
    color: var(--waiting);
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
