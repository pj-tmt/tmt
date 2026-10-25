# Page runtime

For a page that must react to the reader (filter, sort, toggle, update a table), opt in to the
bundled runtime instead of hand-writing DOM scripts. Colab loads React 19, htm, Tailwind and a
curated Lucide icon set into the page before its own script, with no network and no build step.
A static page stays plain HTML: read [page-style](page-style.md) (`tmt colab skill page-style`).

## Opt in

Put one marker in the page. The first `meta` with this name decides; the tokens are separated by
ASCII whitespace and either `react` or `tailwind` loads the whole runtime (other tokens are ignored):

```html
<meta name="tmt-colab-runtime" content="react tailwind" />
```

Without the marker nothing extra loads, and `tmt` does not exist.

## Script

Everything is on one global, `tmt`. There is no JSX, Babel or import: `html` is htm bound to React,
so components are tagged templates, and `{...}` values are escaped like React text.

```html
<meta name="tmt-colab-runtime" content="react tailwind" />
<div id="app"></div>
<script>
  const { React, createRoot, html, icons } = tmt;
  const { useState } = React;
  const stateClass = {
    working: 'text-working',
    waiting: 'text-waiting',
    blocked: 'text-blocked',
  };
  const rows = [
    { name: 'Build', state: 'working' },
    { name: 'Review', state: 'waiting' },
    { name: 'Deploy', state: 'blocked' },
  ];
  function App() {
    const [query, setQuery] = useState('');
    const shown = rows.filter((row) => row.name.toLowerCase().includes(query.toLowerCase()));
    return html`
      <section class="flex flex-col gap-4 p-4">
        <label class="flex items-center gap-2 border border-edge bg-sheet px-2 py-1">
          <${icons.Search} aria-label="Search" />
          <input
            class="grow bg-transparent text-text outline-none"
            placeholder="Filter"
            value=${query}
            onInput=${(event) => setQuery(event.target.value)}
          />
        </label>
        <ul class="flex flex-col gap-2">
          ${shown.map(
            (row) => html`
              <li key=${row.name} class="flex items-center gap-2 border-b border-rule pb-2">
                <span class=${stateClass[row.state]}>◆</span>
                <span class="grow">${row.name}</span>
                <span class="text-secondary">${row.state}</span>
              </li>
            `,
          )}
        </ul>
      </section>
    `;
  }
  createRoot(document.getElementById('app')).render(html`<${App} />`);
</script>
```

Use ordinary React: `useState`, `useEffect`, `useMemo`, keys on lists. Mount into an element you
write; do not replace `body`. Keep the page's data in the script or in attributes: the frame has
no network, forms or storage.

## Look

Tailwind's utility classes work, with Colab's theme instead of its default palette. Colour names are
the TMT roles and follow Colab's light or dark theme by themselves, so a page needs no theme code:
`bg-paper`, `bg-sheet`, `bg-code`, `bg-command`, `bg-selection`, `bg-disabled`, `text-text`,
`text-secondary`, `text-link`, `border-edge`, `border-rule`, and the state colours `text-working`,
`text-waiting`, `text-review`, `text-blocked` (`bg-`, `border-` and so on work for each). Also
`black`, `white`, `transparent` and `current`. `rounded` and every sized `rounded-*` take the one
control radius (`rounded-full` stays a pill, `rounded-none` square), shadows are removed and focus is
one outline in the focus colour; spacing, flex, grid and type scales are Tailwind's. Do not use a colour Tailwind's default palette names (`bg-blue-500`): it does not exist
here. Write each class name in full (a lookup of complete names, never `text-${state}`). Mark every state with a symbol and a word, never colour alone.

On a `column` or `full` page Colab sets the width and gaps ([layout](layout.md)); do not set your own
page width, margin or block gap. Do not add a header, product mark or sticky bar.

## Icons

`icons` holds a curated Lucide set; list it with `Object.keys(tmt.icons)` where the page runs, and use a
member as a component: `<${icons.Check} />`. Icons are 16 px, square-capped and `currentColor` by
default, and decorative unless you give `aria-label` or `title`. An icon that is not listed is
`undefined`: if you need one, use a text mark (`◆ ● ✓ ✗`) instead of drawing SVG paths by hand, and ask
for it to be added.

## Never

- No CDN, `<script src>`, `@import`, font or stylesheet links: the frame denies all network access
  and the page would show nothing.
- No `import` or `require`; no `eval` or `new Function`.
- No second copy of React or Tailwind in the page.
- Do not copy the runtime into the page source; the marker is the only thing the page carries.
