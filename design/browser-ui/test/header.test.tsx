import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { BrowserHeader } from '../src/react';

it('renders the shared 04D-1 mark, exactly as its static-host copy, inside a retained brand link', () => {
  const copy = readFileSync(new URL('../generated/brand-mark-static.svg', import.meta.url), 'utf8');
  const html = renderToStaticMarkup(
    <BrowserHeader
      productLabel="Colab"
      title="Pages"
      brandLink={(brand) => <a href="/pages">{brand}</a>}
    />,
  );
  // Gradient ids are per instance; everything else is byte-equal.
  const sameIds = (markup: string) =>
    markup
      .replace(/\s+/g, ' ')
      .replace(/ ?> ?</g, '><')
      .replace(/ >/g, '>')
      .replace(
        /(id="|url\(#)[^")]*-(aurora|far|panel|mono|halo-weight|halo-mask|feather|feather-mask|spill|sheen|rim|panel-clip|halo|soft)(?=[")])/g,
        '$1$2',
      );
  expect(html).toContain('<a href="/pages"><svg');
  expect(html.match(/<svg/g)).toHaveLength(1);
  expect(sameIds(html.match(/<svg[\s\S]*?<\/svg>/)![0])).toBe(sameIds(copy.trim()));
  expect(html).not.toContain('style=');
  expect(html).not.toContain('<title');
  expect(html).toContain('tmt-ui-wordmark">Colab</span>');
  expect(html.match(/<h1/g)).toHaveLength(1);
});

it('keeps linked and plain labels neutral and removes the section divider', () => {
  const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
  const block = (selector: string) => {
    expect(css).toContain(`${selector} {`);
    return css.slice(css.indexOf(`${selector} {`)).split('}')[0];
  };
  expect(block('.tmt-ui-brand > a')).toContain('color: var(--tmt-ui-color-text)');
  expect(block('.tmt-ui-wordmark')).toContain('color: var(--tmt-ui-color-text)');
  expect(block('.tmt-ui-heading,\n.tmt-ui-header > .tmt-ui-title')).not.toMatch(
    /border-left|padding-left/,
  );
});
