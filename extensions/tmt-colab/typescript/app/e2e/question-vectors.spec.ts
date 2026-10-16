import { expect, test } from '@playwright/test';
import {
  expectedAdmitted,
  project,
  sortFindings,
  vectors,
  type VectorCase,
} from '../test/question-vectors.js';
import type { QuestionInspection } from '../src/questions.js';

// The browser's own HTML parser is the oracle for every `parser` case; the Rust checker must
// agree with these exact verdicts (extensions/tmt-colab/rust/tmt-colab/tests/page_markup_vectors.rs).
const cases = vectors.cases.filter((candidate) => candidate.scope === 'parser');
const module = '/src/questions.ts';
const results = new Map<string, QuestionInspection>();

test.beforeAll(async ({ browser }) => {
  const page = await browser.newPage();
  await page.goto('/');
  const inspected = await page.evaluate(
    async ({ module, sources }) => {
      const { inspectQuestions } = await import(module);
      return sources.map((source: string) => inspectQuestions(source));
    },
    { module, sources: cases.map((candidate) => candidate.source.join('\n')) },
  );
  cases.forEach((candidate, index) => results.set(candidate.name, inspected[index]!));
  await page.close();
});

function expected(candidate: VectorCase) {
  return { admitted: expectedAdmitted(candidate), findings: sortFindings(candidate.findings) };
}
for (const candidate of cases)
  test(`${candidate.name}: ${candidate.why}`, () => {
    expect(project(results.get(candidate.name)!)).toEqual(expected(candidate));
  });
