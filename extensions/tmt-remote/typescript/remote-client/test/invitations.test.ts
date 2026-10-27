import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { openSession } from '../src/device.js';
import { invitations, parsePendingInvitation, parseInvitationOutcome } from '../src/invitations.js';
import { base64url } from '../src/canonical-bytes.js';
import { ClientError, operations } from '../src/operations.js';
import { Door, paired } from './door.js';

const candidate = {
  clientId: crypto.randomUUID(),
  name: 'Guest',
  words: ['apple', 'birch', 'cedar', 'dawn'],
  snapshot: base64url(new Uint8Array(32).fill(7)),
};
const pending = {
  state: 'current',
  offerId: crypto.randomUUID(),
  expiresAtMs: 2000,
  phase: 'awaiting-owner',
  localConfirmationKnown: false,
  cleanup: 'not-requested',
  candidate,
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
  const client = invitations(session);
  door.managementReply = (operation, value) =>
    operation === 'remote.invites.pending'
      ? pending
      : {
          operationId: value.operationId,
          state: 'committed',
          sessionEnded: false,
          result: {
            invitation: value.matched
              ? {
                  offerId: value.offerId,
                  clientId: candidate.clientId,
                  state: 'confirmed',
                  memberId: crypto.randomUUID(),
                  sendingEnabled: value.sending,
                }
              : { offerId: value.offerId, clientId: candidate.clientId, state: 'refused' },
          },
        };
  return { door, session, client, device };
}
const input = () => ({
  operationId: crypto.randomUUID(),
  offerId: pending.offerId,
  snapshot: candidate.snapshot,
  matched: true,
});
test('invitation decisions freeze the snapshot and default sending off on the shared signing lane', async () => {
  const { client, session, door } = await ready();
  assert.deepEqual(await client.pending(), pending);
  const change = input();
  const decision = client.decide(change);
  change.snapshot = base64url(new Uint8Array(32).fill(8));
  const [outcome] = await Promise.all([decision, operations(session).listAgents()]);
  assert.equal(outcome.state, 'committed');
  assert.equal(door.calls[1]!.payload.snapshot, candidate.snapshot);
  assert.equal(door.calls[1]!.payload.sending, false);
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '3'],
  );
  assert.equal(door.opens, 1);
});
test('explicit default-no sending and rejection remain separate decisions', async () => {
  for (const sending of [false, true]) {
    const { client } = await ready();
    const outcome = await client.decide({ ...input(), sending });
    assert.equal(outcome.state, 'committed');
    assert.ok(outcome.state === 'committed' && outcome.result.invitation.state === 'confirmed');
    assert.equal(outcome.result.invitation.sendingEnabled, sending);
  }
  const { client, door } = await ready();
  const outcome = await client.decide({ ...input(), matched: false });
  assert.equal(outcome.state === 'committed' && outcome.result.invitation.state, 'refused');
  assert.equal(door.calls[0]!.payload.sending, false);
  assert.throws(() => client.decide({ ...input(), matched: false, sending: true }), TypeError);
  assert.throws(() => client.decide({ ...input(), candidate: 'unbound' } as never), TypeError);
});
test('signed owner refusal differs from lost answer; only original-ID lookup follows uncertainty', async () => {
  const refused = await ready();
  refused.door.error = {
    code: 'REMOTE_MANAGEMENT_READ_ONLY',
    message: 'Designated owner required.',
  };
  const change = input();
  assert.deepEqual(await refused.client.decide(change), {
    operationId: change.operationId,
    state: 'refused',
    reason: 'REMOTE_MANAGEMENT_READ_ONLY',
  });
  const { client, door, device } = await ready();
  const original = {
    operationId: change.operationId,
    state: 'unknown',
    reason: 'effect_outcome_unconfirmed',
  };
  const response = door.response;
  door.response = async () => new Response('{}', { status: 404 });
  await assert.rejects(
    client.decide(change),
    (error) =>
      error instanceof ClientError &&
      error.operationId === change.operationId &&
      error.code === 'transport_failure',
  );
  door.response = response;
  door.managementReply = () => original;
  const reopened = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  assert.deepEqual(await invitations(reopened).operation(change.operationId), original);
  assert.equal(
    door.calls.filter((call) => call.envelope.operation === 'remote.invites.confirm').length,
    1,
  );
  assert.equal(door.opens, 2);
});
test('strict pending codec rejects secrets, changed snapshot framing and candidate phase mismatch', () => {
  assert.deepEqual(parsePendingInvitation({ state: 'none' }), { state: 'none' });
  for (const value of [
    { ...pending, code: 'SECRET' },
    { ...pending, phase: 'confirmed' },
    { ...pending, candidate: { ...candidate, words: ['one'] } },
    { ...pending, candidate: { ...candidate, snapshot: `${candidate.snapshot}=` } },
    { ...pending, candidate: { ...candidate, name: 'Guest\nadmin' } },
    { ...pending, candidate: { ...candidate, responseKey: 'SECRET' } },
  ])
    assert.throws(() => parsePendingInvitation(value));
});
test('authenticated but mismatched invitation outcome is uncertainty, not confirmation', async () => {
  const { client, door } = await ready();
  door.managementReply = (_, value) => ({
    operationId: value.operationId,
    state: 'committed',
    sessionEnded: false,
    result: {
      invitation: {
        offerId: crypto.randomUUID(),
        clientId: candidate.clientId,
        memberId: 'member',
        state: 'confirmed',
        sendingEnabled: false,
      },
    },
  });
  const change = input();
  await assert.rejects(
    client.decide(change),
    (error) => error instanceof ClientError && error.operationId === change.operationId,
  );
  assert.equal(door.calls.length, 1);
  assert.throws(() =>
    parseInvitationOutcome(
      {
        operationId: change.operationId,
        state: 'committed',
        sessionEnded: false,
        result: {
          invitation: {
            offerId: pending.offerId,
            clientId: candidate.clientId,
            state: 'refused',
            secret: 'SECRET',
          },
        },
      },
      change.operationId,
    ),
  );
});
