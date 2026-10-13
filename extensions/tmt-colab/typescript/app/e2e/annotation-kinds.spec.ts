import { mkdirSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { captureDirectory } from './captures.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`explicit person and default agent mention styles ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/annotation-kind-browser.tsx';
        (await import(path)).mount();
      });
      const host = page.locator('#annotation-kind-fixture');
      await expect(host.locator('.message-mention[data-kind="agent"]')).toHaveCount(3);
      await expect(host.locator('.message-mention[data-kind="person"]')).toHaveCount(3);
      for (const chip of await host.locator('.message-mention').all()) {
        await expect(chip).toHaveCSS('border-top-width', '0px');
        await expect(chip).toHaveCSS('border-radius', '0px');
        await expect(chip.locator('svg')).toBeVisible();
        const contrast = await chip.evaluate((node) => {
          const style = getComputedStyle(node);
          const luminance = (color: string) => {
            const rgb = color
              .match(/[\d.]+/g)!
              .slice(0, 3)
              .map((value) => {
                const channel = Number(value) / 255;
                return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
              });
            return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
          };
          const first = luminance(style.color),
            second = luminance(style.backgroundColor);
          return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
        });
        expect(contrast).toBeGreaterThanOrEqual(4.5);
      }
      await expect(host.locator('button,[data-recipient]')).toHaveCount(0);
      mkdirSync(captureDirectory(), { recursive: true });
      await page.screenshot({
        path: `${captureDirectory()}/annotation-mention-kinds-${width}-${theme}.png`,
        fullPage: true,
      });
    });
  }
