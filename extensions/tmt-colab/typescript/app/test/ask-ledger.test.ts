import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import { binary } from '@tmt/colab-client';
import { AskController } from '../src/ask-attempt.js';
import { AskRecordStore } from '../src/ask-record-store.js';
import { ASK_OBSERVATION_MS, readAskViews, type AskLedgerView } from '../src/ask-records.js';
import { FrozenAsk } from '../src/ask-intent.js';
import {
  createRemoteClient,
  SessionEndedError,
  SessionEvictedError,
  ReadRefusedError,
} from '../src/ask-remote.js';
import type { OwnState } from '../src/fold-protocol.js';
import { destination, id, RemoteDouble, selection } from './ask-fixtures.js';
const draft = new Map<string, unknown>();
vi.mock('../src/storage.js', () => ({
  record: async (key: string, ...values: unknown[]) => {
    if (values.length) draft.set(key, structuredClone(values[0]));
    else return draft.get(key);
  },
}));
const locks = new Map<string, Promise<unknown>>();
vi.stubGlobal('navigator', {
  locks: {
    request: async (key: string, action: () => Promise<unknown>) => {
      const task = (locks.get(key) ?? Promise.resolve()).then(action);
      locks.set(
        key,
        task.catch(() => {}),
      );
      return task;
    },
  },
});
const vector = JSON.parse(
  readFileSync(new URL('../../../contracts/vectors/send-preview-v1.json', import.meta.url), 'utf8'),
);
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (v) => parseInt(v, 16));
async function setup(onPublish?: (root: string) => void) {
  draft.clear();
  const key = await crypto.subtle.importKey(
    'pkcs8',
    hex('302e020100300506032b657004220420' + vector.seed),
    'Ed25519',
    false,
    ['sign'],
  );
  const own: OwnState = { [id(4)]: { intents: {}, messages: {}, replies: {}, threads: {} } };
  const store = new AskRecordStore({
    space: selection().space,
    page: id(1),
    deviceId: id(4),
    publicKey: hex(vector.publicKey),
    readOwn: () => own,
    publish: async (root, key, value) => {
      onPublish?.(root);
      own[id(4)][root][key] = structuredClone(value);
    },
  });
  const remote = new RemoteDouble();
  const controller = new AskController({ store, remote, key, selection });
  return { key, own, store, remote, controller };
}
it('publishes immutable intent and dispatching before one effect, then reads an empty final without resend', async () => {
  const { controller, remote, store, own, key } = await setup();
  await controller.destinations();
  const preview = controller.prepare(destination());
  const send = remote.send.bind(remote);
  remote.send = async (input) => {
    expect((await store.view(input.operationId)).state).toBe('dispatching');
    expect(own[id(4)].intents[input.operationId]).toBeDefined();
    return send(input);
  };
  const one = controller.send(preview),
    two = controller.send(preview);
  expect(one).toBe(two);
  const accepted = await one;
  expect(accepted.state).toBe('accepted');
  expect(remote.sends).toHaveLength(1);
  expect(remote.sends[0].message).toBe(preview.view.message);
  expect(preview.view.deliveredMessage).toBe(`[remote: Fixture browser]\n${preview.view.message}`);
  const replied = await controller.recover(preview.view.operationId);
  expect(replied.reply?.body).toBe('');
  expect(replied.reply?.agentId).toBe(id(6));
  expect(remote.sends).toHaveLength(1);
  const reloaded = new AskController({ store, remote, key, selection });
  expect((await reloaded.recover(preview.view.operationId)).reply?.body).toBe('');
  expect(remote.sends).toHaveLength(1);
});
it('same-operation concurrent controllers cannot dispatch twice and conflicts preserve original intent', async () => {
  const { controller, remote, store, key } = await setup();
  const second = new AskController({ store, remote, key, selection });
  await controller.destinations();
  await second.destinations();
  const frozen = controller.prepare(destination());
  const results = await Promise.all([controller.send(frozen), second.send(frozen)]);
  expect(results.map((r) => r.state)).toEqual(['accepted', 'accepted']);
  expect(remote.sends).toHaveLength(1);
  const changed = FrozenAsk.capture({ ...selection(), comment: 'changed' }, destination(), {
    operationId: frozen.view.operationId,
  });
  await expect(second.send(changed)).rejects.toThrow('INTENT_CONFLICT');
  const third = new AskController({ store, remote, key, selection });
  await expect(third.send(changed)).rejects.toThrow('INTENT_CONFLICT');
  expect((await store.view(frozen.view.operationId)).intent.message).toBe(frozen.view.message);
  expect(remote.sends).toHaveLength(1);
});
it('a lost effect response stays uncertain through absent recovery and abandon never cancels or retries', async () => {
  const { controller, remote } = await setup();
  remote.mode = 'throw';
  await controller.destinations();
  const frozen = controller.prepare(destination());
  expect((await controller.send(frozen)).state).toBe('uncertain');
  expect((await controller.recover(frozen.view.operationId)).state).toBe('uncertain');
  expect((await controller.abandon(frozen.view.operationId)).reason).toBe(
    'MAY_HAVE_BEEN_DELIVERED',
  );
  expect((await controller.recover(frozen.view.operationId)).state).toBe('abandoned');
  expect(remote.sends).toHaveLength(1);
});
it('a rename or changed grant revision before publication refuses without sending', async () => {
  const { controller, remote, store } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  const context = await remote.context();
  remote.context = async () => ({ ...context, deviceName: 'Renamed', grantRevision: '2' });
  await expect(controller.send(frozen)).rejects.toThrow();
  expect(remote.sends).toHaveLength(0);
  expect(await store.views()).toHaveLength(0);
});
it('all-stream views verify possession and same-stream agent/request attribution', async () => {
  const { controller, remote, own } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  const view = await controller.send(frozen);
  await controller.recover(view.intent.operationId);
  const signer = (writer: string) => (writer === id(4) ? hex(vector.publicKey) : undefined);
  expect(
    (await readAskViews(own, { space: selection().space, page: id(1) }, signer))[0].reply?.body,
  ).toBe('');
  const wrong = structuredClone(own);
  (wrong[id(4)].replies[view.intent.operationId] as unknown as AskLedgerView['reply'])!.agentId =
    id(99);
  expect(await readAskViews(wrong, { space: selection().space, page: id(1) }, signer)).toHaveLength(
    0,
  );
  const other = { [id(99)]: own[id(4)] };
  expect(
    await readAskViews(other, { space: selection().space, page: id(1) }, () =>
      hex(vector.publicKey),
    ),
  ).toHaveLength(0);
  const changed = structuredClone(own);
  const ask = changed[id(4)].intents[view.intent.operationId] as unknown as {
    signed: { finalBytes: string };
  };
  ask.signed.finalBytes = btoa('changed');
  expect(
    await readAskViews(changed, { space: selection().space, page: id(1) }, signer),
  ).toHaveLength(0);
  expect(remote.sends).toHaveLength(1);
});
it('SDK adapter keeps the shared session and observes the original ID and ignores unknown delivery values', async () => {
  let wrappers = 0;
  const sends: unknown[] = [],
    observations: string[] = [];
  const session = {
    sessionId: id(7),
    serverTimeMs: Date.now(),
    grantRevision: 1,
    expiresAtMs: null,
  };
  const adapter = await createRemoteClient(
    new URL('http://example.test/x/colab/'),
    {
      operations: (shared) => {
        expect(shared).toBe(session);
        wrappers++;
        return {
          listAgents: async () => [
            { id: id(6), name: 'Agent', presence: 'active', delivery: { future: 'unknown' } },
          ],
          send: async (input) => {
            sends.push(input);
            throw new Error('timeout');
          },
          operation: async (operationId) => {
            observations.push(operationId);
            return { state: 'uncertain', operationId };
          },
          result: async (requestId) => ({ state: 'replied', requestId, message: '' }),
        };
      },
    },
    session,
    async (url) =>
      new Response(
        JSON.stringify(
          String(url).endsWith('/api/session')
            ? {
                deviceId: id(4),
                publicKey: vector.publicKey,
                name: 'Fixture browser',
                grantRevision: '1',
              }
            : {
                machineId: id(5),
                windowId: id(7),
                address: 'http://example.test',
                extension: 'colab',
                mount: '/x/colab/',
              },
        ),
      ),
  );
  expect(await adapter.listAgents()).toEqual([{ id: id(6), name: 'Agent', presence: 'active' }]);
  expect((await adapter.context()).deviceName).toBe('Fixture browser');
  expect((await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' })).state).toBe(
    'uncertain',
  );
  expect((await adapter.operation(id(9))).state).toBe('uncertain');
  expect(sends).toHaveLength(1);
  expect(observations).toEqual([id(9)]);
  expect(wrappers).toBe(1);
  expect(await adapter.result(`req_${id(8)}`)).toEqual({
    state: 'replied',
    requestId: `req_${id(8)}`,
    message: '',
  });
  expect(binary(vector.finalBytes, 65536).length).toBeGreaterThan(0);
});

it('visible bounded observation publishes finals through reads only and abort stops further tracking', async () => {
  const { controller, remote } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const stop = new AbortController();
  await controller.observe(stop.signal);
  expect((await controller.recover(frozen.view.operationId)).reply?.body).toBe('');
  expect(remote.sends).toHaveLength(1);
  stop.abort();
  await controller.observe(stop.signal);
  expect(remote.sends).toHaveLength(1);
});

it('activation reads an old annotation once while ordinary polling keeps its operation horizon', async () => {
  const { controller, remote, store, key } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const before = await store.view(frozen.view.operationId);
  const result = vi.spyOn(remote, 'result');
  const clock = vi.spyOn(Date, 'now').mockReturnValue(before.intent.issuedAt + ASK_OBSERVATION_MS);
  try {
    const reloaded = new AskController({ store, remote, key, selection });
    await reloaded.observe(new AbortController().signal);
    expect(result).not.toHaveBeenCalled();
    expect(await store.view(frozen.view.operationId)).toEqual(before);
    await reloaded.observe(new AbortController().signal, true);
    const recovered = await store.view(frozen.view.operationId);
    expect(recovered.reply).toMatchObject({
      requestId: before.requestId,
      agentId: id(6),
      body: '',
    });
    expect(result).toHaveBeenCalledExactlyOnceWith(before.requestId);
    await reloaded.observe(new AbortController().signal, true);
    expect(result).toHaveBeenCalledOnce();
    expect(remote.sends).toHaveLength(1);
  } finally {
    clock.mockRestore();
    result.mockRestore();
  }
});

it('activation makes at most 256 sequential old-ask reads and does not retry failed older reads', async () => {
  const { controller, remote, store } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const view = await store.view(frozen.view.operationId);
  const clock = vi
    .spyOn(Date, 'now')
    .mockReturnValue(view.intent.issuedAt + ASK_OBSERVATION_MS + 1000);
  const views = Array.from({ length: 257 }, (_, index) => ({
    ...view,
    intent: {
      ...view.intent,
      operationId: id(index + 100),
      issuedAt: view.intent.issuedAt - index,
    },
  }));
  const records = vi.spyOn(store, 'views').mockResolvedValue([...views].reverse());
  let active = 0,
    peak = 0;
  const recover = vi.spyOn(controller, 'recover').mockImplementation(async () => {
    peak = Math.max(peak, ++active);
    await Promise.resolve();
    active--;
    throw new ReadRefusedError('REMOTE_STATE_UNAVAILABLE');
  });
  try {
    await controller.observe(new AbortController().signal, true);
    expect(recover.mock.calls.map(([operationId]) => operationId)).toEqual(
      views.slice(0, 256).map((value) => value.intent.operationId),
    );
    expect(peak).toBe(1);
    await controller.observe(new AbortController().signal);
    expect(recover).toHaveBeenCalledTimes(256);
    expect(remote.sends).toHaveLength(1);
  } finally {
    recover.mockRestore();
    records.mockRestore();
    clock.mockRestore();
  }
});

it('preserves verified scope refusal and never dispatches again during recovery', async () => {
  const { controller, remote } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  remote.send = async (input) => {
    remote.sends.push(input);
    return { state: 'refused', operationId: input.operationId, reason: 'REMOTE_SCOPE_DENIED' };
  };
  expect((await controller.send(frozen)).reason).toBe('REMOTE_SCOPE_DENIED');
  expect((await controller.recover(frozen.view.operationId)).state).toBe('refused');
  expect(remote.sends).toHaveLength(1);
  expect(remote.reads).toEqual([]);
});

it('publishes uncertainty before requesting Session replacement and never reuses its preview', async () => {
  const { store, key, remote, own } = await setup();
  const notifications: string[] = [];
  const controller = new AskController({
    store,
    key,
    remote,
    selection,
    sessionEnded: () => {
      const states = Object.values(own[id(4)].messages) as unknown as { state: string }[];
      notifications.push(states.at(-1)!.state);
    },
  });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  remote.send = async (input) => {
    remote.sends.push(input);
    return { state: 'uncertain', operationId: input.operationId, reason: 'REMOTE_SESSION_ENDED' };
  };
  expect((await controller.send(frozen)).reason).toBe('REMOTE_SESSION_ENDED');
  expect(notifications).toEqual(['uncertain']);
  await controller.observe(new AbortController().signal);
  expect(remote.reads).toEqual([]);
  expect(() => controller.prepare(destination())).toThrow();
  expect(remote.sends).toHaveLength(1);
});

it('a session-ending final read stops observation, preserves its accepted record, then a new controller recovers read-only', async () => {
  const { store, remote, key, own } = await setup();
  const ended = vi.fn();
  const controller = new AskController({ store, remote, key, selection, sessionEnded: ended });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const before = structuredClone(own);
  const read = vi.fn(async () => {
    throw new SessionEndedError('REMOTE_SESSION_ENDED');
  });
  remote.result = read;
  await controller.observe(new AbortController().signal);
  await controller.observe(new AbortController().signal);
  expect(read).toHaveBeenCalledOnce();
  expect(ended).toHaveBeenCalledOnce();
  expect(own).toEqual(before);
  const next = new RemoteDouble();
  const reconnected = new AskController({ store, remote: next, key, selection });
  expect((await reconnected.recover(frozen.view.operationId)).reply?.body).toBe('');
  expect(next.sends).toEqual([]);
});

it('an evicted Ask read passes the verified limit to the page without dispatching again', async () => {
  const { store, remote, key } = await setup();
  const ended = vi.fn();
  const controller = new AskController({ store, remote, key, selection, sessionEnded: ended });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const eviction = new SessionEvictedError(8, 'https://example.test/remote/settings');
  remote.result = vi.fn(async () => {
    throw eviction;
  });
  await expect(controller.recover(frozen.view.operationId)).rejects.toBe(eviction);
  expect(ended).toHaveBeenCalledExactlyOnceWith(eviction);
  await controller.observe(new AbortController().signal);
  expect(remote.sends).toHaveLength(1);
});

it('an evicted send records uncertainty before stopping the page and never resends', async () => {
  const { store, remote, key, own } = await setup();
  const eviction = new SessionEvictedError(2);
  const observed = vi.fn(() => {
    const states = Object.values(own[id(4)].messages) as unknown as {
      state: string;
      reason: string;
    }[];
    expect(states.at(-1)?.state).toBe('uncertain');
    expect(states.at(-1)?.reason).toBe('REMOTE_SESSION_EVICTED');
  });
  const controller = new AskController({ store, remote, key, selection, sessionEnded: observed });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  remote.send = async (input) => {
    remote.sends.push(input);
    throw eviction;
  };
  expect((await controller.send(frozen)).state).toBe('uncertain');
  expect(observed).toHaveBeenCalledExactlyOnceWith(eviction);
  expect(remote.sends).toHaveLength(1);
  await controller.observe(new AbortController().signal);
  expect(remote.sends).toHaveLength(1);
});

it('SDK class and code identify session faults; error-shaped objects never cause replacement', async () => {
  class ClientError extends Error {
    constructor(readonly code: string) {
      super(code);
    }
  }
  class RefusalError extends Error {
    constructor(
      readonly code: string,
      readonly limit?: number,
      readonly settingsUrl?: string,
    ) {
      super(code);
    }
  }
  let fault: unknown;
  let refusal = false;
  const open = () =>
    createRemoteClient(
      new URL('http://example.test/x/colab/'),
      {
        ClientError,
        RefusalError,
        operations: () => ({
          listAgents: async () => {
            throw fault;
          },
          send: async ({ operationId }) => {
            if (refusal) return { state: 'refused', operationId, reason: 'REMOTE_SESSION_ENDED' };
            throw fault;
          },
          operation: async () => {
            throw fault;
          },
          result: async () => {
            throw fault;
          },
        }),
      },
      { sessionId: id(7), serverTimeMs: Date.now(), grantRevision: 1, expiresAtMs: null },
    );
  let adapter = await open();
  for (const [error, reason] of [
    [new ClientError('sequence_unavailable'), 'REMOTE_SEQUENCE_UNAVAILABLE'],
    [new RefusalError('REMOTE_SESSION_ENDED'), 'REMOTE_SESSION_ENDED'],
  ] as const) {
    fault = error;
    expect(await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' })).toEqual({
      state: 'uncertain',
      operationId: id(9),
      reason,
    });
    await expect(adapter.result(`req_${id(8)}`)).rejects.toBeInstanceOf(SessionEndedError);
    await expect(adapter.listAgents()).rejects.toBeInstanceOf(SessionEndedError);
  }
  for (const code of [
    'REMOTE_INPUT_TOO_LARGE',
    'REMOTE_STATE_UNAVAILABLE',
    'REMOTE_CORE_UNAVAILABLE',
  ]) {
    fault = new RefusalError(code);
    expect(
      (await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' })).state,
    ).toBe('uncertain');
    await expect(adapter.result(`req_${id(8)}`)).rejects.toBeInstanceOf(ReadRefusedError);
  }
  fault = new RefusalError('REMOTE_SESSION_EVICTED', 8, 'https://example.test/remote/settings');
  await expect(adapter.listAgents()).rejects.toMatchObject({
    limit: 8,
    settingsUrl: 'https://example.test/remote/settings',
  });
  await expect(adapter.result(`req_${id(8)}`)).rejects.toBeInstanceOf(SessionEvictedError);
  await expect(
    adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' }),
  ).rejects.toBeInstanceOf(SessionEvictedError);
  // An evicted adapter keeps its verified terminal cause. Error-shaped negative
  // controls use a distinct Session/client, rather than reviving that channel.
  adapter = await open();
  fault = { code: 'sequence_unavailable' };
  expect((await adapter.operation(id(9))).state).toBe('uncertain');
  expect(((await adapter.operation(id(9))) as { reason?: string }).reason).toBeUndefined();
  await expect(adapter.result(`req_${id(8)}`)).rejects.toBe(fault);
  refusal = true;
  expect(await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' })).toEqual({
    state: 'refused',
    operationId: id(9),
    reason: 'REMOTE_SESSION_ENDED',
  });
});

