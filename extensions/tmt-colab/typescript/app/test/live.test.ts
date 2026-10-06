import { expect, it, vi } from 'vite-plus/test';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
import type { LiveAskOptions } from '../src/live-ask.js';
import {
  createRemoteClient,
  SessionEndedError,
  SessionEvictedError,
  type RemoteClient,
} from '../src/ask-remote.js';
import type { PageView } from '../src/transport.js';
import type { Registration } from '../src/registration.js';
import { Live } from '../src/live.js';

const connections = vi.hoisted(
  () =>
    [] as {
      failed(error: Error): void;
      publish(value: PageView): void;
      close: ReturnType<typeof vi.fn>;
      admission: {
        head: { revision: bigint; hash: Uint8Array } | null;
        root: object | null;
        validatePage: ReturnType<typeof vi.fn>;
        registration: Registration;
      };
    }[],
);
const asks = vi.hoisted(() => ({
  project: vi.fn(async () => []),
  signals: [] as AbortSignal[],
  activation: [] as boolean[],
  instances: [] as { options: LiveAskOptions; close: ReturnType<typeof vi.fn> }[],
}));
vi.mock('../src/live-ask.js', () => ({
  LiveAsk: class {
    close = vi.fn();
    constructor(readonly options: LiveAskOptions) {
      asks.instances.push(this);
    }
    async observe(signal: AbortSignal, refreshOlder = false) {
      asks.signals.push(signal);
      asks.activation.push(refreshOlder);
      await new Promise<void>((resolve) =>
        signal.addEventListener('abort', () => resolve(), { once: true }),
      );
    }
  },
  pageAsks: asks.project,
}));
vi.mock('../src/admission.js', () => ({
  Admission: class {
    head = { revision: 2n, hash: new Uint8Array(32).fill(10) };
    root = {};
    validatePage = vi.fn();
    constructor(
      readonly space: string,
      readonly page: string,
      readonly epoch: string,
      _owner: Uint8Array,
      readonly registration: Registration,
    ) {}
    async restore() {}
  },
}));
vi.mock('../src/connection.js', () => ({
  Connection: class {
    objects = { ownSigningKey: () => undefined };
    ready = Promise.resolve({ source: 'verified', title: 'Page' });
    constructor(
      readonly admission: (typeof connections)[number]['admission'],
      _mount: URL,
      _sharing: string,
      readonly publish: (value: PageView) => void,
      readonly failed: (error: Error) => void,
    ) {
      connections.push(this);
      publish({ source: 'verified', title: 'Page' });
    }
    async run<T>(fn: () => Promise<T>) {
      return fn();
    }
    close = vi.fn();
  },
}));
vi.mock('../src/writer.js', () => ({
  Writer: class {
    close() {}
  },
}));

it('successful catchup resets reconnect failures across the page lifetime', async () => {
  connections.length = 0;
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
  );
  const failed = vi.fn();
  live.subscribe(() => {}, failed);
  try {
    await live.snapshot();
    for (let attempt = 0; attempt < 5; attempt++) {
      connections.at(-1)!.failed(new Error('Sync disconnected'));
      expect((await live.snapshot()).source).toBe('verified');
      expect(connections).toHaveLength(attempt + 2);
    }
    expect(failed).not.toHaveBeenCalled();
  } finally {
    live.close();
  }
});

