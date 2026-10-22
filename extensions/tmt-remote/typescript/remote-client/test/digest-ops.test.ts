import { expect, it, vi } from 'vite-plus/test';
import type { OwnerReadAnswer } from '../src/owner-reads.js';
import { mockDigestSource } from '../src/digest-mock.js';
import { digestSource, USAGE_RANGES } from '../src/digest-source.js';

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
  for (const range of USAGE_RANGES) {
    const report = await digestSource.report(range);
    expect(report).toMatchObject({ range, sample: false });
    expect(report.document).toBeUndefined();
    expect(report.unavailable).toBe(
      'This browser has no owner session for your machine. Pair it from Remote to see your usage.',
    );
  }
  // Usage and the limit history, each range once; nothing is read from anywhere else.
  expect(new Set(asked)).toEqual(new Set(['digest/24h', 'digest/7d', 'digest/30d', 'limits']));
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
