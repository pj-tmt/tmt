import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserChoice } from '../src/choice.js';

const css = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
const tokens = JSON.parse(
  readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);

const hex = (value: string) => [1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16));
const lum = ([r, g, b]: number[]) => {
  const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r!) + 0.7152 * lin(g!) + 0.0722 * lin(b!);
};
const ratio = (a: number[], b: number[]) => {
  const [hi, lo] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (hi! + 0.05) / (lo! + 0.05);
};

/** The role the hint rule reads for a property, resolved to the theme's colour. */
function role(property: 'background' | 'color', theme: 'light' | 'dark') {
  const rule = new RegExp(
    `\\.tmt-ui-choice-hint \\{[^}]*?(?<![-\\w])${property}: var\\(--tmt-ui-(surface|color)-([a-z-]+)\\)`,
  ).exec(css);
  expect(rule, `hint ${property}`).not.toBeNull();
  return hex(tokens.browser[rule![1]!][rule![2]!][theme]);
}

it('ends the row with the hint as a key that assistive technology skips', () => {
  const html = renderToStaticMarkup(
    <BrowserChoice type="radio" name="pick" label="Compact rail" hint="2" />,
  );
  expect(html).toMatch(/<kbd class="tmt-ui-choice-hint" aria-hidden="true">2<\/kbd><\/label>$/);
  // The key sits inside the label, so the control is named by the label text and not "Compact rail2".
  expect(html).toContain('aria-label="Compact rail"');
  expect(html).toContain('<span>Compact rail</span>');
});

it('draws no key when the row has no hint, for a check row as well', () => {
  for (const type of ['radio', 'checkbox'] as const) {
    const html = renderToStaticMarkup(<BrowserChoice type={type} label="Replies" />);
    expect(html).not.toContain('<kbd');
    expect(html).not.toContain('aria-label');
  }
  expect(
    renderToStaticMarkup(<BrowserChoice type="checkbox" label="Replies" hint="1" />),
  ).toContain('<kbd class="tmt-ui-choice-hint" aria-hidden="true">1</kbd>');
});

it('keeps the hint legible, 4.5:1 on its key cap, in light and dark', () => {
  for (const theme of ['light', 'dark'] as const) {
    expect(ratio(role('color', theme), role('background', theme)), theme).toBeGreaterThanOrEqual(
      4.5,
    );
  }
});
