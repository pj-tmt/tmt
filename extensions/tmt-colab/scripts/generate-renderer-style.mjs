import { readFile, writeFile } from 'node:fs/promises';

const tokens = JSON.parse(
  await readFile(new URL('../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const file = new URL('../typescript/app/public/renderer.html', import.meta.url);
const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode))
  throw new Error('Use --write or --check');

// The opaque renderer cannot fetch styles. Project its paint roles and shared circle at build time.
const roles = [
  ['surface', 'annotation-highlight'],
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
const start = '          // BEGIN GENERATED ANNOTATION TOKENS';
const end = '          // END GENERATED ANNOTATION TOKENS';
const source = await readFile(file, 'utf8');
const from = source.indexOf(start);
const to = source.indexOf(end);
if (from < 0 || to <= from) throw new Error('Missing renderer token markers');
const literal = `'${css.replaceAll('\\', '\\\\').replaceAll("'", "\\'")}'`;
const generated = `${start}\n          const annotationTokens =\n            ${literal};\n`;
const expected = source.slice(0, from) + generated + source.slice(to);
if (mode === '--write') await writeFile(file, expected);
else if (source !== expected) throw new Error('Renderer annotation tokens are stale');
