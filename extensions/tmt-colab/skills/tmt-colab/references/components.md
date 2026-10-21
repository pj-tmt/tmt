# Page components

The shortest markup for each part a page can use. Each sample is a run of top-level blocks for a
`column` page (see [layout](layout.md), `tmt colab skill layout`); drop the ones you do not need.
Group related text in one block, and run `tmt colab page check FILE` before publishing.

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
<section>
  <h2>Summary</h2>
  <p>One short paragraph: the result and what the reader should do next.</p>
</section>
```

## Section

A boxed block for one topic. `data-gap="tight"` keeps a related block close:

```html
<section>
  <h2>Findings</h2>
  <p>One topic.</p>
</section>
<section data-gap="tight">
  <h2>Related finding</h2>
  <p>Eight pixels below the previous block.</p>
</section>
```

## Table

The table fills its block; never set a width.

```html
<section>
  <table>
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
<section class="compare">
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
