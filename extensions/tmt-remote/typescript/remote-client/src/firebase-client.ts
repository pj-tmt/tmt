/** Lazy hosted-only Firebase edge. No popup resolver, Admin client or local-door authority. */
import { deleteApp, initializeApp } from 'firebase/app';
import { browserLocalPersistence, initializeAuth, signInAnonymously, signOut } from 'firebase/auth';
import type { FirebaseAuthPort } from './firebase-auth.js';
import type { PublicWebConfig } from './firebase-bootstrap.js';

interface Shared {
  refs: number;
  port: FirebaseAuthPort;
}
const clients = new Map<string, Shared>();

export async function initialize(config: PublicWebConfig): Promise<FirebaseAuthPort> {
  const name = JSON.stringify([config.projectId, config.appId, config.authDomain, config.apiKey]);
  let shared = clients.get(name);
  if (!shared) {
    const app = initializeApp(config, name);
    try {
      const auth = initializeAuth(app, { persistence: browserLocalPersistence });
      let pending: Promise<string> | undefined;
      shared = {
        refs: 0,
        port: {
          signInAnonymous() {
            pending ??= signInAnonymously(auth)
              .then((result) => result.user.uid)
              .finally(() => {
                pending = undefined;
              });
            return pending;
          },
          async signOut() {
            if (pending) throw { code: 'remote/auth-pending' };
            await signOut(auth);
          },
          async close() {
            await deleteApp(app);
          },
        },
      };
      clients.set(name, shared);
    } catch (error) {
      await deleteApp(app);
      throw error;
    }
  }
  const entry = shared;
  entry.refs++;
  let closed = false;
  return {
    signInAnonymous: () => entry.port.signInAnonymous(),
    signOut: () => entry.port.signOut(),
    async close() {
      if (closed) return;
      closed = true;
      if (--entry.refs === 0) {
        clients.delete(name);
        await entry.port.close();
      }
    },
  };
}
