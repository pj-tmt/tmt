import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`shared input without mentions keeps editing and an inline accessory ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/test/rich-input-browser.html');
      const input = page.getByRole('textbox', { name: 'Shared text', exact: true });
      await input.pressSequentially('@agent');
      await expect(input).toHaveText('@agent');
      await expect(page.getByRole('listbox')).toHaveCount(0);
      await expect(input.locator('.agent-chip')).toHaveCount(0);
      await input.press('Shift+Enter');
      await input.pressSequentially('second line');
      await input.press('Enter');
      await expect(page.getByLabel('Applied text')).toHaveText('@agent\nsecond line');
      await input.press('ControlOrMeta+A');
      await input.press('Backspace');
      await expect(input).toHaveText('');
      await input.press('ControlOrMeta+Z');
      await expect(input).toHaveText('@agent\nsecond line', { useInnerText: true });
      await page.getByRole('button', { name: 'Toggle access' }).click();
      await expect(input).toHaveAttribute('contenteditable', 'false');
      await page.getByRole('button', { name: 'Toggle access' }).click();
      await expect(input).toHaveText('@agent\nsecond line', { useInnerText: true });
      await input.press('Escape');
      await expect(input).toHaveText('');
      await input.press('ControlOrMeta+Z');
      await expect(input).toHaveText('');
      await input.pressSequentially('Shared left edge');
      const root = await input.elementHandle();
      await input.press('Home');
      await input.press('ArrowRight');
      await input.press('ArrowRight');
      for (const selected of [false, true]) {
        if (selected) await input.press('Shift+ArrowRight');
        const selection = () =>
          input.evaluate(() => {
            const range = window.getSelection()!;
            return { anchor: range.anchorOffset, focus: range.focusOffset, text: range.toString() };
          });
        const before = await selection();
        for (const visible of [false, true]) {
          await input.press('Alt+a');
          if (visible)
            await expect(page.getByText('Fixture accessory', { exact: true })).toBeVisible();
          else await expect(page.getByText('Fixture accessory', { exact: true })).toHaveCount(0);
          await expect.poll(selection).toEqual(before);
          expect(
            await root!.evaluate((node) => node.isConnected && document.activeElement === node),
          ).toBe(true);
        }
      }
      const geometry = await input.evaluate((node) => {
        const range = document.createRange();
        const text = node.querySelector('p')!.firstChild!;
        range.setStart(text, 0);
        range.setEnd(text, 1);
        const glyph = range.getBoundingClientRect();
        const accessory = document.querySelector('.rich-input-accessory')!;
        const row = accessory.getBoundingClientRect();
        const style = getComputedStyle(accessory);
        const surface = node.parentElement!.getBoundingClientRect();
        return {
          textLeft: glyph.left,
          rowLeft: row.left + parseFloat(style.paddingLeft),
          rowTop: row.top,
          textBottom: node.getBoundingClientRect().bottom,
          rowBottom: row.bottom,
          surfaceBottom: surface.bottom,
          surfaceRight: surface.right,
          viewport: innerWidth,
        };
      });
      expect(Math.abs(geometry.textLeft - geometry.rowLeft)).toBeLessThanOrEqual(1);
      expect(geometry.rowTop).toBeGreaterThanOrEqual(geometry.textBottom);
      expect(geometry.rowBottom).toBeLessThanOrEqual(geometry.surfaceBottom);
      expect(geometry.surfaceRight).toBeLessThanOrEqual(geometry.viewport);
      const message = page.getByRole('combobox', { name: 'Message with accessory', exact: true });
      await message.pressSequentially('Message draft');
      await expect(message).toHaveText('Message draft');
      await expect(page.getByText('Message accessory', { exact: true })).toBeVisible();
      const directory = process.env.COLAB_RICH_INPUT_CAPTURE_DIR;
      if (directory) {
        mkdirSync(directory, { recursive: true });
        await page.screenshot({ path: `${directory}/${width}-${theme}.png` });
      }
    });
  }
}
