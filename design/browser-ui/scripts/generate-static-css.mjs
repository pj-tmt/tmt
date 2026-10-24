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
const fixed = [
  ...Object.entries(tokens.font).map(
    ([name, value]) => `  --tmt-ui-font-${name}: ${value.stack.replaceAll('"', "'")};`,
  ),
  ...Object.entries(tokens.header).map(([name, value]) => `  --tmt-ui-header-${name}: ${value};`),
  ...Object.entries(tokens.browser.metric).map(([name, value]) => `  --tmt-ui-${name}: ${value};`),
  motion('--tmt-ui-motion', '  '),
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
  }
}
`;
const componentCss = `${fragments.map((s) => bare(s.replaceAll('\r\n', '\n').replaceAll('__compact-max-width__', tokens.header['compact-max-width']))).join('\n')}\n`;
export const authoredStaticCss = `${tokenCss}${componentCss}`;
/** Use the pinned production CSS compiler; authored fragments remain formatted and readable. */
async function compileStaticCss() {
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
          if (id === '\0browser-leaf.css') return componentCss;
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
  return `${tokenCss}${asset.source}`;
}

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
  const css = await compileStaticCss();
  const output = new URL('generated/static.css', home);
  if (mode === '--write') await writeFile(output, css, 'utf8');
  else if ((await readFile(output, 'utf8')) !== css)
    throw new Error('Generated static.css is stale');
}
