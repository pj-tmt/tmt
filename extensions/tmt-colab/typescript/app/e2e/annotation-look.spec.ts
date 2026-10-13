import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { captureDirectory } from './captures.js';
import { destination } from '../test/ask-fixtures.js';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const captures = captureDirectory();
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`annotation design matrix ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await run(page, 'mount', {
        agents: [{ ...destination(), agentName: 'Atlas', presence: 'active' }],
      });
      await run(page, 'annotationLook');
      const app = page.locator('#ask-page-fixture');
      const frame = page.frameLocator('#ask-page-fixture iframe');
      const markers = frame.locator('[data-colab-thread]');
      await expect(markers).toHaveCount(4);
      await expect(markers).toHaveText(['1', '2', '3', '4']);
      for (const marker of await markers.all()) {
        await expect(marker).toHaveCSS('border-top-width', '0px');
        await expect(marker).toHaveCSS('box-shadow', 'none');
        await expect(marker).toHaveCSS('color', 'rgb(255, 255, 255)');
      }
      const authored = await frame.locator('body').evaluate((body) => {
        const clone = body.cloneNode(true) as HTMLElement;
        clone.querySelectorAll('[data-colab-highlight]').forEach((node) => node.remove());
        return clone.innerHTML;
      });
      mkdirSync(captures, { recursive: true });
      await page.screenshot({ path: `${captures}/annotation-ranges-${width}-${theme}.png` });
      const range = frame.locator('#selected');
      await range.evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
      });
      const annotate = page.getByTestId('selection-ask');
      await expect(annotate).toHaveText('Annotate');
      await expect(annotate).toHaveAttribute('aria-keyshortcuts', 'c Alt+Enter');
      await expect(annotate.locator('svg')).toBeVisible();
      await expect(annotate.locator('kbd')).toHaveCount(0);
      await page.screenshot({ path: `${captures}/annotation-affordance-${width}-${theme}.png` });
      await annotate.click();
      const window = page.getByTestId('annotation-window');
      await expect(window).toBeVisible();
      const editor = window.getByRole('combobox', { name: 'Message', exact: true });
      await editor.fill('Could @Atlas check this at 390px?');
      await page.screenshot({ path: `${captures}/annotation-new-${width}-${theme}.png` });
      if (width === 390) {
        const box = (await page.locator('.annotation-popover').boundingBox())!;
        expect(box.y + box.height).toBeGreaterThan(800);
        await expect(page.locator('.annotation-popover')).not.toHaveAttribute('data-leader');
      }
      await page.keyboard.press('Escape');
      await markers.filter({ hasText: /^1$/ }).click();
      const thread = page.getByTestId('comment-thread');
      await expect(thread).toBeVisible();
      await expect(thread.locator('.annotation-number')).toHaveText('1');
      const human = thread.locator('.conversation-turn[data-turn-role="user"]');
      const agent = thread.locator('.conversation-turn[data-turn-role="agent"]');
      await expect(human.locator('.lucide-user-round')).toBeVisible();
      await expect(agent.locator('.lucide-bot')).toBeVisible();
      await expect(agent.locator('.conversation-agent-label')).toContainText('AGENT');
      await expect(agent).toHaveCSS('border-left-width', '0px');
      await expect(human).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
      await expect(thread.locator('.message-mention[data-kind="agent"]')).toHaveCount(2);
      await page.screenshot({ path: `${captures}/annotation-thread-${width}-${theme}.png` });
      await page.keyboard.press('Escape');
      const comments = await pageAction(app, 'Comments');
      await comments.click();
      const panel = page.getByTestId('comments-panel');
      await expect(panel.locator('.annotation-row .annotation-number')).toHaveText([
        '1',
        '2',
        '3',
        '4',
        '5',
      ]);
      await panel.locator('.annotation-row').filter({ hasText: 'old decision text' }).click();
      const resolved = panel.getByTestId('comment-thread');
      await expect(
        resolved.getByRole('button', { name: 'Show conversation', exact: true }),
      ).toBeVisible();
      await expect(resolved.locator('.conversation-history')).toBeHidden();
      await expect(resolved.locator('.resolved-summary')).toContainText(
        '1 messages · 0 attachments',
      );
      await resolved.scrollIntoViewIfNeeded();
      const rejectedDrop = await resolved.evaluate((element) => {
        const dataTransfer = new DataTransfer();
        dataTransfer.items.add(new File(['draft'], 'hidden.txt', { type: 'text/plain' }));
        const event = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer });
        element.dispatchEvent(event);
        return event.defaultPrevented;
      });
      expect(rejectedDrop).toBe(true);
      await page.screenshot({ path: `${captures}/annotation-resolved-${width}-${theme}.png` });
      await resolved.getByRole('button', { name: 'Show conversation', exact: true }).click();
      await expect(resolved.locator('.conversation-history')).toBeVisible();
      await expect(resolved.locator('.attachment-pending')).toHaveCount(0);
      await resolved.scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${captures}/annotation-resolved-expanded-${width}-${theme}.png`,
      });
      await run(page, 'annotationLookDelete');
      await expect(panel.locator('.annotation-row .annotation-number')).toHaveText([
        '1',
        '2',
        '3',
        '4',
      ]);
      await expect(markers).toHaveText(['1', '2', '3']);
      expect(
        await frame.locator('body').evaluate((body) => {
          const clone = body.cloneNode(true) as HTMLElement;
          clone.querySelectorAll('[data-colab-highlight]').forEach((node) => node.remove());
          return clone.innerHTML;
        }),
      ).toBe(authored);
      expect(await run(page, 'proof')).toMatchObject({ sends: [], actions: [] });
    });
  }
