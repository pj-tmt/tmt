import { initializeApp, deleteApp } from 'firebase-admin/app';
import { getAuth } from 'firebase-admin/auth';
import { getFirestore } from 'firebase-admin/firestore';

/** Privileged emulator oracle; run only in the credential-free Docker test owner. */
export function createPairingEmulatorFixture() {
  if (
    process.env.GCLOUD_PROJECT !== 'demo-tmt-office' ||
    process.env.FIREBASE_AUTH_EMULATOR_HOST !== '127.0.0.1:9099' ||
    process.env.FIRESTORE_EMULATOR_HOST !== '127.0.0.1:8080'
  )
    throw new Error('Pairing tests require isolated demo emulators.');
  const app = initializeApp({ projectId: 'demo-tmt-office' }, crypto.randomUUID());
  const db = getFirestore(app);
  return {
    db,
    auth: getAuth(app),
    async dispose() {
      try {
        await db.terminate();
      } finally {
        await deleteApp(app);
      }
    },
  };
}