it('tunnel disconnects and resyncs preserve the Session and Remote without reopening or stopping another page', async () => {
  asks.instances.length = 0;
  const registration = {
    deviceId: 'device',
    keys: { sign: {}, signPublic: new Uint8Array(32) },
    remoteSession: {},
  } as unknown as Registration;
  const remote = {} as RemoteClient;
  const reopenSession = vi.fn(async () => ({}));
  const reconnect = vi.fn(async () => ({
    registration: { ...registration, remoteSession: await reopenSession() },
    remote,
  }));
  const create = () =>
    new Live(
      new URL('https://example.test/colab/'),
      {
        space: 'space',
        revision: '1',
        owner: new Uint8Array(32),
        pageIds: [],
        pages: [],
      } as Bootstrap,
      registration,
      {
        pageId: '10000000-0000-4000-8000-000000000001',
        epoch: '1',
        sharing: 'private',
      } as PageInfo,
      undefined,
      remote,
      { reconnect },
    );
  const live = create(),
    other = create();
  try {
    await Promise.all([live.snapshot(), other.snapshot()]);
    const otherConnection = connections.at(-1)!;
    const otherAsk = asks.instances[1];
    let connection = connections.at(-2)!;
    let controller = asks.instances[0];
    for (const reason of [
      'Sync disconnected',
      'RESYNC_REQUIRED',
      'Fresh membership catchup required',
    ]) {
      connection.failed(new Error(reason));
      expect((await live.snapshot()).source).toBe('verified');
      expect(connection.close).toHaveBeenCalledOnce();
      expect(controller.close).toHaveBeenCalledOnce();
      expect(live.registration).toBe(registration);
      controller = asks.instances.at(-1)!;
      expect(controller.options.remote).toBe(remote);
      connection = connections.at(-1)!;
      expect(otherConnection.close).not.toHaveBeenCalled();
      expect(otherAsk.close).not.toHaveBeenCalled();
      expect(other.registration).toBe(registration);
    }
    expect(reconnect).not.toHaveBeenCalled();
    expect(reopenSession).not.toHaveBeenCalled();
  } finally {
    live.close();
    other.close();
  }
});

it('exports admitted committed view/head and denies blocked, missing-key and closed bindings', async () => {
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'uqvpga22vglwpngpg7jd7zpk5vzjtoud',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: '00000000-0000-4000-8000-000000000002', epoch: '1', sharing: 'private' } as PageInfo,
  );
  try {
    await live.snapshot();
    const c = connections.at(-1)!;
    const bundle = await live.export();
    expect(await bundle.blob('page.html').text()).toBe('verified');
    const manifest = JSON.parse(await bundle.blob('manifest.json').text());
    expect(manifest.title).toBe('Page');
    expect(manifest).not.toHaveProperty('originalAuthor');
    expect(manifest).not.toHaveProperty('publisherAgent');
    expect(manifest.membershipHead).toEqual({ revision: '2', statementHash: '0a'.repeat(32) });
    expect(manifest.epoch).toBe('1');
    expect(c.admission.validatePage).toHaveBeenCalledWith('private');
    c.admission.root = null;
    await expect(live.export()).rejects.toThrow();
    c.admission.root = {};
    c.admission.head = null;
    await expect(live.export()).rejects.toThrow();
    c.failed(new Error('Invalid signature'));
    await expect(live.export()).rejects.toThrow();
  } finally {
    live.close();
  }
  await expect(live.export()).rejects.toThrow();
});

it('slow Ask verification keeps one latest view while source export reads the committed decoder state', async () => {
  asks.project.mockClear();
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'uqvpga22vglwpngpg7jd7zpk5vzjtoud',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: '00000000-0000-4000-8000-000000000002', epoch: '1', sharing: 'private' } as PageInfo,
  );
  const seen: string[] = [];
  try {
    await live.snapshot();
    live.subscribe(
      (value) => seen.push(value.source),
      () => {},
    );
    let release!: () => void;
    asks.project.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve([]);
        }),
    );
    const c = connections.at(-1)!;
    c.publish({
      source: 'slow',
      title: 'Page',
      originalAuthor: 'Alice creator',
      publisherAgent: 'Bob slow',
    });
    await vi.waitFor(() => expect(release).toBeDefined());
    for (let n = 0; n < 20; n++)
      c.publish({
        source: `latest ${n}`,
        title: 'Page',
        originalAuthor: 'Alice creator',
        publisherAgent: `Bob ${n}`,
      });
    const frozen = await live.export();
    expect(await frozen.blob('page.html').text()).toBe('latest 19');
    const manifest = JSON.parse(await frozen.blob('manifest.json').text());
    expect(manifest.originalAuthor).toBe('Alice creator');
    expect(manifest.publisherAgent).toBe('Bob 19');
    expect(seen).toEqual(['verified']);
    release();
    const latest = await live.snapshot();
    expect(latest.source).toBe('latest 19');
    expect(latest.originalAuthor).toBe('Alice creator');
    expect(latest.publisherAgent).toBe('Bob 19');
    expect(seen).toEqual(['verified', 'latest 19']);
    expect(asks.project).toHaveBeenCalledTimes(3);
    c.publish({ source: 'next', title: 'Page', originalAuthor: 'Alice creator' });
    await vi.waitFor(async () => expect((await live.snapshot()).source).toBe('next'));
    expect((await live.snapshot()).originalAuthor).toBe('Alice creator');
    expect((await live.snapshot()).publisherAgent).toBeUndefined();
    expect(JSON.parse(await frozen.blob('manifest.json').text())).toEqual(manifest);
    expect(await frozen.blob('page.html').text()).toBe('latest 19');
  } finally {
    live.close();
  }
});

