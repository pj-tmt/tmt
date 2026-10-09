import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { openSession } from '../src/device.js';
import { management } from '../src/management.js';
import { ClientError, RefusalError, operations } from '../src/operations.js';
import { Door, paired } from './door.js';
const saved = {
  open: true,
  source: 'default',
  sessionsPerDevice: '18446744073709551615',
  sessionsPerDeviceSource: 'settings.json',
  warning: null,
};
async function ready() {
  const door = new Door();
  const device = await paired(door);
  const session = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  door.managementReply = (operation, value) =>
    operation === 'remote.settings.show'
      ? {
          settings: saved,
          capabilities: { settingsWrite: true, devicesWrite: true },
          readOnlyReason: null,
        }
      : {
          operationId: value.operationId,
          state: 'committed',
          result: { settings: saved },
          sessionEnded: false,
        };
  return { door, session, client: management(session), device };
}
test('decimal cap values stay exact and management shares the agent signing lane', async () => {
  const { door, session, client } = await ready();
  const change = {
    operationId: crypto.randomUUID(),
    setting: 'sessions-per-device' as const,
    value: '18446744073709551615',
  };
  const pending = client.set(change);
  change.value = '8';
  const [outcome, view] = await Promise.all([
    pending,
    client.settings(),
    operations(session).listAgents(),
  ]);
  assert.equal(outcome.state, 'committed');
  assert.equal(view.settings.sessionsPerDevice, '18446744073709551615');
  assert.equal(door.calls[0]!.payload.value, '18446744073709551615');
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '3'],
  );
  assert.equal(door.opens, 1);
});
test('malformed/default warning and persistent read-only capability are admitted data', async () => {
  const { door, client } = await ready();
  door.managementReply = () => ({
    settings: {
      ...saved,
      sessionsPerDevice: '8',
      sessionsPerDeviceSource: 'default',
      warning: 'settings.json could not be read; defaults apply',
    },
    capabilities: { settingsWrite: false, devicesWrite: false },
    readOnlyReason: 'local_cli_required',
  });
  const view = await client.settings();
  assert.equal(view.settings.warning, 'settings.json could not be read; defaults apply');
  assert.equal(view.capabilities.settingsWrite, false);
});
test('signed read-only refusal differs from a lost mutation response and creates no hold', async () => {
  const { door, client } = await ready();
  door.error = { code: 'REMOTE_MANAGEMENT_READ_ONLY', message: 'Read-only.' };
  const id = crypto.randomUUID();
  assert.deepEqual(await client.set({ operationId: id, setting: 'open', value: false }), {
    operationId: id,
    state: 'refused',
    reason: 'REMOTE_MANAGEMENT_READ_ONLY',
  });
  assert.equal(
    door.calls.filter((call) => call.envelope.operation === 'remote.settings.set').length,
    1,
  );
  door.error = undefined;
  door.managementReply = () => ({
    error: { code: 'REMOTE_MANAGEMENT_READ_ONLY', message: 'Read-only.' },
  });
  await assert.rejects(
    client.settings(),
    (error: unknown) =>
      error instanceof RefusalError && error.code === 'REMOTE_MANAGEMENT_READ_ONLY',
  );
});
test('post-publication 404 is unknown and keeps the original ID without a resend or reopen', async () => {
  const { door, client } = await ready();
  let published = 0;
  // Registered Session retains its transport; inject through the fixture response owner.
  door.response = async () => {
    published++;
    return new Response('{}', { status: 404 });
  };
  const id = crypto.randomUUID();
  await assert.rejects(
    client.revoke({ operationId: id, clientId: door.clientId }),
    (error: unknown) =>
      error instanceof ClientError &&
      error.operationId === id &&
      error.code === 'transport_failure',
  );
  assert.equal(published, 1);
  assert.equal(door.calls.length, 1);
  assert.equal(door.opens, 1);
});
test('unverified acknowledgment stays unknown; explicit original-ID lookup never resends', async () => {
  const { door, client } = await ready();
  const id = crypto.randomUUID();
  door.tamper.signature = true;
  await assert.rejects(
    client.set({ operationId: id, setting: 'open', value: false }),
    (error: unknown) =>
      error instanceof ClientError &&
      error.operationId === id &&
      error.code === 'unverifiable_response',
  );
  door.tamper.signature = false;
  door.managementReply = (operation, payload) => {
    assert.equal(operation, 'remote.management.operation');
    return {
      operationId: payload.operationId,
      state: 'unknown',
      reason: 'effect_outcome_unconfirmed',
    };
  };
  const outcome = await client.operation(id);
  assert.deepEqual(outcome, {
    operationId: id,
    state: 'unknown',
    reason: 'effect_outcome_unconfirmed',
  });
  assert.equal(
    door.calls.filter((call) => call.envelope.operation === 'remote.settings.set').length,
    1,
  );
  assert.equal(door.calls.at(-1)!.payload.operationId, id);
});
test('strict projection rejects rounded caps, secret fields and contradictory capability', async () => {
  for (const corrupt of [
    { settings: { ...saved, sessionsPerDevice: 9007199254740992 } },
    { settings: { ...saved, publicKey: 'secret' } },
    { capabilities: { settingsWrite: true, devicesWrite: false } },
  ]) {
    const { door, client } = await ready();
    door.managementReply = () => ({
      settings: saved,
      capabilities: { settingsWrite: true, devicesWrite: true },
      readOnlyReason: null,
      ...corrupt,
    });
    await assert.rejects(
      client.settings(),
      (error: unknown) => error instanceof ClientError && error.code === 'unverifiable_response',
    );
  }
});

