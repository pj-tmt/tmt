import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';

const flipped = readFileSync(
  new URL('../../../contracts/vectors/ed25519-829.jsonl', import.meta.url),
  'utf8',
)
  .trim()
  .split('\n')
  .map((line) => JSON.parse(line))
  .find((row) => row.name === 'mixed-A1-R3');

for (const entry of ['/', '/r/abcd/x/colab/', '/r/abcd/x/colab/read'])
  test(`native-policy divergence refuses startup at ${entry}`, async ({ page, context }) => {
    let opened = 0;
    await context.route('**/sdk/remote-v1.js', (route) => {
      opened++;
      return route.abort();
    });
    if (entry.endsWith('/read'))
      await context.route('**/r/abcd/x/colab/read', async (route) => {
        const response = await route.fetch();
        await route.fulfill({
          response,
          body: (await response.text()).replace('/src/main.tsx', '/src/reader-main.tsx'),
        });
      });
    await page.addInitScript((signature) => {
      const verify = crypto.subtle.verify.bind(crypto.subtle);
      crypto.subtle.verify = (...args) => {
        const bytes = args[2] as Uint8Array;
        const encoded = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
        return encoded === signature ? Promise.resolve(true) : verify(...args);
      };
    }, flipped.signature);
    await page.goto(entry);
    await expect(page.getByTestId('unsupported-browser')).toBeVisible();
    await expect(
      page.getByTestId('unsupported-browser').getByRole('heading', { name: text.browserUpdate }),
    ).toBeVisible();
    await expect(page.getByRole('alert')).toContainText(text.browserUpdateBody);
    await expect(page.getByRole('button')).toHaveCount(0);
    await expect(page.locator('iframe')).toHaveCount(0);
    await expect(page.getByRole('link')).toHaveCount(0);
    expect(opened).toBe(0);
    if (entry === '/')
      for (const width of [1440, 390])
        for (const colorScheme of ['light', 'dark'] as const) {
          await page.setViewportSize({ width, height: 900 });
          await page.emulateMedia({ colorScheme });
          await page.screenshot({
            path: capturePath(`unsupported-browser-${width}-${colorScheme}.png`),
            fullPage: true,
          });
        }
  });

test('native-policy capability probe permits normal startup and page opening', async ({ page }) => {
  await page.goto('/');
  const link = page.getByRole('link', { name: 'A shared page', exact: true });
  await expect(link).toBeVisible();
  await expect(page.getByTestId('unsupported-browser')).toHaveCount(0);
  await link.click();
  await expect(page.locator('iframe').first()).toBeVisible();
});
