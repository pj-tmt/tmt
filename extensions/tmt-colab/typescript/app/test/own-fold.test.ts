import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as Y from 'yjs';
import { binary } from '@tmt/colab-client';
import type { FoldCommand, FoldResult } from '../src/fold-protocol.js';
const v = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/own-v1.json', import.meta.url), 'utf8'),
);
const a = '00000000-0000-4000-8000-000000000001';
const b = '00000000-0000-4000-8000-000000000002';
const bytes = (raw: string) => binary(raw, 4 * 1024 * 1024);
afterEach(() => vi.unstubAllGlobals());
async function worker() {
  vi.resetModules();
  let result: FoldResult & { error?: string };
  const host = {
    onmessage: null as unknown as (event: unknown) => Promise<void>,
    postMessage(value: typeof result) {
      result = structuredClone(value);
    },
  };
  vi.stubGlobal('self', host);
  await import('../src/fold.worker.js');
  let id = 0;
  return async (command: FoldCommand) => {
    await host.onmessage({ data: { id: ++id, command } });
    if (result.error) throw new Error(result.error);
    return result;
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
  const edit = await run({ type: 'prepare', source: 'content only' });
  const content = await run({ type: 'apply', updates: [edit.update] });
  expect(content.source).toBe('content only');
  expect(content.own).toEqual(final.own);
});
it('checks do not commit own drafts and failed combined candidates change no committed document', async () => {
  const run = await worker();
  await run({ type: 'check', updates: [], own: [{ writer: a, update: bytes(v.checkpoint) }] });
  expect((await run({ type: 'apply', updates: [] })).own).toEqual({});
  const edit = await run({ type: 'prepare', source: 'must not commit' });
  await expect(
    run({
      type: 'apply',
      updates: [edit.update],
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

it('bounds aggregate content and all own state rather than allocating 4 MiB per writer', async () => {
  const run = await worker(),
    doc = new Y.Doc();
  doc.clientID = 1264;
  doc.getMap('replies').set('large', 'x'.repeat(2 * 1024 * 1024));
  const update = Y.encodeStateAsUpdate(doc);
  doc.destroy();
  await run({ type: 'checkpoint', writer: a, update });
  await expect(run({ type: 'checkpoint', writer: b, update })).rejects.toThrow();
  const after = await run({ type: 'apply', updates: [] });
  expect(Object.keys(after.own)).toEqual([a]);
  expect(after.own[a].replies.large).toHaveLength(2 * 1024 * 1024);
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
