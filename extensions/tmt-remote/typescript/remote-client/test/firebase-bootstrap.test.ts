import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
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
const descriptor =
  '{"checksum":"42deaf4f32ad7a7986ef7963b589cf88a0c1ac6158f75730f5c2d098f7da9dcf","kind":"firestore","publicWebConfig":{"apiKey":"public-key","appId":"1:123:web:mine","authDomain":"demo-remote-1.firebaseapp.com","projectId":"demo-remote-1"},"version":1}';
const declarationBytes = JSON.stringify({
  version: 1,
  extension: 'fixture',
  backend: 'firestore',
  resources: [],
  admission: { artifact: 'admission.rules', digest: '0'.repeat(64), entryPoint: 'admitted' },
});
const declaration = {
  bytes: declarationBytes,
  digest: Array.from(
    new Uint8Array(
      await crypto.subtle.digest('SHA-256', new TextEncoder().encode(declarationBytes)),
    ),
    (b) => b.toString(16).padStart(2, '0'),
  ).join(''),
};
const origin = 'https://demo-remote-1.web.app';
const link = `${origin}/pair#${base64url(new TextEncoder().encode(descriptor))}`;
const reply = (value: unknown = config) => new Response(JSON.stringify(value));
function fake() {
  const calls: string[] = [];
  const port: FirebaseAuthPort = {
    async signInAnonymous() {
      calls.push('sign-in');
      return 'anonymous-profile-uid';
    },
    async signOut() {
      calls.push('sign-out');
    },
    async close() {
      calls.push('close');
    },
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
      async load() {
        calls.push('load');
        return {
          async initialize(value: unknown) {
            assert.deepEqual(value, config);
            calls.push('initialize');
            return port;
          },
        };
      },
    },
  };
}

test('hosted bootstrap verifies the carrier and same-origin config before loading Auth', async () => {
  const f = fake();
  const remote = await connectHosted(link, declaration, f.deps);
  assert.deepEqual(f.calls, [`${origin}/__/firebase/init.json`, 'load', 'initialize']);
  assert.equal(await remote.signInAnonymous(), 'anonymous-profile-uid');
  await remote.close();
  assert.equal(f.calls.at(-1), 'close');
});

test('config disagreement refuses before loading Firebase, never selects another project', async () => {
  for (const key of ['projectId', 'appId', 'authDomain'] as const) {
    const f = fake();
    f.deps.fetch = async () => reply({ ...config, [key]: 'another-project' });
    await assert.rejects(
      connectHosted(link, declaration, f.deps),
      (e: unknown) => e instanceof ClientError && e.code === 'unverifiable_response',
    );
    assert.deepEqual(f.calls, []);
  }
});

test('closed schema, canonical carrier and corruption checksum are required', async () => {
  for (const bytes of [
    descriptor.replace('public-key', 'changed'),
    descriptor + ' ',
    descriptor.replace('"version":1', '"version":1,"version":1'),
    descriptor.replace('"version":1', '"version":1,"token":"TOKEN_CANARY"'),
  ]) {
    await assert.rejects(
      verifyHostedLink(`${origin}/pair#${base64url(new TextEncoder().encode(bytes))}`, origin),
    );
  }
  await assert.rejects(verifyHostedLink(link + '=', origin));
  await assert.rejects(verifyHostedLink(link, 'http://127.0.0.1:1234'));
  await assert.rejects(verifyHostedLink('x'.repeat(8193), origin));
});

test('missing lazy asset and disabled anonymous provider are fixed typed refusals', async () => {
  const f = fake();
  f.deps.load = async () => {
    throw new Error('TOKEN_CANARY');
  };
  await assert.rejects(
    connectHosted(link, declaration, f.deps),
    (e: unknown) =>
      e instanceof RefusalError &&
      e.code === 'REMOTE_STATE_UNAVAILABLE' &&
      !e.message.includes('TOKEN_CANARY'),
  );
  const g = fake();
  g.port.signInAnonymous = async () => {
    throw { code: 'auth/operation-not-allowed', message: 'TOKEN_CANARY' };
  };
  const remote = await connectHosted(link, declaration, g.deps);
  await assert.rejects(
    remote.signInAnonymous(),
    (e: unknown) =>
      e instanceof RefusalError &&
      e.code === 'REMOTE_SCOPE_DENIED' &&
      e.scope === undefined &&
      !e.message.includes('TOKEN_CANARY'),
  );
  await remote.close();
});

