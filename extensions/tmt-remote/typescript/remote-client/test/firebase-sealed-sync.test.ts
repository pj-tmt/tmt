import { readFileSync } from 'node:fs';
import { expect, test, beforeEach, afterEach, vi } from 'vite-plus/test';
import { bindSyncContext, type SyncContext, type SyncCall } from '../src/sealed-sync.js';
import { createSealedSync, type SyncWakePort } from '../src/firebase-sealed-sync.js';
import type { SyncAdmissionCost } from '../src/firebase-sync-http.js';

// Contract deadlines use a controlled clock; compiler/crypto contention is not elapsed fixture time.
beforeEach(() => vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'] }));
afterEach(() => vi.useRealTimers());

// Remote-authored codec evidence, never a Colab declaration source.
const vector = JSON.parse(
  readFileSync(
    new URL('../../../rust/tmt-remote/tests/fixtures/sealed_sync/codec-v1.json', import.meta.url),
    'utf8',
  ),
) as { declaration: string; declarationDigest: string; document: string; documentDigest: string };
const config = {
  projectId: 'demo-remote-1',
  apiKey: 'public-key',
  appId: '1:123:web:mine',
  authDomain: 'demo-remote-1.firebaseapp.com',
};
const call = (): SyncCall => ({
  signal: new AbortController().signal,
  deadlineAtMs: Date.now() + 1000,
});
async function fixture(modes?: readonly string[]) {
  const requests: { url: string; init: RequestInit }[] = [];
  const costs: SyncAdmissionCost[] = [];
  let next: unknown = {
    name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
    fields: JSON.parse(vector.document),
  };
  let changed!: () => void, failed!: () => void;
  let listenerCount = 0,
    detached = 0;
  const declaration = JSON.parse(vector.declaration);
  if (modes) {
    declaration.resources[0].kind = 'checkpoint';
    declaration.resources[0].sync.writeModes = modes;
  }
  const declarationBytes = modes ? JSON.stringify(declaration) : vector.declaration;
  const declarationDigest = modes
    ? Array.from(
        new Uint8Array(
          await crypto.subtle.digest('SHA-256', new TextEncoder().encode(declarationBytes)),
        ),
        (b) => b.toString(16).padStart(2, '0'),
      ).join('')
    : vector.declarationDigest;
  const context: SyncContext = await bindSyncContext(
    config,
    { bytes: declarationBytes, digest: declarationDigest },
    'fixture-space',
    {
      identity: { projectId: config.projectId, uid: 'fixture-user' },
      async token() {
        return {
          projectId: config.projectId,
          uid: 'fixture-user',
          token: 'TOKEN_CANARY',
          expiresAtMs: Date.now() + 60000,
        };
      },
    },
  );
  const wake: SyncWakePort = {
    async listen(_context, interest, onChange, onFailure) {
      expect(interest).toEqual({
        resource: 'rows',
        scope: { tenant: { stringValue: 'fixture-space' } },
      });
      listenerCount++;
      changed = onChange;
      failed = onFailure;
      return () => {
        detached++;
      };
    },
  };
  const port = createSealedSync(
    context,
    {
      async admit(cost) {
        costs.push(cost);
      },
    },
    (async (url: string | URL | Request, init?: RequestInit) => {
      requests.push({ url: String(url), init: init! });
      const value = typeof next === 'function' ? next(String(url)) : { status: 200, body: next };
      return new Response(JSON.stringify(value.body), { status: value.status });
    }) as typeof fetch,
    wake,
    () => {},
  );
  return {
    port,
    requests,
    costs,
    context,
    setResponse(value: unknown) {
      next = value;
    },
    changed() {
      changed();
    },
    failed() {
      failed();
    },
    listenerCount: () => listenerCount,
    detached: () => detached,
  };
}

test('REST read validates the declared collection, canonical fields, space and full digest', async () => {
  const f = await fixture();
  const d = await f.port.read({ resource: 'rows', id: 'id+1' }, call());
  expect(d?.digest).toBe(vector.documentDigest);
  expect(new TextDecoder().decode(d?.bytes)).toBe(vector.document);
  f.setResponse({
    name: 'projects/other/databases/(default)/documents/x/notes/rows/id+1',
    fields: JSON.parse(vector.document),
  });
  await expect(f.port.read({ resource: 'rows', id: 'id+1' }, call())).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
  });
  f.port.close();
});

