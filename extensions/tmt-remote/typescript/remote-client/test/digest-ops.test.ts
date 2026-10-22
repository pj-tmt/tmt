import { expect, it } from 'vite-plus/test';
import { formatTokens, usageRows } from '../src/digest-page.js';
import { digestSource, mockDigestSource, USAGE_RANGES } from '../src/digest-source.js';

it('formats token counts compactly and orders agents by share', async () => {
  expect([
    formatTokens(1_918_900_000),
    formatTokens(48_200_000),
    formatTokens(310_400),
    formatTokens(940),
  ]).toEqual(['1.9B', '48.2M', '310K', '940']);
  const rows = usageRows({
    range: '7d',
    sample: false,
    agents: [
      { agent: 'b', tokens: 1 },
      { agent: 'a', tokens: 3, turns: 2 },
    ],
  });
  expect(rows.map((row) => [row.agent, row.share])).toEqual([
    ['a', 75],
    ['b', 25],
  ]);
  expect(usageRows({ range: '5m', sample: false, agents: [] })).toEqual([]);
});

it('reads no sample data in the product: the default Digest source names the terminal command', async () => {
  for (const range of USAGE_RANGES) {
    const report = await digestSource.usage(range);
    expect(report).toMatchObject({ range, agents: [], sample: false });
    expect(report.unavailable).toBe(
      'Usage is not in the browser yet. Open Digest in `tmt ops ui`.',
    );
  }
});

it('marks the mock Digest source (tests, gallery, dev) as sample data for every range', async () => {
  expect(USAGE_RANGES).toEqual(['5m', '1h', '6h', '1d', '7d', '30d']);
  for (const range of USAGE_RANGES) {
    const report = await mockDigestSource.usage(range);
    expect(report.sample).toBe(true);
    expect(report.range).toBe(range);
  }
});
