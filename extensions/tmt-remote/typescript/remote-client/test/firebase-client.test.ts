import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
const calls = vi.hoisted(() => ({
  apps: 0,
  removed: 0,
  auth: [] as unknown[],
  signs: 0,
  state: {
    currentUser: undefined as
      | undefined
      | {
          uid: string;
          isAnonymous: boolean;
          getIdTokenResult(): Promise<{
            token: string;
            claims: { aud: string; sub: string };
            signInProvider: string;
            expirationTime: string;
          }>;
        },
  },
  firestores: 0,
  terminated: 0,
  cache: [] as unknown[],
  filters: [] as unknown[],
  queries: [] as { collection: unknown; constraints: unknown[] }[],
  listens: 0,
  detached: 0,
  changed: undefined as
    | undefined
    | ((snapshot: { metadata: { fromCache: boolean }; docChanges(): unknown[] }) => void),
}));
vi.mock('firebase/app', () => ({
  initializeApp: () => {
    calls.apps++;
    return {};
  },
  deleteApp: async () => {
    calls.removed++;
  },
}));
vi.mock('firebase/auth', () => ({
  browserLocalPersistence: 'local',
  initializeAuth: (_app: unknown, options: unknown) => {
    calls.auth.push(options);
    calls.state = { currentUser: undefined };
    return calls.state;
  },
  signInAnonymously: async () => {
    calls.signs++;
    return { user: { uid: 'fixture-uid' } };
  },
  signOut: async () => {
    calls.state.currentUser = undefined;
  },
}));
vi.mock('firebase/firestore', () => ({
  initializeFirestore: (_app: unknown, options: unknown) => {
    calls.firestores++;
    calls.cache.push(options);
    return {};
  },
  memoryLocalCache: () => 'memory-only',
  collection: (_db: unknown, path: string) => path,
  where: (field: string, op: string, value: unknown) => {
    calls.filters.push([field, op, value]);
    return [field, op, value];
  },
  query: (collection: unknown, ...constraints: unknown[]) => {
    const built = { collection, constraints };
    calls.queries.push(built);
    return built;
  },
  onSnapshot: (_query: unknown, options: unknown, changed: typeof calls.changed) => {
    assert.deepEqual(options, { includeMetadataChanges: false });
    calls.listens++;
    calls.changed = changed;
    return () => {
      calls.detached++;
    };
  },
  terminate: async () => {
    calls.terminated++;
  },
  Timestamp: class {
    constructor(
      readonly seconds: number,
      readonly nanoseconds: number,
    ) {}
  },
  Bytes: { fromBase64String: (v: string) => v },
}));
import type { SyncContext } from '../src/sealed-sync.js';
import { initialize } from '../src/firebase-client.js';
test('one shared Firebase App/Auth uses local persistence without popup resolver and closes on last lease', async () => {
  const config = {
    apiKey: 'public',
    projectId: 'demo-fixture',
    authDomain: 'demo-fixture.firebaseapp.com',
    appId: 'fixture-app',
  };
  const a = await initialize(config);
  const b = await initialize(config);
  assert.equal(calls.apps, 1);
  assert.equal(calls.firestores, 1, 'the shared owner initializes its one memory-only client');
  assert.equal(calls.listens, 0, 'creating the shared client starts no listener');
  assert.deepEqual(calls.cache, [{ localCache: 'memory-only' }]);
  assert.deepEqual(calls.auth, [{ persistence: 'local' }]);
  assert.deepEqual(await Promise.all([a.signInAnonymous(), b.signInAnonymous()]), [
    'fixture-uid',
    'fixture-uid',
  ]);
  assert.equal(calls.signs, 1);
  await a.close();
  assert.equal(calls.removed, 0);
  await b.close();
  assert.equal(calls.removed, 1);
  await b.close();
  assert.equal(calls.removed, 1);
});

