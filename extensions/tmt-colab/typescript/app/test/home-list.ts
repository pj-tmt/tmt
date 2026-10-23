import { expect, type Page } from '@playwright/test';

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

/**
 * Back to the page list through the shell: the rail's nested All pages on a wide screen, the
 * module menu's All pages on a phone (the shell's own 900px breakpoint). The bar has no module
 * crumb to click.
 */
export async function backToPageList(page: Page) {
  const phone = (page.viewportSize()?.width ?? 1440) <= 900;
  await page
    .locator(phone ? '.tmt-ui-module-menu a:visible' : '.tmt-ui-rail-sub a:visible', {
      hasText: 'All pages',
    })
    .click();
}
