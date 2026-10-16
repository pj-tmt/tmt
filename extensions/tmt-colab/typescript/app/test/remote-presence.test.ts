import { expect, it, vi } from 'vite-plus/test';
import {
  RemotePresence,
  type RemoteDevice,
  type RemoteDevicePage,
} from '../src/remote-presence.js';

const row = (overrides: Partial<RemoteDevice> = {}): RemoteDevice => ({
  clientId: 'device',
  name: 'Studio',
  kind: 'browser',
  thisBrowser: false,
  liveSessionCount: 1,
  lastActivityAtMs: 100,
  revoked: false,
  expiresAtMs: null,
  ...overrides,
});
function setup(
  devices = vi.fn(async (): Promise<RemoteDevicePage> => ({ devices: [], nextCursor: null })),
) {
  const source = { generation: {}, devices };
  const current: { value: typeof source | undefined } = { value: source };
  const owner = new RemotePresence(
    () => current.value,
    'https://door.test/settings',
    () => 1000,
  );
  return { owner, current, source, devices };
}

it('publishes only a complete paginated snapshot and orders this browser before recent devices', async () => {
  let release!: (page: RemoteDevicePage) => void;
  const devices = vi
    .fn()
    .mockResolvedValueOnce({ devices: [row({ clientId: 'old' })], nextCursor: 'next' })
    .mockImplementationOnce(
      () =>
        new Promise<RemoteDevicePage>((resolve) => {
          release = resolve;
        }),
    );
  const { owner } = setup(devices);
  const reading = owner.refresh();
  await vi.waitFor(() => expect(devices).toHaveBeenCalledTimes(2));
  expect(owner.snapshot().kind).toBe('hidden');
  release({
    devices: [
      row({ clientId: 'recent', liveSessionCount: 2, lastActivityAtMs: 900 }),
      row({ clientId: 'self', thisBrowser: true }),
    ],
    nextCursor: null,
  });
  await reading;
  expect(owner.snapshot()).toMatchObject({
    kind: 'confirmed',
    count: 4,
    atMs: 1000,
    settingsUrl: 'https://door.test/settings',
  });
  const snapshot = owner.snapshot();
  if (snapshot.kind !== 'confirmed') throw new Error('Expected confirmed');
  expect(snapshot.devices.map((device) => device.clientId)).toEqual(['self', 'recent', 'old']);
  expect(devices.mock.calls).toEqual([
    [{ cursor: null, limit: 50 }],
    [{ cursor: 'next', limit: 50 }],
  ]);
});

it('never publishes a partial total at the 20-page bound', async () => {
  const devices = vi.fn(async (): Promise<RemoteDevicePage> => ({
    devices: [row({ clientId: `${devices.mock.calls.length}` })],
    nextCursor: `${devices.mock.calls.length}`,
  }));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(devices).toHaveBeenCalledTimes(20);
  expect(owner.snapshot()).toEqual({ kind: 'unconfirmed', atMs: 1000 });
});

it('accepts a complete twentieth page', async () => {
  const devices = vi.fn(async (): Promise<RemoteDevicePage> => ({
    devices: [row({ clientId: `${devices.mock.calls.length}` })],
    nextCursor: devices.mock.calls.length < 20 ? `${devices.mock.calls.length}` : null,
  }));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(owner.snapshot()).toMatchObject({ kind: 'confirmed', count: 20 });
});

it('coalesces concurrent triggers into exactly one read and does not retry failures', async () => {
  let reject!: (error: Error) => void;
  const devices = vi.fn(
    () =>
      new Promise<RemoteDevicePage>((_, fail) => {
        reject = fail;
      }),
  );
  const { owner } = setup(devices);
  const first = owner.refresh();
  expect(owner.refresh()).toBe(first);
  expect(owner.refresh()).toBe(first);
  reject(new Error('Admission refused'));
  await first;
  expect(devices).toHaveBeenCalledTimes(1);
  expect(owner.snapshot()).toEqual({ kind: 'unconfirmed', atMs: 1000 });
});

it('does not turn a later page refusal into a partial sum or a settings link', async () => {
  const devices = vi
    .fn()
    .mockResolvedValueOnce({ devices: [row()], nextCursor: 'next' })
    .mockRejectedValueOnce(new Error('Refused'));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(owner.snapshot()).toEqual({ kind: 'unconfirmed', atMs: 1000 });
});

it('drops revoked and expired rows but retains live zero-count rows and unknown activity', async () => {
  const devices = vi.fn(async () => ({
    devices: [
      row({ clientId: 'revoked', revoked: true, liveSessionCount: 0, lastActivityAtMs: null }),
      row({ clientId: 'expired', expiresAtMs: 999, liveSessionCount: 0, lastActivityAtMs: null }),
      row({ clientId: 'valid', liveSessionCount: 0, lastActivityAtMs: null }),
    ],
    nextCursor: null,
  }));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(owner.snapshot()).toMatchObject({
    kind: 'confirmed',
    count: 0,
    devices: [row({ clientId: 'valid', liveSessionCount: 0, lastActivityAtMs: null })],
  });
});

it('rejects repeated cursors and duplicate device identities instead of double-counting', async () => {
  const devices = vi.fn(async () => ({ devices: [row()], nextCursor: 'same' }));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(devices).toHaveBeenCalledTimes(2);
  expect(owner.snapshot().kind).toBe('unconfirmed');
});

it('fences replacement sessions and a closed owner without starting a successor', async () => {
  let release!: (page: RemoteDevicePage) => void;
  const devices = vi.fn(
    () =>
      new Promise<RemoteDevicePage>((resolve) => {
        release = resolve;
      }),
  );
  const { owner, current, source } = setup(devices);
  const reading = owner.refresh();
  current.value = { ...source, generation: {} };
  release({ devices: [row()], nextCursor: 'next' });
  await reading;
  expect(owner.snapshot().kind).toBe('hidden');
  const second = owner.refresh();
  const closing = owner.close();
  release({ devices: [row()], nextCursor: 'next' });
  await Promise.all([second, closing]);
  await owner.refresh();
  expect(devices).toHaveBeenCalledTimes(2);
  expect(owner.snapshot().kind).toBe('hidden');
});

it('hides without a mounted session and clears a previous confirmed observation', async () => {
  const { owner, current, devices } = setup();
  await owner.refresh();
  expect(owner.snapshot()).toMatchObject({ kind: 'confirmed', count: 0 });
  current.value = undefined;
  await owner.refresh();
  expect(owner.snapshot()).toEqual({ kind: 'hidden' });
  expect(devices).toHaveBeenCalledTimes(1);
});

it('keeps the door session count when the browser clock is ahead of the expiry timestamp', async () => {
  const devices = vi.fn(async () => ({
    devices: [row({ expiresAtMs: 900, liveSessionCount: 2, lastActivityAtMs: 800 })],
    nextCursor: null,
  }));
  const { owner } = setup(devices);
  await owner.refresh();
  expect(owner.snapshot()).toMatchObject({ kind: 'confirmed', count: 2 });
});
