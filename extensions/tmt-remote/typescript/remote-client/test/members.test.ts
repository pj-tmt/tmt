import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { openSession } from '../src/device.js';
import { members, type MemberMachine, type RemoteMember } from '../src/members.js';
import { ClientError, operations } from '../src/operations.js';
import { Door, paired } from './door.js';

async function ready() {
  const door = new Door();
  const device = await paired(door);
  const session = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  const client = members(session);
  const member: RemoteMember = {
    remoteId: door.descriptor.machineId,
    memberId: 'legacy-person-id',
    role: 'user',
    displayName: 'Person',
    sendingEnabled: false,
    revision: 1,
    createdAtMs: 1,
    revokedAtMs: null,
  };
  const roster = {
    remoteId: member.remoteId,
    administrator: false,
    displayName: null,
    members: [{ ...member, lastUsedAtMs: null as number | null }],
    nextCursor: null,
    invitation: null,
    routes: { state: 'unavailable' },
  };
  door.managementReply = (operation, value) =>
    operation === 'remote.members.list'
      ? roster
      : {
          operationId: value.operationId,
          state: 'committed',
          result: {
            member: { ...member, displayName: value.name ?? member.displayName, revision: 2 },
          },
          sessionEnded: false,
        };
  return { door, client, session, member, roster };
}
test('member roster retains opaque legacy identifiers and unknown observations', async () => {
  const { client, member } = await ready();
  const view = await client.list({ cursor: null, limit: 50 });
  assert.equal(view.members[0]!.memberId, member.memberId);
  assert.equal(view.administrator, false);
  assert.equal(view.invitation, null);
  assert.deepEqual(view.routes, { state: 'unavailable' });
});
test('member mutation freezes intent and shares the existing agent signing lane', async () => {
  const { door, client, session, member } = await ready();
  const change = {
    operationId: crypto.randomUUID(),
    remoteId: member.remoteId,
    memberId: member.memberId,
    memberRevision: 1,
    name: 'Frozen',
  };
  const pending = client.rename(change);
  change.name = 'Changed after submission';
  const [outcome] = await Promise.all([
    pending,
    client.list({ cursor: null, limit: 50 }),
    operations(session).listAgents(),
  ]);
  assert.equal(outcome.state, 'committed');
  assert.equal(door.calls[0]!.payload.name, 'Frozen');
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '3'],
  );
  assert.equal(door.opens, 1);
});
test('member administration refusal is definite, while a missing answer keeps the original ID', async () => {
  const { door, client, member } = await ready();
  const change = {
    operationId: crypto.randomUUID(),
    remoteId: member.remoteId,
    memberId: member.memberId,
    memberRevision: 1,
    enabled: true,
  };
  door.error = { code: 'REMOTE_MEMBER_OWNER_REQUIRED', message: 'Administrator required.' };
  assert.deepEqual(await client.sending(change), {
    operationId: change.operationId,
    state: 'refused',
    reason: 'REMOTE_MEMBER_OWNER_REQUIRED',
  });
  const lost = await ready();
  lost.door.response = async () => new Response('{}', { status: 404 });
  const id = crypto.randomUUID();
  assert.throws(
    () => lost.client.revoke({ ...change, operationId: id, enabled: undefined } as never),
    TypeError,
  );
  await assert.rejects(
    lost.client.revoke({
      operationId: id,
      remoteId: lost.member.remoteId,
      memberId: lost.member.memberId,
      memberRevision: 1,
    }),
    (error) =>
      error instanceof ClientError &&
      error.operationId === id &&
      error.code === 'transport_failure',
  );
  assert.equal(door.calls.length, 1);
  assert.equal(lost.door.calls.length, 1);
  assert.equal(door.opens, 1);
  assert.equal(lost.door.opens, 1);
});
test('unverifiable member answer is observed only by the original ID without resending', async () => {
  const { door, client, member } = await ready();
  const id = crypto.randomUUID();
  door.tamper.signature = true;
  await assert.rejects(
    client.rename({
      operationId: id,
      remoteId: member.remoteId,
      memberId: member.memberId,
      memberRevision: 1,
      name: 'Name',
    }),
    (error) =>
      error instanceof ClientError &&
      error.operationId === id &&
      error.code === 'unverifiable_response',
  );
  door.tamper.signature = false;
  door.managementReply = (operation, input) => {
    assert.equal(operation, 'remote.management.operation');
    return {
      operationId: input.operationId,
      state: 'unknown',
      reason: 'effect_outcome_unconfirmed',
    };
  };
  assert.deepEqual(await client.operation(id), {
    operationId: id,
    state: 'unknown',
    reason: 'effect_outcome_unconfirmed',
  });
  assert.equal(
    door.calls.filter((call) => call.envelope.operation === 'remote.members.rename').length,
    1,
  );
  assert.equal(door.opens, 1);
});
test('member helper can recover exact historical device outcomes', async () => {
  const { door, client } = await ready();
  const id = crypto.randomUUID();
  const historical = {
    operationId: id,
    state: 'committed',
    result: {
      device: {
        clientId: door.clientId,
        name: 'Historical machine',
        kind: 'browser',
        issuedAtMs: 1,
        expiresAtMs: null,
        revision: 1,
        revoked: false,
        talkEnabled: false,
      },
    },
    sessionEnded: false,
  };
  door.managementReply = () => historical;
  assert.deepEqual(await client.operation(id), historical);
});
test('strict roster decoding refuses another Remote, private fields, invented presence and oversize pages', async () => {
  for (const mutate of [
    (r: Record<string, unknown>) => {
      r.remoteId = crypto.randomUUID();
    },
    (r: Record<string, unknown>) => {
      r.uid = 'private';
    },
    (r: Record<string, unknown>) => {
      r.routes = { state: 'online' };
    },
    (r: Record<string, unknown>) => {
      r.members = [...(r.members as unknown[]), ...(r.members as unknown[])];
    },
    (r: Record<string, unknown>) => {
      r.invitation = { state: 'no-current-offer' };
    },
  ]) {
    const { door, client, roster } = await ready();
    const bad = structuredClone(roster) as Record<string, unknown>;
    mutate(bad);
    door.managementReply = () => bad;
    await assert.rejects(client.list({ cursor: null, limit: 1 }), ClientError);
  }
});
test('member result correlation refuses another member and invalid arguments never publish', async () => {
  const { door, client, member } = await ready();
  await assert.rejects(
    client.renameRemote({
      operationId: crypto.randomUUID(),
      remoteId: crypto.randomUUID(),
      name: 'Wrong Remote',
    }),
    TypeError,
  );
  assert.throws(() => client.list({ cursor: null, limit: 51 }), TypeError);
  assert.equal(door.calls.length, 0);
  door.managementReply = (_, input) => ({
    operationId: input.operationId,
    state: 'committed',
    result: {
      member: { ...member, memberId: 'another-person', displayName: input.name, revision: 2 },
    },
    sessionEnded: false,
  });
  const id = crypto.randomUUID();
  await assert.rejects(
    client.rename({
      operationId: id,
      remoteId: member.remoteId,
      memberId: member.memberId,
      memberRevision: 1,
      name: 'Name',
    }),
    (error) => error instanceof ClientError && error.operationId === id,
  );
  assert.equal(door.calls.length, 1);
});

