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

for (const theme of ['light', 'dark'] as const)
  test(`all busy action variants keep idle paint under hover, press and keyboard focus (${theme})`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.goto('/?theme=' + theme + '#actions');
    for (const variant of ['primary', 'secondary', 'text', 'destructive', 'ghost']) {
      const pair = page
        .locator('.gallery-pair')
        .filter({
          has: page.getByRole('heading', { name: 'BrowserAction · ' + variant, exact: true }),
        })
        .locator('.gallery-column[data-theme="' + theme + '"]');
      const cell = (name: string) =>
        pair
          .locator('.gallery-cell')
          .filter({
            has: page.locator('.gallery-cell-label', { hasText: new RegExp('^' + name + '$') }),
          })
          .locator('button');
      const idle = cell('idle');
      const busy = cell('busy');
      await page.mouse.move(0, 0);
      const paint = (node: HTMLElement | SVGElement) => ({
        fill: getComputedStyle(node).backgroundColor,
        ink: getComputedStyle(node.querySelector('.tmt-ui-action-label')!).color,
      });
      const expected = await idle.evaluate(paint);
      await busy.hover();
      expect(await busy.evaluate(paint)).toEqual(expected);
      expect(await busy.evaluate((node: HTMLButtonElement) => node.disabled)).toBe(false);
      await page.mouse.down();
      expect(await busy.evaluate(paint)).toEqual(expected);
      await expect(busy).toHaveCSS('scale', 'none');
      await page.mouse.up();
      await busy.focus();
      await page.keyboard.press('Tab');
      await page.keyboard.press('Shift+Tab');
      await expect(busy).toBeFocused();
      expect(await busy.evaluate(paint)).toEqual(expected);
      const frame = await busy.evaluate((node) => {
        const s = getComputedStyle(node);
        return parseFloat(s.outlineWidth) || parseFloat(s.borderTopWidth);
      });
      expect(frame).toBeGreaterThanOrEqual(1);
    }
  });

for (const width of [1586, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`Source header action preserves draft, busy focus and dismissal ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/?frame=source&open=false&theme=' + theme);
      await page.getByRole('button', { name: 'Open source', exact: true }).click();
      const drawer = page.getByRole('dialog', { name: 'Source', exact: true });
      const save = drawer.getByRole('button', { name: 'Save', exact: true });
      const close = drawer.getByRole('button', { name: 'Close Source', exact: true });
      await expect(drawer.locator('.tmt-ui-drawer-header button')).toHaveText(['Save', '']);
      const source = drawer.getByRole('textbox', { name: 'Page source', exact: true });
      await source.fill('Retained draft');
      await page.mouse.move(0, 0);
      const idle = await save.evaluate((node) => ({
        fill: getComputedStyle(node).backgroundColor,
        ink: getComputedStyle(node).color,
      }));
      const box = (await save.boundingBox())!;
      await save.click();
      await expect(save).toBeFocused();
      await expect(save).toHaveAttribute('aria-busy', 'true');
      expect(await save.evaluate((node: HTMLButtonElement) => node.disabled)).toBe(false);
      expect(
        await save.evaluate((node) => ({
          fill: getComputedStyle(node).backgroundColor,
          ink: getComputedStyle(node).color,
        })),
      ).toEqual(idle);
      const pending = (await save.boundingBox())!;
      expect(pending.width).toBe(box.width);
      expect(pending.height).toBe(box.height);
      await page.keyboard.press('Enter');
      await page.keyboard.press('Space');
      await page.mouse.click(pending.x + pending.width / 2, pending.y + pending.height / 2);
      await expect(page.locator('[data-source-activations]')).toHaveAttribute(
        'data-source-activations',
        '1',
      );
      await page.keyboard.press('Tab');
      await expect(close).toBeFocused();
      await page.keyboard.press('Escape');
      await expect(drawer).toBeHidden();
      const opener = page.getByRole('button', { name: 'Open source', exact: true });
      await expect(opener).toBeFocused();
      await opener.click();
      await expect(drawer).toBeVisible();
      await expect(source).toHaveValue('Retained draft');
      const boundaries = await controlBoundaries(page, '.tmt-ui-drawer');
      expect(boundaries.borders.misses).toEqual([]);
      expect(boundaries.fields.measured).toBeGreaterThan(0);
      expect(boundaries.fields.sameFill).toEqual([]);
      expect(boundaries.fields.unlabelled).toEqual([]);
      expect(await textAtSurfaceEdge(page, '.tmt-ui-drawer-header')).toEqual([]);
      await page.keyboard.press('Tab');
      const frames = await focusFrames(page, '.tmt-ui-drawer-header');
      expect(frames.checked).toBeGreaterThanOrEqual(2);
      expect(frames.clipped).toEqual([]);
      expect(frames.doubled).toEqual([]);
    });
