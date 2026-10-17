import { beforeEach, expect, it, vi } from 'vite-plus/test';
import type { LiveSessionOwner } from '../src/live.js';
import type { Pending } from '../src/management.js';
import type { RemoteClient } from '../src/ask-remote.js';
import { mountedTransport } from '../src/mounted.js';
import { RecoveryRequiredError } from '../src/session-recovery.js';
import { AdmissionRefusedError } from '../src/registration.js';
import { reopenWithBackoff, retryableReopen } from '../src/readmit.js';
const setup = vi.hoisted(() => {
  const first = { deviceId: 'device', remoteSession: { id: 'first' } };
  const second = { deviceId: 'device', remoteSession: { id: 'second' } };
  return {
    first,
    second,
    sdk: {
      reopenSession: vi.fn(async () => ({})),
      certifyKey: vi.fn(async () => ({})),
      management: vi.fn((_session: unknown, _options?: { timeoutMs?: number }) => ({
        devices: vi.fn(async () => ({ devices: [], nextCursor: null })),
      })),
    },
    signals: [] as AbortSignal[],
    ports: [] as RemoteClient[],
    events: [] as string[],
    pages: [{ pageId: 'page', sharing: 'private' }] as {
      pageId: string;
      sharing: string;
      archived?: boolean;
    }[],
    register: vi.fn(),
    discover: vi.fn(),
    registrations: [] as unknown[],
    verify: vi.fn(async () => {}),
    remote: vi.fn(),
    owner: null as LiveSessionOwner | null,
    titleRemember: vi.fn(async () => {}),
    cachedTitles: new Map<string, string>(),
    titleRead: vi.fn(async () => ({ title: 'Verified title' })),
    titleJoin: vi.fn(async () => {}),
    titleConnections: [] as {
      args: unknown[];
      close: ReturnType<typeof vi.fn>;
      join: ReturnType<typeof vi.fn>;
    }[],
    managementRead: vi.fn(async () => ({ page: { pageId: 'page' } })),
    managementPrepare: vi.fn(async () => ({ id: 'operation' })),
    managementSend: vi.fn(async () => ({ operationId: 'operation' })),
    managementVerify: vi.fn(async () => ({ page: { pageId: 'page' } })),
  };
});
vi.mock('../src/bootstrap.js', () => ({
  mountUrl: () => new URL('https://example.test/colab/'),
  discover: async (_mount: URL, verify: (space: string, owner: Uint8Array) => Promise<void>) => {
    await setup.discover();
    const owner = new Uint8Array(32);
    await verify('space', owner);
    return { space: 'space', owner, pageIds: [], pages: setup.pages };
  },
}));
vi.mock('../src/registration.js', async (original) => ({
  ...(await original<typeof import('../src/registration.js')>()),
  remoteSdk: async () => setup.sdk,
  register: setup.register,
  verifyRegistration: setup.verify,
}));
vi.mock('../src/ask-remote.js', () => ({ createRemoteClient: setup.remote }));
vi.mock('../src/title-cache.js', () => ({
  TitleCache: class {
    remember = setup.titleRemember;
    async read(page: string) {
      return setup.cachedTitles.get(page);
    }
    async fillMissing(page: string, title: string, signal: AbortSignal) {
      if (signal.aborted) return undefined;
      if (!setup.cachedTitles.has(page)) setup.cachedTitles.set(page, title);
      return setup.cachedTitles.get(page);
    }
  },
}));
vi.mock('../src/admission.js', () => ({
  Admission: class {
    async restore() {}
    closeKeys() {}
  },
}));
vi.mock('../src/connection.js', () => ({
  Connection: class {
    ready: Promise<{ title: string }>;
    close: ReturnType<typeof vi.fn<() => void>>;
    closeAndJoin: ReturnType<typeof vi.fn<() => Promise<void>>>;
    constructor(...args: unknown[]) {
      let reject!: (error: Error) => void;
      const stopped = new Promise<never>((_resolve, failed) => {
        reject = failed;
      });
      this.ready = Promise.race([setup.titleRead(), stopped]);
      this.close = vi.fn(() => reject(new Error('Page closed')));
      this.closeAndJoin = vi.fn(async () => {
        this.close();
        await setup.titleJoin();
      });
      setup.titleConnections.push({ args, close: this.close, join: this.closeAndJoin });
    }
  },
}));
vi.mock('../src/management.js', () => ({
  ManagementError: class extends Error {
    constructor(readonly code: string) {
      super(code);
    }
  },
  project: (page: unknown) => ({ page }),
  ManagementClient: class {
    read = setup.managementRead;
    prepare = setup.managementPrepare;
    send = setup.managementSend;
    verify = setup.managementVerify;
    async snapshot() {
      return { boot: { pages: [], pageIds: [] }, log: [] };
    }
  },
}));
vi.mock('../src/live.js', () => ({
  Live: class {
    constructor(
      _mount: URL,
      _bootstrap: unknown,
      _registration: unknown,
      _page: unknown,
      signal: AbortSignal,
      remote: RemoteClient,
      owner: LiveSessionOwner,
    ) {
      setup.events.push('connection');
      setup.registrations.push(_registration);
      setup.signals.push(signal);
      setup.ports.push(remote);
      setup.owner = owner;
    }
    async snapshot() {
      return { id: 'page', source: '', title: '', sharing: 'private' };
    }
    close() {}
  },
}));

