import { expect, test, type Page } from '@playwright/test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { build } from 'vite';
import { longIdentity } from './settings-identities-source.js';

const origin = 'http://identities.test';
const assets = new URL('../../../rust/tmt-remote/assets/', import.meta.url);
let directory = '',
  harness = '',
  html = '',
  css = '';
test.beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), 'settings-identities-'));
  await build({
    root: new URL('..', import.meta.url).pathname,
    logLevel: 'silent',
    build: {
      outDir: directory,
      emptyOutDir: true,
      minify: false,
      lib: {
        entry: new URL('./settings-identities-harness.ts', import.meta.url).pathname,
        formats: ['es'],
        fileName: () => 'harness.js',
      },
      rollupOptions: { output: { inlineDynamicImports: true } },
    },
  });
  harness = await readFile(join(directory, 'harness.js'), 'utf8');
  html = (await readFile(new URL('tmt-settings.html', assets), 'utf8')).replace(
    '<script type="module" src="/sdk/shell.js"></script>',
    '<script type="module" src="/harness.js"></script>',
  );
  css =
    (await readFile(
      new URL('../../../../../design/browser-ui/generated/static.css', import.meta.url),
      'utf8',
    )) +
    (await readFile(new URL('pages.css', assets), 'utf8')) +
    (await readFile(new URL('tmt-settings.css', assets), 'utf8'));
});
test.afterAll(async () => {
  if (directory) await rm(directory, { recursive: true, force: true });
});
async function open(page: Page, state: string, width = 1440, theme = 'light') {
  await page.setViewportSize({ width, height: 1050 });
  await page.addInitScript((theme) => localStorage.setItem('tmt-ui-theme', theme), theme);
  await page.route(`${origin}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      contentType:
        path === '/harness.js'
          ? 'text/javascript'
          : path === '/sdk/pages.css'
            ? 'text/css'
            : 'text/html',
      body: path === '/harness.js' ? harness : path === '/sdk/pages.css' ? css : html,
    });
  });
  await page.goto(`${origin}/settings/?state=${state}#identities`);
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`injected identity table and selected detail at ${width} ${theme}`, async ({ page }) => {
      await open(page, 'injected', width, theme);
      await expect(
        page.getByRole('heading', { name: 'Sample identities', exact: true }),
      ).toBeVisible();
      await expect(page.locator('#tmt-settings-note')).toBeHidden();
      await expect(page.locator('.tmt-settings-identity-notice')).toContainText('not this machine');
      const names = page.locator('.tmt-settings-identity-name');
      await expect(names).toHaveText(['core-1', 'tmt-lead', longIdentity, 'ux-1']);
      await expect(page.locator('tbody')).toContainText('Codex');
      await expect(page.locator('tbody')).toContainText('Claude');
      await expect(page.locator('tbody')).toContainText('Not reported');
      await expect(names.first()).toHaveAttribute('aria-current', 'true');
      const selected = page.locator('tr[data-selected]');
      await expect(selected).toHaveCount(1);
      await expect(selected).toHaveCSS('outline-style', 'solid');
      await expect(selected).toHaveCSS('outline-width', '1px');
      expect(await selected.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(
        'rgba(0, 0, 0, 0)',
      );
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
      const noticeBox = await page.locator('.tmt-settings-identity-notice').boundingBox();
      const gridBox = await page.locator('.tmt-settings-grid').boundingBox();
      expect(noticeBox!.width).toBeCloseTo(gridBox!.width, 0);
      const labelsFit = await page
        .locator('tbody .tmt-ui-chip-label')
        .evaluateAll((labels) => labels.every((label) => label.scrollWidth <= label.clientWidth));
      expect(labelsFit).toBe(true);
      const lifetimeFits = await page.locator('tbody td:nth-child(2)').evaluateAll((cells) =>
        cells.every((cell) => {
          const text = document.createRange();
          text.selectNodeContents(cell);
          return text.getClientRects().length === 1;
        }),
      );
      expect(lifetimeFits).toBe(true);

      if (process.env.TMT_IDENTITIES_CAPTURE_DIR) {
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({
          path: join(
            process.env.TMT_IDENTITIES_CAPTURE_DIR,
            `identities-injected-${width}-${theme}.png`,
          ),
          fullPage: true,
        });
      }
      const before = await page.evaluate(() => JSON.stringify(localStorage));
      await names.nth(2).focus();
      await page.keyboard.press('Enter');
      await expect(names.nth(2)).toBeFocused();
      await expect(names.nth(2)).toHaveAttribute('aria-current', 'true');
      await expect(selected).toHaveCount(1);
      const detail = page.locator('.tmt-settings-identity-detail');
      await expect(detail.getByRole('heading')).toHaveText(longIdentity);
      await expect(detail).toContainText('Temporary');
      await expect(detail).toContainText('Codex');
      await expect(detail).toContainText('Offline');
      const tableBox = await page.locator('table').boundingBox(),
        detailBox = await detail.boundingBox();
      if (width === 390) expect(detailBox!.y).toBeGreaterThan(tableBox!.y + tableBox!.height);
      if (process.env.TMT_IDENTITIES_CAPTURE_DIR)
        await page.screenshot({
          path: join(
            process.env.TMT_IDENTITIES_CAPTURE_DIR,
            `identities-selected-${width}-${theme}.png`,
          ),
          fullPage: true,
        });
      await names.nth(3).click();
      await expect(detail).toContainText('Saved');
      await expect(detail).toContainText('Not reported');
      expect(await page.evaluate(() => window.identitySourceReads)).toBe(1);
      expect(await page.evaluate(() => JSON.stringify(localStorage))).toBe(before);
      await expect(page.locator('html')).toHaveJSProperty('scrollWidth', width);
    });
for (const state of ['unavailable', 'empty', 'refused', 'failed', 'observed'])
  test(`identity source state ${state} stays truthful`, async ({ page }) => {
    await open(page, state);
    if (state === 'failed') {
      await expect(page.locator('#tmt-settings-note')).toHaveText('Settings could not be read.');
      await expect(page.locator('#tmt-settings-section')).toBeEmpty();
    } else if (state === 'observed') {
      await expect(page.locator('#tmt-settings-identities-title')).toBeVisible();
      await expect(page.locator('.tmt-settings-identity-notice')).toContainText(
        'Read-only identity records',
      );
      await expect(page.locator('.tmt-settings-identity-name')).toHaveCount(4);
      await expect(page.locator('#tmt-settings-note')).toBeHidden();
    } else {
      await expect(page.locator('#tmt-settings-section')).toContainText(
        state === 'empty'
          ? 'No identities yet.'
          : state === 'refused'
            ? 'Permission refused for identity inventory.'
            : 'Identity inventory unavailable',
      );
      await expect(page.locator('tbody')).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Copy tmt ls' })).toBeVisible();
    }
    expect(await page.evaluate(() => window.identitySourceReads)).toBe(1);
  });
