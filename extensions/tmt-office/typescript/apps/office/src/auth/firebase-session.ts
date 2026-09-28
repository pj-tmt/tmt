import { deleteApp, initializeApp } from 'firebase/app';
import {
  browserPopupRedirectResolver,
  connectAuthEmulator,
  GoogleAuthProvider,
  initializeAuth,
  inMemoryPersistence,
  onAuthStateChanged,
  signInWithPopup,
  signOut,
} from 'firebase/auth';
import { createSession } from './session.js';
import {
  connectFirestoreEmulator,
  initializeFirestore,
  memoryLocalCache,
  terminate,
} from 'firebase/firestore';
import { officeFirebaseConfig, pairingEndpoint } from './firebase-config.js';
import { createWorldPort } from '../worlds/firebase-worlds.js';
import { createWorldState } from '../worlds/world-state.js';
import { createBlockPort } from '../blocks/firebase-blocks.js';
import { createPairingPort } from '../pairing/pairing-transport.js';
import { createSpacePort } from '../spaces/firebase-spaces.js';

/** One composition root for both explicit environments, outside React rendering. */
export function startOfficeRuntime(
  mode: string,
  hostname: string,
  settings: Record<string, unknown> = {}
) {
  const config = officeFirebaseConfig(mode, hostname, settings);
  if (!config) return undefined;
  const pairingUrl = pairingEndpoint(mode, settings);
  const app = initializeApp(config, `office-${crypto.randomUUID()}`);
  const auth = initializeAuth(app, {
    persistence: inMemoryPersistence,
    popupRedirectResolver: browserPopupRedirectResolver,
  });
  const db = initializeFirestore(app, { localCache: memoryLocalCache() });
  if (mode === 'emulator') {
    connectAuthEmulator(auth, 'http://127.0.0.1:9099');
    connectFirestoreEmulator(db, '127.0.0.1', 8080);
  }
  const session = createSession({
    observe: (changed) =>
      onAuthStateChanged(auth, (user) =>
        changed(user ? { uid: user.uid, displayName: user.displayName } : null)
      ),
    signIn: async () => {
      await signInWithPopup(auth, new GoogleAuthProvider());
    },
    signOut: () => signOut(auth),
    dispose: async () => {
      await terminate(db);
      await deleteApp(app);
    },
  });
  const worlds = createWorldState(session, createWorldPort(db));
  return {
    session,
    worlds,
    blocks: createBlockPort(db),
    spaces: createSpacePort(db),
    pairing: pairingUrl ? createPairingPort(auth, pairingUrl) : undefined,
    mode,
    dispose: async () => {
      worlds.dispose();
      await session.dispose();
    },
  };
}

export type OfficeRuntime = NonNullable<ReturnType<typeof startOfficeRuntime>>;
