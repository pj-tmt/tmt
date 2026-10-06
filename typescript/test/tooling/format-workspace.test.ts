import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
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

  it('selects current and nested design Markdown without widening code or other design files', () => {
    const design = fileURLToPath(new URL('../../../design/', import.meta.url));
    const fixture = mkdtempSync(path.join(design, 'format-fixture-'));
    const codeBefore = formatterTargets();
    try {
      const nested = path.join(fixture, 'nested');
      mkdirSync(nested);
      const guide = path.join(fixture, 'guide.md');
      const nestedGuide = path.join(nested, 'guide.md');
      writeFileSync(guide, '# Guide\n');
      writeFileSync(nestedGuide, '# Nested\n');
      const excluded = ['component.tsx', 'tokens.json', 'component.css', 'guide.mdx'].map((name) =>
        path.join(nested, name)
      );
      for (const target of excluded) writeFileSync(target, 'fixture\n');
      const docs = formatterTargets(true);
      expect(docs).toContain(path.join(design, 'cli-style.md'));
      expect(docs).toContain(path.join(design, 'gui-style.md'));
      expect(docs).toContain(guide);
      expect(docs).toContain(nestedGuide);
      for (const target of excluded) expect(docs).not.toContain(target);
      expect(new Set(docs).size).toBe(docs.length);
      expect(formatterTargets()).toEqual(codeBefore);
    } finally {
      rmSync(fixture, { recursive: true, force: true });
    }
    expect(formatterTargets(true).some((target: string) => target.startsWith(fixture))).toBe(false);
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
