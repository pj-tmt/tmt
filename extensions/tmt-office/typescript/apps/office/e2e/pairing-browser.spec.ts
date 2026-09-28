import { randomBytes, createHash } from 'node:crypto';
import { expect } from '@playwright/test';
import type { Page } from '@playwright/test';
import { signInWithCustomToken } from 'firebase/auth';
import { doc, getDocFromServer, setDoc, serverTimestamp } from 'firebase/firestore';
import { test, signIn } from './browser-session.js';
import {
  createFirestoreFixture,
  setTester,
  writeAgentGrantFields,
  writeBlockFields,
} from './firestore-fixture.js';
import { createPairingEmulatorFixture } from '../../../services/office/functions/test/emulator-fixture.js';

const endpoint = 'http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing';

function requestFor(worldId: string) {
  const bytes = randomBytes(32);
  const secret = bytes.toString('base64url');
  const request = {
    version: 1,
    pairingId: createHash('sha256').update(bytes).digest('hex'),
    worldId,
    installationId: crypto.randomUUID(),
    identityId: crypto.randomUUID(),
    installationLabel: 'My workstation',
    identityLabel: 'Alice <helper>',
    capabilities: ['layout.read', 'layout.write'],
  };
  const fragment = `tmt-pair=${Buffer.from(JSON.stringify(request)).toString('base64url')}`;
  return { request, secret, url: `http://127.0.0.1:4173/worlds/${worldId}/pair#${fragment}` };
}

async function admitOwner(
  page: Page,
  db: ReturnType<typeof createPairingEmulatorFixture>['db'],
  worldId: string
) {
  await signIn(page, 'OfficeOwner');
  const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
  await db
    .collection('worlds')
    .doc(worldId)
    .set({ version: 1, name: 'My personal office', ownerUid: uid, createdAt: new Date() });
  await expect(page.getByRole('heading', { name: 'Waiting for access' })).toBeVisible();
  await setTester(uid, true);
  await expect(page.getByRole('region', { name: 'Agent pairing request' })).toBeVisible();
  return uid;
}

test('explicit browser consent issues usable scoped access and revocation preserves the workspace', async ({
  openSession,
}, info) => {
  const admin = createPairingEmulatorFixture();
  const clients = await createFirestoreFixture();
  try {
    const worldId = admin.db.collection('worlds').doc().id;
    const { request, secret, url } = requestFor(worldId);
    const { page, requests } = await openSession(url);
    await expect(page.getByRole('button', { name: 'Approve pairing' })).toHaveCount(0);
    await admitOwner(page, admin.db, worldId);
    const record = admin.db.collection('officePairings').doc(request.pairingId);
    expect((await record.get()).exists).toBe(false);
    expect(requests.filter((target) => target.pathname.endsWith('/approve'))).toEqual([]);
    await expect(page.getByText('Alice <helper>', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Approve pairing' })).toBeDisabled();
    await page.getByRole('link', { name: 'Skip to content' }).focus();
    await page.keyboard.press('Enter');
    expect(page.url()).toBe(url);
    await expect(page.locator('#main')).toBeFocused();
    await page.locator('#main').evaluate((element) => element.blur());
    await page.screenshot({ path: info.outputPath('pairing-desktop.png'), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: info.outputPath('pairing-narrow.png'), fullPage: true });
    await page.getByRole('checkbox', { name: 'I recognize this agent and installation.' }).check();
    const approvalRequest = page.waitForRequest(
      (outgoing) => outgoing.url() === `${endpoint}/approve` && outgoing.method() === 'POST'
    );
    await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
    const outgoing = await approvalRequest;
    expect(outgoing.postDataJSON()).toEqual(request);
    expect(outgoing.url()).not.toContain(secret);
    expect(outgoing.postData()).not.toContain(secret);
    expect(await outgoing.headerValue('authorization')).toMatch(/^Bearer \S+$/);
    await expect(page.getByRole('status').filter({ hasText: 'Approved. Return' })).toBeVisible();
    const stored = (await record.get()).data()!;
    expect(stored.request).toEqual(request);
    const approvals = requests.filter((target) => target.pathname.endsWith('/approve')).length;
    await page.getByRole('link', { name: 'Office', exact: true }).click();
    await expect(page.getByRole('region', { name: 'Agent pairing request' })).toHaveCount(0);
    await page.goBack();
    await expect(page.getByRole('button', { name: 'Approve pairing', exact: true })).toBeDisabled();
    expect(page.url()).toBe(url);
    expect(requests.filter((target) => target.pathname.endsWith('/approve'))).toHaveLength(
      approvals
    );
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Approved. Return' })).toBeVisible();
    expect((await record.get()).data()).toEqual(stored);
    const claim = await fetch(`${endpoint}/claim`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ version: 1, secret }),
      signal: AbortSignal.timeout(10_000),
    });
    expect(claim.status).toBe(200);
    const credential = (await claim.json()) as {
      customToken: string;
      principalUid: string;
      blockId: string;
    };
    expect(credential.principalUid).toBe(stored.principalUid);
    expect(credential.blockId).toBe(stored.blockId);
    const agent = await clients.client(false, 'device');
    await signInWithCustomToken(agent.auth, credential.customToken);
    const target = doc(agent.db, 'worlds', worldId, 'blocks', credential.blockId);
    await setDoc(target, {
      version: 1,
      revision: 1,
      objects: ['d000'],
      updatedAt: serverTimestamp(),
    });
    const block = admin.db
      .collection('worlds')
      .doc(worldId)
      .collection('blocks')
      .doc(credential.blockId);
    const before = (await block.get()).data();
    expect(before?.objects).toEqual(['d000']);
    await page.getByRole('button', { name: 'Revoke request' }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Request revoked.' })).toBeVisible();
    expect((await record.get()).data()?.enabled).toBe(false);
    await expect(getDocFromServer(target)).rejects.toMatchObject({ code: 'permission-denied' });
    expect((await block.get()).data()).toEqual(before);
    expect(page.url()).not.toContain(secret);
    expect(await page.locator('body').innerText()).not.toContain(credential.customToken);
    expect(requests.some((target) => target.href.includes(secret))).toBe(false);
  } finally {
    try {
      await clients.dispose();
    } finally {
      await admin.dispose();
    }
  }
});

