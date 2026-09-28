import { randomBytes } from 'node:crypto';
import { expect, test } from '@playwright/test';
import { signInWithCustomToken } from 'firebase/auth';
import { doc, getDocFromServer, setDoc, serverTimestamp } from 'firebase/firestore';
import {
  APPROVAL_MS,
  parseClaim,
} from '../../../services/office/functions/src/pairing-contract.js';
import { withPairing, post, seedSource } from './pairing-service-fixture.js';

test('retained approval reserves the exact disabled source, retries exactly and rejects changed intent', async () => {
  await withPairing(async ({ db, owner, request, token }) => {
    const source = await seedSource(db, owner.uid, request, undefined, undefined, {
      ...request,
      installationId: crypto.randomUUID(),
      identityId: crypto.randomUUID(),
    });
    const input = { ...request, replacesPrincipalUid: source.principalUid };
    const approved = await post('approve', input, token);
    expect(approved.status).toBe(200);
    expect(approved.body).toMatchObject({
      ...request,
      principalUid: expect.stringMatching(/^office-agent:[0-9a-f-]{36}$/),
      blockId: source.blockId,
    });
    expect(Object.keys(approved.body).sort()).toEqual(
      [...Object.keys(request), 'principalUid', 'blockId', 'expiresAt'].sort()
    );
    const record = db.collection('officePairings').doc(request.pairingId);
    const stored = (await record.get()).data()!;
    expect(stored.replacesPrincipalUid).toBe(source.principalUid);
    expect(stored.blockId).toBe(source.blockId);
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('agentGrants')
          .doc(source.principalUid)
          .get()
      ).data()
    ).toEqual({ ...source.grant, replacedByPairingId: request.pairingId });
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('blocks')
          .doc(source.blockId)
          .get()
      ).data()
    ).toEqual(source.block);
    expect(await post('approve', input, token)).toEqual(approved);

    const other = await seedSource(db, owner.uid, request);
    expect(
      (await post('approve', { ...input, replacesPrincipalUid: other.principalUid }, token)).status
    ).toBe(409);
    expect((await record.get()).data()).toEqual(stored);
  });
});

test('retained approval denies missing, malformed, active and foreign source grants', async () => {
  await withPairing(async ({ db, fixture, owner, request, token }) => {
    const cases = [
      { name: 'missing', source: { principalUid: `office-agent:${crypto.randomUUID()}` } },
      {
        name: 'malformed',
        source: await seedSource(db, owner.uid, request),
      },
      {
        name: 'active',
        source: await seedSource(db, owner.uid, request),
      },
      {
        name: 'foreign',
        source: await seedSource(db, (await fixture.client(true)).uid, request),
      },
    ];
    await db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(cases[1]!.source.principalUid)
      .update({ replacedByPairingId: 'not-a-pairing-id' });
    await db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(cases[2]!.source.principalUid)
      .update({ enabled: true });
    for (const current of cases) {
      const secret = randomBytes(32).toString('base64url');
      const currentRequest = { ...request, pairingId: parseClaim({ version: 1, secret }) };
      expect(
        (
          await post(
            'approve',
            { ...currentRequest, replacesPrincipalUid: current.source.principalUid },
            token
          )
        ).status,
        current.name
      ).toBe(409);
      expect(
        (await db.collection('officePairings').doc(currentRequest.pairingId).get()).exists,
        current.name
      ).toBe(false);
    }
  });
});

