import { expect, it } from 'vite-plus/test';
import { readFileSync, readdirSync } from 'node:fs';

/** #5821: the shadcn set is skinned only through the token roles, with no shadcn default palette or
 * radius left, in its sources and in the utilities it ships. */
const literalColour = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch)\(/i;
const defaultRadius = /\brounded(?:-(?:none|xs|sm|md|lg|xl|2xl|3xl|full))?\b(?![-\w(])/;
const shadcnTheme =
  /var\(--(?:radius|background|foreground|primary|secondary|muted|accent|destructive|border|input|ring|card|popover)\b/;

const directory = new URL('../src/ui/', import.meta.url);
const sources = readdirSync(directory)
  .filter((name) => name.endsWith('.tsx') || name.endsWith('.ts'))
  .map((name) => [name, readFileSync(new URL(name, directory), 'utf8')] as const);

it('writes no colour, default radius or shadcn theme variable in the ui sources', () => {
  expect(sources.length).toBeGreaterThan(20);
  for (const [name, source] of sources) {
    const code = source.replace(/\/\/.*$|\/\*[\s\S]*?\*\//gm, '');
    expect(code, name).not.toMatch(literalColour);
    expect(code, name).not.toMatch(defaultRadius);
    expect(code, name).not.toMatch(shadcnTheme);
  }
  for (const part of ['select', 'skeleton', 'empty', 'card'])
    expect(
      sources.some(([name]) => name === `${part}.tsx`),
      part,
    ).toBe(true);
});

it('ships utilities that read only foundation roles for colour and radius', () => {
  const css = readFileSync(new URL('../generated/ui.css', import.meta.url), 'utf8');
  const utilities = css.slice(css.indexOf(':where([data-tmt-ui]) {'));
  expect(utilities.length).toBeGreaterThan(1000);
  // Values live only in the foundation token blocks above; a utility reads a role by name.
  const declarations = [...utilities.matchAll(/(?<![\w-])([a-z-]+):\s*([^;{}]+);/g)].filter(
    ([, property]) => !property!.startsWith('--'),
  );
  for (const [declaration, property, value] of declarations) {
    if (/color|background|border|outline|fill|stroke|shadow/.test(property!))
      expect(value, declaration).not.toMatch(literalColour);
    if (property === 'border-radius')
      expect(value, declaration).toMatch(/^var\(--tmt-foundation-radius(?:-s|-l|-pill)?\)$/);
  }
  expect(utilities).not.toMatch(shadcnTheme);
});
