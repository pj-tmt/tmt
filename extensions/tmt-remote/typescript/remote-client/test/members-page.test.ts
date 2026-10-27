import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { MembersPage, pendingMemberObservations, type MemberIntent } from '../src/members-page.js';
import type {
  MemberOriginalOutcome,
  MemberOutcome,
  MemberRoster,
  RemoteMember,
  RemoteMembers,
} from '../src/members.js';

const remoteId = '11111111-1111-4111-8111-111111111111';
const operationId = '22222222-2222-4222-8222-222222222222';
const person: RemoteMember = {
  remoteId,
  memberId: 'member-1',
  role: 'user',
  displayName: 'A person',
  sendingEnabled: false,
  revision: 4,
  createdAtMs: 1,
  revokedAtMs: null,
};
const roster = (administrator = true): MemberRoster => ({
  remoteId,
  administrator,
  displayName: 'Remote',
  members: [{ ...person }],
  nextCursor: null,
  invitation: administrator ? { state: 'no-current-offer' } : null,
  routes: { state: 'unavailable' },
});
const sending = (): MemberIntent => ({
  kind: 'sending',
  input: { operationId, remoteId, memberId: person.memberId, memberRevision: 4, enabled: true },
});

function fixture() {
  let calls = { lists: 0, machines: 0, effects: 0, originals: 0, reopens: 0 };
  let view = roster();
  let readError = false;
  let answer: MemberOriginalOutcome = {
    operationId,
    state: 'unknown',
    reason: 'effect_outcome_unconfirmed',
  };
  let effectError = false;
  const client: RemoteMembers = {
    clientId: 'current',
    async list() {
      calls.lists++;
      if (readError) throw new Error('unavailable');
      return view;
    },
    async machines(input) {
      calls.machines++;
      return { remoteId, memberId: input.memberId, machines: [], nextCursor: null };
    },
    async rename(input) {
      calls.effects++;
      if (effectError) throw new Error('lost answer');
      return {
        operationId: input.operationId,
        state: 'unknown',
        reason: 'effect_outcome_unconfirmed',
      };
    },
    async sending(input) {
      calls.effects++;
      if (effectError) throw new Error('lost answer');
      return {
        operationId: input.operationId,
        state: 'unknown',
        reason: 'effect_outcome_unconfirmed',
      };
    },
    async revoke(input) {
      calls.effects++;
      return {
        operationId: input.operationId,
        state: 'unknown',
        reason: 'effect_outcome_unconfirmed',
      };
    },
    async renameRemote(input) {
      return {
        operationId: input.operationId,
        state: 'unknown',
        reason: 'effect_outcome_unconfirmed',
      };
    },
    async operation() {
      calls.originals++;
      return answer;
    },
  };
  const page = new MembersPage(client, async () => {
    calls.reopens++;
    return client;
  });
  return {
    page,
    client,
    calls: () => ({ ...calls }),
    view: (value: MemberRoster) => {
      view = value;
    },
    failRead: () => {
      readError = true;
    },
    failEffect: () => {
      effectError = true;
    },
    answer: (value: MemberOriginalOutcome) => {
      answer = value;
    },
  };
}

test('one roster is read; machine pages require an explicit admitted expansion', async () => {
  const f = fixture();
  await f.page.refresh();
  assert.equal(f.page.access, 'live');
  assert.equal(f.page.roster?.members[0]?.memberId, person.memberId);
  assert.equal(f.calls().machines, 0);
  await f.page.openMachines('not-admitted');
  assert.equal(f.calls().machines, 0);
  await f.page.openMachines(person.memberId);
  assert.equal(f.calls().machines, 1);
  assert.equal(f.page.machines?.memberId, person.memberId);
  await f.page.refresh();
  assert.equal(f.page.machines, undefined);
  await f.page.openMachines(person.memberId);
  f.failRead();
  await f.page.refresh();
  assert.equal(f.page.access, 'unconfirmed');
  assert.equal(f.page.roster, undefined);
  assert.equal(f.page.machines, undefined);
});

