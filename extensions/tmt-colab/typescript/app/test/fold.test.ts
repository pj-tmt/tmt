import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import { Fold, decodeBudgetMs } from '../src/fold.js';
import {
  WRITE_TAIL_BYTES,
  validateProjection,
  type DecoderCommand,
  type FoldCommand,
} from '../src/fold-protocol.js';
import { contentBase } from './content-base.js';
import shellVectors from '../../../contracts/vectors/shell-v1.json';
import workVectors from '../../../contracts/vectors/work-v1.json';
import * as Y from 'yjs';

it('validates structured shell projections with the shared corpus and refuses HTML replacement', () => {
  for (const fixture of shellVectors.cases) {
    if ('action' in fixture) continue;
    const projection = {
      source: '',
      title: 'Shell',
      pageType: 'shell' as const,
      shell: fixture.document,
    };
    if (fixture.valid) expect(() => validateProjection(projection)).not.toThrow();
    else expect(() => validateProjection(projection)).toThrow();
  }
  const base = {
    source: '',
    title: 'Shell',
    pageType: 'shell' as const,
    shell: { version: 1 as const, items: {}, order: [] },
    own: {},
  };
  for (const bad of [
    { ...base, source: '<p>HTML</p>' },
    { ...base, attachments: [] },
    { ...base, pageType: undefined },
    { ...base, shell: undefined },
  ])
    expect(() => validateProjection(bad)).toThrow();
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  expect(() => fold.prepareContent('erase', base)).toThrow('cannot replace a shell');
  expect(worker.postMessage).not.toHaveBeenCalled();
  fold.close();
});

it('validates work on an ordinary page with the shared corpus and never beside the shell page type', () => {
  for (const fixture of workVectors.cases) {
    if ('action' in fixture) continue;
    const projection = { source: '<p>page</p>', title: 'Page', work: fixture.document };
    if (fixture.valid) expect(() => validateProjection(projection)).not.toThrow();
    else expect(() => validateProjection(projection)).toThrow();
  }
  const shell = {
    source: '',
    title: 'Shell',
    pageType: 'shell' as const,
    shell: { version: 1 as const, items: {}, order: [] },
  };
  expect(() =>
    validateProjection({ ...shell, work: { version: 1, next: 1, tasks: {}, order: [] } }),
  ).toThrow();
});

it('carries admitted work through the parent and rejects a preparation that drops or changes it', async () => {
  const work = {
    version: 1 as const,
    next: 2,
    tasks: { 'T-1': { title: 'Write it', state: 'todo' as const } },
    order: ['T-1'],
  };
  const open = async () => {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: unknown) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.run({ type: 'apply', updates: [] });
    worker.onmessage({
      data: {
        id: 1,
        source: '<p>page</p>',
        title: 'Page',
        work,
        own: {},
        update: new Uint8Array([0, 0]),
      },
    });
    const initial = await pending;
    return { worker, fold, initial, base: contentBase(initial) };
  };
  const reply = (worker: { onmessage: (event: unknown) => void }, projection: object) =>
    worker.onmessage({
      data: {
        id: 2,
        type: 'prepared-content',
        kind: 'updates',
        projection,
        updates: [new Uint8Array([0, 0])],
      },
    });
  const kept = await open();
  expect(kept.initial.work).toEqual(work);
  const prepared = kept.fold.prepareContent('<p>changed</p>', kept.base);
  reply(kept.worker, { ...kept.base, source: '<p>changed</p>' });
  expect((await prepared).projection.work).toEqual(work);
  expect(kept.worker.terminate).not.toHaveBeenCalled();
  for (const tampered of [
    // The reply drops the work, or changes it, or invents it on a page that has none.
    (base: object) => {
      const { work: _dropped, ...rest } = base as { work: unknown };
      return rest;
    },
    (base: object) => ({ ...base, work: { ...work, next: 3 } }),
  ]) {
    const { worker, fold, base } = await open();
    const rejected = expect(fold.prepareContent('<p>changed</p>', base)).rejects.toThrow();
    reply(worker, { ...tampered(base), source: '<p>changed</p>' });
    await rejected;
    expect(worker.terminate).toHaveBeenCalledOnce();
  }
});

