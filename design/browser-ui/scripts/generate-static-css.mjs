import { readFile, writeFile, realpath } from 'node:fs/promises';
import { build } from 'vite-plus';
import { pathToFileURL } from 'node:url';
const home = new URL('../', import.meta.url);
const tokens = JSON.parse(
  await readFile(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);
// Theme-invariant roles inherit from the root; nested theme switches never change them.
const themed = (theme, indent = '  ', root = false) =>
  [
    ['color', tokens.browser.color],
    ['surface', tokens.browser.surface],
    ['shadow', tokens.browser.shadow],
    ['brand-mark', tokens.brand.mark],
  ]
    .flatMap(([group, roles]) =>
      Object.entries(roles)
        .filter(([name, value]) => !name.startsWith('$') && (root || value.light !== value.dark))
        .map(
          ([name, value]) => `${indent}--tmt-ui-${group}-${name}: ${value[theme].toLowerCase()};`,
        ),
    )
    .join('\n');
/** The motion tokens as custom properties; `reduced` zeroes every duration. */
function motion(prefix, indent, reduced = false) {
  return Object.entries(tokens.browser.motion)
    .filter(([name]) => !name.startsWith('$') && (!reduced || ['fast', 'base'].includes(name)))
    .map(([name, value]) => `${indent}${prefix}-${name}: ${reduced ? '0ms' : value};`)
    .join('\n');
}
/** The editorial ramp (#5794): a part naming a metric token reuses it; `compact-` parts apply through
 * the compact width. */
const typeValue = (value) =>
  Object.hasOwn(tokens.browser.metric, value) ? `var(--tmt-ui-${value})` : value;
const typeRoles = Object.entries(tokens.browser.type).filter(([role]) => !role.startsWith('$'));
const typeParts = (indent, compact) =>
  typeRoles.flatMap(([role, parts]) =>
    Object.entries(parts)
      .filter(([part]) => part.startsWith('compact-') === compact)
      .map(
        ([part, value]) =>
          `${indent}--tmt-ui-type-${role}-${part.replace(/^compact-/, '')}: ${typeValue(value)};`,
      ),
  );
const fixed = [
  ...Object.entries(tokens.font).map(
    ([name, value]) => `  --tmt-ui-font-${name}: ${value.stack.replaceAll('"', "'")};`,
  ),
  ...Object.entries(tokens.header).map(([name, value]) => `  --tmt-ui-header-${name}: ${value};`),
  ...Object.entries(tokens.browser.metric).map(([name, value]) => `  --tmt-ui-${name}: ${value};`),
  motion('--tmt-ui-motion', '  '),
  ...typeParts('  ', false),
].join('\n');
/** A fragment as shipped: its comments explain the source, not the page, so they stay in `src/`. */
const bare = (source) =>
  source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/[ \t]+$/gm, '')
    .replace(/\n{2,}/g, '\n')
    .trim();
const fragments = await Promise.all(
  [
    'header',
    'brand-mark',
    'shell',
    'swap',
    'notice',
    'field',
    'action',
    'icon-action',
    'toggle',
    'checkbox',
    'choice',
    'glyph',
    'avatar',
    'pager',
    'status-tile',
    'state-mark',
    'card',
    'meter',
    'page-head',
    'stats',
    'banner',
    'facts',
    'list',
    'drawer',
    'panel',
    'dialog',
    'menu',
    'count-chip',
    'tabs',
    'notifications',
    'chips',
    'spinner',
    'command',
    'chart',
    'section',
    'table',
    'disclosure',
    'visually-hidden',
  ].map((name) => readFile(new URL(`src/${name}.css`, home), 'utf8')),
);
const tokenCss = `/* Generated: generate-static-css */
:root {\n${fixed}\n${themed('light', '  ', true)}\n}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme='light']) {\n${themed('dark', '    ')}\n  }
}
[data-theme='light'] {\n${themed('light')}\n}
[data-theme='dark'] {\n${themed('dark')}\n}
@media (prefers-reduced-motion: reduce) {
  :root {\n${motion('--tmt-ui-motion', '    ', true)}\n  }
}
@media (max-width: ${tokens.header['compact-max-width']}) {
  :root {
    --tmt-ui-header-height: var(--tmt-ui-header-compact-height);
    --tmt-ui-header-padding-inline: var(--tmt-ui-header-compact-padding-inline);
${typeParts('    ', true).join('\n')}
  }
}
`;
const shipped = (source) =>
  bare(
    source
      .replaceAll('\r\n', '\n')
      .replaceAll('__compact-max-width__', tokens.header['compact-max-width']),
  );
const componentCss = `${fragments.map(shipped).join('\n')}\n`;
export const authoredStaticCss = `${tokenCss}${componentCss}`;
/** The editorial layer (#5794), its own entry: the prose rules, then the table look applied to a bare
 * `table` in prose. The table rules are the marked region of table.css with the class rewritten, so
 * prose tables and `.tmt-ui-table` cannot differ. */
async function editorialSource() {
  const [prose, table] = await Promise.all(
    ['editorial', 'table'].map((name) => readFile(new URL(`src/${name}.css`, home), 'utf8')),
  );
  const begin = '/* BEGIN editorial table */';
  const end = '/* END editorial table */';
  const from = table.indexOf(begin);
  const to = table.indexOf(end);
  if (from < 0 || to <= from) throw new Error('Missing editorial table markers in table.css');
  const rows = table.slice(from + begin.length, to).replace(/\.tmt-ui-table(?![\w-])/g, '& table');
  return `${shipped(prose)}\n:where(.tmt-ui-prose) {\n${shipped(rows)}\n}\n`;
}
export const authoredEditorialCss = await editorialSource();
/** Use the pinned production CSS compiler; authored fragments remain formatted and readable. */
async function compileCss(source) {
  const result = await build({
    configFile: false,
    publicDir: false,
    logLevel: 'silent',
    plugins: [
      {
        name: 'browser-leaf-static-css',
        resolveId(id) {
          if (id === 'virtual:browser-leaf.css') return '\0browser-leaf.css';
        },
        load(id) {
          if (id === '\0browser-leaf.css') return source;
        },
      },
    ],
    build: {
      write: false,
      emptyOutDir: false,
      minify: false,
      cssMinify: 'lightningcss',
      cssTarget: 'esnext',
      cssCodeSplit: true,
      rolldownOptions: { input: 'virtual:browser-leaf.css' },
    },
  });
  const asset = result.output.find(
    (entry) => entry.type === 'asset' && entry.fileName.endsWith('.css'),
  );
  if (!asset || typeof asset.source !== 'string')
    throw new Error('Static CSS compiler produced no stylesheet');
  return asset.source;
}
const compileStaticCss = async () => `${tokenCss}${await compileCss(componentCss)}`;
const compileEditorialCss = async () =>
  `/* Generated: generate-static-css */\n${await compileCss(authoredEditorialCss)}`;

const isMain =
  process.argv[1] &&
  (await realpath(process.argv[1]).then(
    (path) => pathToFileURL(path).href === import.meta.url,
    () => false,
  ));
if (isMain) {
  const mode = process.argv[2];
  if (process.argv.length !== 3 || !['--write', '--check'].includes(mode))
    throw new Error('Use --write or --check');
  for (const [name, compile] of [
    ['static', compileStaticCss],
    ['editorial', compileEditorialCss],
  ]) {
    const css = await compile();
    const output = new URL(`generated/${name}.css`, home);
    if (mode === '--write') await writeFile(output, css, 'utf8');
    else if ((await readFile(output, 'utf8').catch(() => '')) !== css)
      throw new Error(`Generated ${name}.css is stale`);
  }
}
