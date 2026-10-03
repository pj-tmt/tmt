import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import { binary } from '@tmt/colab-client';
import { AskController } from '../src/ask-attempt.js';
import { AskRecordStore } from '../src/ask-record-store.js';
import { readAskViews, type AskLedgerView } from '../src/ask-records.js';
import { FrozenAsk } from '../src/ask-intent.js';
import { createRemoteClient } from '../src/ask-remote.js';
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
async function setup() {
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
