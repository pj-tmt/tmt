import assert from 'node:assert/strict';
import { test, vi } from 'vite-plus/test';
import { landingPage, pairingPage } from '../src/browser.js';
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
    admissions = 0,
    observations = 0;
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
  const nodes = new Map<
    string,
    {
      textContent: string;
      hidden: boolean;
      dataset: Record<string, string>;
      addEventListener: () => void;
    }
  >();
  for (const id of [
    'status',
    'mark',
    'notice',
    'state-label',
    'heading',
    'checked-time',
    'machine-id',
    'protocol-address',
    'trust-pin',
    'steps-missing',
    'steps-different',
    'steps-refused',
    'steps-unconfirmed',
    'steps-unreadable',
    'command-pair',
    'command-devices',
    'command-status',
    'command-location',
    'copy-feedback',
    'copy-pair',
    'copy-devices',
    'copy-status',
  ])
    nodes.set(id, { textContent: '', hidden: false, dataset: {}, addEventListener: () => {} });
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
    const operation = JSON.parse(init!.body as string).operation;
    if (operation === 'session.open') {
      admissions++;
      if (mode === 'expired-reopen-refused' && admissions === 2)
        return new Response('', { status: 404 });
    }
    if (operation === 'capabilities') {
      observations++;
      if (mode.startsWith('expired-') && observations === 2) {
        if (mode !== 'expired-signed') return new Response('', { status: 404 });
        door.error = { code: 'REMOTE_SESSION_ENDED', message: 'Idle session expired.' };
        try {
          return await door.fetch(url, init);
        } finally {
          door.error = undefined;
        }
      }
      if (mode === 'expired-again' && observations > 2) return new Response('', { status: 404 });
    }
    if (mode === 'pending-admission') {
      arrive();
      await blocked;
      return new Response('', { status: 404 });
    }
    if (mode === 'signing') return new Response('', { status: 404 });
    if (mode === 'refused' || mode === 'stale') return new Response('', { status: 404 });
    door.tamper.signature = mode === 'unverified';
    if (mode === 'signed-refusal')
      door.error = { code: 'REMOTE_SESSION_EVICTED', message: 'ended', limit: 4 };
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
    door,
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
test('entry automatically checks once, then manual checks reuse the verified session without effects', async () => {
  const f = await fixture();
  try {
    await landingPage();
    assert.equal(f.nodes.get('heading')!.textContent, 'Connected');
    assert.equal(
      f.nodes.get('status')!.textContent,
      'Open your app from its link in this browser.',
    );
    assert.deepEqual(f.counts(), { mounts: 1, admissions: 1 });
    assert.deepEqual(
      f.door.calls.map((call) => call.envelope.operation),
      ['capabilities'],
    );
    f.click();
    await vi.waitFor(() => assert.equal(f.door.calls.length, 2));
    await vi.waitFor(() => assert.equal(f.button.disabled, false));
    assert.deepEqual(f.counts(), { mounts: 1, admissions: 1 });
    assert.deepEqual(
      f.door.calls.map((call) => call.envelope.operation),
      ['capabilities', 'capabilities'],
    );
    assert.notEqual(f.door.calls[0]!.envelope.id, f.door.calls[1]!.envelope.id);
  } finally {
    vi.unstubAllGlobals();
  }
});

for (const mode of ['expired-404', 'expired-signed', 'expired-reopen-refused', 'expired-again']) {
  test(`manual check reopens an expired reused session once: ${mode}`, async () => {
    const f = await fixture(mode);
    try {
      await landingPage();
      assert.equal(f.nodes.get('heading')!.textContent, 'Connected');
      f.click();
      await vi.waitFor(() => assert.equal(f.button.disabled, false));
      const connected = mode === 'expired-404' || mode === 'expired-signed';
      assert.equal(
        f.nodes.get('state-label')!.textContent,
        connected ? 'Connected' : "Can't reach Remote",
      );
      assert.notEqual(f.nodes.get('state-label')!.textContent, 'Not accepted');
      assert.deepEqual(f.counts(), {
        mounts: mode === 'expired-reopen-refused' ? 3 : 2,
        admissions: 2,
      });
      assert.equal(f.door.opens, connected || mode === 'expired-again' ? 2 : 1);
      assert.ok(f.door.calls.every((call) => call.envelope.operation === 'capabilities'));
    } finally {
      vi.unstubAllGlobals();
    }
  });
}

test('checked time follows viewer local zones rather than ISO or forced UTC', async () => {
  const before = process.env.TZ;
  const displayed: string[] = [];
  try {
    for (const zone of ['Asia/Tokyo', 'America/Los_Angeles']) {
      process.env.TZ = zone;
      const f = await fixture();
      try {
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(new Date('2026-10-09T01:23:45Z'));
        await landingPage();
        const expected = new Intl.DateTimeFormat(undefined, {
          dateStyle: 'medium',
          timeStyle: 'short',
        }).format(new Date());
        const actual = f.nodes.get('checked-time')!.textContent;
        assert.equal(actual, expected, zone);
        assert.ok(!actual.includes('2026-10-09T'));
        displayed.push(actual);
      } finally {
        vi.useRealTimers();
        vi.unstubAllGlobals();
      }
    }
    assert.notEqual(displayed[0], displayed[1]);
  } finally {
    if (before === undefined) delete process.env.TZ;
    else process.env.TZ = before;
  }
});

