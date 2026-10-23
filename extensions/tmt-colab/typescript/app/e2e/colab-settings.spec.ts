import { expect, test, type Page } from '@playwright/test';

async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('shells', '/settings');
  });
  return page.locator('#list-fixture');
}
for (const theme of ['light', 'dark'] as const)
  for (const width of [1586, 390])
    test(`reference defaults are examples, ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme });
      const app = await mount(page);
      await expect(app.getByRole('heading', { level: 1 })).toHaveText('Colab settings');
      await expect(app.locator('.tmt-ui-header')).toHaveAccessibleName('Colab · Colab settings');
      const panel = app.getByRole('region', { name: 'Reference defaults', exact: true });
      await expect(panel).toBeVisible();
      await expect(panel).toContainText('CLI managed');
      await expect(panel).toContainText(
        'An illustration of the current schema, not your live settings.',
      );
      await expect(panel.locator('dd')).toHaveText(['On', '1 per agent', 'Not set', '180 days']);
      await expect(app.getByRole('button', { name: 'Copy', exact: true })).toBeEnabled();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      await expect(panel.locator('input, select, button')).toHaveCount(0);
    });

test('an injected non-default source never becomes reference values', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/colab-settings-browser.tsx';
    await (
      await import(path)
    ).mount({
      open: false,
      wipLimit: { default: 4, agents: { pilot: 2 } },
      githubRepo: 'team/research',
      pageExpiryDays: null,
      sample: false,
    });
  });
  const app = page.locator('#settings-fixture');
  await expect(app.getByRole('region', { name: 'Current settings', exact: true })).toBeVisible();
  await expect(app.locator('dd')).toHaveText([
    'Off',
    '4 per agent · pilot 2',
    'team/research',
    'Never',
  ]);
  await expect(app.getByText('Reference defaults', { exact: true })).toHaveCount(0);
});

test('command copy keeps trusted success and refusal without running a command', async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          (window as unknown as { copied: string[] }).copied.push(text);
        },
      },
    });
    (window as unknown as { copied: string[] }).copied = [];
  });
  const app = await mount(page);
  const copy = app.getByRole('button', { name: 'Copy', exact: true });
  await copy.dispatchEvent('click');
  expect(await page.evaluate(() => (window as unknown as { copied: string[] }).copied)).toEqual([]);
  await copy.focus();
  await page.keyboard.press('Enter');
  await expect(app.getByRole('status')).toContainText('Copied');
  expect(await page.evaluate(() => (window as unknown as { copied: string[] }).copied)).toEqual([
    'tmt colab settings',
  ]);
  await page.evaluate(() => {
    navigator.clipboard.writeText = async () => {
      throw new Error('denied');
    };
  });
  await copy.click();
  await expect(app.getByRole('status')).toContainText('Clipboard unavailable');
});
