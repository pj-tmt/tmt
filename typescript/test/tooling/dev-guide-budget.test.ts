import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';

// DEVELOPMENT.md holds only cross-cutting material; per-area procedures belong in
// .agents/skills/<area>/ (see its area table). Raise the budget only with an issue
// that explains what moved into the shared guide.
const DEVELOPMENT_LINE_BUDGET = 600;

export function lineCount(text: string): number {
  return text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length;
}

describe('shared development guide budget', () => {
  it('counts lines without the final newline', () => {
    expect(lineCount('a\nb\n')).toBe(2);
    expect(lineCount('a\nb')).toBe(2);
    expect(lineCount('')).toBe(1);
  });

  it('keeps DEVELOPMENT.md within its line budget', () => {
    const text = readFileSync(new URL('../../../DEVELOPMENT.md', import.meta.url), 'utf8');
    expect(lineCount(text)).toBeLessThanOrEqual(DEVELOPMENT_LINE_BUDGET);
  });

  it('fails when the guide grows past the budget', () => {
    const grown = `${'line\n'.repeat(DEVELOPMENT_LINE_BUDGET)}one more\n`;
    expect(lineCount(grown)).toBeGreaterThan(DEVELOPMENT_LINE_BUDGET);
  });
});
