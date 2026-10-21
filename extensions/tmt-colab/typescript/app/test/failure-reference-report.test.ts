import { describe, expect, it } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import { text } from '../src/strings.js';

// The acceptance report keeps its own copy of the ended causes, as a boundary check independent of
// the product. The harness writes a cause by its `text.failureReference` key, so a key the product
// adds is dropped by the report until the report lists it: this fails when it does not.
const report = readFileSync(
  new URL('../../../../../typescript/scripts/colab-acceptance-report.mjs', import.meta.url),
  'utf8',
);
const reportCauses = [
  ...(/const causes = \[([^\]]*)\]/.exec(report)?.[1] ?? '').matchAll(/'([^']+)'/g),
].map((match) => match[1]);

describe("the acceptance report's ended causes", () => {
  it("reads the report's list", () => {
    expect(reportCauses).toEqual(expect.arrayContaining(['none', 'other']));
    expect(reportCauses.length).toBeGreaterThanOrEqual(12);
  });

  it.each(Object.keys(text.failureReference))('lists the failure cause %s', (cause) => {
    expect(reportCauses).toContain(cause);
  });
});
