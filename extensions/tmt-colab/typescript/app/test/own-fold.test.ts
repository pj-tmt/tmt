import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as Y from 'yjs';
import { binary, digest, frame, text } from '@tmt/colab-client';
import type {
  FoldCommand,
  FoldResult,
  DecoderCommand,
  PrepareContentCommand,
  ContentPreparation,
  ContentSnapshot,
} from '../src/fold-protocol.js';
import { contentBase, updatesOf } from './content-base.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/own-v1.json', import.meta.url), 'utf8'),
);
const a = '00000000-0000-4000-8000-000000000001';
const b = '00000000-0000-4000-8000-000000000002';
const bytes = (raw: string) => binary(raw, 4 * 1024 * 1024);
afterEach(() => vi.unstubAllGlobals());
async function worker() {
  vi.resetModules();
  let result: (FoldResult | ContentPreparation) & { error?: string };
  const host = {
    onmessage: null as unknown as (event: unknown) => Promise<void>,
    postMessage(value: typeof result) {
      result = structuredClone(value);
    },
  };
  vi.stubGlobal('self', host);
  await import('../src/fold.worker.js');
  let id = 0;
  return (async (command: DecoderCommand) => {
    await host.onmessage({ data: { id: ++id, command } });
    if (result.error) throw new Error(result.error);
    return result;
  }) as {
    (command: FoldCommand): Promise<FoldResult>;
    (command: PrepareContentCommand): Promise<ContentPreparation>;
  };
}
it('matches native raw own projections and isolates colliding keys and client IDs by writer', async () => {
  const run = await worker();
  await run({ type: 'checkpoint', writer: a, update: bytes(v.checkpoint) });
  const prefix = await run({
    type: 'apply',
    updates: [],
    own: [{ writer: b, update: bytes(v.checkpoint) }],
  });
  expect(prefix.own[a]).toEqual(v.expectedPrefix);
  expect(prefix.own[b]).toEqual(v.expectedPrefix);
  const final = await run({
    type: 'apply',
    updates: [],
    own: [{ writer: a, update: bytes(v.tail) }],
  });
  expect(final.own[a]).toEqual(v.expected);
  expect(final.own[b]).toEqual(v.expectedPrefix);
  expect(final.source).toBe('');
  const edit = await run({
    type: 'prepare-content',
    source: 'content only',
    base: contentBase(final),
  });
  const content = await run({ type: 'apply', updates: updatesOf(edit) });
  expect(content.source).toBe('content only');
  expect(content.own).toEqual(final.own);
});
it('checks do not commit own drafts and failed combined candidates change no committed document', async () => {
  const run = await worker();
  await run({ type: 'check', updates: [], own: [{ writer: a, update: bytes(v.checkpoint) }] });
  const unchanged = await run({ type: 'apply', updates: [] });
  expect(unchanged.own).toEqual({});
  const edit = await run({
    type: 'prepare-content',
    source: 'must not commit',
    base: contentBase(unchanged),
  });
  await expect(
    run({
      type: 'apply',
      updates: updatesOf(edit),
      own: [{ writer: a, update: bytes(v.negative[0].update) }],
    }),
  ).rejects.toThrow();
  const committed = await run({ type: 'apply', updates: [] });
  expect(committed.source).toBe('');
  expect(committed.own).toEqual({});
});
it('preserves pending structs and delete sets across unpublished steps; final checks reject missing dependencies', async () => {
  const run = await worker();
  await run({ type: 'checkpoint', writer: a, update: bytes(v.tail) });
  await run({ type: 'checkpoint', writer: b, update: bytes(v.checkpoint) });
  await run({ type: 'checkpoint', writer: a, update: bytes(v.checkpoint) });
  const final = await run({ type: 'apply', updates: [] });
  expect(final.own[a]).toEqual(v.expected);
  const empty = await worker();
  await empty({ type: 'checkpoint', writer: a, update: bytes(v.tail) });
  await expect(empty({ type: 'apply', updates: [] })).rejects.toThrow();
});
it('shared negatives reject with positive controls and UTF-8 body boundary passes', async () => {
  const run = await worker();
  await run({ type: 'apply', updates: [], own: [{ writer: a, update: bytes(v.bodyBoundary) }] });
  for (const negative of v.negative) {
    const fresh = await worker();
    await expect(
      fresh({ type: 'apply', updates: [], own: [{ writer: a, update: bytes(negative.update) }] }),
      negative.name,
    ).rejects.toThrow();
  }
  const fresh = await worker();
  await expect(fresh({ type: 'apply', updates: [bytes(v.checkpoint)] })).rejects.toThrow();
});
it('page thread cap counts colliding writer maps separately at 1000 and rejects 1001', async () => {
  const run = await worker();
  const at = await run({
    type: 'apply',
    updates: [],
    own: [a, b].map((writer) => ({ writer, update: bytes(v.threadBatch) })),
  });
  expect(Object.keys(at.own[a].threads).length + Object.keys(at.own[b].threads).length).toBe(1000);
  await expect(
    run({ type: 'apply', updates: [], own: [{ writer: a, update: bytes(v.extraThread) }] }),
  ).rejects.toThrow();
  expect((await run({ type: 'apply', updates: [] })).own).toEqual(at.own);
});

