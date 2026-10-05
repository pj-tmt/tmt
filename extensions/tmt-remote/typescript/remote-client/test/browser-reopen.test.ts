import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
import { reopenSession } from '../src/browser.js';
import { openSession, DeviceKey } from '../src/device.js';
import { RefusalError } from '../src/operations.js';
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
    if (mode === 'success' || mode === 'unverified') {
      door.tamper.signature = mode === 'unverified';
      return door.fetch(url, init);
    }
    return new Response('', { status: 404 });
  });
  return {
    previous,
    record,
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