test('owner pages retained spaces, submits only the source principal and freezes retry intent', async ({
  openSession,
}, info) => {
  const admin = createPairingEmulatorFixture();
  try {
    const worldId = admin.db.collection('worlds').doc().id;
    const { request, url } = requestFor(worldId);
    const { page } = await openSession(url);
    const ownerUid = await admitOwner(page, admin.db, worldId);
    const sourcePrincipal = 'office-agent:ffffffff-ffff-4fff-8fff-ffffffffffff';
    const sourceBlock = crypto.randomUUID();
    const sourceIdentity = crypto.randomUUID();
    const sourceInstallation = crypto.randomUUID();
    const sourceFields = {
      version: { integerValue: '1' },
      ownerUid: { stringValue: ownerUid },
      installationId: { stringValue: sourceInstallation },
      identityId: { stringValue: sourceIdentity },
      blockId: { stringValue: sourceBlock },
      capabilities: {
        arrayValue: { values: [{ stringValue: 'layout.read' }, { stringValue: 'layout.write' }] },
      },
      enabled: { booleanValue: false },
      createdAt: { timestampValue: new Date(Date.now() - 120_000).toISOString() },
      expiresAt: { timestampValue: new Date(Date.now() - 60_000).toISOString() },
    };
    for (let index = 0; index < 20; index++) {
      await writeAgentGrantFields(
        worldId,
        `office-agent:00000000-0000-4000-8000-${index.toString().padStart(12, '0')}`,
        {
          ...sourceFields,
          installationId: { stringValue: crypto.randomUUID() },
          identityId: { stringValue: crypto.randomUUID() },
          blockId: { stringValue: crypto.randomUUID() },
        }
      );
    }
    await writeAgentGrantFields(worldId, sourcePrincipal, sourceFields);
    await writeBlockFields(worldId, sourceBlock, {
      version: { integerValue: '1' },
      revision: { integerValue: '1' },
      objects: { arrayValue: { values: [{ stringValue: 'd000' }] } },
      updatedAt: { timestampValue: new Date().toISOString() },
    });
    const sourceRef = admin.db
      .collection('worlds')
      .doc(worldId)
      .collection('agentGrants')
      .doc(sourcePrincipal);
    const blockRef = admin.db
      .collection('worlds')
      .doc(worldId)
      .collection('blocks')
      .doc(sourceBlock);
    const sourceBefore = (await sourceRef.get()).data()!;
    const blockBefore = (await blockRef.get()).data()!;

    await page.getByRole('button', { name: 'Refresh retained spaces', exact: true }).click();
    await expect(
      page.getByRole('button', { name: 'Next retained spaces', exact: true })
    ).toBeEnabled();
    await page.getByRole('button', { name: 'Next retained spaces', exact: true }).click();
    const retained = page.getByRole('radio', {
      name: new RegExp(`Retained block ${sourceBlock} · Previous identity ${sourceIdentity}`),
    });
    await expect(retained).toBeVisible();
    await retained.check();
    await expect(
      page.getByRole('radio', { name: 'New empty block', exact: true })
    ).not.toBeChecked();
    await page.getByRole('checkbox', { name: 'I recognize this agent and installation.' }).check();
    await page.screenshot({
      path: info.outputPath('retained-pairing-desktop.png'),
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: info.outputPath('retained-pairing-narrow.png'), fullPage: true });

    const firstRequest = page.waitForRequest(
      (outgoing) => outgoing.url() === `${endpoint}/approve` && outgoing.method() === 'POST'
    );
    await page.route(
      '**/officePairing/approve',
      async (route) => {
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        await route.abort();
      },
      { times: 1 }
    );
    await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
    const firstOutgoing = await firstRequest;
    expect(firstOutgoing.postDataJSON()).toEqual({
      ...request,
      replacesPrincipalUid: sourcePrincipal,
    });
    await expect(page.getByRole('alert')).toContainText('may already have completed');
    await expect(
      page.getByText(`Selected retained block: ${sourceBlock} · Source grant: ${sourcePrincipal}`, {
        exact: true,
      })
    ).toBeVisible();
    await expect(
      page.getByText('The choice is fixed for this request. Retry keeps the same assignment.', {
        exact: true,
      })
    ).toBeVisible();
    await expect(page.getByRole('radio', { name: /Retained block/ })).toHaveCount(0);
    await expect(page.getByRole('radio', { name: 'New empty block', exact: true })).toBeDisabled();

    const retryRequest = page.waitForRequest(
      (outgoing) => outgoing.url() === `${endpoint}/approve` && outgoing.method() === 'POST'
    );
    await page.getByRole('button', { name: 'Retry same approval', exact: true }).click();
    expect((await retryRequest).postDataJSON()).toEqual({
      ...request,
      replacesPrincipalUid: sourcePrincipal,
    });
    await expect(page.getByRole('status').filter({ hasText: 'Approved. Return' })).toBeVisible();
    const pairing = (
      await admin.db.collection('officePairings').doc(request.pairingId).get()
    ).data()!;
    expect(pairing.blockId).toBe(sourceBlock);
    expect(pairing.replacesPrincipalUid).toBe(sourcePrincipal);
    expect((await sourceRef.get()).data()).toEqual({
      ...sourceBefore,
      replacedByPairingId: request.pairingId,
    });
    expect((await blockRef.get()).data()).toEqual(blockBefore);
  } finally {
    await admin.dispose();
  }
});

