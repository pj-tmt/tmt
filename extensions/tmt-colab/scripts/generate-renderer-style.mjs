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
  generate(source, 'ANNOTATION TOKENS', 'annotationTokens', css),
  'PAGE FRAME',
  'pageFrame',
  frameStyle,
);
if (mode === '--write') await writeFile(file, expected);
else if (source !== expected) throw new Error('Renderer generated styles are stale');
