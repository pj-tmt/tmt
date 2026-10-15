import { expect, test } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { captureDirectory } from './captures.js';
const directory = `${captureDirectory()}/header-actions`;
mkdirSync(directory, { recursive: true });
for (const width of [1440, 1024, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`shared header actions and centered Sort ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await page.evaluate(async (theme) => {
        document.documentElement.dataset.theme = theme;
        const path = '/test/ask-page-browser.tsx';
        await (await import(path)).mount({ attachments: true });
      }, theme);
      const actions = page.getByRole('navigation', { name: 'Page actions' });
      const discussion = actions.getByRole('button', { name: /^Discussion \(/ });
      const more = actions.getByRole('button', { name: 'More', exact: true });
      await expect(discussion).toBeVisible();
      const bounds = await actions.locator('button').evaluateAll((nodes) =>
        nodes
          .map((node) => node.getBoundingClientRect())
          .filter((box) => box.width)
          .map((box) => ({ left: box.left, right: box.right })),
      );
      for (let i = 1; i < bounds.length; i++)
        expect(bounds[i].left - bounds[i - 1].right).toBeGreaterThanOrEqual(6);
      await discussion.hover();
      expect(await discussion.evaluate((node) => node.matches(':focus-visible'))).toBe(false);
      await expect(discussion).toHaveCSS('outline-style', 'none');
      await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(1);
      await page.screenshot({ path: `${directory}/header-hover-${width}-${theme}.png` });
      await page.mouse.move(0, 899);
      await discussion.focus();
      await page.keyboard.press('Tab');
      await page.keyboard.press('Shift+Tab');
      await expect(discussion).toBeFocused();
      expect(await discussion.evaluate((node) => node.matches(':focus-visible'))).toBe(true);
      await expect(discussion).toHaveCSS('outline-width', '1px');
      await expect(discussion).toHaveCSS('outline-offset', '-1px');
      await page.screenshot({ path: `${directory}/header-keyboard-${width}-${theme}.png` });
      await more.hover();
      await expect(more.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeVisible();
      await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(1);
      await expect(discussion.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeHidden();
      await page.screenshot({ path: `${directory}/header-tooltips-${width}-${theme}.png` });
      writeFileSync(
        `${directory}/header-${width}-${theme}.json`,
        JSON.stringify(
          await actions.evaluate((node) => ({
            gap: getComputedStyle(node).gap,
            tooltips: document.querySelectorAll('.tmt-ui-icon-action-tooltip:popover-open').length,
            buttons: [...node.querySelectorAll('button')]
              .filter((b) => b.getBoundingClientRect().width)
              .map((b) => ({
                name: b.ariaLabel,
                rect: b.getBoundingClientRect().toJSON(),
                outline: getComputedStyle(b).outline,
              })),
          })),
          null,
          2,
        ),
      );
      await page.keyboard.press('Escape');
      await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
      await page.mouse.move(0, 899);
      await more.hover();
      await expect(more.locator('..').locator('.tmt-ui-icon-action-tooltip')).toBeVisible();
      await more.click();
      await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/page-list-browser.tsx';
        await (await import(path)).mountPageList('few');
      });
      const sort = page.locator('#list-fixture').getByRole('button', { name: /^Sort:/ });
      await expect(sort).toBeVisible();
      const chevron = sort.locator('svg.lucide-chevron-down');
      await expect(chevron).toHaveCount(1);
      const center = await sort.evaluate((node) => {
        const a = node.getBoundingClientRect(),
          b = node.querySelector('svg')!.getBoundingClientRect();
        return Math.abs((a.top + a.bottom - b.top - b.bottom) / 2);
      });
      expect(center).toBeLessThanOrEqual(1);
      await expect(chevron).toHaveAttribute('aria-hidden', 'true');
      await page.screenshot({ path: `${directory}/sort-closed-${width}-${theme}.png` });
      await sort.click();
      await expect(page.getByRole('menu', { name: 'Sort' })).toBeVisible();
      await page.screenshot({ path: `${directory}/sort-open-${width}-${theme}.png` });
      // A non-popup action retains focus after activation, so this checks pointer
      // focus itself rather than an opener that immediately moves focus into a menu.
      await page.goto('/');
      await page.evaluate(async () => {
        const fixture = '/test/browser-overlays.tsx';
        (await import(fixture)).mount();
      });
      const action = page.getByRole('button', { name: 'Header action', exact: true });
      await action.click();
      await expect(page.getByTestId('header-count')).toHaveText('1');
      await expect(action).toBeFocused();
      expect(await action.evaluate((node) => node.matches(':focus-visible'))).toBe(false);
      await expect(action).toHaveCSS('outline-style', 'none');
      await page.keyboard.press('Tab');
      await page.keyboard.press('Shift+Tab');
      await expect(action).toBeFocused();
      await expect(action).toHaveCSS('outline-width', '1px');
    });
  }
