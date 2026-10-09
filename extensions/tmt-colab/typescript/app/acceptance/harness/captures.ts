import fs from 'node:fs';
import path from 'node:path';
import type { Page } from '@playwright/test';

/** With `COLAB_LIFECYCLE_CAPTURE_DIR` set, saves the page at 1440/390 in light and dark. */
export async function captureResponsive(page: Page, name: string) {
  const directory = process.env.COLAB_LIFECYCLE_CAPTURE_DIR;
  if (!directory) return;
  fs.mkdirSync(directory, { recursive: true });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
      await page.screenshot({ path: path.join(directory, `${name}-${width}-${theme}.png`) });
    }
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}