it('opens archived pages through the same verified mounted read binding', async () => {
  setup.events.length = 0;
  setup.pages = [
    { pageId: 'page', sharing: 'private' },
    { pageId: 'archived', sharing: 'private', archived: true },
  ];
  setup.register.mockReset().mockResolvedValueOnce(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  await mounted.transport.page('archived', new AbortController().signal);
  expect(setup.events).toEqual(['connection']);
  await expect(mounted.transport.page('missing')).rejects.toThrow('Page unavailable');
  expect(setup.events).toEqual(['connection']);
  setup.pages = [{ pageId: 'page', sharing: 'private' }];
});

it('replaces a verified mounted session once for concurrent reconnects and creates Remote before sync', async () => {
  setup.events.length = 0;
  setup.register.mockReset().mockResolvedValueOnce(setup.first);
  setup.remote.mockReset().mockImplementation(async (_mount, _sdk, session) => {
    setup.events.push(`remote:${session.id}`);
    return {};
  });
  const mounted = await mountedTransport();
  expect(setup.register).toHaveBeenCalledExactlyOnceWith(
    expect.any(URL),
    expect.objectContaining({
      reopenSession: expect.any(Function),
      certifyKey: expect.any(Function),
      transportUrl: expect.any(Function),
    }),
  );
  await mounted.transport.page('page', new AbortController().signal);
  expect(setup.events).toEqual(['remote:first', 'connection']);
  let release!: (value: typeof setup.second) => void;
  setup.register.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const one = setup.owner!.reconnect(setup.first as never);
  const two = setup.owner!.reconnect(setup.first as never);
  expect(one).toBe(two);
  release(setup.second);
  const replacement = await one;
  expect(replacement.registration).toBe(setup.second);
  expect(setup.register).toHaveBeenCalledTimes(2);
  // A reconnect is a bounded re-admission of that exact Session (Remote R2), under the tab's life.
  expect(setup.register).toHaveBeenLastCalledWith(expect.any(URL), expect.anything(), {
    previous: setup.first.remoteSession,
    signal: expect.any(AbortSignal),
  });
  expect(setup.verify).toHaveBeenLastCalledWith(setup.second, 'space', expect.any(Uint8Array));
  expect(setup.remote).toHaveBeenLastCalledWith(
    expect.any(URL),
    undefined,
    setup.second.remoteSession,
  );
  expect(await setup.owner!.reconnect(setup.first as never)).toBe(replacement);
  expect(setup.register).toHaveBeenCalledTimes(2);
  await mounted.transport.page('page', new AbortController().signal);
  expect(setup.events).toEqual(['remote:first', 'connection', 'remote:second', 'connection']);
  setup.register.mockResolvedValueOnce({ deviceId: 'other-device', remoteSession: {} });
  await expect(setup.owner!.reconnect(setup.second as never)).rejects.toThrow();
  expect(setup.remote).toHaveBeenCalledTimes(2);
  expect(await setup.owner!.reconnect(setup.first as never)).toBe(replacement);
});

it('closing a tab aborts page signals and fences Remote methods, certification and reconnect', async () => {
  setup.sdk.reopenSession.mockClear();
  setup.sdk.certifyKey.mockClear();
  setup.signals.length = 0;
  setup.ports.length = 0;
  const call = vi.fn(async () => ({}));
  setup.remote.mockReset().mockResolvedValue({
    context: call,
    listAgents: call,
    send: call,
    operation: call,
    result: call,
  });
  setup.register.mockReset().mockImplementation(async (_mount, sdk) => {
    await sdk.reopenSession();
    await sdk.certifyKey('sign', new Uint8Array(32));
    return setup.first;
  });
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  // The wrapper hands the Session and options to the paired SDK unchanged.
  const readmit = { retry: 'bounded', signal: new AbortController().signal } as const;
  await setup.register.mock.calls[0][1].reopenSession(setup.first.remoteSession, readmit);
  expect(setup.sdk.reopenSession).toHaveBeenLastCalledWith(setup.first.remoteSession, readmit);
  const remote = setup.ports[0];
  const sdk = setup.register.mock.calls[0][1];
  mounted.close();
  expect(setup.signals[0].aborted).toBe(true);
  for (const result of [
    remote.context(),
    remote.listAgents(),
    remote.send({ operationId: 'op', agentId: 'agent', message: 'message' }),
    remote.operation('op'),
    remote.result('request'),
  ])
    await expect(result).rejects.toMatchObject({ name: 'AbortError' });
  await expect(setup.owner!.reconnect(setup.first as never)).rejects.toMatchObject({
    name: 'AbortError',
  });
  await expect(sdk.reopenSession()).rejects.toMatchObject({ name: 'AbortError' });
  await expect(sdk.certifyKey('sign', new Uint8Array(32))).rejects.toMatchObject({
    name: 'AbortError',
  });
  await expect(mounted.transport.page('page')).rejects.toMatchObject({ name: 'AbortError' });
  expect(call).not.toHaveBeenCalled();
  // Registration's boot call and the forwarded call above; nothing after the tab closed.
  expect(setup.sdk.reopenSession).toHaveBeenCalledTimes(2);
  expect(setup.sdk.certifyKey).toHaveBeenCalledOnce();
  expect(setup.register).toHaveBeenCalledOnce();
});

it('closing a tab during explicit recovery prevents a late reload or reopen', async () => {
  const items = new Map<string, string>();
  const reload = vi.fn();
  vi.stubGlobal('sessionStorage', {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => {
      items.set(key, value);
    },
    removeItem: (key: string) => {
      items.delete(key);
    },
  });
  vi.stubGlobal('location', { reload });
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue(null);
  let release!: () => void;
  setup.sdk.reopenSession.mockReset().mockImplementation(
    () =>
      new Promise<object>((resolve) => {
        release = () => resolve({});
      }),
  );
  try {
    const mounted = await mountedTransport();
    await mounted.transport.page('page');
    const recovering = setup.owner!.recover!();
    expect(setup.sdk.reopenSession).toHaveBeenCalledOnce();
    mounted.close();
    release();
    await expect(recovering).rejects.toMatchObject({ name: 'AbortError' });
    expect(reload).not.toHaveBeenCalled();
    await expect(setup.owner!.recover!()).rejects.toMatchObject({ name: 'AbortError' });
    expect(setup.sdk.reopenSession).toHaveBeenCalledOnce();
  } finally {
    vi.unstubAllGlobals();
  }
});

it('mounted explicit recovery releases a failed network marker for the next trusted action', async () => {
  const items = new Map<string, string>();
  const reload = vi.fn();
  vi.stubGlobal('sessionStorage', {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => items.set(key, value),
    removeItem: (key: string) => items.delete(key),
  });
  vi.stubGlobal('location', { reload });
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue(null);
  const error = new TypeError('Failed to fetch');
  setup.sdk.reopenSession.mockReset().mockRejectedValueOnce(error).mockResolvedValueOnce({});
  const mounted = await mountedTransport();
  try {
    await mounted.transport.page('page');
    await expect(setup.owner!.recover!()).rejects.toBeInstanceOf(RecoveryRequiredError);
    expect(items.size).toBe(0);
    expect(reload).not.toHaveBeenCalled();
    expect(await setup.owner!.recover!()).toBe(true);
    expect(setup.sdk.reopenSession).toHaveBeenCalledTimes(2);
    expect(reload).toHaveBeenCalledOnce();
    expect([...items.values()]).toEqual(['attempted']);
  } finally {
    mounted.close();
    vi.unstubAllGlobals();
  }
});

it('closing during management signing prevents POST and does not adopt the late signed request', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue(null);
  setup.managementSend.mockClear();
  const mounted = await mountedTransport();
  const port = mounted.transport.management!;
  const view = await port.read('page');
  let release!: () => void;
  setup.managementPrepare.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = () => resolve({ id: 'operation' });
      }),
  );
  const preparing = port.prepare(view, { operation: 'page.archive', value: { pageId: 'page' } });
  mounted.close();
  release();
  await expect(preparing).rejects.toMatchObject({ name: 'AbortError' });
  await expect(port.send({} as Pending)).rejects.toMatchObject({ name: 'AbortError' });
  await expect(port.read('page')).rejects.toMatchObject({ name: 'AbortError' });
  expect(setup.managementSend).not.toHaveBeenCalled();
});

