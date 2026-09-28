import { randomBytes } from 'node:crypto';
import { test, expect } from '@playwright/test';
import { signInWithCustomToken } from 'firebase/auth';
import { doc, getDocFromServer, setDoc, serverTimestamp } from 'firebase/firestore';
import { setTester } from './firestore-fixture.js';
import {
  parseClaim,
  parseApproval,
  CLAIM_INTERVAL_MS,
  APPROVAL_MS,
} from '../../../services/office/functions/src/pairing-contract.js';
import { createPairingStore } from '../../../services/office/functions/src/pairing-store.js';
import { createPairingService } from '../../../services/office/functions/src/pairing-service.js';
import { endpoint, field, post, withPairing } from './pairing-service-fixture.js';

test('HTTP approval and proof claim produce an actually usable scoped credential', async () => {
  await withPairing(async ({ fixture, db, owner, world, secret, request, token }) => {
    const approved = await post('approve', request, token);
    expect(approved.status).toBe(200);
    expect(approved.body).toMatchObject(request);
    expect(approved.body).not.toHaveProperty('customToken');
    const repeated = await post('approve', request, token);
    expect(repeated).toEqual(approved);
    const claims = await Promise.all([
      post('claim', { version: 1, secret }),
      post('claim', { version: 1, secret }),
    ]);
    expect(claims.map((r) => r.status).sort()).toEqual([200, 429]);
    const claimed = claims.find((r) => r.status === 200)!.body;
    const principalUid = field(claimed, 'principalUid');
    const blockId = field(claimed, 'blockId');
    expect(principalUid).toBe(approved.body.principalUid);
    const issuedGrant = (
      await db.collection('worlds').doc(world.id).collection('agentGrants').doc(principalUid).get()
    ).data();
    expect(claimed.grantExpiresAt).toBe(issuedGrant?.expiresAt.toMillis());
    expect(claimed.expiresAt).toBe(approved.body.expiresAt);
    expect(claimed.grantExpiresAt).toBeGreaterThan(claimed.expiresAt as number);
    expect(approved.body).not.toHaveProperty('grantExpiresAt');
    const agent = await fixture.client(false, 'device');
    await signInWithCustomToken(agent.auth, field(claimed, 'customToken'));
    expect(agent.auth.currentUser!.uid).toBe(principalUid);
    const target = doc(agent.db, 'worlds', world.id, 'blocks', blockId);
    await setDoc(target, {
      version: 1,
      revision: 1,
      objects: ['d000'],
      updatedAt: serverTimestamp(),
    });
    const stored = (
      await getDocFromServer(doc(owner.db, 'worlds', world.id, 'blocks', blockId))
    ).data();
    expect(stored?.objects).toEqual(['d000']);
    expect((await db.collection('worlds').doc(world.id).collection('agentGrants').get()).size).toBe(
      1
    );
    await expect(
      getDocFromServer(doc(agent.db, 'worlds', world.id, 'blocks', 'home'))
    ).rejects.toMatchObject({ code: 'permission-denied' });
    const revoked = await post('revoke', { version: 1, pairingId: request.pairingId }, token);
    expect(revoked).toEqual({
      status: 200,
      body: { version: 1, pairingId: request.pairingId, revoked: true },
    });
    await expect(getDocFromServer(target)).rejects.toMatchObject({ code: 'permission-denied' });
    expect(
      (await getDocFromServer(doc(owner.db, 'worlds', world.id, 'blocks', blockId))).data()
    ).toEqual(stored);
    expect((await post('claim', { version: 1, secret })).status).toBe(404);
    expect((await post('approve', request, token)).status).toBe(404);
  });
});