test('a non-administrator cannot submit sending or revoke from presentation', async () => {
  const f = fixture();
  f.view(roster(false));
  await f.page.refresh();
  await assert.rejects(f.page.submit(sending()));
  await assert.rejects(f.page.submit({ kind: 'revoke', input: sending().input }));
  assert.equal(f.calls().effects, 0);
});

test('unknown effect retains its frozen original across refresh and recovery never resends', async () => {
  const f = fixture();
  await f.page.refresh();
  f.failEffect();
  const intent = sending();
  await f.page.submit(intent);
  intent.input.memberRevision = 99;
  assert.equal(f.page.intent?.input.memberRevision, 4);
  assert.equal(f.page.outcome?.state, 'unknown');
  await f.page.refresh();
  assert.equal(f.page.intent?.input.operationId, operationId);
  await assert.rejects(f.page.submit(sending()));
  f.answer({
    operationId,
    state: 'committed',
    result: { member: { ...person, sendingEnabled: true, revision: 5 } },
    sessionEnded: false,
  });
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'committed');
  assert.deepEqual(f.calls(), { lists: 2, machines: 0, effects: 1, originals: 1, reopens: 1 });
});

test('a mismatched recovered result stays unknown and cannot authorize another effect', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.submit(sending());
  f.answer({
    operationId,
    state: 'committed',
    result: { member: { ...person, memberId: 'other', sendingEnabled: true } },
    sessionEnded: false,
  });
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'unknown');
  await f.page.recover();
  await assert.rejects(f.page.submit(sending()));
  assert.equal(f.calls().effects, 1);
  assert.equal(f.calls().originals, 2);
});

test('unknown original can be checked again and a later committed result unlocks changes', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.submit(sending());
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'unknown');
  assert.equal(f.page.intent?.input.operationId, operationId);
  assert.equal(f.page.canRecover, true);
  await assert.rejects(f.page.submit(sending()));
  f.answer({
    operationId,
    state: 'committed',
    result: { member: { ...person, sendingEnabled: true, revision: 5 } },
    sessionEnded: false,
  });
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'committed');
  assert.equal(f.page.canRecover, false);
  assert.deepEqual(f.calls(), { lists: 1, machines: 0, effects: 1, originals: 2, reopens: 2 });
});

test('a failed original lookup can be explicitly checked again without replay', async () => {
  const f = fixture();
  const original = f.client.operation.bind(f.client);
  let attempts = 0;
  const originalIds: string[] = [];
  f.client.operation = async (id) => {
    attempts++;
    originalIds.push(id);
    if (attempts === 1) throw new Error('lookup unavailable');
    return original(id);
  };
  await f.page.refresh();
  await f.page.submit(sending());
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'unknown');
  assert.equal(f.page.intent?.input.operationId, operationId);
  assert.equal(f.page.access, 'unconfirmed');
  assert.equal(f.page.canRecover, true);
  f.answer({
    operationId,
    state: 'committed',
    result: { member: { ...person, sendingEnabled: true, revision: 5 } },
    sessionEnded: false,
  });
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'committed');
  assert.equal(attempts, 2);
  assert.deepEqual(originalIds, [operationId, operationId]);
  assert.equal(f.calls().effects, 1);
  assert.equal(f.calls().reopens, 2);
  await f.page.refresh();
  assert.equal(f.page.access, 'live');
});

test('an in-flight original lookup fences another lookup and every effect', async () => {
  const f = fixture();
  let release!: (outcome: MemberOriginalOutcome) => void;
  f.client.operation = () =>
    new Promise((resolve) => {
      release = resolve;
    });
  await f.page.refresh();
  await f.page.submit(sending());
  const pending = f.page.recover();
  await Promise.resolve();
  assert.equal(f.page.canRecover, false);
  await f.page.recover();
  await assert.rejects(f.page.submit(sending()));
  release({ operationId, state: 'unknown', reason: 'effect_outcome_unconfirmed' });
  await pending;
  assert.equal(f.page.canRecover, true);
  assert.equal(f.calls().effects, 1);
  assert.equal(f.calls().reopens, 1);
});