it('a replaced session refuses old management views and frozen retries but permits read-only verification', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValueOnce(setup.second);
  setup.remote.mockReset().mockResolvedValue(null);
  setup.managementSend.mockClear();
  setup.managementPrepare.mockResolvedValue({ id: 'operation' });
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  const port = mounted.transport.management!;
  const view = await port.read('page');
  const selection = { operation: 'page.archive' as const, value: { pageId: 'page' } };
  const pending = await port.prepare(view, selection);
  await setup.owner!.reconnect(setup.first as never);
  await expect(port.send(pending)).rejects.toMatchObject({ code: 'DENIED' });
  await expect(port.prepare(view, selection)).rejects.toMatchObject({ code: 'DENIED' });
  expect(setup.managementSend).not.toHaveBeenCalled();
  await port.verify(pending, {
    operationId: pending.id,
    membershipHead: { revision: '2', statementHash: 'hash' },
  });
  const refreshed = await port.read('page');
  await port.send(await port.prepare(refreshed, selection));
  expect(setup.managementSend).toHaveBeenCalledOnce();
  mounted.close();
});

it('a management POST already started cannot publish its result after tab close', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue(null);
  setup.managementSend.mockClear();
  const mounted = await mountedTransport();
  const port = mounted.transport.management!;
  const pending = await port.prepare(await port.read('page'), {
    operation: 'page.archive',
    value: { pageId: 'page' },
  });
  let release!: () => void;
  setup.managementSend.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = () => resolve({ operationId: 'operation' });
      }),
  );
  const sending = port.send(pending);
  mounted.close();
  release();
  await expect(sending).rejects.toMatchObject({ name: 'AbortError' });
  expect(setup.managementSend).toHaveBeenCalledOnce();
});

