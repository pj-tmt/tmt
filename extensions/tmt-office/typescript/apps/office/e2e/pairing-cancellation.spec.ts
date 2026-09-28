import { expect, test } from '@playwright/test';
import { post, withPairing } from './pairing-service-fixture.js';

test('owner cancellation serializes with approval and never resurrects an unknown request', async () => {
  await withPairing(async ({ db, request, token, secret }) => {
    const results = await Promise.all([
      post('approve', request, token),
      post('revoke', { version: 1, publicApproval: request }, token),
    ]);
    expect([200, 404]).toContain(results[0]!.status);
    expect(results[1]!.status).toBe(200);
    const pairing = db.collection('officePairings').doc(request.pairingId);
    expect((await pairing.get()).data()).toMatchObject({ request, enabled: false, claimed: false });
    expect((await post('approve', request, token)).status).toBe(404);
    expect((await post('claim', { version: 1, secret })).status).toBe(404);
    expect(
      (await db.collection('worlds').doc(request.worldId).collection('agentGrants').get()).empty
    ).toBe(true);
    expect(
      (await db.collection('worlds').doc(request.worldId).collection('blocks').get()).empty
    ).toBe(true);
  });
});

test('unknown-request cancellation requires live human ownership and preserves denied state', async () => {
  await withPairing(async ({ fixture, db, request, token, owner }) => {
    const stranger = await fixture.client(true);
    const otherToken = await stranger.auth.currentUser!.getIdToken();
    const body = { version: 1, publicApproval: request };
    const pairing = db.collection('officePairings').doc(request.pairingId);
    expect((await post('revoke', body, otherToken)).status).toBe(403);
    expect((await pairing.get()).exists).toBe(false);
    await db.collection('testers').doc(owner.uid).delete();
    expect((await post('revoke', body, token)).status).toBe(403);
    expect((await pairing.get()).exists).toBe(false);
    await db.collection('testers').doc(owner.uid).set({ enabled: true });
    expect((await post('revoke', body, token)).status).toBe(200);
    const before = (await pairing.get()).data();
    expect(
      (
        await post(
          'revoke',
          { version: 1, publicApproval: { ...request, identityLabel: 'Changed' } },
          token
        )
      ).status
    ).toBe(409);
    expect((await pairing.get()).data()).toEqual(before);
    expect((await post('revoke', body, token)).status).toBe(200);
    expect((await pairing.get()).data()).toEqual(before);
  });
});
