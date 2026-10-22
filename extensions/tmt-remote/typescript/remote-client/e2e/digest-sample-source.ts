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
    return { range, sample: true, document: mockDocument(range, sampleShape()) };
  },
};
