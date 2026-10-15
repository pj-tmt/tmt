import { digest } from '@tmt/colab-client';
import { expect, it } from 'vite-plus/test';
import type {
  SealedDocument,
  SealedSyncPort,
  SyncMutation,
  SyncTarget,
} from '../../../../tmt-remote/typescript/remote-client/src/sealed-sync.js';
import { HostedAdmission, type HostedLinkAdmission } from '../src/hosted-admission.js';
import { HostedBudget, type HostedUsage } from '../src/hosted-budget.js';
import * as math from '../../../../tmt-remote/typescript/remote-client/src/budget.js';

const input: HostedLinkAdmission = {
  space: 'a'.repeat(32),
  page: '11111111-1111-4111-8111-111111111111',
  link: '22222222-2222-4222-8222-222222222222',
  role: 'commenter',
  token: 'A'.repeat(43),
};
const uid = 'holder-uid';
const member = { resource: 'members', id: `${input.space}_${input.page}_${uid}` };
const page = { resource: 'pages', id: `${input.space}_${input.page}` };
const call = () => ({ signal: new AbortController().signal, deadlineAtMs: Date.now() + 30_000 });
async function document(
  target: SyncTarget,
  fields: Record<string, unknown>,
): Promise<SealedDocument> {
  const bytes = new TextEncoder().encode(
    JSON.stringify(
      Object.fromEntries(Object.entries(fields).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))),
    ),
  );
  return {
    target,
    bytes,
    digest: Array.from(await digest(bytes), (byte) => byte.toString(16).padStart(2, '0')).join(''),
  };
}
const memberFields = () => ({
  ...Object.fromEntries(
    Object.entries({ ...input, uid }).map(([key, value]) => [key, { stringValue: value }]),
  ),
  seq: { integerValue: '1' },
});
const pageFields = () => ({
  space: { stringValue: input.space },
  page: { stringValue: input.page },
  epoch: { integerValue: '7' },
  state: { stringValue: 'open' },
  expiresAt: { timestampValue: '2027-01-01T00:00:00Z' },
});

async function fixture(existing = false) {
  const events: unknown[] = [];
  let row = existing ? await document(member, memberFields()) : undefined;
  const pageDocument = await document(page, pageFields());
  let reads: ((target: SyncTarget) => Promise<SealedDocument | undefined>) | undefined;
  let publish:
    | ((mutation: SyncMutation) => Promise<{ kind: 'confirmed' | 'unknown' | 'already-present' }>)
    | undefined;
  const port = {
    context: { space: input.space, uid },
    async read(target: SyncTarget) {
      events.push(['read', target]);
      return reads ? reads(target) : target.resource === 'members' ? row : pageDocument;
    },
    async publish(mutation: SyncMutation) {
      events.push(['publish', mutation]);
      if (publish) return publish(mutation);
      if (mutation.mode !== 'create') throw new Error('only a members create is allowed');
      row = mutation.document;
      return { kind: 'confirmed' };
    },
  } as unknown as SealedSyncPort;
  const admission = new HostedAdmission(port);
  return {
    admission,
    events,
    port,
    pageDocument,
    read: (value: typeof reads) => {
      reads = value;
    },
    publish: (value: typeof publish) => {
      publish = value;
    },
  };
}

it('creates exactly the Rules-shaped own member once, then gets page hints before any discovery', async () => {
  const f = await fixture();
  expect(await f.admission.open(input, call())).toEqual({
    epoch: '7',
    expiresAtMs: Date.parse('2027-01-01T00:00:00Z'),
  });
  expect(f.events.map((event) => (event as unknown[])[0])).toEqual(['read', 'publish', 'read']);
  const mutation = (f.events[1] as [string, SyncMutation])[1];
  expect(mutation.mode).toBe('create');
  if (mutation.mode === 'delete') throw new Error('unexpected mutation');
  expect(mutation.document.target).toEqual(member);
  expect(JSON.parse(new TextDecoder().decode(mutation.document.bytes))).toEqual(memberFields());
  expect(f.events[2]).toEqual(['read', page]);
});

it('reads an existing own member without re-creating it', async () => {
  const f = await fixture(true);
  expect((await f.admission.open(input, call())).epoch).toBe('7');
  expect(f.events).toEqual([
    ['read', member],
    ['read', page],
  ]);
});

it('joins duplicate opens to the same single admission attempt', async () => {
  const f = await fixture();
  await Promise.all([f.admission.open(input, call()), f.admission.open(input, call())]);
  expect(f.events.filter((event) => (event as unknown[])[0] === 'publish')).toHaveLength(1);
});

it('permits one create only after the own-target missing-row denial; never probes roles', async () => {
  const f = await fixture();
  f.read(async (target) => {
    if (target.resource === 'members') throw { code: 'REMOTE_SCOPE_DENIED' };
    return f.pageDocument;
  });
  await f.admission.open(input, call());
  expect(f.events.filter((event) => (event as unknown[])[0] === 'publish')).toHaveLength(1);
});

