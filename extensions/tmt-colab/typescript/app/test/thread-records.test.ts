import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { JsonValue, OwnRecord, OwnState } from '../src/fold-protocol.js';
import { ThreadStore, commentForAsk } from '../src/thread-store.js';
import {
  discussionKey,
  readThreads,
  validateDiscussionRecord,
  type ThreadRecord,
  type CommentRecord,
} from '../src/thread-records.js';
const fixture = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
);
const a = fixture.thread.senderDevice as string,
  b = fixture.comment.senderDevice as string;
const roots = () => ({ threads: {}, messages: {}, intents: {}, replies: {} });
function put(own: OwnState, record: ThreadRecord | CommentRecord, writer = record.senderDevice) {
  own[writer] ??= roots();
  own[writer][record.kind === 'thread' ? 'threads' : 'messages'][discussionKey(record)] =
    structuredClone(record) as unknown as JsonValue;
}
afterEach(() => vi.unstubAllGlobals());
it('browser admits the same literal grammar and rejects fields, keys, tombstone text and Unicode overflows', () => {
  for (const record of [fixture.thread, fixture.comment]) {
    const root = record.kind === 'thread' ? 'threads' : 'messages',
      key = discussionKey(record);
    expect(() => validateDiscussionRecord(root, key, record)).not.toThrow();
    for (const [field, invalid] of [
      ['version', 2],
      ['epoch', '01'],
      ['revision', '0'],
      ['senderDevice', 'wrong'],
      ['deleted', 'false'],
      ['deviceName', 'é'.repeat(65)],
      ['unexpected', true],
    ] as const) {
      expect(
        () => validateDiscussionRecord(root, key, { ...record, [field]: invalid }),
        `${record.kind}/${field}`,
      ).toThrow();
    }
    expect(() => validateDiscussionRecord('replies', key, record)).toThrow();
    expect(() => validateDiscussionRecord(root, 'wrong-key', record)).toThrow();
    expect(() => validateDiscussionRecord(root, key, { ...record, deleted: true })).toThrow();
    const boundary = structuredClone(record);
    if (record.kind === 'thread') boundary.anchor.prefix = '🐈'.repeat(32);
    else boundary.body = 'é'.repeat(8192);
    expect(() => validateDiscussionRecord(root, key, boundary)).not.toThrow();
    if (record.kind === 'thread') boundary.anchor.prefix += '🐈';
    else boundary.body += 'é';
    expect(() => validateDiscussionRecord(root, key, boundary)).toThrow();
  }
});
it('joins foreign replies by verified writer references; substituted scopes and absent historical keys stay inert', () => {
  const own: OwnState = {};
  put(own, fixture.thread);
  put(own, fixture.comment);
  const read = () => readThreads(own, fixture.scope, () => new Uint8Array(32));
  expect(read()[0].comments[0]).toMatchObject({
    ref: { writer: b, id: fixture.comment.messageId },
    body: fixture.comment.body,
  });
  put(own, { ...fixture.thread, resolved: true }, b);
  expect(read()).toHaveLength(1);
  expect(read()[0].resolved).toBe(false);
  put(own, { ...fixture.thread, revision: '2', epoch: '2', resolved: true });
  expect(read()[0].revision).toBe('1');
  expect(readThreads(own, fixture.scope, () => undefined)).toEqual([]);
  // A retained key remains sufficient for display after live authority is lost.
  expect(
    readThreads(own, fixture.scope, (writer) => (writer === a ? new Uint8Array(32) : undefined))[0]
      .comments,
  ).toEqual([]);
});
it('projects revisions and terminal tombstones but rejects changed references, missing revisions and resurrection', () => {
  const own: OwnState = {};
  put(own, fixture.thread);
  put(own, fixture.comment);
  const read = () => readThreads(own, fixture.scope, () => new Uint8Array(32));
  put(own, { ...fixture.comment, revision: '2', body: 'Edited' });
  expect(read()[0].comments[0].body).toBe('Edited');
  put(own, { ...fixture.comment, revision: '3', body: '', deleted: true });
  expect(read()[0].comments[0]).toMatchObject({ revision: '3', body: '', deleted: true });
  put(own, { ...fixture.comment, revision: '4', body: 'Resurrected' });
  expect(read()[0].comments).toEqual([]);
  delete own[b].messages[`${fixture.comment.messageId}:4`];
  put(own, { ...fixture.thread, revision: '3', resolved: true });
  expect(read()).toEqual([]);
  delete own[a].threads[`${fixture.thread.threadId}:3`];
  put(own, {
    ...fixture.comment,
    revision: '2',
    thread: { writer: b, id: fixture.thread.threadId },
    body: 'Moved',
  });
  expect(read()[0].comments).toEqual([]);
});
function storeFixture() {
  let device = a,
    denied = false,
    failed = false;
  const own: OwnState = {},
    batches: OwnRecord[][] = [];
  let lane = Promise.resolve();
  vi.stubGlobal('navigator', {
    locks: {
      request: (_key: string, action: () => Promise<void>) => {
        const task = lane.then(action);
        lane = task.catch(() => {});
        return task;
      },
    },
  });
  const c = {
    active: true,
    objects: { ownSigningKey: () => new Uint8Array(32) },
    admission: {
      head: { revision: 1n },
      root: {},
      validatePage() {},
      author() {
        if (denied) throw new Error('Denied');
      },
    },
    run: async <T>(action: () => Promise<T>) => action(),
  } as unknown as Connection;
  const store = new ThreadStore({
    ...fixture.scope,
    sharing: 'private',
    deviceId: () => device,
    deviceName: () => 'Browser',
    own: () => own,
    available: () => true,
    connection: async () => c,
    publish: async (records) => {
      if (failed) throw new Error('Publication failed');
      batches.push(structuredClone(records));
      for (const r of records) {
        own[device] ??= roots();
        own[device][r.root][r.key] = structuredClone(r.value);
      }
    },
  });
  return {
    store,
    own,
    batches,
    device(value: string) {
      device = value;
    },
    deny() {
      denied = true;
    },
    fail(value: boolean) {
      failed = value;
    },
  };
}
it('publishes thread/opening comment atomically, preserves failures, and prevents stale/concurrent/foreign edits', async () => {
  const f = storeFixture();
  f.fail(true);
  await expect(f.store.create('Opening', fixture.thread.anchor)).rejects.toThrow();
  expect(f.own).toEqual({});
  f.fail(false);
  await f.store.create('Opening', fixture.thread.anchor);
  expect(f.batches).toHaveLength(1);
  expect(f.batches[0]).toHaveLength(2);
  const read = () => readThreads(f.own, fixture.scope, () => new Uint8Array(32));
  const thread = read()[0],
    message = thread.comments[0];
  const edits = await Promise.allSettled([
    f.store.edit(message.ref, '1', 'First'),
    f.store.edit(message.ref, '1', 'Second'),
  ]);
  expect(edits.filter((v) => v.status === 'fulfilled')).toHaveLength(1);
  expect(read()[0].comments[0].body).toBe('First');
  f.device(b);
  await f.store.reply(thread.ref, 'Other device reply');
  expect(read()[0].comments).toHaveLength(2);
  await expect(f.store.edit(message.ref, '2', 'Foreign edit')).rejects.toThrow();
  await expect(f.store.updateThread(thread.ref, '1', { resolved: true })).rejects.toThrow();
  f.device(a);
  await f.store.updateThread(thread.ref, '1', { resolved: true });
  expect(read()[0].resolved).toBe(true);
  f.deny();
  await expect(f.store.reply(thread.ref, 'Revoked')).rejects.toThrow();
  expect(read()[0].comments).toHaveLength(2);
});
it('Ask gets exact verified comment context and rejects stale revisions or duplicate UUID origins', () => {
  const own: OwnState = {};
  put(own, fixture.thread);
  put(own, fixture.comment);
  const read = () => readThreads(own, fixture.scope, () => new Uint8Array(32));
  const thread = read()[0],
    comment = thread.comments[0];
  const context = {
    thread: thread.ref,
    message: comment.ref,
    threadRevision: '1',
    messageRevision: '1',
  };
  expect(commentForAsk(read(), context)).toEqual({
    thread: thread.threadId,
    messageIds: [comment.messageId],
    quote: thread.anchor!.exact,
    comment: comment.body,
  });
  expect(() => commentForAsk(read(), { ...context, messageRevision: '2' })).toThrow();
  put(own, { ...fixture.thread, senderDevice: b });
  expect(() => commentForAsk(read(), context)).toThrow();
});
