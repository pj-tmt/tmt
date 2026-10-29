import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (
      await import(path)
    ).mountPageList('projects', '/', ['Project 03', 'Project 01', 'Project 07']);
  });
}
const rail = (page: Page) => page.locator('#list-fixture .tmt-ui-rail-sub');
const names = (page: Page) =>
  rail(page).locator('a[data-id^="shortcut:"] .tmt-ui-module-menu-label');

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test(`migrated shortcuts are usable at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      if (width === 390) await page.getByRole('button', { name: 'Open navigation' }).click();
      await expect(names(page)).toHaveText(['Project 03', 'Project 01', 'Project 07']);
      const selectedColors = await rail(page)
        .locator('a[aria-current="page"]')
        .evaluate((node) => {
          const style = getComputedStyle(node);
          return [style.color, style.backgroundColor];
        });
      expect(selectedColors[0]).not.toBe(selectedColors[1]);
      await expect(rail(page).getByRole('button', { name: 'Manage shortcuts' })).toBeVisible();
      await expect(rail(page).getByRole('button', { name: 'New shortcut' })).toBeVisible();
      await page.screenshot({ path: capturePath(`5507-sidebar-${width}-${theme}.png`) });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      const first = rail(page).getByRole('link', { name: /^Project 03/ });
      await first.hover();
      await page.screenshot({ path: capturePath(`5507-sidebar-actions-${width}-${theme}.png`) });
      await rail(page).getByRole('button', { name: 'Reorder shortcut: Project 03' }).focus();
      await page.keyboard.press('ArrowDown');
      await expect(names(page)).toHaveText(['Project 01', 'Project 03', 'Project 07']);
      await expect(
        rail(page).getByRole('button', { name: 'Reorder shortcut: Project 03' }),
      ).toBeFocused();
      expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
    });
  }

test('sidebar edit, delete and new change only local shortcuts', async ({ page }) => {
  await mount(page);
  await rail(page)
    .getByRole('link', { name: /^Project 03/ })
    .hover();
  await rail(page).getByRole('button', { name: 'Edit shortcut Project 03' }).click();
  const edit = page.getByRole('dialog', { name: 'Edit shortcut' });
  await edit.getByRole('textbox', { name: 'Name' }).fill('Reviews');
  await edit.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(names(page)).toHaveText(['Reviews', 'Project 01', 'Project 07']);
  await rail(page)
    .getByRole('link', { name: /^Reviews/ })
    .hover();
  await rail(page).getByRole('button', { name: 'Delete shortcut Reviews' }).click();
  const confirm = page.getByRole('dialog', { name: 'Delete shortcut Reviews?' });
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(names(page)).toHaveText(['Reviews', 'Project 01', 'Project 07']);
  await rail(page)
    .getByRole('link', { name: /^Reviews/ })
    .hover();
  await rail(page).getByRole('button', { name: 'Delete shortcut Reviews' }).click();
  await confirm.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(names(page)).toHaveText(['Project 01', 'Project 07']);
  await rail(page).getByRole('button', { name: 'New shortcut' }).click();
  const create = page.getByRole('dialog', { name: 'New shortcut' });
  await create.getByRole('textbox', { name: 'Name' }).fill('New view');
  await create.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(names(page)).toHaveText(['Project 01', 'Project 07', 'New view']);
  expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
});

test.describe('touch phone', () => {
  test.use({ viewport: { width: 390, height: 900 }, hasTouch: true, isMobile: true });
  test('row actions and reorder through Manage are reachable without hover', async ({ page }) => {
    await mount(page);
    await page.getByRole('button', { name: 'Open navigation' }).tap();
    await expect(
      rail(page).getByRole('button', { name: 'Edit shortcut Project 03' }),
    ).toBeVisible();
    await expect(
      rail(page).getByRole('button', { name: 'Delete shortcut Project 03' }),
    ).toBeVisible();
    await rail(page).getByRole('button', { name: 'Manage shortcuts' }).tap();
    const dialog = page.getByRole('dialog', { name: 'Manage shortcuts' });
    await dialog.getByRole('button', { name: 'Move Project 03 down' }).tap();
    await expect(dialog.locator('.shortcut-list > li').first()).toContainText('Project 01');
    expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
  });
});
