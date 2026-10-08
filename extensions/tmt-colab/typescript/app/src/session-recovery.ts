/** One recovery chain spans its successful reload. Authenticated boot clears
 * the marker; another guidance response after reload cannot reopen in a loop. */
export function recoveryKey(mount: URL) {
  return `colab-recovery:${mount.pathname}`;
}
export async function recoverSession(options: {
  mount: URL;
  /** Only a trusted-parent click may retry a network-failed attempt. */
  explicit?: boolean;
  storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  reopen(): Promise<unknown>;
  reload(): void;
}): Promise<boolean> {
  try {
    const key = recoveryKey(options.mount);
    if (options.storage.getItem(key) !== null) return false;
    options.storage.setItem(key, 'attempted');
    try {
      await options.reopen();
    } catch (error) {
      if (options.explicit && error instanceof TypeError) {
        // A fetch rejection performed no reload. Release only this attempt's
        // marker so another explicit click can check the paired door again.
        options.storage.removeItem(key);
        throw new RecoveryRequiredError(error);
      }
      throw error;
    }
    options.reload();
    return true;
  } catch (error) {
    if (error instanceof RecoveryRequiredError) throw error;
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
/** A stopped transport can be recovered by an explicit trusted-parent action.
 * This offers no admission or permission to replay a mutation. */
export class RecoveryRequiredError extends Error {
  constructor(cause: Error) {
    super('Connection lost. Reconnect to resume.', { cause });
  }
}
