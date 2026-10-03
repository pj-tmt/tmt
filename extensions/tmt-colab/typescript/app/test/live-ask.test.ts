import { afterEach, expect, it, vi } from 'vite-plus/test';
import type { Connection } from '../src/connection.js';
import type { Admission } from '../src/admission.js';
import type { OwnState } from '../src/fold-protocol.js';
import { LiveAsk, pageAsks } from '../src/live-ask.js';
import { destination, id, RemoteDouble } from './ask-fixtures.js';
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
async function fixture() {
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
    url: 'https://example.test/page#private',
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
  expect(attempt.preview.view.message).not.toContain('#private');
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
