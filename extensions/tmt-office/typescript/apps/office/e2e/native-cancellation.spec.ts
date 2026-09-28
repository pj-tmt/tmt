import { expect } from '@playwright/test';
import { test, signIn } from './browser-session.js';
import { setTester } from './firestore-fixture.js';
import { createPairingEmulatorFixture } from '../../../services/office/functions/test/emulator-fixture.js';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import {
  installNativeOffice,
  protectedOfficeRecord,
  officeScopeKeys,
  clearOfficeScopes,
} from './native-office-fixture.js';
import { post } from './pairing-service-fixture.js';

test('expired native request cancels without approval and re-pairs under the same identity', async ({
  openSession,
}) => {
  test.setTimeout(120_000);
  const admin = createPairingEmulatorFixture();
  try {
    await withSandbox(async (sandbox) => {
      try {
        const prefix = await installNativeOffice(sandbox);
        expect(
          (await runCli(sandbox, ['identity', 'create', 'CancelNative', '--json'])).status
        ).toBe(0);
        const worldId = admin.db.collection('worlds').doc().id;
        const world = `http://127.0.0.1:4173/worlds/${worldId}`;
        const office = (operation: string, extra: string[] = []) =>
          runCli(
            sandbox,
            [
              'office',
              operation,
              '--prefix',
              prefix,
              '--world',
              world,
              '--identity',
              'CancelNative',
              '--emulator',
              '--json',
              ...extra,
            ],
            { deadlineMs: 35_000 }
          );
        const pending = await office('pair', ['--timeout', '5']);
        expect(JSON.parse(pending.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
        const link = pending.stderr.trim();
        const keys = officeScopeKeys(sandbox);
        expect(keys).toHaveLength(1);
        const key = keys[0]!;
        const stored = await protectedOfficeRecord(sandbox, key, 'lookup');
        expect(stored.status).toBe(0);
        const record = JSON.parse(stored.stdout);
        const request = record.approval;
        // Move only the bounded local approval clock; no five-minute sleep or
        // fake remote success. The actual CLI must observe expiration.
        record.createdAt = Date.now() - 301_000;
        record.expiresAt = record.createdAt + 300_000;
        record.phase.nextClaimAt = record.createdAt;
        expect(
          (await protectedOfficeRecord(sandbox, key, 'store', JSON.stringify(record))).status
        ).toBe(0);
        expect(JSON.parse((await office('status')).stdout).state).toBe('expired');
        const before = (await protectedOfficeRecord(sandbox, key, 'lookup')).stdout;
        const unknown = await office('unpair');
        expect(unknown.status).toBe(1);
        expect(JSON.parse(unknown.stdout).error.code).toBe('OFFICE_OWNER_CANCELLATION_REQUIRED');
        expect(unknown.stderr.trim()).toBe(link);
        expect((await protectedOfficeRecord(sandbox, key, 'lookup')).stdout).toBe(before);
        expect(
          (await admin.db.collection('officePairings').doc(request.pairingId).get()).exists
        ).toBe(false);

        const { page } = await openSession(link);
        await signIn(page, 'Cancellation owner');
        const ownerUid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
        await admin.db
          .collection('worlds')
          .doc(worldId)
          .set({ version: 1, name: 'Cancellation office', ownerUid, createdAt: new Date() });
        await setTester(ownerUid, true);
        await page.getByRole('button', { name: 'Cancel request', exact: true }).click();
        await expect(
          page.getByRole('status').filter({ hasText: 'Request cancelled.' })
        ).toBeVisible();
        const tombstone = (
          await admin.db.collection('officePairings').doc(request.pairingId).get()
        ).data()!;
        expect(tombstone).toMatchObject({ enabled: false, claimed: false, request });
        expect(
          (await admin.db.collection('worlds').doc(worldId).collection('agentGrants').get()).empty
        ).toBe(true);
        expect((await post('claim', { version: 1, secret: record.phase.secret })).status).toBe(404);
        const cancelled = await office('unpair');
        expect(cancelled.status, cancelled.stdout).toBe(0);
        expect(JSON.parse(cancelled.stdout).state).toBe('revoked');
        const receipt = JSON.parse((await protectedOfficeRecord(sandbox, key, 'lookup')).stdout);
        expect(receipt.phase).toEqual({ state: 'revoked' });
        expect((await office('unpair')).status).toBe(0);

        const fresh = await office('pair', ['--read-only', '--timeout', '5']);
        expect(JSON.parse(fresh.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
        const next = JSON.parse((await protectedOfficeRecord(sandbox, key, 'lookup')).stdout);
        expect(next.approval.identityId).toBe(request.identityId);
        expect(next.approval.pairingId).not.toBe(request.pairingId);
        expect(next.phase.secret).not.toBe(record.phase.secret);
        expect(next.approval.capabilities).toEqual(['layout.read']);
        await page.goto(fresh.stderr.trim());
        await page
          .getByRole('checkbox', { name: 'I recognize this agent and installation.' })
          .check();
        await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
        await expect(
          page.getByRole('status').filter({ hasText: 'Approved. Return' })
        ).toBeVisible();
        const paired = await office('pair', ['--read-only', '--timeout', '20']);
        expect(paired.status, paired.stdout).toBe(0);
        expect(JSON.parse(paired.stdout).identityId).toBe(request.identityId);
        expect((await office('inspect')).status).toBe(0);
        const newRemote = (
          await admin.db.collection('officePairings').doc(next.approval.pairingId).get()
        ).data()!;
        expect(newRemote.enabled).toBe(true);
        const retainedBlock = admin.db
          .collection('worlds')
          .doc(worldId)
          .collection('blocks')
          .doc(newRemote.blockId);
        await retainedBlock.set({
          version: 1,
          revision: 1,
          objects: ['d000'],
          updatedAt: new Date(),
        });
        const retainedBefore = (await retainedBlock.get()).data();
        expect((await office('unpair')).status).toBe(0);
        expect(
          (await admin.db.collection('officePairings').doc(next.approval.pairingId).get()).data()
            ?.enabled
        ).toBe(false);
        expect((await retainedBlock.get()).data()).toEqual(retainedBefore);
        expect(
          (
            await admin.db
              .collection('worlds')
              .doc(worldId)
              .collection('agentGrants')
              .doc(newRemote.principalUid)
              .get()
          ).data()?.enabled
        ).toBe(false);
        const replacement = await office('pair', ['--timeout', '5']);
        expect(JSON.parse(replacement.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
        const replacementRecord = JSON.parse(
          (await protectedOfficeRecord(sandbox, key, 'lookup')).stdout
        );
        expect(replacementRecord.approval.identityId).toBe(request.identityId);
        expect(replacementRecord.approval.capabilities).toEqual(['layout.read', 'layout.write']);
        expect(replacementRecord.approval.pairingId).not.toBe(next.approval.pairingId);
        await page.goto(replacement.stderr.trim());
        await page
          .getByRole('checkbox', { name: 'I recognize this agent and installation.' })
          .check();
        await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
        await expect(
          page.getByRole('status').filter({ hasText: 'Approved. Return' })
        ).toBeVisible();
        expect((await office('pair', ['--timeout', '20'])).status).toBe(0);
        const replacementRemote = (
          await admin.db
            .collection('officePairings')
            .doc(replacementRecord.approval.pairingId)
            .get()
        ).data()!;
        expect(replacementRemote.blockId).not.toBe(newRemote.blockId);
        expect((await retainedBlock.get()).data()).toEqual(retainedBefore);
        expect((await office('unpair')).status).toBe(0);
      } finally {
        await clearOfficeScopes(sandbox);
      }
    });
  } finally {
    await admin.dispose();
  }
});
