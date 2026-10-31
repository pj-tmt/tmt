import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { readFile, writeFile } from 'node:fs/promises';
import { buildPageUi } from './page-ui.mjs';

const app = createRequire(new URL('../typescript/app/package.json', import.meta.url));
const { browserElements } = await import(pathToFileURL(app.resolve('@tmt/browser-ui/elements-manifest')).href);

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
// A page the design system paints takes the scheme it paints for these roles too (see data-colab-scheme
// below); these rules are unlayered, so they must name it as well as the viewer's theme.
const css = `:root{${variables('light')}}:root[data-theme="dark"]{${variables('dark')}}:root[data-colab-scheme="light"]{${variables('light')}}:root[data-colab-scheme="dark"]{${variables('dark')}}${numberStyle.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\s+/g, ' ').trim()}`;
const frameStyle = (
  await readFile(new URL('../typescript/app/src/page-frame.css', import.meta.url), 'utf8')
)
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/\s+/g, ' ')
  // Double quotes keep the formatter from rewriting the single-quoted literal that holds it.
  .replaceAll("'", '"')
  .trim();
// The design system the frame injects into every page (#5057) and the icons it draws; page-ui.mjs builds
// both, and the standalone export carries the same bytes.
const { staticCss, editorialCss, pageUiStyle, iconNodes, pageIcons } = await buildPageUi();
// `page check` reads the same two lists the frame draws from: a `tmt-ui-*` class it does not know and an
// icon name it does not know are findings, not silent no-ops.
const classes = [...new Set([...(staticCss + editorialCss).matchAll(/\.(tmt-ui-[A-Za-z0-9_-]+)/g)].map((m) => m[1]))].sort();
const names = new URL('../rust/tmt-colab/src/page_check/page_ui.json', import.meta.url);
const namesText = `${JSON.stringify({ classes, icons: Object.keys(iconNodes).sort(), elements: browserElements }, null, 1)}\n`;
const source = await readFile(file, 'utf8');
// Each generated constant sits between a pair of marker comments in renderer.html.
const generate = (text, name, constant, value, indent = '          ') => {
  const start = `${indent}// BEGIN GENERATED ${name}`;
  const end = `${indent}// END GENERATED ${name}`;
  const from = text.indexOf(start);
  const to = text.indexOf(end);
  if (from < 0 || to <= from) throw new Error(`Missing renderer ${name.toLowerCase()} markers`);
  const literal = `'${value.replaceAll('\\', '\\\\').replaceAll("'", "\\'")}'`;
  return `${text.slice(0, from)}${start}\n${indent}const ${constant} =\n${indent}  ${literal};\n${text.slice(to)}`;
};
const expected = generate(
  generate(generate(source, 'ANNOTATION TOKENS', 'annotationTokens', css), 'PAGE FRAME', 'pageFrame', frameStyle),
  'PAGE UI',
  'pageUi',
  pageUiStyle,
);
const withIcons = generate(
  generate(expected, 'PAGE ICONS', 'pageIcons', pageIcons),
  'UI ELEMENT NAMES', 'widgetElementNames', browserElements.map(({ tag }) => tag).join(' '), '        ',
);
if (mode === '--write') {
  await writeFile(file, withIcons);
  await writeFile(names, namesText);
} else if (source !== withIcons || (await readFile(names, 'utf8').catch(() => '')) !== namesText)
  throw new Error('Renderer generated styles are stale');
