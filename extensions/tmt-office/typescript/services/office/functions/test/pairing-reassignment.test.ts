import { Timestamp } from 'firebase-admin/firestore';
import { describe, expect, it } from 'vitest';
import { APPROVAL_MS, GRANT_MS } from '../src/pairing-contract.js';
import { createPairingStore } from '../src/pairing-store.js';
import {
  BLOCK,
  IDENTITY,
  INSTALLATION,
  NOW,
  OWNER,
  PAIRING,
  PRINCIPAL,
  WORLD,
  seed,
} from './pairing-store-fixture.js';

const replacement = PRINCIPAL;
const newPairingId = 'b'.repeat(64);
const newPrincipal = 'office-agent:00000000-0000-4000-8000-000000000099';

function request(pairingId = newPairingId) {
  return {
    version: 1 as const,
    pairingId,
    worldId: WORLD,
    installationId: INSTALLATION,
    identityId: IDENTITY,
    installationLabel: 'Installation',
    identityLabel: 'Alice',
    capabilities: ['layout.read', 'layout.write'] as ['layout.read', 'layout.write'],
  };
}

function sourceFixture() {
  const fixture = seed(NOW - 1);
  fixture.values.delete(`officePairings/${PAIRING}`);
  fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
    version: 1,
    ownerUid: OWNER,
    installationId: INSTALLATION,
    identityId: IDENTITY,
    blockId: BLOCK,
    capabilities: ['layout.read', 'layout.write'],
    enabled: false,
    createdAt: Timestamp.fromMillis(NOW - GRANT_MS),
    expiresAt: Timestamp.fromMillis(NOW - 1),
  });
  return fixture;
}

function sourceValue(fixture: ReturnType<typeof sourceFixture>): Record<string, unknown> {
  return fixture.values.get(`worlds/${WORLD}/agentGrants/${replacement}`) as Record<
    string,
    unknown
  >;
}

