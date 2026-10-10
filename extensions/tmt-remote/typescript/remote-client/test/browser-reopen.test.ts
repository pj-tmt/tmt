import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
import { reopenSession, ReopenSessionError } from '../src/browser.js';
import { openSession, DeviceKey } from '../src/device.js';
import { RefusalError, ClientError } from '../src/operations.js';
import { Door, paired } from './door.js';
function request(result: unknown) {
  const pending: { result: unknown; onsuccess?: () => void } = { result };
  queueMicrotask(() => pending.onsuccess?.());
  return pending;
}
async function fixture() {
  const door = new Door();
  const device = await paired(door);
  const previous = await openSession(
    device.result,
    device.key,
    door.descriptor.windowId,
    door.fetch,
  );
  const record = {
    handle: device.key.handle(),
    publicKey: device.key.publicKey(),
    paired: device.result,
  };
  let mounts = 0,
    admissions = 0;
  let mode = 'refused';
  vi.stubGlobal('location', {
    origin: new URL(door.descriptor.address).origin,
    pathname: '/settings',
  });
  vi.stubGlobal('indexedDB', {
    open: () =>
      request({ transaction: () => ({ objectStore: () => ({ get: () => request(record) }) }) }),
  });
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
    if (url === '/sdk/mount') {
      mounts++;
      if (mode === 'unavailable' || (mode === 'late-unavailable' && mounts === 2))
        return new Response('', { status: 503 });
      return Response.json({
        machineId: door.descriptor.machineId,
        address: door.descriptor.address,
        windowId:
          mode === 'malformed'
            ? 'invalid'
            : mode === 'stale' && mounts === 2
              ? crypto.randomUUID()
              : door.descriptor.windowId,
        extension: null,
        mount: null,
      });
    }
    admissions++;
    if (mode === 'transport') throw new Error('Transport unavailable.');
    if (mode === 'capacity' || mode === 'unverified-capacity') {
      door.mutate = (reply) => {
        reply.sessionId = 'new';
        reply.sequence = '1';
      };
      door.tamper.signature = mode === 'unverified-capacity';
      return door.response(JSON.parse(init!.body as string), {
        error: {
          code: 'REMOTE_SESSION_LIMIT',
          message: 'Capacity reached.',
          limit: 8,
          settingsUrl: `${new URL(door.descriptor.address).origin}/settings`,
        },
      });
    }
    if (mode === 'success' || mode === 'unverified' || (mode === 'recovered' && admissions > 1)) {
      door.tamper.signature = mode === 'unverified';
      return door.fetch(url, init);
    }
    return new Response('', { status: 404 });
  });
  return {
    previous,
    record,
    door,
    counts: () => ({ mounts, admissions }),
    mode: (value: string) => {
      mode = value;
    },
  };
}
test('previous-session recovery makes one fresh admission and rechecks only the descriptor on refusal', async () => {
  const f = await fixture();
  try {
    await assert.rejects(
      reopenSession(f.previous),
      (error) => error instanceof RefusalError && error.code === 'REMOTE_SESSION_ENDED',
    );
    assert.deepEqual(f.counts(), { mounts: 2, admissions: 1 });
  } finally {
    vi.unstubAllGlobals();
  }
});
test('stale, malformed, unavailable, transport and unverified recovery remain unconfirmed', async () => {
  for (const mode of [
    'stale',
    'malformed',
    'unavailable',
    'late-unavailable',
    'transport',
    'unverified',
  ]) {
    const f = await fixture();
    try {
      f.mode(mode);
      await assert.rejects(
        reopenSession(f.previous),
        (error) => error instanceof Error && !(error instanceof RefusalError),
      );
      assert.equal(f.counts().admissions, ['malformed', 'unavailable'].includes(mode) ? 0 : 1);
    } finally {
      vi.unstubAllGlobals();
    }
  }
});
test('re-pair cannot use a different identity to recover the old operation, while matching live identity can reopen', async () => {
  const f = await fixture();
  try {
    f.record.paired = { ...f.record.paired, clientId: crypto.randomUUID() };
    await assert.rejects(reopenSession(f.previous), (error) => error instanceof RefusalError);
    assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
  } finally {
    vi.unstubAllGlobals();
  }
  const live = await fixture();
  try {
    live.mode('success');
    const fresh = await reopenSession(live.previous);
    assert.notEqual(fresh.sessionId, live.previous.sessionId);
    assert.deepEqual(live.counts(), { mounts: 1, admissions: 1 });
  } finally {
    vi.unstubAllGlobals();
  }
});
test('existing no-argument reopen retains its ordinary generic refusal behavior', async () => {
  const f = await fixture();
  try {
    await assert.rejects(
      reopenSession(),
      (error) =>
        error instanceof Error &&
        !(error instanceof RefusalError) &&
        error.message === 'The session was refused.',
    );
    assert.deepEqual(f.counts(), { mounts: 1, admissions: 1 });
  } finally {
    vi.unstubAllGlobals();
  }
});

