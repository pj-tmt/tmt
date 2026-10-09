import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';

// The shared guides hold only cross-cutting material. Per-area procedures belong in
// .agents/skills/<area>/ (DEVELOPMENT.md's area table) and per-module architecture in
// .agents/skills/tmt-core-runtime and the other area skills (ARCHITECTURE.md's owner
// maps). Raise a budget only with an issue that explains what moved into the guide.
const BUDGETS = {
  'DEVELOPMENT.md': 558,
  'ARCHITECTURE.md': 1248,
} as const;
const BYTE_BUDGETS = {
  'DEVELOPMENT.md': 21_086,
  'ARCHITECTURE.md': 96_960,
} as const;

export function lineCount(text: string): number {
  return text.endsWith('\n') ? text.split('\n').length - 1 : text.split('\n').length;
}

export function withinBudget(text: string, budget: number): boolean {
  return lineCount(text) <= budget;
}

export function withinByteBudget(text: string, budget: number): boolean {
  return Buffer.byteLength(text, 'utf8') <= budget;
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
      const lines = lineCount(text);
      expect(
        lines,
        `${guide}: ${lines} lines vs cap ${budget} lines; move area detail to its owning skill reference (DEVELOPMENT.md area table; ARCHITECTURE.md#maintenance-contract)`
      ).toBeLessThanOrEqual(budget);
    });
  }

  for (const [guide, budget] of Object.entries(BYTE_BUDGETS)) {
    it(`keeps ${guide} within its ${budget}-byte budget`, () => {
      const text = readFileSync(new URL(`../../../${guide}`, import.meta.url), 'utf8');
      const bytes = Buffer.byteLength(text, 'utf8');
      expect(
        bytes,
        `${guide}: ${bytes} B vs cap ${budget} B; move area detail to its owning skill reference (DEVELOPMENT.md area table; ARCHITECTURE.md#maintenance-contract)`
      ).toBeLessThanOrEqual(budget);
    });
  }

  it('rejects a guide one line over its budget and accepts one at it', () => {
    for (const budget of Object.values(BUDGETS)) {
      expect(withinBudget(`${'line\n'.repeat(budget)}`, budget)).toBe(true);
      expect(withinBudget(`${'line\n'.repeat(budget + 1)}`, budget)).toBe(false);
    }
  });

  it('counts UTF-8 bytes including the final newline', () => {
    expect(withinByteBudget('é😀\n', 7)).toBe(true);
    expect(withinByteBudget('é😀\n', 6)).toBe(false);
    expect(withinByteBudget('', 0)).toBe(true);
  });

  it('rejects a guide one byte over its budget and accepts one at it', () => {
    for (const budget of Object.values(BYTE_BUDGETS)) {
      const text = 'é'.repeat(Math.floor(budget / 2)) + 'a'.repeat(budget % 2);
      expect(withinByteBudget(text, budget)).toBe(true);
      expect(withinByteBudget(`${text}a`, budget)).toBe(false);
    }
  });
});
