import { expect, test } from '@playwright/test';

// The legacy bar cluster (search and chip slots) sits left of the module's own actions, so the
// module's last action closes the bar at its far right edge.
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the shell cluster leads and the module's action closes the bar at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/?frame=shell&theme=${theme}`);
      const bar = page.locator('.tmt-ui-shell-bar');
      const search = page.getByRole('button', { name: 'Search pages', exact: true });
      const chip = page.getByRole('button', { name: 'Remote · 2', exact: true });
      const action = bar.getByRole('button', { name: 'Settings', exact: true });
      const heading = bar.getByRole('heading', { name: 'Pages', exact: true });
      await expect(action).toBeVisible();
      const [searchBox, chipBox, actionBox, headingBox] = await Promise.all(
        [search, chip, action, heading].map((control) => control.boundingBox()),
      );
      expect(searchBox!.x + searchBox!.width).toBeLessThanOrEqual(chipBox!.x + 0.5);
      expect(chipBox!.x + chipBox!.width).toBeLessThanOrEqual(actionBox!.x + 0.5);
      const barBox = (await bar.boundingBox())!;
      const inset = await bar
        .locator('.tmt-ui-header')
        .evaluate((node) => Number.parseFloat(getComputedStyle(node).paddingRight));
      expect(
        Math.abs(actionBox!.x + actionBox!.width - (barBox.x + barBox.width - inset)),
      ).toBeLessThanOrEqual(1);
      // The title keeps clear of the cluster.
      expect(headingBox!.x + headingBox!.width).toBeLessThanOrEqual(searchBox!.x + 0.5);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth),
      ).toBe(0);
      // The cluster and the action share one row of the bar.
      expect(
        Math.abs(searchBox!.y + searchBox!.height / 2 - (actionBox!.y + actionBox!.height / 2)),
      ).toBeLessThanOrEqual(1);
    });