it('failed own publication reserves the operation and a later adoption cannot dispatch', async () => {
  let fail = true;
  const { controller, remote, store, key } = await setup(() => {
    if (fail) throw new Error('disk full');
  });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await expect(controller.send(frozen)).rejects.toThrow('disk full');
  expect(remote.sends).toEqual([]);
  fail = false;
  const reloaded = new AskController({ store, remote, key, selection });
  expect((await reloaded.send(frozen)).state).toBe('uncertain');
  expect(remote.sends).toEqual([]);
});

it('expiry during durable publication refuses before the Remote effect', async () => {
  let expiry = 0;
  const { controller, remote } = await setup((root) => {
    if (root === 'messages') vi.spyOn(Date, 'now').mockReturnValue(expiry);
  });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  expiry = frozen.expiresAt;
  try {
    expect((await controller.send(frozen)).state).toBe('expired');
  } finally {
    vi.restoreAllMocks();
  }
  expect(remote.sends).toEqual([]);
});

it('mis-correlated effects stay uncertain; held reads do not grow the ledger or dispatch again', async () => {
  for (const mode of ['wrong_id', 'held'] as const) {
    const { controller, remote, own } = await setup();
    remote.mode = mode;
    await controller.destinations();
    const frozen = controller.prepare(destination());
    expect((await controller.send(frozen)).state).toBe(mode === 'held' ? 'held' : 'uncertain');
    if (mode === 'held') {
      remote.operation = async (operationId) => ({ state: 'held', operationId });
      const before = structuredClone(own);
      await controller.recover(frozen.view.operationId);
      await controller.recover(frozen.view.operationId);
      expect(own).toEqual(before);
    }
    expect(remote.sends).toHaveLength(1);
  }
});

