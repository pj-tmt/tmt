import { expect, it, vi } from 'vite-plus/test';
import type { Bootstrap, PageInfo } from '../src/bootstrap.js';
import type { LiveAskOptions } from '../src/live-ask.js';
import type { RemoteClient } from '../src/ask-remote.js';
import type { PageView } from '../src/transport.js';
import type { Registration } from '../src/registration.js';
import { Live } from '../src/live.js';

const connections = vi.hoisted(
  () =>
    [] as {
      failed(error: Error): void;
      publish(value: PageView): void;
      admission: {
        head: { revision: bigint; hash: Uint8Array } | null;
        root: object | null;
        validatePage: ReturnType<typeof vi.fn>;
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
    close() {}
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

it('session-end replaces registration and controller before reconnect, then resumes observation after catchup', async () => {
  asks.instances.length = 0;
  asks.signals.length = 0;
  const first = {
    deviceId: 'device',
    keys: { sign: {}, signPublic: new Uint8Array(32) },
    remoteSession: {},
  } as unknown as Registration;
  const second = { ...first, remoteSession: {} };
  const remote = {} as RemoteClient,
    replacementRemote = {} as RemoteClient;
  const reconnect = vi.fn(async () => ({ registration: second, remote: replacementRemote }));
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
    expect(previous.options.remote).toBe(remote);
    previous.options.sessionEnded?.();
    await live.snapshot();
    expect(reconnect).toHaveBeenCalledExactlyOnceWith(first);
    expect(previous.close).toHaveBeenCalled();
    expect(live.registration).toBe(second);
    expect(asks.instances).toHaveLength(2);
    expect(asks.instances[1].options.remote).toBe(replacementRemote);
    await vi.waitFor(() => expect(asks.signals).toHaveLength(2));
    expect(asks.signals[0].aborted).toBe(true);
    expect(asks.signals[1].aborted).toBe(false);
  } finally {
    live.close();
  }
});
