import { expect, test } from '@playwright/test';
import { focusFrames, textAtSurfaceEdge } from './audits.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`compact controls ${width} ${theme}: geometry, retained state and menu parity`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/?frame=compact-controls&presentation=compact&theme=${theme}`);
      const all = page.getByRole('tab', { name: 'All, 6', exact: true });
      await expect(all).toHaveCSS('height', width === 390 ? '28px' : '31px');
      await expect(all).toHaveCSS('font-size', width === 390 ? '10px' : '11px');
      await expect(all.locator('.tmt-ui-tab-count')).toHaveCSS('font-size', '9px');
      await page.getByLabel('All draft', { exact: true }).fill('Keep this draft');
      await all.focus();
      await page.keyboard.press('ArrowRight');
      await expect(page.getByRole('tab', { name: 'Needs you, 1', exact: true })).toBeFocused();
      await page.keyboard.press('Home');
      await expect(all).toBeFocused();
      await expect(page.getByLabel('All draft', { exact: true })).toHaveValue('Keep this draft');
      for (const prefix of ['Sort', 'Native sort']) {
        const sort = page.getByRole('button', { name: `${prefix}: Recently updated`, exact: true });
        await expect(sort).toHaveCSS('width', width === 390 ? '145px' : '164px');
        await expect(sort).toHaveCSS('height', '39px');
        await expect(sort.locator('.tmt-ui-menu-value')).toHaveCSS('white-space', 'nowrap');
        await sort.press('ArrowDown');
        const menu = page.getByRole('menu', { name: `${prefix}: Recently updated`, exact: true });
        await expect(
          menu.getByRole('menuitemradio', { name: 'Recently updated', exact: true }),
        ).toBeFocused();
        await page.keyboard.press('Escape');
        await expect(menu).toBeHidden();
        await expect(sort).toBeFocused();
      }
      const action = page.getByRole('button', { name: 'New page', exact: true });
      const box = await action.boundingBox();
      // Natural text width follows the platform system font, as on the website.
      // Assert the adjacent mark/label geometry rather than one machine's glyph widths.
      const layout = await action.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const mark = node.querySelector('.tmt-ui-action-mark')!.getBoundingClientRect();
        const label = node.querySelector('.tmt-ui-action-label')!.getBoundingClientRect();
        return {
          left: mark.left - box.left,
          gap: label.left - mark.right,
          right: box.right - label.right,
          mark: mark.width,
        };
      });
      expect(layout.mark).toBe(17);
      expect(layout.gap).toBeCloseTo(8, 1);
      expect(layout.left).toBeCloseTo(width === 390 ? 11 : 14, 1);
      expect(layout.right).toBeCloseTo(width === 390 ? 11 : 14, 1);
      expect(box!.height).toBe(width === 390 ? 35 : 37);
      const frames = await focusFrames(page, '#compact-controls');
      expect(frames.clipped).toEqual([]);
      expect(frames.doubled).toEqual([]);
      expect(await textAtSurfaceEdge(page, '.tmt-ui-tabs')).toEqual([]);
    });
    test(`compact busy ${width} ${theme}: idle paint, mark and focus stay stable`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto(`/?frame=compact-controls&presentation=compact&theme=${theme}`);
      const buttons = page.locator('[data-testid="compact-variants"] button');
      const idle = await buttons.evaluateAll((nodes) =>
        nodes.map((n) => ({
          fill: getComputedStyle(n).backgroundColor,
          ink: getComputedStyle(n).color,
          width: n.getBoundingClientRect().width,
        })),
      );
      await page.getByRole('button', { name: 'Toggle fixture busy', exact: true }).click();
      for (let i = 0; i < 5; i++) {
        const b = buttons.nth(i);
        await b.focus();
        await b.hover();
        await expect(b).toHaveAttribute('aria-busy', 'true');
        await expect(b).toBeFocused();
        await expect(b).toHaveJSProperty('disabled', false);
        await expect(b).toHaveAttribute('aria-disabled', 'true');
        await expect(b).toHaveCSS('background-color', idle[i].fill);
        await expect(b).toHaveCSS('color', idle[i].ink);
        expect((await b.boundingBox())!.width).toBeCloseTo(idle[i].width, 1);
        await b.press('Enter');
        await expect(b).toBeFocused();
        await expect(b.locator('.tmt-ui-action-mark')).toHaveAttribute('aria-hidden', 'true');
        await expect(b.locator('.tmt-ui-action-mark')).toHaveAttribute('data-busy', 'true');
        await expect(b.locator('.tmt-ui-action-mark')).toHaveCSS('width', '17px');
      }
      await expect(page.getByTestId('compact-activations')).toHaveText('0');
    });
  }
test('default controls keep their existing presentation', async ({ page }) => {
  await page.goto('/?frame=compact-controls&theme=light');
  await expect(page.locator('[data-presentation="compact"]')).toHaveCount(0);
  await expect(page.getByRole('tab', { name: 'All, 6', exact: true })).toHaveCSS(
    'font-size',
    '14px',
  );
  await expect(page.getByRole('button', { name: 'New page', exact: true })).toHaveCSS(
    'display',
    // A flex item blockifies inline-grid; it keeps the default three-track grid.
    'grid',
  );
  await expect(
    page.getByRole('button', { name: 'Sort: Recently updated', exact: true }),
  ).not.toHaveCSS('width', '164px');
});