it('interrupted sends become uncertain, then refused reads preserve that record and later reads can recover', async () => {
  for (const state of ['dispatching', 'uncertain'] as const) {
    const { store, remote, key, own } = await setup();
    const frozen = FrozenAsk.capture(selection(), destination());
    await store.adopt(await frozen.signed(key), frozen.view);
    await store.state(frozen.view.operationId, state);
    const controller = new AskController({ store, remote, key, selection });
    const before = structuredClone(own);
    remote.operation = async (operationId) => ({
      state: 'refused',
      operationId,
      reason: 'REMOTE_STATE_UNAVAILABLE',
    });
    await expect(controller.recover(frozen.view.operationId)).rejects.toBeInstanceOf(
      ReadRefusedError,
    );
    if (state === 'uncertain') expect(own).toEqual(before);
    else {
      expect((await store.view(frozen.view.operationId)).state).toBe('uncertain');
      expect((await store.view(frozen.view.operationId)).reason).toBe('OBSERVATION_INTERRUPTED');
    }
    const interrupted = structuredClone(own);
    await expect(controller.recover(frozen.view.operationId)).rejects.toBeInstanceOf(
      ReadRefusedError,
    );
    expect(own).toEqual(interrupted);
    remote.operation = async (operationId) => ({
      state: 'accepted',
      operationId,
      requestId: `req_${id(8)}`,
    });
    expect((await controller.recover(frozen.view.operationId)).reply?.body).toBe('');
    expect(remote.sends).toEqual([]);
  }
});

