import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LocalHttpError, startLocalRuntime } from './local-runtime.js';
import { REQUEST_HISTORY_PAGE_BYTES } from './request-history-contract.js';

const recipientId = '11111111-1111-4111-8111-111111111111';
const operationId = '22222222-2222-4222-8222-222222222222';
const requestId = 'req_33333333-3333-4333-8333-333333333333';
const token = 'a'.repeat(43);
const item = {
  requestId,
  roomId: null,
  recipientId,
  sender: { kind: 'unknown', identityId: null },
  kind: 'request',
  preparedAtMs: 1000,
  delivery: 'queued',
  recipientAcknowledged: false,
  final: { status: 'not_submitted' },
};

describe('local request inspection transport', () => {
  beforeEach(() => history.replaceState(null, '', `/local#token=${token}`));
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('uses authenticated typed POST reads and never exposes acknowledgment or sends', async () => {
    const fetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({ items: [{ ...item, preview: 'Question' }], nextBefore: null })
        )
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.requests.list({ recipientId })).resolves.toMatchObject({
      items: [{ requestId }],
    });
    expect(fetch).toHaveBeenCalledExactlyOnceWith(
      '/api/v1/local/requests/list',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({ recipientId, limit: 20 }),
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      })
    );
    runtime.dispose();
  });

  it('rejects detail and receipt replies for a different operation or request', async () => {
    const fetch = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            ...item,
            requestId: `req_${operationId}`,
            prompt: { status: 'unavailable' },
          })
        )
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.requests.show(requestId)).rejects.toThrow('Unexpected request detail');
    fetch.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({
            operationId: recipientId,
            createdAtMs: 1000,
            items: [{ recipientId, requestId, acceptance: 'queued' }],
          })
        )
    );
    await expect(runtime.requests.receipt(operationId)).rejects.toThrow(
      'Unexpected dispatch receipt'
    );
    runtime.dispose();
  });

  it('recovers a receipt without resending and distinguishes absence from a service failure', async () => {
    const receipt = {
      operationId,
      createdAtMs: 1000,
      items: [{ recipientId, requestId, acceptance: 'queued' }],
    };
    const fetch = vi.fn(
      async (_path: string, _init?: RequestInit) => new Response(JSON.stringify(receipt))
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.requests.receipt(operationId)).resolves.toEqual(receipt);
    expect(fetch.mock.calls[0]?.[0]).toBe('/api/v1/local/dispatch/show');
    fetch.mockImplementation(
      async () => new Response('{"error":"DISPATCH_NOT_FOUND"}', { status: 404 })
    );
    await expect(runtime.requests.receipt(operationId)).resolves.toBeNull();
    fetch.mockImplementation(
      async () => new Response('{"error":"STORAGE_UNAVAILABLE"}', { status: 500 })
    );
    await expect(runtime.requests.receipt(operationId)).rejects.toBeInstanceOf(LocalHttpError);
    expect(fetch).toHaveBeenCalledTimes(3);
    runtime.dispose();
  });

  it('bounds history responses and rejects invalid queries before network effects', async () => {
    const fetch = vi.fn(async () => new Response(' '.repeat(REQUEST_HISTORY_PAGE_BYTES + 1)));
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    await expect(runtime.requests.list({ recipientId: 'Alice' })).rejects.toThrow();
    expect(fetch).not.toHaveBeenCalled();
    await expect(runtime.requests.list({ recipientId })).rejects.toThrow('exceeds its limit');
    runtime.dispose();
  });

  it('aborts inspection on view cancellation, runtime disposal and request deadline', async () => {
    vi.useFakeTimers();
    const fetch = vi.fn(
      (_path: string, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener(
            'abort',
            () => reject(new DOMException('Stopped', 'AbortError')),
            { once: true }
          );
        })
    );
    vi.stubGlobal('fetch', fetch);
    const runtime = startLocalRuntime(window.location);
    const lifetime = new AbortController();
    const cancelled = expect(
      runtime.requests.list({ recipientId }, lifetime.signal)
    ).rejects.toThrow('Stopped');
    lifetime.abort();
    await cancelled;
    const deadline = expect(runtime.requests.show(requestId)).rejects.toThrow('Stopped');
    await vi.advanceTimersByTimeAsync(5000);
    await deadline;
    const disposed = expect(runtime.requests.receipt(operationId)).rejects.toThrow('Stopped');
    runtime.dispose();
    await disposed;
    expect(vi.getTimerCount()).toBe(0);
  });
});
