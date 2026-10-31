import { expect, it } from 'vite-plus/test';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const repository = new URL('../../../../../', import.meta.url);
const generator = 'extensions/tmt-colab/scripts/generate-page-style.mjs';
const skillFile = 'extensions/tmt-colab/skills/tmt-colab/references/page-style.md';
const rootSkillFile = 'extensions/tmt-colab/skills/tmt-colab/SKILL.md';
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

it('ships a compact starter that is only glue on the injected design system', () => {
  const skill = read(skillFile);
  const starter = starterFrom(skill);
  const style = starter.slice(0, starter.indexOf('<!-- Examples to copy'));
  const tokens = JSON.parse(read(tokenFile));
  const { metric } = tokens.browser;
  run(new URL(generator, repository).pathname);
  expect(read(skillFile)).toBe(skill);
  expect(Buffer.byteLength(style)).toBeLessThanOrEqual(2_000);
  expect(Buffer.byteLength(read(rootSkillFile))).toBeLessThanOrEqual(10_300);
  expect(Buffer.byteLength(skill)).toBeLessThanOrEqual(12_000);
  expect(skill.split('\n').length - 1).toBeLessThanOrEqual(500);
  expect(skill).toContain('## Page look');
  expect(read(rootSkillFile)).toContain('references/page-style.md');
  expect(read(rootSkillFile)).toContain('tmt colab skill page-style');
  expect(skill).toContain('Colab already shows the brand, page title and actions.');
  expect(skill).toContain('Do not add a site header,');
  expect(skill).toContain('any sticky or fixed bar.');
  expect(skill).toContain('colour with the `--tmt-ui-*`');
  expect(skill).toContain('CSS keyed only on prefers-color-scheme follows the OS.');

  // Type, rhythm, tables and code come from the injected editorial layer (#5794); the starter's own
  // spacing comes from the design tokens, not from numbers of its own.
  for (const declaration of [
    `padding: ${metric['notice-heading-gap']};`,
    `section { margin: 0 0 ${metric['notice-heading-gap']}; }`,
  ]) {
    expect(style).toContain(declaration);
  }
  expect(style).not.toMatch(/\bh[1-6]\b|\bpre\b|\btable\b|font/);

  // It sits in the starter layer between the frame's and the design system's, owns no palette and
  // reads only roles the injected design system defines.
  expect(style).toMatch(/^<style>\n@layer tmt-starter \{\n/);
  const staticCss = read('design/browser-ui/generated/static.css');
  const roles = [...style.matchAll(/var\((--tmt-ui-[\w-]+)\)/g)].map((m) => m[1]);
  expect(roles.length).toBeGreaterThan(0);
  for (const role of roles) expect(staticCss, role).toContain(`${role}:`);
  expect(style).not.toMatch(
    /#[0-9a-f]{3,8}\b|:root|@media|prefers-color-scheme|data-theme|url\s*\(|@import|https?:|\/\/|shadow|position\s*:|header|nav|chrome/i,
  );
  // Only the page column, section spacing and two text-colour utilities belong in the starter.
  const selectors = [...style.matchAll(/([^{}]+)\{/g)].map((m) => m[1].trim());
  expect(selectors).toEqual([
    '<style>\n@layer tmt-starter',
    '*',
    'body',
    'section',
    'section > :last-child',
    '.muted',
    '.waiting',
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
      skill.replace(
        'section > :last-child { margin-bottom: 0; }',
        'section > :last-child { margin-bottom: 1px; }',
      ),
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
    tokens.browser.metric['notice-heading-gap'] = '18px';
    writeFileSync(path.join(root, tokenFile), JSON.stringify(tokens));
    expect(() => run(script)).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(skill);
    run(script, '--write');
    run(script);
    const updated = readFileSync(file, 'utf8');
    expect(updated.split(begin)[0]).toBe(skill.split(begin)[0]);
    expect(updated.split(end)[1]).toBe(skill.split(end)[1]);
    expect(starterFrom(updated)).toContain('padding: 18px;');
    expect(() => run(script, '--repair')).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(updated);
    for (const oversized of ['x'.repeat(12_000) + updated, '\n'.repeat(500) + updated]) {
      writeFileSync(file, oversized);
      expect(() => run(script, '--write')).toThrow();
      expect(readFileSync(file, 'utf8')).toBe(oversized);
    }
    writeFileSync(file, updated);
    tokens.browser.metric['notice-heading-gap'] += 'x'.repeat(3_000);
    writeFileSync(path.join(root, tokenFile), JSON.stringify(tokens));
    expect(() => run(script, '--write')).toThrow();
    expect(readFileSync(file, 'utf8')).toBe(updated);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
