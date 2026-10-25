import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
// @ts-expect-error The generator is a plain script with no type declarations.
import { buildTheme, excluded } from '../../../scripts/generate-runtime-theme.mjs';

const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
);
const committed = readFileSync(new URL('../src/runtime/theme.css', import.meta.url), 'utf8');
const theme: string = buildTheme(tokens);

it('is the committed theme, so a token change must be regenerated', () => {
  expect(committed).toBe(theme);
});

it('names every browser colour and surface role except the excluded annotation paint', () => {
  for (const group of ['color', 'surface'] as const) {
    for (const key of Object.keys(tokens.browser[group])) {
      const declared = theme.includes(`--color-${key}: var(--tmt-${group}-${key});`);
      expect(declared, `${group}.${key}`).toBe(!excluded[group].includes(key));
    }
    // A stale entry would hide a role that no longer exists.
    for (const key of excluded[group]) expect(tokens.browser[group]).toHaveProperty(key);
  }
  expect(
    excluded.color
      .concat(excluded.surface)
      .every((key: string) => /^(annotation|mention)-/.test(key)),
  ).toBe(true);
});

it('switches every role with the app theme from the token light and dark values', () => {
  const block = (selector: string) =>
    theme.slice(theme.indexOf(`${selector} {`)).split('\n}')[0] ?? '';
  const light = block(':root');
  const dark = block(":root[data-theme='dark']");
  expect(theme).toContain(
    "@custom-variant dark (&:where([data-theme='dark'], [data-theme='dark'] *));",
  );
  for (const [group, key] of [
    ['color', 'text'],
    ['color', 'focus'],
    ['surface', 'paper'],
    ['surface', 'scrim'],
  ] as const) {
    const role = tokens.browser[group][key];
    expect(light).toContain(`--tmt-${group}-${key}: ${role.light.toLowerCase()};`);
    expect(dark).toContain(`--tmt-${group}-${key}: ${role.dark.toLowerCase()};`);
  }
});

it('drops the default palette and shadows and takes the radius, focus and fonts from the tokens', () => {
  for (const reset of [
    '--color-*',
    '--shadow-*',
    '--inset-shadow-*',
    '--drop-shadow-*',
    '--radius-*',
  ])
    expect(theme).toContain(`${reset}: initial;`);
  const colors = [...theme.matchAll(/^ {2}--color-([\w-]+):/gm)].map((match) => match[1]);
  expect(colors).toEqual(expect.arrayContaining(['black', 'white', 'transparent', 'current']));
  expect(colors).not.toContain('red-500');
  expect(theme).toContain(`--radius: ${tokens.browser.metric.radius};`);
  expect(theme).toContain(`--radius-lg: ${tokens.browser.metric.radius};`);
  expect(theme).toContain(
    `outline: ${tokens.browser.metric['focus-width']} solid var(--color-focus);`,
  );
  expect(theme).toContain('outline-offset: 0;');
  // Only the system part of each stack: the leading quoted web family cannot load in the frame.
  for (const [name, utility] of [
    ['body', 'sans'],
    ['mono', 'mono'],
    ['display', 'display'],
  ] as const) {
    const stack: string = tokens.font[name].stack;
    expect(stack).toMatch(/^"[^"]+",/);
    const system = stack.replace(/^"[^"]+",\s*/, '').replaceAll('"', "'");
    expect(theme).toContain(`--font-${utility}: ${system};`);
  }
});

it('stays source for a text/tailwindcss style block', () => {
  // An @import would make the browser build skip its own default import of Tailwind.
  expect(theme).not.toContain('@import');
  expect(theme).not.toContain('</style');
  expect(theme.length).toBeLessThan(16 * 1024);
});

it('refuses a token set it cannot map', () => {
  const clone = () => structuredClone(tokens);
  const clash = clone();
  clash.browser.surface.text = clash.browser.surface.paper;
  expect(() => buildTheme(clash)).toThrow(/both define "text"/);
  const noFocus = clone();
  delete noFocus.browser.color.focus;
  expect(() => buildTheme(noFocus)).toThrow(/focus/);
  const noWebFont = clone();
  noWebFont.font.mono.stack = 'ui-monospace, monospace';
  expect(() => buildTheme(noWebFont)).toThrow(/font\.mono/);
});
