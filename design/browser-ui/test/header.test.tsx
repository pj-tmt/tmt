import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { BrowserHeader } from '../src/react';

it('renders the handbook aperture as one hidden currentColor SVG inside a retained brand link', () => {
  const source = readFileSync(
    new URL('../../../site/src/home/assets/v9-0.svg', import.meta.url),
    'utf8',
  );
  const html = renderToStaticMarkup(
    <BrowserHeader
      productLabel="Colab"
      title="Pages"
      brandLink={(brand) => <a href="/pages">{brand}</a>}
    />,
  );
  expect(html).toContain('<a href="/pages"><svg');
  expect(html).toContain('class="tmt-ui-mark"');
  expect(html).toContain('viewBox="0 0 200 200"');
  expect(html).toContain('aria-hidden="true"');
  expect(html).toContain('fill="currentColor"');
  expect(html).not.toContain('style=');
  expect(html).not.toContain('<title');
  expect(html.match(/<svg/g)).toHaveLength(1);
  expect([...html.matchAll(/\bd="([^"]+)"/g)].map((match) => match[1])).toEqual(
    [...source.matchAll(/\bd="([^"]+)"/g)].map((match) => match[1]),
  );
  expect([...html.matchAll(/transform="([^"]+)"/g)].map((match) => match[1])).toEqual(
    [...source.matchAll(/transform="([^"]+)"/g)].map((match) => match[1]),
  );
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
