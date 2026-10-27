import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { storagePort } from '../src/storage-usage.js';

const page = '11111111-1111-4111-8111-111111111111';
const row = {
  pageId: page,
  title: 'Page',
  contentBytes: 10,
  historyBytes: 20,
  commentsBytes: 5,
  attachmentsBytes: null,
  unusedAttachmentBytes: null,
  totalBytes: null,
  freeable: { history: 4, files: 0 },
  lastCleanedAutomaticallyAtMs: null,
  skipped: null,
};
const result = {
  version: 1,
  page,
  dryRun: false,
  freeable: { history: 4, files: 0 },
  freed: { history: 3, files: 0 },
  kept: ['comments', 'threads', 'archive'],
  skipped: null,
};
afterEach(() => vi.unstubAllGlobals());
describe('owner storage endpoint', () => {
  it('sends the page and validates the returned units', async () => {
    const fetch = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(row), { status: 200 }),
    );
    vi.stubGlobal('fetch', fetch);
    const port = storagePort(new URL('https://example.test/module/'));
    expect(await port.read(page)).toEqual(row);
    expect(fetch.mock.calls[0][0].toString()).toBe('https://example.test/module/api/storage');
    expect(JSON.parse(fetch.mock.calls[0][1]!.body as string)).toEqual({
      version: 1,
      page,
      dryRun: false,
    });
  });
  it('returns measured and unknown errors from HTTP 503', async () => {
    const failure = {
      ...result,
      freed: { history: null, files: 0 },
      error: { code: 'COLAB_UNAVAILABLE', message: 'cleanup failed' },
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify(failure), { status: 503 })),
    );
    expect(await storagePort(new URL('https://example.test/')).reclaim(page)).toEqual(failure);
  });
  it('preserves admission refusals and rejects an unknown successful total', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ code: 'DENIED' }), { status: 403 })),
    );
    await expect(storagePort(new URL('https://example.test/')).reclaim(page)).rejects.toMatchObject(
      { status: 403 },
    );
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify({ ...result, freed: { history: null, files: 0 } }), {
            status: 200,
          }),
      ),
    );
    await expect(storagePort(new URL('https://example.test/')).reclaim(page)).rejects.toThrow();
  });
  it('refuses inconsistent or malformed storage responses', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify({ ...row, freeable: { history: 21, files: 0 } }), {
            status: 200,
          }),
      ),
    );
    await expect(storagePort(new URL('https://example.test/')).read(page)).rejects.toThrow();
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify({ ...result, freed: { history: -1, files: 0 } }), {
            status: 200,
          }),
      ),
    );
    await expect(storagePort(new URL('https://example.test/')).reclaim(page)).rejects.toThrow();
  });
});