test('concurrent retained approvals consume one source; unclaimed expired or disabled reservations reclaim, claimed ones deny', async () => {
  await withPairing(async ({ db, owner, request, token }) => {
    const source = await seedSource(db, owner.uid, request, undefined, undefined, {
      ...request,
      installationId: crypto.randomUUID(),
      identityId: crypto.randomUUID(),
    });
    const contenders = [
      {
        ...request,
        pairingId: parseClaim({ version: 1, secret: randomBytes(32).toString('base64url') }),
      },
      {
        ...request,
        pairingId: parseClaim({ version: 1, secret: randomBytes(32).toString('base64url') }),
      },
    ];
    const results = await Promise.all(
      contenders.map((current) =>
        post('approve', { ...current, replacesPrincipalUid: source.principalUid }, token)
      )
    );
    expect(results.map((result) => result.status).sort()).toEqual([200, 409]);
    const winner = results.find((result) => result.status === 200)!.body;
    expect(winner.blockId).toBe(source.blockId);
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('agentGrants')
          .doc(source.principalUid)
          .get()
      ).data()
    ).toMatchObject({ ...source.grant, replacedByPairingId: winner.pairingId });
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('blocks')
          .doc(source.blockId)
          .get()
      ).data()
    ).toEqual(source.block);

    for (const state of ['expired', 'disabled'] as const) {
      const reclaimSource = await seedSource(db, owner.uid, request, undefined, undefined, {
        ...request,
        installationId: crypto.randomUUID(),
        identityId: crypto.randomUUID(),
      });
      const firstSecret = randomBytes(32).toString('base64url');
      const firstRequest = {
        ...request,
        pairingId: parseClaim({ version: 1, secret: firstSecret }),
      };
      expect(
        (
          await post(
            'approve',
            { ...firstRequest, replacesPrincipalUid: reclaimSource.principalUid },
            token
          )
        ).status
      ).toBe(200);
      const firstRecord = db.collection('officePairings').doc(firstRequest.pairingId);
      if (state === 'expired') {
        const expired = Date.now() - 1000;
        await firstRecord.update({
          createdAt: new Date(expired - APPROVAL_MS),
          expiresAt: new Date(expired),
        });
      } else await firstRecord.update({ enabled: false });
      const secondSecret = randomBytes(32).toString('base64url');
      const secondRequest = {
        ...request,
        pairingId: parseClaim({ version: 1, secret: secondSecret }),
      };
      const reclaimed = await post(
        'approve',
        { ...secondRequest, replacesPrincipalUid: reclaimSource.principalUid },
        token
      );
      expect(reclaimed.status, state).toBe(200);
      expect(reclaimed.body.blockId, state).toBe(reclaimSource.blockId);
      expect((await firstRecord.get()).data()?.enabled, state).toBe(false);
    }

    const claimedSource = await seedSource(db, owner.uid, request, undefined, undefined, {
      ...request,
      installationId: crypto.randomUUID(),
      identityId: crypto.randomUUID(),
    });
    const firstSecret = randomBytes(32).toString('base64url');
    const firstRequest = { ...request, pairingId: parseClaim({ version: 1, secret: firstSecret }) };
    expect(
      (
        await post(
          'approve',
          { ...firstRequest, replacesPrincipalUid: claimedSource.principalUid },
          token
        )
      ).status
    ).toBe(200);
    expect((await post('claim', { version: 1, secret: firstSecret })).status).toBe(200);
    const secondSecret = randomBytes(32).toString('base64url');
    expect(
      (
        await post(
          'approve',
          {
            ...request,
            pairingId: parseClaim({ version: 1, secret: secondSecret }),
            replacesPrincipalUid: claimedSource.principalUid,
          },
          token
        )
      ).status
    ).toBe(409);
  });
});

