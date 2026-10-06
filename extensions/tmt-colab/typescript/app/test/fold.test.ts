import { expect, it, vi } from 'vite-plus/test';
import { Fold } from '../src/fold.js';
import { WRITE_TAIL_BYTES } from '../src/fold-protocol.js';

it('terminates a stalled decoder and rejects further use without publishing output', async () => {
  vi.useFakeTimers();
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'prepare', source: 'uncommitted' });
  const rejection = expect(pending).rejects.toThrow('time budget');
  await expect(fold.prepareContent('second', { source: '', title: '', own: {} })).rejects.toThrow(
    'unavailable',
  );
  await vi.advanceTimersByTimeAsync(2000);
  await rejection;
  expect(worker.terminate).toHaveBeenCalledOnce();
  await expect(fold.run({ type: 'prepare', source: 'late' })).rejects.toThrow('unavailable');
  vi.useRealTimers();
});
it('permits only one in-flight decoder and clears it on route cleanup', async () => {
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'apply', updates: [] });
  const rejection = expect(pending).rejects.toThrow('failed');
  await expect(fold.run({ type: 'apply', updates: [] })).rejects.toThrow('unavailable');
  fold.close();
  await rejection;
});
it('gives a checkpoint the read bound while keeping the write tail budget unchanged', async () => {
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  await expect(
    fold.run({ type: 'check', updates: [new Uint8Array(WRITE_TAIL_BYTES + 1)] }),
  ).rejects.toThrow('capacity');
  await expect(
    fold.run({ type: 'checkpoint', update: new Uint8Array(24 * 1024 * 1024 + 1) }),
  ).rejects.toThrow('checkpoint capacity');
  expect(worker.postMessage).not.toHaveBeenCalled();
  const accepted = fold.run({ type: 'checkpoint', update: new Uint8Array(300 * 1024) });
  expect(worker.postMessage).toHaveBeenCalledOnce();
  const rejection = expect(accepted).rejects.toThrow('failed');
  fold.close();
  await rejection;
});

it('rejects unadmitted own writers and malformed raw maps from the Worker', async () => {
  for (const own of [
    {
      '00000000-0000-4000-8000-000000000002': {
        threads: {},
        messages: {},
        intents: {},
        replies: {},
      },
    },
    {
      '00000000-0000-4000-8000-000000000001': {
        threads: [],
        messages: {},
        intents: {},
        replies: {},
      },
    },
    {
      '00000000-0000-4000-8000-000000000001': {
        threads: {},
        messages: { id: { body: 42 } },
        intents: {},
        replies: {},
      },
    },
  ]) {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: unknown) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.run({
      type: 'apply',
      updates: [],
      own: [{ writer: '00000000-0000-4000-8000-000000000001', update: new Uint8Array([0, 0]) }],
    });
    const rejected = expect(pending).rejects.toThrow();
    worker.onmessage({ data: { id: 1, source: '', title: '', update: new Uint8Array(), own } });
    await rejected;
    expect(worker.terminate).toHaveBeenCalledOnce();
  }
});
it('bounds aggregate content and own input before posting to the Worker', async () => {
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  await expect(
    fold.run({
      type: 'check',
      updates: [new Uint8Array(WRITE_TAIL_BYTES)],
      own: [{ writer: '00000000-0000-4000-8000-000000000001', update: new Uint8Array([0, 0]) }],
    }),
  ).rejects.toThrow('capacity');
  expect(worker.postMessage).not.toHaveBeenCalled();
  fold.close();
});

it('admits only correlated bounded content batches with the exact expected projection', async () => {
  const base = { source: 'old', title: 'T', publisherAgent: 'P', own: {} };
  const valid = {
    id: 1,
    type: 'prepared-content',
    kind: 'updates',
    projection: { ...base, source: 'new' },
    updates: [new Uint8Array([1])],
  };
  const mutations = [
    { id: 2 },
    { type: 'prepare' },
    { extra: true },
    { kind: 'invalid' },
    { projection: { ...valid.projection, source: 'wrong' } },
    { projection: { ...valid.projection, title: 'wrong' } },
    { projection: { ...valid.projection, publisherAgent: 'wrong' } },
    { projection: { ...valid.projection, extra: true } },
    { projection: { ...valid.projection, own: [] } },
    {
      projection: {
        ...valid.projection,
        own: {
          '00000000-0000-4000-8000-000000000002': {
            threads: {},
            messages: {},
            intents: {},
            replies: {},
          },
        },
      },
    },
    { updates: [] },
    { updates: ['bad'] },
    { updates: [new Uint8Array()] },
    { updates: [new Uint8Array(256 * 1024 + 1)] },
    { updates: Array.from({ length: 201 }, () => new Uint8Array([1])) },
    { updates: Array.from({ length: 17 }, () => new Uint8Array(256 * 1024)) },
    { kind: 'noop', updates: undefined },
  ];
  for (const mutation of mutations) {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: MessageEvent<unknown>) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.prepareContent('new', base);
    const rejection = expect(pending).rejects.toThrow('failed');
    worker.onmessage({ data: { ...valid, ...mutation } } as MessageEvent<unknown>);
    await rejection;
    expect(worker.terminate).toHaveBeenCalledOnce();
  }
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onerror: null,
    onmessage: null as unknown as (event: MessageEvent<unknown>) => void,
  };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.prepareContent('new', base);
  worker.onmessage({ data: valid } as MessageEvent<unknown>);
  expect((await pending).kind).toBe('updates');
  const noop = fold.prepareContent('old', base);
  worker.onmessage({
    data: { id: 2, type: 'prepared-content', kind: 'noop', projection: base },
  } as MessageEvent<unknown>);
  expect((await noop).kind).toBe('noop');
  fold.close();
});
it('keeps the batch preparation deadline and rejects unsupported source before worker dispatch', async () => {
  vi.useFakeTimers();
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  const base = { source: '', title: '', own: {} };
  expect(() => fold.prepareContent('x'.repeat(2 * 1024 * 1024 + 1), base)).toThrow();
  expect(worker.postMessage).not.toHaveBeenCalled();
  const pending = fold.prepareContent('new', base);
  const rejection = expect(pending).rejects.toThrow('time budget');
  await expect(fold.prepareContent('second', { source: '', title: '', own: {} })).rejects.toThrow(
    'unavailable',
  );
  await vi.advanceTimersByTimeAsync(2000);
  await rejection;
  expect(worker.terminate).toHaveBeenCalledOnce();
  vi.useRealTimers();
});
