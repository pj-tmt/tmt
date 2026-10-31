# Page components

The shortest markup for each part a page can use. Each sample is a run of top-level blocks for a
`column` page (see [layout](layout.md), `tmt colab skill layout`); drop the ones you do not need.
Group related text in one block, and run `tmt colab page check FILE` before publishing.

## Design system

Every page frame carries the product's own component classes (`tmt-ui-*`), the design tokens and the
`tmt-icon` element: nothing to import, no network. The markup below is the whole promise; the frame
holds other `tmt-ui-*` classes that are not documented and may change. Your own CSS wins over these
classes, and a page that uses them and sets no `body` background or text colour is painted with the
viewer theme's paper and ink (a link takes the accent); set both or neither. The classes' colours
follow the scheme the page paints: a page you paint light stays light under a dark viewer, and an
unpainted one follows the viewer. `tmt colab page check` reports a `tmt-ui-*` class or an icon name the frame
does not have (`unknown-ui-class`, `unknown-icon`). The author elements below provide interactive controls without an author script.

Section label and card; a card runs its table edge to edge inside its padding:

```html
<section class="tmt-ui-section">
  <h2 class="tmt-ui-section-heading" data-variant="label">Checks</h2>
  <div class="tmt-ui-card">
    <div class="tmt-ui-card-head">
      <h3 class="tmt-ui-card-title">Release</h3>
      <span class="tmt-ui-card-meta">2 rows</span>
    </div>
    <table class="tmt-ui-table">
      <thead>
        <tr>
          <th>Check</th>
          <th>State</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td>Build</td>
          <td>✓ Done</td>
        </tr>
        <tr>
          <td>Review</td>
          <td>◆ Waiting</td>
        </tr>
      </tbody>
    </table>
  </div>
</section>
```

Actions (`data-variant`: `primary`, `text`, `ghost`, `destructive`; none is the plain bordered button) and
count chips (`data-tone="attention"` for what needs the reader). Put them in a paragraph so they sit in a
row, side by side (a bare button fills its block). A page's own script gives them behaviour:

```html
<p>
  <button class="tmt-ui-action" data-variant="primary">
    <span class="tmt-ui-action-label">Approve</span>
  </button>
  <button class="tmt-ui-action"><span class="tmt-ui-action-label">Cancel</span></button>
  <button class="tmt-ui-action" data-variant="text">
    <span class="tmt-ui-action-label">Details</span>
  </button>
  <button class="tmt-ui-count-chip">
    <span>Comments</span><span class="tmt-ui-count">3</span>
  </button>
  <button class="tmt-ui-count-chip" data-tone="attention">
    <span>Needs you</span><span class="tmt-ui-count">2</span>
  </button>
</p>
```

Callout (`data-tone`: `working`, `waiting`, `review`, `blocked`; add `data-tinted` for the soft fill):

```html
<div class="tmt-ui-notice" data-tinted data-tone="waiting">
  <div class="tmt-ui-notice-eyebrow">Needs you</div>
  <h3 class="tmt-ui-notice-heading">Decide before release</h3>
  <div class="tmt-ui-notice-body">What the reader has to decide, in a sentence.</div>
</div>
```

List rows, a disclosure, a field and code:

```html
<ul class="tmt-ui-list" aria-label="Tasks">
  <li class="tmt-ui-list-row">
    <div class="tmt-ui-list-title"><span class="tmt-ui-list-label">Write the report</span></div>
    <div class="tmt-ui-list-meta"><div class="tmt-ui-list-state">Done</div></div>
  </li>
</ul>
<details class="tmt-ui-disclosure">
  <summary>More detail</summary>
  <p>Hidden until opened.</p>
</details>
<div class="tmt-ui-field">
  <label class="tmt-ui-field-label" for="note">Note</label>
  <input class="tmt-ui-field-control" id="note" />
</div>
<p>Run <code class="tmt-ui-code">tmt colab page check page.html</code> first.</p>
```

`tmt-ui-code` is for code inside a line. A code block is a plain `<pre><code>` that takes its look from
the starter ([page-style](page-style.md)), so give it no class.

Icons are a bounded Lucide set, named in lowercase kebab-case: `<tmt-icon name="circle-check"></tmt-icon>`
draws a 16px icon in the text colour; add `label="Passed"` to name it for assistive technology (otherwise it
is decorative). An unknown name draws nothing.

## Text

Headings, paragraphs and lists, grouped in one block:

```html
<div>
  <h2>Heading</h2>
  <p>A paragraph with <a href="/p/10000000">another Colab page</a>.</p>
  <ul>
    <li>One</li>
    <li>Two</li>
  </ul>
</div>
```

## Links

Plain `a[href]` links open through the trusted app, without navigating the page's isolated frame.
Use `/p/PAGE_PREFIX` for another page in the same space, optionally followed by `#element-id`.
Use `https://example.com/path` for an outside destination: the app shows its full URL and asks
the reader to Open or Cancel before opening a new tab. Same-page `#element-id` links scroll
inside the page. Do not script `location` changes or `window.open`; direct frame navigation still
stops the preview. Other URL schemes are refused.

## Summary

```html
<section class="tmt-ui-card">
  <h2 class="tmt-ui-card-title">Summary</h2>
  <p>One short paragraph: the result and what the reader should do next.</p>
</section>
```

## Section

