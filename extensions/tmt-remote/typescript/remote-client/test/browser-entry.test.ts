import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
import { landingPage } from '../src/browser.js';
import { Door, paired } from './door.js';

function request(result: unknown) {
  const pending: { result: unknown; onsuccess?: () => void } = { result };
  queueMicrotask(() => pending.onsuccess?.());
  return pending;
}
async function fixture(mode = 'success') {
  const door = new Door();
  const device = await paired(door);
  let record: unknown = {
    handle: device.key.handle(),
    publicKey: device.key.publicKey(),
    paired: device.result,
  };
  let mounts = 0,
    admissions = 0;
  let release = () => {};
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  let arrive = () => {};
  const reached = new Promise<void>((resolve) => {
    arrive = resolve;
  });
  let click: () => void | Promise<void> = () => {};
  let hide = () => {};
  let done: () => void;
  const settled = new Promise<void>((resolve) => {
    done = resolve;
  });
  const nodes = new Map<
    string,
    { textContent: string; hidden: boolean; dataset: Record<string, string> }
  >();
  for (const id of ['status', 'mark', 'pairing-status', 'access-status'])
    nodes.set(id, { textContent: '', hidden: false, dataset: {} });
  const access = nodes.get('access-status')!;
  let accessText = '';
  Object.defineProperty(access, 'textContent', {
    get: () => accessText,
    set: (text: string) => {
      accessText = text;
      if (text !== 'Connecting…') done();
    },
  });
  const button = {
    disabled: true,
    isConnected: true,
    addEventListener: (_: string, listener: () => void | Promise<void>) => {
      click = listener;
    },
  };
  vi.stubGlobal('document', {
    readyState: 'complete',
    getElementById: (id: string) => (id === 'check' ? button : nodes.get(id)),
  });
  vi.stubGlobal('window', {
    addEventListener: (_: string, listener: () => void) => {
      hide = listener;
    },
  });
  vi.stubGlobal('location', { origin: new URL(door.descriptor.address).origin, pathname: '/' });
  vi.stubGlobal('indexedDB', {
    open: () =>
      request({ transaction: () => ({ objectStore: () => ({ get: () => request(record) }) }) }),
  });
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
    if (url === '/sdk/mount') {
      mounts++;
      if (mode === 'delayed') await blocked;
      if (mode === 'transport') throw new Error('private transport cause');
      return Response.json({
        machineId: mode === 'other-machine' ? crypto.randomUUID() : door.descriptor.machineId,
        address: door.descriptor.address,
        windowId: mode === 'stale' && mounts === 2 ? crypto.randomUUID() : door.descriptor.windowId,
        extension: null,
        mount: null,
      });
    }
    admissions++;
    if (mode === 'pending-admission') {
      arrive();
      await blocked;
      return new Response('', { status: 404 });
    }
    if (mode === 'signing') return new Response('', { status: 404 });
    if (mode === 'refused' || mode === 'stale') return new Response('', { status: 404 });
    door.tamper.signature = mode === 'unverified';
    return door.fetch(url, init);
  });
  if (mode === 'signing') {
    const sign = crypto.subtle.sign.bind(crypto.subtle);
    vi.spyOn(crypto.subtle, 'sign').mockImplementation(async (algorithm, key, data) => {
      const signature = await sign(algorithm, key, data);
      if (new TextDecoder().decode(data).includes('session.open')) {
        arrive();
        await blocked;
      }
      return signature;
    });
  }
  return {
    reached,
    device,
    nodes,
    button,
    settled,
    record: (value: unknown) => {
      record = value;
    },
    counts: () => ({ mounts, admissions }),
    release,
    click: () => click(),
    hide: () => {
      hide();
      button.isConnected = false;
    },
  };
}
test('entry reads only validated local pairing evidence and never connects on load', async () => {
  for (const state of ['missing', 'saved', 'invalid']) {
    const f = await fixture();
    try {
      if (state === 'missing') f.record(undefined);
      if (state === 'invalid') f.record(null);
      await landingPage();
      assert.equal(
        f.nodes.get('pairing-status')!.textContent,
        {
          missing: 'No saved pairing',
          saved: 'Pairing saved in this browser',
          invalid: 'Pairing status unknown',
        }[state],
      );
      assert.equal(f.button.disabled, state !== 'saved');
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
    } finally {
      vi.unstubAllGlobals();
    }
  }
});
test('entry validates complete stored identities, origin, address and exact key pins before network', async () => {
  for (const change of [
    'machine',
    'client',
    'origin',
    'address',
    'revision',
    'short-pin',
    'empty-pin',
    'public-key',
    'handle',
  ]) {
    const f = await fixture();
    try {
      const record = {
        handle: f.device.key.handle(),
        publicKey: f.device.key.publicKey(),
        paired: { ...f.device.result },
      };
      if (change === 'machine') record.paired.machineId = 'not-an-identity';
      if (change === 'client') record.paired.clientId = 'not-an-identity';
      if (change === 'origin') record.paired.origin = 'https://example.com';
      if (change === 'address') record.paired.address += '?code=not-a-route';
      if (change === 'revision') record.paired.grantRevision = Number.NaN;
      if (change === 'short-pin')
        record.paired.machinePublicKey = record.paired.machinePublicKey.slice(0, 31);
      if (change === 'empty-pin') record.paired.machinePublicKey = new Uint8Array();
      if (change === 'public-key') record.publicKey = new Uint8Array(32);
      if (change === 'handle') record.handle = {} as CryptoKey;
      f.record(record);
      await landingPage();
      assert.equal(f.nodes.get('pairing-status')!.textContent, 'Pairing status unknown', change);
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 }, change);
    } finally {
      vi.unstubAllGlobals();
    }
  }
});
test('explicit entry check distinguishes verified access, current refusal and unconfirmed failures without effects', async () => {
  for (const mode of ['success', 'refused', 'stale', 'transport', 'unverified', 'other-machine']) {
    const f = await fixture(mode);
    try {
      await landingPage();
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
      f.click();
      await f.settled;
      assert.equal(
        f.nodes.get('access-status')!.textContent,
        mode === 'success'
          ? 'Access confirmed'
          : mode === 'refused'
            ? 'Could not verify access'
            : mode === 'other-machine'
              ? 'Not checked'
              : 'Could not verify access',
      );
      assert.deepEqual(f.counts(), {
        mounts: ['refused', 'stale'].includes(mode) ? 2 : 1,
        admissions: ['transport', 'other-machine'].includes(mode) ? 0 : 1,
      });
      assert.equal(
        f.nodes.get('pairing-status')!.textContent,
        mode === 'other-machine'
          ? 'Saved pairing does not match this Remote'
          : 'Pairing saved in this browser',
      );
      assert.ok(!f.nodes.get('status')!.textContent.includes('private transport cause'));
      if (mode === 'success')
        assert.match(f.nodes.get('status')!.textContent, /Checked at \d{4}-\d{2}-\d{2}T/);
      if (mode === 'refused')
        assert.ok(f.nodes.get('status')!.textContent.includes('not a verified refusal reason'));
    } finally {
      vi.unstubAllGlobals();
    }
  }
});