it('closing during acknowledgment verification cancels its read and refuses a late view', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue(null);
  setup.managementSend.mockClear();
  setup.managementVerify.mockClear();
  const mounted = await mountedTransport();
  const port = mounted.transport.management!;
  const pending = await port.prepare(await port.read('page'), {
    operation: 'page.archive',
    value: { pageId: 'page' },
  });
  const ack = {
    operationId: pending.id,
    membershipHead: { revision: '2', statementHash: 'hash' },
  };
  let release!: () => void;
  setup.managementVerify.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = () => resolve({ page: { pageId: 'page' } });
      }),
  );
  const verifying = port.verify(pending, ack);
  mounted.close();
  expect(setup.managementVerify).toHaveBeenCalledWith(
    pending,
    ack,
    expect.objectContaining({ aborted: true }),
  );
  release();
  await expect(verifying).rejects.toMatchObject({ name: 'AbortError' });
  expect(setup.managementSend).not.toHaveBeenCalled();
});

it('title hints from a replaced registration or closed tab cannot reach the cache', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValueOnce(setup.second);
  setup.remote.mockReset().mockResolvedValue(null);
  setup.titleRemember.mockClear();
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  const owner = setup.owner!;
  await owner.reconnect(setup.first as never);
  await owner.rememberTitle!('page', 'Old session', setup.first as never);
  expect(setup.titleRemember).not.toHaveBeenCalled();
  await owner.rememberTitle!('page', 'Current session', setup.second as never);
  expect(setup.titleRemember).toHaveBeenCalledExactlyOnceWith(
    'page',
    'Current session',
    expect.any(AbortSignal),
  );
  mounted.close();
  await owner.rememberTitle!('page', 'Late view', setup.second as never);
  expect(setup.titleRemember).toHaveBeenCalledOnce();
});