test('member machine decoding preserves local eligibility and unknown Session activity', async () => {
  const { door, client, member } = await ready();
  const machine: MemberMachine = {
    clientId: door.clientId,
    name: 'Machine',
    kind: 'browser',
    revision: 1,
    revoked: false,
    expiresAtMs: null,
    sendingScope: false,
    delivery: 'hold',
    recipientAccess: 'restricted',
    grantLive: true,
    lastUsedAtMs: null,
    thisBrowser: true,
    liveSessionCount: null,
    lastActivityAtMs: null,
  };
  door.managementReply = () => ({
    remoteId: member.remoteId,
    memberId: member.memberId,
    machines: [machine],
    nextCursor: null,
  });
  const page = { memberId: member.memberId, cursor: null, limit: 1 };
  assert.deepEqual((await client.machines(page)).machines, [machine]);
  machine.liveSessionCount = 0;
  assert.equal((await client.machines(page)).machines[0]!.liveSessionCount, 0);
  for (const changed of [
    { liveSessionCount: 0, lastActivityAtMs: 1 },
    { uid: 'private-provider-principal' },
    { revoked: true, grantLive: true },
    { recipientAccess: 'all-with-agent-ids' },
  ]) {
    door.managementReply = () => ({
      remoteId: member.remoteId,
      memberId: member.memberId,
      machines: [{ ...machine, ...changed }],
      nextCursor: null,
    });
    await assert.rejects(client.machines(page), ClientError);
  }
});

