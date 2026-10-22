import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserAvatar } from '../src/avatar.js';

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

/** The role an avatar rule reads for a property, resolved to the theme's colour. */
function role(selector: string, property: 'background' | 'color', theme: 'light' | 'dark') {
  const rule = new RegExp(
    `${selector} \\{[^}]*?(?<![-\\w])${property}: var\\(--tmt-ui-(surface|color)-([a-z-]+)\\)`,
  ).exec(css);
  expect(rule, `${selector} ${property}`).not.toBeNull();
  return hex(tokens.browser[rule![1]!][rule![2]!][theme]);
}

const person = '\\.tmt-ui-avatar';
const agent = "\\.tmt-ui-avatar\\[data-kind='agent'\\]";

it('marks the kind on the element and defaults to a person', () => {
  expect(renderToStaticMarkup(<BrowserAvatar name="Ben" />)).toContain('data-kind="person"');
  expect(renderToStaticMarkup(<BrowserAvatar name="core-3" kind="agent" />)).toContain(
    'data-kind="agent"',
  );
});

it('draws a person and an agent in different fills that both keep 4.5:1 in light and dark', () => {
  for (const theme of ['light', 'dark'] as const) {
    const personFill = role(person, 'background', theme);
    const agentFill = role(agent, 'background', theme);
    expect(personFill, theme).not.toEqual(agentFill);
    expect(
      ratio(role(person, 'color', theme), personFill),
      `${theme} person`,
    ).toBeGreaterThanOrEqual(4.5);
    expect(ratio(role(agent, 'color', theme), agentFill), `${theme} agent`).toBeGreaterThanOrEqual(
      4.5,
    );
  }
});
