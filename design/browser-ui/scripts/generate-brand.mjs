// Generates the static brand assets from the mark's one definition (src/brand-mark-svg.ts), its
// stylesheet (src/brand-mark.css) and the mark-only `brand.mark` roles in tokens.json: the static-host header mark,
// the favicon and the 512 px app icons. Hosts copy these bytes; nothing hand-copies a colour.
import { readFile, writeFile } from 'node:fs/promises';
import { brandMarkSvg, brandTileInner } from '../src/brand-mark-svg.ts';

const mode = process.argv[2];
if (process.argv.length !== 3 || !['--write', '--check'].includes(mode)) {
  throw new Error('Use --write or --check');
}
const tokens = JSON.parse(
  await readFile(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);
const roles = Object.entries(tokens.brand.mark).filter(([name]) => !name.startsWith('$'));
const vars = (theme) =>
  roles
    .map(([name, value]) => `--tmt-ui-brand-mark-${name}:${value[theme].toLowerCase()}`)
    .join(';');
// The mark's own rules, without the lockup's, in one line.
const css = (await readFile(new URL('../src/brand-mark.css', import.meta.url), 'utf8'))
  .split('.tmt-brand-lockup {')[0]
  .replace(/\/\*[\s\S]*?\*\//g, '')
  .replace(/\s*([{}:;,])\s*/g, '$1')
  .replace(/\s+/g, ' ')
  .replace(/;}/g, '}')
  .trim();
const style = (themes) => `<style>${themes}${css}</style>`;
const xmlns = 'xmlns="http://www.w3.org/2000/svg"';

const outputs = {
  // The header mark for static hosts (Remote pages, Colab's native guidance page).
  'brand-mark-static.svg': `${brandMarkSvg({ size: 24, className: 'tmt-brand-mark tmt-ui-mark', id: 'tmt-brand' })}\n`,
  // The tab icon: the small form on the dark tile, the same in light and dark browser chrome.
  'favicon.svg':
    `<svg ${xmlns} width="96" height="96" viewBox="0 0 96 96">` +
    style(`svg{${vars('dark')}}`) +
    `${brandTileInner(true, 'tab')}</svg>\n`,
};
// Square app icons: the full mark on the dark rounded tile, in each theme's mark roles.
for (const theme of ['light', 'dark'])
  outputs[`app-icon-${theme}.svg`] =
    `<svg ${xmlns} width="512" height="512" viewBox="0 0 96 96">` +
    style(`svg{${vars(theme)}}`) +
    `${brandTileInner(false, 'icon')}</svg>\n`;

let stale = [];
for (const [name, text] of Object.entries(outputs)) {
  const file = new URL(`../generated/${name}`, import.meta.url);
  if (mode === '--write') await writeFile(file, text);
  else if ((await readFile(file, 'utf8').catch(() => '')) !== text) stale.push(name);
}
if (stale.length) throw new Error(`Stale generated brand assets: ${stale.join(', ')}`);