test('member policy and Remote-name results retain strict correlation and original identities', async () => {
  const { door, client, member } = await ready();
  const target = {
    operationId: crypto.randomUUID(),
    remoteId: member.remoteId,
    memberId: member.memberId,
    memberRevision: 1,
  };
  door.managementReply = (op, input) => ({
    operationId: input.operationId,
    state: 'committed',
    result:
      op === 'remote.name.set'
        ? { remote: { remoteId: member.remoteId, displayName: input.name } }
        : {
            member: {
              ...member,
              revision: 2,
              sendingEnabled: op === 'remote.members.sending',
              revokedAtMs: op === 'remote.members.revoke' ? 5 : null,
            },
          },
    sessionEnded: false,
  });
  assert.equal((await client.sending({ ...target, enabled: true })).state, 'committed');
  assert.equal(
    (await client.revoke({ ...target, operationId: crypto.randomUUID() })).state,
    'committed',
  );
  const rename = {
    operationId: crypto.randomUUID(),
    remoteId: member.remoteId,
    name: 'Shared Remote',
  };
  const result = await client.renameRemote(rename);
  assert.deepEqual(result, {
    operationId: rename.operationId,
    state: 'committed',
    result: { remote: { remoteId: member.remoteId, displayName: rename.name } },
    sessionEnded: false,
  });
  door.managementReply = () => result;
  assert.deepEqual(await client.operation(rename.operationId), result);
  door.managementReply = (_, input) => ({
    operationId: input.operationId,
    state: 'committed',
    result: { remote: { remoteId: crypto.randomUUID(), displayName: rename.name } },
    sessionEnded: false,
  });
  const id = crypto.randomUUID();
  await assert.rejects(
    client.renameRemote({ ...rename, operationId: id }),
    (e) => e instanceof ClientError && e.operationId === id,
  );
});

test('member summary preserves nullable recorded facts and uses the verified read lane', async () => {
  const { door, client, session, member } = await ready();
  const view = { remoteId: member.remoteId, memberCount: 4, lastActiveAtMs: 123 };
  const original = door.managementReply!;
  door.managementReply = (operation, value) => {
    if (operation !== 'remote.members.summary') return original(operation, value);
    assert.deepEqual(value, {});
    return view;
  };
  const [summary] = await Promise.all([
    client.summary(),
    client.list({ cursor: null, limit: 50 }),
    operations(session).listAgents(),
  ]);
  assert.deepEqual(summary, view);
  assert.deepEqual(
    door.calls.map((call) => call.envelope.sequence),
    ['1', '2', '3'],
  );
  assert.equal(door.opens, 1);
  door.managementReply = () => ({ ...view, memberCount: null, lastActiveAtMs: null });
  assert.deepEqual(await client.summary(), { ...view, memberCount: null, lastActiveAtMs: null });
});

test('member summary refuses uncorrelated or unsafe facts and unknown fields', async () => {
  for (const changed of [
    { remoteId: crypto.randomUUID() },
    { online: true },
    { memberCount: -1 },
    { memberCount: 1.5 },
    { memberCount: Number.MAX_SAFE_INTEGER + 1 },
    { memberCount: '1' },
    { lastActiveAtMs: -1 },
    { lastActiveAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { lastActiveAtMs: 'now' },
  ]) {
    const { door, client, member } = await ready();
    const view = { remoteId: member.remoteId, memberCount: 1, lastActiveAtMs: null };
    door.managementReply = () => ({ ...view, ...changed });
    await assert.rejects(client.summary(), ClientError);
    assert.equal(door.opens, 1);
    assert.deepEqual(
      door.calls.map((call) => call.envelope.operation),
      ['remote.members.summary'],
    );
  }
});

test('roster use is nullable historical data and never part of a mutation result', async () => {
  const { client, roster, member } = await ready();
  for (const value of [null, 0, 1234]) {
    roster.members[0]!.lastUsedAtMs = value;
    assert.equal((await client.list({ cursor: null, limit: 50 })).members[0]!.lastUsedAtMs, value);
  }
  const outcome = await client.rename({
    operationId: crypto.randomUUID(),
    remoteId: member.remoteId,
    memberId: member.memberId,
    memberRevision: 1,
    name: 'New name',
  });
  assert.equal(outcome.state, 'committed');
  if (outcome.state === 'committed' && 'member' in outcome.result)
    assert.equal(Object.hasOwn(outcome.result.member, 'lastUsedAtMs'), false);
  for (const bad of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, 'today', undefined]) {
    Object.assign(roster.members[0]!, { lastUsedAtMs: bad });
    await assert.rejects(client.list({ cursor: null, limit: 50 }));
  }
});