test('missing and unreadable pairing send nothing', async () => {
  for (const state of ['missing', 'invalid']) {
    const f = await fixture();
    try {
      f.record(state === 'missing' ? undefined : null);
      await landingPage();
      assert.equal(
        f.nodes.get('state-label')!.textContent,
        state === 'missing' ? 'Not paired' : 'Pairing unreadable',
      );
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
      assert.deepEqual(f.door.calls, []);
    } finally {
      vi.unstubAllGlobals();
    }
  }
});

test('entry validates stored identities, origin, address and exact key pins before network', async () => {
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
      assert.equal(f.nodes.get('state-label')!.textContent, 'Pairing unreadable', change);
      assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 }, change);
    } finally {
      vi.unstubAllGlobals();
    }
  }
});

test('entry separates signed refusal from opaque, mismatched and unverifiable responses', async () => {
  for (const mode of [
    'refused',
    'stale',
    'transport',
    'unverified',
    'other-machine',
    'signed-refusal',
  ]) {
    const f = await fixture(mode);
    try {
      await landingPage();
      assert.equal(
        f.nodes.get('state-label')!.textContent,
        mode === 'other-machine'
          ? 'Different machine'
          : mode === 'signed-refusal'
            ? 'Not accepted'
            : "Can't reach Remote",
      );
      assert.deepEqual(f.counts(), {
        mounts: ['refused', 'stale'].includes(mode) ? 2 : 1,
        admissions: ['transport', 'other-machine'].includes(mode) ? 0 : 1,
      });
      assert.ok(!f.nodes.get('status')!.textContent.includes('private transport cause'));
      assert.ok(f.door.calls.every((call) => call.envelope.operation === 'capabilities'));
    } finally {
      vi.unstubAllGlobals();
    }
  }
});

for (const mode of ['delayed', 'signing', 'pending-admission']) {
  test(`departure during ${mode} prevents late painting and new requests`, async () => {
    const f = await fixture(mode);
    const pending = landingPage();
    try {
      if (mode === 'delayed') await vi.waitFor(() => assert.equal(f.counts().mounts, 1));
      else await f.reached;
      const status = f.nodes.get('status')!.textContent;
      f.hide();
      f.release();
      await pending;
      assert.deepEqual(f.counts(), { mounts: 1, admissions: mode === 'pending-admission' ? 1 : 0 });
      assert.equal(f.nodes.get('status')!.textContent, status);
      assert.equal(f.nodes.get('state-label')!.textContent, 'Checking');
    } finally {
      f.release();
      await pending;
      vi.restoreAllMocks();
      vi.unstubAllGlobals();
    }
  });
}

for (const outcome of ['valid', 'refused', 'malformed', 'transport']) {
  test(`pairing enables only with its listener and a validated offer: ${outcome}`, async () => {
    const door = new Door();
    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    let arrive!: () => void;
    const reached = new Promise<void>((resolve) => {
      arrive = resolve;
    });
    let listener: ((event: Event) => void) | undefined;
    let validated = false;
    let disabled = true;
    let enables = 0;
    const button = {
      get disabled() {
        return disabled;
      },
      set disabled(value: boolean) {
        if (!value) {
          assert.equal(validated, true, 'offer response precedes enablement');
          assert.equal(typeof listener, 'function', 'submit listener precedes enablement');
          enables++;
        }
        disabled = value;
      },
    };
    const form = {
      hidden: false,
      querySelector: () => button,
      addEventListener: (type: string, value: (event: Event) => void) => {
        assert.equal(type, 'submit');
        listener = value;
      },
    };
    const status = { dataset: {} as Record<string, string>, textContent: '' };
    const mark = { textContent: '' };
    const notice = { dataset: {} as Record<string, string> };
    const stateLabel = { textContent: 'Initializing' };
    vi.stubGlobal('document', {
      readyState: 'complete',
      getElementById: (id: string) =>
        (
          ({ pair: form, status, mark, notice, 'state-label': stateLabel }) as Record<
            string,
            unknown
          >
        )[id],
    });
    vi.stubGlobal('fetch', async () => {
      arrive();
      await held;
      if (outcome === 'transport') throw new Error('Fixture offer failed.');
      validated = outcome === 'valid';
      return Response.json(outcome === 'valid' ? door.descriptor : {}, {
        status: outcome === 'refused' ? 404 : 200,
      });
    });
    const attempt = pairingPage(door.link());
    try {
      await reached;
      assert.equal(button.disabled, true);
      assert.equal(listener, undefined);
      assert.equal(enables, 0);
      release();
      await attempt;
      assert.equal(button.disabled, outcome !== 'valid');
      assert.equal(enables, outcome === 'valid' ? 1 : 0);
      assert.equal(form.hidden, outcome !== 'valid');
      if (outcome === 'valid') assert.equal(stateLabel.textContent, 'Ready to pair');
      if (outcome !== 'valid') {
        assert.equal(listener, undefined);
        assert.equal(status.dataset.state, 'blocked');
        assert.equal(notice.dataset.tone, 'blocked');
        assert.equal(stateLabel.textContent, 'Unavailable');
      }
    } finally {
      release();
      await attempt.catch(() => {});
      vi.unstubAllGlobals();
    }
  });
}
