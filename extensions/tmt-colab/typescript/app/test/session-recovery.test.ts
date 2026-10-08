import { expect, it, vi } from 'vite-plus/test';
import {
  clearRecovery,
  recoverSession,
  recoveryKey,
  RecoveryRequiredError,
} from '../src/session-recovery.js';
const mount = new URL('https://example.test/r/abcd/x/colab/');
function storage() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => {
      items.set(key, value);
    },
    removeItem: (key: string) => {
      items.delete(key);
    },
  };
}
it('marks before reopening, coalesces concurrent recovery and prevents a reload loop until authenticated boot', async () => {
  const store = storage(),
    reload = vi.fn();
  let release!: () => void;
  const reopen = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        release = resolve;
      }),
  );
  const options = { mount, storage: store, reopen, reload };
  const first = recoverSession(options);
  expect(store.getItem(recoveryKey(mount))).toBe('attempted');
  expect(await recoverSession(options)).toBe(false);
  expect(reopen).toHaveBeenCalledOnce();
  expect(reload).not.toHaveBeenCalled();
  release();
  expect(await first).toBe(true);
  expect(reload).toHaveBeenCalledOnce();
  expect(await recoverSession(options)).toBe(false);
  clearRecovery(mount, store);
  const next = recoverSession(options);
  release();
  expect(await next).toBe(true);
  expect(reopen).toHaveBeenCalledTimes(2);
});
it('unpaired keys or failed reopening leave guidance without reload or another attempt', async () => {
  for (const reason of [
    'This browser is not paired.',
    'Paired with another machine.',
    'Remote unavailable',
  ]) {
    const reopen = vi.fn(async () => {
        throw new Error(reason);
      }),
      reload = vi.fn();
    const options = { mount, storage: storage(), reopen, reload };
    expect(await recoverSession(options)).toBe(false);
    expect(await recoverSession(options)).toBe(false);
    expect(reopen).toHaveBeenCalledOnce();
    expect(reload).not.toHaveBeenCalled();
  }
});
it('unavailable session storage fails closed before any Remote call', async () => {
  const reopen = vi.fn(),
    reload = vi.fn();
  expect(
    await recoverSession({
      mount,
      reopen,
      reload,
      storage: {
        getItem() {
          throw new Error('Storage disabled');
        },
        setItem() {},
        removeItem() {},
      },
    }),
  ).toBe(false);
  expect(reopen).not.toHaveBeenCalled();
  expect(reload).not.toHaveBeenCalled();
});

it('an explicit network failure releases only its marker for another click, with no reload', async () => {
  const store = storage();
  const error = new TypeError('Failed to fetch');
  const reopen = vi.fn().mockRejectedValueOnce(error).mockResolvedValueOnce({});
  const reload = vi.fn();
  const options = { mount, storage: store, reopen, reload, explicit: true };
  await expect(recoverSession(options)).rejects.toMatchObject({
    constructor: RecoveryRequiredError,
    cause: error,
  });
  expect(store.getItem(recoveryKey(mount))).toBeNull();
  expect(reload).not.toHaveBeenCalled();
  expect(reopen).toHaveBeenCalledOnce();
  expect(await recoverSession(options)).toBe(true);
  expect(reopen).toHaveBeenCalledTimes(2);
  expect(reload).toHaveBeenCalledOnce();
  expect(store.getItem(recoveryKey(mount))).toBe('attempted');
});

it('an automatic network failure keeps its marker and cannot reopen or reload in a loop', async () => {
  const store = storage();
  const reopen = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));
  const reload = vi.fn();
  const options = { mount, storage: store, reopen, reload };
  expect(await recoverSession(options)).toBe(false);
  expect(await recoverSession(options)).toBe(false);
  expect(store.getItem(recoveryKey(mount))).toBe('attempted');
  expect(reopen).toHaveBeenCalledOnce();
  expect(reload).not.toHaveBeenCalled();
});

it('an explicit authority refusal keeps its marker and is never a network recovery', async () => {
  const store = storage();
  const reopen = vi.fn().mockRejectedValue(new Error('The session was refused.'));
  const reload = vi.fn();
  const options = { mount, storage: store, reopen, reload, explicit: true };
  expect(await recoverSession(options)).toBe(false);
  expect(await recoverSession(options)).toBe(false);
  expect(store.getItem(recoveryKey(mount))).toBe('attempted');
  expect(reopen).toHaveBeenCalledOnce();
  expect(reload).not.toHaveBeenCalled();
});
