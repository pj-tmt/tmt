import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';

// The shared guides hold only cross-cutting material. Per-area procedures belong in
// .agents/skills/<area>/ (DEVELOPMENT.md's area table) and per-module architecture in
// .agents/skills/tmt-core-runtime and the other area skills (ARCHITECTURE.md's owner
// maps). Raise a budget only with an issue that explains what moved into the guide.
const BUDGETS = {
  'DEVELOPMENT.md': 600,
  'ARCHITECTURE.md': 1250,
} as const;

export function lineCount(text: string): number {
  return text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length;
}

export function withinBudget(text: string, budget: number): boolean {
  return lineCount(text) <= budget;
}

describe('shared guide budgets', () => {
  it('counts lines without the final newline', () => {
    expect(lineCount('a\nb\n')).toBe(2);
    expect(lineCount('a\nb')).toBe(2);
    expect(lineCount('')).toBe(1);
  });

  for (const [guide, budget] of Object.entries(BUDGETS)) {
    it(`keeps ${guide} within its ${budget}-line budget`, () => {
      const text = readFileSync(new URL(`../../../${guide}`, import.meta.url), 'utf8');
      expect(withinBudget(text, budget)).toBe(true);
    });
  }

  it('rejects a guide one line over its budget and accepts one at it', () => {
    for (const budget of Object.values(BUDGETS)) {
      expect(withinBudget(`${'line\n'.repeat(budget)}`, budget)).toBe(true);
      expect(withinBudget(`${'line\n'.repeat(budget + 1)}`, budget)).toBe(false);
    }
  });
});
