import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vite-plus/test';
import { cn, UiTheme, useUiPortalContainer } from '../src/ui/index.js';
import { assertUiUtilities } from '../scripts/ui-source-policy.mjs';
import hashes from './fixtures/ui/legacy-hashes.json';

it('retains exact existing Browser sources and checked static CSS from the accepted base', () => {
  for (const [file, expected] of Object.entries(hashes.sha256)) {
    const bytes = readFileSync(new URL(`../${file}`, import.meta.url));
    expect(createHash('sha256').update(bytes).digest('hex'), file).toBe(expected);
  }
});
it('merges conflicting prefixed classes while preserving variants and unrelated legacy classes', () => {
  expect(cn('tmt:p-2 tmt:text-foreground', ['tmt:p-4'], { 'tmt:flex': true }, false)).toBe(
    'tmt:text-foreground tmt:p-4 tmt:flex',
  );
  expect(cn('tmt:p-2 tmt:hover:p-4', 'tmt:hover:p-2', 'tmt-ui-field')).toBe(
    'tmt:p-2 tmt:hover:p-2 tmt-ui-field',
  );
});
it('does not invent a body portal before its own island container is mounted', () => {
  function PortalConsumer() {
    return <span>{useUiPortalContainer() === null ? 'Waiting for own container' : 'Ready'}</span>;
  }
  const html = renderToStaticMarkup(
    <UiTheme theme="dark">
      <PortalConsumer />
    </UiTheme>,
  );
  expect(html).toContain('data-tmt-theme="dark"');
  expect(html).toContain('Waiting for own container');
  expect(html).toContain('data-tmt-ui-portal=""');
  expect(() => renderToStaticMarkup(<PortalConsumer />)).toThrow('originating UiTheme');
});
it('publishes only a separate scoped namespace, without preflight, global variables or global registrations', () => {
  const css = readFileSync(new URL('../generated/ui.css', import.meta.url), 'utf8');
  expect(css).not.toMatch(/:root|@property|@import|@theme|@tailwind|@layer|--tw-|--tmt-ui-/);
  expect(() => assertUiUtilities(css)).not.toThrow();
  expect(css).toContain(':where([data-tmt-ui]) {');
  expect(css).toContain('.tmt\\:bg-background');
  expect(css).not.toMatch(/box-shadow:/);
  const radiusValues = [...css.matchAll(/border-radius:\s*([^;]+);/g)].map((match) => match[1]);
  expect(radiusValues.length).toBeGreaterThan(0);
  expect(radiusValues.every((value) => value === 'var(--tmt-foundation-radius)')).toBe(true);
  const tokens = JSON.parse(
    readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
  );
  for (const role of ['paper', 'sheet'])
    for (const theme of ['light', 'dark'])
      expect(css).toContain(tokens.browser.surface[role][theme].toLowerCase());
  for (const role of ['edge', 'focus', 'text'])
    for (const theme of ['light', 'dark'])
      expect(css).toContain(tokens.browser.color[role][theme].toLowerCase());
  for (const [key, value] of Object.entries(tokens.browser.ui))
    expect(css).toContain(`--tmt-foundation-${key}: ${value};`);
});

it('leaves every existing static token projection unchanged', () => {
  const tokens = JSON.parse(
    readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
  );
  expect({
    font: tokens.font,
    header: tokens.header,
    browser: {
      color: tokens.browser.color,
      surface: tokens.browser.surface,
      metric: tokens.browser.metric,
    },
  }).toEqual(hashes.tokenProjection);
});

it('keeps original source export targets and direct dependency pins explicit', () => {
  const manifest = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  const provenance = JSON.parse(
    readFileSync(new URL('../src/ui/provenance.json', import.meta.url), 'utf8'),
  );
  expect(manifest.exports['./react']).toBe('./src/react.ts');
  expect(manifest.exports['./static']).toBe('./src/static.ts');
  expect(manifest.exports['./static.css']).toBe('./generated/static.css');
  expect(manifest.exports['./ui']).toBe('./src/ui/index.ts');
  expect(manifest.exports['./ui.css']).toBe('./generated/ui.css');
  for (const [name, value] of Object.entries(provenance.dependencies) as [
    string,
    { version: string; license: string },
  ][])
    expect(
      manifest.dependencies?.[name] ??
        manifest.devDependencies?.[name] ??
        manifest.peerDependencies?.[name],
    ).toBe(value.version);
  const cva = provenance.dependencies['class-variance-authority'];
  expect(cva.version).toBe('0.7.1');
  expect(cva.license).toBe('Apache-2.0');
  expect(
    createHash('sha256')
      .update(readFileSync(new URL('../src/ui/LICENSE.cva', import.meta.url)))
      .digest('hex'),
  ).toBe(cva.licenseSha256);
  const license = readFileSync(new URL('../src/ui/LICENSE.shadcn', import.meta.url));
  expect(createHash('sha256').update(license).digest('hex')).toBe(
    provenance.files.find((file: { path: string }) => file.path === 'LICENSE.md').sha256,
  );
});
