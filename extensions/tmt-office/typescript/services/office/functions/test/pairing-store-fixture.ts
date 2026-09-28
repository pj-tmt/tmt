import { Timestamp } from 'firebase-admin/firestore';
import type { Firestore } from 'firebase-admin/firestore';
import { APPROVAL_MS, GRANT_MS } from '../src/pairing-contract.js';

export const OWNER = 'owner-uid';
export const WORLD = 'abcdefghijklmnopqrst';
export const PAIRING = 'a'.repeat(64);
export const PRINCIPAL = 'office-agent:00000000-0000-4000-8000-000000000001';
export const INSTALLATION = '00000000-0000-4000-8000-000000000002';
export const IDENTITY = '00000000-0000-4000-8000-000000000003';
export const BLOCK = '00000000-0000-4000-8000-000000000004';
export const NOW = 1_700_000_000_000;

type Ref = { path: string; id: string; collection(name: string): Ref; doc(id: string): Ref };

function reference(path: string): Ref {
  const parts = path.split('/');
  const ref = {
    path,
    id: parts.at(-1)!,
    collection(name: string) {
      return reference(`${path}/${name}`);
    },
    doc(id: string) {
      return reference(`${path}/${id}`);
    },
  };
  return ref;
}

function fakeFirestore(initial: Record<string, unknown>) {
  const values = new Map(Object.entries(initial));
  const updates: Array<{ path: string; value: Record<string, unknown> }> = [];
  const snapshot = (ref: Ref) => ({
    ref,
    exists: values.has(ref.path),
    data: () => values.get(ref.path),
  });
  const db = {
    collection: (name: string) => reference(name),
    runTransaction: async (operation: (transaction: unknown) => Promise<unknown>) =>
      operation({
        get: async (ref: Ref) => snapshot(ref),
        getAll: async (...refs: Ref[]) => refs.map((ref) => snapshot(ref)),
        create: (ref: Ref, value: Record<string, unknown>) => {
          if (values.has(ref.path)) throw new Error(`already exists: ${ref.path}`);
          values.set(ref.path, value);
        },
        update: (ref: Ref, value: Record<string, unknown>) => {
          const current = values.get(ref.path);
          values.set(ref.path, { ...(current as Record<string, unknown>), ...value });
          updates.push({ path: ref.path, value });
        },
      }),
  };
  return { db: db as unknown as Firestore, values, updates };
}

export function seed(
  grantExpiresAt: number,
  options: {
    pairingEnabled?: boolean;
    claimed?: boolean;
    ownerAdmitted?: boolean;
    requestPatch?: Record<string, unknown>;
    pairingPatch?: Record<string, unknown>;
    grant?: Record<string, unknown>;
    missingGrant?: boolean;
  } = {}
) {
  const pairing = {
    request: {
      version: 1 as const,
      pairingId: PAIRING,
      worldId: WORLD,
      installationId: INSTALLATION,
      identityId: IDENTITY,
      installationLabel: 'Installation',
      identityLabel: 'Alice',
      capabilities: ['layout.read', 'layout.write'] as ['layout.read', 'layout.write'],
      ...options.requestPatch,
    },
    ownerUid: OWNER,
    principalUid: PRINCIPAL,
    blockId: BLOCK,
    createdAt: Timestamp.fromMillis(NOW - APPROVAL_MS),
    expiresAt: Timestamp.fromMillis(NOW),
    enabled: options.pairingEnabled ?? true,
    claimed: options.claimed ?? true,
    nextClaimAt: Timestamp.fromMillis(NOW),
    ...options.pairingPatch,
  };
  const grant = {
    version: 1,
    ownerUid: OWNER,
    installationId: INSTALLATION,
    identityId: IDENTITY,
    blockId: BLOCK,
    capabilities: ['layout.read', 'layout.write'],
    enabled: true,
    createdAt: Timestamp.fromMillis(grantExpiresAt - GRANT_MS),
    expiresAt: Timestamp.fromMillis(grantExpiresAt),
    ...options.grant,
  };
  const initial: Record<string, unknown> = {
    [`officePairings/${PAIRING}`]: pairing,
    [`worlds/${WORLD}`]: { ownerUid: OWNER },
  };
  if (options.ownerAdmitted !== false) initial[`testers/${OWNER}`] = { enabled: true };
  if (!options.missingGrant) initial[`worlds/${WORLD}/agentGrants/${PRINCIPAL}`] = grant;
  return fakeFirestore(initial);
}

export const agent = { uid: PRINCIPAL, installationId: INSTALLATION, identityId: IDENTITY };