test('signed phase-ambiguous failure preserves original outcome after a recorded effect', async () => {
  for (const code of ['REMOTE_STATE_UNAVAILABLE', 'REMOTE_MANAGEMENT_UNAVAILABLE']) {
    for (const state of ['committed', 'unknown'] as const) {
      const { door, client } = await ready();
      const id = crypto.randomUUID();
      let effects = 0;
      const receipt =
        state === 'committed'
          ? { operationId: id, state, result: { settings: saved }, sessionEnded: false }
          : { operationId: id, state, reason: 'effect_outcome_unconfirmed' };
      door.managementReply = (operation, payload) => {
        if (operation === 'remote.settings.set') {
          effects++;
          return { error: { code, message: 'Outcome publication failed.' } };
        }
        assert.equal(operation, 'remote.management.operation');
        assert.equal(payload.operationId, id);
        return receipt;
      };
      await assert.rejects(
        client.set({ operationId: id, setting: 'open', value: false }),
        (error: unknown) =>
          error instanceof ClientError &&
          error.operationId === id &&
          error.code === 'outcome_unconfirmed',
      );
      assert.deepEqual(await client.operation(id), receipt);
      assert.equal(effects, 1);
      assert.equal(
        door.calls.filter((call) => call.envelope.operation === 'remote.settings.set').length,
        1,
      );
      door.managementReply = () => ({ error: { code, message: 'Read unavailable.' } });
      await assert.rejects(
        client.settings(),
        (error: unknown) => error instanceof RefusalError && error.code === code,
      );
    }
  }
});

test('sending toggle checks the target and returned scope bit without changing other policy', async () => {
  const { door, client } = await ready();
  const id = crypto.randomUUID();
  door.managementReply = (operation, input) => {
    assert.equal(operation, 'remote.devices.talk');
    assert.deepEqual(input, { operationId: id, clientId: door.clientId, enabled: false });
    return {
      operationId: id,
      state: 'committed',
      result: {
        device: {
          clientId: door.clientId,
          name: 'Device',
          kind: 'browser',
          issuedAtMs: 1,
          expiresAtMs: null,
          revision: 2,
          revoked: false,
          talkEnabled: false,
        },
      },
      sessionEnded: true,
    };
  };
  const result = await client.talk({ operationId: id, clientId: door.clientId, enabled: false });
  assert.equal(result.state, 'committed');
  assert.equal(door.calls.length, 1);
});