test('authenticated renewal converges across concurrent callers and never revives self-revoked authority', async () => {
  await withPairing(async ({ fixture, db, auth, world, secret, request, token }) => {
    expect((await post('approve', request, token)).status).toBe(200);
    const claimed = await post('claim', { version: 1, secret });
    expect(claimed.status).toBe(200);
    const principal = field(claimed.body, 'principalUid');
    const blockId = field(claimed.body, 'blockId');
    const agent = await fixture.client(false, 'device');
    await signInWithCustomToken(agent.auth, field(claimed.body, 'customToken'));
    const agentToken = await agent.auth.currentUser!.getIdToken();
    const target = doc(agent.db, 'worlds', world.id, 'blocks', blockId);
    await setDoc(target, {
      version: 1,
      revision: 1,
      objects: ['d000'],
      updatedAt: serverTimestamp(),
    });
    const block = db.collection('worlds').doc(world.id).collection('blocks').doc(blockId);
    const stored = (await block.get()).data();
    expect((await getDocFromServer(target)).data()?.objects).toEqual(['d000']);
    const grant = db.collection('worlds').doc(world.id).collection('agentGrants').doc(principal);
    const original = (await grant.get()).data()!;
    const expiredAt = Date.now() - 1000;
    await grant.update({ createdAt: new Date(expiredAt - 1000), expiresAt: new Date(expiredAt) });
    const renewal = { version: 1, pairingId: request.pairingId, grantExpiresAt: expiredAt };
    expect((await post('renew', renewal)).status).toBe(401);
    expect((await post('renew', renewal, token)).status).toBe(403);
    const wrong = await fixture.client(false, 'device');
    await signInWithCustomToken(
      wrong.auth,
      await auth.createCustomToken('office-agent:00000000-0000-4000-8000-000000000099', {
        tmtOfficeAgent: true,
        tmtInstallationId: request.installationId,
        tmtIdentityId: request.identityId,
      })
    );
    expect((await post('renew', renewal, await wrong.auth.currentUser!.getIdToken())).status).toBe(
      404
    );
    expect((await grant.get()).data()!.expiresAt.toMillis()).toBe(expiredAt);
    const results = await Promise.all([
      post('renew', renewal, agentToken),
      post('renew', renewal, agentToken),
    ]);
    expect(results[0].status).toBe(200);
    expect(results[1]).toEqual(results[0]);
    const renewed = (await grant.get()).data()!;
    expect(results[0].body).toEqual({
      version: 1,
      pairingId: request.pairingId,
      worldId: world.id,
      installationId: request.installationId,
      identityId: request.identityId,
      principalUid: principal,
      blockId,
      capabilities: request.capabilities,
      grantExpiresAt: renewed.expiresAt.toMillis(),
    });
    expect(renewed).toEqual({
      ...original,
      createdAt: renewed.createdAt,
      expiresAt: renewed.expiresAt,
    });
    expect(renewed.expiresAt.toMillis() - renewed.createdAt.toMillis()).toBe(24 * 60 * 60_000);
    expect(await post('renew', renewal, agentToken)).toEqual(results[0]);
    expect((await grant.get()).data()).toEqual(renewed);

    // Real competing transactions must leave revocation terminal in either
    // ordering. A successful renewal response is not subsequent access authority.
    await grant.delete();
    expect((await post('renew', renewal, agentToken)).status).toBe(404);
    expect((await grant.get()).exists).toBe(false);
    // Restore only through the privileged test oracle, then make a real lease
    // mutation compete with revoke (not a no-op read of a long-lived lease).
    const raceExpiry = Date.now() + 1000;
    await grant.set({
      ...renewed,
      createdAt: new Date(raceExpiry - 1000),
      expiresAt: new Date(raceExpiry),
    });
    const current = { ...renewal, grantExpiresAt: raceExpiry };
    const [raced, revoked] = await Promise.all([
      post('renew', current, agentToken),
      post('revoke', { version: 1, pairingId: request.pairingId }, agentToken),
    ]);
    expect([200, 404]).toContain(raced.status);
    expect(revoked.status).toBe(200);
    expect((await grant.get()).data()!.enabled).toBe(false);
    expect(
      (await db.collection('officePairings').doc(request.pairingId).get()).data()?.enabled
    ).toBe(false);
    expect((await block.get()).data()).toEqual(stored);
    expect((await post('renew', current, agentToken)).status).toBe(404);
    await expect(
      getDocFromServer(doc(agent.db, 'worlds', world.id, 'blocks', blockId))
    ).rejects.toMatchObject({ code: 'permission-denied' });
  });
});

