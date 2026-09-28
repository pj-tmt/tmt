import { readdirSync } from 'node:fs';
import path from 'node:path';
import { expect } from '@playwright/test';
import { test, signIn } from './browser-session.js';
import { setTester } from './firestore-fixture.js';
import { installNativeOffice, protectedOfficeRecord } from './native-office-fixture.js';
import { createPairingEmulatorFixture } from '../../../services/office/functions/test/emulator-fixture.js';
import { withE2EFixture } from '../../../../../../typescript/test/e2e/harness.js';
import {
  runCli,
  withSandbox,
  type Sandbox,
} from '../../../../../../typescript/test/support/cli-process.js';

// An independent SQLite CLI observes state without invoking product housekeeping.
async function sql(sandbox: Sandbox, statement: string, write = false) {
  const result = await runCli({ ...sandbox, cli: { executable: '/usr/bin/sqlite3', args: [] } }, [
    ...(write ? [] : ['-readonly']),
    '-json',
    sandbox.database,
    statement,
  ]);
  expect(result.status, result.stderr).toBe(0);
  return result.stdout ? JSON.parse(result.stdout) : [];
}

test('retired pending proof stays retryable until a known approval can be cancelled', async ({
  openSession,
}) => {
  test.setTimeout(90_000);
  const admin = createPairingEmulatorFixture();
  try {
    await withSandbox(async (sandbox) => {
      const prefix = await installNativeOffice(sandbox);
      expect((await runCli(sandbox, ['identity', 'create', 'PendingHooks', '--json'])).status).toBe(
        0
      );
      const worldId = admin.db.collection('worlds').doc().id;
      const world = `http://127.0.0.1:4173/worlds/${worldId}`;
      const pending = await runCli(
        sandbox,
        [
          'office',
          'pair',
          '--prefix',
          prefix,
          '--world',
          world,
          '--identity',
          'PendingHooks',
          '--emulator',
          '--timeout',
          '5',
          '--json',
        ],
        { deadlineMs: 15_000 }
      );
      expect(pending.status).toBe(1);
      expect(JSON.parse(pending.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
      const link = pending.stderr.trim();
      expect(link.startsWith(`${world}/pair#tmt-pair=`)).toBe(true);
      const approval = JSON.parse(
        Buffer.from(link.split('#tmt-pair=')[1]!, 'base64url').toString('utf8')
      );
      const scopeFile = readdirSync(path.join(sandbox.globalDir, 'office')).find((name) =>
        /^[0-9a-f]{64}\.lock$/.test(name)
      );
      expect(scopeFile).toBeDefined();
      const key = scopeFile!.slice(0, -'.lock'.length);
      const sync = () =>
        runCli(sandbox, ['office', 'sync', '--prefix', prefix, '--json'], { deadlineMs: 35_000 });
      const pairing = admin.db.collection('officePairings').doc(approval.pairingId);
      try {
        const proof = await protectedOfficeRecord(sandbox, key, 'lookup');
        expect(proof.status).toBe(0);
        expect(JSON.parse(proof.stdout).phase.state).toBe('pending');
        expect((await runCli(sandbox, ['rm', 'PendingHooks', '--force', '--json'])).status).toBe(0);
        const unknown = await sync();
        expect(unknown.status).toBe(1);
        expect(JSON.parse(unknown.stdout)).toEqual({
          completed: 0,
          failed: 1,
          pending: 1,
          failureCode: 'OFFICE_REMOTE_DENIED',
        });
        expect((await pairing.get()).exists).toBe(false);
        expect((await protectedOfficeRecord(sandbox, key, 'lookup')).stdout).toBe(proof.stdout);

        // A browser may still hold the old public link. No native claim/token is
        // needed to reduce this subsequently approved scope using its proof.
        const { page } = await openSession(link);
        await signIn(page, 'PendingHooksOwner');
        const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
        await admin.db
          .collection('worlds')
          .doc(worldId)
          .set({ version: 1, name: 'Pending hook', ownerUid: uid, createdAt: new Date() });
        await setTester(uid, true);
        await page
          .getByRole('checkbox', { name: 'I recognize this agent and installation.' })
          .check();
        await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
        await expect(
          page.getByRole('status').filter({ hasText: 'Approved. Return' })
        ).toBeVisible();
        const approved = (await pairing.get()).data()!;
        expect(approved.enabled).toBe(true);
        expect(approved.claimed).toBe(false);
        const cancelled = await sync();
        expect(cancelled.status, cancelled.stdout).toBe(0);
        expect(JSON.parse(cancelled.stdout)).toEqual({
          completed: 1,
          failed: 0,
          pending: 0,
          failureCode: null,
        });
        expect((await pairing.get()).data()).toEqual({ ...approved, enabled: false });
        const grants = await admin.db
          .collection('worlds')
          .doc(worldId)
          .collection('agentGrants')
          .get();
        expect(grants.empty).toBe(true);
        expect(await sql(sandbox, 'SELECT state, attempt_count FROM identity_hooks')).toEqual([
          { state: 'delivered', attempt_count: 2 },
        ]);
        expect(
          JSON.parse((await protectedOfficeRecord(sandbox, key, 'lookup')).stdout).phase
        ).toEqual({ state: 'revoked' });
      } finally {
        expect((await protectedOfficeRecord(sandbox, key, 'clear')).status).toBe(0);
        expect((await protectedOfficeRecord(sandbox, key, 'lookup')).status).toBe(1);
      }
    });
  } finally {
    await admin.dispose();
  }
});

test('retirement delivers Office hooks and recovers after vault and acknowledgment failures', async ({
  openSession,
}) => {
  test.setTimeout(120_000);
  const admin = createPairingEmulatorFixture();
  try {
    await withSandbox(async (sandbox) => {
      await withE2EFixture(
        async (fixture) => {
          const prefix = await installNativeOffice(sandbox);
          const pane = await fixture.createMockPane('temporary-hooks');
          const added = await fixture.runJsonCli<{ id: string; lifetime: string }>([
            'add',
            pane.pane,
            'TemporaryHooks',
          ]);
          expect(added.code, added.stderr).toBe(0);
          expect(added.json?.lifetime).toBe('temporary');
          const identityId = added.json!.id;
          expect(identityId).toMatch(/^[0-9a-f-]{36}$/);
          const worldId = admin.db.collection('worlds').doc().id;
          const world = `http://127.0.0.1:4173/worlds/${worldId}`;
          const office = (operation: string, extra: string[] = [], env = sandbox.env) =>
            runCli(
              { ...sandbox, env },
              ['office', operation, '--prefix', prefix, '--json', ...extra],
              { deadlineMs: 35_000 }
            );
          const scope = ['--world', world, '--identity', 'TemporaryHooks', '--emulator'];
          const pending = await office('pair', [...scope, '--timeout', '5']);
          expect(pending.status).toBe(1);
          expect(JSON.parse(pending.stdout).error.code).toBe('OFFICE_PAIRING_PENDING');
          const link = pending.stderr.trim();
          expect(link.startsWith(`${world}/pair#tmt-pair=`)).toBe(true);
          const approval = JSON.parse(
            Buffer.from(link.split('#tmt-pair=')[1]!, 'base64url').toString('utf8')
          );
          expect(approval.identityId).toBe(identityId);
          expect(approval.worldId).toBe(worldId);
          const hook = () =>
            sql(
              sandbox,
              `SELECT state, attempt_count FROM identity_hooks WHERE consumer = 'tmt-office' AND identity_id = '${identityId}'`
            );
          expect(await hook()).toEqual([{ state: 'registered', attempt_count: 0 }]);
          const scopeFile = readdirSync(path.join(sandbox.globalDir, 'office')).find((name) =>
            /^[0-9a-f]{64}\.lock$/.test(name)
          );
          expect(scopeFile).toBeDefined();
          const key = scopeFile!.slice(0, -'.lock'.length);
          const vault = (operation: 'lookup' | 'store' | 'clear', input?: string) =>
            protectedOfficeRecord(sandbox, key, operation, input);
          try {
            const { page } = await openSession(link);
            await signIn(page, 'NativeIdentityHooksOwner');
            const uid = (await page.getByText(/^UID: /).innerText()).replace(/^UID: /, '').trim();
            await admin.db
              .collection('worlds')
              .doc(worldId)
              .set({ version: 1, name: 'Identity hooks', ownerUid: uid, createdAt: new Date() });
            await setTester(uid, true);
            await page
              .getByRole('checkbox', { name: 'I recognize this agent and installation.' })
              .check();
            await page.getByRole('button', { name: 'Approve pairing', exact: true }).click();
            await expect(
              page.getByRole('status').filter({ hasText: 'Approved. Return' })
            ).toBeVisible();
            const resumed = await office('pair', [...scope, '--timeout', '25']);
            expect(resumed.status, resumed.stdout).toBe(0);
            expect(JSON.parse(resumed.stdout).state).toBe('credential');
            const pairing = admin.db.collection('officePairings').doc(approval.pairingId);
            const remote = (await pairing.get()).data()!;
            const grant = admin.db
              .collection('worlds')
              .doc(worldId)
              .collection('agentGrants')
              .doc(remote.principalUid);
            const savedGrant = (await grant.get()).data()!;
            const block = admin.db
              .collection('worlds')
              .doc(worldId)
              .collection('blocks')
              .doc(remote.blockId);
            await block.set({ version: 1, revision: 1, objects: ['d000'], updatedAt: new Date() });
            const savedBlock = (await block.get()).data();
            const stored = await vault('lookup');
            expect(stored.status).toBe(0);
            const record = JSON.parse(stored.stdout);
            const token = record.phase.credential.idToken;
            const blockUrl = `http://127.0.0.1:8080/v1/projects/demo-tmt-office/databases/(default)/documents/worlds/${worldId}/blocks/${remote.blockId}`;
            const readBlock = () =>
              fetch(blockUrl, {
                headers: { authorization: `Bearer ${token}` },
                signal: AbortSignal.timeout(10_000),
              });
            expect((await readBlock()).status).toBe(200);
            // Require a real Auth refresh after retirement, solely for reduction.
            record.phase.credential.tokenExpiresAt = 0;
            expect((await vault('store', JSON.stringify(record))).status).toBe(0);

            fixture.tmux(['kill-pane', '-t', pane.pane]);
            expect(
              fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}']).trim().split('\n')
            ).not.toContain(pane.pane);
            const listed = await fixture.runJsonCli<{ identities: Array<{ id: string }> }>(['ls']);
            expect(listed.code, listed.stderr).toBe(0);
            expect(listed.json!.identities.some((identity) => identity.id === identityId)).toBe(
              false
            );
            expect(await hook()).toEqual([{ state: 'pending', attempt_count: 0 }]);
            expect((await grant.get()).data()).toEqual(savedGrant);
            expect((await pairing.get()).data()).toEqual(remote);
            expect((await readBlock()).status).toBe(200);

            const unavailable = await office('sync', [], {
              ...sandbox.env,
              DBUS_SESSION_BUS_ADDRESS: `unix:path=${sandbox.root}/missing-session-bus`,
            });
            expect(unavailable.status).toBe(1);
            expect(JSON.parse(unavailable.stdout)).toEqual({
              completed: 0,
              failed: 1,
              pending: 1,
              failureCode: 'OFFICE_CREDENTIALS_UNAVAILABLE',
            });
            expect(await hook()).toEqual([{ state: 'pending', attempt_count: 1 }]);
            expect((await grant.get()).data()).toEqual(savedGrant);

            // A deleted vault entry is missing evidence, not proof of revocation.
            expect((await vault('clear')).status).toBe(0);
            const missing = await office('sync');
            expect(missing.status).toBe(1);
            expect(JSON.parse(missing.stdout)).toEqual(JSON.parse(unavailable.stdout));
            expect(await hook()).toEqual([{ state: 'pending', attempt_count: 2 }]);
            expect((await grant.get()).data()).toEqual(savedGrant);
            expect((await vault('store', JSON.stringify(record))).status).toBe(0);

            await sql(
              sandbox,
              "CREATE TRIGGER reject_office_hook_ack BEFORE UPDATE OF state ON identity_hooks WHEN NEW.state = 'delivered' BEGIN SELECT RAISE(ABORT, 'fixture acknowledgment failure'); END",
              true
            );
            const unacknowledged = await office('sync');
            expect(unacknowledged.status).toBe(1);
            expect(JSON.parse(unacknowledged.stdout)).toEqual({
              completed: 0,
              failed: 1,
              pending: 1,
              failureCode: 'OFFICE_CREDENTIALS_UNAVAILABLE',
            });
            expect(await hook()).toEqual([{ state: 'pending', attempt_count: 3 }]);
            expect((await grant.get()).data()).toEqual({ ...savedGrant, enabled: false });
            expect((await pairing.get()).data()).toEqual({ ...remote, enabled: false });
            expect((await block.get()).data()).toEqual(savedBlock);
            const tombstone = await vault('lookup');
            expect(tombstone.status).toBe(0);
            expect(JSON.parse(tombstone.stdout).phase).toEqual({ state: 'revoked' });
            expect((await readBlock()).status).toBe(403);

            await sql(sandbox, 'DROP TRIGGER reject_office_hook_ack', true);
            const retried = await office('sync');
            expect(retried.status, retried.stdout).toBe(0);
            expect(JSON.parse(retried.stdout)).toEqual({
              completed: 1,
              failed: 0,
              pending: 0,
              failureCode: null,
            });
            expect(await hook()).toEqual([{ state: 'delivered', attempt_count: 4 }]);
            expect((await vault('lookup')).stdout).toBe(tombstone.stdout);
            const repeated = await office('sync');
            expect(repeated.status).toBe(0);
            expect(JSON.parse(repeated.stdout)).toEqual({
              completed: 0,
              failed: 0,
              pending: 0,
              failureCode: null,
            });
            expect(await hook()).toEqual([{ state: 'delivered', attempt_count: 4 }]);
            expect((await grant.get()).data()).toEqual({ ...savedGrant, enabled: false });
            expect((await block.get()).data()).toEqual(savedBlock);
            await page.getByRole('link', { name: 'Office', exact: true }).click();
            await page.getByLabel('World ID', { exact: true }).fill(worldId);
            await page.getByRole('button', { name: 'Open world', exact: true }).click();
            await expect(page.getByText(/^Revoked · Lease expires/)).toBeVisible();
            await page
              .getByRole('button', { name: `Open block ${remote.blockId}`, exact: true })
              .click();
            await expect(page.getByRole('button', { name: 'Desk 1', exact: true })).toBeVisible();
            expect((await block.get()).data()).toEqual(savedBlock);
          } finally {
            expect((await vault('clear')).status).toBe(0);
            expect((await vault('lookup')).status).toBe(1);
          }
        },
        { globalDir: sandbox.globalDir }
      );
    });
  } finally {
    await admin.dispose();
  }
});