test('reserved-source reuse denies missing, malformed and mismatched predecessor evidence', async () => {
  await withPairing(async ({ db, owner, request, token }) => {
    const missing = await seedSource(db, owner.uid, request);
    await db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(missing.principalUid)
      .update({ replacedByPairingId: 'a'.repeat(64) });
    const missingRetry = {
      ...request,
      pairingId: parseClaim({ version: 1, secret: randomBytes(32).toString('base64url') }),
    };
    expect(
      (
        await post(
          'approve',
          { ...missingRetry, replacesPrincipalUid: missing.principalUid },
          token
        )
      ).status
    ).toBe(409);

    const malformed = await seedSource(db, owner.uid, request);
    const malformedId = 'b'.repeat(64);
    await db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(malformed.principalUid)
      .update({ replacedByPairingId: malformedId });
    await db.collection('officePairings').doc(malformedId).set({ malformed: true });
    const malformedRetry = {
      ...request,
      pairingId: parseClaim({ version: 1, secret: randomBytes(32).toString('base64url') }),
    };
    expect(
      (
        await post(
          'approve',
          { ...malformedRetry, replacesPrincipalUid: malformed.principalUid },
          token
        )
      ).status
    ).toBe(409);

    const mismatched = await seedSource(db, owner.uid, request);
    const firstSecret = randomBytes(32).toString('base64url');
    const firstRequest = {
      ...request,
      pairingId: parseClaim({ version: 1, secret: firstSecret }),
    };
    expect(
      (
        await post(
          'approve',
          { ...firstRequest, replacesPrincipalUid: mismatched.principalUid },
          token
        )
      ).status
    ).toBe(200);
    await db
      .collection('officePairings')
      .doc(firstRequest.pairingId)
      .update({ enabled: false, blockId: crypto.randomUUID() });
    const mismatchedRetry = {
      ...request,
      pairingId: parseClaim({ version: 1, secret: randomBytes(32).toString('base64url') }),
    };
    expect(
      (
        await post(
          'approve',
          { ...mismatchedRetry, replacesPrincipalUid: mismatched.principalUid },
          token
        )
      ).status
    ).toBe(409);
  });
});

test('retained transfer denies the old cached principal while the newly claimed principal reads the same block', async () => {
  await withPairing(async ({ fixture, db, auth, owner, request, token, secret }) => {
    const newRequest = { ...request, capabilities: ['layout.read'] as ['layout.read'] };
    const sourceRequest = {
      ...request,
      installationId: crypto.randomUUID(),
      identityId: crypto.randomUUID(),
    };
    const source = await seedSource(db, owner.uid, request, undefined, undefined, sourceRequest);
    const sourceRef = db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(source.principalUid);
    await sourceRef.update({
      enabled: true,
      createdAt: new Date(Date.now() - 1_000),
      expiresAt: new Date(Date.now() + 3_600_000),
    });
    const oldAgent = await fixture.client(false, 'device');
    await signInWithCustomToken(
      oldAgent.auth,
      await auth.createCustomToken(source.principalUid, {
        tmtOfficeAgent: true,
        tmtInstallationId: sourceRequest.installationId,
        tmtIdentityId: sourceRequest.identityId,
      })
    );
    expect(
      (
        await getDocFromServer(
          doc(oldAgent.db, 'worlds', request.worldId, 'blocks', source.blockId)
        )
      ).data()
    ).toMatchObject({ version: 1, revision: 1, objects: ['d000'] });
    await sourceRef.update({ enabled: false });
    expect(
      (await post('approve', { ...newRequest, replacesPrincipalUid: source.principalUid }, token))
        .status
    ).toBe(200);
    const claimed = await post('claim', { version: 1, secret });
    expect(claimed.status).toBe(200);
    expect(claimed.body.capabilities).toEqual(['layout.read']);
    const newAgent = await fixture.client(false, 'device');
    await signInWithCustomToken(newAgent.auth, String(claimed.body.customToken));
    await expect(
      getDocFromServer(doc(oldAgent.db, 'worlds', request.worldId, 'blocks', source.blockId))
    ).rejects.toMatchObject({ code: 'permission-denied' });
    expect(
      (
        await getDocFromServer(
          doc(newAgent.db, 'worlds', request.worldId, 'blocks', source.blockId)
        )
      ).data()
    ).toMatchObject({ version: 1, revision: 1, objects: ['d000'] });
    await expect(
      setDoc(doc(newAgent.db, 'worlds', request.worldId, 'blocks', source.blockId), {
        version: 1,
        revision: 2,
        objects: [],
        updatedAt: serverTimestamp(),
      })
    ).rejects.toMatchObject({ code: 'permission-denied' });
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('blocks')
          .doc(source.blockId)
          .get()
      ).data()
    ).toEqual(source.block);
  });
});
