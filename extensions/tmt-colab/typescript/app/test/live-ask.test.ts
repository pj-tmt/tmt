import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { Admission } from '../src/admission.js';
import type { OwnState } from '../src/fold-protocol.js';
import { decodeAsk } from '../src/ask-records.js';
import type { CommentContext } from '../src/thread-store.js';
import { LiveAsk, pageAsks } from '../src/live-ask.js';
import { ReadRefusedError, SessionEndedError, SessionEvictedError } from '../src/ask-remote.js';
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
  const sessionEnded = vi.fn();
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
    sessionEnded,
    own: () => own,
    commentContext,
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
    sessionEnded,
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

it('status observes admitted presence without admitting an Ask preview or publishing', async () => {
  const f = await fixture();
  vi.spyOn(f.remote, 'listAgents').mockResolvedValue([
    { id: id(6), name: 'Same name', presence: 'active' },
  ]);
  const observed = await f.ask.observeDestinations();
  expect(observed.kind).toBe('ready');
  if (observed.kind !== 'ready') throw new Error('Expected admitted directory');
  expect(observed.destinations).toEqual([
    expect.objectContaining({
      agent: id(6),
      agentName: 'Same name',
      presence: 'active',
      machine: id(5),
    }),
  ]);
  expect(observed.checkedAt).toBeGreaterThan(0);
  await expect(f.ask.prepare(f.input)).rejects.toThrow();
  expect(f.own).toEqual({});
  expect(f.remote.sends).toEqual([]);
  expect(f.sessionEnded).not.toHaveBeenCalled();
});

for (const phase of ['session', 'directory'] as const) {
  for (const [error, failure, code] of [
    [new SessionEndedError('REMOTE_SESSION_ENDED'), 'ended', 'REMOTE_SESSION_ENDED'],
    [
      new SessionEndedError('REMOTE_SEQUENCE_UNAVAILABLE'),
      'unavailable',
      'REMOTE_SEQUENCE_UNAVAILABLE',
    ],
    [new SessionEvictedError(3), 'evicted', 'REMOTE_SESSION_EVICTED'],
    [new ReadRefusedError('REMOTE_SCOPE_DENIED'), 'refused', 'REMOTE_SCOPE_DENIED'],
    [new Error('private transport diagnostics'), 'unavailable', undefined],
  ] as const) {
    it(`status reports ${phase} ${failure} (${code ?? 'transport'}) without recovery; Ask retains its session policy`, async () => {
      const f = await fixture();
      if (phase === 'session') vi.spyOn(f.remote, 'context').mockRejectedValue(error);
      else vi.spyOn(f.remote, 'listAgents').mockRejectedValue(error);
      expect(await f.ask.observeDestinations()).toEqual({
        kind: 'unavailable',
        phase,
        failure,
        ...(code ? { code } : {}),
      });
      expect(f.sessionEnded).not.toHaveBeenCalled();
      expect(f.remote.sends).toEqual([]);
      expect(f.own).toEqual({});
      await expect(f.ask.destinations()).rejects.toBe(error);
      expect(f.sessionEnded).toHaveBeenCalledTimes(
        error instanceof SessionEndedError || error instanceof SessionEvictedError ? 1 : 0,
      );
    });
  }
}

it('status rejects a different admitted device before reading the directory', async () => {
  const f = await fixture();
  const original = await f.remote.context();
  vi.spyOn(f.remote, 'context').mockResolvedValue({ ...original, deviceId: id(99) });
  const list = vi.spyOn(f.remote, 'listAgents');
  expect(await f.ask.observeDestinations()).toEqual({
    kind: 'unavailable',
    phase: 'session',
    failure: 'unavailable',
  });
  expect(list).not.toHaveBeenCalled();
  expect(f.sessionEnded).not.toHaveBeenCalled();
  expect(f.remote.sends).toEqual([]);
});

it('closing the parent generation fences a pending status result', async () => {
  const f = await fixture();
  let release!: (agents: Awaited<ReturnType<typeof f.remote.listAgents>>) => void;
  vi.spyOn(f.remote, 'listAgents').mockImplementation(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const observation = f.ask.observeDestinations();
  await vi.waitFor(() => expect(release).toBeDefined());
  f.ask.close();
  release(await new RemoteDouble().listAgents());
  await expect(observation).rejects.toThrow();
  expect(f.sessionEnded).not.toHaveBeenCalled();
  expect(f.remote.sends).toEqual([]);
  expect(f.own).toEqual({});
});

it('page admission refuses status before Remote reads and never uses writer authority', async () => {
  const f = await fixture();
  const context = vi.spyOn(f.remote, 'context');
  const directory = vi.spyOn(f.remote, 'listAgents');
  const author = vi.spyOn(f.admission, 'author');
  await f.ask.observeDestinations();
  expect(author).not.toHaveBeenCalled();
  context.mockClear();
  directory.mockClear();
  f.deny();
  await expect(f.ask.observeDestinations()).rejects.toThrow('Denied');
  expect(context).not.toHaveBeenCalled();
  expect(directory).not.toHaveBeenCalled();
  expect(f.sessionEnded).not.toHaveBeenCalled();
  expect(f.remote.sends).toEqual([]);
});