it('transient result refusals preserve accepted state and allow the next read to publish a final', async () => {
  const { controller, remote, own } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const before = structuredClone(own);
  remote.result = async () => {
    throw new ReadRefusedError('REMOTE_CORE_UNAVAILABLE');
  };
  await expect(controller.recover(frozen.view.operationId)).rejects.toBeInstanceOf(
    ReadRefusedError,
  );
  expect(own).toEqual(before);
  remote.result = async (requestId) => ({ state: 'replied', requestId, message: '' });
  expect((await controller.recover(frozen.view.operationId)).reply?.body).toBe('');
  expect(remote.sends).toHaveLength(1);
});

it('a verified pre-admission Session refusal stays refused before reconnect notification and cannot resend', async () => {
  const { store, remote, key } = await setup();
  const notified = vi.fn();
  const controller = new AskController({ store, remote, key, selection, sessionEnded: notified });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  remote.send = async (input) => {
    remote.sends.push(input);
    return { state: 'refused', operationId: input.operationId, reason: 'REMOTE_SESSION_ENDED' };
  };
  const refused = await controller.send(frozen);
  expect(refused.state).toBe('refused');
  expect(refused.reason).toBe('REMOTE_SESSION_ENDED');
  expect(notified).toHaveBeenCalledOnce();
  expect((await store.view(frozen.view.operationId)).state).toBe('refused');
  await controller.send(frozen);
  const next = new AskController({ store, remote, key, selection });
  expect((await next.recover(frozen.view.operationId)).state).toBe('refused');
  expect(remote.sends).toHaveLength(1);
});

