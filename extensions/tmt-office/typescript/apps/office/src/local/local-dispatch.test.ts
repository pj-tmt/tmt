import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { startLocalRuntime } from './local-runtime.js';
import {
  decodeDispatchInput,
  decodeDispatchReceipt,
  DISPATCH_MESSAGE_LIMIT,
  DispatchRejected,
} from './dispatch-contract.js';

const operationId = '11111111-1111-4111-8111-111111111111';
const recipientId = '22222222-2222-4222-8222-222222222222';
const otherId = '33333333-3333-4333-8333-333333333333';
const input = { operationId, recipientIds: [recipientId], message: '  Look here.\n' };
const receipt = {
  operationId,
  createdAtMs: 100,
  items: [{ recipientId, requestId: `req_${otherId}`, acceptance: 'queued' }],
};

beforeEach(() => history.replaceState(null, '', `/local#token=${'a'.repeat(43)}`));
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('admits exact text and canonicalizes only the selected audience', () => {
  expect(decodeDispatchInput({ ...input, message: 'Robot 🤖\n' }).message).toBe('Robot 🤖\n');
  expect(decodeDispatchInput({ ...input, recipientIds: [otherId, recipientId, otherId] })).toEqual({
    ...input,
    recipientIds: [recipientId, otherId],
  });
  expect(
    decodeDispatchInput({ ...input, message: 'é'.repeat(DISPATCH_MESSAGE_LIMIT / 2) }).message
      .length
  ).toBe(DISPATCH_MESSAGE_LIMIT / 2);
  for (const patch of [
    { recipientIds: [] },
    { recipientIds: Array(65).fill(recipientId) },
    { recipientIds: ['00000000-0000-0000-0000-000000000000'] },
    { message: ' \n\t' },
    { message: '\u0085' },
    { message: '\uD800' },
    { message: '\uDC00' },
    { message: 'é'.repeat(DISPATCH_MESSAGE_LIMIT / 2) + 'x' },
    { command: 'ignored is not allowed' },
  ])
    expect(() => decodeDispatchInput({ ...input, ...patch })).toThrow();
});

it('rejects malformed, duplicate or unsorted receipt items and unknown fields', () => {
  expect(decodeDispatchReceipt(receipt)).toEqual(receipt);
  const original = receipt.items[0]!;
  for (const patch of [
    { createdAtMs: 0 },
    { createdAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { items: [] },
    { items: [original, original] },
    { items: [{ ...original, recipientId: otherId }, original] },
    { items: [{ ...original, acceptance: 'delivered' }] },
    { items: [{ ...original, requestId: otherId }] },
    { items: [{ ...original, proof: 'unexpected' }] },
    { token: 'unexpected' },
  ])
    expect(() => decodeDispatchReceipt({ ...receipt, ...patch })).toThrow();
});

it('decodes optional wake truth separately from immutable inbox acceptance', () => {
  for (const [status, paneAttempted] of [
    ['notAttempted', false],
    ['unknown', null],
    ['sent', true],
    ['unavailable', false],
    ['uncertain', true],
  ] as const) {
    const withWake = { ...receipt, wake: { status, paneAttempted, agentProcessed: null } };
    expect(decodeDispatchReceipt(withWake)).toEqual(withWake);
    expect(() =>
      decodeDispatchReceipt({
        ...withWake,
        wake: { ...withWake.wake, paneAttempted: !paneAttempted },
      })
    ).toThrow();
  }
  expect(() =>
    decodeDispatchReceipt({
      ...receipt,
      wake: { status: 'processed', paneAttempted: true, agentProcessed: true },
    })
  ).toThrow();
});

it('distinguishes direct room context from an explicit full-roster fence', () => {
  const direct = { kind: 'direct', roomId: otherId };
  const roster = { kind: 'roster', roomId: otherId, revision: 2 };
  expect(decodeDispatchInput({ ...input, room: direct }).room).toEqual(direct);
  expect(decodeDispatchInput({ ...input, room: roster }).room).toEqual(roster);
  expect(() =>
    decodeDispatchInput({ ...input, recipientIds: [recipientId, otherId], room: direct })
  ).toThrow();
  for (const room of [
    { roomId: otherId, revision: 2 },
    { ...direct, revision: 2 },
    { kind: 'roster', roomId: otherId },
    { ...roster, revision: Number.MAX_SAFE_INTEGER + 1 },
    { ...roster, revision: 0 },
    { kind: 'unknown', roomId: otherId },
    { kind: 'direct', roomId: 'Design' },
  ])
    expect(() => decodeDispatchInput({ ...input, room })).toThrow();
});

it('preserves explicit announcement policy while normalizing the existing request default', async () => {
  expect(decodeDispatchInput({ ...input, kind: 'request' })).toEqual(input);
  const announcement = { ...input, kind: 'announcement' as const };
  expect(decodeDispatchInput(announcement)).toEqual(announcement);
  for (const kind of [null, undefined, 'broadcast', 'response', 1])
    expect(() => decodeDispatchInput({ ...input, kind })).toThrow();
  const fetch = vi.fn(async () => new Response(JSON.stringify(receipt)));
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.dispatch.send(announcement)).resolves.toEqual(receipt);
  expect(fetch).toHaveBeenCalledTimes(1);
  expect(fetch).toHaveBeenCalledWith(
    '/api/v1/local/dispatch',
    expect.objectContaining({ body: JSON.stringify(announcement) })
  );
  runtime.dispose();
});

