import { afterEach, expect, it, vi } from 'vite-plus/test';
import { deriveSpaceId, encodeBinary } from '@tmt/colab-client';
import { AUTHOR_BYTES, DISCOVERY_BYTES, discover } from '../src/bootstrap.js';
vi.mock('../src/storage.js', () => ({ record: async () => undefined }));
afterEach(() => vi.unstubAllGlobals());
const mount = new URL('https://example.test/r/abcd/x/colab/');
const metadata = {
  pageId: '10000000-0000-4000-8000-000000000001',
  epoch: '18446744073709551615',
  sharing: 'private',
  history: 'current',
  archived: false,
  retentionDays: Number.MAX_SAFE_INTEGER,
  lastUpdateAtMs: Number.MAX_SAFE_INTEGER,
  expiresAtMs: null,
  warnings: ['not-in-cloud'],
  labels: [] as string[],
};
async function response(pages: unknown[]) {
  const owner = new Uint8Array(32).fill(9),
    space = await deriveSpaceId(owner);
  vi.stubGlobal('location', { hash: `#space=${space}` });
  const body = JSON.stringify({
    spaceId: space,
    ownerKey: encodeBinary(owner),
    revision: '1',
    pages,
    pageIds: pages.map((page) => ({ pageId: (page as { pageId: string }).pageId, deleted: false })),
  });
  vi.stubGlobal('fetch', async () => new Response(body));
  return body;
}
it('accepts 1000 worst-sized valid expiry rows within the existing bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages).toHaveLength(1000);
  expect(boot.pages[999].lastUpdateAtMs).toBe(Number.MAX_SAFE_INTEGER);
  expect(boot.pages[0].warnings).toEqual(['not-in-cloud']);
});
it('keeps the worst-sized labels of a full space inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // The space limit is 1,000 label assignments: 125 pages of 8 names, each the longest allowed.
  const rockets = '\u{1F680}'.repeat(31);
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    // 32 code points each (a digit and 31 four-byte characters), unique within the page.
    labels: index < 125 ? Array.from({ length: 8 }, (_, n) => `${n}${rockets}`) : [],
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages[0].labels).toHaveLength(8);
  expect(boot.pages[999].labels).toEqual([]);
});
it('keeps 1000 worst-sized summaries inside the bounded response', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  // Quotes escape to two bytes each in JSON, so 128 of them is the longest allowed author on the wire.
  const author = '"'.repeat(AUTHOR_BYTES);
  const rockets = '\u{1F680}'.repeat(31);
  const pages = Array.from({ length: 1000 }, (_, index) => ({
    ...metadata,
    pageId: `10000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    labels: index < 125 ? Array.from({ length: 8 }, (_, n) => `${n}${rockets}`) : [],
    summary: { comments: 4_294_967_295, waiting: true, needsYou: 350, author },
  }));
  const body = await response(pages);
  expect(new TextEncoder().encode(body).length).toBeLessThan(DISCOVERY_BYTES);
  const boot = await discover(mount);
  expect(boot.pages[999].summary).toEqual({
    comments: 4_294_967_295,
    waiting: true,
    needsYou: 350,
    author,
  });
});
it('accepts a summary with or without a creator and leaves absent summaries absent', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const id = (n: number) => `10000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
  await response([
    { ...metadata, pageId: id(1), summary: { comments: 0, waiting: false, needsYou: 0 } },
    {
      ...metadata,
      pageId: id(2),
      summary: { comments: 3, waiting: true, needsYou: 1, author: 'Ada \u{1F680}' },
    },
    { ...metadata, pageId: id(3) },
  ]);
  const { pages } = await discover(mount);
  expect(pages[0].summary).toEqual({ comments: 0, waiting: false, needsYou: 0 });
  expect(pages[1].summary).toEqual({
    comments: 3,
    waiting: true,
    needsYou: 1,
    author: 'Ada \u{1F680}',
  });
  expect('summary' in pages[2]).toBe(false);
});
it('refuses summaries outside the bounds the owner derives', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const summary of [
    null,
    [],
    'x',
    {},
    { comments: 1 },
    { waiting: true, needsYou: 1 },
    { comments: -1, waiting: false, needsYou: 0 },
    { comments: 0, waiting: true, needsYou: 0 },
    { comments: 0, waiting: false, needsYou: 1 },
    { comments: 0, waiting: true, needsYou: 351 },
    { comments: 0, waiting: true, needsYou: 1.5 },
    { comments: 0, waiting: false, needsYou: -1 },
    { comments: 0, waiting: true, needsYou: '1' },
    { comments: 0, waiting: false },
    { comments: 1.5, waiting: false, needsYou: 0 },
    { comments: 4_294_967_296, waiting: false, needsYou: 0 },
    { comments: '1', waiting: false, needsYou: 0 },
    { comments: 1, waiting: 'yes' },
    { comments: 1, waiting: false, needsYou: 0, extra: 1 },
    { comments: 1, waiting: false, needsYou: 0, author: '' },
    { comments: 1, waiting: false, needsYou: 0, author: 7 },
    { comments: 1, waiting: false, needsYou: 0, author: 'a'.repeat(AUTHOR_BYTES + 1) },
    { comments: 1, waiting: false, needsYou: 0, author: '\u{1F680}'.repeat(33) },
    { comments: 1, waiting: false, needsYou: 0, author: 'tab\there' },
  ]) {
    await response([{ ...metadata, summary }]);
    await expect(discover(mount)).rejects.toThrow();
  }
  // The longest allowed author is accepted at the byte boundary.
  await response([
    {
      ...metadata,
      summary: { comments: 1, waiting: false, needsYou: 0, author: 'a'.repeat(AUTHOR_BYTES) },
    },
  ]);
  expect((await discover(mount)).pages[0].summary?.author).toHaveLength(AUTHOR_BYTES);
});
it('accepts sorted display-only labels and refuses what the owner store never writes', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([{ ...metadata, labels: ['a b', 'q4 plan', '日本語'] }]);
  expect((await discover(mount)).pages[0].labels).toEqual(['a b', 'q4 plan', '日本語']);
  for (const labels of [
    'urgent',
    [1],
    [''],
    ['a', 'a'],
    ['x'.repeat(33)],
    ['tab\there'],
    Array.from({ length: 9 }, (_, n) => `l${n}`),
    null,
  ]) {
    await response([{ ...metadata, labels }]);
    await expect(discover(mount)).rejects.toThrow();
  }
  // A page without the key is not the shape the owner serves.
  const { labels: _omitted, ...bare } = metadata;
  await response([bare]);
  await expect(discover(mount)).rejects.toThrow();
});
it('rejects unsafe or ill-typed expiry hints before accepting discovery', async () => {
  for (const patch of [
    { lastUpdateAtMs: -1 },
    { lastUpdateAtMs: 1.5 },
    { expiresAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { retentionDays: 0 },
    { retentionDays: '30' },
    { warnings: ['unknown'] },
    { warnings: ['not-in-cloud', 'expires-soon'] },
    { warnings: ['expired'] },
    { warnings: ['expiry-unavailable'] },
    { warnings: [1] },
  ]) {
    await response([{ ...metadata, ...patch }]);
    await expect(discover(mount)).rejects.toThrow();
  }
});
it('rejects oversized discovery bodies even without Content-Length', async () => {
  await response([]);
  vi.stubGlobal('fetch', async () => new Response(' '.repeat(DISCOVERY_BYTES + 1)));
  await expect(discover(mount)).rejects.toThrow('exceeds limit');
});

it('retains an anonymous short-link target across authenticated discovery and mount pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  await response([]);
  vi.stubGlobal('location', { hash: '#path=%2Fshort%2F12345678' });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  const boot = await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(
    null,
    '',
    `${mount.pathname}#space=${boot.space}&path=%2Fshort%2F12345678`,
  );
});
it('rejects secret or malformed fragments instead of retaining them in a short-link recovery', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const hash of [
    '#seed=secret',
    '#path=%2Fshort%2F1234567',
    '#path=%2Fshort%2F12345678&path=%2Fpages%2Ffoo',
  ]) {
    await response([]);
    vi.stubGlobal('location', { hash });
    await expect(discover(mount)).rejects.toThrow();
  }
});

it('rejects inconsistent or title-bearing tombstone metadata before pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  for (const pageIds of [
    [{ pageId: metadata.pageId, deleted: true }],
    [{ pageId: metadata.pageId, deleted: false, title: 'deleted text' }],
    [{ pageId: metadata.pageId, deleted: 'true' }],
    [],
  ]) {
    const body = JSON.parse(await response([metadata]));
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ ...body, pageIds })));
    await expect(discover(mount)).rejects.toThrow();
  }
});
