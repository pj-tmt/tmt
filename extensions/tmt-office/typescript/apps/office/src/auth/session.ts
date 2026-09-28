export interface SessionUser {
  readonly uid: string;
  readonly displayName: string | null;
}

export interface SessionSnapshot {
  readonly ready: boolean;
  readonly user: SessionUser | null;
  readonly pending: boolean;
  readonly error: string | null;
}

export interface SessionAdapter {
  observe: (changed: (user: SessionUser | null) => void) => () => void;
  signIn: () => Promise<void>;
  signOut: () => Promise<void>;
  dispose: () => Promise<void>;
}

export interface OfficeSession {
  getSnapshot: () => SessionSnapshot;
  subscribe: (listener: () => void) => () => void;
  signIn: () => Promise<void>;
  signOut: () => Promise<void>;
  dispose: () => Promise<void>;
}

function actionError(error: unknown): string {
  const code = typeof error === 'object' && error !== null && 'code' in error ? error.code : null;
  switch (code) {
    case 'auth/popup-closed-by-user':
    case 'auth/cancelled-popup-request':
      return 'Sign-in cancelled. You can try again.';
    case 'auth/popup-blocked':
      return 'Allow popups for this page, then try again.';
    case 'auth/network-request-failed':
      return 'Cannot reach the sign-in service. Check your connection and try again.';
    default:
      return 'The session action failed. Please try again.';
  }
}

/** One observer owns identity; action completion never manufactures a signed-in user. */
export function createSession(adapter: SessionAdapter): OfficeSession {
  let snapshot: SessionSnapshot = { ready: false, user: null, pending: false, error: null };
  const listeners = new Set<() => void>();
  let disposed = false;
  let disposal: Promise<void> | undefined;
  function publish(next: SessionSnapshot): void {
    if (disposed) return;
    snapshot = next;
    for (const listener of listeners) listener();
  }
  const unsubscribe = adapter.observe((user) => publish({ ...snapshot, ready: true, user }));

  async function act(action: () => Promise<void>): Promise<void> {
    if (disposed || !snapshot.ready || snapshot.pending) return;
    publish({ ...snapshot, pending: true, error: null });
    try {
      await action();
    } catch (error) {
      publish({ ...snapshot, error: actionError(error) });
    } finally {
      publish({ ...snapshot, pending: false });
    }
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      if (disposed) return () => {};
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    signIn: () => act(adapter.signIn),
    signOut: () => act(adapter.signOut),
    dispose() {
      if (disposal) return disposal;
      disposed = true;
      unsubscribe();
      listeners.clear();
      snapshot = { ready: false, user: null, pending: false, error: null };
      disposal = adapter.dispose();
      return disposal;
    },
  };
}
