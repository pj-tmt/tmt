import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * Colab's own shell adopts the shared top bar (#5237): Colab is the root of the common controls, so
 * Search, Theme and the Remote link come from the bar once and the routes keep only their actions.
 */
test.beforeEach(async ({ page }) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
});

async function mount(page: Page, mode: 'home' | 'page' = 'home') {
  await page.goto('/');
  await page.evaluate(async (mode) => {
    const fixture = '/test/quick-switcher-browser.tsx';
    await (await import(/* @vite-ignore */ fixture)).mount(mode);
  }, mode);
  await expect(page.locator('.tmt-ui-topbar')).toBeVisible();
}
const bar = (page: Page) => page.locator('.tmt-ui-topbar:visible');
const search = (page: Page) => page.getByRole('button', { name: 'Search TMT', exact: true });
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Search pages', exact: true });
const shortcut = (page: Page) =>
  page.evaluate(() => (/Mac|iPhone|iPad|iPod/.test(navigator.platform) ? 'Meta+k' : 'Control+k'));

for (const mode of ['home', 'page'] as const)
  test(`the ${mode} route has one Search, one Theme and one Remote link, all the bar's`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mount(page, mode);
    await expect(page.locator('.tmt-ui-topbar')).toHaveCount(1);
    await expect(page.getByRole('button', { name: /^Search/ })).toHaveCount(1);
    await expect(search(page)).toBeVisible();
    await expect(page.getByRole('button', { name: /^Theme/ })).toHaveCount(1);
    await expect(page.getByRole('button', { name: /^Remote:/ })).toHaveCount(1);
    // None of the controls Colab used to draw beside the bar's.
    await expect(
      page.locator('.quick-switcher-control, .remote-header-chip, .remote-home-more, .theme'),
    ).toHaveCount(0);
    // Search is first at the left, Theme and Remote close the right group.
    const [searchBox, themeBox, remoteBox] = await Promise.all([
      search(page).boundingBox(),
      bar(page)
        .getByRole('button', { name: /^Theme/ })
        .boundingBox(),
      bar(page)
        .getByRole('button', { name: /^Remote:/ })
        .boundingBox(),
    ]);
    expect(searchBox!.x).toBeLessThan(themeBox!.x);
    expect(themeBox!.x).toBeLessThan(remoteBox!.x);
  });

test('the shortcut opens one dialog from anywhere and Escape returns focus to the bar Search', async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press(await shortcut(page));
  await expect(dialog(page)).toHaveCount(1);
  // The bar owns the one listener: a second press while it is open opens no second dialog.
  await page.keyboard.press(await shortcut(page));
  await expect(dialog(page)).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await search(page).focus();
  await page.keyboard.press('Enter');
  await expect(dialog(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  await expect(search(page)).toBeFocused();
});

test('Search opens on Colab settings too, a route the old header never searched from', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page);
  await page.getByRole('link', { name: 'Colab settings', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Colab settings' })).toBeVisible();
  await search(page).click();
  await expect(dialog(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(search(page)).toBeFocused();
});

test('the page actions belong to the route: they appear on a page and leave with it', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page, 'page');
  await expect(bar(page).getByRole('navigation', { name: 'Page actions' })).toBeVisible();
  await page.getByRole('link', { name: /^All pages(?: · .+)?$/ }).click();
  await expect(
    page.locator('#root, body').getByRole('heading', { level: 1 }).first(),
  ).toBeVisible();
  await expect(bar(page).getByRole('navigation', { name: 'Page actions' })).toHaveCount(0);
  // The bar itself, with Search, Theme and Remote, stays.
  await expect(search(page)).toBeVisible();
  await expect(bar(page).getByRole('button', { name: /^Theme/ })).toBeVisible();
});

for (const mode of ['home', 'page'] as const)
  test(`the ${mode} bar does not overflow at 390 and 320`, async ({ page }) => {
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      await mount(page, mode);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      expect(await bar(page).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      const box = (await search(page).boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      const remote = (await bar(page)
        .getByRole('button', { name: /^Remote:/ })
        .boundingBox())!;
      expect(remote.x + remote.width).toBeLessThanOrEqual(width);
      // No search hint on a phone: the control is its icon alone.
      await expect(search(page).locator('kbd')).toBeHidden();
    }
  });

test('hosted Colab adopts the host bar and does not add another shortcut or controls', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.evaluate(async () => {
    const fixture = '/test/spa-links-browser.tsx';
    (await import(/* @vite-ignore */ fixture)).mountHostApp('/colab/', undefined, true);
  });
  await expect(page.locator('.tmt-ui-topbar')).toHaveCount(1);
  await expect(page.getByRole('button', { name: /^Theme:/ })).toHaveCount(1);
  await expect(page.getByRole('button', { name: /^Remote:/ })).toHaveCount(1);
  await page.getByRole('button', { name: /^Remote:/ }).click();
  await expect(
    page
      .getByRole('dialog', { name: 'Remote', exact: true })
      .getByRole('link', { name: 'Remote settings' }),
  ).toHaveAttribute('href', /\/remote\/settings$/);
  await page.keyboard.press('Escape');
  await expect(
    page.locator('.remote-home-more, .remote-header-chip, .theme, .quick-switcher-control'),
  ).toHaveCount(0);
  await page.keyboard.press(await shortcut(page));
  await expect(dialog(page)).toHaveCount(0);
});

for (const width of [320, 390, 900, 901, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`Colab navigation capture ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'page');
      await expect(bar(page).getByRole('button', { name: /^Theme:/ })).toBeVisible();
      await expect(bar(page).getByRole('button', { name: /^Remote:/ })).toBeVisible();
      await expect(bar(page).getByRole('button', { name: 'More', exact: true })).toBeVisible();
      await page.screenshot({ path: capturePath(`nav-colab-${width}-${theme}.png`) });
    });
