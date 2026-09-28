import { Timestamp } from 'firebase-admin/firestore';
import type { DecodedIdToken } from 'firebase-admin/auth';
import { describe, expect, it } from 'vitest';
import { APPROVAL_MS, GRANT_MS, RENEWAL_WINDOW_MS } from '../src/pairing-contract.js';
import { createPairingStore } from '../src/pairing-store.js';
import type { PairingAuthentication } from '../src/pairing-service.js';
import { createPairingService } from '../src/pairing-service.js';
import type { PairingStore } from '../src/pairing-store.js';

import {
  OWNER,
  WORLD,
  PAIRING,
  PRINCIPAL,
  INSTALLATION,
  IDENTITY,
  BLOCK,
  NOW,
  agent,
  seed,
} from './pairing-store-fixture.js';

describe('renewal authentication', () => {
  it('requires a checked agent token and returns only the exact public lease view', async () => {
    let observed: unknown[] | undefined;
    const response = {
      version: 1 as const,
      pairingId: PAIRING,
      worldId: WORLD,
      installationId: INSTALLATION,
      identityId: IDENTITY,
      principalUid: PRINCIPAL,
      blockId: BLOCK,
      capabilities: ['layout.read'] as ['layout.read'],
      grantExpiresAt: NOW + GRANT_MS,
    };
    const auth: PairingAuthentication = {
      verifyIdToken: async (token, checkRevoked) => {
        expect(token).toBe('agent-token');
        expect(checkRevoked).toBe(true);
        return {
          uid: PRINCIPAL,
          tmtOfficeAgent: true,
          tmtInstallationId: INSTALLATION,
          tmtIdentityId: IDENTITY,
        } as unknown as DecodedIdToken;
      },
      createCustomToken: async () => 'unused',
    };
    const store = {
      renew: async (...args: unknown[]) => {
        observed = args;
        return response;
      },
    } as unknown as PairingStore;
    const service = createPairingService(store, auth);

    await expect(
      service.renew({ version: 1, pairingId: PAIRING, grantExpiresAt: NOW }, 'Bearer agent-token')
    ).resolves.toEqual(response);
    expect(observed).toEqual([
      { uid: PRINCIPAL, installationId: INSTALLATION, identityId: IDENTITY },
      PAIRING,
      NOW,
    ]);
  });

  it.each([
    { tmtOfficeAgent: false },
    { tmtInstallationId: 'not-a-uuid' },
    { tmtIdentityId: 'not-a-uuid' },
  ])('rejects a non-agent or malformed custom claim %j', async (claims) => {
    const auth: PairingAuthentication = {
      verifyIdToken: async () =>
        ({
          uid: PRINCIPAL,
          tmtOfficeAgent: true,
          tmtInstallationId: INSTALLATION,
          tmtIdentityId: IDENTITY,
          ...claims,
        }) as unknown as DecodedIdToken,
      createCustomToken: async () => 'unused',
    };
    const store = { renew: async () => responseForTest() } as unknown as PairingStore;

    await expect(
      createPairingService(store, auth).renew(
        { version: 1, pairingId: PAIRING, grantExpiresAt: NOW },
        'Bearer agent-token'
      )
    ).rejects.toMatchObject({ code: 'PERMISSION_DENIED' });
  });
});

function responseForTest() {
  return {
    version: 1 as const,
    pairingId: PAIRING,
    worldId: WORLD,
    installationId: INSTALLATION,
    identityId: IDENTITY,
    principalUid: PRINCIPAL,
    blockId: BLOCK,
    capabilities: ['layout.read'] as ['layout.read'],
    grantExpiresAt: NOW + GRANT_MS,
  };
}

