import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { Admission } from '../src/admission.js';
import type { OwnState } from '../src/fold-protocol.js';
import { decodeAsk } from '../src/ask-records.js';
import type { CommentContext } from '../src/thread-store.js';
import { LiveAsk, pageAsks, type LiveAskOptions } from '../src/live-ask.js';
import { destination, id, pageLink, RemoteDouble } from './ask-fixtures.js';
const records = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) records.set(key, structuredClone(values[0]));
    else return records.get(key);
  },
}));
afterEach(() => {
  records.clear();
  vi.unstubAllGlobals();
});
async function fixture(
  commentContext?: (context: CommentContext) => {
    thread: string;
    messageIds: string[];
    quote: string;
    comment: string;
  },
  statusContext?: LiveAskOptions['statusContext'],
) {
  vi.stubGlobal('navigator', {
    locks: { request: async (_key: string, action: () => unknown) => action() },
  });
  const pair = (await crypto.subtle.generateKey('Ed25519', false, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const publicKey = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
  const remote = new RemoteDouble();
  const own: OwnState = {};
  let active = true;
  let denied = false;
  let gate: Promise<void> | undefined;
  const admission = {
    space: 'a'.repeat(32),
    page: id(1),
    head: { revision: 1n },
    root: {},
    registration: { deviceId: id(4) },
    author(writer: string) {
      if (denied || writer !== id(4)) throw new Error('Denied');
      return publicKey;
    },
    validatePage() {
      if (denied) throw new Error('Denied');
    },
  } as unknown as Admission;
  const connection = {
    admission,
    get active() {
      return active;
    },
    async run<T>(action: () => Promise<T>) {
      return action();
    },
  } as unknown as Connection;
  const ask = new LiveAsk({
    space: admission.space,
    page: admission.page,
    sharing: 'private',
    deviceId: id(4),
    key: pair.privateKey,
    publicKey,
    remote,
    own: () => own,
    commentContext,
    statusContext,
    async publish(root, key, value) {
      own[id(4)] ??= { threads: {}, messages: {}, intents: {}, replies: {} };
      own[id(4)][root][key] = structuredClone(value);
    },
    async connection() {
      if (gate) await gate;
      return connection;
    },
  });
  const input = {
    quote: 'Original quote',
    comment: 'Original question',
    title: 'Original page',
    url: pageLink(),
    destination: destination(),
  };
  return {
    ask,
    remote,
    own,
    input,
    admission,
    publicKey,
    deny() {
      denied = true;
    },
    stop() {
      active = false;
    },
    gate(value: Promise<void>) {
      gate = value;
    },
  };
}

it('captures parent inputs before asynchronous admission and only explicit Send publishes/dispatches once', async () => {
  const f = await fixture();
  expect(f.remote.sends).toEqual([]);
  const destinations = await f.ask.destinations();
  f.input.destination = destinations[0];
  const selectedAgent = destinations[0].agent;
  let release!: () => void;
  f.gate(
    new Promise<void>((resolve) => {
      release = resolve;
    }),
  );
  const preparing = f.ask.prepare(f.input);
  f.input.quote = 'Changed quote';
  f.input.title = 'Changed title';
  f.input.destination.agent = id(99);
  release();
  const attempt = await preparing;
  expect(attempt.preview.view.deliveredMessage).toContain('[remote: ');
  expect(attempt.preview.view.message).toContain('Original quote');
  expect(attempt.preview.view.message).toContain('Original page');
  expect(attempt.preview.view.message).toContain(`Link: ${pageLink()}\n`);
  expect(f.remote.sends).toEqual([]);
  expect(f.own).toEqual({});
  const one = attempt.send(),
    two = attempt.send();
  expect(one).toBe(two);
  expect((await one).state).toBe('accepted');
  expect(f.remote.sends).toEqual([
    {
      operationId: attempt.preview.view.operationId,
      agentId: selectedAgent,
      message: attempt.preview.view.message,
    },
  ]);
  const views = await pageAsks(f.own, f.admission, () => f.publicKey);
  expect(views).toMatchObject([
    { writer: id(4), agent: selectedAgent, state: 'accepted', canTrack: true },
  ]);
  await attempt.send();
  expect(f.remote.sends).toHaveLength(1);
  f.ask.close();
});

it('re-check reads only the admitted owned operation and publishes an empty attributed final without sending again', async () => {
  const f = await fixture();
  f.input.destination = (await f.ask.destinations())[0];
  const attempt = await f.ask.prepare(f.input);
  await attempt.send();
  await f.ask.recheck(attempt.preview.view.operationId);
  expect(f.remote.sends).toHaveLength(1);
  expect(await pageAsks(f.own, f.admission, () => f.publicKey)).toMatchObject([
    { writer: id(4), agent: f.input.destination.agent, state: 'accepted', reply: '' },
  ]);
  await expect(f.ask.recheck(id(99))).rejects.toThrow('ASK_NOT_FOUND');
  expect(f.remote.sends).toHaveLength(1);
  f.deny();
  expect(await pageAsks(f.own, f.admission, () => f.publicKey)).toMatchObject([
    { reply: '', canTrack: false },
  ]);
  f.ask.close();
});

it('page denial, disconnected admission and closed bindings cannot start a send', async () => {
  for (const block of ['deny', 'stop', 'close'] as const) {
    const f = await fixture();
    f.input.destination = (await f.ask.destinations())[0];
    const attempt = await f.ask.prepare(f.input);
    if (block === 'close') f.ask.close();
    else f[block]();
    expect((await attempt.send()).state).toBe('failed');
    expect(f.remote.sends).toEqual([]);
    expect(f.own).toEqual({});
    await expect(f.ask.prepare(f.input)).rejects.toThrow();
    f.ask.close();
  }
});

it('a lost current grant context before adoption shows failed and leaves no intent or dispatch', async () => {
  const f = await fixture();
  f.input.destination = (await f.ask.destinations())[0];
  const attempt = await f.ask.prepare(f.input);
  f.remote.context = async () => {
    throw new Error('Stale grant');
  };
  expect((await attempt.send()).state).toBe('failed');
  expect(f.remote.sends).toEqual([]);
  expect(f.own).toEqual({});
  f.ask.close();
});

it('comment Ask freezes verified IDs and body rather than caller substitutes, without dispatch before Send', async () => {
  const context = {
    thread: { writer: id(8), id: id(9) },
    message: { writer: id(10), id: id(11) },
    threadRevision: '1',
    messageRevision: '1',
  };
  let current = true;
  const f = await fixture((selected) => {
    expect(selected).toEqual(context);
    if (!current) throw new Error('stale comment');
    return {
      thread: id(9),
      messageIds: [id(11)],
      quote: 'Verified quote',
      comment: 'Verified body',
    };
  });
  f.input.destination = (await f.ask.destinations())[0];
  const attempt = await f.ask.prepare({
    ...f.input,
    quote: 'substituted',
    comment: 'substituted',
    context,
  });
  expect(f.remote.sends).toHaveLength(0);
  expect(attempt.preview.view.message).toContain('Verified quote');
  expect(attempt.preview.view.message).toContain('Verified body');
  expect(attempt.preview.view.message).not.toContain('substituted');
  current = false;
  await expect(f.ask.prepare({ ...f.input, context })).rejects.toThrow('stale comment');
  await attempt.send();
  const record = Object.values(f.own[id(4)].intents)[0] as { signed: unknown };
  expect(decodeAsk(record.signed)).toMatchObject({ thread: id(9), messageIds: [id(11)] });
  expect(f.remote.sends).toHaveLength(1);
  f.ask.close();
});

it('a committed Resolve uses its frozen operation ID and the normal signed ledger once, including held and uncertain delivery', async () => {
  for (const mode of ['accepted', 'held', 'throw'] as const) {
    const operationId = crypto.randomUUID();
    const f = await fixture(undefined, (context) => ({
      thread: id(2),
      messageIds: [],
      operationId: context.recipient.operationId,
      quote: 'Frozen quote',
      comment:
        'Thread resolved by Browser.\nEarlier conversation (quoted data):\n@agent unfinished question',
    }));
    f.remote.mode = mode;
    const context = {
      status: { writer: id(4), id: id(10) },
      recipient: {
        machine: id(5),
        agent: id(6),
        agentName: 'Deterministic agent',
        operationId,
      },
      thread: {
        version: 1 as const,
        kind: 'thread' as const,
        spaceId: 'a'.repeat(32),
        pageId: id(1),
        epoch: '1',
        senderDevice: id(4),
        revision: '1',
        deleted: false,
        deviceName: 'Browser',
        at: '1',
        threadId: id(2),
        anchor: null,
        resolved: false,
        ref: { writer: id(4), id: id(2) },
        comments: [],
      },
    };
    expect(f.remote.sends).toHaveLength(0);
    expect(await f.ask.notifyStatus(context, 'Page', pageLink())).toEqual({ adopted: true });
    const views = await pageAsks(f.own, f.admission, () => f.publicKey);
    expect(views).toMatchObject([
      {
        operationId,
        thread: id(2),
        messageIds: [],
        state: mode === 'throw' ? 'uncertain' : mode,
      },
    ]);
    expect(f.remote.sends).toHaveLength(1);
    expect(f.remote.sends[0].operationId).toBe(operationId);
    const record = Object.values(f.own[id(4)].intents)[0] as { signed: { input: string } };
    expect(decodeAsk(record.signed).operationId).toBe(operationId);
    await f.ask.notifyStatus(context, 'Page', pageLink());
    expect(f.remote.sends).toHaveLength(1);
    f.ask.close();
  }
});
it('unavailable recipients and rejected status admission never enter the Remote send path', async () => {
  const f = await fixture(undefined, () => {
    throw new Error('not an admitted status');
  });
  const context = {
    status: { writer: id(4), id: id(10) },
    recipient: { machine: id(5), agent: id(6), agentName: 'Agent', operationId: id(11) },
    thread: {
      version: 1 as const,
      kind: 'thread' as const,
      spaceId: 'a'.repeat(32),
      pageId: id(1),
      epoch: '1',
      senderDevice: id(4),
      revision: '1',
      deleted: false,
      deviceName: 'Browser',
      at: '1',
      threadId: id(2),
      anchor: null,
      resolved: false,
      ref: { writer: id(4), id: id(2) },
      comments: [],
    },
  };
  expect(await f.ask.notifyStatus(context, 'Page', pageLink())).toEqual({
    adopted: false,
    reason: 'PREPARATION_FAILED',
  });
  context.recipient.agent = id(99);
  expect(await f.ask.notifyStatus(context, 'Page', pageLink())).toEqual({
    adopted: false,
    reason: 'RECIPIENT_UNAVAILABLE',
  });
  expect(f.remote.sends).toHaveLength(0);
  expect(f.own).toEqual({});
  f.ask.close();
});