it('bounds aggregate content and all own state rather than allocating 24 MiB per writer', async () => {
  const run = await worker(),
    doc = new Y.Doc();
  doc.clientID = 1264;
  doc.getMap('replies').set('large', 'x'.repeat(12 * 1024 * 1024 + 64));
  const update = Y.encodeStateAsUpdate(doc);
  doc.destroy();
  await run({ type: 'checkpoint', writer: a, update });
  await expect(run({ type: 'checkpoint', writer: b, update })).rejects.toThrow();
  const after = await run({ type: 'apply', updates: [] });
  expect(Object.keys(after.own)).toEqual([a]);
  expect(after.own[a].replies.large).toHaveLength(12 * 1024 * 1024 + 64);
});

it('prepares immutable own records without publishing until the durable append is admitted', async () => {
  const run = await worker();
  const value = {
    version: 1,
    kind: 'ask-state',
    operationId: a,
    revision: '1',
    state: 'dispatching',
    requestId: null,
    reason: null,
  };
  const prepared = await run({
    type: 'prepare-own',
    writer: a,
    records: [{ root: 'messages', key: `${a}:1`, value }],
  });
  expect(prepared.own[a].messages[`${a}:1`]).toEqual(value);
  expect((await run({ type: 'apply', updates: [] })).own).toEqual({});
  const admitted = await run({
    type: 'apply',
    updates: [],
    own: [{ writer: a, update: prepared.update }],
  });
  expect(admitted.own[a].messages[`${a}:1`]).toEqual(value);
  await expect(
    run({
      type: 'prepare-own',
      writer: a,
      records: [{ root: 'messages', key: `${a}:1`, value: { ...value, state: 'accepted' } }],
    }),
  ).rejects.toThrow();
  expect((await run({ type: 'apply', updates: [] })).own).toEqual(admitted.own);
});

