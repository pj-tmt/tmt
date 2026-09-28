import { describe, expect, it, vi } from 'vitest';
import { createSession } from './session.js';
import type { SessionUser } from './session.js';

function fixture() {
  let changed: (user: SessionUser | null) => void = () => {};
  const unsubscribe = vi.fn();
  const adapter = {
    observe: vi.fn((listener: typeof changed) => {
      changed = listener;
      return unsubscribe;
    }),
    signIn: vi.fn(async () => {}),
    signOut: vi.fn(async () => {}),
    dispose: vi.fn(async () => {}),
  };
  const session = createSession(adapter);
  return { session, adapter, unsubscribe, changed: (user: SessionUser | null) => changed(user) };
}

describe('App-owned session', () => {
  it('waits for the observer and never invents identity from a successful action', async () => {
    const { session, adapter, changed } = fixture();
    await session.signIn();
    expect(adapter.signIn).not.toHaveBeenCalled();
    changed(null);
    await session.signIn();
    expect(adapter.signIn).toHaveBeenCalledOnce();
    expect(session.getSnapshot()).toEqual({ ready: true, pending: false, user: null, error: null });
    changed({ uid: 'alice', displayName: 'Alice' });
    expect(session.getSnapshot().user?.uid).toBe('alice');
    await session.signOut();
    expect(session.getSnapshot().user?.uid).toBe('alice');
    changed(null);
    expect(session.getSnapshot().user).toBeNull();
    await session.dispose();
  });

  it('serializes actions and prevents late completion or observers from reviving disposed state', async () => {
    const { session, adapter, changed, unsubscribe } = fixture();
    let finish!: () => void;
    adapter.signIn.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        })
    );
    changed(null);
    const listener = vi.fn();
    session.subscribe(listener);
    const action = session.signIn();
    await session.signIn();
    await session.signOut();
    expect(adapter.signIn).toHaveBeenCalledOnce();
    expect(adapter.signOut).not.toHaveBeenCalled();
    expect(session.getSnapshot().pending).toBe(true);
    await session.dispose();
    await session.dispose();
    listener.mockClear();
    changed({ uid: 'late', displayName: null });
    finish();
    await action;
    await session.signIn();
    expect(listener).not.toHaveBeenCalled();
    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(adapter.dispose).toHaveBeenCalledOnce();
    expect(session.getSnapshot()).toEqual({
      ready: false,
      pending: false,
      user: null,
      error: null,
    });
  });

  it.each([
    ['auth/popup-closed-by-user', 'Sign-in cancelled. You can try again.'],
    ['auth/cancelled-popup-request', 'Sign-in cancelled. You can try again.'],
    ['auth/popup-blocked', 'Allow popups for this page, then try again.'],
    [
      'auth/network-request-failed',
      'Cannot reach the sign-in service. Check your connection and try again.',
    ],
    ['secret-token-value', 'The session action failed. Please try again.'],
  ])(
    'sanitizes %s and allows recovery without changing observed identity',
    async (code, message) => {
      const { session, adapter, changed } = fixture();
      changed({ uid: 'alice', displayName: null });
      adapter.signOut.mockRejectedValueOnce({ code, message: 'private credential' });
      await session.signOut();
      expect(session.getSnapshot()).toEqual({
        ready: true,
        pending: false,
        user: { uid: 'alice', displayName: null },
        error: message,
      });
      await session.signOut();
      expect(session.getSnapshot().error).toBeNull();
      expect(adapter.signOut).toHaveBeenCalledTimes(2);
      await session.dispose();
    }
  );

  it('isolates sessions and releases view subscriptions independently', async () => {
    const first = fixture();
    const second = fixture();
    const listener = vi.fn();
    const stop = first.session.subscribe(listener);
    first.changed({ uid: 'alice', displayName: null });
    expect(listener).toHaveBeenCalledOnce();
    stop();
    first.changed(null);
    expect(listener).toHaveBeenCalledOnce();
    expect(second.session.getSnapshot().ready).toBe(false);
    await first.session.dispose();
    await second.session.dispose();
  });
});
