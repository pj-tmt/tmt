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
async function openLongThread(page: Page, width: number, theme: 'light' | 'dark', height = 700) {
  await page.setViewportSize({ width, height });
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
  // Opening a thread shows its latest message, the way it did when the composer took focus.
  await expect.poll(() => visibleIn(entries.last())).toBe(true);
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

/** The message is fully inside the scroller's visible box and clear of the pinned composer. */
const visibleIn = (message: Locator) =>
  message.evaluate((node) => {
    const body = node.closest('.tmt-ui-drawer-body')!.getBoundingClientRect();
    const bar = node
      .closest('.comments-panel')
      ?.querySelector('.conversation-composer')
      ?.getBoundingClientRect();
    const item = node.getBoundingClientRect();
    const floor = bar && bar.top < body.bottom ? bar.top : body.bottom;
    return item.top >= body.top && item.bottom <= floor + 1;
  });

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
      for (let step = 0; step < 60 && !(await visibleIn(last)); step++) {
        await page.mouse.wheel(0, 600);
        await page.waitForTimeout(50);
      }
      expect(await visibleIn(last)).toBe(true);

      // Keyboard scrolling reaches it too: focus a message, then Home and End.
      await entries.first().click();
      await page.keyboard.press('Home');
      await expect.poll(() => scroller.evaluate((node) => node.scrollTop)).toBe(0);
      await page.keyboard.press('End');
      await expect.poll(() => visibleIn(last)).toBe(true);

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
  await expect.poll(() => visibleIn(entries.last())).toBe(true);
  await expect(entries.last()).toContainText('A new turn from another browser.');
});

const lineHeight = (node: Locator) =>
  node.evaluate((element) => Number.parseFloat(getComputedStyle(element).lineHeight));
/** The composer's bottom edge sits on the scroller's bottom edge (within a pixel), or, at the end
 * of the scroll, on the top of the Resolve footer that follows it. */
const pinned = async (composer: Locator, scroller: Locator) => {
  const footer = scroller.locator('.discussion-footer');
  const [bar, box, below] = await Promise.all([
    composer.boundingBox(),
    scroller.boundingBox(),
    footer.boundingBox(),
  ]);
  if (!bar || !box) return false;
  const edge = Math.abs(bar.y + bar.height - (box.y + box.height)) <= 1;
  const onFooter = !!below && Math.abs(bar.y + bar.height - below.y) <= 1;
  return edge || onFooter;
};

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the Comments composer stays pinned and compact: ${width}px ${theme}`, async ({
      page,
    }) => {
      const { panel, entries, scroller } = await openLongThread(page, width, theme);
      const composer = panel.locator('.conversation-composer');
      const field = composer.locator('.rich-input-field');

      // Pinned at the top, in the middle and at the end of the scroll, and never a scroller itself.
      for (const top of [0, 700, 100000]) {
        await scroller.evaluate((node, value) => {
          node.scrollTop = value;
        }, top);
        await expect.poll(() => pinned(composer, scroller)).toBe(true);
      }
      await scroller.evaluate((node) => {
        node.scrollTop = 0;
      });
      expect(await scrollContainers(composer.getByTestId('annotation-compose'))).toEqual([
        'tmt-ui-drawer-body',
      ]);

      // At rest the reply field shows four lines (the website's) and the note shares one row with Reply.
      const line = await lineHeight(field);
      const rest = (await field.boundingBox())!.height;
      expect(Math.abs(rest - 4 * line)).toBeLessThanOrEqual(1);
      const note = composer.locator('.annotation-anchor-note');
      const hint = composer.locator('.annotation-hint');
      await expect(note).toHaveText('Attached to this sentence');
      const [noteBox, hintBox] = await Promise.all([note.boundingBox(), hint.boundingBox()]);
      expect(Math.abs(noteBox!.y - hintBox!.y)).toBeLessThanOrEqual(1);
      await expect(hint).toHaveText('Posts as a comment.');

      await page.screenshot({
        path: await capturePath(`comments-composer-rest-${width}-${theme}.png`),
      });

      // It grows while typing, up to its existing max, and stays pinned.
      await field.click();
      for (let index = 0; index < 12; index++) {
        await page.keyboard.type(`Line ${index + 1}`);
        await page.keyboard.press('Shift+Enter');
      }
      const grown = (await field.boundingBox())!.height;
      const max = await field.evaluate((node) =>
        Number.parseFloat(getComputedStyle(node).maxHeight),
      );
      expect(grown).toBeGreaterThan(rest + line);
      expect(grown).toBeLessThanOrEqual(max + 1);
      await expect.poll(() => pinned(composer, scroller)).toBe(true);
      await expect(entries.first()).toBeAttached();
      await page.screenshot({ path: await capturePath(`comments-composer-${width}-${theme}.png`) });
    });

test('the on-screen keyboard shrinks the Comments message area, not the composer', async ({
  page,
}) => {
  const { panel, scroller } = await openLongThread(page, 390, 'light');
  const composer = panel.locator('.conversation-composer');
  const field = composer.locator('.rich-input-field');
  const resting = (await composer.boundingBox())!.height;
  // A short viewport stands in for the on-screen keyboard: the visible area shrinks.
  await page.setViewportSize({ width: 390, height: 420 });
  await field.click();
  await expect(field).toBeFocused();
  await expect.poll(() => pinned(composer, scroller)).toBe(true);
  const [bar, box] = await Promise.all([composer.boundingBox(), scroller.boundingBox()]);
  expect(Math.abs(bar!.height - resting)).toBeLessThanOrEqual(1);
  // The composer is fully on screen and messages keep room above it.
  expect(box!.height - bar!.height).toBeGreaterThan(100);
  // The last message stays reachable above the pinned composer.
  await scroller.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });
  await expect.poll(() => visibleIn(panel.getByTestId('comment-entry').last())).toBe(true);
  await page.screenshot({ path: await capturePath('comments-composer-390-focused-short.png') });
});
