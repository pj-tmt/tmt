import { expect, it, vi } from 'vite-plus/test';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
import type { LiveAskOptions } from '../src/live-ask.js';
import { SessionEndedError, type RemoteClient } from '../src/ask-remote.js';
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
  instances: [] as { options: LiveAskOptions; close: ReturnType<typeof vi.fn> }[],
}));
vi.mock('../src/live-ask.js', () => ({
  LiveAsk: class {
    close = vi.fn();
    constructor(readonly options: LiveAskOptions) {
      asks.instances.push(this);
    }
    async observe(signal: AbortSignal) {
      asks.signals.push(signal);
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
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    { deviceId: 'device' } as Registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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
      { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
      registration,
      { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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
    { space: 'uqvpga22vglwpngpg7jd7zpk5vzjtoud', owner: new Uint8Array(32) } as Bootstrap,
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
    { space: 'uqvpga22vglwpngpg7jd7zpk5vzjtoud', owner: new Uint8Array(32) } as Bootstrap,
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
    c.publish({ source: 'slow', title: 'Page' });
    await vi.waitFor(() => expect(release).toBeDefined());
    for (let n = 0; n < 20; n++) c.publish({ source: `latest ${n}`, title: 'Page' });
    expect(await (await live.export()).blob('page.html').text()).toBe('latest 19');
    expect(seen).toEqual(['verified']);
    release();
    expect((await live.snapshot()).source).toBe('latest 19');
    expect(seen).toEqual(['verified', 'latest 19']);
    expect(asks.project).toHaveBeenCalledTimes(3);
  } finally {
    live.close();
  }
});

it('page observer stops on hidden/close and resumes visible without another controller or send', async () => {
  const document = Object.assign(new EventTarget(), { visibilityState: 'visible' });
  vi.stubGlobal('document', document);
  asks.signals.length = 0;
  const live = new Live(
    new URL('https://example.test/colab/'),
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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
    document.visibilityState = 'hidden';
    document.dispatchEvent(new Event('visibilitychange'));
    expect(asks.signals[0].aborted).toBe(true);
    document.visibilityState = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(asks.signals).toHaveLength(2));
    expect(asks.signals[1].aborted).toBe(false);
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
      { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
      first,
      { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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

it('mounted ownership loss closes the Ask controller, observer and tunnel without session recovery', async () => {
  asks.instances.length = 0;
  asks.signals.length = 0;
  const lifetime = new AbortController();
  const reconnect = vi.fn();
  const live = new Live(
    new URL('https://example.test/colab/'),
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    {
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
    } as unknown as Registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
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
    { space: 'space', owner: new Uint8Array(32) } as Bootstrap,
    registration,
    { pageId: 'page', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    null,
    { reconnect: async () => ({ registration, remote: null }), rememberTitle },
  );
  try {
    await live.snapshot();
    expect(rememberTitle).toHaveBeenCalledWith('page', 'Page', registration);
    connections.at(-1)!.publish({ source: 'renamed source', title: 'Renamed' });
    expect((await live.snapshot()).title).toBe('Renamed');
    expect(rememberTitle).toHaveBeenLastCalledWith('page', 'Renamed', registration);
    asks.project.mockRejectedValueOnce(new Error('Rejected own evidence'));
    connections.at(-1)!.publish({ source: 'rejected', title: 'Do not cache' });
    await live.snapshot();
    expect(rememberTitle).not.toHaveBeenCalledWith('page', 'Do not cache', registration);
  } finally {
    live.close();
  }
});