it('sends only on explicit invocation through the shared authenticated transport and preserves retry identity', async () => {
  const fetch = vi.fn(async () => new Response(JSON.stringify(receipt)));
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  expect(fetch).not.toHaveBeenCalled();
  await expect(runtime.dispatch.send(input)).resolves.toEqual(receipt);
  await expect(runtime.dispatch.send(input)).resolves.toEqual(receipt);
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(fetch).toHaveBeenNthCalledWith(
    1,
    '/api/v1/local/dispatch',
    expect.objectContaining({
      method: 'POST',
      body: JSON.stringify(input),
      headers: { Authorization: `Bearer ${'a'.repeat(43)}`, 'Content-Type': 'application/json' },
    })
  );
  expect(input.message).toBe('  Look here.\n');
  runtime.dispose();
});

it('rejects invalid input before transport and unrelated operation or audience receipts after transport', async () => {
  const fetch = vi.fn(async () => new Response(JSON.stringify(receipt)));
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.dispatch.send({ ...input, recipientIds: [] })).rejects.toThrow();
  expect(fetch).not.toHaveBeenCalled();
  for (const patch of [
    { operationId: otherId },
    { items: [{ ...receipt.items[0], recipientId: otherId }] },
  ]) {
    fetch.mockImplementation(async () => new Response(JSON.stringify({ ...receipt, ...patch })));
    await expect(runtime.dispatch.send(input)).rejects.toThrow('Unexpected dispatch receipt');
  }
  fetch.mockImplementation(
    async () => new Response('{"error":"DISPATCH_IDEMPOTENCY_CONFLICT"}', { status: 409 })
  );
  await expect(runtime.dispatch.send(input)).rejects.toMatchObject({
    reason: 'operationConflict',
  });
  expect(fetch).toHaveBeenCalledTimes(3);
  runtime.dispose();
});

it('classifies only exact definitive host rejections, preserving unknown failures as uncertain', async () => {
  const fetch = vi.fn();
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  for (const [status, code, reason] of [
    [400, 'DISPATCH_INVALID', 'invalid'],
    [409, 'DISPATCH_IDEMPOTENCY_CONFLICT', 'operationConflict'],
    [409, 'ROOM_RECIPIENT_NOT_MEMBER', 'roomMembership'],
  ] as const) {
    fetch.mockResolvedValue(new Response(JSON.stringify({ error: code }), { status }));
    await expect(runtime.dispatch.send(input)).rejects.toEqual(new DispatchRejected(reason));
  }
  fetch.mockResolvedValue(new Response('{"error":"DISPATCH_INVALID"}', { status: 500 }));
  await expect(runtime.dispatch.send(input)).rejects.not.toBeInstanceOf(DispatchRejected);
  runtime.dispose();
});

it('uses shared caller cancellation, timeout and runtime disposal with no hidden retries', async () => {
  vi.useFakeTimers();
  const signals: AbortSignal[] = [];
  const fetch = vi.fn(
    (_path: string, init: RequestInit) =>
      new Promise<Response>((_resolve, reject) => {
        const signal = init.signal!;
        signals.push(signal);
        const abort = () => reject(new DOMException('Aborted', 'AbortError'));
        if (signal.aborted) abort();
        else signal.addEventListener('abort', abort, { once: true });
      })
  );
  vi.stubGlobal('fetch', fetch);
  const runtime = startLocalRuntime(window.location);
  const lifetime = new AbortController();
  const cancelled = expect(runtime.dispatch.send(input, lifetime.signal)).rejects.toMatchObject({
    name: 'AbortError',
  });
  lifetime.abort();
  await cancelled;
  const timedOut = expect(runtime.dispatch.send(input)).rejects.toMatchObject({
    name: 'AbortError',
  });
  await vi.advanceTimersByTimeAsync(5_000);
  await timedOut;
  const disposed = expect(runtime.dispatch.send(input)).rejects.toMatchObject({
    name: 'AbortError',
  });
  runtime.dispose();
  await disposed;
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(signals.every((signal) => signal.aborted)).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});
