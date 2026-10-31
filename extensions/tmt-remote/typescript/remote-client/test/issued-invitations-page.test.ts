import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { IssuedInvitationsPage } from '../src/issued-invitations-page.js';
import type { RemoteInvitations } from '../src/invitations.js';

const operationId = '11111111-1111-4111-8111-111111111111';
const offerId = '22222222-2222-4222-8222-222222222222';
const made = {
  operationId,
  state: 'committed' as const,
  sessionEnded: false as const,
  result: { invitation: { offerId, state: 'created' as const, expiresAtMs: 5000 } },
};
function fixture() {
  const calls = { create: 0, cancel: 0, operation: 0, list: 0 };
  let lost = false;
  const client = {
    async list({ cursor }: { cursor: string | null }) {
      calls.list++;
      return { remoteId: 'remote', invitations: [], nextCursor: cursor === null ? 'next' : null };
    },
    async create() {
      calls.create++;
      if (lost) throw new Error('lost');
      return { ...made, secret: { code: 'one-shot', link: null } };
    },
    async cancel(input: { operationId: string; offerId: string }) {
      calls.cancel++;
      return {
        operationId: input.operationId,
        state: 'committed' as const,
        sessionEnded: false as const,
        result: {
          invitation: {
            offerId: input.offerId,
            state: 'cancelled' as const,
            code: 'REMOTE_INVITATION_CANCELLED' as const,
          },
        },
      };
    },
    async operation() {
      calls.operation++;
      return made;
    },
  } as unknown as RemoteInvitations;
  return {
    page: new IssuedInvitationsPage(client, async () => client),
    calls,
    lose: () => {
      lost = true;
    },
  };
}

test('one-shot secret stays in the current view and is cleared on disposal', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.create(operationId);
  assert.equal(f.page.secret?.code, 'one-shot');
  assert.equal('secret' in f.page.outcome!, false);
  assert.equal(f.calls.create, 1);
  f.page.dispose();
  assert.equal(f.page.secret, undefined);
  assert.equal(f.page.history, undefined);
});

test('unknown creation is recovered by its original id without a second create or a secret', async () => {
  const f = fixture();
  await f.page.refresh();
  f.lose();
  await f.page.create(operationId);
  assert.equal(f.page.outcome?.state, 'unknown');
  await assert.rejects(f.page.create(operationId));
  await f.page.recover();
  assert.equal(f.page.outcome?.state, 'committed');
  assert.equal(f.page.secret, undefined);
  assert.equal(f.calls.create, 1);
  assert.equal(f.calls.operation, 1);
});

test('list pagination and cancellation use explicit inputs', async () => {
  const f = fixture();
  await f.page.refresh();
  assert.equal(f.page.history?.nextCursor, 'next');
  await f.page.refresh('next');
  assert.equal(f.page.history?.nextCursor, null);
  await f.page.cancel(operationId, offerId);
  assert.equal(f.calls.cancel, 1);
});

test('a late creation response after disposal cannot restore the secret', async () => {
  let finish!: (value: unknown) => void;
  const response = new Promise((resolve) => {
    finish = resolve;
  });
  const client = {
    async list() {
      return { remoteId: 'remote', invitations: [], nextCursor: null };
    },
    async create() {
      return response;
    },
  } as unknown as RemoteInvitations;
  const page = new IssuedInvitationsPage(client, async () => client);
  await page.refresh();
  const pending = page.create(operationId);
  page.dispose();
  finish({ ...made, secret: { code: 'late-secret', link: null } });
  await pending;
  assert.equal(page.secret, undefined);
  assert.equal(page.outcome, undefined);
});

test('a mismatched recovery never settles the original or enables a new effect', async () => {
  const f = fixture();
  await f.page.refresh();
  f.lose();
  await f.page.create(operationId);
  const foreign = { ...made, operationId: '33333333-3333-4333-8333-333333333333' };
  const client = {
    async operation() {
      return foreign;
    },
  } as unknown as RemoteInvitations;
  const page = new IssuedInvitationsPage(
    {
      async list() {
        return { remoteId: 'remote', invitations: [], nextCursor: null };
      },
      async create() {
        throw new Error('lost');
      },
    } as unknown as RemoteInvitations,
    async () => client,
  );
  await page.refresh();
  await page.create(operationId);
  await page.recover();
  assert.equal(page.outcome?.state, 'unknown');
  await assert.rejects(page.cancel(operationId, offerId));
});

test('closing the created dialog clears the one-shot secret without changing its outcome', async () => {
  const f = fixture();
  await f.page.refresh();
  await f.page.create(operationId);
  assert.equal(f.page.secret?.code, 'one-shot');
  f.page.clearSecret();
  assert.equal(f.page.secret, undefined);
  assert.equal(f.page.outcome?.state, 'committed');
});
