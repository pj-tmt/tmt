import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { startLocalRuntime } from './local-runtime.js';
import { NOTEBOOK_ENVELOPE_BYTES } from '../notebooks/notebook-contract.js';

const identityId = '11111111-1111-4111-8111-111111111111';
const notebook = { identityId, name: 'Alice', content: '\ufeff# Notes\r\n\u0000🤖' };
const token = 'a'.repeat(43);
beforeEach(() => history.replaceState(null, '', `/local#token=${token}`));
afterEach(() => vi.unstubAllGlobals());

it('reads exact notes through authenticated GET, with no write or hidden refresh', async () => {
  const fetch = vi.fn(
    async (_path: string, _init?: RequestInit) => new Response(JSON.stringify(notebook))
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  try {
    await expect(runtime.notebooks.read(identityId)).resolves.toEqual(notebook);
    expect(fetch).toHaveBeenCalledExactlyOnceWith(
      `/api/v1/local/notebooks/${identityId}`,
      expect.objectContaining({
        headers: { Authorization: `Bearer ${token}` },
      })
    );
    expect(fetch.mock.calls[0]?.[1]).not.toHaveProperty('body');
    expect(fetch.mock.calls[0]?.[1]).not.toHaveProperty('method');
  } finally {
    runtime.dispose();
  }
});

it('rejects invalid paths before transport, unrelated responses, oversized envelopes and missing notes', async () => {
  const fetch = vi.fn(async () => new Response(JSON.stringify(notebook)));
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  try {
    await expect(runtime.notebooks.read('../notes')).rejects.toThrow();
    expect(fetch).not.toHaveBeenCalled();
    fetch.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({ ...notebook, identityId: '22222222-2222-4222-8222-222222222222' })
        )
    );
    await expect(runtime.notebooks.read(identityId)).rejects.toThrow(
      'Unexpected notebook identity'
    );
    fetch.mockImplementation(
      async () =>
        new Response(' ', { headers: { 'content-length': String(NOTEBOOK_ENVELOPE_BYTES + 1) } })
    );
    await expect(runtime.notebooks.read(identityId)).rejects.toThrow();
    fetch.mockImplementation(
      async () => new Response('{"error":"NOTEBOOK_NOT_FOUND"}', { status: 404 })
    );
    await expect(runtime.notebooks.read(identityId)).rejects.toMatchObject({
      status: 404,
      code: 'NOTEBOOK_NOT_FOUND',
    });
    expect(fetch).toHaveBeenCalledTimes(3);
  } finally {
    runtime.dispose();
  }
});
