import { expect, test } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
test('native page titles become durable browser-local labels after their first verified fold', async () => {
  const testInfo = test.info();
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const title = 'Planning & <notes>';
    const created = createPage(world, title, '<p>Title acceptance.</p>');
    const browser = await pairBrowser(world, 'title-browser');
    const page = await openColab(door, browser);
    await page.setViewportSize({ width: 1440, height: 900 });
    const row = () => page.locator(`[data-page-id="${created.pageId}"]`);
    await expect(row().getByRole('heading')).toHaveText('Untitled, not opened in this browser yet');
    await row().locator('a').click();
    await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(`${title} · Colab`);
    await page.getByRole('button', { name: 'Manage page', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(dialog.getByText('Active', { exact: true })).toBeVisible();
    await expect(dialog.getByText(`Page ID: ${created.pageId}`, { exact: true })).toBeHidden();
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
        await page.screenshot({ path: testInfo.outputPath(`title-dialog-${width}-${theme}.png`) });
        expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      }
    }
    await page.setViewportSize({ width: 1440, height: 900 });
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await page.getByRole('link', { name: 'Space home', exact: true }).click();
    await expect(row().getByRole('heading')).toHaveText(title);
    await expect(page).toHaveTitle('Colab');
    await page.reload();
    await expect(row().getByRole('heading')).toHaveText(title);
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
        await page.screenshot({ path: testInfo.outputPath(`title-home-${width}-${theme}.png`) });
        expect(await row().evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      }
    }
    await row().getByText('Details', { exact: true }).click();
    await expect(row().getByText(`Page ID: ${created.pageId}`, { exact: true })).toBeVisible();
    const other = await pairBrowser(world, 'other-title-browser');
    const otherPage = await openColab(door, other);
    await expect(
      otherPage.locator(`[data-page-id="${created.pageId}"]`).getByRole('heading'),
    ).toHaveText('Untitled, not opened in this browser yet');
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});