it('the actual Worker reads work from the document and keeps it through a source edit', async () => {
  const contentUpdate = (work?: unknown) => {
    const doc = new Y.Doc();
    doc.transact(() => {
      doc.getText('html').insert(0, '<p>page</p>');
      doc.getMap('meta').set('title', 'Page');
      if (work !== undefined) doc.getMap('meta').set('work', work);
    });
    return Y.encodeStateAsUpdate(doc);
  };
  const work = {
    version: 1,
    next: 2,
    tasks: { 'T-1': { title: 'Write it', state: 'todo' } },
    order: ['T-1'],
  };
  const open = async () => {
    vi.resetModules();
    const surface = {
      onmessage: null as unknown as (event: {
        data: { id: number; command: DecoderCommand };
      }) => Promise<void>,
      postMessage: vi.fn(),
    };
    vi.stubGlobal('self', surface);
    await import('../src/fold.worker.js');
    return async (command: DecoderCommand) => {
      surface.postMessage.mockClear();
      await surface.onmessage({ data: { id: 1, command } });
      return surface.postMessage.mock.calls[0][0];
    };
  };
  try {
    const send = await open();
    const applied = await send({ type: 'apply', updates: [contentUpdate(work)] });
    expect(applied.error).toBeUndefined();
    expect(applied.work).toEqual(work);
    const edit = await send({
      type: 'prepare-content',
      source: '<p>Later source</p>',
      base: contentBase(applied),
    });
    expect(edit.error).toBeUndefined();
    expect(edit.projection.work).toEqual(work);
    const later = await send({ type: 'apply', updates: edit.updates });
    expect(later.source).toBe('<p>Later source</p>');
    expect(later.work).toEqual(work);
    // A page with no work projects none, and a value the contract refuses never projects.
    expect(
      Object.hasOwn(await (await open())({ type: 'apply', updates: [contentUpdate()] }), 'work'),
    ).toBe(false);
    const invalid = await (
      await open()
    )({
      type: 'apply',
      updates: [contentUpdate({ ...work, order: ['T-9'] })],
    });
    expect(invalid.error).toBeDefined();
  } finally {
    vi.unstubAllGlobals();
  }
});

