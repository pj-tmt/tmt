import { expect, it, vi } from 'vite-plus/test';
import type { LiveSessionOwner } from '../src/live.js';
import { mountedTransport } from '../src/mounted.js';
const setup = vi.hoisted(() => {
  const first = { deviceId: 'device', remoteSession: { id: 'first' } };
  const second = { deviceId: 'device', remoteSession: { id: 'second' } };
  return {
    first,
    second,
    sdk: {},
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
      _signal: unknown,
      _remote: unknown,
      owner: LiveSessionOwner,
    ) {
      setup.events.push('connection');
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
  const mounted = await mountedTransport();
  expect(setup.register).toHaveBeenCalledExactlyOnceWith(expect.any(URL), setup.sdk);
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