it('thread creation prepares one atomic typed batch; malformed and conflicting candidates preserve committed state', async () => {
  const fixture = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
  );
  const run = await worker();
  const records = [
    { root: 'threads' as const, key: `${fixture.thread.threadId}:1`, value: fixture.thread },
    {
      root: 'messages' as const,
      key: `${fixture.comment.messageId}:1`,
      value: { ...fixture.comment, senderDevice: fixture.thread.senderDevice },
    },
  ];
  const draft = await run({ type: 'prepare-own', writer: fixture.thread.senderDevice, records });
  expect((await run({ type: 'apply', updates: [] })).own).toEqual({});
  const committed = await run({
    type: 'apply',
    updates: [],
    own: [{ writer: fixture.thread.senderDevice, update: draft.update }],
  });
  expect(Object.keys(committed.own[fixture.thread.senderDevice].threads)).toHaveLength(1);
  expect(Object.keys(committed.own[fixture.thread.senderDevice].messages)).toHaveLength(1);
  await expect(
    run({
      type: 'prepare-own',
      writer: fixture.thread.senderDevice,
      records: [records[0], { ...records[1], value: { ...records[1].value, body: 'conflicting' } }],
    }),
  ).rejects.toThrow();
  await expect(
    run({
      type: 'prepare-own',
      writer: fixture.thread.senderDevice,
      records: [{ ...records[0], root: 'replies' }],
    }),
  ).rejects.toThrow();
  expect((await run({ type: 'apply', updates: [] })).own).toEqual(committed.own);
});

it('admitted discussion updates cannot overwrite or remove existing immutable keys', async () => {
  const fixture = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
  );
  for (const remove of [false, true]) {
    const run = await worker(),
      doc = new Y.Doc(),
      key = `${fixture.thread.threadId}:1`;
    doc.getMap('threads').set(key, fixture.thread);
    const first = Y.encodeStateAsUpdate(doc),
      vector = Y.encodeStateVector(doc);
    const before = await run({
      type: 'apply',
      updates: [],
      own: [{ writer: fixture.thread.senderDevice, update: first }],
    });
    if (remove) doc.getMap('threads').delete(key);
    else doc.getMap('threads').set(key, { ...fixture.thread, resolved: true });
    await expect(
      run({
        type: 'apply',
        updates: [],
        own: [{ writer: fixture.thread.senderDevice, update: Y.encodeStateAsUpdate(doc, vector) }],
      }),
    ).rejects.toThrow();
    expect((await run({ type: 'apply', updates: [] })).own).toEqual(before.own);
    doc.destroy();
  }
});

