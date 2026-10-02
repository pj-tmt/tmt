import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import scene from '../../../../../contracts/whiteboard-scene-v1.json';
import { startLocalRuntime } from './local-runtime.js';
import { SNAPSHOT_PNG_LIMIT } from '../whiteboard/snapshot-contract.js';

const token = 'a'.repeat(43);
const id = '11111111-1111-4111-8111-111111111111';
const other = '22222222-2222-4222-8222-222222222222';
const input = {
  expectedRevision: 1,
  operationId: id,
  selectedElementIds: [],
  annotation: 'Review this.',
};
const captured = {
  id,
  documentId: 'lobby',
  documentRevision: 1,
  scene,
  selectedElementIds: [],
  annotation: input.annotation,
  createdAtMs: 100,
};

beforeEach(() => history.replaceState(null, '', `/local#token=${token}`));
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('captures and reads exact resources and sends PNG bodies through the same private session', async () => {
  const send = vi.fn(async (path: string, _init?: RequestInit) =>
    path.endsWith('/image')
      ? new Response(new Uint8Array([1, 2, 3]), { headers: { 'Content-Type': 'image/png' } })
      : new Response(JSON.stringify(captured))
  );
  vi.stubGlobal('fetch', send);
  const runtime = startLocalRuntime(window.location);
  expect(await runtime.whiteboardSnapshots.capture('lobby', input)).toEqual(captured);
  expect(await runtime.whiteboardSnapshots.show(id)).toEqual(captured);
  const original = new Blob(['PNG upload'], { type: 'image/png' });
  const image = await runtime.whiteboardSnapshots.attachImage(id, original);
  expect(image).toMatchObject({ type: 'image/png', size: 3 });
  expect(await runtime.whiteboardSnapshots.image(id)).toMatchObject({ type: 'image/png', size: 3 });
  expect(send.mock.calls[0]).toMatchObject([
    '/api/v1/local/whiteboards/lobby/snapshots',
    {
      method: 'POST',
      body: JSON.stringify(input),
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    },
  ]);
  expect(send.mock.calls[2]).toMatchObject([
    `/api/v1/local/whiteboard-snapshots/${id}/image`,
    {
      method: 'PUT',
      body: original,
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'image/png' },
    },
  ]);
  expect(send.mock.calls[3]![1]!.headers).toEqual({ Authorization: `Bearer ${token}` });
  expect(location.hash).toBe('');
  runtime.dispose();
});

it('rejects invalid inputs before transport and unrelated capture echoes without retry', async () => {
  const send = vi.fn(async () => new Response(JSON.stringify(captured)));
  vi.stubGlobal('fetch', send);
  const runtime = startLocalRuntime(window.location);
  await expect(runtime.whiteboardSnapshots.capture('../other', input)).rejects.toThrow();
  await expect(
    runtime.whiteboardSnapshots.capture('lobby', { ...input, expectedRevision: 0 })
  ).rejects.toThrow();
  await expect(runtime.whiteboardSnapshots.image('lobby')).rejects.toThrow();
  await expect(runtime.whiteboardSnapshots.attachImage(id, new Blob([]))).rejects.toThrow();
  expect(send).not.toHaveBeenCalled();
  for (const patch of [
    { id: other },
    { documentId: other },
    { documentRevision: 2 },
    { annotation: 'changed' },
    { selectedElementIds: [scene.elements[0]!.id] },
  ]) {
    send.mockImplementation(async () => new Response(JSON.stringify({ ...captured, ...patch })));
    await expect(runtime.whiteboardSnapshots.capture('lobby', input)).rejects.toThrow(
      'Unexpected whiteboard snapshot'
    );
  }
  expect(send).toHaveBeenCalledTimes(5);
  send.mockImplementation(async () => new Response(JSON.stringify({ ...captured, id: other })));
  await expect(runtime.whiteboardSnapshots.show(id)).rejects.toThrow(
    'Unexpected whiteboard snapshot'
  );
  runtime.dispose();
});

it('preserves image conflict errors and cancels oversize successful bodies', async () => {
  const send = vi.fn(
    async () => new Response('{"error":"WHITEBOARD_IDEMPOTENCY_CONFLICT"}', { status: 409 })
  );
  vi.stubGlobal('fetch', send);
  const runtime = startLocalRuntime(window.location);
  await expect(
    runtime.whiteboardSnapshots.attachImage(id, new Blob(['png'], { type: 'image/png' }))
  ).rejects.toMatchObject({ status: 409, code: 'WHITEBOARD_IDEMPOTENCY_CONFLICT' });
  expect(send).toHaveBeenCalledOnce();
  send.mockImplementation(
    async () =>
      new Response(new Uint8Array(SNAPSHOT_PNG_LIMIT), { headers: { 'Content-Type': 'image/png' } })
  );
  expect(await runtime.whiteboardSnapshots.image(id)).toHaveProperty('size', SNAPSHOT_PNG_LIMIT);
  const cancel = vi.fn();
  send.mockImplementation(
    async () =>
      new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(new Uint8Array(SNAPSHOT_PNG_LIMIT + 1));
          },
          cancel,
        }),
        { headers: { 'Content-Type': 'image/png' } }
      )
  );
  await expect(runtime.whiteboardSnapshots.image(id)).rejects.toThrow('exceeds');
  expect(cancel).toHaveBeenCalledOnce();
  send.mockImplementation(
    async () => new Response('not png', { headers: { 'Content-Type': 'text/html' } })
  );
  await expect(runtime.whiteboardSnapshots.image(id)).rejects.toThrow('Unexpected snapshot image');
  runtime.dispose();
});

it('keeps the shared deadline and lifetime active while consuming a binary body', async () => {
  vi.useFakeTimers();
  const signals: AbortSignal[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (_path: string, init: RequestInit) => {
      const signal = init.signal!;
      signals.push(signal);
      return new Response(
        new ReadableStream({
          start(controller) {
            const abort = () => controller.error(new DOMException('Aborted', 'AbortError'));
            if (signal.aborted) abort();
            else signal.addEventListener('abort', abort, { once: true });
          },
        }),
        { headers: { 'Content-Type': 'image/png' } }
      );
    })
  );
  const runtime = startLocalRuntime(window.location);
  const timed = expect(runtime.whiteboardSnapshots.image(id)).rejects.toMatchObject({
    name: 'AbortError',
  });
  await vi.advanceTimersByTimeAsync(5000);
  await timed;
  const lifetime = new AbortController();
  const cancelled = expect(
    runtime.whiteboardSnapshots.image(id, lifetime.signal)
  ).rejects.toMatchObject({ name: 'AbortError' });
  lifetime.abort();
  await cancelled;
  const pending = expect(runtime.whiteboardSnapshots.image(id)).rejects.toMatchObject({
    name: 'AbortError',
  });
  runtime.dispose();
  await pending;
  expect(signals).toHaveLength(3);
  expect(signals.every((signal) => signal.aborted)).toBe(true);
  expect(vi.getTimerCount()).toBe(0);
});