test('anonymous requests single-flight and a closed generation never publishes a late uid', async () => {
  const f = fake();
  let finish!: (uid: string) => void;
  f.port.signInAnonymous = () => {
    f.calls.push('pending');
    return new Promise((resolve) => {
      finish = resolve;
    });
  };
  const remote = await connectHosted(link, declaration, f.deps);
  const a = remote.signInAnonymous();
  const b = remote.signInAnonymous();
  assert.equal(f.calls.filter((c) => c === 'pending').length, 1);
  await remote.close();
  finish('late-uid');
  for (const pending of [a, b])
    await assert.rejects(
      pending,
      (e: unknown) => e instanceof RefusalError && e.code === 'REMOTE_CLOSED',
    );
  assert.equal(f.calls.filter((c) => c === 'close').length, 1);
  await remote.close();
});

test('declaration digest mismatch refuses before config fetch or provider loading', async () => {
  const f = fake();
  await assert.rejects(
    connectHosted(link, { ...declaration, digest: '0'.repeat(64) }, f.deps),
    (error: unknown) => error instanceof RefusalError && error.code === 'REMOTE_INPUT_INVALID',
  );
  assert.deepEqual(f.calls, []);
});

test('declared collections are data: arbitrary valid resources work, reserved paths refuse', async () => {
  for (const path of [
    'notes',
    'other-rows',
    'nested/rows',
    'append',
    'subscribe',
    'ack',
    'pair',
    'x',
    '../escape',
  ]) {
    const f = fake();
    const parsed = JSON.parse(declaration.bytes);
    parsed.resources = [
      {
        name: 'rows',
        kind: 'checkpoint',
        path,
        limits: { maxObjectBytes: 100, maxNamespaceBytes: 1000, maxEntries: 10 },
        ttlField: null,
        indexes: [],
      },
    ];
    const bytes = JSON.stringify(parsed);
    const digest = Array.from(
      new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(bytes))),
      (b) => b.toString(16).padStart(2, '0'),
    ).join('');
    const attempt = connectHosted(link, { bytes, digest }, f.deps);
    if (['notes', 'other-rows', 'nested/rows'].includes(path)) await (await attempt).close();
    else {
      await assert.rejects(attempt);
      assert.deepEqual(f.calls, []);
    }
  }
});

test('timed-out sign-in never publishes a late uid', async () => {
  vi.useFakeTimers();
  try {
    const f = fake();
    const remote = await connectHosted(link, declaration, f.deps);
    let finish!: (uid: string) => void;
    f.port.signInAnonymous = () =>
      new Promise((resolve) => {
        finish = resolve;
      });
    const pending = remote.signInAnonymous();
    const rejected = assert.rejects(
      pending,
      (error: unknown) => error instanceof ClientError && error.code === 'timeout',
    );
    await vi.advanceTimersByTimeAsync(10_001);
    await rejected;
    finish('late-uid');
    await remote.close();
    assert.equal(f.calls.filter((c) => c === 'close').length, 1);
  } finally {
    vi.useRealTimers();
  }
});

test('resource kinds and index directions reject array coercion before provider access', async () => {
  for (const variant of ['kind', 'direction']) {
    const f = fake();
    const parsed = JSON.parse(declaration.bytes);
    parsed.resources = [
      {
        name: 'rows',
        kind: variant === 'kind' ? ['checkpoint'] : 'checkpoint',
        path: 'rows',
        limits: { maxObjectBytes: 100, maxNamespaceBytes: 1000, maxEntries: 10 },
        ttlField: null,
        indexes: variant === 'direction' ? [{ field: 'sequence', direction: ['asc'] }] : [],
      },
    ];
    const bytes = JSON.stringify(parsed);
    const digest = Array.from(
      new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(bytes))),
      (b) => b.toString(16).padStart(2, '0'),
    ).join('');
    await assert.rejects(connectHosted(link, { bytes, digest }, f.deps));
    assert.deepEqual(f.calls, []);
  }
});
