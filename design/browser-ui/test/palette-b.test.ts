import { expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';

const tokens = JSON.parse(
  readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
);
const { ref, color, surface } = tokens.browser;
const palette = tokens.browser['palette-b'];
const reads = (value: string) => [...value.matchAll(/\{([\w-]+)\}/g)].map((match) => match[1]!);
const themed = (roles: Record<string, { dark: string; light: string }>) =>
  Object.entries(roles).filter(([name]) => !name.startsWith('$'));

it('keeps the primitive tier to raw values', () => {
  for (const [name, value] of Object.entries(ref))
    if (!name.startsWith('$'))
      expect(String(value), name).toMatch(/^(#[0-9a-f]{6}([0-9a-f]{2})?|\d+px)$/);
});

it('maps palette B onto existing roles, reading only refs that exist', () => {
  for (const [group, roles] of [
    ['color', palette.color],
    ['surface', palette.surface],
  ] as const)
    for (const [role, value] of themed(roles)) {
      expect(tokens.browser[group][role], `${group}.${role}`).toBeDefined();
      for (const name of [...reads(value.dark), ...reads(value.light)])
        expect(ref[name], `${group}.${role} reads ${name}`).toBeDefined();
    }
  for (const [role, value] of themed(palette.shadow)) {
    expect(tokens.browser.shadow[role], `shadow.${role}`).toBeDefined();
    for (const name of [...reads(value.dark), ...reads(value.light)])
      expect(ref[name], `shadow.${role} reads ${name}`).toBeDefined();
  }
  for (const [role, value] of themed(palette['brand-mark'])) {
    expect(tokens.brand.mark[role], `brand-mark.${role}`).toBeDefined();
    for (const name of [...reads(value.dark), ...reads(value.light)])
      expect(ref[name], `brand-mark.${role} reads ${name}`).toBeDefined();
  }
  for (const [role, value] of Object.entries(palette.metric) as [string, string][]) {
    expect(tokens.browser.metric[role], role).toBeDefined();
    for (const name of reads(value)) expect(ref[name], `${role} reads ${name}`).toBeDefined();
  }
  // Chart series and seats are data colours, not the palette: B leaves them as they are.
  expect(Object.keys(palette.color).some((role) => /^(series|seat)-/.test(role))).toBe(false);
  expect(color).toBeDefined();
  expect(surface).toBeDefined();
});

it('never colours a state with the accent primitive', () => {
  const accent = new Set(
    ['accent', 'accent-strong'].flatMap((role) => [
      ...reads(palette.color[role].dark),
      ...reads(palette.color[role].light),
    ]),
  );
  expect(accent.size).toBeGreaterThan(0);
  const status = [
    ...['working', 'waiting', 'review', 'blocked', 'focus-destructive'].map(
      (role) => palette.color[role],
    ),
    ...['working-soft', 'waiting-soft', 'review-soft', 'blocked-soft'].map(
      (role) => palette.surface[role],
    ),
  ];
  for (const value of status)
    for (const name of [...reads(value.dark), ...reads(value.light)]) {
      expect(accent.has(name), name).toBe(false);
      expect(ref[name], name).not.toBe(ref['red-500']);
    }
});

it('maps every colour and surface role, so nothing under B falls back to the old palette', () => {
  // Chart series and seats are data colours, kept on purpose; elevation-0 is no shadow at all.
  for (const group of ['color', 'surface'] as const)
    for (const role of Object.keys(tokens.browser[group]).filter(
      (name) => !name.startsWith('$') && !/^(series|seat)-/.test(name),
    ))
      expect(palette[group][role], `${group}.${role}`).toBeDefined();
  for (const role of Object.keys(tokens.browser.shadow).filter(
    (name) => !name.startsWith('$') && name !== 'elevation-0',
  ))
    expect(palette.shadow[role], `shadow.${role}`).toBeDefined();
});

it('keeps the docs and the Colab mock free of literal colours', () => {
  for (const file of ['docs.css', 'mock-colab.css', 'mock-reader.css', 'mock-list.css']) {
    const css = readFileSync(new URL(`../docs/${file}`, import.meta.url), 'utf8').replace(
      /\/\*[\s\S]*?\*\//g,
      '',
    );
    expect(css, file).not.toMatch(/#[0-9a-f]{3,8}\b|\b(?:rgb|hsl|oklch)a?\(/i);
  }
});
