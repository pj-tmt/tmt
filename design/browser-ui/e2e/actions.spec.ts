import { expect, test } from '@playwright/test';
import { controlBoundaries, focusFrames, textAtSurfaceEdge } from './audits';

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 390]) {
    test(`leading action ${width} ${theme}: one decorative slot, busy fence and stable focus`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.clock.install({ time: 0 });
      await page.clock.pauseAt(0);
      await page.goto('/?theme=' + theme + '#actions');
      const example = page.locator(
        '.gallery-column[data-theme="' + theme + '"] [data-action-example]',
      );
      const add = example.getByRole('button', { name: 'New project', exact: true });
      const ghost = example.getByRole('button', { name: 'Delete', exact: true });
      await expect(add).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      await expect(add).toHaveAccessibleName('New project');
      const leadingStates = page
        .locator('.gallery-pair')
        .filter({
          has: page.getByRole('heading', {
            name: 'BrowserAction · leading icon and busy mark',
            exact: true,
          }),
        })
        .locator('.gallery-column[data-theme="' + theme + '"]');
      const disabled = leadingStates.locator('button:disabled');
      await expect(disabled).toHaveAccessibleName('New project');
      await expect(disabled).toHaveAccessibleDescription('Read only');
      await expect(add.getByRole('img')).toHaveCount(0);
      await expect(add.locator('.tmt-ui-action-mark')).toHaveCount(1);
      await expect(add.locator('.lucide-plus')).toHaveCount(1);
      await expect(ghost).toHaveCSS('border-top-width', '0px');
      await expect(ghost).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
      await expect(ghost).toHaveCSS('text-decoration-line', 'none');
      const ink = await ghost.evaluate((node) => {
        const style = getComputedStyle(node);
        return {
          secondary: style.getPropertyValue('--tmt-ui-color-secondary').trim(),
          primary: style.getPropertyValue('--tmt-ui-color-text').trim(),
        };
      });
      const rgb = (hex: string) =>
        'rgb(' +
        (hex.match(/[a-f0-9]{2}/gi) ?? []).map((part) => parseInt(part, 16)).join(', ') +
        ')';
      await expect(ghost).toHaveCSS('color', rgb(ink.secondary));
      await ghost.hover();
      await expect(ghost).toHaveCSS('color', rgb(ink.primary));
      await page.mouse.move(0, 0);
      await ghost.focus();
      await expect(ghost).toHaveCSS('color', rgb(ink.primary));
      await ghost.evaluate((node) => (node as HTMLButtonElement).blur());
      await expect(ghost).toHaveCSS('color', rgb(ink.secondary));
      const before = (await add.boundingBox())!;
      await add.click();
      await expect(add).toBeFocused();
      await expect(add).toHaveAttribute('aria-busy', 'true');
      await expect(add).toHaveAttribute('aria-disabled', 'true');
      expect(await add.evaluate((node) => (node as HTMLButtonElement).disabled)).toBe(false);
      await expect(add.locator('.lucide-plus')).toHaveCount(1);
      await page.clock.runFor(401);
      await expect(add.locator('.tmt-ui-busy-mark')).toHaveCount(1);
      await expect(add.locator('.lucide-plus')).toHaveCount(0);
      const pending = (await add.boundingBox())!;
      expect(pending.width).toBe(before.width);
      expect(pending.height).toBe(before.height);
      await page.keyboard.press('Enter');
      await page.keyboard.press('Space');
      await page.mouse.click(pending.x + pending.width / 2, pending.y + pending.height / 2);
      await expect(example.getByRole('status')).toHaveText('Requests 1 · Submits 1');
      await expect(add).toBeFocused();
      await example
        .getByRole('button', { name: 'Finish example', exact: true })
        .evaluate((node) => (node as HTMLButtonElement).click());
      await page.clock.runFor(401);
      await expect(add.locator('.lucide-plus')).toHaveCount(1);
      await expect(add.locator('.tmt-ui-busy-mark')).toHaveCount(0);
      await expect(add).toBeFocused();
      expect((await add.boundingBox())!.width).toBe(before.width);
      await page.keyboard.press('Enter');
      await expect(example.getByRole('status')).toHaveText('Requests 2 · Submits 2');
      await example
        .getByRole('button', { name: 'Finish example', exact: true })
        .evaluate((node) => (node as HTMLButtonElement).click());
      await page.clock.runFor(401);
      const selector = '.gallery-column[data-theme="' + theme + '"] [data-action-example]';
      const boundaries = await controlBoundaries(page, selector);
      expect(boundaries.borders.misses).toEqual([]);
      expect(await textAtSurfaceEdge(page, selector)).toEqual([]);
      const focus = await focusFrames(page, selector);
      expect(focus.checked).toBeGreaterThan(0);
      expect(focus.clipped).toEqual([]);
      expect(focus.doubled).toEqual([]);
    });
  }
