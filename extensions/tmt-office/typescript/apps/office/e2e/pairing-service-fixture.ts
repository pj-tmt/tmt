import { randomBytes } from 'node:crypto';
import { expect } from '@playwright/test';
import { createFirestoreFixture } from './firestore-fixture.js';
import { createPairingEmulatorFixture } from '../../../services/office/functions/test/emulator-fixture.js';
import { createWorldPort } from '../src/worlds/firebase-worlds.js';
import { parseClaim } from '../../../services/office/functions/src/pairing-contract.js';

export const endpoint = 'http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing';

export async function post(operation: string, input: unknown, token?: string) {
  const response = await fetch(`${endpoint}/${operation}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify(input),
    signal: AbortSignal.timeout(10_000),
  });
  expect(response.headers.get('cache-control')).toBe('no-store');
  return { status: response.status, body: (await response.json()) as Record<string, unknown> };
}

export function field(body: Record<string, unknown>, key: string): string {
  const value = body[key];
  if (typeof value !== 'string' || !value) throw new Error(`Missing response field ${key}`);
  return value;
}

export async function withPairing(
  run: (context: Awaited<ReturnType<typeof setup>>) => Promise<void>
) {
  const fixture = await createFirestoreFixture();
  const admin = createPairingEmulatorFixture();
  try {
    await run(await setup(fixture, admin.db, admin.auth));
  } finally {
    try {
      await fixture.dispose();
    } finally {
      await admin.dispose();
    }
  }
}

async function setup(
  fixture: Awaited<ReturnType<typeof createFirestoreFixture>>,
  db: ReturnType<typeof createPairingEmulatorFixture>['db'],
  auth: ReturnType<typeof createPairingEmulatorFixture>['auth']
) {
  const owner = await fixture.client(true);
  const worlds = createWorldPort(owner.db);
  const world = worlds.draft('Pairing workshop');
  await worlds.create(world, owner.uid);
  const secret = randomBytes(32).toString('base64url');
  const pairingId = parseClaim({ version: 1, secret });
  const request = {
    version: 1 as const,
    pairingId,
    worldId: world.id,
    installationId: crypto.randomUUID(),
    identityId: crypto.randomUUID(),
    installationLabel: 'Test installation',
    identityLabel: 'Alice',
    capabilities: ['layout.read', 'layout.write'] as ['layout.read', 'layout.write'],
  };
  const token = await owner.auth.currentUser!.getIdToken();
  return { fixture, db, auth, owner, world, secret, request, token };
}

export async function seedSource(
  db: ReturnType<typeof createPairingEmulatorFixture>['db'],
  ownerUid: string,
  request: {
    worldId: string;
    installationId: string;
    identityId: string;
    capabilities: ['layout.read'] | ['layout.read', 'layout.write'];
  },
  principalUid = `office-agent:${crypto.randomUUID()}`,
  blockId = crypto.randomUUID(),
  sourceRequest = request
) {
  const createdAt = new Date(Date.now() - 120_000);
  const fields = {
    version: 1,
    ownerUid,
    installationId: sourceRequest.installationId,
    identityId: sourceRequest.identityId,
    blockId,
    capabilities: sourceRequest.capabilities,
    enabled: false,
    createdAt,
    expiresAt: new Date(createdAt.getTime() + 60_000),
  };
  const grantRef = db
    .collection('worlds')
    .doc(request.worldId)
    .collection('agentGrants')
    .doc(principalUid);
  await grantRef.set(fields);
  const blockRef = db.collection('worlds').doc(request.worldId).collection('blocks').doc(blockId);
  await blockRef.set({ version: 1, revision: 1, objects: ['d000'], updatedAt: new Date() });
  return {
    principalUid,
    blockId,
    grant: (await grantRef.get()).data()!,
    block: (await blockRef.get()).data()!,
  };
}