test('key or machine trust-pin replacement cannot recover with the prior Session', async () => {
  for (const change of ['key', 'machine']) {
    const f = await fixture();
    try {
      if (change === 'key') {
        const replacement = await DeviceKey.generate();
        f.record.handle = replacement.handle();
        f.record.publicKey = replacement.publicKey();
      } else {
        f.record.paired = { ...f.record.paired, machinePublicKey: new Uint8Array(32).fill(4) };
      }
      await assert.rejects(reopenSession(f.previous), (error) => error instanceof RefusalError);
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
    } finally {
      vi.unstubAllGlobals();
    }
  }
});

for (const length of [31, 0, 33]) {
  test(`prior-Session recovery rejects a ${length}-byte machine pin before network access`, async () => {
    const f = await fixture();
    try {
      const pin = new Uint8Array(length);
      pin.set(f.record.paired.machinePublicKey.slice(0, length));
      f.record.paired = { ...f.record.paired, machinePublicKey: pin };
      let rejected: unknown;
      try {
        await reopenSession(f.previous);
      } catch (error) {
        rejected = error;
      }
      console.info('machine-pin fence', { length, ...f.counts() });
      assert.ok(rejected instanceof RefusalError);
      assert.equal(rejected.code, 'REMOTE_SESSION_ENDED');
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
    } finally {
      vi.unstubAllGlobals();
    }
  });
}

// Continuity regression: an opaque end of the old run is not a revoked pairing.
test('bounded reopen recovers a refused old run without any operation resend', async () => {
  const f = await fixture();
  try {
    f.mode('recovered');
    const fresh = await reopenSession(f.previous, { retry: 'bounded' });
    assert.notEqual(fresh.sessionId, f.previous.sessionId);
    assert.equal(f.counts().admissions, 2);
    assert.equal(f.door.calls.length, 0);
    const { operations } = await import('../src/operations.js');
    await operations(fresh).listAgents();
    assert.equal(f.door.calls.length, 1);
  } finally {
    vi.unstubAllGlobals();
  }
});

for (const change of ['client', 'key', 'machine']) {
  test(`bounded recovery classifies ${change} replacement as mismatch without network or retry`, async () => {
    const f = await fixture();
    try {
      if (change === 'client')
        f.record.paired = { ...f.record.paired, clientId: crypto.randomUUID() };
      if (change === 'machine')
        f.record.paired = { ...f.record.paired, machinePublicKey: new Uint8Array(32) };
      if (change === 'key') {
        const key = await DeviceKey.generate();
        f.record.handle = key.handle();
        f.record.publicKey = key.publicKey();
      }
      await assert.rejects(
        reopenSession(f.previous, { retry: 'bounded' }),
        (error) => error instanceof ReopenSessionError && error.reason === 'mismatch',
      );
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
    } finally {
      vi.unstubAllGlobals();
    }
  });
}
test('bounded callers join one admission and do not send an original operation', async () => {
  const f = await fixture();
  try {
    f.mode('success');
    const first = reopenSession(f.previous, { retry: 'bounded' });
    const joined = reopenSession(f.previous, { retry: 'bounded' });
    assert.equal(first, joined);
    assert.equal(await first, await joined);
    assert.equal(f.counts().admissions, 1);
    assert.equal(f.door.calls.length, 0);
  } finally {
    vi.unstubAllGlobals();
  }
});
test('absent pairing is unpaired; unreadable pairing is transient and never admitted', async () => {
  for (const unavailable of [false, true]) {
    const f = await fixture();
    try {
      vi.stubGlobal('indexedDB', {
        open: () => {
          if (unavailable) throw new Error('storage unavailable');
          return request({
            transaction: () => ({ objectStore: () => ({ get: () => request(undefined) }) }),
          });
        },
      });
      await assert.rejects(
        reopenSession(f.previous, { retry: 'bounded', signal: AbortSignal.timeout(50) }),
        (error) =>
          error instanceof ReopenSessionError &&
          error.reason === (unavailable ? 'transient' : 'unpaired'),
      );
      assert.equal(f.counts().admissions, 0);
    } finally {
      vi.unstubAllGlobals();
    }
  }
});
test('cancelled owner performs no admission', async () => {
  const f = await fixture();
  try {
    const owner = new AbortController();
    owner.abort();
    await assert.rejects(
      reopenSession(f.previous, { retry: 'bounded', signal: owner.signal }),
      (error) => error instanceof ReopenSessionError && error.detail === 'cancelled',
    );
    assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
  } finally {
    vi.unstubAllGlobals();
  }
});