it.each(['REMOTE_SCOPE_DENIED', 'REMOTE_BUDGET_EXHAUSTED', 'REMOTE_RATE_LIMITED'])(
  'never retries a commit refusal %s',
  async (code) => {
    const f = await fixture();
    const error = { code, resetAtMs: 99, retryAfterMs: 1 };
    f.publish(async () => {
      throw error;
    });
    await expect(f.admission.open(input, call())).rejects.toEqual(error);
    await expect(f.admission.open(input, call())).rejects.toEqual(error);
    expect(f.events.filter((event) => (event as unknown[])[0] === 'publish')).toHaveLength(1);
    expect(f.events.some((event) => JSON.stringify(event) === JSON.stringify(['read', page]))).toBe(
      false,
    );
  },
);

it('reads back an unknown create once and never sends it again', async () => {
  const f = await fixture();
  let reads = 0;
  f.read(async (target) =>
    target.resource === 'pages'
      ? f.pageDocument
      : ++reads === 1
        ? undefined
        : await document(member, memberFields()),
  );
  f.publish(async () => ({ kind: 'unknown' }));
  expect((await f.admission.open(input, call())).epoch).toBe('7');
  expect(f.events.map((event) => (event as unknown[])[0])).toEqual([
    'read',
    'publish',
    'read',
    'read',
  ]);
});

it('keeps an absent unknown create unavailable, with no re-create or page read', async () => {
  const f = await fixture();
  f.publish(async () => ({ kind: 'unknown' }));
  await expect(f.admission.open(input, call())).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
  });
  await expect(f.admission.open(input, call())).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
  });
  expect(f.events.filter((event) => (event as unknown[])[0] === 'publish')).toHaveLength(1);
});

it('refuses a different existing role/link/token without overwriting or probing', async () => {
  const f = await fixture();
  f.read(async () => document(member, { ...memberFields(), role: { stringValue: 'viewer' } }));
  await expect(f.admission.open(input, call())).rejects.toMatchObject({
    code: 'REMOTE_SCOPE_DENIED',
  });
  expect(f.events).toEqual([['read', member]]);
});

it('never assumes epoch one or discovers after a denied page get', async () => {
  const f = await fixture(true);
  f.read(async (target) => {
    if (target.resource === 'pages') throw { code: 'REMOTE_SCOPE_DENIED' };
    return document(member, memberFields());
  });
  await expect(f.admission.open(input, call())).rejects.toEqual({ code: 'REMOTE_SCOPE_DENIED' });
});

it('fences cancellation before any read or effect', async () => {
  const f = await fixture();
  const stopped = AbortSignal.abort();
  await expect(
    f.admission.open(input, { signal: stopped, deadlineAtMs: Date.now() + 30_000 }),
  ).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' });
  expect(f.events).toHaveLength(0);
});

it('charges both bootstrap gets and the sole members commit through the caller guard', async () => {
  const f = await fixture();
  let usage: HostedUsage | undefined;
  const guard = new HostedBudget(math, {
    transact: async (change) => {
      const update = change(usage);
      usage = update.usage;
      return update.result;
    },
  });
  const read = f.port.read.bind(f.port),
    publish = f.port.publish.bind(f.port);
  f.port.read = async (target, budget) => {
    await guard.admit({ operation: 'read', reads: 1, writes: 0, rulesLookupsPerDocument: 3 });
    return read(target, budget);
  };
  f.port.publish = async (mutation, budget) => {
    await guard.admit({ operation: 'commit', reads: 0, writes: 1, rulesLookupsPerDocument: 4 });
    return publish(mutation, budget);
  };
  await f.admission.open(input, call());
  expect(usage).toMatchObject({ reads: 12, writes: 1 });
});

it.each([
  { epoch: { integerValue: '0' } },
  { epoch: { integerValue: '01' } },
  { expiresAt: { timestampValue: '2027-02-30T00:00:00Z' } },
  { page: { stringValue: '33333333-3333-4333-8333-333333333333' } },
])('rejects unusable page hints without issuing any query: %j', async (change) => {
  const f = await fixture(true);
  f.read(async (target) =>
    target.resource === 'members'
      ? document(member, memberFields())
      : document(page, { ...pageFields(), ...change }),
  );
  await expect(f.admission.open(input, call())).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
  });
  expect(f.events).toEqual([
    ['read', member],
    ['read', page],
  ]);
});

it('refreshes the page routing hint without ever re-creating the same membership', async () => {
  const f = await fixture();
  await f.admission.open(input, call());
  f.read(async (target) =>
    target.resource === 'members'
      ? document(member, memberFields())
      : document(page, { ...pageFields(), epoch: { integerValue: '9' } }),
  );
  expect((await f.admission.open(input, call())).epoch).toBe('9');
  expect(f.events.filter((event) => (event as unknown[])[0] === 'publish')).toHaveLength(1);
});
