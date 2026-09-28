import Database from '../../../../../../typescript/test/support/sqlite-oracle.js';
import { expect, test } from '@playwright/test';
import { runCli, withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import { openOfficeObjects } from './office-navigation.js';

function requestCount(databasePath: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare('SELECT count(*) FROM request_attempts').pluck().get();
  } finally {
    database.close();
  }
}

test('a discussion reference reaches the selected inbox without turning posts or copies into dispatch', async ({
  page,
}, testInfo) => {
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
    try {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(started.url);
      await openOfficeObjects(page);
      const openBoard = page.getByRole('button', { name: 'Open discussion board', exact: true });
      await openBoard.click();
      const board = page.getByRole('dialog', { name: 'Discussion board', exact: true });
      await board.getByRole('button', { name: 'New post', exact: true }).click();
      await board.getByLabel('Title', { exact: true }).fill('One request owner');
      await board
        .getByRole('textbox', { name: 'Message', exact: true })
        .fill('Reuse the canonical inbox.');
      await board.getByRole('button', { name: 'Post as owner', exact: true }).click();
      await expect(
        board.getByRole('heading', { name: 'One request owner', exact: true })
      ).toBeVisible();
      expect(requestCount(sandbox.database)).toBe(0);
      await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
      await board.getByRole('button', { name: 'Copy reference', exact: true }).click();
      await expect(board.getByText('Reference copied.')).toBeVisible();
      const reference = await page.evaluate(() => navigator.clipboard.readText());
      expect(reference).toMatch(/^[0-9a-f-]{36}$/);
      expect((await office(['board', 'show', reference])).thread.body).toBe(
        'Reuse the canonical inbox.'
      );
      expect(requestCount(sandbox.database)).toBe(0);
      await board.getByRole('button', { name: 'Ask agents', exact: true }).click();
      await board.getByRole('checkbox', { name: 'Alice · offline', exact: true }).check();
      await board
        .getByRole('textbox', { name: 'Question', exact: true })
        .fill('Who should own retries?');
      // The outer spatial panel retains this same request draft when closed.
      await page.keyboard.press('Escape');
      await openBoard.click();
      await expect(board.getByRole('textbox', { name: 'Question', exact: true })).toHaveValue(
        'Who should own retries?'
      );
      await board.getByRole('button', { name: 'Review request', exact: true }).click();
      const message = await board.getByRole('textbox', { name: 'Exact request' }).inputValue();
      expect(message).toContain(`Discussion thread: ${reference}`);
      expect(message).toContain('live discussion reference, not a frozen snapshot');
      expect(requestCount(sandbox.database)).toBe(0);
      await page.screenshot({ path: testInfo.outputPath('discussion-request-desktop.png') });
      await page.setViewportSize({ width: 390, height: 844 });
      await board.getByRole('button', { name: 'Send request', exact: true }).click();
      await expect(board.getByText(/Request recorded/)).toBeVisible();
      expect(await board.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
        true
      );
      await page.screenshot({ path: testInfo.outputPath('discussion-request-narrow.png') });
      expect(requestCount(sandbox.database)).toBe(1);
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
      expect(incoming.identityId).toBe(alice.id);
      expect(incoming.items).toHaveLength(1);
      const requestId = incoming.items[0].requestId;
      const detail = (await cli(['x', 'show', requestId, '--incoming', '--identity', 'Alice']))
        .exchange;
      expect(detail.prompt.message).toBe(message);
      const receivedReference = detail.prompt.message.match(
        /^Discussion thread: ([0-9a-f-]{36})$/m
      )?.[1];
      expect(receivedReference).toBe(reference);
      const received = await office(['board', 'show', receivedReference!]);
      expect(received.thread).toMatchObject({
        id: reference,
        title: 'One request owner',
        body: 'Reuse the canonical inbox.',
      });
      const response = `Use the canonical request service. Reviewed thread ${receivedReference}.`;
      await cli(['reply', requestId, '--receipt', detail.reply.receipt, '--message', response]);
      expect(await cli(['result', requestId])).toMatchObject({ status: 'completed', response });
      const untouched = await cli([
        'x',
        'listen',
        '--identity',
        'Bob',
        '--timeout',
        '1ms',
        '--debounce',
        '1ms',
      ]);
      expect(untouched.items).toEqual([]);
      await board.getByRole('button', { name: 'Back to discussion', exact: true }).click();
      await expect(board.getByRole('button', { name: 'Ask agents', exact: true })).toBeFocused();
      expect((await office(['board', 'show', reference])).thread.body).toBe(
        'Reuse the canonical inbox.'
      );
    } finally {
      await page.goto('about:blank');
      await office(['stop']);
    }
  });
});
