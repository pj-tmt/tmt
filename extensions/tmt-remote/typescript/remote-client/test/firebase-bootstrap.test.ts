import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test, vi } from 'vite-plus/test';
import { base64url } from '../src/canonical-bytes.js';
import { originLink, verifyHostedLink } from '../src/firebase-bootstrap.js';
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

test('origin bootstrap produces the independent canonical link and still runs every connect check', async () => {
  const calls: string[] = [];
  const fetcher = (async (url, options) => {
    calls.push(String(url));
    assert.equal(options?.method, undefined);
    assert.equal(options?.redirect, 'error');
    assert.equal(options?.credentials, 'omit');
    assert.equal(options?.cache, 'no-store');
    assert.ok(options?.signal);
    return reply({
      ...config,
      messagingSenderId: '123',
      databaseURL: 'https://PUBLIC_OTHER.example',
      extra: { token: 'EXTRA_CANARY' },
    });
  }) as typeof fetch;
  const derived = await originLink(origin, fetcher);
  assert.equal(derived, link);
  assert.deepEqual(await verifyHostedLink(derived, origin), config);
  assert.deepEqual(calls, [`${origin}/__/firebase/init.json`]);
  const f = fake();
  await (await connectHosted(derived, declaration, f.deps)).close();
  assert.deepEqual(f.calls, [`${origin}/__/firebase/init.json`, 'load', 'initialize', 'close']);
});

test('origin bootstrap refuses invalid public config and mismatched origins before any Firebase load', async () => {
  for (const [at, value] of [
    [origin, { ...config, apiKey: null }],
    [origin, { ...config, projectId: 'other-project' }],
    [origin, { ...config, authDomain: 'other-project.firebaseapp.com' }],
    [origin, { ...config, appId: ['1:123:web:mine'] }],
    [origin, []],
    ['https://other-project.web.app', config],
    ['http://demo-remote-1.web.app', config],
    ['https://demo-remote-1.firebaseapp.com', config],
  ] as const) {
    await assert.rejects(
      originLink(at, async () => reply(value)),
      (error: unknown) => error instanceof RefusalError && error.code === 'REMOTE_INPUT_INVALID',
    );
  }
});

test('origin bootstrap refuses failed, redirected, malformed or oversized fetches with one fixed error', async () => {
  const redirected = reply();
  Object.defineProperty(redirected, 'url', {
    value: 'https://other-project.web.app/__/firebase/init.json',
  });
  for (const fetcher of [
    async () => {
      throw new Error('PRIVATE_FETCH_CANARY');
    },
    async () => new Response('{}', { status: 503 }),
    async () => redirected,
    async () => new Response('{'),
    async () => new Response(' '.repeat(8193)),
  ]) {
    await assert.rejects(
      originLink(origin, fetcher),
      (error: unknown) =>
        error instanceof RefusalError &&
        error.code === 'REMOTE_INPUT_INVALID' &&
        !error.message.includes('CANARY'),
    );
  }
});

test('origin bootstrap bounds a fetch that ignores cancellation', async () => {
  vi.useFakeTimers();
  try {
    let signal: AbortSignal | undefined;
    const pending = originLink(origin, (async (_url, options) => {
      signal = options?.signal as AbortSignal;
      return new Promise<Response>(() => {});
    }) as typeof fetch);
    const refused = assert.rejects(
      pending,
      (error: unknown) => error instanceof RefusalError && error.code === 'REMOTE_INPUT_INVALID',
    );
    await vi.advanceTimersByTimeAsync(10_001);
    await refused;
    assert.equal(signal?.aborted, true);
  } finally {
    vi.useRealTimers();
  }
});
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

test('sealed sync requires an established anonymous identity and never signs in implicitly', async () => {
  const f = fake();
  const remote = await connectHosted(link, declaration, f.deps);
  const factory = Reflect.get(remote, 'sealedSync') as
    | ((space: string, guard: { admit(cost: unknown): Promise<void> }) => Promise<unknown>)
    | undefined;
  assert.equal(typeof factory, 'function', 'the verified hosted Auth owner supplies sealedSync');
  await assert.rejects(
    factory!('space-1', { async admit() {} }),
    (error: unknown) =>
      typeof error === 'object' &&
      error !== null &&
      'code' in error &&
      error.code === 'REMOTE_STATE_UNAVAILABLE',
  );
  assert.equal(f.calls.includes('sign-in'), false);
  await remote.close();
});

test('sealed sync factory binds the verified declaration and closes its generation on sign-out', async () => {
  const f = fake();
  f.port.syncAuth = (uid) => ({
    identity: { projectId: config.projectId, uid },
    async token() {
      throw Error('no I/O during binding');
    },
  });
  f.port.wake = {
    async listen() {
      throw Error('no listener during binding');
    },
  };
  const auth = await connectHosted(link, declaration, f.deps);
  await auth.signInAnonymous();
  const sync = await auth.sealedSync('space-1', { async admit() {} });
  assert.ok(Object.isFrozen(sync));
  assert.ok(Object.isFrozen(sync.context));
  assert.equal(sync.context.declarationDigest, declaration.digest);
  assert.equal(sync.context.uid, 'anonymous-profile-uid');
  assert.equal(sync.context.config.projectId, config.projectId);
  await auth.signOut();
  await assert.rejects(
    auth.sealedSync('space-1', { async admit() {} }),
    (error: unknown) => error instanceof RefusalError && error.code === 'REMOTE_STATE_UNAVAILABLE',
  );
  await auth.close();
});

test('the page interest cap is shared across hosted Auth owners, not reset by another factory', async () => {
  const f = fake();
  f.port.syncAuth = (uid) => ({
    identity: { projectId: config.projectId, uid },
    async token() {
      return {
        projectId: config.projectId,
        uid,
        token: 'TOKEN_CANARY',
        expiresAtMs: Date.now() + 60000,
      };
    },
  });
  let started = 0;
  f.port.wake = {
    async listen() {
      started++;
      return () => {};
    },
  };
  const vector = JSON.parse(
    readFileSync(
      new URL('../../../rust/tmt-remote/tests/fixtures/sealed_sync/codec-v1.json', import.meta.url),
      'utf8',
    ),
  ) as { declaration: string; declarationDigest: string };
  const supplied = { bytes: vector.declaration, digest: vector.declarationDigest };
  const a = await connectHosted(link, supplied, f.deps),
    b = await connectHosted(link, supplied, f.deps);
  await a.signInAnonymous();
  await b.signInAnonymous();
  const guard = { async admit() {} };
  const one = await a.sealedSync('fixture-space', guard),
    two = await b.sealedSync('fixture-space', guard);
  const interest = { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } };
  const call = () => ({ signal: new AbortController().signal, deadlineAtMs: Date.now() + 10000 });
  await one.subscribe(interest, call());
  await two.subscribe(interest, call());
  await two.subscribe(interest, call());
  await assert.rejects(
    two.subscribe(interest, call()),
    (error: unknown) =>
      typeof error === 'object' &&
      error !== null &&
      'code' in error &&
      error.code === 'REMOTE_RATE_LIMITED',
  );
  assert.equal(started, 3);
  await a.close();
  await two.subscribe(interest, call());
  assert.equal(started, 4);
  await b.close();
});
