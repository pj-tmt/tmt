import assert from 'node:assert/strict';
import { test } from 'vite-plus/test';
import { base64url } from '../src/canonical-bytes.js';
import { verifyHostedLink } from '../src/firebase-bootstrap.js';
import { connectHosted, type FirebaseAuthPort } from '../src/firebase-auth.js';
import { ClientError, RefusalError } from '../src/operations.js';

// Independent public carrier from the Remote link contract; no credentials or grant.
const config = {
  apiKey: 'public-key',
  appId: '1:123:web:mine',
  authDomain: 'demo-remote-1.firebaseapp.com',
  projectId: 'demo-remote-1',
};
const descriptor = '{"checksum":"42deaf4f32ad7a7986ef7963b589cf88a0c1ac6158f75730f5c2d098f7da9dcf","kind":"firestore","publicWebConfig":{"apiKey":"public-key","appId":"1:123:web:mine","authDomain":"demo-remote-1.firebaseapp.com","projectId":"demo-remote-1"},"version":1}';
const origin = 'https://demo-remote-1.web.app';
const link = `${origin}/pair#${base64url(new TextEncoder().encode(descriptor))}`;
const reply = (value: unknown = config) => new Response(JSON.stringify(value));
function fake() {
  const calls: string[] = [];
  const port: FirebaseAuthPort = {
    async signInAnonymous() { calls.push('sign-in'); return 'anonymous-profile-uid'; },
    async signOut() { calls.push('sign-out'); },
    async close() { calls.push('close'); },
  };
  return {
    calls,
    port,
    deps: {
      origin,
      fetch: (async (url: string | URL | Request, options?: RequestInit) => {
        calls.push(String(url));
        assert.equal(options?.redirect, 'error');
        assert.equal(options?.credentials, 'omit');
        return reply();
      }) as typeof fetch,
      async load() { calls.push('load'); return { async initialize(value: unknown) {
        assert.deepEqual(value, config); calls.push('initialize'); return port;
      } }; },
    },
  };
}

test('hosted bootstrap verifies the carrier and same-origin config before loading Auth', async () => {
  const f = fake();
  const remote = await connectHosted(link, f.deps);
  assert.deepEqual(f.calls, [`${origin}/__/firebase/init.json`, 'load', 'initialize']);
  assert.equal(await remote.signInAnonymous(), 'anonymous-profile-uid');
  await remote.close();
  assert.equal(f.calls.at(-1), 'close');
});

test('config disagreement refuses before loading Firebase, never selects another project', async () => {
  for (const key of ['projectId', 'appId', 'authDomain'] as const) {
    const f = fake();
    f.deps.fetch = async () => reply({ ...config, [key]: 'another-project' });
    await assert.rejects(connectHosted(link, f.deps), (e: unknown) =>
      e instanceof ClientError && e.code === 'unverifiable_response');
    assert.deepEqual(f.calls, []);
  }
});

test('closed schema, canonical carrier and corruption checksum are required', async () => {
  for (const bytes of [descriptor.replace('public-key', 'changed'), descriptor + ' ',
    descriptor.replace('"version":1', '"version":1,"version":1'),
    descriptor.replace('"version":1', '"version":1,"token":"TOKEN_CANARY"')]) {
    await assert.rejects(verifyHostedLink(`${origin}/pair#${base64url(new TextEncoder().encode(bytes))}`, origin));
  }
  await assert.rejects(verifyHostedLink(link + '=', origin));
  await assert.rejects(verifyHostedLink(link, 'http://127.0.0.1:1234'));
  await assert.rejects(verifyHostedLink('x'.repeat(8193), origin));
});

test('missing lazy asset and disabled anonymous provider are fixed typed refusals', async () => {
  const f = fake();
  f.deps.load = async () => { throw new Error('TOKEN_CANARY'); };
  await assert.rejects(connectHosted(link, f.deps), (e: unknown) =>
    e instanceof RefusalError && e.code === 'REMOTE_STATE_UNAVAILABLE' && !e.message.includes('TOKEN_CANARY'));
  const g = fake();
  g.port.signInAnonymous = async () => { throw { code: 'auth/operation-not-allowed', message: 'TOKEN_CANARY' }; };
  const remote = await connectHosted(link, g.deps);
  await assert.rejects(remote.signInAnonymous(), (e: unknown) =>
    e instanceof RefusalError && e.code === 'REMOTE_SCOPE_DENIED' && e.scope === undefined && !e.message.includes('TOKEN_CANARY'));
  await remote.close();
});

test('anonymous requests single-flight and a closed generation never publishes a late uid', async () => {
  const f = fake();
  let finish!: (uid: string) => void;
  f.port.signInAnonymous = () => { f.calls.push('pending'); return new Promise(resolve => { finish = resolve; }); };
  const remote = await connectHosted(link, f.deps);
  const a = remote.signInAnonymous();
  const b = remote.signInAnonymous();
  assert.equal(f.calls.filter(c => c === 'pending').length, 1);
  await remote.close();
  finish('late-uid');
  for (const pending of [a, b]) await assert.rejects(pending, (e: unknown) => e instanceof RefusalError && e.code === 'REMOTE_CLOSED');
  assert.equal(f.calls.filter(c => c === 'close').length, 1);
  await remote.close();
});
