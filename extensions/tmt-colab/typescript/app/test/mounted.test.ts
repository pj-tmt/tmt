import { expect, it, vi } from 'vite-plus/test';
import type { LiveSessionOwner } from '../src/live.js';
import { InactiveTabError, type TabOwnership } from '../src/active-tab.js';
import type { RemoteClient } from '../src/ask-remote.js';
import { mountedTransport } from '../src/mounted.js';
const setup = vi.hoisted(() => {
  const first = { deviceId: 'device', remoteSession: { id: 'first' } };
  const second = { deviceId: 'device', remoteSession: { id: 'second' } };
  return {
    first,
    second,
    sdk: { reopenSession: vi.fn(async () => ({})), certifyKey: vi.fn(async () => ({})) },
    signals: [] as AbortSignal[],
    ports: [] as RemoteClient[],
    events: [] as string[],
    register: vi.fn(),
    verify: vi.fn(async () => {}),
    remote: vi.fn(),
    owner: null as LiveSessionOwner | null,
  };
});
vi.mock('../src/bootstrap.js', () => ({
  mountUrl: () => new URL('https://example.test/colab/'),
  discover: async (_mount: URL, verify: (space: string, owner: Uint8Array) => Promise<void>) => {
    const owner = new Uint8Array(32);
    await verify('space', owner);
    return { space: 'space', owner, pages: [{ pageId: 'page', sharing: 'private' }] };
  },
}));
vi.mock('../src/registration.js', () => ({
  remoteSdk: async () => setup.sdk,
  register: setup.register,
  verifyRegistration: setup.verify,
}));
vi.mock('../src/ask-remote.js', () => ({ createRemoteClient: setup.remote }));
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

it('replaces a verified mounted session once for concurrent reconnects and creates Remote before sync', async () => {
  setup.events.length = 0;
  setup.register.mockReset().mockResolvedValueOnce(setup.first);
  setup.remote.mockReset().mockImplementation(async (_mount, _sdk, session) => {
    setup.events.push(`remote:${session.id}`);
    return {};
  });
  const mounted = await mountedTransport({ active: true, run: (action) => action() });
  expect(setup.register).toHaveBeenCalledExactlyOnceWith(
    expect.any(URL),
    expect.objectContaining({
      reopenSession: expect.any(Function),
      certifyKey: expect.any(Function),
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

it('inactive ownership closes all page signals and fences every Remote method, certification and session reconnect', async () => {
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
  let active = true;
  const ownership: TabOwnership = {
    get active() {
      return active;
    },
    async run(action) {
      if (!active) throw new InactiveTabError();
      return action();
    },
  };
  const mounted = await mountedTransport(ownership);
  await mounted.transport.page('page');
  const remote = setup.ports[0];
  const sdk = setup.register.mock.calls[0][1];
  active = false;
  mounted.close();
  expect(setup.signals[0].aborted).toBe(true);
  for (const result of [
    remote.context(),
    remote.listAgents(),
    remote.send({ operationId: 'op', agentId: 'agent', message: 'message' }),
    remote.operation('op'),
    remote.result('request'),
  ])
    await expect(result).rejects.toBeInstanceOf(InactiveTabError);
  await expect(setup.owner!.reconnect(setup.first as never)).rejects.toBeInstanceOf(
    InactiveTabError,
  );
  await expect(sdk.reopenSession()).rejects.toBeInstanceOf(InactiveTabError);
  await expect(sdk.certifyKey('sign', new Uint8Array(32))).rejects.toBeInstanceOf(InactiveTabError);
  await expect(mounted.transport.page('page')).rejects.toBeInstanceOf(InactiveTabError);
  expect(call).not.toHaveBeenCalled();
  expect(setup.sdk.reopenSession).toHaveBeenCalledOnce();
  expect(setup.sdk.certifyKey).toHaveBeenCalledOnce();
  expect(setup.register).toHaveBeenCalledOnce();
});

it('a takeover during explicit recovery prevents the old tab from reloading or reopening again', async () => {
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
  let active = true;
  const ownership: TabOwnership = {
    get active() {
      return active;
    },
    async run(action) {
      if (!active) throw new InactiveTabError();
      return action();
    },
  };
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
    const mounted = await mountedTransport(ownership);
    await mounted.transport.page('page');
    const recovering = setup.owner!.recover!();
    expect(setup.sdk.reopenSession).toHaveBeenCalledOnce();
    active = false;
    mounted.close();
    release();
    expect(await recovering).toBe(false);
    expect(reload).not.toHaveBeenCalled();
    await expect(setup.owner!.recover!()).rejects.toBeInstanceOf(InactiveTabError);
    expect(setup.sdk.reopenSession).toHaveBeenCalledOnce();
  } finally {
    vi.unstubAllGlobals();
  }
});