describe('renewal store', () => {
  it.each([0, Number.MAX_SAFE_INTEGER])('rejects an impossible renewal clock %s', async (now) => {
    const fixture = seed(NOW - 1);
    const store = createPairingStore(fixture.db, () => now);

    await expect(store.renew(agent, PAIRING, NOW - 1)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect(fixture.updates).toHaveLength(0);
  });

  it('renews an expired lease without changing its resource binding', async () => {
    const fixture = seed(NOW - 1);
    const store = createPairingStore(fixture.db, () => NOW);

    const renewed = await store.renew(agent, PAIRING, NOW - 1);

    expect(renewed).toEqual({
      version: 1,
      pairingId: PAIRING,
      worldId: WORLD,
      installationId: INSTALLATION,
      identityId: IDENTITY,
      principalUid: PRINCIPAL,
      blockId: BLOCK,
      capabilities: ['layout.read', 'layout.write'],
      grantExpiresAt: NOW + GRANT_MS,
    });
    expect(fixture.updates).toHaveLength(1);
    expect(fixture.values.get(`worlds/${WORLD}/agentGrants/${PRINCIPAL}`)).toMatchObject({
      ownerUid: OWNER,
      installationId: INSTALLATION,
      identityId: IDENTITY,
      blockId: BLOCK,
      capabilities: ['layout.read', 'layout.write'],
      createdAt: Timestamp.fromMillis(NOW),
      expiresAt: Timestamp.fromMillis(NOW + GRANT_MS),
    });
  });

  it('returns a newer live lease for stale expectations without extending it', async () => {
    const current = NOW + 60_000;
    const fixture = seed(current);
    const store = createPairingStore(fixture.db, () => NOW);

    await expect(store.renew(agent, PAIRING, NOW - 1)).resolves.toMatchObject({
      grantExpiresAt: current,
    });
    expect(fixture.updates).toHaveLength(0);
  });

  it('returns an expired current lease for a stale retry instead of extending it', async () => {
    const current = NOW - 1;
    const fixture = seed(current);
    const store = createPairingStore(fixture.db, () => NOW);

    await expect(store.renew(agent, PAIRING, NOW - 2)).resolves.toMatchObject({
      grantExpiresAt: current,
    });
    expect(fixture.updates).toHaveLength(0);
  });

  it.each([
    [RENEWAL_WINDOW_MS, 1],
    [RENEWAL_WINDOW_MS + 1, 0],
  ])(
    'renews only when an equal expectation is within the five-minute window (%s)',
    async (offset, updates) => {
      const current = NOW + offset;
      const fixture = seed(current);
      const store = createPairingStore(fixture.db, () => NOW);

      await store.renew(agent, PAIRING, current);

      expect(fixture.updates).toHaveLength(updates);
    }
  );

  it('rejects higher expectations and mismatched agent claims', async () => {
    const fixture = seed(NOW - 1);
    const store = createPairingStore(fixture.db, () => NOW);
    await expect(store.renew(agent, PAIRING, NOW + 1)).rejects.toMatchObject({
      code: 'PAIRING_CONFLICT',
    });
    for (const mismatch of [
      { ...agent, uid: `${PRINCIPAL}-other` },
      { ...agent, installationId: IDENTITY },
      { ...agent, identityId: INSTALLATION },
    ]) {
      await expect(store.renew(mismatch, PAIRING, NOW - 1)).rejects.toMatchObject({
        code: 'PAIRING_UNAVAILABLE',
      });
    }
  });

  it.each([
    ['disabled pairing', { pairingEnabled: false }],
    ['unclaimed approval', { claimed: false }],
    ['owner admission removed', { ownerAdmitted: false }],
    ['malformed approval', { requestPatch: { worldId: 'bad-world' } }],
    ['disabled grant', { grant: { enabled: false } }],
    ['missing grant', { missingGrant: true }],
    ['malformed grant', { grant: { extra: true } }],
    [
      'fractional grant timestamp',
      {
        grant: {
          createdAt: new Timestamp(
            Timestamp.fromMillis(NOW - GRANT_MS).seconds,
            Timestamp.fromMillis(NOW - GRANT_MS).nanoseconds + 1
          ),
        },
      },
    ],
    [
      'fractional approval timestamp',
      {
        pairingPatch: {
          createdAt: new Timestamp(
            Timestamp.fromMillis(NOW - APPROVAL_MS).seconds,
            Timestamp.fromMillis(NOW - APPROVAL_MS).nanoseconds + 1
          ),
        },
      },
    ],
  ])('%s cannot be renewed or recreated', async (_name, options) => {
    const fixture = seed(NOW - 1, options);
    const hasGrant = !('missingGrant' in options && options.missingGrant);
    const store = createPairingStore(fixture.db, () => NOW);

    await expect(store.renew(agent, PAIRING, NOW - 1)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect(fixture.updates).toHaveLength(0);
    expect(fixture.values.has(`worlds/${WORLD}/agentGrants/${PRINCIPAL}`)).toBe(hasGrant);
  });
});