test('stream query names one writer and refuses a gap instead of inventing progress', async () => {
  const f = await fixture();
  const response = {
    document: {
      name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
      fields: JSON.parse(vector.document),
    },
  };
  f.setResponse([response]);
  const q = {
    shape: 'stream' as const,
    query: {
      resource: 'rows',
      scope: {
        tenant: { stringValue: 'fixture-space' },
        streamKey: { stringValue: 'writer-stream' },
      },
      stream: 'writer-stream',
    },
  };
  expect(await f.port.query(q, call())).toMatchObject({
    shape: 'stream',
    page: { cursor: { stream: 'writer-stream', seq: 1 } },
  });
  expect(f.requests[0]?.url).toBe(
    'https://firestore.googleapis.com/v1/projects/demo-remote-1/databases/(default)/documents/x/notes:runQuery',
  );
  response.document.fields.counter = { integerValue: '3' };
  f.setResponse([response]);
  await expect(f.port.query(q, call())).rejects.toMatchObject({
    code: 'REMOTE_STATE_UNAVAILABLE',
    state: 'catch-up-required',
  });
  f.port.close();
});

test('discovery paginates one bounded page and refuses a changed binding before I/O', async () => {
  const f = await fixture();
  f.setResponse([
    {
      document: {
        name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
        fields: JSON.parse(vector.document),
      },
    },
  ]);
  const q = {
    shape: 'discovery' as const,
    query: {
      resource: 'rows',
      shape: 'arrivals',
      scope: { tenant: { stringValue: 'fixture-space' } },
    },
  };
  const first = await f.port.query(q, call());
  expect(first.shape).toBe('discovery');
  if (first.shape !== 'discovery') throw Error('wrong page');
  expect(first.page.continuation?.length).toBeLessThanOrEqual(1024);
  f.setResponse([]);
  const second = await f.port.query(
    { ...q, query: { ...q.query, continuation: first.page.continuation } },
    call(),
  );
  expect(second).toEqual({ shape: 'discovery', page: { documents: [] } });
  const count = f.requests.length;
  await expect(
    f.port.query(
      { ...q, query: { ...q.query, continuation: `${first.page.continuation}X` } },
      call(),
    ),
  ).rejects.toMatchObject({ code: 'REMOTE_INPUT_INVALID' });
  expect(f.requests).toHaveLength(count);
  f.port.close();
});

test('create is one target, copied before token acquisition, no update mask or prep read', async () => {
  const f = await fixture();
  f.setResponse({
    writeResults: [{ updateTime: '2026-10-13T01:02:03Z' }],
    commitTime: '2026-10-13T01:02:03Z',
  });
  const bytes = new TextEncoder().encode(vector.document);
  const effect = f.port.publish(
    {
      mode: 'create',
      document: { target: { resource: 'rows', id: 'id+1' }, bytes, digest: vector.documentDigest },
    },
    call(),
  );
  bytes.fill(0);
  expect(await effect).toEqual({ kind: 'confirmed' });
  expect(f.requests).toHaveLength(1);
  const body = JSON.parse(f.requests[0]!.init.body as string);
  expect(body.writes[0]).toEqual({
    update: {
      name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
      fields: JSON.parse(vector.document),
    },
    currentDocument: { exists: false },
  });
  f.port.close();
});

test('one page interest covers an empty page, new writers and later writes, hints coalesce', async () => {
  const f = await fixture();
  const s = await f.port.subscribe(
    { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } },
    call(),
  );
  expect(f.listenerCount()).toBe(1);
  expect(f.costs[0]?.listener).toEqual({
    initialTail: 'all-matching-documents',
    deliveries: 'including-own-writes',
    catchUp: 'additional-rest-reads',
    reconnects: 'fresh-query-cost',
  });
  f.changed();
  f.changed();
  expect(await s.next(call())).toEqual({ kind: 'changed' });
  const later = s.next(call());
  f.changed();
  expect(await later).toEqual({ kind: 'changed' });
  expect(f.requests).toHaveLength(0);
  const pending = s.next(call());
  s.close();
  await expect(pending).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
  f.changed();
  await expect(s.next(call())).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
  expect(f.detached()).toBe(1);
  f.port.close();
});

test('listener error is unavailable, never proof of revoke, and late hints after close disappear', async () => {
  const f = await fixture();
  const s = await f.port.subscribe(
    { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } },
    call(),
  );
  const pending = s.next(call());
  f.failed();
  await expect(pending).rejects.toEqual({ code: 'REMOTE_STATE_UNAVAILABLE' });
  f.port.close();
  f.changed();
  expect(f.detached()).toBe(1);
});