describe('retained block reassignment', () => {
  it('reserves a disabled source and exact retries preserve its principal choice and block', async () => {
    const fixture = sourceFixture();
    const store = createPairingStore(fixture.db, () => NOW);
    const first = await store.approve(OWNER, request(), replacement);
    const second = await store.approve(OWNER, request(), replacement);

    expect(second).toEqual(first);
    expect(first.blockId).toBe(BLOCK);
    expect(first.principalUid).toBeTruthy();
    expect(first.principalUid).not.toBe(replacement);
    expect(fixture.values.get(`worlds/${WORLD}/agentGrants/${replacement}`)).toMatchObject({
      enabled: false,
      replacedByPairingId: newPairingId,
    });
    expect(fixture.values.get(`officePairings/${newPairingId}`)).toMatchObject({
      blockId: BLOCK,
      replacesPrincipalUid: replacement,
    });
  });

  it('conflicts when a retry changes the retained-source selection', async () => {
    const fixture = sourceFixture();
    const store = createPairingStore(fixture.db, () => NOW);
    await store.approve(OWNER, request(), replacement);
    await expect(store.approve(OWNER, request(), undefined)).rejects.toMatchObject({
      code: 'PAIRING_CONFLICT',
    });
  });

  it('rejects an exact retry when the retained source block receipt is corrupted', async () => {
    const fixture = sourceFixture();
    const store = createPairingStore(fixture.db, () => NOW);
    await store.approve(OWNER, request(), replacement);
    fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
      ...sourceValue(fixture),
      blockId: '00000000-0000-4000-8000-000000000012',
    });
    await expect(store.approve(OWNER, request(), replacement)).rejects.toMatchObject({
      code: 'PAIRING_CONFLICT',
    });
  });

  it('allows the new pairing scope to differ from the retained grant scope', async () => {
    const fixture = sourceFixture();
    const store = createPairingStore(fixture.db, () => NOW);
    const approved = await store.approve(
      OWNER,
      {
        ...request(),
        installationId: '00000000-0000-4000-8000-000000000010',
        identityId: '00000000-0000-4000-8000-000000000011',
        capabilities: ['layout.read'],
      },
      replacement
    );
    expect(approved.blockId).toBe(BLOCK);
  });

  it.each([
    [
      'missing',
      (fixture: ReturnType<typeof sourceFixture>) =>
        fixture.values.delete(`worlds/${WORLD}/agentGrants/${replacement}`),
    ],
    [
      'active',
      (fixture: ReturnType<typeof sourceFixture>) =>
        fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
          ...sourceValue(fixture),
          enabled: true,
        }),
    ],
    [
      'foreign owner',
      (fixture: ReturnType<typeof sourceFixture>) =>
        fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
          ...sourceValue(fixture),
          ownerUid: 'other-owner',
        }),
    ],
    [
      'malformed',
      (fixture: ReturnType<typeof sourceFixture>) =>
        fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, { invalid: true }),
    ],
  ])('denies a %s retained source without creating a pairing', async (_name, mutate) => {
    const fixture = sourceFixture();
    mutate(fixture);
    const store = createPairingStore(fixture.db, () => NOW);
    await expect(store.approve(OWNER, request(), replacement)).rejects.toMatchObject({
      code: 'PAIRING_CONFLICT',
    });
    expect(fixture.values.has(`officePairings/${newPairingId}`)).toBe(false);
  });

  it.each([
    ['expired', { enabled: true, expiresAt: Timestamp.fromMillis(NOW) }],
    ['disabled', { enabled: false, expiresAt: Timestamp.fromMillis(NOW) }],
  ])(
    'reclaims an unclaimed %s predecessor and disables it before publishing the replacement',
    async (_name, state) => {
      const fixture = sourceFixture();
      const predecessorId = PAIRING;
      const predecessorPrincipal = newPrincipal;
      fixture.values.set(`officePairings/${predecessorId}`, {
        request: { ...request(predecessorId) },
        ownerUid: OWNER,
        principalUid: predecessorPrincipal,
        blockId: BLOCK,
        createdAt: Timestamp.fromMillis(NOW - APPROVAL_MS),
        expiresAt: state.expiresAt,
        enabled: state.enabled,
        claimed: false,
        nextClaimAt: Timestamp.fromMillis(NOW),
        replacesPrincipalUid: replacement,
      });
      fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
        ...sourceValue(fixture),
        replacedByPairingId: predecessorId,
      });
      const approved = await createPairingStore(fixture.db, () => NOW).approve(
        OWNER,
        request(),
        replacement
      );
      expect(approved.blockId).toBe(BLOCK);
      expect(fixture.values.get(`officePairings/${predecessorId}`)).toMatchObject({
        enabled: false,
      });
      expect(fixture.values.get(`worlds/${WORLD}/agentGrants/${replacement}`)).toMatchObject({
        replacedByPairingId: newPairingId,
      });
    }
  );

  it('rejects a claimed predecessor and never dislodges its reservation', async () => {
    const fixture = sourceFixture();
    fixture.values.set(`officePairings/${PAIRING}`, {
      request: { ...request(PAIRING) },
      ownerUid: OWNER,
      principalUid: newPrincipal,
      blockId: BLOCK,
      createdAt: Timestamp.fromMillis(NOW - APPROVAL_MS),
      expiresAt: Timestamp.fromMillis(NOW),
      enabled: false,
      claimed: true,
      nextClaimAt: Timestamp.fromMillis(NOW),
      replacesPrincipalUid: replacement,
    });
    fixture.values.set(`worlds/${WORLD}/agentGrants/${replacement}`, {
      ...sourceValue(fixture),
      replacedByPairingId: PAIRING,
    });
    await expect(
      createPairingStore(fixture.db, () => NOW).approve(OWNER, request(), replacement)
    ).rejects.toMatchObject({
      code: 'PAIRING_CONFLICT',
    });
    expect(fixture.values.has(`officePairings/${newPairingId}`)).toBe(false);
    expect(fixture.values.get(`worlds/${WORLD}/agentGrants/${replacement}`)).toMatchObject({
      replacedByPairingId: PAIRING,
    });
  });
});
