/** Lazy hosted-only Firebase edge. No popup resolver, Admin client or local-door authority. */
import { deleteApp, initializeApp } from 'firebase/app';
import { browserLocalPersistence, initializeAuth, signInAnonymously, signOut } from 'firebase/auth';
import {
  initializeFirestore,
  memoryLocalCache,
  collection,
  query,
  where,
  onSnapshot,
  terminate,
  Timestamp,
  Bytes,
} from 'firebase/firestore';
import { syncWait } from './firebase-sync-http.js';
import type { SyncValue } from './sealed-sync.js';
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
      const firestore = initializeFirestore(app, { localCache: memoryLocalCache() });
      let pending: Promise<string> | undefined;
      const fieldValue = (v: SyncValue): unknown => {
        if ('stringValue' in v) return v.stringValue;
        if ('integerValue' in v) return Number(v.integerValue);
        if ('booleanValue' in v) return v.booleanValue;
        if ('nullValue' in v) return null;
        if ('bytesValue' in v) return Bytes.fromBase64String(v.bytesValue);
        const millis = Date.parse(v.timestampValue);
        const fraction = /\.(\d+)Z$/.exec(v.timestampValue)?.[1] ?? '';
        return new Timestamp(Math.floor(millis / 1000), Number(fraction.padEnd(9, '0')));
      };
      shared = {
        refs: 0,
        port: {
          assertSyncIdentity(uid) {
            if (!auth.currentUser || !auth.currentUser.isAnonymous || auth.currentUser.uid !== uid)
              throw { code: 'REMOTE_SCOPE_DENIED' };
          },
          syncAuth(uid) {
            return {
              identity: Object.freeze({ projectId: config.projectId, uid }),
              async token(call) {
                const user = auth.currentUser;
                if (!user || !user.isAnonymous || user.uid !== uid)
                  return { code: 'REMOTE_STATE_UNAVAILABLE', state: 'sign-in-needed' };
                try {
                  const token = await syncWait(call, () => user.getIdTokenResult());
                  if (call.signal.aborted || Date.now() >= call.deadlineAtMs)
                    return { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
                  if (
                    auth.currentUser?.uid !== user.uid ||
                    !auth.currentUser.isAnonymous ||
                    token.claims.aud !== config.projectId ||
                    token.claims.sub !== uid ||
                    token.signInProvider !== 'anonymous' ||
                    !Number.isFinite(Date.parse(token.expirationTime)) ||
                    Date.parse(token.expirationTime) <= Date.now()
                  )
                    return { code: 'REMOTE_STATE_UNAVAILABLE', state: 'sign-in-needed' };
                  return {
                    projectId: config.projectId,
                    uid,
                    token: token.token,
                    expiresAtMs: Date.parse(token.expirationTime),
                  };
                } catch (error) {
                  if (call.signal.aborted || Date.now() >= call.deadlineAtMs)
                    return { code: 'REMOTE_STATE_UNAVAILABLE', state: 'cancelled' };
                  const code =
                    typeof error === 'object' && error !== null && 'code' in error
                      ? error.code
                      : undefined;
                  return {
                    code: 'REMOTE_STATE_UNAVAILABLE',
                    state: [
                      'auth/user-disabled',
                      'auth/user-not-found',
                      'auth/invalid-user-token',
                      'auth/user-token-expired',
                    ].includes(String(code))
                      ? 'sign-in-needed'
                      : 'unavailable',
                  };
                }
              },
            };
          },
          wake: {
            async listen(context, interest, changed, failed) {
              const user = auth.currentUser;
              if (
                !user ||
                !user.isAnonymous ||
                user.uid !== context.uid ||
                context.config.projectId !== config.projectId
              )
                throw { code: 'REMOTE_SCOPE_DENIED' };
              const resource = context.resources.find((r) => r.name === interest.resource)!;
              const constraints = Object.entries(interest.scope).map(([field, value]) =>
                where(field, '==', fieldValue(value)),
              );
              let firstServer = true;
              return onSnapshot(
                query(
                  collection(firestore, `x/${context.extension}/${resource.path}`),
                  ...constraints,
                ),
                { includeMetadataChanges: false },
                (snapshot) => {
                  if (auth.currentUser?.uid !== user.uid || !auth.currentUser.isAnonymous) {
                    failed({ code: 'REMOTE_SCOPE_DENIED' });
                    return;
                  }
                  if (
                    !snapshot.metadata.fromCache &&
                    (firstServer || snapshot.docChanges().length > 0)
                  ) {
                    const documents = firstServer ? snapshot.size : snapshot.docChanges().length;
                    firstServer = false;
                    changed(documents);
                  }
                },
                (error) =>
                  failed({
                    code:
                      error.code === 'permission-denied' || error.code === 'unauthenticated'
                        ? 'REMOTE_SCOPE_DENIED'
                        : error.code === 'resource-exhausted'
                          ? 'REMOTE_RATE_LIMITED'
                          : 'REMOTE_STATE_UNAVAILABLE',
                  }),
              );
            },
          },
          signInAnonymous() {
            pending ??= (async () => {
              if (auth.authStateReady) await auth.authStateReady();
              if (auth.currentUser) {
                if (!auth.currentUser.isAnonymous) throw { code: 'remote/auth-pending' };
                return auth.currentUser.uid;
              }
              return (await signInAnonymously(auth)).user.uid;
            })().finally(() => {
              pending = undefined;
            });
            return pending;
          },
          async signOut() {
            if (pending) throw { code: 'remote/auth-pending' };
            await signOut(auth);
          },
          async close() {
            await terminate(firestore);
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
    assertSyncIdentity: (uid) => entry.port.assertSyncIdentity!(uid),
    syncAuth: (uid) => entry.port.syncAuth!(uid),
    wake: entry.port.wake,
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