it('page observer stops on hidden/close and resumes visible without another controller or send', async () => {
  const document = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  vi.stubGlobal('document', document);
  asks.signals.length = 0;
  asks.activation.length = 0;
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    {} as RemoteClient,
  );
  try {
    await live.snapshot();
    const unsubscribe = live.subscribe(
      () => {},
      () => {},
    );
    expect(asks.signals).toHaveLength(1);
    expect(asks.activation).toEqual([true]);
    document.visibilityState = 'hidden';
    document.dispatchEvent(new Event('visibilitychange'));
    expect(asks.signals[0].aborted).toBe(true);
    document.visibilityState = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(asks.signals).toHaveLength(2));
    expect(asks.signals[1].aborted).toBe(false);
    expect(asks.activation).toEqual([true, true]);
    unsubscribe();
    await vi.waitFor(() => expect(asks.signals[1].aborted).toBe(true));
  } finally {
    live.close();
    vi.unstubAllGlobals();
  }
});

it.each(['callback', 'message', 'typed error'] as const)(
  'session-end (%s) replaces once before reconnect and coalesces duplicate signals',
  async (trigger) => {
    asks.instances.length = 0;
    asks.signals.length = 0;
    const first = {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
      remoteSession: {},
    } as unknown as Registration;
    const nextSession = {};
    const second = { ...first, remoteSession: nextSession };
    const remote = {} as RemoteClient,
      replacementRemote = {} as RemoteClient;
    let release!: (session: object) => void;
    const reopenSession = vi.fn(
      () =>
        new Promise<object>((resolve) => {
          release = resolve;
        }),
    );
    const reconnect = vi.fn(async () => {
      const session = await reopenSession();
      expect(session).toBe(nextSession);
      return { registration: second, remote: replacementRemote };
    });
    const live = new Live(
      new URL('https://example.test/colab/'),
      {
        space: 'space',
        revision: '1',
        owner: new Uint8Array(32),
        pageIds: [],
        pages: [],
      } as Bootstrap,
      first,
      {
        pageId: '10000000-0000-4000-8000-000000000001',
        epoch: '1',
        sharing: 'private',
      } as PageInfo,
      undefined,
      remote,
      { reconnect },
    );
    try {
      await live.snapshot();
      live.subscribe(
        () => {},
        () => {},
      );
      const previous = asks.instances[0];
      const connection = connections.at(-1)!;
      expect(previous.options.remote).toBe(remote);
      const end = () => {
        if (trigger === 'callback') previous.options.sessionEnded?.();
        else
          connection.failed(
            trigger === 'message'
              ? new Error('Remote session ended')
              : new SessionEndedError('REMOTE_SESSION_ENDED'),
          );
      };
      end();
      end();
      expect(reconnect).toHaveBeenCalledExactlyOnceWith(first);
      expect(reopenSession).toHaveBeenCalledOnce();
      expect(previous.close).toHaveBeenCalledOnce();
      expect(connection.close).toHaveBeenCalledOnce();
      release(nextSession);
      await live.snapshot();
      expect(reconnect).toHaveBeenCalledOnce();
      expect(reopenSession).toHaveBeenCalledOnce();
      expect(live.registration).toBe(second);
      expect(asks.instances).toHaveLength(2);
      expect(asks.instances[1].options.remote).toBe(replacementRemote);
      await vi.waitFor(() => expect(asks.signals).toHaveLength(2));
      expect(asks.signals[0].aborted).toBe(true);
      expect(asks.signals[1].aborted).toBe(false);
    } finally {
      live.close();
    }
  },
);

