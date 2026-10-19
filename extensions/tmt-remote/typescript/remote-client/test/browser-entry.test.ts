import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test, vi } from 'vite-plus/test';
import { landingPage, pairingPage } from '../src/browser.js';
import { Door, paired } from './door.js';
import { DeviceKey } from '../src/device.js';

for (const failure of ['consumed', 'transport', 'storage-colab'] as const) {
  test(`automatic ${failure} offers show the approved recovery command as code`, async () => {
    const children: { textContent: string; tagName?: string; className?: string }[] = [];
    const status = {
      dataset: {} as Record<string, string>,
      textContent: '',
      replaceChildren: (...nodes: typeof children) => {
        children.splice(0, children.length, ...nodes);
        status.textContent = nodes.map((node) => node.textContent).join('');
      },
    };
    const form = { hidden: false, querySelector: () => ({ disabled: true }) };
    const comparison = { hidden: false };
    const entry = { hidden: false };
    const nodes: Record<string, unknown> = {
      pair: form,
      comparison,
      'entry-link': entry,
      status,
      mark: { textContent: '' },
      notice: { dataset: {} },
      'state-label': { textContent: '' },
    };
    vi.stubGlobal('document', {
      readyState: 'complete',
      getElementById: (id: string) => nodes[id],
      createElement: (tag: string) => ({
        tagName: tag.toUpperCase(),
        className: '',
        textContent: '',
      }),
      createTextNode: (textContent: string) => ({ textContent }),
    });
    const door = new Door();
    vi.stubGlobal('indexedDB', {
      open: () => {
        throw new Error('Fixture storage failure.');
      },
    });
    let requests = 0;
    vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) => {
      requests++;
      assert.equal(init?.method, undefined, 'no enrollment after a failed descriptor');
      if (failure === 'transport') throw new Error('Fixture transport failure.');
      return failure === 'storage-colab'
        ? Response.json({ ...door.descriptor, automatic: true, target: '/p/abcd' })
        : new Response('{}', { status: 404 });
    });
    try {
      await pairingPage(door.link().replace('#', '#auto='));
      assert.equal(form.hidden, true);
      assert.equal(comparison.hidden, true, 'a reused manual page must hide its old comparison');
      assert.equal(entry.hidden, true, 'a reused manual page must hide its old success link');
      assert.equal(status.dataset.state, 'blocked');
      assert.equal(
        status.textContent,
        failure === 'consumed'
          ? 'This link no longer works. Open the page again from the machine with tmt remote open.'
          : `Could not connect this browser. Open the page again from the machine with ${failure === 'storage-colab' ? 'tmt colab open' : 'tmt remote open'}. If it still fails, run tmt remote pair there.`,
      );
      assert.deepEqual(
        children.filter((node) => node.tagName === 'CODE').map((node) => node.textContent),
        failure === 'consumed'
          ? ['tmt remote open']
          : [failure === 'storage-colab' ? 'tmt colab open' : 'tmt remote open', 'tmt remote pair'],
      );
      assert.ok(
        children
          .filter((node) => node.tagName === 'CODE')
          .every((node) => node.className === 'tmt-ui-code'),
      );
      assert.equal(requests, 1);
    } finally {
      vi.unstubAllGlobals();
    }
  });
}

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
    'pairing-note',
    'command-location',
    'copy-feedback-missing-pair',
    'copy-feedback-different-pair',
    'copy-feedback-refused-devices',
    'copy-feedback-refused-pair',
    'copy-feedback-unconfirmed-status',
    'copy-feedback-unreadable-pair',
    'copy-missing-pair',
    'copy-different-pair',
    'copy-refused-devices',
    'copy-refused-pair',
    'copy-unconfirmed-status',
    'copy-unreadable-pair',
  ])
    nodes.set(id, { textContent: '', hidden: false, dataset: {}, addEventListener: () => {} });
  const statusChildren: { textContent: string; tagName?: string; className?: string }[] = [];
  const status = nodes.get('status')!;
  let statusText = '';
  Object.defineProperty(status, 'textContent', {
    get: () =>
      statusChildren.length ? statusChildren.map((node) => node.textContent).join('') : statusText,
    set: (value: string) => {
      statusText = value;
      statusChildren.length = 0;
    },
  });
  Object.assign(status, {
    replaceChildren: (...children: typeof statusChildren) => {
      statusText = '';
      statusChildren.splice(0, statusChildren.length, ...children);
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
    createElement: (tag: string) => ({
      tagName: tag.toUpperCase(),
      className: '',
      textContent: '',
    }),
    createTextNode: (textContent: string) => ({ textContent }),
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
      if (['revoked-proof', 'expired-proof', 'ended-proof'].includes(mode)) {
        door.mutate = (reply) => {
          reply.sessionId = 'new';
          reply.sequence = '1';
        };
        return door.response(JSON.parse(init!.body as string), {
          error: {
            code:
              mode === 'revoked-proof'
                ? 'REMOTE_DEVICE_REVOKED'
                : mode === 'expired-proof'
                  ? 'REMOTE_GRANT_EXPIRED'
                  : 'REMOTE_SESSION_ENDED',
            message: 'Authority ended.',
          },
        });
      }
      if (mode === 'expired-reopen-refused' && admissions === 2)
        return new Response('', { status: 404 });
    }
    if (operation === 'capabilities') {
      observations++;
      if (mode.startsWith('expired-') && observations === 2) {
        if (mode !== 'expired-signed' && mode !== 'expired-evicted')
          return new Response('', { status: 404 });
        door.error =
          mode === 'expired-evicted'
            ? { code: 'REMOTE_SESSION_EVICTED', message: 'Session evicted.', limit: 4 }
            : { code: 'REMOTE_SESSION_ENDED', message: 'Idle session expired.' };
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
    if (['REMOTE_CLOSED', 'REMOTE_GRANT_CHANGED', 'REMOTE_SERVE_STOPPED'].includes(mode))
      door.error = { code: mode, message: 'ended' };
    if (mode === 'ended-proof') door.error = { code: 'REMOTE_SESSION_ENDED', message: 'ended' };
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
    statusChildren,
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
    // The two departures are separate predicates: a connected page that receives pagehide, and
    // a check button detached without one.
    pagehide: () => hide(),
    detach: () => {
      button.isConnected = false;
    },
  };
}
test('entry automatically checks once, then manual checks reuse the verified session without effects', async () => {
  const f = await fixture();
  try {
    await landingPage();
    assert.equal(f.nodes.get('heading')!.textContent, 'This browser can use Remote');
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

for (const mode of [
  'expired-404',
  'expired-signed',
  'expired-evicted',
  'expired-reopen-refused',
  'expired-again',
]) {
  test(`manual check reopens an expired reused session once: ${mode}`, async () => {
    const f = await fixture(mode);
    try {
      await landingPage();
      assert.equal(f.nodes.get('heading')!.textContent, 'This browser can use Remote');
      f.click();
      await vi.waitFor(() => assert.equal(f.button.disabled, false));
      const connected =
        mode === 'expired-404' || mode === 'expired-signed' || mode === 'expired-evicted';
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

test('not-paired steps precede the muted privacy note without duplicate status copy', async () => {
  const f = await fixture();
  try {
    f.record(undefined);
    await landingPage();
    assert.equal(f.nodes.get('state-label')!.textContent, 'Not paired');
    assert.equal(f.nodes.get('status')!.hidden, true);
    assert.equal(f.nodes.get('status')!.textContent, '');
    assert.equal(f.nodes.get('pairing-note')!.hidden, false);
    assert.equal(f.nodes.get('steps-missing')!.hidden, false);
    assert.equal(f.nodes.get('command-location')!.hidden, false);
    assert.deepEqual(f.counts(), { mounts: 0, admissions: 0 });
  } finally {
    vi.unstubAllGlobals();
  }
});

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
      assert.equal(
        f.nodes.get('notice')!.dataset.tone,
        state === 'missing' ? 'waiting' : 'blocked',
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
    'REMOTE_CLOSED',
    'REMOTE_GRANT_CHANGED',
    'REMOTE_SERVE_STOPPED',
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
      assert.equal(
        f.nodes.get('notice')!.dataset.tone,
        mode === 'other-machine' ? 'waiting' : 'blocked',
        mode,
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

for (const [mode, departure] of ['delayed', 'signing', 'pending-admission'].flatMap((mode) =>
  (['pagehide', 'detach'] as const).map((departure) => [mode, departure] as const),
)) {
  test(`departure by ${departure} during ${mode} prevents late painting and new requests`, async () => {
    const f = await fixture(mode);
    const pending = landingPage();
    try {
      if (mode === 'delayed') await vi.waitFor(() => assert.equal(f.counts().mounts, 1));
      else await f.reached;
      const status = f.nodes.get('status')!.textContent;
      f[departure]();
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

test('Command block Copy actions have command-specific names and no trailing action period', () => {
  const html = readFileSync(
    new URL('../../../rust/tmt-remote/assets/landing.html', import.meta.url),
    'utf8',
  );
  for (const [state, command] of [
    ['missing', 'pair'],
    ['different', 'pair'],
    ['refused', 'devices'],
    ['refused', 'pair'],
    ['unconfirmed', 'status'],
    ['unreadable', 'pair'],
  ]) {
    const action = html.match(
      new RegExp(`<button\\s+id="copy-${state}-${command}"([\\s\\S]*?)>\\s*Copy\\s*</button\\s*>`),
    );
    assert.ok(action);
    assert.ok(action[1]!.includes(`aria-label="Copy tmt remote ${command}"`));
  }
  assert.equal(html.match(/class="entry-command tmt-ui-command"/g)!.length, 6);
  assert.ok(!/<\/span\s*>\s*\./.test(html));
  assert.ok(html.includes('run this on it:'));
});

for (const ownsFocus of [true, false]) {
  test(`pairing failure keeps visible focus without stealing it: ${ownsFocus}`, async () => {
    const door = new Door();
    let submit!: (event: Event) => void;
    let fail!: () => void;
    const generated = new Promise<never>((_, reject) => {
      fail = () => reject(new Error('Fixture key failure'));
    });
    let finished!: () => void;
    const blocked = new Promise<void>((resolve) => {
      finished = resolve;
    });
    const other = {};
    const button = { disabled: true };
    const doc = {
      readyState: 'complete',
      activeElement: ownsFocus ? button : other,
      getElementById: (id: string) => nodes[id],
    };
    const status = {
      dataset: {},
      set textContent(value: string) {
        if (value.includes('Pairing did not complete')) finished();
      },
      focus: () => {
        doc.activeElement = status;
      },
    };
    const form = {
      hidden: false,
      contains: (node: unknown) => node === button,
      querySelector: () => button,
      addEventListener: (_: string, listener: (event: Event) => void) => {
        submit = listener;
      },
    };
    const nodes: Record<string, unknown> = {
      pair: form,
      status,
      name: { value: 'Fixture' },
      mark: { textContent: '' },
      notice: { dataset: {} },
      'state-label': { textContent: '' },
      comparison: {},
    };
    vi.stubGlobal('document', doc);
    vi.stubGlobal('fetch', async () => Response.json(door.descriptor));
    const key = vi.spyOn(DeviceKey, 'generate').mockImplementation(() => generated);
    try {
      await pairingPage(door.link());
      submit({ preventDefault() {} } as Event);
      assert.equal(form.hidden, true);
      assert.equal(doc.activeElement, ownsFocus ? status : other);
      fail();
      await blocked;
      assert.equal(doc.activeElement, ownsFocus ? status : other);
    } finally {
      key.mockRestore();
      vi.unstubAllGlobals();
    }
  });
}

for (const mode of ['revoked-proof', 'expired-proof', 'ended-proof', 'signed-refusal']) {
  test(`entry ${mode} names only the verified refusal evidence without changing actions`, async () => {
    const f = await fixture(mode);
    try {
      await landingPage();
      assert.equal(f.nodes.get('state-label')!.textContent, 'Not accepted');
      assert.equal(f.nodes.get('steps-refused')!.hidden, false);
      const authorityEnded = mode === 'revoked-proof' || mode === 'expired-proof';
      assert.equal(
        f.nodes.get('status')!.textContent,
        authorityEnded
          ? 'This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine.'
          : "This browser's session ended. Choose Check again, or open your app from its link to start a new one.",
      );
      assert.equal(f.button.disabled, false);
      assert.equal((f.button as { hidden?: boolean }).hidden, false);
      assert.deepEqual(
        f.statusChildren
          .filter((node) => node.tagName === 'CODE')
          .map((node) => ({
            text: node.textContent,
            className: node.className,
          })),
        authorityEnded ? [{ text: 'tmt remote pair', className: 'tmt-ui-code' }] : [],
      );
      assert.deepEqual(f.counts(), { mounts: 1, admissions: 1 });
      assert.deepEqual(
        f.door.calls.map((call) => call.envelope.operation),
        mode === 'signed-refusal' ? ['capabilities'] : [],
      );
    } finally {
      vi.unstubAllGlobals();
    }
  });
}
