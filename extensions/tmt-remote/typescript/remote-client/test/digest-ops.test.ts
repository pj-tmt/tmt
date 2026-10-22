import { expect, it } from 'vite-plus/test';
import { mockDigestSource } from '../src/digest-mock.js';
import { digestSource, USAGE_RANGES } from '../src/digest-source.js';

it('reads no sample data in the product: the default Digest source names the terminal command', async () => {
  for (const range of USAGE_RANGES) {
    const report = await digestSource.report(range);
    expect(report).toMatchObject({ range, sample: false });
    expect(report.document).toBeUndefined();
    expect(report.unavailable).toBe(
      'Usage is not in the browser yet. Open Digest in `tmt ops ui`.',
    );
  }
});

it('marks the mock Digest source (tests, gallery, dev) as sample data for every range', async () => {
  expect(USAGE_RANGES).toEqual(['24h', '7d', '30d']);
  for (const range of USAGE_RANGES) {
    const report = await mockDigestSource.report(range);
    expect(report.sample).toBe(true);
    expect(report.range).toBe(range);
    expect(report.document?.since).toBe(range);
  }
});
