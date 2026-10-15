import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import type { SyncAdmissionCost } from '../../../../tmt-remote/typescript/remote-client/src/firebase-sync-http.js';
import * as math from '../../../../tmt-remote/typescript/remote-client/src/budget.js';
import { HostedBudget, type HostedBudgetStore, type HostedUsage } from '../src/hosted-budget.js';

const now = Date.parse('2026-10-15T12:00:00Z');
const today = math.pacificDay(now);
const listen = { operation: 'listen', reads: null, writes: 0, rulesLookupsPerDocument: 3 } as const;
const read = { operation: 'read', reads: 1, writes: 0, rulesLookupsPerDocument: 3 } as const;
function fixture(stored: unknown = undefined) {
  let usage = stored;
  let serial = Promise.resolve();
  const store: HostedBudgetStore = {
    transact(change) {
      const job = serial.then(() => {
        const update = change(usage);
        usage = update.usage;
        return update.result;
      });
      serial = job.then(
        () => {},
        () => {},
      );
      return job;
    },
  };
  let clock = now;
  const guard = new HostedBudget(math, store, () => clock);
  return {
    guard,
    store,
    usage: () => usage as HostedUsage,
    clock: (value: number) => {
      clock = value;
    },
  };
}

it('charges three complete 1024-document tails, then every coalesced delivery', async () => {
  const f = fixture();
  await Promise.all(Array.from({ length: 3 }, () => f.guard.admit(listen)));
  await Promise.all(Array.from({ length: 3 }, () => f.guard.received(1024)));
  await f.guard.received(428);
  expect(await f.guard.snapshot()).toEqual({
    day: today.day,
    reads: 3500,
    writes: 0,
    state: 'warn',
  });
});

it('precharges REST upper bounds plus Rules lookups and only the admission write', async () => {
  const f = fixture();
  await f.guard.admit(read);
  await f.guard.admit({ operation: 'query', reads: 256, writes: 0, rulesLookupsPerDocument: 3 });
  await f.guard.admit(commit('members'));
  expect(f.usage()).toEqual({ day: today.day, reads: 1032, writes: 1 });
});

it('refuses a REST reservation before the effect without spending it', async () => {
  const f = fixture({ day: today.day, reads: 4496, writes: 0 });
  await expect(f.guard.admit(read)).rejects.toEqual({
    code: 'REMOTE_BUDGET_EXHAUSTED',
    resetAtMs: today.resetAtMs,
    retryAfterMs: today.resetAtMs - now,
  });
  expect(f.usage().reads).toBe(4496);
});

it('records an already received crossing batch and refuses all fresh listeners', async () => {
  const f = fixture({ day: today.day, reads: 4400, writes: 0 });
  await expect(f.guard.received(1024)).rejects.toMatchObject({ code: 'REMOTE_BUDGET_EXHAUSTED' });
  expect(f.usage().reads).toBe(5424);
  await expect(f.guard.admit(listen)).rejects.toMatchObject({ code: 'REMOTE_BUDGET_EXHAUSTED' });
  expect(f.usage().reads).toBe(5424);
});

it('shares one origin counter across concurrently admitted tabs', async () => {
  const f = fixture({ day: today.day, reads: 4492, writes: 0 });
  const other = new HostedBudget(math, f.store, () => now);
  const attempts = await Promise.allSettled([f.guard.admit(read), other.admit(read)]);
  expect(attempts.map((result) => result.status)).toEqual(['fulfilled', 'rejected']);
  expect(f.usage().reads).toBe(4496);
});

it('resets only at the SDK Pacific day boundary, including DST', async () => {
  const f = fixture({ day: today.day, reads: 5000, writes: 1 });
  f.clock(today.resetAtMs - 1);
  await expect(f.guard.admit(listen)).rejects.toMatchObject({ code: 'REMOTE_BUDGET_EXHAUSTED' });
  f.clock(today.resetAtMs);
  await f.guard.admit(read);
  expect(f.usage()).toEqual({ day: today.day + 1, reads: 4, writes: 0 });
  expect(math.pacificDay(Date.parse('2026-11-01T08:30:00Z')).resetAtMs).toBe(
    Date.parse('2026-11-02T08:00:00Z'),
  );
});