A card for one topic (`tmt-ui-card`; a plain `<section>` is unboxed). `data-gap="tight"` keeps a related block close:

```html
<section class="tmt-ui-card">
  <h2 class="tmt-ui-card-title">Findings</h2>
  <p>One topic.</p>
</section>
<section class="tmt-ui-card" data-gap="tight">
  <h2 class="tmt-ui-card-title">Related finding</h2>
  <p>Eight pixels below the previous block.</p>
</section>
```

## Table

The table fills its block; never set a width. `tmt-ui-table` gives it the design system's header and rules; a bare
`<table>` gets the starter's plain rules.

```html
<section class="tmt-ui-card">
  <table class="tmt-ui-table">
    <thead>
      <tr>
        <th>Check</th>
        <th>State</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Build</td>
        <td>✓ Done</td>
      </tr>
      <tr>
        <td>Review</td>
        <td>◆ Waiting</td>
      </tr>
    </tbody>
  </table>
</section>
```

## Code

```html
<section>
  <pre><code>tmt colab page check page.html</code></pre>
</section>
```

## State

Write a mark and a word, never colour alone: `●` running, `◆` waiting on the reader, `✓` done,
`✗` failed or blocked, `◌` idle. Colour only the waiting mark, with the starter's `waiting` class
([page-style](page-style.md)); leave the others neutral.

```html
<p><span class="waiting">◆</span> Waiting for your decision</p>
<p>✗ Blocked on the release</p>
```

## Compare

Two columns, such as now and after, that stack on a narrow window. The layout inside a block is
yours:

```html
<style>
  .compare {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 16px;
  }
</style>
<section class="tmt-ui-card compare">
  <div>
    <h3>Now</h3>
    <p>Today's behaviour.</p>
  </div>
  <div>
    <h3>After</h3>
    <p>The proposed behaviour.</p>
  </div>
</section>
```

## Questions

Colab draws the controls. Every ID is a fresh canonical UUID, unique on the page. Grammar and
limits: [questions](questions.md) (`tmt colab skill questions`).

```html
<tmt-questions data-id="20000000-0000-4000-8000-000000000001">
  <p data-title>One decision</p>
  <tmt-question
    data-id="10000000-0000-4000-8000-000000000001"
    data-kind="single"
    data-agent="Your agent name"
  >
    <p data-question>Which option fits?</p>
    <span data-choice>First</span><span data-choice>Second</span>
  </tmt-question>
</tmt-questions>
```

## Review

A part of the page the reader confirms. Grammar and limits: [reviews](reviews.md)
(`tmt colab skill reviews`).

```html
<tmt-review data-id="30000000-0000-4000-8000-000000000001" data-agent="Your agent name">
  <h2 data-title>Section to confirm</h2>
  <p>The content the reader confirms.</p>
</tmt-review>
```

## Images

Attach the file first, then reference its attachment ID. Add `width` and `height` so the page does
not jump, and `max-width: 100%` so the image never overflows. Details:
[attachments](attachments.md) (`tmt colab skill attachments`).

```html
<div>
  <img
    src="colab-attachment:9c01aa3e"
    alt="Weekly trend"
    width="640"
    height="360"
    style="max-width: 100%; height: auto"
  />
</div>
```

## Live author widgets and Design catalog

Open Design in the Colab menu’s separate **Design system** group, or visit
`/colab/design/gui` in the composed app. The routes work independently of the menu
entry. Shared GUI/TUI tabs select the system; counted category tabs filter the live
cards. The GUI catalog contains content-sized previews, state examples, attributes,
tokens, a script-free proposal document and copyable embed markup. `/colab/design/tui` lists real terminal sources; painter
previews are unavailable until the shared terminal exporter and frame land.

Use `tmt-ui-button`, `tmt-ui-icon-button`, `tmt-ui-field`, `tmt-ui-toggle`,
`tmt-ui-checkbox`, `tmt-ui-menu`, `tmt-ui-tabs`, `tmt-ui-dialog`, `tmt-ui-chips`,
`tmt-ui-chip`, `tmt-ui-list-table` and `tmt-ui-notice` as documented by the live catalog. The page runtime
loads automatically for these HTML elements. No script, runtime marker, external
library or second stylesheet is needed. They use the viewer/page theme and the same
controls as the application. Existing classes and `tmt-icon` remain lightweight.

```html
<tmt-ui-toggle label="Include archived"></tmt-ui-toggle>
<tmt-ui-tabs label="Tasks" value="open">
  <section slot="panel" data-value="open" data-label="Open">Open tasks</section>
  <section slot="panel" data-value="done" data-label="Done">Completed tasks</section>
</tmt-ui-tabs>
```

Widget state is local to the renderer frame. It does not save a page, send a message
or invoke a host command. Dialog dismissal retains its draft; replacing source or
removing the element resets its local state. Replace the host to change child slots
or item structure. The checker reports unknown tags/attributes, invalid enum or
boolean values, missing labels, and invalid slot/item structure. See the
[shared element contract](../../../../../design/browser-ui/README.md#colab-author-elements).

The list-table element uses the shared responsive presentation. Set `columns` to a
JSON array of nonempty labels and `rows` to arrays of text cells, one per column.
Cells never interpret HTML. Supply the real host track variables on the element;
compact tracks can stack its cells. The Design catalog has a copyable example.
