import { pageAction } from '../test/page-actions.js';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openChat, openPage, selectInRenderer } from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
async function captures(page: Page, panel: Locator, surface: string, phase: string) {
  const directory = process.env.COLAB_2185_CAPTURE_DIR;
  if (!directory) return;
  mkdirSync(directory, { recursive: true });
  for (const width of [1440, 390])
    for (const theme of ['light', 'dark']) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate(
        (value) => document.documentElement.setAttribute('data-theme', value),
        theme,
      );
      await expect(panel).toBeVisible();
      await page.screenshot({
        path: path.join(directory, `${surface}-${width}-${theme}-${phase}.png`),
      });
    }
}
for (const surface of ['chat', 'thread'])
  test(`${surface}: a pre-adoption network failure can ask only that recipient again on its durable original comment`, async () => {
    await withWorld(async (world) => {
      const door = await startDoor(world, await freePort());
      const alpha = await world.startAgent('again-alpha', { gated: true });
      const beta = await world.startAgent('again-beta', { gated: true });
      const created = createPage(
        world,
        'Ask one recipient again',
        '<p id="quote">One original comment, two recipients.</p>',
        alpha.pane,
      );
      const browser = await pairBrowser(world, 'again-author', { talk: true });
      const page = await openPage(door, browser, created);
      if (surface === 'chat') await openChat(page);
      else {
        await selectInRenderer(page, '#quote');
        await page.getByTestId('selection-ask').click();
      }
      const panel =
        surface === 'chat'
          ? page.getByTestId('chat-panel')
          : page.getByRole('dialog', { name: 'Annotate selection' });
      const input = panel.getByRole('combobox', { name: 'Message', exact: true });
      await expect(input).toBeVisible();
      await expect(panel.locator('.annotation-status-row')).not.toContainText('Checking');
      const body = `@${alpha.name} @${beta.name} Explain the original comment.\n  Keep these bytes.  `;
      await input.fill(body);
      await expect(panel.locator('.annotation-status-row')).toContainText(
        `Asks @${alpha.name}, @${beta.name}.`,
      );
      // Lose one ordinary context GET before adoption; no signed Remote outcome,
      // object append, directory response or dispatch is fabricated.
      let failures = 0;
      await page.route('**/api/session', async (route) => {
        if (route.request().method() === 'GET' && failures === 0) {
          failures++;
          await route.abort('failed');
        } else await route.continue();
      });
      await input.press('Enter');
      await expect(panel.getByTestId('recipient-failure')).toContainText(
        `@${alpha.name} · Not delivered`,
      );
      const again = panel.getByRole('button', { name: 'Ask again', exact: true });
      await expect(again).toBeEnabled();
      await until(() => beta.received().length === 1, 'the accepted sibling delivery');
      expect(alpha.received()).toHaveLength(0);
      expect(failures).toBe(1);
      const comment = panel.getByTestId('comment-entry');
      await expect(comment).toHaveCount(1);
      const messageId = await comment.getAttribute('data-message-id');
      const threadId = await (
        surface === 'chat'
          ? panel.locator('[data-thread-id]').first()
          : panel.getByTestId('comment-thread')
      ).getAttribute('data-thread-id');
      await expect(comment.locator('.comment-body')).toHaveText(body, { useInnerText: true });
      const siblingOperation = await panel
        .getByTestId('ask-entry')
        .getAttribute('data-operation-id');
      await captures(page, panel, surface, 'failure');
      await again.click();
      await until(
        () => alpha.received().length === 1,
        'one fresh delivery to the unsent recipient',
      );
      await expect(again).toHaveCount(0);
      await expect(panel.getByTestId('ask-entry')).toHaveCount(2);
      const operations = await panel
        .getByTestId('ask-entry')
        .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.operationId));
      expect(new Set(operations).size).toBe(2);
      expect(operations).toContain(siblingOperation);
      await expect(comment).toHaveCount(1);
      await expect(comment).toHaveAttribute('data-message-id', messageId!);
      await expect(comment.locator('.comment-body')).toHaveText(body, { useInnerText: true });
      await expect(panel.locator('[data-ledger-state=accepted]')).toHaveCount(2);
      expect(beta.received()).toHaveLength(1);
      expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
        2,
      );
      expect(alpha.received()[0].message).toContain(body);
      const marker = await page.evaluate(async () => {
        const database = await new Promise<IDBDatabase>((resolve, reject) => {
          const request = indexedDB.open('tmt-colab', 1);
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        });
        try {
          return await new Promise<{ key: string; value: string }[]>((resolve, reject) => {
            const transaction = database.transaction('keys', 'readonly');
            const result: { key: string; value: string }[] = [];
            const cursor = transaction.objectStore('keys').openCursor();
            cursor.onsuccess = () => {
              if (!cursor.result) return;
              const key = String(cursor.result.key);
              if (key.startsWith('ask-recipient:'))
                result.push({ key, value: cursor.result.value });
              cursor.result.continue();
            };
            transaction.oncomplete = () => resolve(result);
            transaction.onabort = () => reject(transaction.error);
          });
        } finally {
          database.close();
        }
      });
      expect(marker).toHaveLength(1);
      expect(marker[0].key).toContain(`:${threadId}:${messageId}:`);
      expect(operations).toContain(marker[0].value);
      expect(marker[0].value).not.toBe(siblingOperation);
      await captures(page, panel, surface, 'after');
      if (process.env.COLAB_2185_CAPTURE_DIR)
        writeFileSync(
          path.join(process.env.COLAB_2185_CAPTURE_DIR, `${surface}-proof.json`),
          JSON.stringify(
            {
              pageId: created.pageId,
              threadId,
              messageId,
              marker,
              operations,
              failures,
              alpha: alpha.received(),
              beta: beta.received(),
              dispatches: world.coreCalls().filter((call) => call.operation === 'dispatch.create'),
            },
            null,
            2,
          ),
        );
      await page.reload();
      if (surface === 'chat') await openChat(page);
      else {
        const toggle = await pageAction(page, 'Comments');
        await toggle.click();
        await page.locator(`[data-testid=annotation-row][data-thread-id="${threadId}"]`).click();
      }
      const restored =
        surface === 'chat' ? page.getByTestId('chat-panel') : page.getByTestId('comment-thread');
      await expect(restored.getByTestId('comment-entry')).toHaveCount(1);
      await expect(restored.getByTestId('ask-entry')).toHaveCount(2);
      await expect(restored.getByRole('button', { name: 'Ask again', exact: true })).toHaveCount(0);
      expect(alpha.received()).toHaveLength(1);
      expect(beta.received()).toHaveLength(1);
      expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
        2,
      );
    });
  });
