import { afterEach, expect, it, vi } from 'vite-plus/test';
import { deriveSpaceId, encodeBinary } from '@tmt/colab-client';
import { discover } from '../src/bootstrap.js';
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
  warnings: ['expiry-out-of-range'],
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
  expect(new TextEncoder().encode(body).length).toBeLessThan(256 * 1024);
  const boot = await discover(mount);
  expect(boot.pages).toHaveLength(1000);
  expect(boot.pages[999].lastUpdateAtMs).toBe(Number.MAX_SAFE_INTEGER);
  expect(boot.pages[0].warnings).toEqual(['expiry-out-of-range']);
});
it('rejects unsafe or ill-typed expiry hints before accepting discovery', async () => {
  for (const patch of [
    { lastUpdateAtMs: -1 },
    { lastUpdateAtMs: 1.5 },
    { expiresAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { retentionDays: 0 },
    { retentionDays: '30' },
    { warnings: ['unknown'] },
    { warnings: ['expired', 'expires-soon'] },
    { warnings: [1] },
  ]) {
    await response([{ ...metadata, ...patch }]);
    await expect(discover(mount)).rejects.toThrow();
  }
});
it('rejects oversized discovery bodies even without Content-Length', async () => {
  await response([]);
  vi.stubGlobal('fetch', async () => new Response(' '.repeat(256 * 1024 + 1)));
  await expect(discover(mount)).rejects.toThrow('exceeds limit');
});
