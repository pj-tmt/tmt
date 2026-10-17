import { expect, type Locator, type Page } from '@playwright/test';

/** Today is the home; the page list is its All pages view. Waits until that view is shown. */
export async function openAllPages(page: Page, scope: Locator | Page = page) {
  await scope
    .getByRole('navigation', { name: 'Home views' })
    .getByRole('link', { name: 'All pages' })
    .click();
  await expect(scope.locator('section.home')).toBeVisible();
}

/** The Today greeting is the home's own heading. */
export const todayHeading = (page: Page) =>
  page.getByRole('heading', { level: 1, name: /^Good (morning|afternoon|evening)$/ });