it('display labels remain bounded publisher claims outside the signed input and routing', async () => {
  const { controller, own } = await setup();
  await controller.destinations();
  const frozen = controller.prepare(destination());
  const sent = await controller.send(frozen);
  expect(sent.agentName).toBe('Deterministic agent');
  expect(sent.deviceName).toBe('Fixture browser');
  const changed = structuredClone(own);
  const intent = changed[id(4)].intents[sent.intent.operationId] as {
    agentName: string;
    deviceName: string;
  };
  intent.agentName = '😀'.repeat(32);
  intent.deviceName = 'Another label';
  const views = await readAskViews(changed, { space: selection().space, page: id(1) }, () =>
    hex(vector.publicKey),
  );
  expect(views[0].signed).toEqual(sent.signed);
  expect(views[0].intent.agent).toBe(sent.intent.agent);
  expect(views[0].agentName).toBe(intent.agentName);
  intent.agentName += 'x';
  expect(
    await readAskViews(changed, { space: selection().space, page: id(1) }, () =>
      hex(vector.publicKey),
    ),
  ).toEqual([]);
  expect(() =>
    FrozenAsk.capture(selection(), { ...destination(), deviceName: '😀'.repeat(33) }),
  ).toThrow();
});

it.each(['send', 'operation'] as const)(
  'SDK returned eviction from %s retains metadata before later ended-channel reads',
  async (method) => {
    class RefusalError extends Error {
      constructor(readonly code: string) {
        super(code);
      }
    }
    const sdk = {
      RefusalError,
      operations: () => ({
        listAgents: vi.fn(async () => {
          throw new RefusalError('REMOTE_SESSION_ENDED');
        }),
        send: vi.fn(async ({ operationId }: { operationId: string }) => ({
          state: 'refused' as const,
          operationId,
          reason: 'REMOTE_SESSION_EVICTED',
          limit: 8,
          settingsUrl: 'https://example.test/remote/settings',
        })),
        operation: vi.fn(async (operationId: string) => ({
          state: 'refused' as const,
          operationId,
          reason: 'REMOTE_SESSION_EVICTED',
          limit: 8,
          settingsUrl: 'https://example.test/remote/settings',
        })),
        result: vi.fn(async () => ({ state: 'pending' as const })),
      }),
    };
    const adapter = await createRemoteClient(new URL('https://example.test/x/colab/'), sdk, {
      sessionId: id(7),
      serverTimeMs: Date.now(),
      grantRevision: 1,
      expiresAtMs: null,
    });
    const value =
      method === 'send'
        ? await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' })
        : await adapter.operation(id(9));
    expect(value).toEqual({
      state: 'refused',
      operationId: id(9),
      reason: 'REMOTE_SESSION_EVICTED',
      limit: 8,
      settingsUrl: 'https://example.test/remote/settings',
    });
    await expect(adapter.listAgents()).rejects.toMatchObject({
      limit: 8,
      settingsUrl: 'https://example.test/remote/settings',
    });
  },
);