test('a departed page cannot paint a late connection result or continue admission', async () => {
  const f = await fixture('delayed');
  try {
    await landingPage();
    const attempt = f.click();
    // Wait for the actual descriptor request, then replace the page before its answer.
    await vi.waitFor(() => assert.equal(f.counts().mounts, 1));
    f.hide();
    f.release();
    await attempt;
    assert.deepEqual(f.counts(), { mounts: 1, admissions: 0 });
    assert.equal(f.nodes.get('access-status')!.textContent, 'Connecting…');
  } finally {
    vi.unstubAllGlobals();
  }
});

for (const mode of ['signing', 'pending-admission']) {
  test(`departure during ${mode} prevents new requests after the pending boundary`, async () => {
    const f = await fixture(mode);
    try {
      await landingPage();
      const attempt = f.click();
      await f.reached;
      const status = f.nodes.get('status')!.textContent;
      f.hide();
      f.release();
      await attempt;
      assert.deepEqual(f.counts(), {
        mounts: 1,
        admissions: mode === 'signing' ? 0 : 1,
      });
      assert.equal(f.nodes.get('access-status')!.textContent, 'Connecting…');
      assert.equal(f.nodes.get('status')!.textContent, status);
    } finally {
      vi.restoreAllMocks();
      vi.unstubAllGlobals();
    }
  });
}
