import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { withE2EFixture } from '../../../../../../typescript/test/e2e/harness.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';
import type { ProfileProjection } from '../src/profiles/profile-contract.js';
import type { IdentityStatusSnapshot } from '../src/identities/identity-status.js';
import type { DispatchReceipt } from '../src/local/dispatch-contract.js';

function stored(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      statuses: database.prepare('SELECT * FROM identity_status ORDER BY identity_id').all(),
      profiles: database.prepare('SELECT * FROM office_local_profiles ORDER BY identity_id').all(),
      worlds: database.prepare('SELECT * FROM office_local_worlds').all(),
      requests: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
    };
  } finally {
    database.close();
  }
}

test('canonical CLI status drives live actor cues and Info, expires without a read, and yields to a real reply', async ({
  page,
}, info) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    await withE2EFixture(
      async (fixture) => {
        async function cli<T>(args: string[]): Promise<T> {
          const result = await fixture.runJsonCli<T>(args);
          expect(result.code, result.stdout + result.stderr).toBe(0);
          expect(result.json).toBeDefined();
          return result.json!;
        }
        const office = <T>(args: string[]) => cli<T>(['office', '--prefix', prefix, ...args]);
        const alicePane = await fixture.createMockPane('alice');
        const pipPane = await fixture.createMockPane('pip');
        const alice = await cli<{ id: string }>(['add', alicePane.pane, 'Alice', '-s']);
        await cli(['add', pipPane.pane, 'Pip']);
        const report = await cli<{ identityId: string; status: IdentityStatusSnapshot }>([
          'identity',
          'status',
          'set',
          'Reviewing the map',
          '--mood',
          'focused',
          '--for',
          '60m',
          '--identity',
          'Alice',
        ]);
        await cli([
          'identity',
          'status',
          'set',
          'Testing',
          '--mood',
          'curious',
          '--for',
          '90m',
          '--identity',
          'Pip',
        ]);
        expect(report.identityId).toBe(alice.id);
        const before = stored(sandbox.database);
        expect(before.statuses).toHaveLength(2);
        expect(before.requests).toBe(0);
        const launch = () =>
          unusedLoopbackPort().then((port) =>
            office<{ url: string }>(['start', '--port', String(port)])
          );
        const started = await launch();
        const renderErrors: string[] = [];
        page.on('pageerror', (error) => renderErrors.push(error.message));
        let directoryReads = 0;
        page.on('request', (request) => {
          if (new URL(request.url()).pathname === '/api/v1/local/profiles') directoryReads++;
        });
        try {
          await page.setViewportSize({ width: 1440, height: 1000 });
          await page.clock.install({ time: new Date() });
          const response = page.waitForResponse(
            (response) => new URL(response.url()).pathname === '/api/v1/local/profiles'
          );
          await page.goto(started.url);
          const profiles: ProfileProjection[] = await (await response).json();
          expect(profiles.find((profile) => profile.identityId === alice.id)).toMatchObject({
            presence: 'active' as const,
            selfReportedStatus: report.status,
          });
          await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
          await page.screenshot({ path: info.outputPath('status-bubbles-desktop.png') });
          const fresh = await page.locator('.office-canvas').screenshot();
          const readsBeforeExpiry = directoryReads;
          // Advance only the browser clock. The host record remains untouched;
          // expiry must be driven by the mounted directory's deadline, not a GET.
          await page.clock.fastForward(3_600_001);
          await expect(page.locator('.office-map')).toHaveAttribute('data-scene-ready', 'true');
          const expired = await page.locator('.office-canvas').screenshot();
          expect(expired.equals(fresh)).toBe(false);
          expect(directoryReads).toBe(readsBeforeExpiry);
          expect(stored(sandbox.database)).toEqual(before);
          await openAgentDetails(page, 'Alice');
          const status = page.getByRole('region', { name: 'Self-reported status' });
          await expect(status.getByText('Stale', { exact: true })).toBeVisible();
          await expect(status.getByText('Reviewing the map', { exact: true })).toBeVisible();
          await expect(status.locator('time')).toHaveCount(2);
          await expect(page.getByText('Saved identity · Online', { exact: true })).toBeVisible();
          await page.screenshot({ path: info.outputPath('status-expired-info.png') });
          await openAgentDetails(page, 'Pip');
          await expect(status.getByText('Current', { exact: true })).toBeVisible();
          await expect(page.getByText('Contractor · Online', { exact: true })).toBeVisible();
          await page.getByRole('button', { name: 'Close agent conversation' }).click();
          await page.clock.setSystemTime(new Date());
          const renewed = await cli<{ status: IdentityStatusSnapshot }>([
            'identity',
            'status',
            'set',
            'Ready to review',
            '--mood',
            'calm',
            '--identity',
            'Alice',
          ]);
          await page.getByRole('button', { name: 'Refresh office' }).click();
          await expect(page.getByRole('button', { name: 'Refresh office' })).toBeEnabled();
          await openAgentDetails(page, 'Alice');
          await expect(status.getByText('Ready to review', { exact: true })).toBeVisible();
          await expect(status.getByText('Current', { exact: true })).toBeVisible();
          await page.setViewportSize({ width: 430, height: 932 });
          await page.screenshot({ path: info.outputPath('status-info-narrow.png') });
          await page.setViewportSize({ width: 1440, height: 1000 });
          await page.getByRole('button', { name: 'Message Alice', exact: true }).click();
          const chat = page.getByRole('dialog', { name: 'Agent conversation', exact: true });
          await chat
            .getByRole('textbox', { name: 'Message', exact: true })
            .fill('Review the current layout.');
          const sent = page.waitForResponse(
            (response) => new URL(response.url()).pathname === '/api/v1/local/dispatch'
          );
          await chat.getByRole('button', { name: 'Send', exact: true }).click();
          const receipt: DispatchReceipt = await (await sent).json();
          expect(receipt.items).toHaveLength(1);
          expect(receipt.items[0]!.recipientId).toBe(alice.id);
          const requestId = receipt.items[0]!.requestId!;
          const incoming = await cli<{
            exchange: { prompt: { message: string }; reply: { receipt: string } };
          }>(['x', 'show', requestId, '--incoming', '--identity', 'Alice']);
          expect(incoming.exchange.prompt.message).toBe('Review the current layout.');
          await chat.getByRole('button', { name: 'Close agent conversation' }).click();
          await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
          await cli([
            'reply',
            requestId,
            '--receipt',
            incoming.exchange.reply.receipt,
            '--message',
            'The layout is ready.',
          ]);
          await openAgentDetails(page, 'Alice');
          await page.getByRole('button', { name: 'Message Alice', exact: true }).click();
          await expect(chat.locator('.conversation-reply')).toHaveText('The layout is ready.');
          const inspector = page.getByRole('complementary', { name: 'Agent inspector' });
          const box = (await inspector.boundingBox())!;
          expect(box.x + box.width).toBeLessThanOrEqual(1440);
          expect(box.y + box.height).toBeLessThan(1000);
          await page.screenshot({ path: info.outputPath('status-reply-priority.png') });
          expect(
            (
              await cli<{ status: IdentityStatusSnapshot }>([
                'identity',
                'status',
                'show',
                '--identity',
                'Alice',
              ])
            ).status
          ).toEqual(renewed.status);
          expect(stored(sandbox.database).requests).toBe(1);
          await expect(chat.locator('.conversation-reply')).toHaveText('The layout is ready.');
          expect(renderErrors).toEqual([]);
          await office(['stop']);
          const restarted = await launch();
          await page.goto('about:blank');
          await page.goto(restarted.url);
          await openAgentDetails(page, 'Alice');
          await expect(status.getByText('Ready to review', { exact: true })).toBeVisible();
          expect(stored(sandbox.database).statuses).toHaveLength(2);
          // Core-valid format characters must not make the complete directory fail
          // decoding because JavaScript trim has a different whitespace definition.
          await cli(['identity', 'status', 'set', '\uFEFF', '--identity', 'Alice']);
          await page.getByRole('button', { name: 'Refresh office' }).click();
          await expect(page.getByRole('button', { name: 'Refresh office' })).toBeEnabled();
          await expect(status.getByText('Current', { exact: true })).toBeVisible();
          // Wait for the refreshed projection without whitespace-normalizing FEFF.
          await expect(status.locator('p').first()).toHaveJSProperty('textContent', '\uFEFF');
          // A changed server marker is inconclusive while the recorded process
          // remains alive. Preserve native unknown evidence across HTTP and HUD.
          const database = new Database(sandbox.database);
          try {
            database
              .prepare('UPDATE bindings SET server_id = ? WHERE identity_id = ?')
              .run('11111111-1111-4111-8111-111111111111', alice.id);
          } finally {
            database.close();
          }
          const beforeUnknown = stored(sandbox.database);
          const unknownResponse = page.waitForResponse(
            (response) => new URL(response.url()).pathname === '/api/v1/local/profiles'
          );
          await page.getByRole('button', { name: 'Refresh office' }).click();
          const unknownProfiles: ProfileProjection[] = await (await unknownResponse).json();
          expect(unknownProfiles.find((profile) => profile.identityId === alice.id)?.presence).toBe(
            'unknown'
          );
          await expect(
            page.getByText('Saved identity · Presence unknown', { exact: true })
          ).toBeVisible();
          await page.getByRole('button', { name: 'Close agent conversation' }).click();
          await openAgentDetails(page, 'Alice');
          await expect(status.locator('p').first()).toHaveJSProperty('textContent', '\uFEFF');
          expect(stored(sandbox.database)).toEqual(beforeUnknown);
        } finally {
          await office(['stop']);
          expect(
            (await office<{ service: { running: boolean } }>(['status'])).service.running
          ).toBe(false);
        }
      },
      { globalDir: sandbox.globalDir }
    );
  });
});
