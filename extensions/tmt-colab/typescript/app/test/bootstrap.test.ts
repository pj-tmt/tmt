import { afterEach, expect, it, vi } from 'vite-plus/test';
import { deriveSpaceId, encodeBinary } from '@tmt/colab-client';
import { discover } from '../src/bootstrap.js';
import { record } from '../src/storage.js';
const storage = vi.hoisted(() => ({ pin: undefined as unknown }));
vi.mock('../src/storage.js', () => ({
  record: vi.fn(async (_key: string, ...values: unknown[]) =>
    values.length ? undefined : storage.pin,
  ),
}));
afterEach(() => {
  storage.pin = undefined;
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});
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
  expect(new TextEncoder().encode(body).length).toBeLessThan(512 * 1024);
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
  vi.stubGlobal('fetch', async () => new Response(' '.repeat(512 * 1024 + 1)));
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

it('public page and thread targets do not replace the internal mount or bypass owner verification', async () => {
  await response([metadata]);
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  vi.stubGlobal('location', { pathname: '/p/10000000', hash: `#t=${metadata.pageId}` });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  const verify = vi.fn(async () => {});
  const boot = await discover(mount, verify);
  expect(verify).toHaveBeenCalledWith(boot.space, boot.owner);
  expect(record).toHaveBeenCalledWith(`pin:${mount.href}`, {
    space: boot.space,
    owner: boot.owner,
  });
  expect(replaceState).not.toHaveBeenCalled();
});
it('public entry refuses unexpected fragments and a mismatched existing pin without re-pinning', async () => {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  for (const hash of [
    '#seed=secret',
    '#space=wrong',
    '#t=bad',
    `#t=${metadata.pageId}&t=${metadata.pageId}`,
  ]) {
    await response([metadata]);
    vi.stubGlobal('location', { pathname: '/p/10000000', hash });
    await expect(discover(mount)).rejects.toThrow();
  }
  storage.pin = { space: 'b'.repeat(32), owner: new Uint8Array(32).fill(9) };
  await response([metadata]);
  vi.stubGlobal('location', { pathname: '/p/10000000', hash: '' });
  await expect(discover(mount)).rejects.toThrow();
  expect(vi.mocked(record).mock.calls.every((call) => Array.from(call).length === 1)).toBe(true);
  expect(replaceState).not.toHaveBeenCalled();
});
it('an authenticated legacy page entry canonicalizes to a catalog-derived short target', async () => {
  await response([metadata]);
  const space = await deriveSpaceId(new Uint8Array(32).fill(9));
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, run: () => unknown) => run() },
  });
  vi.stubGlobal('location', {
    pathname: mount.pathname,
    hash: `#space=${space}&path=%2Fpages%2F${metadata.pageId}`,
  });
  const replaceState = vi.fn();
  vi.stubGlobal('history', { state: null, replaceState });
  await discover(mount);
  expect(replaceState).toHaveBeenCalledWith(null, '', '/p/10000000');
});
