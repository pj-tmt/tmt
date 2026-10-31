import markCss from '../src/brand-mark.css?raw';
import { brandTileInner } from '../src/brand-mark-svg.js';

/** The 5100 tab icon in palette B (#5821): the favicon's tile and small mark, painted with the mark
 * roles the B scope resolves on this page, so the tab shows the same red mark as the top bar. */
export function setPaletteBFavicon() {
  const computed = getComputedStyle(document.documentElement);
  const roles = [...markCss.matchAll(/var\((--tmt-ui-brand-mark-[\w-]+)\)/g)].map((m) => m[1]!);
  const vars = [...new Set(roles)]
    .map((name) => `${name}:${computed.getPropertyValue(name).trim()}`)
    .join(';');
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">` +
    `<style>svg{${vars}}${markCss.split('.tmt-brand-lockup {')[0]}</style>${brandTileInner(true, 'tab')}</svg>`;
  let link = document.querySelector<HTMLLinkElement>('link[rel=icon]');
  if (!link) {
    link = document.createElement('link');
    link.rel = 'icon';
    document.head.append(link);
  }
  link.href = `data:image/svg+xml,${encodeURIComponent(svg)}`;
}
