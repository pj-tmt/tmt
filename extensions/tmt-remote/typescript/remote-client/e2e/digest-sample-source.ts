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

const sampleSource: DigestSource = {
  async report(range) {
    const document = mockDocument(range, sampleShape());
    const allowance = mockDocument('7d', sampleShape());
    if (new URLSearchParams(location.search).has('stale-allowance'))
      for (const limit of allowance.limits) {
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
      allowance.limits = [];
      document.limitsUnavailable = [{ line: 'claude', reason: 'not_sampled' }];
    }
    const count = Number(new URLSearchParams(location.search).get('providers'));
    for (let at = 2; at < count; at++)
      allowance.limits = [
        ...allowance.limits,
        { ...allowance.limits[0]!, provider: `sample-provider-${at}` },
      ];
    return { range, sample: true, document, allowance };
  },
};

export const createDigestSource = (): DigestSource => sampleSource;
