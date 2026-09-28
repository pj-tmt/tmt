import { createContext, useContext, useSyncExternalStore } from 'react';
import type { ReactElement } from 'react';
import type { OfficeSession } from './session.js';

export const OfficeSessionContext = createContext<OfficeSession | undefined>(undefined);
export const OfficeModeContext = createContext('emulator');

export function SessionPanel(): ReactElement | null {
  const session = useContext(OfficeSessionContext);
  return session ? <ConnectedSession session={session} /> : null;
}

function ConnectedSession({ session }: { session: OfficeSession }): ReactElement {
  const mode = useContext(OfficeModeContext);
  const { ready, user, pending, error } = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot
  );
  return (
    <section className="session-panel" aria-label="Office session">
      <p>
        {mode === 'emulator' ? 'Auth Emulator only' : 'Private cloud pilot'} · Sign-in does not
        grant world access.
      </p>
      {!ready ? (
        <p role="status">Preparing local sign-in…</p>
      ) : user ? (
        <>
          <p>Signed in as {user.displayName ?? 'Local user'}</p>
          <p>
            UID: <code>{user.uid}</code>
          </p>
          <button type="button" disabled={pending} onClick={() => void session.signOut()}>
            Sign out
          </button>
        </>
      ) : (
        <button type="button" disabled={pending} onClick={() => void session.signIn()}>
          {mode === 'emulator' ? 'Sign in with Google (emulator)' : 'Sign in with Google'}
        </button>
      )}
      {pending && <p role="status">Waiting for the session action…</p>}
      {error && <p role="alert">{error}</p>}
      <p>Session stays in memory. Reloading signs you out.</p>
    </section>
  );
}