function snapshot(value: FoldResult): ContentSnapshot {
  const { source, title, publisherAgent, creationRecipient, own } = value;
  return {
    source,
    title,
    ...(publisherAgent === undefined ? {} : { publisherAgent }),
    ...(Object.hasOwn(value, 'creationRecipient') ? { creationRecipient } : {}),
    own,
  };
}
it('prepares a full 1.5 MiB causal replacement, preserves foreign structs and never commits the draft', async () => {
  const run = await worker();
  const author = new Y.Doc({ guid: 'author' });
  author.clientID = 123;
  author.getText('html').insert(0, 'A'.repeat(1.5 * 1024 * 1024));
  author.getMap('meta').set('title', 'Keep title');
  author.getMap('meta').set('publisherAgent', 'Keep publisher');
  const baseline = Y.encodeStateAsUpdate(author);
  const foreign = new Y.Doc();
  foreign.clientID = 456;
  Y.applyUpdate(foreign, baseline);
  const vector = Y.encodeStateVector(foreign);
  foreign.getText('html').insert(foreign.getText('html').length, '<p>foreign</p>');
  const admitted = Y.encodeStateAsUpdate(foreign, vector);
  const before = snapshot(
    await run({
      type: 'apply',
      updates: [baseline, admitted],
      own: [{ writer: a, update: bytes(v.checkpoint) }],
    }),
  );
  const next = 'B'.repeat(1.5 * 1024 * 1024);
  const prepared = await run({ type: 'prepare-content', source: next, base: before });
  expect(prepared.kind).toBe('updates');
  if (prepared.kind !== 'updates') throw new Error('Expected full replacement');
  expect(prepared.updates.length).toBeGreaterThan(1);
  expect(prepared.updates.every((u) => u.length <= 256 * 1024)).toBe(true);
  expect(prepared.updates.reduce((n, u) => n + u.length, 0)).toBeLessThanOrEqual(4 * 1024 * 1024);
  const replay = new Y.Doc();
  Y.applyUpdate(replay, baseline);
  Y.applyUpdate(replay, admitted);
  const foreignClock = Y.decodeStateVector(Y.encodeStateVector(replay)).get(456);
  for (const update of prepared.updates) {
    Y.applyUpdate(replay, update);
    expect(replay.store.pendingStructs).toBeNull();
    expect(replay.store.pendingDs).toBeNull();
  }
  expect(replay.getText('html').toString()).toBe(next);
  expect(replay.getMap('meta').toJSON()).toEqual({
    title: 'Keep title',
    publisherAgent: 'Keep publisher',
  });
  expect(Y.decodeStateVector(Y.encodeStateVector(replay)).get(456)).toBe(foreignClock);
  expect(snapshot(await run({ type: 'apply', updates: [] }))).toEqual(before);
  const laterVector = Y.encodeStateVector(foreign);
  foreign.getText('html').insert(foreign.getText('html').length, '<p>later foreign</p>');
  const later = await run({
    type: 'apply',
    updates: [Y.encodeStateAsUpdate(foreign, laterVector)],
  });
  expect(later.source).toBe(before.source + '<p>later foreign</p>');
  expect(later.source).not.toContain('BBBB');
  author.destroy();
  foreign.destroy();
  replay.destroy();
});
it('prepares Unicode boundary changes, pure deletion, empty and explicit no-op without publishing', async () => {
  for (const [old, source] of [
    ['start 😀 middle 🐈 end', 'START 😀 middle 🐈 end'],
    ['start 😀 middle 🐈 end', 'start 😀 MIDDLE 🐈 end'],
    ['start 😀 middle 🐈 end', 'start 😀 middle 🐈 END'],
    ['x😀end', 'x😁end'],
    ['old', 'é'.repeat(96 * 1024 - 1) + '🌍漢字' + '界'.repeat(80_000)],
    ['delete me', ''],
    ['', ''],
    ['same 🐈', 'same 🐈'],
  ]) {
    const run = await worker();
    const doc = new Y.Doc();
    doc.getText('html').insert(0, old);
    doc.getMap('meta').set('title', 'T');
    const baseline = Y.encodeStateAsUpdate(doc);
    const before = snapshot(await run({ type: 'apply', updates: [baseline] }));
    const made = await run({ type: 'prepare-content', source, base: before });
    expect(made.kind).toBe(source === old ? 'noop' : 'updates');
    if (made.kind === 'updates') {
      for (const update of made.updates) Y.applyUpdate(doc, update);
      expect(doc.getText('html').toString()).toBe(source);
      expect(doc.store.pendingStructs).toBeNull();
      expect(doc.store.pendingDs).toBeNull();
    }
    expect(made.projection).toEqual({ ...before, source });
    expect(snapshot(await run({ type: 'apply', updates: [] }))).toEqual(before);
    await expect(
      run({ type: 'prepare-content', source: 'bad', base: { ...before, title: 'wrong' } }),
    ).rejects.toThrow();
    expect(snapshot(await run({ type: 'apply', updates: [] }))).toEqual(before);
    doc.destroy();
  }
});

it('Worker replay rejects unordered, unresolved and incomplete causal deltas with a positive control', async () => {
  await worker();
  const { replayContent } = await import('../src/fold.worker.js');
  const base = new Y.Doc();
  base.clientID = 123;
  base.getText('html').insert(0, 'old');
  base.getMap('meta').set('title', 'T');
  const candidate = new Y.Doc();
  candidate.clientID = 456;
  Y.applyUpdate(candidate, Y.encodeStateAsUpdate(base));
  const updates: Uint8Array[] = [];
  candidate.on('update', (bytes: Uint8Array) => updates.push(bytes));
  candidate.transact(() => {
    candidate.getText('html').delete(0, 3);
    candidate.getText('html').insert(0, 'A');
  });
  candidate.getText('html').insert(1, 'B');
  const expected = { source: 'AB', title: 'T' };
  expect(() => replayContent(base, updates, expected)).not.toThrow();
  expect(() => replayContent(base, [...updates].reverse(), expected)).toThrow();
  expect(() => replayContent(base, [updates[1]], expected)).toThrow();
  expect(() => replayContent(base, [updates[0]], expected)).toThrow();
  expect(base.getText('html').toString()).toBe('old');
  base.destroy();
  candidate.destroy();
});

