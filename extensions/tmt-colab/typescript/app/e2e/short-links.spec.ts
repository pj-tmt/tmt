import { expect, test } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
const fixture = '/test/short-links-browser.tsx';
test('an old short link opens a chooser after a new page collides and never guesses a page', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678'), fixture);
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await expect(page.getByRole('link', { name: /Original proposal/ })).toBeVisible();
  await expect(page.getByRole('link', { name: /Later proposal/ })).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(page.locator('#short-fixture header')).toHaveCount(1);
  const directory = process.env.COLAB_SHORT_LINK_CAPTURE_DIR;
  if (directory) {
    await mkdir(directory, { recursive: true });
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
        await page.screenshot({
          path: `${directory}/chooser-${width}-${theme}.png`,
          fullPage: true,
        });
      }
    }
  }
  await page.getByRole('link', { name: /Original proposal/ }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/12345678-0000-4000-8000-000000000001/);
});
test('a unique prefix canonicalizes to the full page route and an unknown prefix fails visibly', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678', false), fixture);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/12345678-0000-4000-8000-000000000001/);
  await page.evaluate(async (path) => (await import(path)).mount('99999999'), fixture);
  await expect(page.getByRole('alert')).toContainText('Page unavailable');
  await expect(page.locator('#short-fixture iframe')).toHaveCount(0);
});

test('deleting the original page reserves old prefixes and never exposes its title or rebinds its link', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678', false, true), fixture);
  await expect(page.getByRole('heading', { name: 'This page was deleted' })).toBeVisible();
  await expect(page.getByText('Original proposal')).toHaveCount(0);
  await expect(page.locator('[aria-disabled="true"]')).toContainText('Deleted page');
  await page.evaluate(async (path) => (await import(path)).mount('12345678', true, true), fixture);
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await expect(page.getByRole('link', { name: /Original proposal/ })).toHaveCount(0);
  await expect(page.getByRole('link', { name: /Later proposal/ })).toBeVisible();
  await expect(page.locator('[aria-disabled="true"]')).toContainText('Deleted page');
  await expect(page.locator('#short-fixture iframe')).toHaveCount(0);
});
