import { expect, test } from '@playwright/test';
import { text } from '../src/strings.js';
import { mkdir } from 'node:fs/promises';

test('verified bearer values copy exactly and remain selectable on clipboard denial', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.goto('/test/command-browser.html');
  await page.getByRole('button', { name: 'Create link', exact: true }).click();
  await page.getByRole('button', { name: 'Confirm create link', exact: true }).click();
  const block = page.locator('.bearer .tmt-ui-command');
  const link = await page.getByLabel('Share link', { exact: true }).inputValue();
  expect(new URL(link).pathname).toBe('/read/22222222-2222-4222-8222-222222222222');
  expect(new URL(link).hash).toContain('seed=');
  const copy = page.getByRole('button', { name: 'Copy share link' });
  await copy.focus();
  await page.keyboard.press('Enter');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(link);
  await expect(block.getByRole('status')).toHaveText('Copied.');
  await page.evaluate(() =>
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async () => {
          throw new Error('denied');
        },
      },
    }),
  );
  await copy.click();
  await expect(block.getByRole('status')).toHaveText(
    'Clipboard unavailable. Select and copy the displayed link.',
  );
  await page.getByLabel('Share link').evaluate((node: HTMLInputElement) => node.select());
  expect(
    await page
      .getByLabel('Share link')
      .evaluate((node: HTMLInputElement) => [node.selectionStart, node.selectionEnd]),
  ).toEqual([0, link.length]);
  expect(
    await block.evaluate((node) => {
      const labels = [...node.querySelectorAll('label')];
      return labels.every((label, index) => {
        const input = label.querySelector('input')!;
        const text = document.createRange();
        text.selectNode(label.firstChild!);
        const labelRect = text.getBoundingClientRect();
        const value = input.getBoundingClientRect();
        const previous = labels[index - 1]?.querySelector('input')?.getBoundingClientRect();
        return (
          value.top > labelRect.bottom + 2 && (!previous || labelRect.top > previous.bottom + 2)
        );
      });
    }),
  ).toBe(true);
  expect(
    await page.getByLabel('Share link').evaluate((node) => {
      const css = getComputedStyle(node);
      return (
        parseFloat(css.outlineWidth) > 0 &&
        parseFloat(css.outlineOffset) + parseFloat(css.outlineWidth) <= 0
      );
    }),
  ).toBe(true);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    for (const width of [1440, 390, 320]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      expect(
        await block.evaluate((node) => {
          const css = getComputedStyle(node);
          return [css.borderTopWidth, css.borderRadius, css.boxShadow];
        }),
      ).toEqual(['1px', '0px', 'none']);
      const directory = process.env.TMT_COMMAND_CAPTURE_DIR;
      if (directory && width !== 320) {
        await mkdir(directory, { recursive: true });
        await page.screenshot({
          path: `${directory}/colab-share-${width}-${theme}.png`,
          fullPage: true,
        });
      }
    }
  }
});

test('long literal commands wrap within a narrow page and copy every byte', async ({
  page,
  context,
}) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto('/test/command-browser.html');
  const expected = await page.evaluate(async () => {
    const fixtureUrl = '/test/command-browser.tsx';
    const fixture = await import(fixtureUrl);
    fixture.mountLong();
    return fixture.longCommand;
  });
  await page.getByRole('button', { name: 'Copy', exact: true }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(expected);
  await expect(page.locator('.tmt-ui-command-feedback')).toHaveText('Copied.');
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(320);
  const block = page.locator('.tmt-ui-command');
  expect(await block.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
});

for (const screen of ['reconnect-error', 'reconnect-terminal'] as const) {
  test(`${screen} card keeps the complete reconnect command sentence`, async ({ page }) => {
    await page.goto('/');
    await page.evaluate(async (screen) => {
      const fixtureUrl = '/test/chrome-browser.tsx';
      await (await import(fixtureUrl)).mount(screen);
    }, screen);
    const card = page.locator('#chrome-fixture .tmt-ui-notice');
    await expect(card).toBeVisible();
    await expect(card.locator('p')).toHaveText(text.reconnectFailed);
    await expect(card.locator('code.tmt-ui-code')).toHaveText('tmt remote pair');
    await expect(card.locator('[data-failure-reference]')).toHaveCount(0);
    const directory = process.env.TMT_COMMAND_CAPTURE_DIR;
    if (directory) {
      await mkdir(directory, { recursive: true });
      for (const theme of ['light', 'dark']) {
        await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: 900 });
          expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
          await page.screenshot({ path: `${directory}/colab-${screen}-${width}-${theme}.png` });
        }
      }
    }
  });
}
