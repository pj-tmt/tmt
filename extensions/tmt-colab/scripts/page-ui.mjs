// The design system a page frame carries (#5057) and a standalone export copies, built once so the two
// cannot differ: the CSS (the leaf's whole static.css and the page host and base profiles, capped), the
// bounded Lucide icon nodes, and the `tmt-icon` element's script. The renderer inlines the CSS and the
// icons as constants (generate-renderer-style.mjs); the runtime build writes the CSS and the script as
// `assets/colab-page-ui.css` and `assets/colab-page-ui.js`, which the export reads.
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';

// The injected page UI CSS cap, in bytes, the one figure the build and the runtime check share: the exact
// measured size, no headroom (GUI lead, #5794): 119,949 -> 125,075 bytes for the type ramp and the
// editorial layer, -> 125,199 for the #5820 work-panel tokens (row-height, subheader-height,
// control-size-l, dialog-narrow-width; GUI lead approved). A change that grows it re-measures and asks
// again; #5809 cuts the injected static.css to what an author page can use and lowers this to its new
// exact figure.
export const PAGE_UI_LIMIT = 125_199;

export async function buildPageUi() {
  // The design system the frame injects into every page (#5057): the leaf's static.css, whole, in the
  // `tmt-ui` layer, with the page host profile. The layer order is declared first, so the frame's own
  // rules win over the starter's, which win over the classes; an author's unlayered CSS beats all three.
  const compactCss = (css) =>
    css
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/\s+/g, ' ')
      .replace(/ ?([{};,]) ?/g, '$1')
      .trim();
  const staticCss = await readFile(
    new URL('../../../design/browser-ui/generated/static.css', import.meta.url),
    'utf8',
  );
  // The editorial layer (#5794) sets an author page's own text: its `.tmt-ui-prose` container is the
  // painted page's body, in the starter's layer, so the classes and an author's CSS both win over it.
  const editorialCss = (
    await readFile(
      new URL('../../../design/browser-ui/generated/editorial.css', import.meta.url),
      'utf8',
    )
  ).replaceAll(':where(.tmt-ui-prose)', ':where([data-colab-design] body)');
  const hostCss = await readFile(
    new URL('../typescript/app/src/page-host.css', import.meta.url),
    'utf8',
  );
  const baseCss = await readFile(
    new URL('../typescript/app/src/page-base.css', import.meta.url),
    'utf8',
  );
  // The tokens follow the viewer's `data-theme`; they also follow `data-colab-scheme`, the scheme the page
  // itself paints, which the frame measures from the page's canvas colour. A page that paints its own light
  // body under a dark viewer then gets light-theme components on it. Each theme block names both selectors,
  // and the second has the higher specificity, so the painted scheme wins over the viewer's.
  const schemed = ['light', 'dark'].reduce((css, name) => {
    const block = `[data-theme='${name}'] {`;
    if (css.split(block).length !== 2)
      throw new Error(`static.css must hold exactly one ${block} token block`);
    return css.replace(block, `[data-theme='${name}'],\n:root[data-colab-scheme='${name}'] {`);
  }, staticCss);
  // Quoted strings must survive the compaction untouched.
  const quoted = (css) => css.match(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g) ?? [];
  if (quoted(compactCss(schemed)).join('\n') !== quoted(schemed).join('\n'))
    throw new Error('Compacting static.css would change a quoted string');
  const pageUiStyle = `@layer colab-frame,tmt-starter,tmt-ui;${compactCss(baseCss)}@layer tmt-starter{${compactCss(editorialCss)}}@layer tmt-ui{${compactCss(schemed)}${compactCss(hostCss)}}`
    // Double quotes keep the formatter from rewriting the single-quoted literal that holds it.
    .replaceAll("'", '"');
  if (Buffer.byteLength(pageUiStyle) > PAGE_UI_LIMIT)
    throw new Error(
      `The injected page UI CSS is ${Buffer.byteLength(pageUiStyle)} bytes, over its ${PAGE_UI_LIMIT} byte cap`,
    );
  // `<tmt-icon name="check">` (#5057): the frame expands the element to inline SVG from the Lucide icons
  // the page runtime already lists (src/runtime/icons.ts is the one list), so a page names an icon in
  // kebab-case and ships no icon code. Only the drawn nodes are kept; the frame adds the Colab defaults.
  const app = createRequire(new URL('../typescript/app/package.json', import.meta.url));
  const lucide = app('lucide-react');
  const { createElement } = app('react');
  const { renderToStaticMarkup } = app('react-dom/server');
  const listed = [
    ...(
      await readFile(new URL('../typescript/app/src/runtime/icons.ts', import.meta.url), 'utf8')
    ).matchAll(/^ {2}([A-Za-z0-9]+): lucide\.\1,$/gm),
  ].map((match) => match[1]);
  if (listed.length < 50) throw new Error('Could not read the runtime icon list');
  const kebab = (name) =>
    name
      .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
      .replace(/([A-Za-z])(\d)/g, '$1-$2')
      .toLowerCase();
  const iconNodes = {};
  for (const name of listed.toSorted()) {
    const markup = renderToStaticMarkup(createElement(lucide[name]));
    const inner = markup.match(/^<svg[^>]*>([\s\S]*)<\/svg>$/)?.[1];
    if (!inner || !new RegExp(`class="[^"]*\\blucide-${kebab(name)}\\b`).test(markup))
      throw new Error(`Lucide icon ${name} is not drawn as lucide-${kebab(name)}`);
    iconNodes[kebab(name)] = inner.replaceAll('></path>', '/>').replaceAll('></circle>', '/>').replaceAll('></rect>', '/>').replaceAll('></line>', '/>').replaceAll('></polyline>', '/>').replaceAll('></ellipse>', '/>').replaceAll('></polygon>', '/>');
  }
  const pageIcons = JSON.stringify(iconNodes);

  // The `tmt-icon` element and the paint measurement are regions of renderer.html between marker comments,
  // so the frame and the export run the same code. In the export they are one head script of their own.
  const renderer = await readFile(
    new URL('../typescript/app/public/renderer.html', import.meta.url),
    'utf8',
  );
  // The code of a region of renderer.html between two markers, dedented, so the frame and the export
  // run the same text.
  const region = (name) => {
    const from = renderer.indexOf(`// BEGIN ${name}`);
    const to = renderer.indexOf(`// END ${name}`);
    if (from < 0 || to <= from) throw new Error(`Missing renderer ${name.toLowerCase()} markers`);
    return renderer
      .slice(renderer.indexOf('\n', from) + 1, to)
      .split('\n')
      .map((line) => line.replace(/^ {10}/, ''))
      .join('\n')
      .trim();
  };
  // The export measures the painted scheme the way the frame does (data-colab-scheme, #5057): once the
  // body exists, and again when the page's own paint or the theme changes.
  const driver = `const swatch = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
let timer;
const measure = () => {
  timer = undefined;
  if (document.body && swatch) document.documentElement.dataset.colabScheme = paintedScheme(swatch);
};
const schedule = () => {
  if (timer === undefined) timer = setTimeout(measure, 0);
};
document.addEventListener('DOMContentLoaded', () => {
  measure();
  const observer = new MutationObserver(schedule);
  for (const node of [document.documentElement, document.body])
    observer.observe(node, { attributes: true, attributeFilter: ['class', 'style', 'data-theme'] });
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', schedule);
});
window.addEventListener('load', measure);`;
  const pageUiScript = `(()=>{const iconNodes=${pageIcons};\n${region('PAGE ICON ELEMENT')}\n${region('PAGE SCHEME')}\n${driver}\n})();\n`;
  for (const [name, text, end] of [
    ['style', pageUiStyle, '</style'],
    ['script', pageUiScript, '</script'],
  ])
    if (text.includes('<!--') || text.toLowerCase().includes(end))
      throw new Error(`The page UI ${name} could end its own element or open a comment`);
  return { staticCss, editorialCss, pageUiStyle, iconNodes, pageIcons, pageUiScript };
}
