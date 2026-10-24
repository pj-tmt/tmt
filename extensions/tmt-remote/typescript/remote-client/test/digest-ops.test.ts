import { expect, it, vi } from 'vite-plus/test';
import type { OwnerReadAnswer } from '../src/owner-reads.js';
import { mockDigestSource } from '../src/digest-mock.js';
import { createDigestSource, USAGE_RANGES } from '../src/digest-source.js';

let readsAnswer: (name: string) => OwnerReadAnswer<unknown> = () => ({ state: 'no-session' });
vi.mock('../src/owner-reads.js', () => ({
  ownerRead: async (name: string) => readsAnswer(name),
}));

it('reads no sample data in the product: the default Digest source asks the owner reads and says why it cannot', async () => {
  const asked: string[] = [];
  readsAnswer = (name) => {
    asked.push(name);
    return { state: 'no-session' };
  };
  const source = createDigestSource();
  for (const range of USAGE_RANGES) {
    const report = await source.report(range);
    expect(report).toMatchObject({ range, sample: false });
    expect(report.document).toBeUndefined();
    expect(report.unavailable).toBe(
      'This browser has no owner session for your machine. Pair it from Remote to see your usage.',
    );
  }
  // One fixed weekly snapshot per mount; range switches never repeat its read.
  expect(asked).toEqual(['digest/24h', 'digest/7d', 'limits', 'digest/30d']);
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
