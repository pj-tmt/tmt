/** Test-only own-stream persistence double. No production Writer or sync proof. */
import { AskController } from '../src/ask-attempt.js';
import { AskRecordStore } from '../src/ask-record-store.js';
import type { AdmittedSelection, AskDestination } from '../src/ask-intent.js';
import type { RemoteClient } from '../src/ask-remote.js';
import type { PreviewAttempt } from '../src/ask-preview.js';
import type { OwnState } from '../src/fold-protocol.js';
import { deviceKeys } from '../src/keyring.js';
import { record } from '../src/storage.js';
import { RemoteDouble } from './ask-fixtures.js';

export async function fixtureAttempt(
  input: AdmittedSelection,
  destination: AskDestination,
  options: {
    unavailable?: boolean;
    mode?: RemoteDouble['mode'];
    operationId?: string;
    issuedAt?: number;
  } = {},
) {
  const captured = structuredClone(input);
  const target = structuredClone(destination);
  const remote = new RemoteDouble();
  remote.mode = options.mode ?? 'accepted';
  const port: RemoteClient = {
    context: async () => ({
      machineId: target.machine,
      deviceId: captured.senderDevice,
      grantRevision: target.grantRevision,
      deviceName: target.deviceName,
      expiresAtMs: target.grantExpiresAt,
      mode: target.mode,
    }),
    listAgents: async () => [{ id: target.agent, name: target.agentName }],
    send: (input) => remote.send(input),
    operation: (id) => remote.operation(id),
    result: (id) => remote.result(id),
  };
  const keys = await deviceKeys(captured.senderDevice);
  const storageKey = `test:ask-own:${captured.space}:${captured.page}:${captured.senderDevice}`;
  let own = (await record<OwnState>(storageKey)) ?? {};
  const store = new AskRecordStore({
    space: captured.space,
    page: captured.page,
    deviceId: captured.senderDevice,
    publicKey: keys.signPublic,
    readOwn: () => own,
    async publish(root, key, value) {
      const latest = (await record<OwnState>(storageKey)) ?? {};
      const writer = (latest[captured.senderDevice] ??= {
        threads: {},
        messages: {},
        intents: {},
        replies: {},
      });
      if (
        writer[root][key] !== undefined &&
        JSON.stringify(writer[root][key]) !== JSON.stringify(value)
      )
        throw new Error('Fixture own conflict');
      writer[root][key] = structuredClone(value);
      await record(storageKey, latest);
      own = latest;
    },
  });
  const controller = new AskController({
    store,
    remote: port,
    key: keys.sign,
    selection: () => captured,
  });
  await controller.destinations();
  const preview = controller.prepare(target, {
    operationId: options.operationId,
    issuedAt: options.issuedAt,
  });
  let state: PreviewAttempt['state'] = { state: 'preview' };
  let pending: ReturnType<PreviewAttempt['send']> | undefined;
  const attempt: PreviewAttempt = {
    preview,
    available: !options.unavailable,
    get state() {
      return state;
    },
    send() {
      if (pending) return pending;
      state = { state: 'preparing' };
      pending = controller.send(preview).then((view) => {
        state = { state: view.state };
        return state;
      });
      return pending;
    },
  };
  return { attempt, remote, keys };
}
