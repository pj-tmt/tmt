import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { text } from '../src/strings.js';

test('the actual isolated Worker admits the shared shell values and refuses malformed raw updates', async ({
  page,
}) => {
  const corpus = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/shell-v1.json', import.meta.url), 'utf8'),
  );
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/');
  const result = await page.evaluate(async (cases) => {
    const path = '/test/shell-browser.tsx';
    return (await import(path)).checkCases(cases);
  }, corpus.cases);
  const expected = corpus.cases
    .filter((c: { action?: string }) => !c.action)
    .map((c: { name: string; valid: boolean; document: unknown }) => ({
      name: c.name,
      accepted: c.valid,
      ...(c.valid ? { shell: c.document, source: '' } : {}),
    }));
  expect(result).toEqual(expected);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`shell opening uses a neutral notice without an HTML frame or source controls ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.route('**/src/main.tsx', (route) =>
        route.fulfill({
          contentType: 'text/javascript',
          body: 'import { mount } from "/test/shell-browser.tsx"; mount();',
        }),
      );
      await page.goto('/');
      await expect(page.getByText(text.shellPageNotice, { exact: true })).toBeVisible();
      await expect(page.locator('header')).toContainText('Release shell');
      await expect(page.locator('iframe')).toHaveCount(0);
      await expect(page.locator('code')).toHaveText(
        'tmt colab shell read 10000000-0000-4000-8000-000000000001',
      );
      await expect(page.getByRole('button', { name: 'Source', exact: true })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Copy', exact: true })).toBeVisible();
      const notice = await page.locator('.shell-notice').boundingBox();
      expect(notice!.width).toBeLessThanOrEqual(1080);
      const command = await page.locator('code').boundingBox();
      expect(command!.x).toBeGreaterThanOrEqual(20);
      if (width === 1440 && theme === 'light') {
        await page.evaluate(() => {
          Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: {
              writeText: async (value: string) => {
                document.body.dataset.copiedCommand = value;
              },
            },
          });
        });
        await page.getByRole('button', { name: 'Copy', exact: true }).click();
        await expect(page.locator('body')).toHaveAttribute(
          'data-copied-command',
          'tmt colab shell read 10000000-0000-4000-8000-000000000001',
        );
        await expect(page.getByText(text.shellCommandCopied, { exact: true })).toBeVisible();
      }
      await expect(page.locator('body')).not.toContainText('Untitled page');
    });
  }

test('an editor reader link shows the shell notice as read-only, never a Source control', async ({
  page,
}) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: 'import { mountReader } from "/test/shell-browser.tsx"; mountReader();',
    }),
  );
  await page.goto('/');
  await expect(page.getByText(text.shellPageNotice, { exact: true })).toBeVisible();
  await expect(page.locator('.chip')).toHaveText(text.readerOnly);
  await expect(page.getByTestId('source-toggle')).toHaveCount(0);
  await expect(page.locator('code')).toHaveText(
    'tmt colab shell read 10000000-0000-4000-8000-000000000001',
  );
  await expect(page.getByRole('button', { name: 'Copy', exact: true })).toHaveCount(0);
  await expect(page.locator('iframe')).toHaveCount(0);
});