test('only the exact issued agent can relinquish its claimed scope after admission loss', async () => {
  await withPairing(async ({ fixture, db, auth, owner, world, secret, request, token }) => {
    expect((await post('approve', request, token)).status).toBe(200);
    const claimed = await post('claim', { version: 1, secret });
    expect(claimed.status).toBe(200);
    const principalUid = field(claimed.body, 'principalUid');
    const record = db.collection('officePairings').doc(request.pairingId);
    const grant = db.collection('worlds').doc(world.id).collection('agentGrants').doc(principalUid);
    const beforePairing = (await record.get()).data();
    const beforeGrant = (await grant.get()).data();
    const agent = await fixture.client(false, 'device');
    await signInWithCustomToken(agent.auth, field(claimed.body, 'customToken'));
    const agentToken = await agent.auth.currentUser!.getIdToken();
    const target = doc(agent.db, 'worlds', world.id, 'blocks', field(claimed.body, 'blockId'));
    await setDoc(target, {
      version: 1,
      revision: 1,
      objects: ['d000'],
      updatedAt: serverTimestamp(),
    });
    const block = db
      .collection('worlds')
      .doc(world.id)
      .collection('blocks')
      .doc(field(claimed.body, 'blockId'));
    const stored = (await block.get()).data();
    expect((await getDocFromServer(target)).data()?.objects).toEqual(['d000']);
    for (const mismatch of [
      {
        uid: `office-agent:${crypto.randomUUID()}`,
        installationId: request.installationId,
        identityId: request.identityId,
      },
      { uid: principalUid, installationId: crypto.randomUUID(), identityId: request.identityId },
      {
        uid: principalUid,
        installationId: request.installationId,
        identityId: crypto.randomUUID(),
      },
    ]) {
      const wrong = await fixture.client(false, 'device');
      await signInWithCustomToken(
        wrong.auth,
        await auth.createCustomToken(mismatch.uid, {
          tmtOfficeAgent: true,
          tmtInstallationId: mismatch.installationId,
          tmtIdentityId: mismatch.identityId,
        })
      );
      expect(
        await post(
          'revoke',
          { version: 1, pairingId: request.pairingId },
          await wrong.auth.currentUser!.getIdToken()
        )
      ).toEqual({ status: 404, body: { error: { code: 'PAIRING_UNAVAILABLE' } } });
      expect((await record.get()).data()).toEqual(beforePairing);
      expect((await grant.get()).data()).toEqual(beforeGrant);
    }
    await setTester(owner.uid, false);
    expect((await post('revoke', { version: 1, pairingId: request.pairingId }, token)).status).toBe(
      403
    );
    const expected = {
      status: 200,
      body: { version: 1, pairingId: request.pairingId, revoked: true },
    };
    expect(await post('revoke', { version: 1, pairingId: request.pairingId }, agentToken)).toEqual(
      expected
    );
    expect(await post('revoke', { version: 1, pairingId: request.pairingId }, agentToken)).toEqual(
      expected
    );
    expect((await record.get()).data()).toEqual({ ...beforePairing, enabled: false });
    expect((await grant.get()).data()).toEqual({ ...beforeGrant, enabled: false });
    // Restore admission to prove the retained disabled grant itself denies use.
    await setTester(owner.uid, true);
    await expect(getDocFromServer(target)).rejects.toMatchObject({ code: 'permission-denied' });
    expect(
      (
        await post(
          'renew',
          { version: 1, pairingId: request.pairingId, grantExpiresAt: claimed.body.grantExpiresAt },
          agentToken
        )
      ).status
    ).toBe(404);
    expect((await block.get()).data()).toEqual(stored);
  });
});

