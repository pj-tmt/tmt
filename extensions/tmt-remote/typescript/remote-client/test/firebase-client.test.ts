import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
const calls = vi.hoisted(() => ({ apps: 0, removed: 0, auth: [] as unknown[], signs: 0 }));
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
    return {};
  },
  signInAnonymously: async () => {
    calls.signs++;
    return { user: { uid: 'fixture-uid' } };
  },
  signOut: async () => {},
}));
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
