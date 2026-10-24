/**
 * Test-only stand-in for `digest-source.ts` in the shell harness: the product module with the
 * sample. `?seats=<n>` and `?one-provider` pick the sample's shape.
 */
import { mockDocument, type MockShape } from '../src/digest-mock.js';
import type { DigestSource } from '../src/digest-source.js';
export { USAGE_RANGES } from '../src/digest-source.js';

export function sampleShape(search = location.search): MockShape {
  const params = new URLSearchParams(search);
  const seats = Number(params.get('seats'));
  return { ...(seats > 0 ? { seats } : {}), oneProvider: params.has('one-provider') };
}

export const digestSource: DigestSource = {
  async report(range) {
    const document = mockDocument(range, sampleShape());
    if (new URLSearchParams(location.search).has('stale-allowance'))
      for (const limit of document.limits) {
        limit.stale = true;
        limit.elapsedPercent = null;
        limit.paceDeltaPoints = null;
      }
    if (new URLSearchParams(location.search).has('missing-evidence')) {
      document.usage.seats = document.usage.seats.map((seat) => ({
        ...seat,
        coverage: { ...seat.coverage, complete: true, gap: false },
        series: seat.series.map((point) => ({ ...point, complete: true })),
      }));
      document.usage.unavailable = [
        { id: 'missing-seat', name: 'missing-seat', retired: false, reason: 'not_sampled' },
      ];
      document.limits = [];
      document.limitsUnavailable = [{ line: 'claude', reason: 'not_sampled' }];
    }
    return { range, sample: true, document };
  },
};
