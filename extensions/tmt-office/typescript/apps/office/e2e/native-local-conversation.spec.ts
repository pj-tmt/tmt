import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openAgentDetails } from './office-navigation.js';
import type { DispatchReceipt } from '../src/local/dispatch-contract.js';

function storedDelivery(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      requests: database
        .prepare(
          'SELECT recipient_identity_id AS recipient, recipient_attention_acknowledged_revision AS acknowledged FROM request_attempts ORDER BY prepared_at_ms, request_id'
        )
        .all(),
      operations: database.prepare('SELECT count(*) FROM office_dispatch_operations').pluck().get(),
    };
  } finally {
    database.close();
  }
}

test('Office conversation reaches a real inbox, renders its reply, and recovers an accepted send after losing the response', async ({
  page,
}, testInfo) => {
  page.setDefaultTimeout(10_000);
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    const cli = async (args: string[]) => {
      const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 30_000 });
      expect(result.status, result.stdout).toBe(0);
      expect(result.stderr).toBe('');
      return JSON.parse(result.stdout);
    };
    const office = (args: string[]) => cli(['office', '--prefix', prefix, ...args]);
    const alice = (await cli(['identity', 'create', 'Alice'])).identity;
    await cli(['identity', 'create', 'Bob']);
    const started = await office(['start', '--port', String(await unusedLoopbackPort())]);
    const renderErrors: string[] = [];
    page.on('pageerror', (error) => renderErrors.push(error.message));
    page.on('dialog', (dialog) => void dialog.accept());
    const openConversation = async () => {
      await openAgentDetails(page, 'Alice');
      await page.getByRole('button', { name: 'Message Alice', exact: true }).click();
      return page.getByRole('dialog', { name: 'Agent conversation', exact: true });
    };
    try {
      await page.setViewportSize({ width: 1536, height: 1024 });
      await page.goto(started.url);
      const panel = await openConversation();
      await expect(panel.getByText('Start a conversation with Alice.')).toBeVisible();
      const message = 'Review the retry boundary.\nKeep the exact request intact.';
      await panel.getByRole('textbox', { name: 'Message', exact: true }).fill(message);
      const input = await panel
        .getByRole('textbox', { name: 'Message', exact: true })
        .elementHandle();
      await page.getByRole('button', { name: 'Refresh office' }).click();
      await expect(page.getByRole('button', { name: 'Refresh office' })).toBeEnabled();
      expect(await input!.evaluate((element) => element.isConnected)).toBe(true);
      await expect(panel.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(
        message
      );
      await panel.getByRole('button', { name: 'Close agent conversation' }).click();
      await expect(page.getByRole('button', { name: 'Office menu' })).toBeFocused();
      await openAgentDetails(page, 'Bob');
      await expect(
        panel.getByRole('region', { name: 'Switch conversation confirmation' })
      ).toBeVisible();
      await panel.getByRole('button', { name: 'Close agent conversation' }).click();
      await openConversation();
      await expect(
        panel.getByRole('region', { name: 'Switch conversation confirmation' })
      ).toHaveCount(0);
      await expect(panel.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(
        message
      );
      expect(storedDelivery(sandbox.database)).toEqual({ requests: [], operations: 0 });
      await expect(panel.getByRole('button', { name: 'Review request', exact: true })).toHaveCount(
        0
      );
      const sending = page.waitForResponse(
        (response) => new URL(response.url()).pathname === '/api/v1/local/dispatch'
      );
      await panel.getByRole('button', { name: 'Send', exact: true }).click();
      const sent = await sending;
      expect(sent.status()).toBe(200);
      const receipt: DispatchReceipt = await sent.json();
      expect(receipt.items).toEqual([
        { recipientId: alice.id, requestId: expect.stringMatching(/^req_/), acceptance: 'queued' },
      ]);
      const requestId = receipt.items[0]!.requestId;
      await expect(panel.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('');
      await expect(panel.locator('.chat-outgoing pre')).toHaveText(message);
      await expect(panel.getByText('Sent', { exact: true })).toBeVisible();
      expect(storedDelivery(sandbox.database)).toEqual({
        requests: [{ recipient: alice.id, acknowledged: 0 }],
        operations: 1,
      });
      const incoming = await cli([
        'x',
        'listen',
        '--identity',
        'Alice',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(incoming.items).toEqual([
        expect.objectContaining({ requestId, direction: 'incoming', delivery: 'queued' }),
      ]);
      const exchange = (await cli(['x', 'show', requestId, '--incoming', '--identity', 'Alice']))
        .exchange;
      expect(exchange.prompt.message).toBe(message);
      await panel.getByRole('button', { name: 'Close agent conversation' }).click();
      await expect(page.getByRole('heading', { name: 'Furniture & devices' })).toBeVisible();
      await expect(panel).toBeHidden();
      const reply = 'The canonical request service owns retries.\nNo second chat store is needed.';
      await cli(['reply', requestId, '--receipt', exchange.reply.receipt, '--message', reply]);
      await openConversation();
      await expect(panel.locator('.conversation-reply')).toHaveText(reply, { timeout: 10_000 });
      await page.screenshot({ path: testInfo.outputPath('conversation-desktop.png') });
      await panel.getByRole('button', { name: 'Close agent conversation' }).click();
      await openConversation();
      await expect(panel.locator('.conversation-reply')).toHaveText(reply);
      await page.setViewportSize({ width: 430, height: 932 });
      await page.screenshot({ path: testInfo.outputPath('conversation-mobile.png') });
      expect(await panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.setViewportSize({ width: 1536, height: 1024 });
      await panel
        .getByRole('textbox', { name: 'Message', exact: true })
        .fill('Check recovery without dispatching twice.');
      // Commit through the real native host, then drop only the browser response.
      // This is a transport fault, not a fabricated acceptance or seeded request.
      await page.route(
        '**/api/v1/local/dispatch',
        async (route) => {
          const response = await route.fetch();
          expect(response.status()).toBe(200);
          await route.abort('failed');
        },
        { times: 1 }
      );
      await panel.getByRole('button', { name: 'Send', exact: true }).click();
      await expect(panel.getByRole('button', { name: 'Retry', exact: true })).toBeEnabled();
      await expect(panel.getByRole('alert')).toBeVisible();
      expect(storedDelivery(sandbox.database).operations).toBe(2);
      // Re-enter through the authenticated URL: credentials are not persisted.
      // Changing only its fragment is same-document navigation, not a reload.
      await page.goto('about:blank');
      await page.goto(started.url);
      await openConversation();
      await expect(panel.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('');
      await expect(panel.locator('.chat-outgoing pre')).toHaveText([
        message,
        'Check recovery without dispatching twice.',
      ]);
      await expect(panel.locator('.conversation-reply')).toHaveText(reply);
      expect(storedDelivery(sandbox.database)).toEqual({
        requests: [
          { recipient: alice.id, acknowledged: 0 },
          { recipient: alice.id, acknowledged: 0 },
        ],
        operations: 2,
      });
      expect(
        await page.evaluate(() =>
          Object.keys(sessionStorage).filter((key) => key.startsWith('tmt.office.pending-request.'))
        )
      ).toEqual([]);
      expect(
        (await cli(['x', 'listen', '--identity', 'Bob', '--timeout', '1ms', '--debounce', '1ms']))
          .items
      ).toEqual([]);
      expect(renderErrors).toEqual([]);
    } finally {
      try {
        await page.goto('about:blank');
      } finally {
        await office(['stop']);
        expect((await office(['status'])).service.running).toBe(false);
      }
    }
  });
});
