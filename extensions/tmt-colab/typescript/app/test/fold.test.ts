import { expect, it, vi } from 'vite-plus/test';
import { Fold } from '../src/fold.js';

it('terminates a stalled decoder and rejects further use without publishing output', async () => {
  vi.useFakeTimers();
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'prepare', source: 'uncommitted' });
  const rejection = expect(pending).rejects.toThrow('time budget');
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
it('gives a single checkpoint the state bound while keeping the tail budget unchanged', async () => {
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  await expect(
    fold.run({ type: 'apply', updates: [new Uint8Array(256 * 1024 + 1)] }),
  ).rejects.toThrow('capacity');
  await expect(
    fold.run({ type: 'checkpoint', update: new Uint8Array(4 * 1024 * 1024 + 1) }),
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
      type: 'apply',
      updates: [new Uint8Array(256 * 1024)],
      own: [{ writer: '00000000-0000-4000-8000-000000000001', update: new Uint8Array([0, 0]) }],
    }),
  ).rejects.toThrow('capacity');
  expect(worker.postMessage).not.toHaveBeenCalled();
  fold.close();
});