for (const claimed of [false, true]) {
  test(`original proof cancels an expired ${claimed ? 'reserved claim' : 'unclaimed approval'} without reconstructing authority`, async () => {
    await withPairing(async ({ db, owner, secret, request, token }) => {
      const proof = { version: 1, secret };
      const record = db.collection('officePairings').doc(request.pairingId);
      expect(await post('revoke', proof)).toEqual({
        status: 404,
        body: { error: { code: 'PAIRING_UNAVAILABLE' } },
      });
      expect((await record.get()).exists).toBe(false);
      expect((await post('approve', request, token)).status).toBe(200);
      if (claimed) {
        // Reserve without signing/delivery: the original proof must also clean
        // up an issued grant whose credential response was never received.
        await createPairingStore(db).reserveClaim(request.pairingId);
      }
      const expiredAt = Date.now() - 1000;
      await record.update({
        createdAt: new Date(expiredAt - APPROVAL_MS),
        expiresAt: new Date(expiredAt),
      });
      const beforePairing = (await record.get()).data()!;
      const grant = db
        .collection('worlds')
        .doc(request.worldId)
        .collection('agentGrants')
        .doc(beforePairing.principalUid);
      const beforeGrant = (await grant.get()).data();
      await setTester(owner.uid, false);
      for (const [input, header] of [
        [{ ...proof, pairingId: request.pairingId }, undefined],
        [proof, token],
      ] as const) {
        expect((await post('revoke', input, header)).status).toBe(400);
        expect((await record.get()).data()).toEqual(beforePairing);
        expect((await grant.get()).data()).toEqual(beforeGrant);
      }
      const expected = {
        status: 200,
        body: { version: 1, pairingId: request.pairingId, revoked: true },
      };
      expect(await post('revoke', proof)).toEqual(expected);
      expect(await post('revoke', proof)).toEqual(expected);
      expect((await record.get()).data()).toEqual({ ...beforePairing, enabled: false });
      expect((await grant.get()).exists).toBe(claimed);
      if (claimed) expect((await grant.get()).data()).toEqual({ ...beforeGrant, enabled: false });
      await setTester(owner.uid, true);
      expect((await post('claim', proof)).status).toBe(404);
      expect((await post('approve', request, token)).status).toBe(404);
    });
  });
}

test('HTTP auth, input and proof failures cannot allocate credentials or cross owner scope', async () => {
  await withPairing(async ({ fixture, db, request, secret, token, owner }) => {
    expect((await post('approve', request)).status).toBe(401);
    expect((await post('approve', request, 'invalid-token')).status).toBe(401);
    for (const provider of ['google', 'unverified', 'device', 'anonymous'] as const) {
      const other = await fixture.client(true, provider);
      expect(
        (await post('approve', request, await other.auth.currentUser!.getIdToken())).status
      ).toBe(403);
    }
    expect((await post('approve', { ...request, ownerUid: owner.uid }, token)).status).toBe(400);
    expect(
      (await post('approve', { ...request, identityLabel: 'x'.repeat(5000) }, token)).status
    ).toBe(413);
    expect((await db.collection('officePairings').doc(request.pairingId).get()).exists).toBe(false);
    expect((await post('claim', { version: 1, secret })).status).toBe(404);
    expect((await post('approve', request, token)).status).toBe(200);
    expect(
      (await post('approve', { ...request, identityId: crypto.randomUUID() }, token)).status
    ).toBe(409);
    expect(
      (await post('claim', { version: 1, secret: randomBytes(32).toString('base64url') })).status
    ).toBe(404);
    await setTester(owner.uid, false);
    expect((await post('claim', { version: 1, secret })).status).toBe(404);
    expect((await post('approve', request, token)).status).toBe(403);
    expect((await post('revoke', { version: 1, pairingId: request.pairingId }, token)).status).toBe(
      403
    );
    expect(
      (await db.collection('worlds').doc(request.worldId).collection('agentGrants').get()).size
    ).toBe(0);
    await setTester(owner.uid, true);
    expect((await post('claim', { version: 1, secret })).status).toBe(200);
  });
});