test('a recovered self-revocation clears the formerly live roster', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.openMachines(person.memberId);
  await f.page.submit({
    kind: 'revoke',
    input: { operationId, remoteId, memberId: person.memberId, memberRevision: 4 },
  });
  f.answer({
    operationId,
    state: 'committed',
    result: { member: { ...person, revokedAtMs: 2 } },
    sessionEnded: true,
  });
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'committed');
  assert.equal(f.page.access, 'lost');
  assert.equal(f.page.roster, undefined);
  assert.equal(f.page.machines, undefined);
  assert.equal(f.calls().effects, 1);
});

test('disposed views ignore a late roster response', async () => {
  let release!: (value: MemberRoster) => void;
  const f = fixture();
  f.client.list = () =>
    new Promise<MemberRoster>((resolve) => {
      release = resolve;
    });
  const page = f.page;
  const pending = page.refresh();
  page.dispose();
  release(roster());
  await pending;
  assert.equal(page.roster, undefined);
});

test('a refused read reopens at most once and a proven ended session clears facts', async () => {
  const f = fixture();
  const expired = new Error('expired');
  let reads = 0;
  f.client.list = async () => {
    reads++;
    if (reads === 1) throw new Error('reopen');
    throw expired;
  };
  let reopens = 0;
  const page = new MembersPage(
    f.client,
    async () => {
      reopens++;
      return f.client;
    },
    (error) => error instanceof Error && error.message === 'reopen',
    (error) => error === expired,
  );
  await page.refresh();
  assert.equal(reads, 2);
  assert.equal(reopens, 1);
  assert.equal(page.access, 'lost');
  assert.equal(page.roster, undefined);
});

test('access loss during an effect keeps the original unknown and ignores the late answer', async () => {
  const f = fixture();
  let release!: (value: MemberOutcome) => void;
  f.client.sending = () =>
    new Promise((resolve) => {
      release = resolve;
    });
  await f.page.refresh();
  const pending = f.page.submit(sending());
  f.page.invalidateAccess(true);
  release({
    operationId,
    state: 'committed',
    result: { member: { ...person, sendingEnabled: true, revision: 5 } },
    sessionEnded: false,
  });
  await pending;
  assert.equal(f.page.access, 'lost');
  assert.equal(f.page.outcome?.state, 'unknown');
  assert.equal(f.page.intent?.input.operationId, operationId);
});

test('a failed explicit machine read keeps the admitted roster without inventing observations', async () => {
  const f = fixture();
  await f.page.refresh();
  f.client.machines = async () => {
    throw new Error('unavailable');
  };
  await f.page.openMachines(person.memberId);
  assert.equal(f.page.access, 'live');
  assert.equal(f.page.machineAccess, 'unconfirmed');
  assert.equal(f.page.machines, undefined);
  assert.equal(f.page.roster?.members[0]?.memberId, person.memberId);
});

test('pending and unavailable cleanup observations stay separate from idle invitation copy', () => {
  const view = roster();
  view.routes = { state: 'not-recorded' };
  assert.equal(pendingMemberObservations(view), '');
  view.routes = { state: 'unavailable' };
  assert.equal(pendingMemberObservations(view), 'Access update status is unavailable.');
  view.routes = { state: 'recorded', publicationPending: true, removalPending: true };
  view.invitation = {
    state: 'current',
    expiresAtMs: null,
    phase: 'unconfirmed',
    localConfirmationKnown: false,
    cleanup: 'unconfirmed',
  };
  assert.equal(
    pendingMemberObservations(view),
    'Invitation cleanup is unconfirmed. An access update is awaiting delivery. An access removal is awaiting cleanup.',
  );
  assert.equal(pendingMemberObservations(undefined), '');
});
