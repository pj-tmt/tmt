import { readFile, writeFile } from 'node:fs/promises';

const tokens = JSON.parse(
  await readFile(new URL('../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const file = new URL('../typescript/app/public/renderer.html', import.meta.url);
const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode))
  throw new Error('Use --write or --check');

// The opaque renderer cannot fetch styles. Project only its paint roles, at development time.
const roles = [
  ['surface', 'annotation-highlight'],
  ['surface', 'annotation-action'],
  ['color', 'annotation-on-action'],
];
const variables = (theme) =>
  roles
    .map(
      ([group, name]) =>
        `--tmt-ui-${group}-${name}:${tokens.browser[group][name][theme].toLowerCase()};`,
    )
    .join('');
const css = `:root{${variables('light')}}:root[data-theme="dark"]{${variables('dark')}}`;
const start = '          // BEGIN GENERATED ANNOTATION TOKENS';
const end = '          // END GENERATED ANNOTATION TOKENS';
const source = await readFile(file, 'utf8');
const from = source.indexOf(start);
const to = source.indexOf(end);
if (from < 0 || to <= from) throw new Error('Missing renderer token markers');
const generated = `${start}\n          const annotationTokens =\n            '${css}';\n`;
const expected = source.slice(0, from) + generated + source.slice(to);
if (mode === '--write') await writeFile(file, expected);
else if (source !== expected) throw new Error('Renderer annotation tokens are stale');