it.each([
  ['evicted', new SessionEvictedError(8, 'https://example.test/remote/settings'), false],
  ['ended', new SessionEndedError('REMOTE_SESSION_ENDED'), true],
] as const)(
  'a closed mounted socket distinguishes %s from ordinary session end',
  async (_, reason, reopen) => {
    const registration = {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration;
    const remote = {
      listAgents: vi.fn(async () => {
        throw reason;
      }),
    } as unknown as RemoteClient;
    const reconnect = vi.fn(async () => ({ registration: { ...registration }, remote }));
    const live = new Live(
      new URL('https://example.test/colab/'),
      {
        space: 'space',
        revision: '1',
        owner: new Uint8Array(32),
        pageIds: [],
        pages: [],
      } as Bootstrap,
      registration,
      {
        pageId: '10000000-0000-4000-8000-000000000001',
        epoch: '1',
        sharing: 'private',
      } as PageInfo,
      undefined,
      remote,
      { reconnect },
    );
    const failed = vi.fn();
    try {
      await live.snapshot();
      live.subscribe(() => {}, failed);
      const before = connections.length;
      connections.at(-1)!.failed(new Error('Sync disconnected'));
      await vi.waitFor(() => expect(remote.listAgents).toHaveBeenCalledOnce());
      if (reopen) {
        await vi.waitFor(() => expect(reconnect).toHaveBeenCalledOnce());
        await vi.waitFor(() => expect(connections).toHaveLength(before + 1));
        expect(failed).not.toHaveBeenCalled();
      } else {
        await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
        expect(failed.mock.calls[0][0]).toBe(reason);
        expect(reconnect).not.toHaveBeenCalled();
        expect(connections).toHaveLength(before);
      }
    } finally {
      live.close();
    }
  },
);

it('mounted ownership loss closes the Ask controller, observer and tunnel without session recovery', async () => {
  asks.instances.length = 0;
  asks.signals.length = 0;
  const lifetime = new AbortController();
  const reconnect = vi.fn();
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    lifetime.signal,
    {} as RemoteClient,
    { reconnect },
  );
  try {
    await live.snapshot();
    live.subscribe(
      () => {},
      () => {},
    );
    const controller = asks.instances[0],
      connection = connections.at(-1)!;
    expect(asks.signals).toHaveLength(1);
    lifetime.abort();
    expect(asks.signals[0].aborted).toBe(true);
    expect(controller.close).toHaveBeenCalledOnce();
    expect(connection.close).toHaveBeenCalledOnce();
    controller.options.sessionEnded?.();
    connection.failed(new Error('Sync disconnected'));
    expect(reconnect).not.toHaveBeenCalled();
    expect(asks.instances).toHaveLength(1);
  } finally {
    live.close();
  }
});

it('explicit recovery stops Live, Ask and observation before reopening, with no automatic retry', async () => {
  const registration = {
    deviceId: 'device',
    keys: { sign: {}, signPublic: new Uint8Array(32) },
  } as unknown as Registration;
  let connection: (typeof connections)[number];
  let ask: (typeof asks.instances)[number];
  let signal: AbortSignal;
  const recover = vi.fn(async () => {
    expect(connection.close).toHaveBeenCalledOnce();
    expect(ask.close).toHaveBeenCalledOnce();
    expect(signal.aborted).toBe(true);
    return false;
  });
  const reconnect = vi.fn();
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    registration,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    {} as RemoteClient,
    { reconnect, recover },
  );
  live.subscribe(
    () => {},
    () => {},
  );
  await live.snapshot();
  connection = connections.at(-1)!;
  ask = asks.instances.at(-1)!;
  signal = asks.signals.at(-1)!;
  expect(await live.reconnect()).toBe(false);
  expect(await live.reconnect()).toBe(false);
  expect(recover).toHaveBeenCalledOnce();
  expect(reconnect).not.toHaveBeenCalled();
});