beforeEach(() => {
  setup.cachedTitles.clear();
  setup.titleConnections.length = 0;
  setup.titleRead.mockReset().mockResolvedValue({ title: 'Verified title' });
  setup.titleJoin.mockReset().mockResolvedValue(undefined);
  setup.discover.mockReset();
  setup.registrations.length = 0;
});

async function mountedPage() {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValue(setup.second);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  setup.discover.mockClear();
  return mounted;
}

it.each([403, 503])(
  'an HTTP %s page refusal refreshes the mounted session before rediscovery',
  async (status) => {
    const mounted = await mountedPage();
    setup.discover.mockRejectedValueOnce(new AdmissionRefusedError(status, '{}', 'DENIED'));
    const signal = new AbortController().signal;
    try {
      await mounted.transport.page('page', signal);
      expect(setup.register).toHaveBeenCalledTimes(2);
      expect(setup.discover).toHaveBeenCalledTimes(2);
      expect(setup.register).toHaveBeenLastCalledWith(expect.any(URL), expect.anything(), {
        previous: setup.first.remoteSession,
        signal: expect.any(AbortSignal),
      });
      expect(setup.registrations).toEqual([setup.first, setup.second]);
      expect(setup.verify).toHaveBeenLastCalledWith(setup.second, 'space', expect.any(Uint8Array));
    } finally {
      mounted.close();
    }
  },
);

it('a named terminal page refusal never refreshes the session', async () => {
  const mounted = await mountedPage();
  const failure = new AdmissionRefusedError(403, '{}', 'REMOTE_DEVICE_REVOKED');
  setup.discover.mockRejectedValueOnce(failure);
  try {
    await expect(mounted.transport.page('page')).rejects.toBe(failure);
    expect(setup.register).toHaveBeenCalledOnce();
    expect(setup.discover).toHaveBeenCalledOnce();
    expect(setup.registrations).toEqual([setup.first]);
  } finally {
    mounted.close();
  }
});

it.each(['discovery', 'refresh'])(
  'persistent HTTP refusal in %s stays inside five page attempts',
  async (stage) => {
    const mounted = await mountedPage();
    const failure = new AdmissionRefusedError(403, '{}', 'DENIED');
    setup.discover.mockRejectedValue(failure);
    if (stage === 'refresh') setup.register.mockRejectedValue(failure);
    try {
      const result = await reopenWithBackoff(() => mounted.transport.page('page'), {
        signal: new AbortController().signal,
        retryable: retryableReopen,
        wait: async () => {},
      });
      expect(result).toEqual({ error: failure });
      expect(setup.register).toHaveBeenCalledTimes(6);
      expect(setup.discover).toHaveBeenCalledTimes(stage === 'discovery' ? 10 : 5);
      expect(setup.registrations).toEqual([setup.first]);
    } finally {
      mounted.close();
    }
  },
);

it('aborting a page refresh refuses a late registration and keeps the prior mounted session', async () => {
  const mounted = await mountedPage();
  setup.discover.mockRejectedValueOnce(new AdmissionRefusedError(403, '{}', 'DENIED'));
  let release!: (value: typeof setup.second) => void;
  setup.register.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const controller = new AbortController();
  const opening = mounted.transport.page('page', controller.signal);
  const failure = opening.catch((error: unknown) => error);
  await vi.waitFor(() => expect(setup.register).toHaveBeenCalledTimes(2));
  const refreshSignal = setup.register.mock.lastCall![2].signal as AbortSignal;
  controller.abort();
  expect(refreshSignal.aborted).toBe(true);
  release(setup.second);
  expect(await failure).toMatchObject({ name: 'AbortError' });
  expect(setup.remote).toHaveBeenCalledOnce();
  await mounted.transport.page('page');
  expect(setup.registrations).toEqual([setup.first, setup.first]);
  mounted.close();
});

it('a page refresh and concurrent Live reconnect share one verified replacement', async () => {
  const mounted = await mountedPage();
  setup.discover.mockRejectedValueOnce(new AdmissionRefusedError(403, '{}', 'DENIED'));
  let release!: (value: typeof setup.second) => void;
  setup.register.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        release = resolve;
      }),
  );
  const opening = mounted.transport
    .page('page', new AbortController().signal)
    .catch((error: unknown) => error);
  await vi.waitFor(() => expect(setup.register).toHaveBeenCalledTimes(2));
  const reconnecting = setup.owner!.reconnect(setup.first as never);
  release(setup.second);
  expect(await opening).toMatchObject({ id: 'page' });
  expect((await reconnecting).registration).toBe(setup.second);
  expect(setup.register).toHaveBeenCalledTimes(2);
  expect(setup.registrations).toEqual([setup.first, setup.second]);
  mounted.close();
});

