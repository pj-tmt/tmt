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
import { RecoveryRequiredError } from '../src/session-recovery.js';
import {
  SaveNotApplied,
  SaveOutcomeUnknown,
  SaveRefused,
  SaveTooLarge,
  type SaveResult,
} from '../src/save.js';

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
const connectionPlans = vi.hoisted(
  () =>
    [] as {
      ready: Promise<PageView>;
      constructed(): void;
      closed(): void;
    }[],
);
const saves = vi.hoisted(() => ({ save: vi.fn(), status: vi.fn() }));
const mutations = vi.hoisted(() => ({
  submitOwn: vi.fn(),
  submitOwnRecords: vi.fn(),
  submitRecords: vi.fn(),
}));
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
      const plan = connectionPlans.shift();
      if (plan) {
        this.ready = plan.ready;
        this.close.mockImplementation(plan.closed);
        plan.constructed();
      } else publish({ source: 'verified', title: 'Page' });
    }
    async run<T>(fn: () => Promise<T>) {
      return fn();
    }
    save(operationId: string, base: string, source: string) {
      return saves.save(this, operationId, base, source);
    }
    saveStatus(operationId: string) {
      return saves.status(this, operationId);
    }
    get active() {
      return this.close.mock.calls.length === 0;
    }
    close = vi.fn();
  },
}));
vi.mock('../src/writer.js', () => ({
  Writer: class {
    submitOwn = mutations.submitOwn;
    submitOwnRecords = mutations.submitOwnRecords;
    submitRecords = mutations.submitRecords;
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
    const seen: PageView[] = [];
    const unsubscribe = live.subscribe(
      (view) => seen.push(view),
      () => {},
    );
    expect(asks.signals).toHaveLength(1);
    expect(asks.activation).toEqual([true]);
    const health = asks.instances.at(-1)!.options.observationUnavailable!;
    health(true);
    expect(seen.at(-1)?.askUnavailable).toBe(true);
    connections.at(-1)!.publish({ source: 'new source', title: 'Page' });
    await live.snapshot();
    expect(seen.at(-1)?.askUnavailable).toBe(true);
    document.visibilityState = 'hidden';
    document.dispatchEvent(new Event('visibilitychange'));
    expect(asks.signals[0].aborted).toBe(true);
    const beforeHidden = seen.length;
    health(false);
    expect(seen).toHaveLength(beforeHidden);
    document.visibilityState = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.waitFor(() => expect(asks.signals).toHaveLength(2));
    expect(asks.signals[1].aborted).toBe(false);
    expect(asks.activation).toEqual([true, true]);
    health(false);
    expect(seen.at(-1)?.askUnavailable).toBe(false);
    unsubscribe();
    await vi.waitFor(() => expect(asks.signals[1].aborted).toBe(true));
    const beforeClose = seen.length;
    health(true);
    expect(seen).toHaveLength(beforeClose);
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

/** Models adapter-normalized errors and Remote's last-transport completion;
 * it does not verify a signed Remote response or run a real mounted socket. */
function heldResync(registration?: Registration, existingRemote?: RemoteClient) {
  let release!: (value: PageView) => void;
  let reject!: (error: Error) => void;
  let constructed!: () => void;
  const ready = new Promise<PageView>((resolve, fail) => {
    release = resolve;
    reject = fail;
  });
  const created = new Promise<void>((resolve) => {
    constructed = resolve;
  });
  const first =
    registration ??
    ({
      deviceId: 'device',
      keys: { sign: {}, signPublic: new Uint8Array(32) },
      remoteSession: {},
    } as unknown as Registration);
  const second: Registration = { ...first, remoteSession: {} };
  const remote =
    existingRemote ??
    ({
      listAgents: vi.fn(async () => []),
      send: vi.fn(),
      operation: vi.fn(),
    } as unknown as RemoteClient);
  const reconnect = vi.fn(async () => ({ registration: second, remote }));
  const recover = vi.fn(async () => false);
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
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    remote,
    { reconnect, recover },
  );
  const failed = vi.fn();
  live.subscribe(() => {}, failed);
  return {
    live,
    first,
    second,
    remote,
    reconnect,
    recover,
    failed,
    ready,
    created,
    constructed,
    release,
    reject,
  };
}

function expectNoRecoveryMutations(remote: RemoteClient) {
  expect(remote.send).not.toHaveBeenCalled();
  expect(remote.operation).not.toHaveBeenCalled();
  expect(mutations.submitOwn).not.toHaveBeenCalled();
  expect(mutations.submitOwnRecords).not.toHaveBeenCalled();
  expect(mutations.submitRecords).not.toHaveBeenCalled();
}

it('last old transport completion admits one typed session replacement while resync readiness is held', async () => {
  const h = heldResync();
  let oldSessionEnded = false;
  try {
    await h.live.snapshot();
    const old = connections.at(-1)!;
    old.close.mockImplementation(() => {
      oldSessionEnded = true;
    });
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    old.failed(new Error('RESYNC_REQUIRED'));
    await h.created;
    expect(oldSessionEnded).toBe(true);
    expect(h.live.registration).toBe(h.first);
    const pendingAsk = asks.instances.at(-1)!;
    pendingAsk.options.sessionEnded?.();
    pendingAsk.options.sessionEnded?.();
    expectNoRecoveryMutations(h.remote);
    expect(h.reconnect).toHaveBeenCalledExactlyOnceWith(h.first);
    h.release({ source: 'verified', title: 'Page' });
    await h.live.snapshot();
    expect(h.live.registration).toBe(h.second);
    expect(h.reconnect).toHaveBeenCalledOnce();
    expect(h.failed).not.toHaveBeenCalled();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    h.release({ source: 'verified', title: 'Page' });
  }
});

it('another live old transport keeps its admitted Session during a held same-session resync', async () => {
  const h = heldResync();
  let other: ReturnType<typeof heldResync> | undefined;
  let oldTransports = 2;
  try {
    await h.live.snapshot();
    const old = connections.at(-1)!;
    const oldAsk = asks.instances.at(-1)!;
    other = heldResync(h.first, h.remote);
    await other.live.snapshot();
    const otherConnection = connections.at(-1)!;
    old.close.mockImplementation(() => {
      oldTransports--;
    });
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    old.failed(new Error('RESYNC_REQUIRED'));
    await h.created;
    expect(oldTransports).toBe(1);
    oldAsk.options.sessionEnded?.();
    oldAsk.options.sessionEnded?.();
    expect(h.reconnect).not.toHaveBeenCalled();
    expect(otherConnection.close).not.toHaveBeenCalled();
    h.release({ source: 'verified', title: 'Page' });
    await h.live.snapshot();
    expect(h.live.registration).toBe(h.first);
    expect(h.reconnect).not.toHaveBeenCalled();
    expect(otherConnection.close).not.toHaveBeenCalled();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    other?.live.close();
    h.release({ source: 'verified', title: 'Page' });
  }
});

it.each([
  ['eviction', new SessionEvictedError(8, 'https://example.test/remote/settings')],
  ['unknown', new Error('unverified directory failure')],
] as const)(
  'a %s fault during held replacement fails closed without replacing the Session',
  async (_, error) => {
    const h = heldResync();
    try {
      await h.live.snapshot();
      const old = connections.at(-1)!;
      connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
      old.failed(new Error('RESYNC_REQUIRED'));
      await h.created;
      if (error instanceof SessionEvictedError)
        asks.instances.at(-1)!.options.sessionEnded?.(error);
      else connections.at(-1)!.failed(error);
      expectNoRecoveryMutations(h.remote);
      expect(h.failed).toHaveBeenCalledExactlyOnceWith(error);
      expect(h.reconnect).not.toHaveBeenCalled();
      expect(h.live.registration).toBe(h.first);
    } finally {
      h.live.close();
      h.release({ source: 'verified', title: 'Page' });
    }
  },
);

it('stale old Ask session-end callbacks cannot replace a ready current controller', async () => {
  const h = heldResync();
  try {
    await h.live.snapshot();
    const old = connections.at(-1)!;
    const oldAsk = asks.instances.at(-1)!;
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    old.failed(new Error('RESYNC_REQUIRED'));
    await h.created;
    h.release({ source: 'verified', title: 'Page' });
    await h.live.snapshot();
    oldAsk.options.sessionEnded?.();
    oldAsk.options.sessionEnded?.();
    expectNoRecoveryMutations(h.remote);
    expect(h.reconnect).not.toHaveBeenCalled();
  } finally {
    h.live.close();
  }
});

it('disposal while replacement readiness is held ignores duplicate session-end and late readiness', async () => {
  const h = heldResync();
  try {
    await h.live.snapshot();
    const old = connections.at(-1)!;
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    old.failed(new Error('RESYNC_REQUIRED'));
    await h.created;
    const pending = connections.at(-1)!;
    const ask = asks.instances.at(-1)!;
    h.live.close();
    ask.options.sessionEnded?.();
    ask.options.sessionEnded?.();
    h.release({ source: 'verified', title: 'Page' });
    await h.live.snapshot().catch(() => {});
    expect(pending.close).toHaveBeenCalledOnce();
    expect(h.reconnect).not.toHaveBeenCalled();
    expect(h.failed).not.toHaveBeenCalled();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    h.release({ source: 'verified', title: 'Page' });
  }
});

it.each(['resolve', 'reject'] as const)(
  'superseded open %s/publication/finalization cannot clear or block a held fresh open',
  async (completion) => {
    const h = heldResync();
    let freshRelease!: (value: PageView) => void;
    let freshConstructed!: () => void;
    const freshReady = new Promise<PageView>((resolve) => {
      freshRelease = resolve;
    });
    const freshCreated = new Promise<void>((resolve) => {
      freshConstructed = resolve;
    });
    const seen: string[] = [];
    try {
      await h.live.snapshot();
      h.live.subscribe(
        (value) => seen.push(value.source),
        () => {},
      );
      const old = connections.at(-1)!;
      connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
      old.failed(new Error('RESYNC_REQUIRED'));
      await h.created;
      const retired = connections.at(-1)!;
      const retiredOpen = h.live.snapshot().catch(() => null);
      const oldAsk = asks.instances.at(-1)!;
      connectionPlans.push({ ready: freshReady, constructed: freshConstructed, closed: () => {} });
      oldAsk.options.sessionEnded?.();
      await freshCreated;
      const fresh = connections.at(-1)!;
      expect(retired.close).toHaveBeenCalledOnce();
      expect(h.reconnect).toHaveBeenCalledExactlyOnceWith(h.first);
      const signals = asks.signals.length;
      retired.publish({ source: 'stale old source', title: 'Stale' });
      if (completion === 'resolve') h.release({ source: 'stale old source', title: 'Stale' });
      else h.reject(new Error('late superseded rejection'));
      await retiredOpen;
      expect(seen).not.toContain('stale old source');
      expect(h.failed).not.toHaveBeenCalled();
      expect(asks.signals).toHaveLength(signals);
      oldAsk.options.sessionEnded?.();
      retired.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
      expect(h.reconnect).toHaveBeenCalledOnce();
      fresh.publish({ source: 'fresh admitted source', title: 'Fresh' });
      freshRelease({ source: 'fresh admitted source', title: 'Fresh' });
      expect((await h.live.snapshot()).source).toBe('fresh admitted source');
      expect(h.live.registration).toBe(h.second);
      expectNoRecoveryMutations(h.remote);
    } finally {
      h.live.close();
      h.release({ source: 'unused', title: 'Unused' });
      freshRelease?.({ source: 'unused', title: 'Unused' });
    }
  },
);

it('replacement owner rejection is terminal with no second session attempt or mutation replay', async () => {
  const h = heldResync();
  let rejectOwner!: (error: Error) => void;
  const ownerReady = new Promise<never>((_, reject) => {
    rejectOwner = reject;
  });
  h.reconnect.mockImplementation(() => ownerReady);
  try {
    await h.live.snapshot();
    connections.at(-1)!.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
    const error = new Error('replacement registration rejected');
    rejectOwner(error);
    await expect(h.live.snapshot()).rejects.toBe(error);
    expect(h.failed).toHaveBeenCalledExactlyOnceWith(error);
    expect(h.reconnect).toHaveBeenCalledOnce();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
  }
});

it('a current end during already-replacing readiness is terminal, and late ready/publish cannot restore it', async () => {
  const h = heldResync();
  try {
    await h.live.snapshot();
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    connections.at(-1)!.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
    await h.created;
    const pending = connections.at(-1)!;
    const ask = asks.instances.at(-1)!;
    ask.options.sessionEnded?.();
    ask.options.sessionEnded?.();
    expect(h.failed).toHaveBeenCalledOnce();
    expect(h.reconnect).toHaveBeenCalledOnce();
    pending.publish({ source: 'late blocked source', title: 'Late' });
    h.release({ source: 'late blocked source', title: 'Late' });
    await expect(h.live.snapshot()).rejects.toThrow();
    expect(h.failed).toHaveBeenCalledOnce();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    h.release({ source: 'unused', title: 'Unused' });
  }
});

it('session end without an existing replacement owner is terminal', async () => {
  const h = heldResync();
  const live = new Live(
    new URL('https://example.test/colab/'),
    {
      space: 'space',
      revision: '1',
      owner: new Uint8Array(32),
      pageIds: [],
      pages: [],
    } as Bootstrap,
    h.first,
    { pageId: '10000000-0000-4000-8000-000000000001', epoch: '1', sharing: 'private' } as PageInfo,
    undefined,
    h.remote,
  );
  const failed = vi.fn();
  try {
    await h.live.snapshot();
    await live.snapshot();
    live.subscribe(() => {}, failed);
    const count = connections.length;
    asks.instances.at(-1)!.options.sessionEnded?.();
    expect(failed).toHaveBeenCalledOnce();
    expect(connections).toHaveLength(count);
    expect(h.reconnect).not.toHaveBeenCalled();
    expectNoRecoveryMutations(h.remote);
  } finally {
    live.close();
    h.live.close();
  }
});

it('late old-session diagnosis cannot evict or reopen a held fresh attempt', async () => {
  const h = heldResync();
  let rejectDiagnosis!: (error: Error) => void;
  const diagnosis = new Promise<never>((_, reject) => {
    rejectDiagnosis = reject;
  });
  vi.mocked(h.remote.listAgents).mockImplementation(() => diagnosis);
  try {
    await h.live.snapshot();
    const old = connections.at(-1)!;
    old.failed(new Error('Sync disconnected'));
    old.failed(new Error('Sync disconnected'));
    expect(h.remote.listAgents).toHaveBeenCalledOnce();
    connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
    asks.instances.at(-1)!.options.sessionEnded?.();
    await h.created;
    rejectDiagnosis(new SessionEvictedError(8, 'https://example.test/remote/settings'));
    await diagnosis.catch(() => {});
    expect(h.failed).not.toHaveBeenCalled();
    expect(h.reconnect).toHaveBeenCalledOnce();
    h.release({ source: 'verified', title: 'Page' });
    await h.live.snapshot();
    expect(h.live.registration).toBe(h.second);
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    h.release({ source: 'unused', title: 'Unused' });
  }
});

it('disposed pending owner reconnect cannot install its late Registration or Connection', async () => {
  const h = heldResync();
  let releaseOwner!: (value: { registration: Registration; remote: RemoteClient }) => void;
  const ownerReady = new Promise<{ registration: Registration; remote: RemoteClient }>(
    (resolve) => {
      releaseOwner = resolve;
    },
  );
  h.reconnect.mockImplementation(() => ownerReady);
  try {
    await h.live.snapshot();
    const count = connections.length;
    connections.at(-1)!.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
    const pending = h.live.snapshot();
    h.live.close();
    releaseOwner({ registration: h.second, remote: h.remote });
    await expect(pending).rejects.toThrow();
    expect(h.live.registration).toBe(h.first);
    expect(connections).toHaveLength(count);
    expect(h.reconnect).toHaveBeenCalledOnce();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
  }
});

/** Reproduces Connection.close ordering: ready rejects before failed is called. */
async function pendingSocketClose(replacing = false) {
  const h = heldResync();
  let resolveRead!: (rows: Awaited<ReturnType<RemoteClient['listAgents']>>) => void;
  let rejectRead!: (error: Error) => void;
  const read = new Promise<Awaited<ReturnType<RemoteClient['listAgents']>>>((resolve, reject) => {
    resolveRead = resolve;
    rejectRead = reject;
  });
  vi.mocked(h.remote.listAgents).mockImplementation(() => read);
  await h.live.snapshot();
  connectionPlans.push({ ready: h.ready, constructed: h.constructed, closed: () => {} });
  connections
    .at(-1)!
    .failed(
      replacing ? new SessionEndedError('REMOTE_SESSION_ENDED') : new Error('RESYNC_REQUIRED'),
    );
  await h.created;
  const pending = connections.at(-1)!;
  const rejected = h.live.snapshot().catch((error: unknown) => error);
  const disconnected = new Error('Sync disconnected');
  h.reject(disconnected);
  pending.failed(disconnected);
  return { ...h, pending, rejected, disconnected, read, resolveRead, rejectRead };
}

it.each(['ended', 'evicted', 'unknown', 'valid'] as const)(
  'pending same-session ready rejection keeps the exact old-session diagnosis until %s',
  async (outcome) => {
    const h = await pendingSocketClose();
    try {
      expect(h.remote.listAgents).toHaveBeenCalledOnce();
      expect(await h.rejected).toBe(h.disconnected);
      expect(h.failed).not.toHaveBeenCalled();
      expect(h.reconnect).not.toHaveBeenCalled();
      h.pending.failed(h.disconnected);
      expect(h.remote.listAgents).toHaveBeenCalledOnce();
      const reason =
        outcome === 'evicted'
          ? new SessionEvictedError(8, 'https://example.test/remote/settings')
          : outcome === 'ended'
            ? new SessionEndedError('REMOTE_SESSION_ENDED')
            : new Error('unverified read failure');
      if (outcome === 'valid') h.resolveRead([]);
      else h.rejectRead(reason);
      await h.read.catch(() => []);
      if (outcome === 'ended') {
        expect(h.reconnect).toHaveBeenCalledExactlyOnceWith(h.first);
        await h.live.snapshot();
        expect(h.live.registration).toBe(h.second);
        expect(h.failed).not.toHaveBeenCalled();
      } else {
        expect(h.failed).toHaveBeenCalledOnce();
        if (outcome === 'evicted') expect(h.failed.mock.calls[0][0]).toBe(reason);
        else {
          expect(h.failed.mock.calls[0][0]).toBeInstanceOf(RecoveryRequiredError);
          expect(h.failed.mock.calls[0][0].cause).toBe(h.disconnected);
        }
        expect(h.reconnect).not.toHaveBeenCalled();
        expect(h.live.registration).toBe(h.first);
      }
      expectNoRecoveryMutations(h.remote);
    } finally {
      h.live.close();
      h.resolveRead([]);
    }
  },
);

it.each(['superseded', 'disposed'] as const)(
  'late pending-socket diagnosis is ignored after its owner is %s',
  async (state) => {
    const h = await pendingSocketClose();
    try {
      expect(h.remote.listAgents).toHaveBeenCalledOnce();
      await h.rejected;
      if (state === 'disposed') h.live.close();
      else {
        asks.instances.at(-1)!.options.sessionEnded?.();
        await h.live.snapshot();
        expect(h.live.registration).toBe(h.second);
      }
      h.rejectRead(new SessionEvictedError(8, 'https://example.test/remote/settings'));
      await h.read.catch(() => []);
      expect(h.failed).not.toHaveBeenCalled();
      expect(h.reconnect).toHaveBeenCalledTimes(state === 'disposed' ? 0 : 1);
      expectNoRecoveryMutations(h.remote);
    } finally {
      h.live.close();
      h.resolveRead([]);
    }
  },
);

it('already-replacing ready rejection offers explicit recovery without an old-session diagnosis or recursive replacement', async () => {
  const h = await pendingSocketClose(true);
  try {
    expect(await h.rejected).toBe(h.disconnected);
    expect(h.remote.listAgents).not.toHaveBeenCalled();
    expect(h.failed).toHaveBeenCalledOnce();
    expect(h.failed.mock.calls[0][0]).toBeInstanceOf(RecoveryRequiredError);
    expect(h.failed.mock.calls[0][0].cause).toBe(h.disconnected);
    expect(h.reconnect).toHaveBeenCalledOnce();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
    h.resolveRead([]);
  }
});

it('a replacement fetch failure retains a typed recovery state, and concurrent explicit clicks check once', async () => {
  const h = heldResync();
  const network = new TypeError('Failed to fetch');
  h.reconnect.mockRejectedValueOnce(network);
  let rejectRecovery!: (error: Error) => void;
  const recovery = new Promise<boolean>((_, reject) => {
    rejectRecovery = reject;
  });
  h.recover.mockImplementationOnce(() => recovery).mockResolvedValueOnce(true);
  try {
    await h.live.snapshot();
    const current = connections.at(-1)!;
    current.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
    await expect(h.live.snapshot()).rejects.toMatchObject({ cause: network });
    expect(h.failed).toHaveBeenCalledOnce();
    expect(h.failed.mock.calls[0][0]).toBeInstanceOf(RecoveryRequiredError);
    expect(h.live.registration).toBe(h.first);
    const one = h.live.reconnect();
    const two = h.live.reconnect();
    expect(one).toBe(two);
    expect(h.recover).toHaveBeenCalledOnce();
    await expect(h.live.edit('unadmitted edit', 'verified')).rejects.toThrow(
      'Page editing unavailable',
    );
    expect(current.close).toHaveBeenCalledOnce();
    expect(asks.instances.at(-1)!.close).toHaveBeenCalled();
    expectNoRecoveryMutations(h.remote);
    const nextFailure = new RecoveryRequiredError(new TypeError('Failed to fetch'));
    rejectRecovery(nextFailure);
    expect(await one).toBe(false);
    expect(h.failed).toHaveBeenCalledTimes(2);
    expect(h.failed.mock.calls[1][0]).toBe(nextFailure);
    expect(h.recover).toHaveBeenCalledOnce();
    expect(await h.live.reconnect()).toBe(true);
    expect(h.recover).toHaveBeenCalledTimes(2);
    expect(h.reconnect).toHaveBeenCalledOnce();
    expectNoRecoveryMutations(h.remote);
  } finally {
    h.live.close();
  }
});

it.each([
  new Error('replacement registration rejected'),
  new SessionEvictedError(8),
  new SessionEndedError('REMOTE_SESSION_ENDED'),
])(
  'a non-network replacement failure stays terminal without explicit recovery: %s',
  async (error) => {
    const h = heldResync();
    h.reconnect.mockRejectedValueOnce(error);
    try {
      await h.live.snapshot();
      connections.at(-1)!.failed(new SessionEndedError('REMOTE_SESSION_ENDED'));
      await expect(h.live.snapshot()).rejects.toBe(error);
      expect(h.failed).toHaveBeenCalledExactlyOnceWith(error);
      expect(await h.live.reconnect()).toBe(false);
      expect(h.recover).not.toHaveBeenCalled();
      expect(h.reconnect).toHaveBeenCalledOnce();
      expectNoRecoveryMutations(h.remote);
    } finally {
      h.live.close();
    }
  },
);

function openLive() {
  connections.length = 0;
  saves.save.mockReset();
  saves.status.mockReset();
  return new Live(
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
}
const committed = (operationId: string): SaveResult => ({
  operationId,
  state: 'committed',
  revision: '2',
});
const shows = (source: string) => ({ source, title: 'Page' }) as PageView;

it('a save resolves once the page shows the saved source, with one operation ID and no status request', async () => {
  const live = openLive();
  try {
    await live.snapshot();
    saves.save.mockImplementation(async (connection, operationId) => {
      setTimeout(() => connection.publish(shows('<p>new</p>')), 20);
      return committed(operationId);
    });
    await live.edit('<p>new</p>', 'verified');
    expect(saves.save).toHaveBeenCalledOnce();
    expect(saves.save.mock.calls[0].slice(1)).toEqual([
      expect.stringMatching(/^[0-9a-f-]{36}$/),
      'verified',
      '<p>new</p>',
    ]);
    expect(saves.status).not.toHaveBeenCalled();
  } finally {
    live.close();
  }
});

it('a lost reply reopens the page and asks for the operation status exactly once, never resending', async () => {
  const live = openLive();
  try {
    await live.snapshot();
    const first = connections.at(-1)!;
    saves.save.mockImplementationOnce(async (connection) => {
      connection.failed(new Error('Sync disconnected'));
      throw new Error('Sync disconnected');
    });
    saves.status.mockImplementation(async (connection, operationId) => {
      connection.publish(shows('<p>new</p>'));
      return committed(operationId);
    });
    await live.edit('<p>new</p>', 'verified');
    const second = connections.at(-1)!;
    expect(second).not.toBe(first);
    expect(saves.save).toHaveBeenCalledOnce();
    expect(saves.status).toHaveBeenCalledOnce();
    expect(saves.status.mock.calls[0][0]).toBe(second);
    expect(saves.status.mock.calls[0][1]).toBe(saves.save.mock.calls[0][1]);
  } finally {
    live.close();
  }
});

it('a lost reply the page never recorded is reported as not applied; a failing status check names the operation', async () => {
  const live = openLive();
  try {
    await live.snapshot();
    const lose = () =>
      saves.save.mockImplementationOnce(async (connection) => {
        connection.failed(new Error('Sync disconnected'));
        throw new Error('Sync disconnected');
      });
    lose();
    saves.status.mockImplementationOnce(async (_connection, operationId) => ({
      operationId,
      state: 'absent',
    }));
    await expect(live.edit('<p>new</p>', 'verified')).rejects.toBeInstanceOf(SaveNotApplied);
    lose();
    saves.status.mockRejectedValueOnce(new Error('Sync disconnected'));
    const unknown = await live.edit('<p>new</p>', 'verified').catch((error) => error);
    expect(unknown).toBeInstanceOf(SaveOutcomeUnknown);
    expect(unknown.operationId).toBe(saves.save.mock.calls[1][1]);
    expect(saves.status).toHaveBeenCalledTimes(2);
    expect(saves.save).toHaveBeenCalledTimes(2);
  } finally {
    live.close();
  }
});

it('refusals and an over-limit source surface unchanged on a connection that stays up', async () => {
  const live = openLive();
  try {
    await live.snapshot();
    saves.save.mockResolvedValueOnce({
      operationId: 'x',
      state: 'rejected',
      code: 'COLAB_STALE_BASE',
      message: 'Moved.',
    });
    await expect(live.edit('<p>new</p>', 'verified')).rejects.toMatchObject({
      constructor: SaveRefused,
      code: 'COLAB_STALE_BASE',
    });
    saves.save.mockRejectedValueOnce(new SaveTooLarge(3 * 1024 * 1024, 2 * 1024 * 1024));
    await expect(live.edit('<p>new</p>', 'verified')).rejects.toBeInstanceOf(SaveTooLarge);
    expect(saves.status).not.toHaveBeenCalled();
    expect(connections).toHaveLength(1);
  } finally {
    live.close();
  }
});