test('lost send acknowledgment keeps the original ID across bounded admission and only observes it', async () => {
  const f = await fixture();
  try {
    const { operations } = await import('../src/operations.js');
    const id = crypto.randomUUID();
    f.door.afterAdoption = async (body) => {
      if (body.operation === 'dispatch.create') throw new Error('lost reply');
    };
    await assert.rejects(
      operations(f.previous).send({
        operationId: id,
        agentId: (f.door.agents[0] as { id: string }).id,
        message: 'one frozen send',
      }),
      (error) => error instanceof ClientError && error.operationId === id,
    );
    f.mode('recovered');
    const fresh = await reopenSession(f.previous, { retry: 'bounded' });
    const recovered = await operations(fresh).operation(id);
    assert.equal(recovered.state, 'accepted');
    assert.equal(recovered.operationId, id);
    assert.equal(
      f.door.calls.filter((call) => call.envelope.operation === 'dispatch.create').length,
      1,
    );
    assert.equal(
      f.door.calls.filter((call) => call.envelope.operation === 'operation.show').length,
      1,
    );
  } finally {
    vi.unstubAllGlobals();
  }
});

test('a late real signed admission cannot resolve a bounded call after owner abort', async () => {
  const f = await fixture();
  try {
    const original = fetch;
    let release!: (response: Response) => void;
    let arrived!: () => void;
    const reached = new Promise<void>((resolve) => {
      arrived = resolve;
    });
    let response!: Response;
    vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
      if (url === '/sdk/mount') return original(url, init);
      response = await f.door.fetch(url, init!);
      const held = new Promise<Response>((resolve) => {
        release = resolve;
      });
      arrived();
      return held;
    });
    const owner = new AbortController();
    let published = false;
    const pending = reopenSession(f.previous, { retry: 'bounded', signal: owner.signal });
    const checked = assert.rejects(
      pending,
      (error) => error instanceof ReopenSessionError && error.detail === 'cancelled',
    );
    void pending.then(
      () => {
        published = true;
      },
      () => {},
    );
    await reached;
    owner.abort();
    release(response);
    await checked;
    assert.equal(published, false);
    assert.equal(f.counts().mounts, 1);
  } finally {
    vi.unstubAllGlobals();
  }
});

for (const mode of ['capacity', 'unverified-capacity']) {
  test(`bounded ${mode} preserves verified cause without retry or uncertain effect resend`, async () => {
    const f = await fixture();
    try {
      const { operations } = await import('../src/operations.js');
      const id = crypto.randomUUID();
      f.door.afterAdoption = async (body) => {
        if (body.operation === 'dispatch.create') throw new Error('lost reply');
      };
      await assert.rejects(
        operations(f.previous).send({
          operationId: id,
          agentId: (f.door.agents[0] as { id: string }).id,
          message: 'one original',
        }),
        (error) => error instanceof ClientError && error.operationId === id,
      );
      f.mode(mode);
      await assert.rejects(reopenSession(f.previous, { retry: 'bounded' }), (error) => {
        assert.ok(error instanceof ReopenSessionError);
        if (mode === 'capacity') {
          assert.equal(error.reason, 'capacity');
          assert.ok(error.cause instanceof RefusalError);
          assert.equal(error.cause.code, 'REMOTE_SESSION_LIMIT');
          assert.equal(error.cause.limit, 8);
          assert.equal(
            error.cause.settingsUrl,
            `${new URL(f.door.descriptor.address).origin}/settings`,
          );
        } else {
          assert.equal(error.reason, 'transient');
          assert.equal(error.detail, 'unverifiable-response');
          assert.equal(error.cause, undefined);
        }
        return true;
      });
      assert.deepEqual(f.counts(), { mounts: 1, admissions: 1 });
      assert.equal(
        f.door.calls.filter((call) => call.envelope.operation === 'dispatch.create').length,
        1,
      );
      assert.equal(
        f.door.calls.filter((call) => call.envelope.operation === 'operation.show').length,
        0,
      );
    } finally {
      vi.unstubAllGlobals();
    }
  });
}
