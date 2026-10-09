import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

async function mount(page: Page, reader = false, unknown = false) {
  await page.goto('/');
  await page.evaluate(
    async ({ reader, unknown }) => {
      const path = '/test/page-layout-browser.tsx';
      const fixture = await import(path);
      await fixture[reader ? 'mountReader' : 'mount']('<h1>Author fixture</h1>', {
        ...(!unknown ? { originalAuthor: 'Alice <b>creator</b>' } : {}),
        publisherAgent: 'Bob latest publisher',
      });
    },
    { reader, unknown },
  );
  await expect(
    page.frameLocator('iframe:visible').getByRole('heading', { name: 'Author fixture' }),
  ).toBeVisible();
}
async function info(page: Page, reader: boolean) {
  if (reader) await page.locator('.reader-information:visible summary').click();
  else await (await pageAction(page, 'About this page')).click();
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`original author and latest publisher stay separate ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await mount(page);
      const header = page.locator('.tmt-ui-header:visible');
      await expect(header.locator('.tmt-ui-caption')).toHaveText('By Alice <b>creator</b>');
      await expect(header.locator('.tmt-ui-caption b')).toHaveCount(0);
      await expect(header).toHaveCSS('height', width === 390 ? '82px' : '56px');
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await info(page, false);
      const attribution = page.locator('.page-drawer[data-panel=about]:visible .page-attribution');
      await expect(attribution).toContainText('Original authorAlice <b>creator</b>');
      await expect(attribution).toContainText('Latest publisherBob latest publisher');
      const captures = process.env.COLAB_AUTHOR_CAPTURE_DIR;
      if (captures) {
        mkdirSync(captures, { recursive: true });
        await page.screenshot({ path: `${captures}/${width}-${theme}-owner.png` });
      }
      await page.evaluate(async () => {
        const path = '/test/page-layout-browser.tsx';
        (await import(path)).updatePublisher();
      });
      await expect(attribution).not.toContainText('Latest publisher');
      await expect(header.locator('.tmt-ui-caption')).toHaveText('By Alice <b>creator</b>');
      await mount(page, false, true);
      await expect(header.locator('.tmt-ui-caption')).toHaveCount(0);
      await info(page, false);
      await expect(page.locator('.page-attribution:visible')).toContainText(
        'Latest publisherBob latest publisher',
      );
      await expect(page.locator('.page-attribution:visible')).toContainText(
        'Original authorUnknown author',
      );
      await mount(page, true);
      await expect(header.locator('.tmt-ui-caption')).toHaveText('By Alice <b>creator</b>');
      await info(page, true);
      await expect(
        page.locator('.reader-information-panel:visible .page-attribution'),
      ).toContainText('Latest publisherBob latest publisher');
      if (captures) await page.screenshot({ path: `${captures}/${width}-${theme}-reader.png` });
      await expect(page.getByRole('button', { name: 'Ask' })).toHaveCount(0);
    });
  }
