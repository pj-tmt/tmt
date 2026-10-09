import { expect, test } from '@playwright/test';
import { mkdir } from 'node:fs/promises';

// The visual server uses 4176; the common suite serves its previews on 4173.
test.use({ baseURL: 'http://127.0.0.1:4176' });

test('reference blocks copy exact local values and keep manual selection on denial', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/e2e/visual/reference-browser.html');
  for (const [name, expected] of [
    ['Local thread reference', '11111111-1111-4111-8111-111111111111'],
    ['Local snapshot reference', 'tmt:whiteboard:snapshot:11111111-1111-4111-8111-111111111111'],
  ]) {
    const field = page.getByRole('textbox', { name });
    const block = field.locator('xpath=ancestor::*[contains(@class,"tmt-ui-command")][1]');
    await block.getByRole('button', { name: 'Copy reference' }).focus();
    await page.keyboard.press('Enter');
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(expected);
    await expect(block.getByRole('status')).toHaveText('Copied.');
  }
  await page.evaluate(() =>
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async () => {
          throw new Error('denied');
        },
      },
    })
  );
  for (const name of ['Local thread reference', 'Local snapshot reference']) {
    const field = page.getByRole('textbox', { name });
    const block = field.locator('xpath=ancestor::*[contains(@class,"tmt-ui-command")][1]');
    await block.getByRole('button', { name: 'Copy reference' }).click();
    await expect(field).toBeFocused();
    expect(
      await field.evaluate((node: HTMLInputElement) => [node.selectionStart, node.selectionEnd])
    ).toEqual([0, (await field.inputValue()).length]);
    await expect(block.getByRole('status')).toContainText('Copy the selected reference manually');
  }
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    for (const width of [1440, 390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      const directory = process.env.TMT_COMMAND_CAPTURE_DIR;
      if (directory && width !== 320) {
        await mkdir(directory, { recursive: true });
        await page.screenshot({
          path: `${directory}/office-references-${width}-${theme}.png`,
          fullPage: true,
        });
      }
    }
  }
});
