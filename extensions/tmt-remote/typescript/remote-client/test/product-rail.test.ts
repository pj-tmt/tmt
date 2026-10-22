import { readFileSync, readdirSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';

// The design gallery is a dev-preview rail item only (.poc on :5182): the product door serves no
// /design/ route, so no built script the door ships may link to it.
it('ships no Design rail item in the product bundles', () => {
  const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
  const scripts = readdirSync(assets).filter((name) => name.endsWith('.js'));
  expect(scripts).toContain('shell.js');
  for (const name of scripts)
    expect(readFileSync(new URL(name, assets), 'utf8'), name).not.toMatch(
      /\}\/design\/|["'`]\/design\/|id: ["']design["']|["']palette["']/,
    );
});
