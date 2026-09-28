import { expect } from '@playwright/test';
import { deleteApp, initializeApp } from 'firebase/app';
import {
  connectAuthEmulator,
  GoogleAuthProvider,
  initializeAuth,
  signInWithCredential,
  signInAnonymously,
  signInWithCustomToken,
} from 'firebase/auth';
import { connectFirestoreEmulator, initializeFirestore, terminate } from 'firebase/firestore';

const projectId = 'demo-tmt-office';
const documents = `http://127.0.0.1:8080/v1/projects/${projectId}/databases/(default)/documents`;

export async function readBlockDocument(worldId: string, blockId = 'home'): Promise<unknown> {
  expect(worldId).toMatch(/^[a-zA-Z0-9]{20}$/);
  expect(blockId).toMatch(/^(home|[0-9a-f-]{36})$/);
  const response = await fetch(`${documents}/worlds/${worldId}/blocks/${blockId}`, {
    headers: { authorization: 'Bearer owner' },
    signal: AbortSignal.timeout(10_000),
  });
  if (response.status === 404) return null;
  expect(response.status).toBe(200);
  return response.json();
}

export async function ownedWorlds(uid: string): Promise<unknown[]> {
  const response = await fetch(`${documents}:runQuery`, {
    method: 'POST',
    headers: { authorization: 'Bearer owner', 'content-type': 'application/json' },
    body: JSON.stringify({
      structuredQuery: {
        from: [{ collectionId: 'worlds' }],
        where: {
          fieldFilter: {
            field: { fieldPath: 'ownerUid' },
            op: 'EQUAL',
            value: { stringValue: uid },
          },
        },
        limit: 10,
      },
    }),
    signal: AbortSignal.timeout(10_000),
  });
  expect(response.status).toBe(200);
  const rows: Array<{ document?: unknown }> = await response.json();
  return rows.flatMap((row) => (row.document ? [row.document] : []));
}

export async function updateWorldName(id: string, name: string): Promise<void> {
  expect(id).toMatch(/^[a-zA-Z0-9]{20}$/);
  const response = await fetch(`${documents}/worlds/${id}?updateMask.fieldPaths=name`, {
    method: 'PATCH',
    headers: { authorization: 'Bearer owner', 'content-type': 'application/json' },
    body: JSON.stringify({ fields: { name: { stringValue: name } } }),
    signal: AbortSignal.timeout(10_000),
  });
  expect(response.status).toBe(200);
}

/** Operator fixture bypass is hard-wired to the disposable loopback emulator. */
export async function setTester(uid: string, enabled: boolean): Promise<void> {
  await writeTesterFields(uid, { enabled: { booleanValue: enabled } });
}

export async function writeTesterFields(
  uid: string,
  fields: Record<string, unknown>
): Promise<void> {
  await writeOperatorDocument(`testers/${encodeURIComponent(uid)}`, fields);
}

export async function writeAgentGrantFields(
  worldId: string,
  principalUid: string,
  fields: Record<string, unknown>
): Promise<void> {
  expect(worldId).toMatch(/^[a-zA-Z0-9]{20}$/);
  await writeOperatorDocument(
    `worlds/${worldId}/agentGrants/${encodeURIComponent(principalUid)}`,
    fields
  );
}

export async function writeBlockFields(
  worldId: string,
  blockId: string,
  fields: Record<string, unknown>
): Promise<void> {
  expect(worldId).toMatch(/^[a-zA-Z0-9]{20}$/);
  expect(blockId).toMatch(/^[0-9a-f-]{36}$/);
  await writeOperatorDocument(`worlds/${worldId}/blocks/${blockId}`, fields);
}

async function writeOperatorDocument(path: string, fields: Record<string, unknown>): Promise<void> {
  const response = await fetch(`${documents}/${path}`, {
    method: 'PATCH',
    headers: { authorization: 'Bearer owner', 'content-type': 'application/json' },
    body: JSON.stringify({ fields }),
    signal: AbortSignal.timeout(10_000),
  });
  expect(response.status).toBe(200);
}

export async function createFirestoreFixture() {
  const clients: Array<() => Promise<void>> = [];
  return {
    async client(
      approved = false,
      provider: 'google' | 'unverified' | 'anonymous' | 'device' = 'google',
      claims: Record<string, unknown> = {}
    ) {
      const app = initializeApp({ projectId, apiKey: 'demo-key' }, crypto.randomUUID());
      const auth = initializeAuth(app);
      connectAuthEmulator(auth, 'http://127.0.0.1:9099', { disableWarnings: true });
      const db = initializeFirestore(app, {});
      connectFirestoreEmulator(db, '127.0.0.1', 8080);
      clients.push(async () => {
        await terminate(db);
        await deleteApp(app);
      });
      const identity = crypto.randomUUID();
      const customToken = () => {
        // Unsigned fixture credential is accepted only by the Auth Emulator.
        const part = (value: object) => Buffer.from(JSON.stringify(value)).toString('base64url');
        return `${part({ alg: 'none', typ: 'JWT' })}.${part({ uid: identity, claims, aud: 'https://identitytoolkit.googleapis.com/google.identity.identitytoolkit.v1.IdentityToolkit', iss: 'fixture@example.test', sub: 'fixture@example.test', iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 3600 })}.`;
      };
      const { user } =
        provider === 'anonymous'
          ? await signInAnonymously(auth)
          : provider === 'device'
            ? await signInWithCustomToken(auth, customToken())
            : await signInWithCredential(
                auth,
                GoogleAuthProvider.credential(
                  JSON.stringify({
                    sub: identity,
                    email: `${identity}@example.test`,
                    email_verified: provider !== 'unverified',
                  })
                )
              );
      if (approved) await setTester(user.uid, true);
      return { db, auth, uid: user.uid };
    },
    async dispose() {
      for (const close of clients.reverse()) await close();
    },
  };
}
// Pinned Firebase uses five transaction attempts with 1s initial retry delay,
// factor 1.5 and +/-50% jitter: four waits can total 12.1875s. Allow transport
// overhead only for failed-transaction assertions, not every UI expectation.
export const TRANSACTION_FAILURE_TIMEOUT_MS = 20_000;