test('shared listener uses one memory cache and exact page filters, never reads snapshot data', async () => {
  const config = {
    apiKey: 'public',
    projectId: 'demo-listener',
    authDomain: 'demo-listener.firebaseapp.com',
    appId: 'listener-app',
  };
  const a = await initialize(config),
    b = await initialize(config);
  const user = {
    uid: 'fixture-uid',
    isAnonymous: true,
    async getIdTokenResult() {
      return {
        token: 'TOKEN_CANARY',
        claims: { aud: config.projectId, sub: 'fixture-uid' },
        signInProvider: 'anonymous',
        expirationTime: new Date(Date.now() + 60000).toISOString(),
      };
    },
  };
  calls.state.currentUser = user;
  const context = {
    config,
    uid: user.uid,
    extension: 'notes',
    declarationDigest: 'a'.repeat(64),
    space: 'space-1',
    auth: a.syncAuth!(user.uid),
    resources: [{ name: 'rows', path: 'rows', maxObjectBytes: 100 }],
  } satisfies SyncContext;
  const interest = {
    resource: 'rows',
    scope: {
      space: { stringValue: 'space-1' },
      page: { stringValue: 'page-1' },
      epoch: { integerValue: '1' },
    },
  };
  let changed = 0,
    failed = 0;
  const before = calls.firestores;
  const stop = await a.wake!.listen(
    context,
    interest,
    () => {
      changed++;
    },
    () => {
      failed++;
    },
  );
  const stop2 = await b.wake!.listen(
    context,
    interest,
    () => {},
    () => {},
  );
  assert.equal(calls.firestores, before);
  assert.deepEqual(calls.cache.at(-1), { localCache: 'memory-only' });
  assert.deepEqual(calls.filters.slice(-3), [
    ['space', '==', 'space-1'],
    ['page', '==', 'page-1'],
    ['epoch', '==', 1],
  ]);
  // The actual Web SDK listener builder is equality-only: index merging, no seq/order/range.
  assert.deepEqual(calls.queries.at(-1), {
    collection: 'x/notes/rows',
    constraints: calls.filters.slice(-3),
  });
  // Re-register to drive the observable callback used by the first owner.
  stop2();
  const stop3 = await a.wake!.listen(
    context,
    interest,
    () => {
      changed++;
    },
    () => {
      failed++;
    },
  );
  calls.changed!({ metadata: { fromCache: true }, docChanges: () => [{}] });
  assert.equal(changed, 0);
  calls.changed!({ metadata: { fromCache: false }, docChanges: () => [] });
  assert.equal(changed, 1);
  calls.changed!({ metadata: { fromCache: false }, docChanges: () => [] });
  assert.equal(changed, 1);
  calls.changed!({ metadata: { fromCache: false }, docChanges: () => [{}] });
  assert.equal(changed, 2);
  await b.signOut();
  calls.changed!({ metadata: { fromCache: false }, docChanges: () => [{}] });
  assert.equal(changed, 2);
  assert.equal(failed, 1);
  stop();
  stop3();
  await a.close();
  await b.close();
});

test('token bridge checks anonymous provider, aud, sub and expiry without minting an identity', async () => {
  const config = {
    apiKey: 'public',
    projectId: 'demo-token',
    authDomain: 'demo-token.firebaseapp.com',
    appId: 'token-app',
  };
  const a = await initialize(config);
  const count = calls.signs;
  const auth = a.syncAuth!('fixture-uid');
  const call = () => ({ signal: new AbortController().signal, deadlineAtMs: Date.now() + 1000 });
  assert.deepEqual(await auth.token(call()), {
    code: 'REMOTE_STATE_UNAVAILABLE',
    state: 'sign-in-needed',
  });
  for (const kind of ['valid', 'aud', 'sub', 'provider', 'expired'] as const) {
    calls.state.currentUser = {
      uid: 'fixture-uid',
      isAnonymous: true,
      async getIdTokenResult() {
        return {
          token: 'TOKEN_CANARY',
          claims: {
            aud: kind === 'aud' ? 'other-project' : config.projectId,
            sub: kind === 'sub' ? 'other-user' : 'fixture-uid',
          },
          signInProvider: kind === 'provider' ? 'google.com' : 'anonymous',
          expirationTime: new Date(kind === 'expired' ? 0 : Date.now() + 60000).toISOString(),
        };
      },
    };
    const answer = await auth.token(call());
    if (kind === 'valid') assert.ok('token' in answer);
    else assert.deepEqual(answer, { code: 'REMOTE_STATE_UNAVAILABLE', state: 'sign-in-needed' });
  }
  assert.equal(calls.signs, count);
  await a.close();
});
