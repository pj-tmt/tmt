import { expect, test } from 'vite-plus/test';
import { syncHttp, type SyncAdmissionCost } from '../src/firebase-sync-http.js';
import type { SyncContext } from '../src/sealed-sync.js';

function fixture() {
  const attempts: { url: string; init: RequestInit }[] = [];
  const costs: SyncAdmissionCost[] = [];
  const identity = { projectId: 'demo-remote-1', uid: 'fixture-user' };
  const context = {
    config: {
      ...identity,
      apiKey: 'public-key',
      appId: '1:123:web:mine',
      authDomain: 'demo-remote-1.firebaseapp.com',
    },
    extension: 'notes',
    declarationDigest: 'a'.repeat(64),
    space: 'fixture-space',
    uid: identity.uid,
    resources: [],
    auth: {
      identity,
      async token() {
        return { ...identity, token: 'TOKEN_CANARY', expiresAtMs: Date.now() + 60000 };
      },
    },
  } satisfies SyncContext;
  const fetcher = (async (url: string | URL | Request, init?: RequestInit) => {
    attempts.push({ url: String(url), init: init! });
    return new Response('{}');
  }) as typeof fetch;
  const guard = {
    async admit(cost: SyncAdmissionCost) {
      costs.push(cost);
    },
  };
  const call = () => ({ signal: new AbortController().signal, deadlineAtMs: Date.now() + 1000 });
  return { attempts, costs, context, fetcher, guard, call };
}

test('REST uses only the bound default database and admits before acquiring credentials', async () => {
  const f = fixture();
  const order: string[] = [];
  const token = f.context.auth.token;
  f.context.auth.token = async () => {
    order.push('token');
    return token();
  };
  const http = syncHttp(
    f.context,
    {
      async admit(cost) {
        order.push('admit');
        f.costs.push(cost);
      },
    },
    f.fetcher,
    () => {},
  );
  await http('read', 'x/notes/rows/id+1', undefined, f.call(), 1);
  expect(order).toEqual(['admit', 'token']);
  expect(f.attempts[0]?.url).toBe(
    'https://firestore.googleapis.com/v1/projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id%2B1',
  );
  expect(f.attempts[0]?.init).toMatchObject({
    redirect: 'error',
    credentials: 'omit',
    cache: 'no-store',
    method: 'GET',
  });
  expect(f.costs[0]).toMatchObject({
    operation: 'read',
    reads: 1,
    rulesLookupsPerDocument: 3,
    writes: 0,
  });
});

test('budget refusal, wrong identity and expired token perform no provider request', async () => {
  for (const kind of ['budget', 'uid', 'project', 'expired'] as const) {
    const f = fixture();
    if (kind !== 'budget')
      f.context.auth.token = async () => ({
        projectId: kind === 'project' ? 'another-project' : 'demo-remote-1',
        uid: kind === 'uid' ? 'another-user' : 'fixture-user',
        token: 'TOKEN_CANARY',
        expiresAtMs: kind === 'expired' ? 0 : Date.now() + 60000,
      });
    const http = syncHttp(
      f.context,
      {
        async admit() {
          if (kind === 'budget')
            throw { code: 'REMOTE_BUDGET_EXHAUSTED', resetAtMs: 123, retryAfterMs: 10 };
        },
      },
      f.fetcher,
      () => {},
    );
    await expect(http('read', 'x/notes/rows/id', undefined, f.call(), 1)).rejects.toMatchObject({
      code: kind === 'budget' ? 'REMOTE_BUDGET_EXHAUSTED' : 'REMOTE_STATE_UNAVAILABLE',
    });
    expect(f.attempts).toHaveLength(0);
  }
});

test('possible commit failure is unknown and never retried, including cancellation', async () => {
  const f = fixture();
  const http = syncHttp(
    f.context,
    f.guard,
    (async (...args: Parameters<typeof fetch>) => {
      f.attempts.push({ url: String(args[0]), init: args[1]! });
      throw Error('TOKEN_CANARY');
    }) as typeof fetch,
    () => {},
  );
  expect(await http('commit', '', '{"writes":[]}', f.call(), 0)).toEqual({ kind: 'unknown' });
  expect(f.attempts).toHaveLength(1);
  expect(f.costs[0]).toMatchObject({ writes: 1, reads: 0 });
});

test('provider metadata counts in the page cap; oversized reads are refused', async () => {
  const f = fixture();
  const http = syncHttp(
    f.context,
    f.guard,
    (async () => new Response(' '.repeat(397313))) as typeof fetch,
    () => {},
  );
  await expect(http('query', '', '{}', f.call(), 1)).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
  });
});

test('an aborted in-flight commit returns unknown even when the fetcher ignores cancellation', async () => {
  const f = fixture();
  const controller = new AbortController();
  let entered!: () => void;
  const ready = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const http = syncHttp(
    f.context,
    f.guard,
    (async () => {
      entered();
      return new Promise<Response>(() => {});
    }) as typeof fetch,
    () => {},
  );
  const result = http(
    'commit',
    '',
    '{"writes":[]}',
    { signal: controller.signal, deadlineAtMs: Date.now() + 1000 },
    0,
  );
  await ready;
  controller.abort();
  expect(await result).toEqual({ kind: 'unknown' });
});

test('abort while waiting for a token refuses before the provider and suppresses its late token', async () => {
  const f = fixture();
  const controller = new AbortController();
  let entered!: () => void,
    finish!: (value: Awaited<ReturnType<typeof f.context.auth.token>>) => void;
  const ready = new Promise<void>((resolve) => {
    entered = resolve;
  });
  f.context.auth.token = () => {
    entered();
    return new Promise((resolve) => {
      finish = resolve;
    });
  };
  const http = syncHttp(f.context, f.guard, f.fetcher, () => {});
  const result = http(
    'read',
    'x/notes/rows/id',
    undefined,
    { signal: controller.signal, deadlineAtMs: Date.now() + 1000 },
    1,
  );
  const rejected = expect(result).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
  await ready;
  controller.abort();
  await rejected;
  finish({
    projectId: 'demo-remote-1',
    uid: 'fixture-user',
    token: 'TOKEN_CANARY',
    expiresAtMs: Date.now() + 60000,
  });
  expect(f.attempts).toHaveLength(0);
});

test('oversized REST input refuses before guard, credentials or provider', async () => {
  const f = fixture();
  const http = syncHttp(f.context, f.guard, f.fetcher, () => {});
  await expect(http('query', 'x/notes', 'x'.repeat(397313), f.call(), 1)).rejects.toMatchObject({
    code: 'REMOTE_INPUT_INVALID',
  });
  expect(f.costs).toHaveLength(0);
  expect(f.attempts).toHaveLength(0);
});