const creationPreference = {
  machineId: '40000000-0000-4000-8000-000000000001',
  agentId: '50000000-0000-1000-8000-000000000001',
};
it('causal Worker preparation preserves creation preference or true absence without committing', async () => {
  for (const preference of [undefined, creationPreference]) {
    const run = await worker();
    const doc = new Y.Doc();
    try {
      doc.getText('html').insert(0, 'old');
      doc.getMap('meta').set('title', 'T');
      if (preference) doc.getMap('meta').set('creationRecipient', preference);
      const admitted = await run({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] });
      const before = snapshot(admitted);
      for (const source of ['old', 'new']) {
        const made = await run({ type: 'prepare-content', source, base: before });
        expect(made.projection).toEqual({ ...before, source });
        expect(Object.hasOwn(made.projection, 'creationRecipient')).toBe(!!preference);
        if (made.kind === 'updates') {
          const replay = new Y.Doc();
          try {
            Y.applyUpdate(replay, Y.encodeStateAsUpdate(doc));
            for (const bytes of made.updates) Y.applyUpdate(replay, bytes);
            expect(replay.getText('html').toString()).toBe('new');
            expect(replay.getMap('meta').has('creationRecipient')).toBe(!!preference);
            expect(replay.getMap('meta').get('creationRecipient')).toEqual(preference);
          } finally {
            replay.destroy();
          }
        }
        const unchanged = snapshot(await run({ type: 'apply', updates: [] }));
        expect(unchanged).toEqual(before);
      }
    } finally {
      doc.destroy();
    }
  }
});
it('Worker preparation rejects missing or mismatched admitted creation preference members', async () => {
  const run = await worker();
  const doc = new Y.Doc();
  try {
    doc.getText('html').insert(0, 'old');
    doc.getMap('meta').set('creationRecipient', creationPreference);
    const before = snapshot(await run({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] }));
    const { creationRecipient: _preference, ...absent } = before;
    for (const base of [
      absent,
      {
        ...before,
        creationRecipient: {
          ...creationPreference,
          machineId: '40000000-0000-4000-8000-000000000002',
        },
      },
      {
        ...before,
        creationRecipient: {
          ...creationPreference,
          agentId: '50000000-0000-1000-8000-000000000002',
        },
      },
      { ...before, creationRecipient: undefined },
    ]) {
      await expect(run({ type: 'prepare-content', source: 'new', base })).rejects.toThrow();
    }
    const unchanged = snapshot(await run({ type: 'apply', updates: [] }));
    expect(unchanged).toEqual(before);
  } finally {
    doc.destroy();
  }
});
it('Worker causal replay compares creation preference presence and both members', async () => {
  await worker();
  const { replayContent } = await import('../src/fold.worker.js');
  const doc = new Y.Doc();
  const candidate = new Y.Doc();
  try {
    doc.getText('html').insert(0, 'old');
    doc.getMap('meta').set('creationRecipient', creationPreference);
    Y.applyUpdate(candidate, Y.encodeStateAsUpdate(doc));
    const vector = Y.encodeStateVector(candidate);
    candidate.getText('html').insert(0, 'new ');
    const updates = [Y.encodeStateAsUpdate(candidate, vector)];
    const expected = { source: 'new old', title: '', creationRecipient: creationPreference };
    expect(() => replayContent(doc, updates, expected)).not.toThrow();
    for (const projection of [
      { source: 'new old', title: '' },
      {
        ...expected,
        creationRecipient: {
          ...creationPreference,
          machineId: '40000000-0000-4000-8000-000000000002',
        },
      },
      {
        ...expected,
        creationRecipient: {
          ...creationPreference,
          agentId: '50000000-0000-1000-8000-000000000002',
        },
      },
      { ...expected, creationRecipient: undefined },
    ])
      expect(() => replayContent(doc, updates, projection)).toThrow();
    expect(doc.getText('html').toString()).toBe('old');
  } finally {
    doc.destroy();
    candidate.destroy();
  }
});
it('preserves attachment descriptors through source preparation and checkpoints without binaries', async () => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
  );
  const c = corpus.cases.find((c: { name: string }) => c.name === 'projection-document');
  const projection = JSON.parse(c.input);
  const doc = new Y.Doc();
  doc.getText('html').insert(0, projection.html);
  for (const [key, value] of Object.entries(projection.meta)) doc.getMap('meta').set(key, value);
  const run = await worker();
  const initial = await run({ type: 'apply', updates: [Y.encodeStateAsUpdate(doc)] });
  expect(initial.attachments).toEqual(projection.meta.attachments);
  const base = contentBase(initial);
  const changed = await run({ type: 'prepare-content', base, source: 'changed source' });
  expect(changed.projection.attachments).toEqual(initial.attachments);
  await expect(
    run({ type: 'prepare-content', base: { ...base, attachments: [] }, source: 'changed source' }),
  ).rejects.toThrow();
  const committed = await run({ type: 'apply', updates: updatesOf(changed) });
  expect(committed.attachments).toEqual(initial.attachments);
  for (const update of updatesOf(changed)) Y.applyUpdate(doc, update);
  const checkpoint = Y.encodeStateAsUpdate(doc);
  const restored = await (await worker())({ type: 'checkpoint', update: checkpoint });
  expect(restored.attachments).toEqual(initial.attachments);
  expect(restored.source).toBe('changed source');
  const baseline = await (
    await worker()
  )({
    type: 'baseline',
    update: checkpoint,
    title: restored.title,
    sourceDigest: await digest(text(restored.source)),
    commitment: await digest(
      frame(text('tmt-colab-baseline-v1'), text('1'), text(restored.source), checkpoint),
    ),
  });
  expect(baseline.attachments).toEqual(initial.attachments);
  expect(baseline.source).toBe(restored.source);
  doc.destroy();
});

