import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';

const tokens = JSON.parse(
  await readFile(new URL('../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const file = new URL('../typescript/app/public/renderer.html', import.meta.url);
const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode))
  throw new Error('Use --write or --check');

// The opaque renderer cannot fetch styles. Project its paint roles, the shared circle and the page
// frame at build time.
const roles = [
  ['surface', 'annotation-highlight'],
  ['surface', 'annotation-highlight-hover'],
  ['surface', 'annotation-highlight-selected'],
  ['color', 'annotation-rule'],
  ['color', 'annotation-rule-selected'],
  ['surface', 'annotation-action'],
  ['color', 'annotation-on-action'],
  ['surface', 'sheet'],
  ['color', 'text'],
];
const variables = (theme) =>
  roles
    .map(
      ([group, name]) =>
        `--tmt-ui-${group}-${name}:${tokens.browser[group][name][theme].toLowerCase()};`,
    )
    .join('');
const numberStyle = await readFile(
  new URL('../typescript/app/src/annotation-number.css', import.meta.url),
  'utf8',
);
const css = `:root{${variables('light')}}:root[data-theme="dark"]{${variables('dark')}}${numberStyle.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\s+/g, ' ').trim()}`;
const frameStyle = (
  await readFile(new URL('../typescript/app/src/page-frame.css', import.meta.url), 'utf8')
)
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/\s+/g, ' ')
  // Double quotes keep the formatter from rewriting the single-quoted literal that holds it.
  .replaceAll("'", '"')
  .trim();
// The design system the frame injects into every page (#5057): the leaf's static.css, whole, in the
// `tmt-ui` layer, with the page host profile. The layer order is declared first, so the frame's own
// rules win over the starter's, which win over the classes; an author's unlayered CSS beats all three.
const PAGE_UI_LIMIT = 120_000;
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
const hostCss = await readFile(
  new URL('../typescript/app/src/page-host.css', import.meta.url),
  'utf8',
);
const baseCss = await readFile(
  new URL('../typescript/app/src/page-base.css', import.meta.url),
  'utf8',
);
// Quoted strings must survive the compaction untouched.
const quoted = (css) => css.match(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g) ?? [];
if (quoted(compactCss(staticCss)).join('\n') !== quoted(staticCss).join('\n'))
  throw new Error('Compacting static.css would change a quoted string');
const pageUiStyle = `@layer colab-frame,tmt-starter,tmt-ui;${compactCss(baseCss)}@layer tmt-ui{${compactCss(staticCss)}${compactCss(hostCss)}}`
  // Double quotes keep the formatter from rewriting the single-quoted literal that holds it.
  .replaceAll("'", '"');
if (pageUiStyle.length > PAGE_UI_LIMIT)
  throw new Error(
    `The injected page UI CSS is ${pageUiStyle.length} bytes, over its ${PAGE_UI_LIMIT} byte cap`,
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
// `page check` reads the same two lists the frame draws from: a `tmt-ui-*` class it does not know and an
// icon name it does not know are findings, not silent no-ops.
const classes = [...new Set([...staticCss.matchAll(/\.(tmt-ui-[A-Za-z0-9_-]+)/g)].map((m) => m[1]))].sort();
const names = new URL('../rust/tmt-colab/src/page_check/page_ui.json', import.meta.url);
const namesText = `${JSON.stringify({ classes, icons: Object.keys(iconNodes).sort() }, null, 1)}\n`;
const source = await readFile(file, 'utf8');
// Each generated constant sits between a pair of marker comments in renderer.html.
const generate = (text, name, constant, value) => {
  const start = `          // BEGIN GENERATED ${name}`;
  const end = `          // END GENERATED ${name}`;
  const from = text.indexOf(start);
  const to = text.indexOf(end);
  if (from < 0 || to <= from) throw new Error(`Missing renderer ${name.toLowerCase()} markers`);
  const literal = `'${value.replaceAll('\\', '\\\\').replaceAll("'", "\\'")}'`;
  return `${text.slice(0, from)}${start}\n          const ${constant} =\n            ${literal};\n${text.slice(to)}`;
};
const expected = generate(
  generate(generate(source, 'ANNOTATION TOKENS', 'annotationTokens', css), 'PAGE FRAME', 'pageFrame', frameStyle),
  'PAGE UI',
  'pageUi',
  pageUiStyle,
);
const withIcons = generate(expected, 'PAGE ICONS', 'pageIcons', pageIcons);
if (mode === '--write') {
  await writeFile(file, withIcons);
  await writeFile(names, namesText);
} else if (source !== withIcons || (await readFile(names, 'utf8').catch(() => '')) !== namesText)
  throw new Error('Renderer generated styles are stale');
