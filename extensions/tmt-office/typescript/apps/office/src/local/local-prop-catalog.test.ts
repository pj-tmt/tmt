import { readFileSync } from 'node:fs';
import path from 'node:path';
import { webcrypto } from 'node:crypto';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { startLocalRuntime } from './local-runtime.js';
import { propDocumentDigest } from '../props/prop-catalog-contract.js';
import { decodePropPack, WORKSHOP_DIGEST } from '../props/prop-contract.js';

const document = readFileSync(
  path.resolve(process.cwd(), '../../../contracts/prop-pack-v2-sample.tmtprop.json'),
  'utf8'
);
const token = 'a'.repeat(43);
beforeEach(() => {
  history.replaceState(null, '', `/local#token=${token}`);
  vi.stubGlobal('crypto', webcrypto);
});
afterEach(() => vi.unstubAllGlobals());

it('preserves exact install intent and verifies the native digest before accepting a retry receipt', async () => {
  const intent = { expectedRevision: 3, document };
  const digest = await propDocumentDigest(intent);
  const fetch = vi.fn(
    async (_path: string, _init?: RequestInit) =>
      new Response(JSON.stringify({ revision: 4, digest, changed: true }))
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  try {
    const pending = runtime.propCatalog.install(intent);
    intent.document = '{}';
    await expect(pending).resolves.toEqual({ revision: 4, digest, changed: true });
    const request = fetch.mock.calls[0]!;
    expect(request[0]).toBe('/api/v1/local/props/install');
    expect(request[1]).toMatchObject({
      method: 'POST',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ expectedRevision: 3, document }),
    });
    fetch.mockImplementation(
      async () => new Response(JSON.stringify({ revision: 4, digest, changed: false }))
    );
    await expect(
      runtime.propCatalog.install({ expectedRevision: 3, document })
    ).resolves.toMatchObject({ changed: false });
    expect(fetch.mock.calls[1]![1]!.body).toBe(request[1]!.body);
    for (const patch of [
      { digest: WORKSHOP_DIGEST },
      { revision: 8 },
      { revision: 3, changed: true },
    ]) {
      fetch.mockImplementation(
        async () => new Response(JSON.stringify({ revision: 4, digest, changed: false, ...patch }))
      );
      await expect(runtime.propCatalog.install({ expectedRevision: 3, document })).rejects.toThrow(
        'Unexpected prop install receipt'
      );
    }
  } finally {
    runtime.dispose();
  }
});

it('uses paged metadata discovery and shared on-demand resolution without installing on read', async () => {
  const digest = await propDocumentDigest({ expectedRevision: 0, document });
  const page = {
    revision: 2,
    entries: [{ digest, label: 'Private art' }],
    excluded: [],
    nextCursor: null,
  };
  const fetch = vi.fn(
    async (path: string) =>
      new Response(
        JSON.stringify(
          path.endsWith('/list')
            ? page
            : {
                catalogRevision: 2,
                packs: [{ digest, pack: JSON.parse(document) }],
                unavailable: [],
              }
        )
      )
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  try {
    await expect(runtime.propCatalog.list()).resolves.toEqual(page);
    await expect(runtime.propCatalog.load(digest)).resolves.toEqual({
      digest,
      pack: decodePropPack(JSON.parse(document)),
    });
    expect(fetch.mock.calls.map(([path]) => path)).toEqual([
      '/api/v1/local/props/list',
      '/api/v1/local/props/resolve',
    ]);
    await expect(runtime.propCatalog.load('../secret')).rejects.toThrow();
    await expect(
      runtime.propCatalog.install({ expectedRevision: 2, document: '{}' })
    ).rejects.toThrow();
    expect(fetch).toHaveBeenCalledTimes(2);
  } finally {
    runtime.dispose();
  }
});

it('keeps catalog conflicts observable without a hidden retry or overwrite', async () => {
  const fetch = vi.fn(
    async () => new Response('{"error":"OFFICE_CATALOG_REVISION_CONFLICT"}', { status: 409 })
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  try {
    await expect(
      runtime.propCatalog.install({ expectedRevision: 0, document })
    ).rejects.toMatchObject({ status: 409, code: 'OFFICE_CATALOG_REVISION_CONFLICT' });
    expect(fetch).toHaveBeenCalledTimes(1);
  } finally {
    runtime.dispose();
  }
});
