import { afterEach, beforeEach, describe, expect, it, vi } from 'vite-plus/test';
import example from '../../../../../contracts/whiteboard-scene-v1.json';
import { startLocalRuntime } from './local-runtime.js';
import { decodeWhiteboardScene } from '../whiteboard/scene-contract.js';

const token = 'a'.repeat(43);
const operationId = '11111111-1111-4111-8111-111111111111';
const otherId = '22222222-2222-4222-8222-222222222222';
const document = { id: 'lobby', revision: 1, scene: example, updatedAtMs: 100 };
const receipt = { documentId: 'lobby', operationId, revision: 1, changed: true, updatedAtMs: 100 };
const input = () => ({ expectedRevision: 0, operationId, scene: decodeWhiteboardScene(example) });

describe('local whiteboard port', () => {
  beforeEach(() => history.replaceState(null, '', `/local#token=${token}`));
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('reads and explicitly saves through the shared authenticated transport, retaining retry identity', async () => {
    const fetch = vi.fn(
      async (_path: string, init?: RequestInit) =>
        new Response(JSON.stringify(init?.method === 'PUT' ? receipt : document))
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.whiteboards.show('lobby')).resolves.toEqual(document);
    const draft = input();
    await expect(runtime.whiteboards.save('lobby', draft)).resolves.toEqual(receipt);
    await expect(runtime.whiteboards.save('lobby', draft)).resolves.toEqual(receipt);
    expect(fetch.mock.calls[0]?.[0]).toBe('/api/v1/local/whiteboards/lobby');
    expect(fetch.mock.calls[0]?.[1]?.method).toBeUndefined();
    expect(fetch.mock.calls[1]?.[1]).toMatchObject({
      method: 'PUT',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify(draft),
    });
    expect(fetch.mock.calls[2]?.[1]?.body).toBe(fetch.mock.calls[1]?.[1]?.body);
    expect(location.hash).toBe('');
    runtime.dispose();
  });

  it('rejects an invalid path or scene before transport and rejects unrelated document/operation receipts', async () => {
    const fetch = vi.fn(async () => new Response(JSON.stringify(receipt)));
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.whiteboards.show('../private')).rejects.toThrow();
    const invalid = input();
    invalid.scene.elements[0]!.id = 'invalid';
    await expect(runtime.whiteboards.save('lobby', invalid)).rejects.toThrow();
    expect(fetch).not.toHaveBeenCalled();
    for (const patch of [
      { documentId: otherId },
      { operationId: otherId },
      { revision: 2 },
      { changed: false },
    ]) {
      fetch.mockImplementation(async () => new Response(JSON.stringify({ ...receipt, ...patch })));
      await expect(runtime.whiteboards.save('lobby', input())).rejects.toThrow(
        'Unexpected whiteboard receipt'
      );
    }
    fetch.mockImplementation(
      async () => new Response(JSON.stringify({ ...document, id: otherId }))
    );
    await expect(runtime.whiteboards.show('lobby')).rejects.toThrow(
      'Unexpected whiteboard document'
    );
    runtime.dispose();
  });

  it('preserves conflict codes and the caller draft without a hidden retry or reload', async () => {
    const fetch = vi.fn(
      async () => new Response('{"error":"WHITEBOARD_REVISION_CONFLICT"}', { status: 409 })
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    const draft = input();
    await expect(runtime.whiteboards.save('lobby', draft)).rejects.toMatchObject({
      status: 409,
      code: 'WHITEBOARD_REVISION_CONFLICT',
    });
    expect(draft).toEqual(input());
    expect(fetch).toHaveBeenCalledTimes(1);
    runtime.dispose();
  });

  it('uses shared cancellation and disposal for reads and saves without leaking timeout handles', async () => {
    vi.useFakeTimers();
    const signals: AbortSignal[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(
        (_path: string, init: RequestInit) =>
          new Promise((_resolve, reject) => {
            const signal = init.signal!;
            signals.push(signal);
            const abort = () => reject(new DOMException('Aborted', 'AbortError'));
            if (signal.aborted) abort();
            else signal.addEventListener('abort', abort, { once: true });
          })
      )
    );
    const runtime = startLocalRuntime(window.location);
    const lifetime = new AbortController();
    const reading = expect(
      runtime.whiteboards.show('lobby', lifetime.signal)
    ).rejects.toMatchObject({ name: 'AbortError' });
    lifetime.abort();
    await reading;
    const saving = expect(runtime.whiteboards.save('lobby', input())).rejects.toMatchObject({
      name: 'AbortError',
    });
    runtime.dispose();
    await saving;
    expect(signals).toHaveLength(2);
    expect(signals.every((signal) => signal.aborted)).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });
});
