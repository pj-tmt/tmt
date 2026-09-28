import { createHash } from 'node:crypto';
import type { DecodedIdToken } from 'firebase-admin/auth';
import { describe, expect, it, vi } from 'vitest';
import { createPairingStore } from '../src/pairing-store.js';
import type { RevocationActor } from '../src/pairing-store.js';
import { createPairingService } from '../src/pairing-service.js';
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

const pairingPath = `officePairings/${PAIRING}`;
const grantPath = `worlds/${WORLD}/agentGrants/${PRINCIPAL}`;
const publicApproval = {
  version: 1 as const,
  pairingId: PAIRING,
  worldId: WORLD,
  installationId: INSTALLATION,
  identityId: IDENTITY,
  installationLabel: 'Installation',
  identityLabel: 'Alice',
  capabilities: ['layout.read', 'layout.write'] as ['layout.read', 'layout.write'],
};
const actors: RevocationActor[] = [
  { kind: 'owner', uid: OWNER },
  { kind: 'agent', agent },
  { kind: 'proof' },
];

describe('revocation transaction', () => {
  it.each(actors)(
    'only disables authority and retains resources across expiry and retries: $kind',
    async (actor) => {
      const fixture = seed(NOW - 1);
      const blockPath = `worlds/${WORLD}/blocks/${BLOCK}`;
      fixture.values.set(blockPath, { content: 'retained' });
      const before = new Map(fixture.values);
      const store = createPairingStore(fixture.db, () => NOW);
      await store.revoke(actor, PAIRING);
      await store.revoke(actor, PAIRING);
      for (const [path, value] of before) {
        expect(fixture.values.get(path)).toEqual(
          path === pairingPath || path === grantPath
            ? { ...(value as Record<string, unknown>), enabled: false }
            : value
        );
      }
      expect(fixture.values.size).toBe(before.size);
    }
  );

  it.each(actors)('never reconstructs a missing grant: $kind', async (actor) => {
    const fixture = seed(NOW - 1, { missingGrant: true });
    await createPairingStore(fixture.db).revoke(actor, PAIRING);
    expect(fixture.values.has(grantPath)).toBe(false);
    expect(fixture.updates).toEqual([{ path: pairingPath, value: { enabled: false } }]);
  });

  it.each(actors.filter((actor) => actor.kind !== 'owner'))(
    'permits reduction after owner admission loss: $kind',
    async (actor) => {
      const fixture = seed(NOW - 1, { ownerAdmitted: false, pairingEnabled: false });
      await createPairingStore(fixture.db).revoke(actor, PAIRING);
      expect(fixture.values.get(grantPath)).toMatchObject({ enabled: false });
    }
  );

  it('cancels an unclaimed approval without creating a grant', async () => {
    const fixture = seed(NOW - 1, { claimed: false, missingGrant: true, ownerAdmitted: false });
    await createPairingStore(fixture.db).revoke({ kind: 'proof' }, PAIRING);
    expect(fixture.values.get(pairingPath)).toMatchObject({ enabled: false, claimed: false });
    expect(fixture.values.has(grantPath)).toBe(false);
  });

  it.each([
    { ...agent, uid: OWNER },
    { ...agent, installationId: IDENTITY },
    { ...agent, identityId: INSTALLATION },
  ])('denies mismatched claimed agent without any mutation: %j', async (mismatch) => {
    const fixture = seed(NOW - 1);
    const before = new Map(fixture.values);
    await expect(
      createPairingStore(fixture.db).revoke({ kind: 'agent', agent: mismatch }, PAIRING)
    ).rejects.toMatchObject({ code: 'PAIRING_UNAVAILABLE' });
    expect(fixture.values).toEqual(before);
    expect(fixture.updates).toEqual([]);
  });

  it('denies an unclaimed agent and an owner with lost admission', async () => {
    for (const actor of actors.slice(0, 2)) {
      const fixture = seed(NOW - 1, { claimed: false, ownerAdmitted: false });
      const before = new Map(fixture.values);
      await expect(createPairingStore(fixture.db).revoke(actor, PAIRING)).rejects.toMatchObject({
        code: actor.kind === 'owner' ? 'PERMISSION_DENIED' : 'PAIRING_UNAVAILABLE',
      });
      expect(fixture.values).toEqual(before);
      expect(fixture.updates).toEqual([]);
    }
  });

  it.each(actors)(
    'denies missing and malformed records without tombstones: $kind',
    async (actor) => {
      for (const malformed of [false, true]) {
        const fixture = seed(NOW - 1);
        if (malformed) fixture.values.set(pairingPath, { invalid: true });
        else fixture.values.delete(pairingPath);
        const before = new Map(fixture.values);
        await expect(createPairingStore(fixture.db).revoke(actor, PAIRING)).rejects.toMatchObject({
          code: actor.kind === 'owner' ? 'PERMISSION_DENIED' : 'PAIRING_UNAVAILABLE',
        });
        expect(fixture.values).toEqual(before);
        expect(fixture.updates).toEqual([]);
      }
    }
  );
});

