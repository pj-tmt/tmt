import { afterEach, expect, it, vi } from 'vite-plus/test';
import {
  ReadingDetailsQueue,
  readingDetails,
  readReadingDetails,
  READING_DETAILS_BYTES,
} from '../src/reading-details.js';

const pageId = '10000000-0000-4000-8000-000000000001';
const readingId = '20000000-0000-4000-8000-000000000001';
const summary = { comments: 1, waiting: true, needsYou: 2 };
const descriptor = {
  id: readingId,
  title: 'Read this',
  agent: 'Agent',
  why: 'Before deciding',
  state: 'unread' as const,
};
const ready = () => ({
  pageId,
  state: 'ready' as const,
  fingerprint: 'a'.repeat(64),
  summary,
  readings: [descriptor],
  needs: [],
});
const need = {
  kind: 'reading' as const,
  id: readingId,
  title: 'Read this',
  agent: 'Agent',
  why: 'Before deciding',
};

it('admits one complete fingerprinted reading result and explicit unknown states', () => {
  expect(readingDetails(ready(), pageId)).toEqual(ready());
  for (const state of ['pending', 'unavailable'])
    expect(readingDetails({ pageId, state }, pageId)).toEqual({ pageId, state });
  const read = readingDetails({ ...ready(), readings: [{ ...descriptor, state: 'read' }] }, pageId);
  expect(read.state === 'ready' && read.readings[0].state).toBe('read');
});

it('admits the named passages and the latest excerpt, each optional field only where it belongs', () => {
  const review = { kind: 'review' as const, id: pageId, title: 'Plan', agent: 'Agent 2' };
  const value = {
    ...ready(),
    needs: [
      review,
      need,
      { kind: 'proposal' as const, id: '30000000-0000-4000-8000-000000000001', title: 'Pick one' },
    ],
    latest: { who: 'Agent', atMs: 1_700_000_000_000, text: 'A reply' },
  };
  expect(readingDetails(value, pageId)).toEqual(value);
  // The label is optional; the excerpt and its time are not.
  const anonymous = { ...ready(), latest: { atMs: 5, text: 'Unsigned' } };
  expect(readingDetails(anonymous, pageId)).toEqual(anonymous);
});

