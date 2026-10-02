import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ownerOf, parseComponentMap } from '../../scripts/ci-scope.mjs';
const { runPackedCommand } = await import(
  new URL('../../scripts/packed-command.mjs', import.meta.url).href
);

const repository = fileURLToPath(new URL('../../../', import.meta.url));
const components = parseComponentMap(
  readFileSync(new URL('../../../.github/components.json', import.meta.url), 'utf8')
);
const layout = JSON.parse(
  readFileSync(new URL('../../../.github/repository-layout.json', import.meta.url), 'utf8')
) as { permanent: string[]; temporary: Record<string, { removeIn: string[] }> };
const entries = [...layout.permanent, ...Object.keys(layout.temporary)];
const allowed = new Set(entries);

function layoutViolations(
  files: readonly string[],
  map = components,
  topLevel = allowed
): string[] {
  const unowned = files
    .filter((file) => ownerOf(file, map) === 'unowned')
    .map((file) => `${file}: assign a component owner in .github/components.json`);
  const unexpected = [...new Set(files.map((file) => file.split('/')[0]))]
    .filter((entry) => !topLevel.has(entry))
    .map((entry) => `${entry}: use an existing home or propose a layout allowlist entry to infra`);
  return [...unowned, ...unexpected];
}

describe('repository layout', () => {
  it('gives every tracked file a component owner and an allowed top-level home', () => {
    const files = runPackedCommand('git', ['ls-files', '-z'], {
      cwd: repository,
      env: process.env,
    })
      .split('\0')
      .filter(Boolean);
    expect(files.length).toBeGreaterThan(0);
    expect(layoutViolations(files)).toEqual([]);
  });

  it('reports an unowned file even when its top-level home is allowed', () => {
    const map = parseComponentMap(JSON.stringify({ components: { cli: { owns: ['rust'] } } }));
    expect(layoutViolations(['retired/cli.rs'], map, new Set(['retired']))).toEqual([
      'retired/cli.rs: assign a component owner in .github/components.json',
    ]);
  });

  it('reports an unapproved home even when the component map owns it', () => {
    expect(layoutViolations(['unexpected/entry.ts'])).toEqual([
      'unexpected: use an existing home or propose a layout allowlist entry to infra',
    ]);
  });

  it('keeps permanent and temporary entries distinct and names each exception removal', () => {
    expect(new Set(entries).size).toBe(entries.length);
    for (const [entry, exception] of Object.entries(layout.temporary)) {
      expect(exception.removeIn.length, `${entry}: name the removal issue or PR`).toBeGreaterThan(
        0
      );
      for (const reference of exception.removeIn) expect(reference).toMatch(/^#[1-9]\d*$/);
    }
  });
});
