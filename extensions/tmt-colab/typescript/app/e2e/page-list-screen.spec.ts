import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/** The website list keeps its hero and controls above independently scrolling page rows. */
async function mount(page: Page, width: number, theme: 'light' | 'dark') {
  await page.setViewportSize({ width, height: width < 480 ? 844 : 900 });
  await page.emulateMedia({ colorScheme: theme });
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(async (theme) => {
    document.documentElement.dataset.theme = theme;
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('work');
  }, theme);
}
const app = (page: Page) => page.locator('#list-fixture');
const rows = (page: Page) => app(page).locator('.pages > li');

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`one screen with a fixed top block and a scrolling list at ${width} ${theme}`, async ({
      page,
    }) => {
      await mount(page, width, theme);
      await expect(
        app(page).getByRole('heading', { name: 'A shared place for ideas.' }),
      ).toBeVisible();
      await expect(app(page).getByRole('tab')).toHaveCount(6);
      for (const name of [
        'All, 75',
        '◆ Needs you, 1 · waiting',
        'Discussion, 25',
        'In progress, 25',
        'Done, 25',
        'Archived, 1',
      ])
        await expect(app(page).getByRole('tab', { name, exact: true })).toBeVisible();
      await expect(app(page).getByRole('tab', { name: 'All, 75' })).toHaveAttribute(
        'aria-selected',
        'true',
      );
      await expect(app(page).locator('.tmt-ui-tabs')).toHaveAttribute('data-variant', 'flat');
      await expect(app(page).locator('.tmt-ui-tabs')).toHaveAttribute('data-wrap', '');
      await expect(app(page).getByRole('button', { name: 'New page' })).toBeVisible();
      await expect(rows(page)).toHaveCount(50);
      const count = app(page).locator('[data-page-list-count]');
      await expect(count).toHaveText('Showing 50 of 75');

      const top = async () => {
        const box = (selector: string) =>
          app(page)
            .locator(selector)
            .first()
            .boundingBox()
            .then((value) => value!);
        const [search, tabs, list] = await Promise.all([
          box('.page-list-search'),
          box('[role="tablist"]'),
          box('.page-list-scroll'),
        ]);
        return { search, tabs, list };
      };
      const before = await top();
      if (width < 480) {
        const tops = await app(page)
          .getByRole('tab')
          .evaluateAll((tabs) => tabs.map((tab) => tab.getBoundingClientRect().top));
        expect(Math.max(...tops)).toBeGreaterThan(Math.min(...tops));
        expect(
          await app(page)
            .getByRole('tablist')
            .evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true);
      }
      await page.screenshot({ path: await capturePath(`4273-list-${width}-${theme}.png`) });

      const list = app(page).locator('.page-list-scroll');
      await list.evaluate((node) => (node.scrollTop = node.scrollHeight));
      await expect(rows(page)).toHaveCount(75);
      await expect(count).toHaveText('75 pages');
      await list.evaluate((node) => (node.scrollTop = node.scrollHeight / 2));
      expect(await top()).toEqual(before);
      expect(await page.evaluate(() => document.documentElement.scrollHeight > innerHeight)).toBe(
        false,
      );
      await page.screenshot({
        path: await capturePath(`4273-list-${width}-${theme}-scrolled.png`),
      });
    });

test('Needs you is a tab: it lists the pages waiting on you and leaves with another tab', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  await app(page)
    .getByRole('tab', { name: /Needs you, 1/ })
    .click();
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).locator('.tmt-ui-list-title > a')).toHaveText('Review 2');
  expect(new URL(page.url()).hash).toContain('needs=true');
  await app(page).getByRole('tab', { name: 'Discussion, 25' }).click();
  await expect(rows(page)).toHaveCount(25);
  expect(new URL(page.url()).hash).not.toContain('needs');
});

test('New page gives the command that creates a page on the machine', async ({ page }) => {
  await mount(page, 1440, 'light');
  await app(page).getByRole('button', { name: 'New page' }).click();
  const dialog = page.getByRole('dialog', { name: 'New page' });
  await expect(dialog).toContainText('tmt colab page create --title "Notes" --layout column');
  await expect(dialog).toContainText('Return to your TMT workspace after creating the page.');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: 'New page' })).toBeFocused();
  await expect(rows(page)).toHaveCount(50);
  await app(page).getByRole('button', { name: 'New page' }).click();
  await dialog.getByRole('button', { name: 'Close New page' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: 'New page' })).toBeFocused();
});

for (const width of [1586, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`website page dialogs remain contained at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      await page.setViewportSize({ width, height: 992 });
      const opener = app(page).getByRole('button', { name: 'New page', exact: true });
      await opener.click();
      const create = app(page).getByRole('dialog', { name: 'New page', exact: true });
      await expect(
        create.locator('footer').getByRole('button', { name: 'Close', exact: true }),
      ).toBeVisible();
      await expect(
        create
          .getByRole('button', { name: 'Copy', exact: true })
          .locator('svg[aria-hidden="true"]'),
      ).toHaveCount(1);
      expect(await create.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      await page.screenshot({ path: await capturePath(`new-page-${width}-${theme}.png`) });
      await create.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(opener).toBeFocused();
      const menu = rows(page)
        .first()
        .getByRole('button', { name: /^Actions for/ });
      await menu.click();
      await page.getByRole('menuitem', { name: 'Archive…', exact: true }).click();
      const archive = app(page).getByRole('dialog');
      await expect(archive).toContainText(
        'An archived page stays readable, but nobody can edit it.',
      );
      await expect(archive.locator('footer')).toHaveCSS('border-top-width', '1px');
      const cancel = archive.locator('footer').getByRole('button', { name: 'Cancel', exact: true });
      await expect(cancel).toHaveCSS('border-top-width', '1px');
      expect(await archive.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({ path: await capturePath(`archive-${width}-${theme}.png`) });
      await cancel.click();
      await expect(archive).toHaveCount(0);
      await expect(menu).toBeFocused();
      expect(await page.evaluate(() => window.__pageOperations)).toEqual([]);
    });
