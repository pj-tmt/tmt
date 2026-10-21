import { expect, type Page } from '@playwright/test';

/** The home list, once the app has loaded the space. */
export const homeList = (page: Page) => page.locator('section.home');

/**
 * The list opens on In progress, and a page with no task is under Discussion, so a spec that looks
 * for a page's row asks for All first.
 */
export async function showAllPages(page: Page) {
  const all = page.getByRole('tab', { name: 'All', exact: true });
  await all.click();
  await expect(all).toHaveAttribute('aria-selected', 'true');
}
