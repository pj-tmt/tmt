import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}

/** Opens the long thread (40 messages) inside the Comments panel; returns its drawer scroller. */
async function openLongThread(page: Page, width: number, theme: 'light' | 'dark') {
  await page.setViewportSize({ width, height: 700 });
  await page.emulateMedia({ colorScheme: theme });
  await page.addInitScript((value) => {
    document.documentElement.dataset.theme = value;
  }, theme);
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'conversation', { surface: 'thread', state: 'replied' });
  await run(page, 'scrollHistory');
  const app = page.locator('#ask-page-fixture');
  await (await pageAction(app, 'Comments')).click();
  const panel = app.getByTestId('comments-panel');
  await panel.locator('.annotation-row').first().click();
  const entries = panel.getByTestId('comment-entry');
  await expect(entries).toHaveCount(40);
  const scroller = panel.locator('xpath=ancestor::*[contains(@class,"tmt-ui-drawer-body")]');
  return { panel, entries, scroller };
}

/** Every ancestor from a message up to the page that can take a vertical scroll of its own. */
const scrollContainers = (message: Locator) =>
  message.evaluate((element) => {
    const found: string[] = [];
    for (let node = element.parentElement; node; node = node.parentElement) {
      const overflow = getComputedStyle(node).overflowY;
      if (overflow === 'auto' || overflow === 'scroll' || overflow === 'overlay')
        found.push(node.className.split(' ')[0] || node.tagName.toLowerCase());
    }
    return found;
  });

/** The last message is fully inside the scroller's visible box. */
const visibleIn = (message: Locator, scroller: Locator) =>
  Promise.all([message.boundingBox(), scroller.boundingBox()]).then(
    ([item, box]) => !!item && !!box && item.y >= box.y && item.y + item.height <= box.y + box.height,
  );

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the Comments panel scrolls in one container: ${width}px ${theme}`, async ({ page }) => {
      const { panel, entries, scroller } = await openLongThread(page, width, theme);

      // One scroller between a message and the page: the drawer body, never the message list.
      expect(await scrollContainers(entries.first())).toEqual(['tmt-ui-drawer-body']);
      // Nor is the composer its own scroller: only the drawer body sits above its fields.
      expect(await scrollContainers(panel.getByTestId('annotation-compose'))).toEqual([
        'tmt-ui-drawer-body',
      ]);
      expect(
        await panel
          .locator('.conversation-composer')
          .evaluate((node) => getComputedStyle(node).overflowY),
      ).toBe('visible');

      // The wheel over a message reaches the last message.
      await scroller.evaluate((node) => {
        node.scrollTop = 0;
      });
      const last = entries.last();
      await expect(entries.first()).toBeVisible();
      const box = (await scroller.boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      for (let step = 0; step < 60 && !(await visibleIn(last, scroller)); step++) {
        await page.mouse.wheel(0, 600);
        await page.waitForTimeout(50);
      }
      expect(await visibleIn(last, scroller)).toBe(true);

      // Keyboard scrolling reaches it too: focus a message, then Home and End.
      await entries.first().click();
      await page.keyboard.press('Home');
      await expect.poll(() => scroller.evaluate((node) => node.scrollTop)).toBe(0);
      await page.keyboard.press('End');
      await expect.poll(() => visibleIn(last, scroller)).toBe(true);

      await page.screenshot({ path: await capturePath(`comments-scroll-${width}-${theme}.png`) });
    });

test('a message arriving in the Comments panel stays reachable and does not move the reader', async ({
  page,
}) => {
  const { entries, scroller } = await openLongThread(page, 1440, 'light');
  await entries.first().scrollIntoViewIfNeeded();
  await scroller.evaluate((node) => {
    node.scrollTop = 0;
  });
  await run(page, 'scrollArrival');
  await expect(entries).toHaveCount(41);
  expect(await scroller.evaluate((node) => node.scrollTop)).toBe(0);
  await entries.first().click();
  await page.keyboard.press('End');
  await expect.poll(() => visibleIn(entries.last(), scroller)).toBe(true);
  await expect(entries.last()).toContainText('A new turn from another browser.');
});