it.each([
  null,
  {},
  { day: today.day, reads: -1, writes: 0 },
  { day: today.day + 1, reads: 0, writes: 0 },
])(
  'fails closed for corrupt or future counters instead of erasing evidence: %j',
  async (stored) => {
    const f = fixture(stored);
    await expect(f.guard.admit(listen)).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
    expect(f.usage()).toEqual(stored);
  },
);

it('fails closed if the durable origin transaction fails', async () => {
  const guard = new HostedBudget(
    math,
    {
      transact: async () => {
        throw new Error('private storage detail');
      },
    },
    () => now,
  );
  await expect(guard.received(1)).rejects.toEqual({ code: 'REMOTE_STATE_UNAVAILABLE' });
});

it('joins pending charges before the page owner releases its group', async () => {
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const f = fixture();
  const guard = new HostedBudget(
    math,
    {
      transact: async (change) => {
        await held;
        return f.store.transact(change);
      },
    },
    () => now,
  );
  const charge = guard.received(1024);
  let joined = false;
  const join = guard.drain().then(() => {
    joined = true;
  });
  await Promise.resolve();
  expect(joined).toBe(false);
  release();
  await Promise.all([charge, join]);
  expect(f.usage().reads).toBe(1024);
});

const commit = (
  resource: string,
  mode: 'create' | 'put' | 'delete' = 'create',
  target = 'fixture-document',
) =>
  ({
    operation: 'commit',
    reads: 0,
    writes: 1,
    rulesLookupsPerDocument: 4,
    mutation: { mode, resource, target },
  }) as const;

it('derives the 13-read own-log reservation and 52-read request from the independent native vector', async () => {
  const vectors = JSON.parse(
    readFileSync(
      new URL(
        '../../../../tmt-remote/rust/tmt-remote/tests/fixtures/firestore_budget/vectors.json',
        import.meta.url,
      ),
      'utf8',
    ),
  );
  const row = vectors.append.find(
    (value: { members: number; writers: number }) => value.members === 4 && value.writers === 4,
  );
  expect(row.readsPerAppend).toBe(13);
  expect(math.BudgetModel.create(4, 4).readsPerAppend()).toBe(row.readsPerAppend);
  const f = fixture();
  for (let index = 0; index < 4; index++)
    await f.guard.admit(commit('log', 'create', `log-${index}`));
  expect(f.usage()).toEqual({ day: today.day, reads: 4 * row.readsPerAppend, writes: 4 });
  expect(f.usage().reads).toBe(52);
  await f.guard.admit(read);
  expect(f.usage().reads).toBe(56);
});

it.each([
  undefined,
  { mode: 'put', resource: 'members', target: 'm' },
  { mode: 'delete', resource: 'log', target: 'l' },
  { mode: 'create', resource: 'checkpoints', target: 'c' },
  { mode: 'create', resource: 'authority', target: 'a' },
  { mode: 'create', resource: 'log', target: '' },
])(
  'refuses an unrecognized commit shape before spending the origin ledger: %j',
  async (mutation) => {
    const f = fixture();
    const cost = { ...commit('log'), mutation } as SyncAdmissionCost;
    await expect(f.guard.admit(cost)).rejects.toEqual({ code: 'REMOTE_SCOPE_DENIED' });
    expect(f.usage()).toBeUndefined();
  },
);

it('preserves the members-create cost and never treats it as an own-log reservation', async () => {
  const f = fixture();
  await f.guard.admit(commit('members'));
  expect(f.usage()).toEqual({ day: today.day, reads: 4, writes: 1 });
});

it('shares the refusal boundary between read and own-log reservations without counting a refused write', async () => {
  const f = fixture({ day: today.day, reads: 4483, writes: 20000 });
  await f.guard.admit(commit('log'));
  expect(f.usage()).toEqual({ day: today.day, reads: 4496, writes: 20001 });
  await expect(f.guard.admit(read)).rejects.toMatchObject({ code: 'REMOTE_BUDGET_EXHAUSTED' });
  await expect(f.guard.admit(commit('log'))).rejects.toMatchObject({
    code: 'REMOTE_BUDGET_EXHAUSTED',
  });
  expect(f.usage()).toEqual({ day: today.day, reads: 4496, writes: 20001 });
});

it('rejects mutation metadata on a read before touching the ledger', async () => {
  const f = fixture();
  await expect(f.guard.admit({ ...read, mutation: commit('log').mutation })).rejects.toEqual({
    code: 'REMOTE_SCOPE_DENIED',
  });
  expect(f.usage()).toBeUndefined();
});
