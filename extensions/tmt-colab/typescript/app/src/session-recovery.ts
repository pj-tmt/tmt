/** One recovery chain spans its successful reload. Authenticated boot clears
 * the marker; another guidance response after reload cannot reopen in a loop. */
export function recoveryKey(mount: URL) {
  return `colab-recovery:${mount.pathname}`;
}
export async function recoverSession(options: {
  mount: URL;
  storage: Pick<Storage, 'getItem' | 'setItem'>;
  reopen(): Promise<unknown>;
  reload(): void;
}): Promise<boolean> {
  try {
    const key = recoveryKey(options.mount);
    if (options.storage.getItem(key) !== null) return false;
    options.storage.setItem(key, 'attempted');
    await options.reopen();
    options.reload();
    return true;
  } catch {
    // No paired key, a refused/failed reopen, or unavailable session storage
    // leaves plain guidance. It never authorizes retrying an Ask.
    return false;
  }
}
export function clearRecovery(mount: URL, storage?: Pick<Storage, 'removeItem'>) {
  try {
    (storage ?? globalThis.sessionStorage)?.removeItem(recoveryKey(mount));
  } catch {
    // A connected page remains usable when browser storage is disabled.
  }
}