it('fills an uncached title with a joined read binding and never opens Live or Ask', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  setup.pages = [{ pageId: 'page', sharing: 'private' }];
  const mounted = await mountedTransport();
  setup.events.length = 0;
  try {
    const budget = { remainingBytes: 1024 };
    expect(await mounted.transport.readTitle!('page', budget, new AbortController().signal)).toBe(
      'Verified title',
    );
    expect(setup.events).toEqual([]);
    const connection = setup.titleConnections[0];
    expect(connection.args.slice(5)).toEqual([['colab-sync-v1'], true, budget]);
    expect(connection.join).toHaveBeenCalledOnce();
    expect(setup.cachedTitles.get('page')).toBe('Verified title');
    setup.discover.mockClear();
    expect(await mounted.transport.readTitle!('page', budget, new AbortController().signal)).toBe(
      'Verified title',
    );
    expect(setup.discover).not.toHaveBeenCalled();
    expect(setup.titleConnections).toHaveLength(1);
  } finally {
    mounted.close();
  }
});

it('joins a cancelled old generation before opening its successor and drops the late title', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  let release!: () => void;
  setup.titleRead.mockReturnValueOnce(new Promise(() => {}));
  setup.titleJoin.mockReturnValueOnce(
    new Promise<void>((done) => {
      release = done;
    }),
  );
  const old = new AbortController(),
    budget = { remainingBytes: 1024 };
  const first = mounted.transport.readTitle!('page', budget, old.signal).catch(
    (error: unknown) => error,
  );
  await vi.waitFor(() => expect(setup.titleConnections).toHaveLength(1));
  old.abort();
  const next = mounted.transport.readTitle!('page', budget, new AbortController().signal);
  await Promise.resolve();
  expect(setup.titleConnections).toHaveLength(1);
  expect(setup.cachedTitles.has('page')).toBe(false);
  release();
  await first;
  expect(await next).toBe('Verified title');
  expect(setup.titleConnections).toHaveLength(2);
  mounted.close();
});

it('session replacement cancels an in-flight title fold without caching its result', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValue(setup.second);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  setup.titleRead.mockReturnValueOnce(new Promise(() => {}));
  const reading = mounted.transport.readTitle!(
    'page',
    { remainingBytes: 1024 },
    new AbortController().signal,
  ).catch((error: unknown) => error);
  await vi.waitFor(() => expect(setup.titleConnections).toHaveLength(1));
  await setup.owner!.reconnect(setup.first as never);
  expect(await reading).toBeInstanceOf(Error);
  expect(setup.titleConnections[0].join).toHaveBeenCalledOnce();
  expect(setup.cachedTitles.has('page')).toBe(false);
  mounted.close();
});

it('an unconfirmed read close blocks successors across Home generations', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  setup.titleJoin.mockRejectedValueOnce(new Error('Title read cleanup unconfirmed'));
  const budget = { remainingBytes: 1024 },
    signal = new AbortController().signal;
  try {
    await expect(mounted.transport.readTitle!('page', budget, signal)).rejects.toThrow(
      'cleanup unconfirmed',
    );
    expect(budget.remainingBytes).toBe(0);
    await expect(
      mounted.transport.readTitle!('page', { remainingBytes: 1024 }, signal),
    ).rejects.toThrow();
    expect(setup.titleConnections).toHaveLength(1);
    expect(setup.cachedTitles.has('page')).toBe(false);
  } finally {
    mounted.close();
  }
});

it('reads Remote presence with the admitted mounted Session and exposes settings only after success', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  setup.sdk.management.mockClear();
  const mounted = await mountedTransport();
  const presence = mounted.transport.remotePresence!;
  expect(presence.snapshot()).toEqual({ kind: 'hidden' });
  await presence.refresh();
  expect(setup.sdk.management).toHaveBeenCalledWith(setup.first.remoteSession, {
    timeoutMs: 20000,
  });
  expect(presence.snapshot()).toMatchObject({
    kind: 'confirmed',
    count: 0,
    settingsUrl: 'https://example.test/settings',
  });
  expect(setup.register).toHaveBeenCalledTimes(1);
  mounted.close();
  expect(presence.snapshot()).toEqual({ kind: 'hidden' });
});

