import { expect, type Locator, type Page } from '@playwright/test';

/** Product geometry: a floating window fills the gap between the top bar and its visible dock. */
export async function expectAvailableDockWindow(page: Page, panel: Locator) {
  const topbar = page.locator('.tmt-ui-topbar:visible');
  const dock = page.locator('.page-floating-actions:visible');
  await expect(topbar).toHaveCount(1);
  await expect(dock).toHaveCount(1);
  await expect
    .poll(async () => {
      const frame = (await panel.boundingBox())!;
      const bar = (await topbar.boundingBox())!;
      const actions = (await dock.boundingBox())!;
      return {
        top: Math.round(frame.y - bar.y - bar.height),
        bottom: Math.round(actions.y - frame.y - frame.height),
      };
    })
    .toEqual({ top: 12, bottom: 16 });
}