it('remembers only accepted fold titles under the exact admission registration', async () => {
  connections.length = 0;
  asks.project.mockResolvedValue([]);
  const registration = { deviceId: 'device' } as Registration;
  const rememberTitle = vi.fn(async () => {});
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    registration,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    null,
    { reconnect: async () => ({ registration, remote: null }), rememberTitle },
  );
  try {
    await live.snapshot();
    expect(rememberTitle).toHaveBeenCalledWith(
      '10000000-0000-4000-8000-000000000001',
      'Page',
      registration,
    );
    connections.at(-1)!.publish({ source: 'renamed source', title: 'Renamed' });
    expect((await live.snapshot()).title).toBe('Renamed');
    expect(rememberTitle).toHaveBeenLastCalledWith(
      '10000000-0000-4000-8000-000000000001',
      'Renamed',
      registration,
    );
    asks.project.mockRejectedValueOnce(new Error('Rejected own evidence'));
    connections.at(-1)!.publish({ source: 'rejected', title: 'Do not cache' });
    await live.snapshot();
    expect(rememberTitle).not.toHaveBeenCalledWith(
      '10000000-0000-4000-8000-000000000001',
      'Do not cache',
      registration,
    );
  } finally {
    live.close();
  }
});

it.each(['send', 'operation'] as const)(
  'a returned %s eviction survives opaque socket close before the Ask page callback',
  async (method) => {
    class RefusalError extends Error {
      constructor(readonly code: string) {
        super(code);
      }
    }
    const operationId = '00000000-0000-4000-8000-000000000009';
    const ops = {
      listAgents: vi.fn(async () => {
        throw new RefusalError('REMOTE_SESSION_ENDED');
      }),
      send: vi.fn(async () => ({
        state: 'refused' as const,
        operationId,
        reason: 'REMOTE_SESSION_EVICTED',
        limit: 8,
        settingsUrl: 'https://example.test/remote/settings',
      })),
      operation: vi.fn(async () => ({
        state: 'refused' as const,
        operationId,
        reason: 'REMOTE_SESSION_EVICTED',
        limit: 8,
        settingsUrl: 'https://example.test/remote/settings',
      })),
      result: vi.fn(async () => ({ state: 'pending' as const })),
    };
    const remote = await createRemoteClient(
      new URL('https://example.test/colab/'),
      { RefusalError, operations: () => ops },
      { sessionId: operationId, serverTimeMs: Date.now(), grantRevision: 1, expiresAtMs: null },
    );
    const registration = {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration;
    const reconnect = vi.fn(async () => ({ registration, remote }));
    const live = new Live(
      new URL('https://example.test/colab/'),
      {
        space: 'space',
        revision: '1',
        owner: new Uint8Array(32),
        pageIds: [],
        pages: [],
      } as Bootstrap,
      registration,
      {
        pageId: '10000000-0000-4000-8000-000000000001',
        epoch: '1',
        sharing: 'private',
      } as PageInfo,
      undefined,
      remote,
      { reconnect },
    );
    const failed = vi.fn();
    try {
      await live.snapshot();
      live.subscribe(() => {}, failed);
      const connection = connections.at(-1)!;
      if (method === 'send')
        await remote.send({ operationId, agentId: operationId, message: 'exact' });
      else await remote.operation(operationId);
      connection.failed(new Error('Sync disconnected'));
      await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
      expect(failed.mock.calls[0][0]).toBeInstanceOf(SessionEvictedError);
      expect(failed.mock.calls[0][0]).toMatchObject({
        limit: 8,
        settingsUrl: 'https://example.test/remote/settings',
      });
      connection.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
      connection.failed(new Error('Sync disconnected'));
      expect(reconnect).not.toHaveBeenCalled();
      expect(ops.listAgents).not.toHaveBeenCalled();
      expect(ops.send).toHaveBeenCalledTimes(method === 'send' ? 1 : 0);
      expect(ops.operation).toHaveBeenCalledTimes(method === 'operation' ? 1 : 0);
    } finally {
      live.close();
    }
  },
);
