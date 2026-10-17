import { openAllPages } from '../test/all-pages.js';
import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function captureHeader(page: Page, testInfo: TestInfo, screen: 'page' | 'home') {
  const header = page.locator('.tmt-ui-header:visible');
  const attribution = header.getByRole('group', { name: /^Remote: local/ });
  await expect(attribution).toBeVisible();
  const fullName = await attribution.getAttribute('aria-label');
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(header).toHaveCSS('height', width === 390 ? '48px' : '56px');
      await expect(attribution).toHaveAttribute('aria-label', fullName!);
      await expect(header.locator('.colab-remote-label'))[
        width === 390 ? 'toBeHidden' : 'toBeVisible'
      ]();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      if (screen === 'page') {
        const chip = header.locator('.page-sharing');
        await expect(chip).toHaveText('Private · Live');
        expect(await chip.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        await expect(header.locator('.page-header-actions').getByRole('button')).toHaveCount(
          width === 390 ? 2 : 5,
        );
      }
      await page.screenshot({
        path: testInfo.outputPath(`one-row-${screen}-${width}-${theme}.png`),
      });
    }
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}

test.afterEach(disposeActiveWorlds);
test('native page titles fill missing visible home labels without opening the page', async () => {
  const testInfo = test.info();
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const title = 'Planning & <notes>';
    const created = createPage(world, title, '<p>Title acceptance.</p>');
    const browser = await pairBrowser(world, 'title-browser');
    const sent: string[] = [];
    browser.context.on('page', (page) =>
      page.on('websocket', (socket) => {
        if (!new URL(socket.url()).pathname.endsWith('/sync')) return;
        socket.on('framesent', ({ payload }) => {
          if (typeof payload === 'string') {
            const frame = JSON.parse(payload) as { type: string };
            sent.push(frame.type);
          }
        });
      }),
    );
    const page = await openColab(door, browser);
    await page.setViewportSize({ width: 1440, height: 900 });
    const row = () => page.locator(`[data-page-id="${created.pageId}"]`);
    await expect(row().getByRole('link')).toHaveText(title);
    expect(sent).toContain('hello');
    expect(sent).not.toContain('subscribe');
    await row().getByRole('link').click();
    await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(page).toHaveTitle(`${title} · Colab`);
    await expect.poll(() => sent.filter((type) => type === 'subscribe').length).toBe(1);
    await captureHeader(page, testInfo, 'page');
    await (await pageAction(page, 'Manage page')).click();
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
    await expect(row().getByRole('link')).toHaveText(title);
    await expect(page).toHaveTitle('Colab');
    await page.reload();
    await expect(row().getByRole('link')).toHaveText(title);
    await captureHeader(page, testInfo, 'home');
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
    await openAllPages(page);
    await row()
      .getByRole('button', { name: /^Actions for/ })
      .click();
    await expect(
      page.getByRole('menu').getByText(`Page ID: ${created.pageId}`, { exact: true }),
    ).toBeVisible();
    const other = await pairBrowser(world, 'other-title-browser');
    const otherPage = await openColab(door, other);
    await expect(
      otherPage.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
    ).toHaveText(title);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});
