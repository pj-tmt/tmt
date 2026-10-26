import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { metadataRequest as m, canonical, digest, encodeBinary, text } from '../src/index.js';
const vectors = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/editor-metadata-v1.json', import.meta.url),
    'utf8',
  ),
);
it('shares closed metadata requests and terminal result grammar with the native decoder', () => {
  for (const [cases, parse] of [
    [vectors.requests, m.metadataRequest],
    [vectors.results, m.editResult],
  ] as const)
    for (const c of cases) {
      let valid = true;
      try {
        parse(c.record, c.root, c.key);
      } catch {
        valid = false;
      }
      expect(valid, c.name).toBe(c.valid);
    }
});
it('requires owner-member provenance and the exact writer, operation and immutable payload', async () => {
  const request = m.metadataRequest(vectors.requests[0].record);
  const receipt = structuredClone(vectors.results[0].record);
  receipt.requestSha256 = encodeBinary(await digest(text(canonical(request))));
  expect(await m.resultFor(receipt, false, receipt.writer, request)).toBeUndefined();
  expect(await m.resultFor(receipt, true, request.operationId, request)).toBeUndefined();
  expect(
    await m.resultFor(receipt, true, receipt.writer, {
      ...request,
      action: 'creator.set',
      body: { selector: 'another' },
    }),
  ).toBeUndefined();
  expect(await m.resultFor(receipt, true, receipt.writer, request)).toEqual(receipt.result);
  expect(canonical({ z: 1, a: { b: null, a: 2 } })).toBe('{"a":{"a":2,"b":null},"z":1}');
});

it('hashes the complete request with the shared canonical digest', async () => {
  for (const c of vectors.requests.filter((c: { valid: boolean }) => c.valid))
    expect(encodeBinary(await digest(text(canonical(c.record))))).toBe(c.requestSha256);
});
