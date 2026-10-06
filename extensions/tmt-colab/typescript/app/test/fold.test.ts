import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import { Fold } from '../src/fold.js';
import { WRITE_TAIL_BYTES, validateProjection, type FoldCommand } from '../src/fold-protocol.js';

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

const recipient = {
  machineId: '40000000-0000-4000-8000-000000000001',
  agentId: '50000000-0000-1000-8000-000000000001',
};
it('admits only complete canonical creation preferences without inferring missing provenance', () => {
  expect(() =>
    validateProjection({ source: '', title: '', creationRecipient: recipient }),
  ).not.toThrow();
  expect(() => validateProjection({ source: '', title: '' })).not.toThrow();
  for (const bad of [
    null,
    {},
    { machineId: recipient.machineId },
    { ...recipient, extra: true },
    { ...recipient, machineId: recipient.agentId },
    { ...recipient, agentId: '00000000-0000-0000-0000-000000000000' },
    { ...recipient, agentId: 'ABCDEF00-0000-1000-8000-000000000001' },
  ]) {
    expect(() => validateProjection({ source: '', title: '', creationRecipient: bad })).toThrow();
  }
});
it('the actual Worker preserves both independent baseline encodings through source edits', async () => {
  const vectors = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/baseline-v1.json', import.meta.url), 'utf8'),
  );
  const decode = (value: string) => new Uint8Array(Buffer.from(value, 'base64url'));
  for (const vector of vectors) {
    for (const alternate of vector.alternateUpdate ? [false, true] : [false]) {
      vi.resetModules();
      const surface = {
        onmessage: null as unknown as (event: {
          data: { id: number; command: FoldCommand };
        }) => Promise<void>,
        postMessage: vi.fn(),
      };
      vi.stubGlobal('self', surface);
      try {
        await import('../src/fold.worker.js');
        const send = async (command: FoldCommand) => {
          surface.postMessage.mockClear();
          await surface.onmessage({ data: { id: 1, command } });
          return surface.postMessage.mock.calls[0][0];
        };
        if (vector.alternateUpdate) {
          const wrong = await send({
            type: 'baseline',
            title: vector.title,
            sourceDigest: decode(vector.sourceDigest),
            update: decode(alternate ? vector.alternateUpdate : vector.update),
            commitment: decode(alternate ? vector.commitment : vector.alternateCommitment),
          });
          expect(wrong.error).toBe('Rejected content update');
        }
        const baseline = await send({
          type: 'baseline',
          title: vector.title,
          sourceDigest: decode(vector.sourceDigest),
          update: decode(alternate ? vector.alternateUpdate : vector.update),
          commitment: decode(alternate ? vector.alternateCommitment : vector.commitment),
        });
        expect(baseline.error).toBeUndefined();
        expect(baseline.creationRecipient).toEqual(vector.creationRecipient);
        const edit = await send({ type: 'prepare', source: '<p>Later source</p>' });
        expect(edit.error).toBeUndefined();
        expect(edit.creationRecipient).toEqual(vector.creationRecipient);
        const applied = await send({ type: 'apply', updates: [edit.update] });
        expect(applied.source).toBe('<p>Later source</p>');
        expect(applied.creationRecipient).toEqual(vector.creationRecipient);
      } finally {
        vi.unstubAllGlobals();
      }
    }
  }
});