it('a returned pre-effect send eviction persists refused before notifying the page and never resends', async () => {
  const { store, remote, key } = await setup();
  const ended = vi.fn();
  const controller = new AskController({ store, remote, key, selection, sessionEnded: ended });
  await controller.destinations();
  const frozen = controller.prepare(destination());
  remote.send = async (input) => {
    remote.sends.push(input);
    return {
      state: 'refused',
      operationId: input.operationId,
      reason: 'REMOTE_SESSION_EVICTED',
      limit: 2,
      settingsUrl: 'https://example.test/remote/settings',
    };
  };
  const result = await controller.send(frozen);
  expect(result).toMatchObject({ state: 'refused', reason: 'REMOTE_SESSION_EVICTED' });
  expect((await store.view(frozen.view.operationId)).state).toBe('refused');
  expect(ended).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({
      limit: 2,
      settingsUrl: 'https://example.test/remote/settings',
    }),
  );
  await controller.observe(new AbortController().signal);
  expect(() => controller.prepare(destination())).toThrow();
  expect(remote.sends).toHaveLength(1);
  expect(remote.reads).toEqual([]);
});

it('a returned operation-read eviction preserves the held ledger and stops reads without another dispatch', async () => {
  const { store, remote, key, own } = await setup();
  const ended = vi.fn();
  const controller = new AskController({ store, remote, key, selection, sessionEnded: ended });
  remote.mode = 'held';
  await controller.destinations();
  const frozen = controller.prepare(destination());
  await controller.send(frozen);
  const before = structuredClone(own);
  remote.operation = async (operationId) => {
    remote.reads.push(operationId);
    return { state: 'refused', operationId, reason: 'REMOTE_SESSION_EVICTED', limit: 8 };
  };
  await expect(controller.recover(frozen.view.operationId)).rejects.toMatchObject({ limit: 8 });
  expect(ended).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ limit: 8 }));
  expect(own).toEqual(before);
  await controller.observe(new AbortController().signal);
  expect(remote.reads).toEqual([frozen.view.operationId]);
  expect(remote.sends).toHaveLength(1);
});

