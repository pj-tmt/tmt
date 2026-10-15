import { expect, it } from 'vite-plus/test';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncCall,
  SyncQueryPage,
  SyncQueryRequest,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { HostedDiscovery } from '../src/hosted-discovery.js';

const query = {
  resource: 'log',
  shape: 'writers',
  scope: {
    space: { stringValue: 'space' },
    page: { stringValue: 'page' },
    epoch: { integerValue: '7' },
  },
} as const;
const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 30_000,
});
const document = (id: string, length = 1): SealedDocument => ({
  target: { resource: 'log', id },
  bytes: new Uint8Array(length),
  digest: 'a'.repeat(64),
});
function fixture(pages: (SyncQueryPage | unknown)[], cap = 3, bytes = 10) {
  const calls: SyncQueryRequest[] = [];
  const port = {
    async query(request: SyncQueryRequest) {
      calls.push(structuredClone(request));
      const next = pages.shift();
      if (!(typeof next === 'object' && next !== null && 'shape' in next)) throw next;
      return next;
    },
  } as unknown as SealedSyncPort;
  return {
    calls,
    discovery: new HostedDiscovery(port, new Map([['log', { documents: cap, bytes }]])),
  };
}
const page = (documents: SealedDocument[], continuation?: string): SyncQueryPage => ({
  shape: 'discovery',
  page: { documents, ...(continuation ? { continuation } : {}) },
});

it('completes one declared discovery run across bounded stateless continuations', async () => {
  const one = document('one'),
    two = document('two');
  const f = fixture([page([one], 'next'), page([two])]);
  expect(await f.discovery.run(query, call())).toEqual([one, two]);
  expect(f.calls).toEqual([
    { shape: 'discovery', query },
    { shape: 'discovery', query: { ...query, continuation: 'next' } },
  ]);
});

it('accepts a complete empty run as empty without inventing a stream', async () => {
  const f = fixture([page([])]);
  expect(await f.discovery.run(query, call())).toEqual([]);
  expect(f.calls).toEqual([{ shape: 'discovery', query }]);
});

it.each([
  'REMOTE_SCOPE_DENIED',
  'REMOTE_BUDGET_EXHAUSTED',
  'REMOTE_RATE_LIMITED',
  'REMOTE_STATE_UNAVAILABLE',
])('exposes no partial discovery or retry when a later page refuses %s', async (code) => {
  const refusal = { code, resetAtMs: 90, retryAfterMs: 1 };
  const f = fixture([page([document('one')], 'next'), refusal]);
  await expect(f.discovery.run(query, call())).rejects.toEqual(refusal);
  expect(f.calls).toHaveLength(2);
});

it('turns an invalid continued token into catch-up-required for the one outer recovery owner', async () => {
  const f = fixture([page([document('one')], 'expired'), { code: 'REMOTE_INPUT_INVALID' }]);
  await expect(f.discovery.run(query, call())).rejects.toEqual({
    code: 'REMOTE_STATE_UNAVAILABLE',
    state: 'catch-up-required',
  });
  expect(f.calls).toHaveLength(2);
});

it('does not reinterpret a first-page input refusal as a continuation expiry', async () => {
  const f = fixture([{ code: 'REMOTE_INPUT_INVALID' }]);
  await expect(f.discovery.run(query, call())).rejects.toEqual({ code: 'REMOTE_INPUT_INVALID' });
  expect(f.calls).toHaveLength(1);
});

it.each([
  { pages: [page([document('one')], 'same'), page([document('two')], 'same')] },
  { pages: [page([document('one')], 'next'), page([document('one')])] },
  { pages: [page([], 'next')] },
  { pages: [page([document('one', 11)])] },
  { pages: [page([document('one'), document('two'), document('three'), document('four')])] },
])(
  'rejects repeated tokens/documents, non-progress, and declared count/byte overflow',
  async ({ pages }) => {
    const f = fixture(pages);
    await expect(f.discovery.run(query, call())).rejects.toMatchObject({
      code: 'REMOTE_STATE_UNAVAILABLE',
    });
    expect(f.calls.length).toBeLessThanOrEqual(2);
  },
);

it('fences a cancelled call before the first query', async () => {
  const f = fixture([page([])]);
  await expect(
    f.discovery.run(query, { signal: AbortSignal.abort(), deadlineAtMs: Date.now() + 30_000 }),
  ).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
  expect(f.calls).toHaveLength(0);
});
