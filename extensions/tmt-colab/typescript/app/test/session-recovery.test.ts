import { expect, it, vi } from 'vite-plus/test';
import { clearRecovery, recoverSession, recoveryKey } from '../src/session-recovery.js';
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
      },
    }),
  ).toBe(false);
  expect(reopen).not.toHaveBeenCalled();
  expect(reload).not.toHaveBeenCalled();
});
