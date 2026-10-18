import { expect, type Locator, type Page } from '@playwright/test';

/** The page list is the home; Today links to it as All pages. Waits until that view is shown. */
export async function openAllPages(page: Page, scope: Locator | Page = page) {
  await scope
    .getByRole('navigation', { name: 'Home views' })
    .getByRole('link', { name: 'All pages' })
    .click();
  await expect(scope.locator('section.home')).toBeVisible();
}

/** The home list, once the app has loaded the space. */
export const homeList = (page: Page) => page.locator('section.home');
