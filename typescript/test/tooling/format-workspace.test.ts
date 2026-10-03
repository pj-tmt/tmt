import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { expandFormatterTargets, formatterTargets } from '../../scripts/format-workspace.mjs';

describe('workspace formatter selection', () => {
  it('expands parent-relative and nested globs to canonical paths without widening docs', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'tmt-format-'));
    try {
      const tooling = path.join(root, 'typescript');
      mkdirSync(tooling);
      mkdirSync(path.join(root, 'docs', 'nested'), { recursive: true });
      writeFileSync(path.join(root, 'docs', 'guide.md'), '# Guide\n');
      writeFileSync(path.join(root, 'docs', 'nested', 'guide.md'), '# Nested\n');
      writeFileSync(path.join(root, 'docs', 'nested', 'data.json'), '{}\n');
      expect(expandFormatterTargets(['../docs/**/*.md', '../docs/guide.md'], tooling)).toEqual([
        path.join(root, 'docs', 'guide.md'),
        path.join(root, 'docs', 'nested', 'guide.md'),
      ]);
      expect(() => expandFormatterTargets(['../missing/**/*.md'], tooling)).toThrow(
        'Formatter target matched no files'
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it('retains the code tree and the separate canonical guidance/docs inventory', () => {
    const code = formatterTargets();
    const docs = formatterTargets(true);
    expect(code.some((target: string) => target.endsWith('/typescript/test'))).toBe(true);
    expect(code.some((target: string) => target.endsWith('/contracts/extension-api.md'))).toBe(
      false
    );
    expect(docs.some((target: string) => target.endsWith('/contracts/extension-api.md'))).toBe(
      true
    );
    expect(docs.some((target: string) => target.endsWith('/.agents/skills/tmt-dev/SKILL.md'))).toBe(
      true
    );
    expect(docs.every((target: string) => target.endsWith('.md'))).toBe(true);
    expect([...code, ...docs].every((target: string) => path.isAbsolute(target))).toBe(true);
  });
});
