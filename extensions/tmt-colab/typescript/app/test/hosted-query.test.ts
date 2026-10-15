import { expect, it } from 'vite-plus/test';
import { text } from '@tmt/colab-client';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncQueryPage,
  SyncQueryRequest,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { hostedQuery } from '../src/hosted-query.js';

const query = { resource: 'log', scope: {}, stream: 'writer' } as const;
const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30_000,
});
const row = (seq: number): SealedDocument => ({
  target: { resource: 'log', id: `row-${seq}` },
  bytes: text(JSON.stringify({ seq: { integerValue: String(seq) } })),
  digest: '0'.repeat(64),
});
const page = (seqs: number[], cursor = seqs.at(-1)): SyncQueryPage => ({
  shape: 'stream',
  page: {
    documents: seqs.map(row),
    ...(cursor === undefined ? {} : { cursor: { stream: 'writer', seq: cursor } }),
  },
});
function fixture(pages: unknown[], cap = 3) {
  const calls: SyncQueryRequest[] = [];
  const port = {
    context: { resources: [{ name: 'log', sync: { query: { seqField: 'seq' } } }] },
    async query(request: SyncQueryRequest) {
      calls.push(structuredClone(request));
      const next = pages.shift();
      if (!(typeof next === 'object' && next !== null && 'shape' in next)) throw next;
      return next;
    },
  } as unknown as SealedSyncPort;
  return {
    calls,
    run: (selected = query, budget = call()) =>
      hostedQuery(port, selected, { documents: cap, bytes: 1000 }, budget),
  };
}

it('reads a finite contiguous tail then confirms empty, keeping query progress private', async () => {
  const f = fixture([page([1]), page([2, 3]), page([], 3)]);
  expect(await f.run()).toEqual([row(1), row(2), row(3)]);
  expect(f.calls.map((request) => request.query)).toEqual([
    { ...query, cursor: { stream: 'writer', seq: 0 } },
    { ...query, cursor: { stream: 'writer', seq: 1 } },
    { ...query, cursor: { stream: 'writer', seq: 3 } },
  ]);
});
it('accepts an empty page with an absent cursor without inventing progress', async () => {
  const f = fixture([page([])]);
  expect(await f.run()).toEqual([]);
  expect(f.calls).toHaveLength(1);
});
it.each([
  'REMOTE_SCOPE_DENIED',
  'REMOTE_BUDGET_EXHAUSTED',
  'REMOTE_RATE_LIMITED',
  'REMOTE_STATE_UNAVAILABLE',
])('returns no partial tail and never retries %s', async (code) => {
  const refusal = { code, resetAtMs: 99, retryAfterMs: 1 };
  const f = fixture([page([1]), refusal]);
  await expect(f.run()).rejects.toEqual(refusal);
  expect(f.calls).toHaveLength(2);
});
it.each([{ seqs: [2] }, { seqs: [1, 3] }])(
  'requires catch-up for a noncontiguous tail %j',
  async ({ seqs }) => {
    const f = fixture([page(seqs)]);
    await expect(f.run()).rejects.toEqual({
      code: 'REMOTE_STATE_UNAVAILABLE',
      state: 'catch-up-required',
    });
    expect(f.calls).toHaveLength(1);
  },
);
it.each([
  { pages: [page([1], 2)] },
  { pages: [page([1]), page([], 2)] },
  { pages: [page([1, 2, 3, 4])] },
  { pages: [page([1, 1])] },
  { pages: [{ shape: 'discovery', page: { documents: [] } }] },
])('refuses a dishonest cursor, over-cap tail or wrong result shape', async ({ pages }) => {
  const f = fixture(pages);
  await expect(f.run()).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
});
it('checks cancellation and the absolute deadline before any query', async () => {
  for (const budget of [
    { signal: AbortSignal.abort(), deadlineAtMs: Date.now() + 1000 },
    { signal: new AbortController().signal, deadlineAtMs: Date.now() - 1 },
  ]) {
    const f = fixture([page([])]);
    await expect(f.run(query, budget)).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
    expect(f.calls).toHaveLength(0);
  }
});