it.each([
  ['another page', () => ({ ...ready(), pageId: readingId })],
  ['no passages key', () => ({ ...ready(), needs: undefined })],
  ['a passage of another kind', () => ({ ...ready(), needs: [{ ...need, kind: 'answer' }] })],
  ['a passage without a title', () => ({ ...ready(), needs: [{ ...need, title: '' }] })],
  [
    'an overlong passage title',
    () => ({ ...ready(), needs: [{ ...need, title: 'x'.repeat(201) }] }),
  ],
  ['control text in a passage', () => ({ ...ready(), needs: [{ ...need, agent: 'a\u0007b' }] })],
  ['a reason on a review', () => ({ ...ready(), needs: [{ ...need, kind: 'review' }] })],
  ['an unknown passage key', () => ({ ...ready(), needs: [{ ...need, extra: 1 }] })],
  ['duplicate passage IDs', () => ({ ...ready(), needs: [need, need] })],
  [
    'more than twenty passages',
    () => ({
      ...ready(),
      needs: Array.from({ length: 21 }, (_, i) => ({
        ...need,
        id: `20000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
      })),
    }),
  ],
  ['a latest excerpt without text', () => ({ ...ready(), latest: { atMs: 1, text: '' } })],
  [
    'an overlong latest excerpt',
    () => ({ ...ready(), latest: { atMs: 1, text: 'x'.repeat(161) } }),
  ],
  ['a latest time that is not a count', () => ({ ...ready(), latest: { atMs: -1, text: 'x' } })],
  ['an unknown latest key', () => ({ ...ready(), latest: { atMs: 1, text: 'x', by: 'y' } })],

  ['missing fingerprint', () => ({ ...ready(), fingerprint: undefined })],
  ['uppercase fingerprint', () => ({ ...ready(), fingerprint: 'A'.repeat(64) })],
  ['duplicate IDs', () => ({ ...ready(), readings: [descriptor, descriptor] })],
  ['unknown state', () => ({ ...ready(), readings: [{ ...descriptor, state: 'maybe' }] })],
  ['a partial success', () => ({ ...ready(), more: true })],
  [
    'pending with a false zero',
    () => ({ pageId, state: 'pending', summary: { comments: 0, waiting: false, needsYou: 0 } }),
  ],
  ['invalid summary', () => ({ ...ready(), summary: { ...summary, waiting: false } })],
  ['overlong title', () => ({ ...ready(), readings: [{ ...descriptor, title: 'x'.repeat(201) }] })],
  ['overlong agent', () => ({ ...ready(), readings: [{ ...descriptor, agent: 'x'.repeat(65) }] })],
  ['overlong reason', () => ({ ...ready(), readings: [{ ...descriptor, why: 'x'.repeat(201) }] })],
  ['control text', () => ({ ...ready(), readings: [{ ...descriptor, why: 'hidden\ntext' }] })],
  [
    'too many descriptors',
    () => ({
      ...ready(),
      readings: Array.from({ length: 51 }, (_, i) => ({
        ...descriptor,
        id: `20000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
      })),
    }),
  ],
])('refuses %s instead of presenting a complete empty list', (_name, value) => {
  expect(() => readingDetails(value(), pageId)).toThrow();
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

it('reads each distinct page once, in order, with one read in flight', async () => {
  const gate = deferred<ReturnType<typeof ready>>();
  const read = vi
    .fn()
    .mockReturnValueOnce(gate.promise)
    .mockResolvedValue({ pageId: readingId, state: 'pending' });
  const publish = vi.fn();
  const queue = new ReadingDetailsQueue(read, publish);
  queue.enqueue([pageId, pageId, readingId]);
  expect(read).toHaveBeenCalledTimes(1);
  gate.resolve(ready());
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(read.mock.calls.map(([id]) => id)).toEqual([pageId, readingId]);
  await vi.waitFor(() => expect(publish).toHaveBeenCalledTimes(2));
  queue.enqueue([pageId, readingId]);
  await queue.close();
  expect(read).toHaveBeenCalledTimes(2);
});

it('caps an opening at twenty pages including failed and unavailable reads', async () => {
  const read = vi.fn(async (id: string) => {
    if (Number(id) % 2) throw new Error('Unavailable');
    return { pageId: id, state: 'unavailable' as const };
  });
  const queue = new ReadingDetailsQueue(read, vi.fn());
  const pages = Array.from({ length: 30 }, (_, i) => `${i}`);
  queue.enqueue(pages);
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(20));
  queue.enqueue(pages);
  await queue.close();
  expect(read.mock.calls.map(([id]) => id)).toEqual(pages.slice(0, 20));
});

it('publishes a failed read as unavailable, never as an empty ready result, and a canceled one as nothing', async () => {
  const gate = deferred<ReturnType<typeof ready>>();
  const read = vi
    .fn()
    .mockRejectedValueOnce(new Error('offline'))
    .mockReturnValueOnce(gate.promise);
  const publish = vi.fn();
  const queue = new ReadingDetailsQueue(read, publish);
  queue.enqueue([pageId, readingId]);
  await vi.waitFor(() =>
    expect(publish).toHaveBeenCalledWith(pageId, { pageId, state: 'unavailable' }),
  );
  await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  const closing = queue.close();
  gate.resolve(ready());
  await closing;
  expect(publish).toHaveBeenCalledTimes(1);
});

it('aborts and joins on leaving, fencing a late result and every successor', async () => {
  const gate = deferred<ReturnType<typeof ready>>();
  const read = vi.fn((_id: string, _signal: AbortSignal) => gate.promise);
  const publish = vi.fn();
  const queue = new ReadingDetailsQueue(read, publish);
  queue.enqueue([pageId, readingId]);
  let closed = false;
  const closing = queue.close().then(() => {
    closed = true;
  });
  expect(read.mock.calls[0][1].aborted).toBe(true);
  await Promise.resolve();
  expect(closed).toBe(false);
  gate.resolve(ready());
  await closing;
  queue.enqueue([readingId]);
  expect(publish).not.toHaveBeenCalled();
  expect(read).toHaveBeenCalledTimes(1);
});

afterEach(() => vi.unstubAllGlobals());

it('uses only the owner details route and rejects a request canceled before fetch', async () => {
  const fetcher = vi.fn(async (_url: URL, _options: RequestInit) => Response.json(ready()));
  vi.stubGlobal('fetch', fetcher);
  const signal = new AbortController();
  await readReadingDetails(new URL('https://example.test/r/owner/x/colab/'), pageId, signal.signal);
  expect(fetcher).toHaveBeenCalledTimes(1);
  expect(String(fetcher.mock.calls[0][0])).toBe(
    `https://example.test/r/owner/x/colab/api/pages/${pageId}/summary-details`,
  );
  expect(fetcher.mock.calls[0][1]).toMatchObject({ credentials: 'same-origin', cache: 'no-store' });
  signal.abort();
  await expect(
    readReadingDetails(new URL('https://example.test/r/owner/x/colab/'), pageId, signal.signal),
  ).rejects.toThrow();
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it('never converts a refused or oversized details response into a ready zero', async () => {
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce(new Response('DENIED', { status: 403 }))
    .mockResolvedValueOnce(
      new Response(' '.repeat(READING_DETAILS_BYTES + 1), {
        headers: { 'content-type': 'application/json' },
      }),
    );
  vi.stubGlobal('fetch', fetcher);
  const mount = new URL('https://example.test/r/owner/x/colab/');
  await expect(readReadingDetails(mount, pageId, new AbortController().signal)).rejects.toThrow();
  await expect(readReadingDetails(mount, pageId, new AbortController().signal)).rejects.toThrow();
  expect(fetcher).toHaveBeenCalledTimes(2);
});

it('admits all fifty bounded non-ASCII readings and twenty passages without truncating the result', () => {
  const value = {
    ...ready(),
    needs: Array.from({ length: 20 }, (_, i) => ({
      kind: 'reading' as const,
      id: `20000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
      title: '🐈'.repeat(200),
      agent: '🦉'.repeat(64),
      why: '🌍'.repeat(200),
    })),
    latest: { who: '🦉'.repeat(64), atMs: 1_700_000_000_000, text: '🐈'.repeat(160) },
    readings: Array.from({ length: 50 }, (_, i) => ({
      id: `20000000-0000-4000-8000-${String(i).padStart(12, '0')}`,
      title: '🐈'.repeat(200),
      agent: '🦉'.repeat(64),
      why: '🌍'.repeat(200),
      state: 'unread',
    })),
  };
  expect(new TextEncoder().encode(JSON.stringify(value)).length).toBeLessThanOrEqual(
    READING_DETAILS_BYTES,
  );
  expect(readingDetails(value, pageId)).toEqual(value);
});