describe('revocation authentication and proof', () => {
  function setup(token: Record<string, unknown> = {}) {
    const fixture = seed(NOW - 1);
    const verifyIdToken = vi.fn(async () => token as unknown as DecodedIdToken);
    const createCustomToken = vi.fn(async () => 'unused');
    const service = createPairingService(createPairingStore(fixture.db), {
      verifyIdToken,
      createCustomToken,
    });
    return { ...fixture, service, verifyIdToken, createCustomToken };
  }
  const agentToken = {
    uid: PRINCIPAL,
    tmtOfficeAgent: true,
    tmtInstallationId: INSTALLATION,
    tmtIdentityId: IDENTITY,
  };

  it.each([
    agentToken,
    { uid: OWNER, email_verified: true, firebase: { sign_in_provider: 'google.com' } },
  ])('uses checked authentication for owner or agent without minting: %j', async (token) => {
    const fixture = setup(token);
    await expect(
      fixture.service.revoke({ version: 1, pairingId: PAIRING }, 'Bearer checked-token')
    ).resolves.toEqual({ version: 1, pairingId: PAIRING, revoked: true });
    expect(fixture.verifyIdToken).toHaveBeenCalledExactlyOnceWith('checked-token', true);
    expect(fixture.createCustomToken).not.toHaveBeenCalled();
    expect(fixture.values.get(grantPath)).toMatchObject({ enabled: false });
  });

  it.each([
    { ...agentToken, tmtInstallationId: 'invalid' },
    { ...agentToken, tmtIdentityId: 'invalid' },
    { ...agentToken, tmtOfficeAgent: false },
    { uid: OWNER, email_verified: false, firebase: { sign_in_provider: 'google.com' } },
  ])('denies malformed or non-authoritative tokens before writes: %j', async (token) => {
    const fixture = setup(token);
    await expect(
      fixture.service.revoke({ version: 1, pairingId: PAIRING }, 'Bearer token')
    ).rejects.toMatchObject({ code: 'PERMISSION_DENIED' });
    expect(fixture.updates).toEqual([]);
  });

  it('denies revoked tokens and absent authentication without writes', async () => {
    const fixture = setup(agentToken);
    fixture.verifyIdToken.mockRejectedValue(new Error('revoked'));
    for (const header of [undefined, 'Bearer revoked']) {
      await expect(
        fixture.service.revoke({ version: 1, pairingId: PAIRING }, header)
      ).rejects.toMatchObject({ code: 'UNAUTHENTICATED' });
    }
    expect(fixture.updates).toEqual([]);
  });

  const bytes = Buffer.alloc(32, 1);
  const secret = bytes.toString('base64url');
  const digest = createHash('sha256').update(bytes).digest('hex');

  it('uses the original byte digest for known-proof cancellation and never authenticates or mints', async () => {
    const fixture = setup();
    const pairing = fixture.values.get(pairingPath) as { request: Record<string, unknown> };
    fixture.values.delete(pairingPath);
    fixture.values.set(`officePairings/${digest}`, {
      ...pairing,
      request: { ...pairing.request, pairingId: digest },
    });
    await expect(fixture.service.revoke({ version: 1, secret })).resolves.toEqual({
      version: 1,
      pairingId: digest,
      revoked: true,
    });
    expect(fixture.values.get(grantPath)).toMatchObject({ enabled: false });
    expect(fixture.verifyIdToken).not.toHaveBeenCalled();
    expect(fixture.createCustomToken).not.toHaveBeenCalled();
  });

  it('keeps unknown proof unavailable without creating a tombstone', async () => {
    const fixture = setup();
    await expect(fixture.service.revoke({ version: 1, secret })).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect(fixture.values.has(`officePairings/${digest}`)).toBe(false);
    expect(fixture.updates).toEqual([]);
  });

  it('lets an admitted owner cancel an unknown request without grants or resources', async () => {
    const fixture = setup({
      uid: OWNER,
      email_verified: true,
      firebase: { sign_in_provider: 'google.com' },
    });
    fixture.values.delete(pairingPath);
    await expect(
      fixture.service.revoke({ version: 1, publicApproval }, 'Bearer owner-token')
    ).resolves.toEqual({ version: 1, pairingId: PAIRING, revoked: true });
    expect(fixture.values.get(pairingPath)).toMatchObject({
      request: publicApproval,
      ownerUid: OWNER,
      enabled: false,
      claimed: false,
    });
    expect(fixture.values.has(grantPath)).toBe(true);
    expect(fixture.values.get(grantPath)).toMatchObject({ enabled: true });
    expect(fixture.values.size).toBe(4);
    expect(fixture.createCustomToken).not.toHaveBeenCalled();
  });

  it('cancels a known request only for the exact admitted owner request and is idempotent', async () => {
    const fixture = setup({
      uid: OWNER,
      email_verified: true,
      firebase: { sign_in_provider: 'google.com' },
    });
    await expect(
      fixture.service.revoke({ version: 1, publicApproval }, 'Bearer owner-token')
    ).resolves.toEqual({ version: 1, pairingId: PAIRING, revoked: true });
    const afterFirst = new Map(fixture.values);
    await expect(
      fixture.service.revoke({ version: 1, publicApproval }, 'Bearer owner-token')
    ).resolves.toEqual({ version: 1, pairingId: PAIRING, revoked: true });
    expect(fixture.values.get(pairingPath)).toMatchObject({ enabled: false });
    expect(fixture.values.get(grantPath)).toMatchObject({ enabled: false });
    expect(fixture.values.get(`worlds/${WORLD}`)).toEqual(afterFirst.get(`worlds/${WORLD}`));
    expect(fixture.verifyIdToken).toHaveBeenCalledTimes(2);
  });

  it('denies changed intent, wrong owner/admission and agent public cancellation without writes', async () => {
    const changed = { ...publicApproval, capabilities: ['layout.read'] as ['layout.read'] };
    const cases = [
      {
        token: { uid: OWNER, email_verified: true, firebase: { sign_in_provider: 'google.com' } },
        input: { version: 1, publicApproval: changed },
        code: 'PAIRING_CONFLICT',
      },
      {
        token: {
          uid: 'other-owner',
          email_verified: true,
          firebase: { sign_in_provider: 'google.com' },
        },
        input: { version: 1, publicApproval },
        code: 'PERMISSION_DENIED',
      },
      {
        token: { uid: OWNER, email_verified: true, firebase: { sign_in_provider: 'google.com' } },
        input: { version: 1, publicApproval },
        code: 'PERMISSION_DENIED',
        ownerAdmitted: false,
      },
      { token: agentToken, input: { version: 1, publicApproval }, code: 'PERMISSION_DENIED' },
    ];
    for (const testCase of cases) {
      const fixture = setup(testCase.token);
      if (testCase.ownerAdmitted === false) fixture.values.delete(`testers/${OWNER}`);
      const before = new Map(fixture.values);
      await expect(fixture.service.revoke(testCase.input, 'Bearer token')).rejects.toMatchObject({
        code: testCase.code,
      });
      expect(fixture.values).toEqual(before);
      expect(fixture.updates).toEqual([]);
    }
  });

  it('prevents late approval and claim of an owner-cancelled unknown request', async () => {
    const fixture = setup({
      uid: OWNER,
      email_verified: true,
      firebase: { sign_in_provider: 'google.com' },
    });
    fixture.values.delete(pairingPath);
    const store = createPairingStore(fixture.db, () => NOW);
    await store.revoke({ kind: 'ownerApproval', uid: OWNER, request: publicApproval }, PAIRING);
    const before = new Map(fixture.values);
    await expect(store.approve(OWNER, publicApproval)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    await expect(store.reserveClaim(PAIRING)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect(fixture.values).toEqual(before);
    expect(fixture.updates).toEqual([]);
  });

  it.each([
    [{ version: 1, secret: secret + '=' }, undefined],
    [{ version: 1, secret: secret.slice(1) }, undefined],
    [{ version: 1, secret: secret.slice(0, -1) + 'R' }, undefined],
    [{ version: 2, secret }, undefined],
    [{ version: 1, secret, pairingId: PAIRING }, undefined],
    [{ version: 1, secret, extra: true }, undefined],
    [{ version: 1, secret }, 'Bearer token'],
    [{ version: 1, secret }, ''],
  ])(
    'rejects malformed or ambiguous proof requests before effects: %j',
    async (input, authorization) => {
      const fixture = setup();
      await expect(fixture.service.revoke(input, authorization)).rejects.toMatchObject({
        code: 'INVALID_ARGUMENT',
      });
      expect(fixture.updates).toEqual([]);
      expect(fixture.verifyIdToken).not.toHaveBeenCalled();
    }
  );
});