test('expired approval remains revocable and a missing issued grant is never recreated', async () => {
  await withPairing(async ({ db, owner, request }) => {
    let now = Date.now();
    const store = createPairingStore(db, () => now);
    const input = parseApproval(request);
    const approved = await store.approve(owner.uid, input);
    await store.reserveClaim(request.pairingId);
    const grant = db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(approved.principalUid);
    await grant.delete();
    now += CLAIM_INTERVAL_MS;
    await expect(store.reserveClaim(request.pairingId)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    await expect(store.approve(owner.uid, input)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect((await grant.get()).exists).toBe(false);
    now += APPROVAL_MS;
    await store.revoke({ kind: 'owner', uid: owner.uid }, request.pairingId);
    await store.revoke({ kind: 'owner', uid: owner.uid }, request.pairingId);
    expect(
      (await db.collection('officePairings').doc(request.pairingId).get()).data()?.enabled
    ).toBe(false);
    expect((await grant.get()).exists).toBe(false);
    await expect(
      store.revoke({ kind: 'owner', uid: owner.uid }, '0'.repeat(64))
    ).rejects.toMatchObject({
      code: 'PERMISSION_DENIED',
    });
    const later = await store.approve(owner.uid, { ...input, pairingId: '1'.repeat(64) });
    await store.reserveClaim(later.pairingId);
    const retained = grant.parent.doc(later.principalUid);
    const before = (await retained.get()).data();
    expect(before?.enabled).toBe(true);
    now += APPROVAL_MS;
    await store.revoke({ kind: 'owner', uid: owner.uid }, later.pairingId);
    expect((await retained.get()).data()).toEqual({ ...before, enabled: false });
  });
});

test('HTTP transport and malformed durable approval fail closed without state effects', async () => {
  await withPairing(async ({ db, request, secret, token }) => {
    for (const [method, origin, status] of [
      ['GET', 'http://127.0.0.1:4173', 405],
      ['POST', 'https://untrusted.example', 403],
    ] as const) {
      const response = await fetch(`${endpoint}/approve`, {
        method,
        headers: { origin, 'content-type': 'application/json', authorization: `Bearer ${token}` },
        ...(method === 'POST' ? { body: JSON.stringify(request) } : {}),
        signal: AbortSignal.timeout(10_000),
      });
      expect(response.status).toBe(status);
    }
    for (const [path, contentType, body, status] of [
      ['approve', 'text/plain', JSON.stringify(request), 400],
      ['approve', 'application/json', '{', 400],
      ['approve', 'application/json', '', 400],
      ['unknown', 'application/json', '{}', 404],
    ] as const) {
      const response = await fetch(`${endpoint}/${path}`, {
        method: 'POST',
        headers: { 'content-type': contentType, authorization: `Bearer ${token}` },
        body,
        signal: AbortSignal.timeout(10_000),
      });
      // Malformed JSON is rejected by the platform before application dispatch.
      expect(response.status).toBe(status);
    }
    const record = db.collection('officePairings').doc(request.pairingId);
    expect((await record.get()).exists).toBe(false);
    expect((await post('approve', request, token)).status).toBe(200);
    await record.update({ unexpected: true });
    const malformed = (await record.get()).data();
    expect((await post('claim', { version: 1, secret })).status).toBe(404);
    expect((await post('approve', request, token)).status).toBe(404);
    expect((await record.get()).data()).toEqual(malformed);
    expect(
      (await db.collection('worlds').doc(request.worldId).collection('agentGrants').get()).empty
    ).toBe(true);
  });
});

test('signer failure and lost-response retries preserve one principal and the original grant lease', async () => {
  await withPairing(async ({ db, auth, owner, request, secret, token }) => {
    let now = Date.now();
    const store = createPairingStore(db, () => now);
    let fail = true;
    const service = createPairingService(store, {
      verifyIdToken: (value, revoked) => auth.verifyIdToken(value, revoked),
      createCustomToken: (uid, claims) => {
        if (fail) throw new Error('Injected signer failure; no credential disclosure');
        return auth.createCustomToken(uid, claims);
      },
    });
    const approved = await service.approve(request, `Bearer ${token}`);
    await expect(service.claim({ version: 1, secret })).rejects.toMatchObject({
      code: 'UNAVAILABLE',
    });
    const grants = db.collection('worlds').doc(request.worldId).collection('agentGrants');
    const original = (await grants.doc(approved.principalUid).get()).data();
    expect(original?.enabled).toBe(true);
    fail = false;
    now += CLAIM_INTERVAL_MS;
    const recovered = await service.claim({ version: 1, secret });
    expect(recovered.principalUid).toBe(approved.principalUid);
    expect(recovered.grantExpiresAt).toBe(original?.expiresAt.toMillis());
    now += CLAIM_INTERVAL_MS;
    expect(await service.claim({ version: 1, secret })).toMatchObject({
      principalUid: approved.principalUid,
      expiresAt: approved.expiresAt,
      grantExpiresAt: recovered.grantExpiresAt,
    });
    expect((await grants.get()).size).toBe(1);
    expect((await grants.doc(approved.principalUid).get()).data()).toEqual(original);
    // A trusted operator can shorten a grant. The response must read that live
    // expiry, not reconstruct a new 24-hour lease from the retry time.
    const shortenedExpiry = now + 60_000;
    await grants.doc(approved.principalUid).update({ expiresAt: new Date(shortenedExpiry) });
    now += CLAIM_INTERVAL_MS;
    expect((await service.claim({ version: 1, secret })).grantExpiresAt).toBe(shortenedExpiry);
    expect((await grants.doc(approved.principalUid).get()).data()?.expiresAt.toMillis()).toBe(
      shortenedExpiry
    );
    await store.revoke({ kind: 'owner', uid: owner.uid }, request.pairingId);
    now += CLAIM_INTERVAL_MS;
    await expect(service.claim({ version: 1, secret })).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
  });
});

test('approval expiry and revocation during signing prevent credential delivery', async () => {
  await withPairing(async ({ db, auth, owner, request, secret, token }) => {
    let now = Date.now();
    const store = createPairingStore(db, () => now);
    const service = createPairingService(store, {
      verifyIdToken: (value, revoked) => auth.verifyIdToken(value, revoked),
      createCustomToken: async (uid, claims) => {
        const credential = await auth.createCustomToken(uid, claims);
        await store.revoke({ kind: 'owner', uid: owner.uid }, request.pairingId);
        return credential;
      },
    });
    const approved = await service.approve(request, `Bearer ${token}`);
    await expect(service.claim({ version: 1, secret })).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    const grant = await db
      .collection('worlds')
      .doc(request.worldId)
      .collection('agentGrants')
      .doc(approved.principalUid)
      .get();
    expect(grant.data()?.enabled).toBe(false);
    const newSecret = randomBytes(32).toString('base64url');
    const newRequest = { ...request, pairingId: parseClaim({ version: 1, secret: newSecret }) };
    const expiring = await store.approve(owner.uid, parseApproval(newRequest));
    now += APPROVAL_MS;
    await expect(store.reserveClaim(expiring.pairingId)).rejects.toMatchObject({
      code: 'PAIRING_UNAVAILABLE',
    });
    expect(
      (
        await db
          .collection('worlds')
          .doc(request.worldId)
          .collection('agentGrants')
          .doc(expiring.principalUid)
          .get()
      ).exists
    ).toBe(false);
  });
});