test('an admitted non-owner cannot use the pairing route to approve another world', async ({
  openSession,
}) => {
  const admin = createPairingEmulatorFixture();
  try {
    const worldId = admin.db.collection('worlds').doc().id;
    const { request, url } = requestFor(worldId);
    await admin.db.collection('worlds').doc(worldId).set({
      version: 1,
      name: 'Another owner private office',
      ownerUid: crypto.randomUUID(),
      createdAt: new Date(),
    });
    const { page, requests } = await openSession(url);
    await signIn(page, 'Visitor');
    const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
    await setTester(uid, true);
    await expect(page.getByRole('heading', { name: 'World unavailable' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Agent pairing request' })).toHaveCount(0);
    expect(requests.filter((target) => target.pathname.endsWith('/approve'))).toEqual([]);
    expect((await admin.db.collection('officePairings').doc(request.pairingId).get()).exists).toBe(
      false
    );
  } finally {
    await admin.dispose();
  }
});

test('lost approval response retries the same binding and logout fences a late confirmation', async ({
  openSession,
}) => {
  const admin = createPairingEmulatorFixture();
  try {
    const worldId = admin.db.collection('worlds').doc().id;
    const { request, url } = requestFor(worldId);
    const { page } = await openSession(url);
    await admitOwner(page, admin.db, worldId);
    await page.route(
      '**/officePairing/approve',
      async (route) => {
        const submitted = await route.fetch();
        expect(submitted.status()).toBe(200);
        await route.abort();
      },
      { times: 1 }
    );
    await page.getByRole('checkbox').check();
    await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('may already have completed');
    const record = admin.db.collection('officePairings').doc(request.pairingId);
    const approved = (await record.get()).data();
    expect(approved?.request).toEqual(request);
    let finish: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      finish = resolve;
    });
    let submitted = false;
    let delivered = false;
    await page.route(
      '**/officePairing/approve',
      async (route) => {
        const response = await route.fetch();
        submitted = true;
        await gate;
        await route.fulfill({ response });
        delivered = true;
      },
      { times: 1 }
    );
    try {
      await page.getByRole('button', { name: 'Retry same approval' }).click();
      await expect.poll(() => submitted).toBe(true);
      await page.getByRole('button', { name: 'Sign out', exact: true }).click();
      await expect(page.getByRole('region', { name: 'Agent pairing request' })).toHaveCount(0);
      finish();
      // The client discards the body after logout; transport delivery, not a
      // fully consumed response body, is the relevant late-completion boundary.
      await expect.poll(() => delivered).toBe(true);
      await expect(page.getByRole('status').filter({ hasText: 'Approved. Return' })).toHaveCount(0);
      expect((await record.get()).data()).toEqual(approved);
    } finally {
      finish();
    }
  } finally {
    await admin.dispose();
  }
});
