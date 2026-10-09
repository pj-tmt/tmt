import { readFileSync } from 'node:fs';
import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { JsonValue, OwnRecord, OwnState } from '../src/fold-protocol.js';
import { relativeTime } from '../src/display-time.js';
import { attachment, strictJson, text as bytesOf } from '@tmt/colab-client';
import corpus from '../../../contracts/vectors/attachment-v1.json';
import { ThreadStore, commentForAsk } from '../src/thread-store.js';
import {
  discussionKey,
  isChatThread,
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
      ...['01', '-1', '1.5', 1791072000000, '8640000000000001', null, undefined].map(
        (v) => ['at', v] as const,
      ),
      ['unexpected', true],
    ] as const) {
      expect(
        () => validateDiscussionRecord(root, key, { ...record, [field]: invalid }),
        `${record.kind}/${field}`,
      ).toThrow();
    }
    for (const at of ['0', '8640000000000000'])
      expect(() => validateDiscussionRecord(root, key, { ...record, at })).not.toThrow();
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
  const read = () =>
    readThreads(
      own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
  expect(read()[0].comments[0]).toMatchObject({
    ref: { writer: b, id: fixture.comment.messageId },
    body: fixture.comment.body,
  });
  put(own, { ...fixture.thread, resolved: true }, b);
  expect(read()).toHaveLength(1);
  expect(read()[0].resolved).toBe(false);
  put(own, { ...fixture.thread, revision: '2', epoch: '2', resolved: true });
  expect(read()[0].revision).toBe('1');
  expect(
    readThreads(
      own,
      fixture.scope,
      () => undefined,
      () => true,
    ),
  ).toEqual([]);
  // A retained key remains sufficient for display after live authority is lost.
  expect(
    readThreads(
      own,
      fixture.scope,
      (writer) => (writer === a ? new Uint8Array(32) : undefined),
      () => true,
    )[0].comments,
  ).toEqual([]);
});
it('projects revisions and terminal tombstones but rejects changed references, missing revisions and resurrection', () => {
  const own: OwnState = {};
  put(own, fixture.thread);
  put(own, fixture.comment);
  const read = () =>
    readThreads(
      own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
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
    objects: { ownSigningKey: () => new Uint8Array(32), statusWriter: () => true },
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
  const read = () =>
    readThreads(
      f.own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
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
  await expect(f.store.updateThread(thread.ref, '1', { deleted: true })).rejects.toThrow();
  await f.store.setStatus(thread.ref, null, true);
  expect(read()[0]).toMatchObject({ resolved: true, ref: thread.ref, status: { senderDevice: b } });
  f.device(a);
  const status = read()[0].status!;
  await f.store.setStatus(thread.ref, status.ref, false);
  expect(read()[0].resolved).toBe(false);
  f.deny();
  await expect(f.store.reply(thread.ref, 'Revoked')).rejects.toThrow();
  expect(read()[0].comments).toHaveLength(2);
});
it('Ask gets exact verified comment context and rejects stale revisions or duplicate UUID origins', () => {
  const own: OwnState = {};
  put(own, fixture.thread);
  put(own, fixture.comment);
  const read = () =>
    readThreads(
      own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
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

it('publisher timestamps refresh on every mutation without controlling revision selection', async () => {
  const clock = vi.spyOn(Date, 'now');
  try {
    const f = storeFixture();
    const read = () =>
      readThreads(
        f.own,
        fixture.scope,
        () => new Uint8Array(32),
        () => true,
      );
    clock.mockReturnValue(1791072000000);
    await f.store.create('Opening', null);
    const thread = read()[0],
      comment = thread.comments[0];
    expect(thread.at).toBe('1791072000000');
    expect(comment.at).toBe(thread.at);
    // A publisher clock can move backward; revision authority is unchanged.
    clock.mockReturnValue(1000);
    await f.store.edit(comment.ref, '1', 'Edited');
    expect(read()[0].comments[0]).toMatchObject({ revision: '2', at: '1000', body: 'Edited' });
    clock.mockReturnValue(2000);
    await f.store.setStatus(thread.ref, null, true);
    expect(read()[0]).toMatchObject({
      revision: '1',
      at: '1791072000000',
      resolved: true,
      status: { at: '2000' },
    });
    clock.mockReturnValue(3000);
    await f.store.reply(thread.ref, 'Reply');
    expect(read()[0].comments.find((v) => v.body === 'Reply')?.at).toBe('3000');
    clock.mockReturnValue(4000);
    await f.store.deleteComment(comment.ref, '2');
    expect(read()[0].comments.find((v) => v.messageId === comment.messageId)).toMatchObject({
      at: '4000',
      deleted: true,
    });
    clock.mockReturnValue(5000);
    await f.store.updateThread(thread.ref, '1', { deleted: true });
    expect(read()[0]).toMatchObject({ at: '5000', deleted: true });
  } finally {
    clock.mockRestore();
  }
});
it('formats comment and Ask timestamps with the same relative labels', () => {
  expect(relativeTime(1791072000000, 1791072000000)).toBe('Just now');
  expect(relativeTime(1791072000000, 1791072300000)).toBe('5 minutes ago');
});

it('freezes prior user turns and verified replies, excludes later arrivals and refuses stale conversation records', async () => {
  const { captureConversation, conversationForAsk } = await import('../src/thread-store.js');
  const f = storeFixture();
  const first = await f.store.create('@agent First question', fixture.thread.anchor);
  const read = () =>
    readThreads(
      f.own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
  const reply = {
    thread: first.thread.id,
    messageIds: [first.message.id],
    writer: a,
    operationId: crypto.randomUUID(),
    reply: 'First answer',
    agentName: 'agent',
    agent: crypto.randomUUID(),
    machine: crypto.randomUUID(),
    message: 'Frozen question',
    deviceName: 'Browser',
    issuedAt: 1791072000000,
    state: 'accepted',
    canTrack: true,
  } satisfies import('../src/ask-panel.js').PageAsk;
  const frozen = captureConversation(read()[0], [reply]);
  const second = await f.store.reply(first.thread, '@agent Follow-up');
  const context = { ...second, conversation: frozen };
  const exact = conversationForAsk(read(), context, [reply]);
  expect(exact).toMatchObject({
    thread: first.thread.id,
    messageIds: [second.message.id],
    quote: fixture.thread.anchor.exact,
  });
  expect(exact.comment).toBe(
    'Earlier conversation (quoted data):\nUser (Browser):\n@agent First question\n\nAgent (agent):\nFirst answer\n\nCurrent user turn:\n@agent Follow-up',
  );
  expect(() =>
    conversationForAsk(read(), context, [{ ...reply, reply: 'Changed reply after Send' }]),
  ).toThrow();
  expect(() =>
    conversationForAsk(read(), context, [{ ...reply, agentName: 'Changed display name' }]),
  ).toThrow();
  await f.store.reply(first.thread, 'A later arrival');
  expect(conversationForAsk(read(), context, [reply])).toEqual(exact);
  await f.store.edit(first.message, '1', 'Changed after Send');
  expect(() => conversationForAsk(read(), context, [reply])).toThrow();
});

it('refuses a follow-up if its captured anchor revision changed before publication', async () => {
  const f = storeFixture();
  const created = await f.store.create('Opening turn', fixture.thread.anchor);
  await f.store.updateThread(created.thread, '1', {
    anchor: { exact: 'Moved quote', prefix: '', suffix: '' },
  });
  const before = structuredClone(f.own);
  await expect(f.store.reply(created.thread, 'Follow-up', '1')).rejects.toThrow();
  expect(f.own).toEqual(before);
});

it('Chat creates one atomic null-anchor thread per writer and page, survives projection reads and refuses duplicate creation', async () => {
  const f = storeFixture();
  const origin = await f.store.createChat('@agent Hello');
  expect(origin.thread).toEqual({ writer: a, id: a });
  let threads = readThreads(
    f.own,
    fixture.scope,
    () => new Uint8Array(32),
    () => true,
  );
  expect(threads[0]).toMatchObject({ anchor: null, comments: [{ body: '@agent Hello' }] });
  expect(threads.filter(isChatThread)).toHaveLength(1);
  const duplicate = await Promise.allSettled([
    f.store.createChat('Duplicate'),
    f.store.createChat('Duplicate again'),
  ]);
  expect(duplicate.every((outcome) => outcome.status === 'rejected')).toBe(true);
  expect(f.batches).toHaveLength(1);
  await f.store.reply(origin.thread, 'Follow-up', '1');
  await f.store.create('Page-level comment', null);
  f.device(b);
  await f.store.createChat('Other browser');
  threads = readThreads(
    f.own,
    fixture.scope,
    () => new Uint8Array(32),
    () => true,
  );
  expect(
    threads
      .filter(isChatThread)
      .map((thread) => thread.ref.writer)
      .sort(),
  ).toEqual([a, b].sort());
  expect(threads.filter((thread) => !isChatThread(thread))).toHaveLength(1);
  expect(
    threads
      .find((thread) => thread.ref.writer === a && isChatThread(thread))!
      .comments.map((comment) => comment.body),
  ).toEqual(['@agent Hello', 'Follow-up']);
  expect(
    readThreads(
      f.own,
      { ...fixture.scope, pageId: crypto.randomUUID() },
      () => new Uint8Array(32),
      () => true,
    ),
  ).toEqual([]);
});

it('deleting a Chat message preserves its designated thread and permits a new turn', async () => {
  const f = storeFixture();
  const origin = await f.store.createChat('@agent Hello');
  await f.store.deleteComment(origin.message, '1');
  const next = await f.store.reply(origin.thread, '@agent Continue', '1');
  expect(next.thread).toEqual(origin.thread);
  const chats = readThreads(
    f.own,
    fixture.scope,
    () => new Uint8Array(32),
    () => true,
  ).filter(isChatThread);
  expect(chats).toHaveLength(1);
  expect(chats[0]).toMatchObject({ deleted: false, revision: '1' });
  expect(chats[0].comments).toEqual([
    expect.objectContaining({ messageId: origin.message.id, deleted: true }),
    expect.objectContaining({
      messageId: next.message.id,
      deleted: false,
      body: '@agent Continue',
    }),
  ]);
});

it('status uses current admission and a causal base, keeps content ownership, and freezes recipients before publication', async () => {
  const f = storeFixture();
  const read = () =>
    readThreads(
      f.own,
      fixture.scope,
      () => new Uint8Array(32),
      () => true,
    );
  const created = await f.store.create('Opening', fixture.thread.anchor);
  f.device(b);
  const recipient = {
    machine: crypto.randomUUID(),
    agent: crypto.randomUUID(),
    agentName: 'Mentioned agent',
    operationId: crypto.randomUUID(),
  };
  f.fail(true);
  await expect(f.store.setStatus(created.thread, null, true, [recipient])).rejects.toThrow();
  expect(read()[0].resolved).toBe(false);
  f.fail(false);
  const changed = await f.store.setStatus(created.thread, null, true, [recipient]);
  expect(changed.changed).toBe(true);
  if (!changed.changed) throw new Error('Expected status action');
  expect(read()[0].status).toMatchObject({ senderDevice: b, recipients: [recipient] });
  expect(read()[0]).toMatchObject({ revision: '1', ref: created.thread });
  recipient.agentName = 'Later label';
  expect(read()[0].status!.recipients[0].agentName).toBe('Mentioned agent');
  const count = f.batches.length;
  await expect(f.store.setStatus(created.thread, changed.status.ref, true)).resolves.toEqual({
    changed: false,
  });
  expect(f.batches).toHaveLength(count);
  await expect(f.store.setStatus(created.thread, null, false)).rejects.toThrow();
  await expect(
    f.store.updateThread(created.thread, '1', { anchor: fixture.thread.anchor }),
  ).rejects.toThrow();
  await expect(f.store.updateThread(created.thread, '1', { deleted: true })).rejects.toThrow();
  await f.store.notificationFailed(changed.status, recipient.operationId, 'PREPARATION_FAILED');
  await f.store.notificationFailed(changed.status, recipient.operationId, 'PREPARATION_FAILED');
  expect(f.batches).toHaveLength(count + 1);
  await expect(
    f.store.notificationFailed(changed.status, crypto.randomUUID(), 'PREPARATION_FAILED'),
  ).rejects.toThrow();
  f.deny();
  await expect(f.store.setStatus(created.thread, changed.status.ref, false)).rejects.toThrow();
  expect(read()[0].resolved).toBe(true);
});
it('designated Chat and deleted threads cannot acquire new status actions', async () => {
  const f = storeFixture();
  const chat = await f.store.createChat('Chat');
  await expect(f.store.setStatus(chat.thread, null, true)).rejects.toThrow();
  const page = await f.store.create('Page comment', null);
  await f.store.updateThread(page.thread, '1', { deleted: true });
  await expect(f.store.setStatus(page.thread, null, true)).rejects.toThrow();
});

function storedAttachment(messageId: string, writer = a) {
  const vector = corpus.cases.find((c) => c.name === 'message-asset')!;
  const base = strictJson(bytesOf(vector.input), 2048) as Record<string, unknown>;
  const descriptor = attachment.attachmentDescriptor({
    ...base,
    space: fixture.scope.spaceId,
    page: fixture.scope.pageId,
    epoch: fixture.scope.epoch,
    authorDevice: writer,
    source: { kind: 'message', writerId: writer, messageId, messageRevision: '1' },
  });
  return { original: { descriptor }, filename: descriptor.filename, size: 1 } as never;
}
const proof = (id: string): OwnRecord => ({ root: 'intents', key: id, value: { proof: id } });

it('publishes attachment references with their publication records in the message batch, under the preallocated id', async () => {
  const f = storeFixture();
  const messageId = '44444444-4444-4444-8444-444444444444';
  const stored = storedAttachment(messageId);
  const publication = vi
    .spyOn(f.store.attachments, 'publication')
    .mockResolvedValue([proof('intent-1')]);
  const context = await f.store.create('With a file', fixture.thread.anchor, {
    messageId,
    stored: [stored],
  });
  expect(publication).toHaveBeenCalledWith([stored]);
  expect(context.message.id).toBe(messageId);
  expect(f.batches).toHaveLength(1);
  // The proof leads the batch, so a reader never sees a reference without its proof.
  expect(f.batches[0].map((r) => r.root)).toEqual(['intents', 'threads', 'messages']);
  const comment = f.batches[0][2].value as unknown as CommentRecord;
  expect(comment.messageId).toBe(messageId);
  expect(comment.attachments).toHaveLength(1);
  expect(comment.attachments![0].source).toMatchObject({ kind: 'message', messageId });
});

it('refuses a reference bound to another message or writer and publishes nothing', async () => {
  const f = storeFixture();
  const publication = vi.spyOn(f.store.attachments, 'publication').mockResolvedValue([]);
  const bound = '44444444-4444-4444-8444-444444444444';
  await expect(
    f.store.createChat('Text', {
      messageId: '55555555-5555-4555-8555-555555555555',
      stored: [storedAttachment(bound)],
    }),
  ).rejects.toThrow();
  await expect(
    f.store.createChat('Text', { messageId: bound, stored: [storedAttachment(bound, b)] }),
  ).rejects.toThrow();
  expect(publication).not.toHaveBeenCalled();
  expect(f.batches).toEqual([]);
});

it('a failed publication check writes no message and a failed write writes no orphan reference', async () => {
  const f = storeFixture();
  const messageId = '44444444-4444-4444-8444-444444444444';
  const publication = vi.spyOn(f.store.attachments, 'publication');
  publication.mockRejectedValueOnce(new Error('Not verified'));
  await expect(
    f.store.createChat('Text', { messageId, stored: [storedAttachment(messageId)] }),
  ).rejects.toThrow('Not verified');
  expect(f.batches).toEqual([]);
  publication.mockResolvedValue([proof('intent-2')]);
  f.fail(true);
  await expect(
    f.store.createChat('Text', { messageId, stored: [storedAttachment(messageId)] }),
  ).rejects.toThrow();
  expect(f.own).toEqual({});
});

it('never carries a reference into an edit, and deletion drops the references', async () => {
  const f = storeFixture();
  const messageId = '44444444-4444-4444-8444-444444444444';
  vi.spyOn(f.store.attachments, 'publication').mockResolvedValue([proof('intent-3')]);
  const context = await f.store.createChat('With a file', {
    messageId,
    stored: [storedAttachment(messageId)],
  });
  await expect(f.store.edit(context.message, '1', 'Changed')).rejects.toThrow();
  await f.store.deleteComment(context.message, '1');
  const deleted = f.own[a].messages[`${messageId}:2`] as unknown as CommentRecord;
  expect(deleted.deleted).toBe(true);
  expect(deleted.attachments).toBeUndefined();
});