test('three page interests fit; fourth refuses before a listener or budget attempt', async () => {
  const f = await fixture();
  const interest = { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } };
  const subscriptions = await Promise.all([1, 2, 3].map(() => f.port.subscribe(interest, call())));
  const charged = f.costs.length;
  await expect(f.port.subscribe(interest, call())).rejects.toMatchObject({
    code: 'REMOTE_RATE_LIMITED',
  });
  expect(f.listenerCount()).toBe(3);
  expect(f.costs.length).toBe(charged);
  subscriptions[0]!.close();
  const replacement = await f.port.subscribe(interest, call());
  replacement.close();
  f.port.close();
  expect(f.detached()).toBe(4);
});

test('closing a registration while its listener starts detaches the late listener', async () => {
  const f = await fixture();
  let finish!: (stop: () => void) => void, entered!: () => void;
  const ready = new Promise<void>((resolve) => {
    entered = resolve;
  });
  let detached = 0;
  const port = createSealedSync(
    f.context,
    { async admit() {} },
    fetch,
    {
      listen() {
        entered();
        return new Promise((resolve) => {
          finish = resolve;
        });
      },
    },
    () => {},
  );
  const pending = port.subscribe(
    { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } },
    call(),
  );
  const rejected = expect(pending).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
  await ready;
  port.close();
  finish(() => {
    detached++;
  });
  await rejected;
  expect(detached).toBe(1);
  f.port.close();
});

test('whole-object put and unconditional delete use one commit without preparation reads', async () => {
  const f = await fixture(['create', 'put', 'delete']);
  f.setResponse({ writeResults: [{}], commitTime: '2026-10-13T01:02:03Z' });
  const document = {
    target: { resource: 'rows', id: 'id+1' },
    bytes: new TextEncoder().encode(vector.document),
    digest: vector.documentDigest,
  };
  expect(await f.port.publish({ mode: 'put', document }, call())).toEqual({ kind: 'confirmed' });
  expect(await f.port.publish({ mode: 'delete', target: document.target }, call())).toEqual({
    kind: 'confirmed',
  });
  expect(f.requests).toHaveLength(2);
  expect(JSON.parse(f.requests[0]!.init.body as string).writes[0]).toEqual({
    update: {
      name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
      fields: JSON.parse(vector.document),
    },
  });
  expect(JSON.parse(f.requests[1]!.init.body as string).writes[0]).toEqual({
    delete: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
  });
  f.port.close();
});

test('an acknowledged create collision reads only that target, confirms identical bytes or refuses conflict', async () => {
  for (const same of [true, false]) {
    const f = await fixture();
    const fields = JSON.parse(vector.document);
    if (!same) fields.envelope = { bytesValue: 'AQI=' };
    f.setResponse((url: string) =>
      url.endsWith(':commit')
        ? { status: 409, body: { error: { status: 'ALREADY_EXISTS' } } }
        : {
            status: 200,
            body: {
              name: 'projects/demo-remote-1/databases/(default)/documents/x/notes/rows/id+1',
              fields,
            },
          },
    );
    const pending = f.port.publish(
      {
        mode: 'create',
        document: {
          target: { resource: 'rows', id: 'id+1' },
          bytes: new TextEncoder().encode(vector.document),
          digest: vector.documentDigest,
        },
      },
      call(),
    );
    if (same) expect(await pending).toEqual({ kind: 'already-present' });
    else
      await expect(pending).rejects.toMatchObject({
        code: 'REMOTE_STATE_UNAVAILABLE',
        state: 'conflict',
      });
    expect(f.requests).toHaveLength(2);
    expect(f.requests.map((r) => r.init.method)).toEqual(['POST', 'GET']);
    f.port.close();
  }
});

test('a pending next expires at its original deadline without ending the interest or inventing a hint', async () => {
  const f = await fixture();
  const s = await f.port.subscribe(
    { resource: 'rows', scope: { tenant: { stringValue: 'fixture-space' } } },
    call(),
  );
  const pending = s.next(call());
  const refused = expect(pending).rejects.toMatchObject({ code: 'REMOTE_STATE_UNAVAILABLE' });
  await vi.advanceTimersByTimeAsync(1000);
  await refused;
  f.changed();
  expect(await s.next(call())).toEqual({ kind: 'changed' });
  f.port.close();
});
