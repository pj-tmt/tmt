import { mkdirSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { captureDirectory } from './captures.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`agent chip, inline and bordered, ${width} ${theme}`, async ({ page }) => {
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
      await expect(host.locator('.agent-chip')).toHaveCount(6);
      await expect(host.locator('.agent-chip[data-variant="chip"]')).toHaveCount(3);
      for (const chip of await host.locator('.agent-chip').all()) {
        const variant = await chip.getAttribute('data-variant');
        // The chip form is a soft accent-tinted tag with no visible edge, as the website's tags
        // are; the inline form is plain text. The edge stays a transparent 1px so the chip keeps
        // its size where colours are forced.
        await expect(chip).toHaveCSS('border-top-width', variant === 'chip' ? '1px' : '0px');
        if (variant === 'chip')
          await expect(chip).toHaveCSS('border-top-color', 'rgba(0, 0, 0, 0)');
        // The chip form rounds at the small radius token (the website's chips: 5 to 7px).
        await expect(chip).toHaveCSS('border-radius', variant === 'chip' ? '6px' : '0px');
        if (variant === 'chip')
          expect(await chip.evaluate((node) => getComputedStyle(node).backgroundColor)).toMatch(
            /\/ 0\.13\)$/,
          );
        else await expect(chip).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
        await expect(chip.locator('svg')).toBeVisible();
        const contrast = await chip.evaluate((node) => {
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
          let surface: Element | null = node.parentElement;
          while (surface && getComputedStyle(surface).backgroundColor === 'rgba(0, 0, 0, 0)')
            surface = surface.parentElement;
          const paper = getComputedStyle(surface ?? document.body).backgroundColor;
          // A tinted chip is read on its fill laid over the surface behind it.
          const tint = getComputedStyle(node).backgroundColor;
          const channels = (color: string) => color.match(/[\d.]+/g)!.map(Number);
          const [pr, pg, pb] = channels(paper);
          const fill = tint === 'rgba(0, 0, 0, 0)' ? undefined : tint;
          const ground = fill
            ? (() => {
                const probe = document.createElement('canvas').getContext('2d')!;
                probe.fillStyle = `rgb(${pr} ${pg} ${pb})`;
                probe.fillRect(0, 0, 1, 1);
                probe.fillStyle = fill;
                probe.fillRect(0, 0, 1, 1);
                const [r, g, b] = probe.getImageData(0, 0, 1, 1).data;
                return `rgb(${r}, ${g}, ${b})`;
              })()
            : paper;
          const first = luminance(getComputedStyle(node).color),
            second = luminance(ground);
          return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
        });
        expect(contrast).toBeGreaterThanOrEqual(4.5);
      }
      await expect(host.locator('button,[data-recipient]')).toHaveCount(0);
      mkdirSync(captureDirectory(), { recursive: true });
      await page.screenshot({
        path: `${captureDirectory()}/agent-chips-${width}-${theme}.png`,
        fullPage: true,
      });
    });
  }