it('preserves attachment creation proofs through checkpoints and refuses mutation, deletion or wrong roots', async () => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/attachment-v1.json', import.meta.url), 'utf8'),
  );
  const publication = JSON.parse(
    corpus.cases.find((v: { name: string }) => v.name === 'publication-document').input,
  );
  const writer = publication.senderDevice,
    key = publication.attachmentId;
  for (const remove of [false, true]) {
    const run = await worker(),
      doc = new Y.Doc();
    doc.getMap('intents').set(key, publication);
    const checkpoint = Y.encodeStateAsUpdate(doc),
      vector = Y.encodeStateVector(doc);
    const before = await run({ type: 'checkpoint', writer, update: checkpoint });
    expect(before.own[writer].intents[key]).toEqual(publication);
    if (remove) doc.getMap('intents').delete(key);
    else doc.getMap('intents').set(key, { ...publication, descriptorHash: '00'.repeat(32) });
    await expect(
      run({
        type: 'apply',
        updates: [],
        own: [{ writer, update: Y.encodeStateAsUpdate(doc, vector) }],
      }),
    ).rejects.toThrow();
    expect((await run({ type: 'apply', updates: [] })).own).toEqual(before.own);
    doc.destroy();
  }
  for (const [root, key] of [
    ['messages', publication.attachmentId],
    ['intents', 'wrong-key'],
  ]) {
    const run = await worker(),
      doc = new Y.Doc();
    doc.getMap(root).set(key, publication);
    await expect(
      run({ type: 'apply', updates: [], own: [{ writer, update: Y.encodeStateAsUpdate(doc) }] }),
    ).rejects.toThrow();
    doc.destroy();
  }
});