it.each(['send', 'operation'] as const)(
  'an already pending ENDED probe cannot overwrite the returned %s eviction',
  async (method) => {
    class RefusalError extends Error {
      constructor(readonly code: string) {
        super(code);
      }
    }
    let finish!: (error: Error) => void;
    const started = vi.fn();
    const ops = {
      listAgents: () =>
        new Promise<never>((_, reject) => {
          finish = reject;
          started();
        }),
      send: async ({ operationId }: { operationId: string }) => ({
        state: 'refused' as const,
        operationId,
        reason: 'REMOTE_SESSION_EVICTED',
        limit: 2,
      }),
      operation: async (operationId: string) => ({
        state: 'refused' as const,
        operationId,
        reason: 'REMOTE_SESSION_EVICTED',
        limit: 2,
      }),
      result: async () => ({ state: 'pending' as const }),
    };
    const adapter = await createRemoteClient(
      new URL('https://example.test/colab/'),
      { RefusalError, operations: () => ops },
      { sessionId: id(7), serverTimeMs: Date.now(), grantRevision: 1, expiresAtMs: null },
    );
    const probe = adapter.listAgents();
    const observed = expect(probe).rejects.toMatchObject({ limit: 2 });
    expect(started).toHaveBeenCalledOnce();
    if (method === 'send')
      await adapter.send({ operationId: id(9), agentId: id(6), message: 'exact' });
    else await adapter.operation(id(9));
    finish(new RefusalError('REMOTE_SESSION_ENDED'));
    await observed;
  },
);