it('a replacement Session reads once after the stale Remote snapshot settles', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValueOnce(setup.second);
  setup.remote.mockReset().mockResolvedValue({});
  setup.sdk.management.mockClear();
  let release!: (value: { devices: []; nextCursor: null }) => void;
  let releaseSecond!: (value: { devices: []; nextCursor: null }) => void;
  const firstRead = vi.fn(
    () =>
      new Promise<{ devices: []; nextCursor: null }>((resolve) => {
        release = resolve;
      }),
  );
  const secondRead = vi.fn(
    () =>
      new Promise<{ devices: []; nextCursor: null }>((resolve) => {
        releaseSecond = resolve;
      }),
  );
  setup.sdk.management
    .mockImplementationOnce(() => ({ devices: firstRead }))
    .mockImplementationOnce(() => ({ devices: secondRead }));
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  const presence = mounted.transport.remotePresence!;
  const reading = presence.refresh();
  await setup.owner!.reconnect(setup.first as never);
  expect(firstRead).toHaveBeenCalledTimes(1);
  expect(secondRead).not.toHaveBeenCalled();
  release({ devices: [], nextCursor: null });
  await reading;
  await vi.waitFor(() => expect(secondRead).toHaveBeenCalledTimes(1));
  expect(presence.snapshot()).toEqual({ kind: 'hidden' });
  expect(setup.sdk.management.mock.calls.map(([session]) => session)).toEqual([
    setup.first.remoteSession,
    setup.second.remoteSession,
  ]);
  releaseSecond({ devices: [], nextCursor: null });
  await presence.refresh();
  expect(presence.snapshot()).toMatchObject({ kind: 'confirmed', count: 0 });
  expect(secondRead).toHaveBeenCalledTimes(1);
  mounted.close();
});

it('joins a canceled chapter-details request across Home generations without opening Live', async () => {
  setup.register.mockReset().mockResolvedValue(setup.first);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  let release!: (response: Response) => void;
  const pageId = '10000000-0000-4000-8000-000000000001';
  const fetcher = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<Response>((done) => {
          release = done;
        }),
    )
    .mockResolvedValueOnce(Response.json({ pageId, state: 'unavailable' }));
  vi.stubGlobal('fetch', fetcher);
  setup.events.length = 0;
  try {
    const old = new AbortController();
    const first = mounted.transport.readingDetails!(pageId, old.signal).catch(
      (error: unknown) => error,
    );
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    old.abort();
    const successor = mounted.transport.readingDetails!(pageId, new AbortController().signal);
    await Promise.resolve();
    expect(fetcher).toHaveBeenCalledTimes(1);
    release(Response.json({ pageId, state: 'pending' }));
    expect(await first).toBeInstanceOf(Error);
    expect(await successor).toEqual({ pageId, state: 'unavailable' });
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(setup.events).toEqual([]);
    expect(setup.titleConnections).toHaveLength(0);
  } finally {
    mounted.close();
    vi.unstubAllGlobals();
  }
});

it('fences chapter metadata returned by a replaced Session and admits the successor read', async () => {
  setup.register.mockReset().mockResolvedValueOnce(setup.first).mockResolvedValue(setup.second);
  setup.remote.mockReset().mockResolvedValue({});
  const mounted = await mountedTransport();
  await mounted.transport.page('page');
  const pageId = '10000000-0000-4000-8000-000000000001';
  let release!: (response: Response) => void;
  const fetcher = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<Response>((done) => {
          release = done;
        }),
    )
    .mockResolvedValueOnce(Response.json({ pageId, state: 'pending' }));
  vi.stubGlobal('fetch', fetcher);
  try {
    const first = mounted.transport.readingDetails!(pageId, new AbortController().signal).catch(
      (error: unknown) => error,
    );
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    await setup.owner!.reconnect(setup.first as never);
    const successor = mounted.transport.readingDetails!(pageId, new AbortController().signal);
    release(Response.json({ pageId, state: 'pending' }));
    expect(await first).toBeInstanceOf(Error);
    expect(await successor).toEqual({ pageId, state: 'pending' });
    expect(fetcher).toHaveBeenCalledTimes(2);
  } finally {
    mounted.close();
    vi.unstubAllGlobals();
  }
});
