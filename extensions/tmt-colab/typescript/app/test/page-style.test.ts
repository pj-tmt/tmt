import { expect, it } from 'vite-plus/test';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const repository = new URL('../../../../../', import.meta.url);
const generator = 'extensions/tmt-colab/scripts/generate-page-style.mjs';
const skillFile = 'extensions/tmt-colab/skills/tmt-colab/SKILL.md';
const tokenFile = 'design/tokens/tokens.json';
const begin = '<!-- BEGIN generated page style -->';
const end = '<!-- END generated page style -->';
const read = (file: string) => readFileSync(new URL(file, repository), 'utf8');
const run = (file: string, mode = '--check') =>
  execFileSync(process.execPath, [file, mode], { timeout: 5_000, stdio: 'pipe' });
const starterFrom = (skill: string) =>
  skill.split(begin)[1].split(end)[0].split('```html\n')[1].split('```')[0];

it('keeps dark secondary text distinct from body text and readable on page surfaces', () => {
  const { color, surface } = JSON.parse(read(tokenFile)).browser;
  const luminance = (hex: string) =>
    [1, 3, 5].reduce((sum, offset, index) => {
      const channel = parseInt(hex.slice(offset, offset + 2), 16) / 255;
      const linear = channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
      return sum + linear * [0.2126, 0.7152, 0.0722][index];
    }, 0);
  expect(color.secondary.dark).not.toBe(color.text.dark);
  for (const background of [surface.paper.dark, surface.sheet.dark]) {
    expect(
      (luminance(color.secondary.dark) + 0.05) / (luminance(background) + 0.05),
    ).toBeGreaterThanOrEqual(4.5);
  }
});

it('ships a compact content-only starter with exact browser token roles and both themes', () => {
  const skill = read(skillFile);
  const starter = starterFrom(skill);
  const tokens = JSON.parse(read(tokenFile));
  run(new URL(generator, repository).pathname);
  expect(read(skillFile)).toBe(skill);
  expect(Buffer.byteLength(starter)).toBeLessThanOrEqual(3_000);
  expect(Buffer.byteLength(skill)).toBeLessThanOrEqual(12_000);
  expect(skill.split('\n').length - 1).toBeLessThanOrEqual(500);
  expect(skill).toContain('## Page look');
  expect(skill.indexOf('## Page look')).toBeLessThan(skill.indexOf('## HTML that renders'));
  expect(skill).toContain('Colab already shows the brand, page title and actions.');
  expect(skill).toContain('Do not add a site header,');
  expect(skill).toContain('any sticky or fixed bar.');
  expect(skill).toContain("Use the starter's explicit data-theme selectors to\nfollow Colab.");
  expect(skill).toContain('CSS keyed only on prefers-color-scheme follows the OS.');

  const declarations = (selector: string) => {
    const block = starter.slice(starter.indexOf(selector) + selector.length).split('}')[0];
    return Object.fromEntries(
      [...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2]]),
    );
  };
  for (const [theme, selector] of [
    ['light', ':root {'],
    ['light', ":root[data-theme='light'] {"],
    ['dark', ":root:not([data-theme='light']) {"],
    ['dark', ":root[data-theme='dark'] {"],
  ]) {
    expect(declarations(selector)).toMatchObject({
      ink: tokens.browser.color.text[theme].toLowerCase(),
      muted: tokens.browser.color.secondary[theme].toLowerCase(),
      paper: tokens.browser.surface.paper[theme].toLowerCase(),
      sheet: tokens.browser.surface.sheet[theme].toLowerCase(),
      edge: tokens.browser.surface.rule[theme].toLowerCase(),
    });
  }
  expect(declarations(':root {')).toMatchObject({
    body: tokens.font.body.stack.replace(/^"[^"]+",\s*/, '').replaceAll('"', "'"),
    mono: tokens.font.mono.stack.replace(/^"[^"]+",\s*/, '').replaceAll('"', "'"),
    size: tokens.header['wordmark-size'],
    heading: '28px',
    rule: tokens.browser.metric['rule-width'],
    gap: tokens.browser.metric['notice-heading-gap'],
    'small-gap': tokens.browser.metric['eyebrow-gap'],
  });
  expect(starter).toContain(`line-height: ${tokens.header['title-line-height']};`);
  expect(starter).toContain('@media (prefers-color-scheme: dark)');
  expect(starter).not.toMatch(
    /url\s*\(|@import|https?:|\/\/|shadow|radius|position\s*:|header|nav|chrome|tmt-ui/i,
  );
  // Only content elements, theme roots and a muted-text utility belong in the starter.
  const selectors = [...starter.matchAll(/([^{}]+)\{/g)].map((m) => m[1].trim());
  expect(selectors).toEqual([
    '<style>\n  :root',
    '@media (prefers-color-scheme: dark)',
    ":root:not([data-theme='light'])",
    ":root[data-theme='light']",
    ":root[data-theme='dark']",
    '*',
    'body',
    'h1,\n  h2,\n  h3',
    'h1',
    'h2',
    'h3',
    'p,\n  ul,\n  ol,\n  table,\n  pre',
    'ul,\n  ol',
    'a',
    'table',
    'th,\n  td',
    'code,\n  pre',
    'pre',
    'section',
    '.muted',
  ]);
});

it('fails non-writing checks for stale, missing, duplicate and changed-token regions; write updates only the region', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'colab-page-style-'));
  try {
    for (const file of [generator, skillFile, tokenFile]) {
      const output = path.join(root, file);
      mkdirSync(path.dirname(output), { recursive: true });
      cpSync(new URL(file, repository), output);
    }
    const script = path.join(root, generator);
    const file = path.join(root, skillFile);
    const skill = read(skillFile);
    for (const invalid of [
      skill.replace('--ink: #343434;', '--ink: #000000;'),
      skill.replace(begin, ''),
      skill.replace(end, ''),
      skill + begin,
      skill + end,
    ]) {
      writeFileSync(file, invalid);
      expect(() => run(script)).toThrow();
      expect(readFileSync(file, 'utf8')).toBe(invalid);
    }
    writeFileSync(file, skill);
    const tokens = JSON.parse(read(tokenFile));
    tokens.browser.surface.paper.dark = '#090909';
    writeFileSync(path.join(root, tokenFile), JSON.stringify(tokens));
    expect(() => run(script)).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(skill);
    run(script, '--write');
    run(script);
    const updated = readFileSync(file, 'utf8');
    expect(updated.split(begin)[0]).toBe(skill.split(begin)[0]);
    expect(updated.split(end)[1]).toBe(skill.split(end)[1]);
    expect(starterFrom(updated)).toContain('--paper: #090909;');
    expect(() => run(script, '--repair')).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(updated);
    for (const oversized of ['x'.repeat(12_000) + updated, '\n'.repeat(500) + updated]) {
      writeFileSync(file, oversized);
      expect(() => run(script, '--write')).toThrow();
      expect(readFileSync(file, 'utf8')).toBe(oversized);
    }
    writeFileSync(file, updated);
    tokens.font.body.stack += ', ' + 'x'.repeat(3_000);
    writeFileSync(path.join(root, tokenFile), JSON.stringify(tokens));
    expect(() => run(script, '--write')).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(updated);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