it('terminates a stalled decoder and rejects further use without publishing output', async () => {
  vi.useFakeTimers();
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'apply', updates: [] });
  const rejection = expect(pending).rejects.toThrow('time budget');
  await expect(fold.prepareContent('second', { source: '', title: '', own: {} })).rejects.toThrow(
    'unavailable',
  );
  await vi.advanceTimersByTimeAsync(2000);
  await rejection;
  expect(worker.terminate).toHaveBeenCalledOnce();
  await expect(fold.run({ type: 'apply', updates: [] })).rejects.toThrow('unavailable');
  vi.useRealTimers();
});
it('gives a command 1 s more per MiB of input before terminating a stalled decoder', async () => {
  vi.useFakeTimers();
  const MiB = 1024 * 1024;
  const worker = { postMessage: vi.fn(), terminate: vi.fn(), onerror: null, onmessage: null };
  const fold = new Fold(worker as unknown as Worker);
  expect(decodeBudgetMs({ type: 'apply', updates: [] })).toBe(2000);
  expect(decodeBudgetMs({ type: 'checkpoint', update: new Uint8Array(1) })).toBe(2000);
  expect(decodeBudgetMs({ type: 'checkpoint', update: new Uint8Array(MiB) })).toBe(3000);
  // 3 MiB of input: a 5 s budget. The old fixed 2 s would already have cut it off.
  const updates = [new Uint8Array(MiB), new Uint8Array(MiB), new Uint8Array(MiB)];
  expect(decodeBudgetMs({ type: 'apply', updates })).toBe(5000);
  const pending = fold.run({ type: 'apply', updates });
  const rejection = expect(pending).rejects.toThrow('time budget');
  await vi.advanceTimersByTimeAsync(4999);
  expect(worker.terminate).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  await rejection;
  expect(worker.terminate).toHaveBeenCalledOnce();
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

it('preserves creation attribution from the strict Worker result without using it as authority', async () => {
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onerror: null,
    onmessage: null as unknown as (event: unknown) => void,
  };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'apply', updates: [] });
  worker.onmessage({
    data: {
      id: 1,
      source: 'page',
      title: 'Title',
      originalAuthor: '<creator>',
      publisherAgent: 'later-agent',
      own: {},
      update: new Uint8Array(),
    },
  });
  expect((await pending).originalAuthor).toBe('<creator>');
  expect(worker.terminate).not.toHaveBeenCalled();
  fold.close();
});
it('rejects malformed original-author labels at the parent projection boundary', async () => {
  for (const originalAuthor of ['', 1, 'line\nbreak', '🚀'.repeat(33)]) {
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
      data: { id: 1, source: '', title: '', originalAuthor, own: {}, update: new Uint8Array() },
    });
    await rejected;
    expect(worker.terminate).toHaveBeenCalledOnce();
  }
});
it('admits a layout only as the kind alone, or a canvas with its trimmed one-line reason', () => {
  const page = { source: '', title: '' };
  for (const ok of [
    {},
    { layout: 'column' },
    { layout: 'full' },
    { layout: 'canvas', layoutReason: 'A status board' },
    { layout: 'canvas', layoutReason: 'x'.repeat(1024) },
  ])
    expect(() => validateProjection({ ...page, ...ok })).not.toThrow();
  for (const bad of [
    { layout: 'freeform' },
    { layout: 'wide' },
    { layout: 1 },
    { layout: 'canvas' },
    { layout: 'canvas', layoutReason: '' },
    { layout: 'canvas', layoutReason: ' padded ' },
    { layout: 'canvas', layoutReason: 'two\nlines' },
    { layout: 'canvas', layoutReason: 'x'.repeat(1025) },
    { layout: 'canvas', layoutReason: '🚀'.repeat(257) },
    { layout: 'column', layoutReason: 'not for a column' },
    { layoutReason: 'orphan reason' },
    { layout: 'column', pageType: 'shell', shell: {} },
  ])
    expect(() => validateProjection({ ...page, ...bad }), JSON.stringify(bad)).toThrow();
});
it('preserves layout from the strict Worker result and refuses a malformed one', async () => {
  const run = async (data: Record<string, unknown>) => {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: unknown) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.run({ type: 'apply', updates: [] });
    worker.onmessage({
      data: { id: 1, source: '', title: '', own: {}, update: new Uint8Array(), ...data },
    });
    return { pending, worker, fold };
  };
  const ok = await run({ layout: 'canvas', layoutReason: 'A board' });
  const result = await ok.pending;
  expect(result.layout).toBe('canvas');
  expect(result.layoutReason).toBe('A board');
  ok.fold.close();
  for (const bad of [{ layout: 'wide' }, { layout: 'canvas' }, { layoutReason: 'orphan' }]) {
    const rejected = await run(bad);
    await expect(rejected.pending).rejects.toThrow();
    expect(rejected.worker.terminate).toHaveBeenCalledOnce();
  }
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
          data: { id: number; command: DecoderCommand };
        }) => Promise<void>,
        postMessage: vi.fn(),
      };
      vi.stubGlobal('self', surface);
      try {
        await import('../src/fold.worker.js');
        const send = async (command: DecoderCommand) => {
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
        expect(baseline.layout).toBe(vector.layout);
        expect(baseline.layoutReason).toBe(vector.layoutReason);
        const edit = await send({
          type: 'prepare-content',
          source: '<p>Later source</p>',
          base: contentBase(baseline),
        });
        expect(edit.error).toBeUndefined();
        expect(edit.projection.creationRecipient).toEqual(vector.creationRecipient);
        expect(edit.projection.layout).toBe(vector.layout);
        expect(edit.projection.layoutReason).toBe(vector.layoutReason);
        const applied = await send({ type: 'apply', updates: edit.updates });
        expect(applied.source).toBe('<p>Later source</p>');
        expect(applied.creationRecipient).toEqual(vector.creationRecipient);
        expect(applied.layout).toBe(vector.layout);
        expect(applied.layoutReason).toBe(vector.layoutReason);
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
it('preserves the original author in parent preparation and rejects changed or dropped replies', async () => {
  const base = { source: 'old', title: 'T', own: {}, originalAuthor: 'Creator' };
  for (const replacement of ['same', 'dropped', 'changed']) {
    const worker = {
      postMessage: vi.fn(),
      terminate: vi.fn(),
      onerror: null,
      onmessage: null as unknown as (event: unknown) => void,
    };
    const fold = new Fold(worker as unknown as Worker);
    const pending = fold.prepareContent('new', base);
    const outcome = replacement === 'same' ? pending : expect(pending).rejects.toThrow();
    worker.onmessage({
      data: {
        id: 1,
        type: 'prepared-content',
        kind: 'updates',
        projection: {
          source: 'new',
          title: 'T',
          own: {},
          ...(replacement === 'dropped'
            ? {}
            : { originalAuthor: replacement === 'same' ? 'Creator' : 'Someone else' }),
        },
        updates: [new Uint8Array([1])],
      },
    });
    if (replacement === 'same') {
      expect((await pending).projection.originalAuthor).toBe('Creator');
      fold.close();
    } else {
      await outcome;
      expect(worker.terminate).toHaveBeenCalledOnce();
    }
  }
});
it('carries admitted attachments through the parent and rejects a preparation that drops them', async () => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
  );
  const projection = JSON.parse(
    corpus.cases.find((c: { name: string }) => c.name === 'projection-document').input,
  );
  const worker = {
    postMessage: vi.fn(),
    terminate: vi.fn(),
    onerror: null,
    onmessage: null as unknown as (event: unknown) => void,
  };
  const fold = new Fold(worker as unknown as Worker);
  const pending = fold.run({ type: 'apply', updates: [] });
  worker.onmessage({
    data: {
      id: 1,
      source: projection.html,
      title: projection.meta.title,
      attachments: projection.meta.attachments,
      own: {},
      update: new Uint8Array([0, 0]),
    },
  });
  const initial = await pending;
  expect(initial.attachments).toEqual(projection.meta.attachments);
  const base = contentBase(initial);
  const prepared = fold.prepareContent('changed source', base);
  worker.onmessage({
    data: {
      id: 2,
      type: 'prepared-content',
      kind: 'updates',
      projection: { ...base, source: 'changed source' },
      updates: [new Uint8Array([0, 0])],
    },
  });
  expect((await prepared).projection.attachments).toEqual(initial.attachments);
  const dropped = fold.prepareContent('changed source', base);
  const rejected = expect(dropped).rejects.toThrow();
  worker.onmessage({
    data: {
      id: 3,
      type: 'prepared-content',
      kind: 'updates',
      projection: { ...base, source: 'changed source', attachments: [] },
      updates: [new Uint8Array([0, 0])],
    },
  });
  await rejected;
  expect(worker.terminate).toHaveBeenCalledOnce();
});
