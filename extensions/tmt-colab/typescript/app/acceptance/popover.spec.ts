import { expect, test, type Page } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, selectInRenderer } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

/** A real touch: CDP dispatches touch events, which the browser turns into touch pointer events. */
async function tap(page: Page, target: ReturnType<Page['locator']>) {
  // Wait for the control to stop moving: its position follows the selection asynchronously.
  let box = (await target.boundingBox())!;
  for (let settled = 0; settled < 3;) {
    await page.waitForTimeout(60);
    const next = (await target.boundingBox())!;
    settled = next.x === box.x && next.y === box.y ? settled + 1 : 0;
    box = next;
  }
  const point = { x: box.x + box.width / 2, y: box.y + box.height / 2, id: 1 };
  const client = await page.context().newCDPSession(page);
  await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [point] });
  await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await client.detach();
}
const clearSelection = (page: Page) =>
  page
    .frameLocator('iframe')
    .locator('body')
    .evaluate((node) => node.ownerDocument.getSelection()?.removeAllRanges());

test.afterEach(disposeActiveWorlds);
test('the annotation popover closes by its × , Escape anywhere in it, an outside press or a cleared selection, and keeps a typed draft', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('popover-agent');
    const browser = await pairBrowser(world, 'popover-reader');
    const html =
      '<style>body{margin:0;padding:24px;font:16px/1.6 sans-serif}</style><h1>Popover page</h1><p id="quote">A quote to annotate.</p><p id="other">Another sentence.</p>';
    const page = await openPage(door, browser, createPage(world, 'Popover page', html, agent.pane));
    const popover = page.getByRole('dialog', { name: 'Annotate selection' });
    const close = popover.getByRole('button', { name: 'Close annotation', exact: true });
    const input = popover.getByRole('combobox', { name: 'Message to agent' });
    const open = async (touch: boolean) => {
      await selectInRenderer(page, '#quote');
      const bubble = page.getByTestId('selection-ask');
      if (touch) await tap(page, bubble);
      else await bubble.click();
      await expect(popover).toBeVisible();
      await expect(input).toHaveValue(`@${agent.name} `);
    };
    for (const width of [1440, 390]) {
      const touch = width === 390;
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        // The × closes, returns focus to the page frame and the selection (and its Annotate
        // control) is still there; clearing the selection then removes the control.
        await open(touch);
        await page.screenshot({ path: `/tmp/1713-native-${width}-${theme}-popover.png` });
        if (touch) await tap(page, close);
        else await close.click();
        await expect(popover).toHaveCount(0);
        await expect(page.locator('iframe')).toBeFocused();
        await expect(page.getByTestId('selection-ask')).toBeVisible();
        await clearSelection(page);
        await expect(page.getByTestId('selection-ask')).toBeHidden();

        // Escape from a control that is not the textarea.
        await open(touch);
        await close.focus();
        await page.keyboard.press('Escape');
        await expect(popover).toHaveCount(0);
        await expect(page.locator('iframe')).toBeFocused();

        // A press outside the popover.
        await open(touch);
        const bar = page.locator('.tmt-ui-header');
        if (touch) await tap(page, bar);
        else await bar.click({ position: { x: 4, y: 4 } });
        await expect(popover).toHaveCount(0);

        // A cleared selection closes an untouched prefill.
        await open(touch);
        await clearSelection(page);
        await expect(popover).toHaveCount(0);
        await expect(page.getByTestId('selection-ask')).toBeHidden();
      }
    }
    // A typed draft is never lost: a page click that clears the selection leaves it open, every
    // close path keeps it, and the same selection brings it back with a quiet note.
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(false);
    await input.fill(`@${agent.name} Keep this draft.`);
    await clearSelection(page);
    await expect(popover).toBeVisible();
    await expect(input).toHaveValue(`@${agent.name} Keep this draft.`);
    await input.press('Escape');
    await expect(popover).toHaveCount(0);
    await selectInRenderer(page, '#quote');
    await page.getByTestId('selection-ask').click();
    await expect(popover).toBeVisible();
    await expect(input).toHaveValue(`@${agent.name} Keep this draft.`);
    await expect(popover.getByText('Draft kept', { exact: true })).toBeVisible();
    await page.screenshot({ path: '/tmp/1713-native-1440-light-draft-kept.png' });
    await page.locator('.tmt-ui-header').click({ position: { x: 4, y: 4 } });
    await expect(popover).toHaveCount(0);
    // A different selection does not inherit it.
    await selectInRenderer(page, '#other');
    await page.getByTestId('selection-ask').click();
    await expect(input).toHaveValue(`@${agent.name} `);
    await expect(popover.getByText('Draft kept', { exact: true })).toHaveCount(0);
    await close.click();
    expect(agent.received()).toHaveLength(0);

    // A send in flight is not interrupted by the ×, Escape or an outside press.
    await selectInRenderer(page, '#quote');
    await page.getByTestId('selection-ask').click();
    await expect(popover.getByText('Draft kept', { exact: true })).toBeVisible();
    // Agents are loaded once the list offers them; only then hold the send.
    await input.fill('@');
    await page
      .getByRole('option')
      .filter({ hasText: `@${agent.name} ·` })
      .click();
    let release = () => {};
    const held = new Promise<void>((resolve) => (release = resolve));
    await page.route('**/append', async (route) => {
      await held;
      await route.continue();
    });
    await input.fill(`@${agent.name} Hold this send.`);
    await input.press('Enter');
    await expect(input).toBeDisabled();
    await close.click();
    await close.focus();
    await page.keyboard.press('Escape');
    await page.locator('.tmt-ui-header').click({ position: { x: 4, y: 4 } });
    await clearSelection(page);
    await page.waitForTimeout(300);
    await expect(popover).toBeVisible();
    release();
    await expect.poll(() => agent.received().length).toBe(1);
    await expect(popover).toHaveCount(0);
  });
});
