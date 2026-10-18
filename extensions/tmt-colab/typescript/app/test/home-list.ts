import type { Page } from '@playwright/test';

/** The home list, once the app has loaded the space. */
export const homeList = (page: Page) => page.locator('section.home');
