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

it('rejects a present undefined creation recipient projection', () => {
  expect(() =>
    validateProjection({ source: '', title: '', creationRecipient: undefined }),
  ).toThrow();
});
it('rejects encoded undefined creation metadata without changing committed Worker state', async () => {
  vi.resetModules();
  const surface = {
    onmessage: null as unknown as (event: {
      data: { id: number; command: FoldCommand };
    }) => Promise<void>,
    postMessage: vi.fn(),
  };
  vi.stubGlobal('self', surface);
  const Y = await import('yjs');
  const doc = new Y.Doc();
  try {
    await import('../src/fold.worker.js');
    const send = async (command: FoldCommand) => {
      surface.postMessage.mockClear();
      await surface.onmessage({ data: { id: 1, command } });
      return surface.postMessage.mock.calls[0][0];
    };
    doc.getText('html').insert(0, 'Committed source');
    doc.getMap('meta').set('title', 'Committed title');
    const initial = await send({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] });
    expect(initial.error).toBeUndefined();
    expect(Object.hasOwn(initial, 'creationRecipient')).toBe(false);
    const vector = Y.encodeStateVector(doc);
    doc.getMap('meta').set('creationRecipient', undefined);
    doc.getText('html').insert(0, 'Untrusted change ');
    const update = Y.encodeStateAsUpdate(doc, vector);
    const decoded = new Y.Doc();
    try {
      Y.applyUpdate(decoded, Y.encodeStateAsUpdate(doc));
      expect(decoded.getMap('meta').has('creationRecipient')).toBe(true);
      expect(decoded.getMap('meta').get('creationRecipient')).toBeUndefined();
    } finally {
      decoded.destroy();
    }
    const rejected = await send({ type: 'apply', updates: [update] });
    expect(rejected.error).toBe('Rejected content update');
    const unchanged = await send({ type: 'apply', updates: [] });
    expect(unchanged.error).toBeUndefined();
    expect(unchanged.source).toBe('Committed source');
    expect(unchanged.title).toBe('Committed title');
    expect(Object.hasOwn(unchanged, 'creationRecipient')).toBe(false);
  } finally {
    doc.destroy();
    vi.unstubAllGlobals();
  }
});
it('closes the parent fold for a successful reply with a present undefined creation recipient', async () => {
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onerror: null,
    onmessage: null as unknown as (event: unknown) => void,
  };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'apply', updates: [] });
  const rejected = expect(pending).rejects.toThrow();
  worker.onmessage({
    data: {
      id: 1,
      source: 'Untrusted source',
      title: '',
      creationRecipient: undefined,
      own: {},
      update: new Uint8Array(),
    },
  });
  await rejected;
  expect(worker.terminate).toHaveBeenCalledOnce();
  await expect(fold.run({ type: 'apply', updates: [] })).rejects.toThrow('unavailable');
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

it('preserves admitted creation preferences in parent preparation and rejects changed replies', async () => {
  const base = { source: 'old', title: 'T', own: {}, creationRecipient: recipient };
  for (const replacement of [
    'valid',
    'absent',
    undefined,
    { ...recipient, machineId: '40000000-0000-4000-8000-000000000002' },
    { ...recipient, agentId: '50000000-0000-1000-8000-000000000002' },
  ]) {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: unknown) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.prepareContent('new', base);
    const outcome = replacement === 'valid' ? pending : expect(pending).rejects.toThrow();
    const projection =
      replacement === 'absent'
        ? { source: 'new', title: 'T', own: {} }
        : {
            ...base,
            source: 'new',
            creationRecipient: replacement === 'valid' ? recipient : replacement,
          };
    worker.onmessage({
      data: {
        id: 1,
        type: 'prepared-content',
        kind: 'updates',
        projection,
        updates: [new Uint8Array([1])],
      },
    });
    if (replacement === 'valid') {
      expect((await pending).projection.creationRecipient).toEqual(recipient);
      const noop = fold.prepareContent('old', base);
      worker.onmessage({
        data: { id: 2, type: 'prepared-content', kind: 'noop', projection: base },
      });
      expect((await noop).projection.creationRecipient).toEqual(recipient);
      fold.close();
    } else {
      await outcome;
      expect(worker.terminate).toHaveBeenCalledOnce();
    }
  }
});
