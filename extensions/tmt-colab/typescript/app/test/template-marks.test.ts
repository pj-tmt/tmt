import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';
import { validateTemplateMark, validateTemplateSource } from '../src/template-marks.js';

const corpus = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/template-marks-v1.json', import.meta.url),
    'utf8',
  ),
) as {
  cases: { name: string; kind: 'template' | 'fromTemplate'; value: unknown; valid: boolean }[];
};

for (const entry of corpus.cases)
  test(`template marks: ${entry.kind} ${entry.name}`, () => {
    const validate = () =>
      entry.kind === 'template'
        ? validateTemplateMark(entry.value)
        : validateTemplateSource(entry.value);
    if (entry.valid) expect(validate).not.toThrow();
    else expect(validate).toThrow();
  });
