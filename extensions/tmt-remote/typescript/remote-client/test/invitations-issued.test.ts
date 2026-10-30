import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { openSession } from '../src/device.js';
import {
  invitations,
  parseIssuedInvitationPage,
  parseInvitationIssueOutcome,
} from '../src/invitations.js';
import { operations, ClientError } from '../src/operations.js';
import { Door, paired } from './door.js';
import vector from '../../../rust/tmt-remote/tests/fixtures/invitations/invite-v2.json' with { type: 'json' };

async function ready() {
  const door = new Door();
  const device = await paired(door);
  const session = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  return { door, device, session, client: invitations(session) };
}
function created(remoteId: string, operationId: string) {
  const now = Date.now();
  const invite = structuredClone(vector.invite);
  invite.context.owner = remoteId;
  invite.context.offer = crypto.randomUUID();
  invite.context.issued_at_ms = now;
  invite.context.expires_at_ms = now + 600_000;
  const code = Buffer.from(JSON.stringify(invite)).toString('base64url');
  return {
    operationId,
    state: 'committed' as const,
    sessionEnded: false as const,
    result: {
      invitation: {
        offerId: invite.context.offer,
        state: 'created' as const,
        expiresAtMs: invite.context.expires_at_ms,
      },
    },
    secret: { code, link: `https://published.test/join#${code}` },
  };
}
test('issue, history and cancellation share one signed lane; original recovery never accepts a secret', async () => {
  const { door, session, client } = await ready();
  const operationId = crypto.randomUUID();
  const response = created(door.descriptor.machineId, operationId);
  const { secret, ...original } = response;
  const offerId = response.result.invitation.offerId;
  door.managementReply = (op, input) =>
    op === 'remote.invites.create'
      ? response
      : op === 'remote.invites.list'
        ? {
            remoteId: door.descriptor.machineId,
            invitations: [
              {
                offerId,
                issuedAtMs: response.result.invitation.expiresAtMs - 600_000,
                expiresAtMs: response.result.invitation.expiresAtMs,
                state: 'open',
                localConfirmationKnown: false,
                cleanup: 'not-requested',
              },
            ],
            nextCursor: null,
          }
        : op === 'remote.invites.cancel'
          ? {
              operationId: input.operationId,
              state: 'committed',
              sessionEnded: false,
              result: {
                invitation: {
                  offerId,
                  state: 'cancelled',
                  code: 'REMOTE_INVITATION_CANCELLED',
                },
              },
            }
          : original;
  const input = { operationId };
  const pending = client.create(input);
  input.operationId = crypto.randomUUID();
  const [answer] = await Promise.all([pending, operations(session).listAgents()]);
  assert.deepEqual(answer, response);
  assert.deepEqual(door.calls[0]!.payload, { operationId });
  const page = await client.list({ cursor: null });
  assert.equal(page.invitations.length, 1);
  const cancel = await client.cancel({ operationId: crypto.randomUUID(), offerId });
  assert.equal(cancel.state, 'committed');
  assert.deepEqual(await client.operation(operationId), original);
  assert.equal(
    door.calls.filter((c) => c.envelope.operation === 'remote.invites.create').length,
    1,
  );
  assert.throws(() => parseInvitationIssueOutcome(response, operationId));
  assert.throws(
    () =>
      parseInvitationIssueOutcome(
        { ...response, secret: { ...secret, code: 'SECRET_CANARY' } },
        operationId,
        door.descriptor.machineId,
      ),
    (error) => error instanceof Error && !error.message.includes('SECRET_CANARY'),
  );
  assert.throws(() => parseInvitationIssueOutcome(response, operationId, crypto.randomUUID()));
  assert.throws(() =>
    parseInvitationIssueOutcome(
      { ...response, secret: { ...secret, link: 'https://published.test/join#different' } },
      operationId,
      door.descriptor.machineId,
    ),
  );
  assert.throws(() =>
    parseIssuedInvitationPage(
      { ...page, remoteId: crypto.randomUUID() },
      door.descriptor.machineId,
    ),
  );
  assert.throws(() =>
    parseIssuedInvitationPage(
      { ...page, invitations: [{ ...page.invitations[0], code: secret.code }] },
      door.descriptor.machineId,
    ),
  );
  assert.throws(() =>
    parseIssuedInvitationPage(
      {
        ...page,
        invitations: [{ ...page.invitations[0], expiresAtMs: Number.MAX_SAFE_INTEGER + 1 }],
      },
      door.descriptor.machineId,
    ),
  );
});
test('a lost creating response is not retried; a reopened session recovers only original metadata', async () => {
  const { door, device, client } = await ready();
  const id = crypto.randomUUID();
  const response = created(door.descriptor.machineId, id);
  const { secret, ...original } = response;
  door.managementReply = () => response;
  const reply = door.response;
  door.response = async () => new Response('{}', { status: 404 });
  await assert.rejects(
    client.create({ operationId: id }),
    (error) => error instanceof ClientError && error.operationId === id,
  );
  assert.equal(
    door.calls.filter((c) => c.envelope.operation === 'remote.invites.create').length,
    1,
  );
  door.response = reply;
  door.managementReply = () => original;
  const reopened = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  const result = await invitations(reopened).operation(id);
  assert.deepEqual(result, original);
  assert.ok(!JSON.stringify(result).includes(secret.code));
  assert.equal(
    door.calls.filter((c) => c.envelope.operation === 'remote.invites.create').length,
    1,
  );
});

test('issuance refuses mismatched committed actions and invalid input before another effect', async () => {
  const { door, client } = await ready();
  const operationId = crypto.randomUUID();
  const offerId = crypto.randomUUID();
  door.managementReply = () => ({
    operationId,
    state: 'committed',
    sessionEnded: false,
    result: { invitation: { offerId, state: 'cancelled', code: 'REMOTE_INVITATION_CANCELLED' } },
  });
  await assert.rejects(
    client.create({ operationId }),
    (error) => error instanceof ClientError && error.operationId === operationId,
  );
  assert.equal(
    door.calls.filter((c) => c.envelope.operation === 'remote.invites.create').length,
    1,
  );
  assert.throws(() => client.create({ operationId: 'invalid' }));
  assert.throws(() => client.create({ operationId, extra: 'secret' } as { operationId: string }));
  assert.throws(() =>
    client.cancel({ operationId, offerId, extra: true } as {
      operationId: string;
      offerId: string;
    }),
  );
  assert.throws(() => client.list({ cursor: '?not-a-cursor' }));
  assert.equal(door.calls.length, 1);
});
