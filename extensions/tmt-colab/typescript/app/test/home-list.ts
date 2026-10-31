import { expect, type Locator, type Page } from '@playwright/test';

/** The home list, once the app has loaded the space. */
export const homeList = (page: Page) => page.locator('section.home');

/**
 * The list opens on All, but a remembered or linked tab may be another, so a spec that looks for a
 * page's row asks for All first.
 */
export async function showAllPages(page: Page) {
  const all = page.getByRole('tab', { name: /^All, \d+\+?$/ });
  await all.click();
  await expect(all).toHaveAttribute('aria-selected', 'true');
}

/** Open the shell's one child tree when the phone drawer owns it. */
export async function openModuleNavigation(page: Page, scope: Locator = page.locator('body')) {
  const open = scope.getByRole('button', { name: 'Open navigation', exact: true });
  if ((page.viewportSize()?.width ?? 1440) <= 900) {
    await expect(open).toBeVisible();
    if ((await open.getAttribute('aria-expanded')) === 'false') await open.click();
  }
  const menu = scope.locator('.tmt-ui-rail-sub:visible');
  await expect(menu).toBeVisible();
  return menu;
}

/** Back through the same All pages row in the wide rail or phone drawer. */
export async function backToPageList(page: Page) {
  const menu = await openModuleNavigation(page);
  await menu.getByRole('link', { name: /^All pages(?: · .+)?$/ }).click();
}
