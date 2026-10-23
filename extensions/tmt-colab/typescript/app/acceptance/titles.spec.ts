import { backToPageList, homeList, showAllPages } from '../test/home-list.js';
import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function captureHeader(page: Page, testInfo: TestInfo, screen: 'page' | 'home') {
  const header = page.locator('.tmt-ui-header:visible');
  // The bar has no module crumb: the header's accessible name carries the module and connection.
  await expect(header).toHaveAttribute('aria-label', /^Colab · local/);
  const fullName = await header.getAttribute('aria-label');
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(header).toHaveCSS('height', width === 390 ? '48px' : '56px');
      await expect(header).toHaveAttribute('aria-label', fullName!);
      const mark = page.locator('.tmt-ui-shell-brand .tmt-brand-mark');
      if (width === 1440) await expect(mark).toBeVisible();
      else await expect(mark).toBeHidden();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      if (screen === 'page') {
        const connection = header.locator('.page-connection');
        await expect(connection).toHaveText('Private · Live');
        expect((await connection.boundingBox())!.width).toBeLessThanOrEqual(1);
        const actions = header.getByRole('navigation', { name: 'Page actions' });
        await expect(actions.getByRole('button')).toHaveCount(3);
        for (const name of [/^Discussion \(/, 'Agents', 'More'])
          await expect(actions.getByRole('button', { name, exact: true })).toBeInViewport();
        await expect(
          page
            .locator('.tmt-ui-shell-tools')
            .getByRole('button', { name: 'Search pages', exact: true }),
        ).toBeInViewport();
        const more = actions.getByRole('button', { name: 'More', exact: true });
        await more.click();
        const menu = actions.getByRole('menu');
        for (const name of [/^Share…/, /^Remote:/, /^Files \(/, 'Source'])
          await expect(menu.getByRole('menuitem', { name, exact: true })).toBeVisible();
        await expect(menu.getByRole('menuitem', { name: /^Share…/ })).toContainText(
          'Private · Live',
        );
        await page.screenshot({ path: testInfo.outputPath(`page-more-${width}-${theme}.png`) });
        await page.keyboard.press('Escape');
        await expect(menu).toHaveCount(0);
        await expect(more).toBeFocused();
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
    await showAllPages(page);
    const row = () => page.locator(`[data-page-id="${created.pageId}"]`);
    await expect(row().getByRole('link')).toHaveText(title);
    expect(sent).toContain('hello');
    expect(sent).not.toContain('subscribe');
    await row().getByRole('link').click();
    await expect(
      page.locator('.tmt-ui-header').getByRole('heading', { name: title, exact: true }),
    ).toBeVisible();
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
    await backToPageList(page);
    await showAllPages(page);
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
    await expect(homeList(page)).toBeVisible();
    await row()
      .getByRole('button', { name: /^Actions for/ })
      .click();
    await expect(
      page.getByRole('menu').getByText(`Page ID: ${created.pageId}`, { exact: true }),
    ).toBeVisible();
    const other = await pairBrowser(world, 'other-title-browser');
    const otherPage = await openColab(door, other);
    await showAllPages(otherPage);
    await expect(
      otherPage.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
    ).toHaveText(title);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});
