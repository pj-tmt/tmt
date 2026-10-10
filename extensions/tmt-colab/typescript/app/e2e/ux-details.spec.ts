import { expect, test, type Page } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { pageAction } from '../test/page-actions.js';

const captures = process.env.COLAB_DETAIL_CAPTURE_DIR;
const native = process.env.COLAB_DETAIL_NATIVE_DIR;
const phase = process.env.COLAB_DETAIL_CAPTURE_PHASE;
const items = process.env.COLAB_DETAIL_CAPTURE_ITEMS?.split(',');
const skill = readFileSync(new URL('../../../skills/tmt-colab/SKILL.md', import.meta.url), 'utf8');
const style = skill.match(/<style>[\s\S]*?<\/style>/)?.[0];
if (!style) throw new Error('The author starter is missing.');
async function fixture(page: Page, name: string, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ name, method, argument }) => {
      const path = `/test/${name}.tsx`;
      return (await import(path))[method](argument);
    },
    { name, method, argument },
  );
}

// Capture-only matrix: ordinary behavioral checks remain in ask-again/live specs.
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`detail evidence ${width} ${theme}`, async ({ page }) => {
      test.skip(!captures || !native, 'Set the detail capture and native response directories.');
      mkdirSync(captures!, { recursive: true });
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const shot = (item: string) =>
        !items || items.includes(item.split('-')[0])
          ? page.screenshot({ path: `${captures}/${item}-${width}-${theme}.png` })
          : Promise.resolve();

      await page.goto('/');
      await fixture(page, 'ask-page-browser', 'mount');
      await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
      const input = page.getByRole('combobox', { name: 'Message', exact: true });
      await input.fill('Review this page.');
      await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
      await shot('1-ready-send');

      await page.goto('/');
      await fixture(
        page,
        'page-layout-browser',
        'mount',
        `${style}<h1>Review notes</h1><p class="muted">Secondary text explains the next step.</p><h2>Release readiness</h2><p>Body text carries the main message.</p><h3>Verification</h3><p>Check the changed behavior before release.</p>`,
      );
      const frame = page.frameLocator('#layout-fixture iframe');
      await expect(frame.getByRole('heading', { name: 'Review notes' })).toBeVisible();
      if (phase === 'after') {
        const sizes = await frame
          .locator('body')
          .evaluate((body) =>
            ['h1', 'h2', 'h3'].map((tag) =>
              parseFloat(getComputedStyle(body.querySelector(tag)!).fontSize),
            ),
          );
        expect(sizes[0]).toBeGreaterThan(sizes[1]);
        expect(sizes[1]).toBeGreaterThan(sizes[2]);
        if (theme === 'dark')
          await expect(frame.locator('.muted')).not.toHaveCSS('color', 'rgb(176, 176, 176)');
      }
      await shot('2-secondary-text');
      await shot('3-heading-scale');

      const css = JSON.parse(readFileSync(`${native}/css.json`, 'utf8'));
      await page.route('**/detail-guidance/assets/chrome.css', (route) => route.fulfill(css));
      for (const state of ['private', 'owner']) {
        const response = JSON.parse(readFileSync(`${native}/${state}.json`, 'utf8'));
        await page.route('**/detail-guidance/', (route) => route.fulfill(response));
        await page.goto('/detail-guidance/');
        await expect(page.locator('.guidance-card')).toBeVisible();
        if (phase === 'after') {
          await expect(page.locator('.guidance-detail')).not.toContainText('This colab');
          await expect(page.locator('.guidance-detail')).not.toContainText('corepack');
          if (state === 'owner') {
            await expect(page.locator('.tmt-ui-notice-heading')).toHaveText(
              'Update Colab to open this page',
            );
            await expect(page.locator('.tmt-ui-notice-mark')).toContainText('Update needed');
          }
        }
        await shot(`4-guidance-${state}`);
        await page.unroute('**/detail-guidance/');
      }

      await page.goto('/');
      await fixture(page, 'ask-again-browser', 'mount', { surface: 'chat', mode: 'held' });
      const multi = page.getByRole('combobox', { name: 'Message', exact: true });
      await expect(page.locator('.annotation-status-row')).not.toContainText('Checking');
      await multi.fill('@alpha @beta Review this page.');
      await multi.press('Enter');
      await expect(page.getByTestId('ask-state')).toHaveCount(2);
      if (phase === 'after')
        await expect(page.getByTestId('ask-state').last()).toHaveText('Waiting for beta');
      await shot('5-recipient-status');

      await page.goto('/');
      await fixture(page, 'ask-page-browser', 'mount', { exportAttachments: true });
      await (await pageAction(page.locator('#ask-page-fixture'), 'Export page')).click();
      await expect(page.getByRole('button', { name: 'Download page.html' })).toBeEnabled();
      if (phase === 'after')
        await expect(page.getByRole('button', { name: 'Close export', exact: true })).toHaveCount(
          0,
        );
      await shot('6-export-panel');

      await page.goto('/');
      await fixture(page, 'ask-page-browser', 'mount', { attachments: true });
      const quote = page.frameLocator('#ask-page-fixture iframe').locator('#selected');
      await expect(quote).toBeVisible();
      await quote.evaluate((node) => {
        const selection = getSelection()!;
        const range = document.createRange();
        range.selectNodeContents(node);
        selection.removeAllRanges();
        selection.addRange(range);
      });
      await page.getByTestId('selection-ask').click();
      await page.getByRole('combobox', { name: 'Message', exact: true }).fill('Review this image.');
      const chooser = page.waitForEvent('filechooser');
      await page.getByRole('button', { name: 'Attach files' }).click();
      await (
        await chooser
      ).setFiles({
        name: 'dot.webp',
        mimeType: 'image/webp',
        buffer: Buffer.from('UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AAAAAA', 'base64'),
      });
      await page.getByRole('button', { name: 'Send', exact: true }).click();
      const row = page.getByTestId('message-attachment');
      await expect(row).toBeVisible();
      if (phase === 'after') {
        const geometry = await row.evaluate((row) => {
          const name = row
            .closest('.conversation-body')!
            .querySelector('.comment-body')!
            .getBoundingClientRect();
          const actions = row.querySelector('.attachment-thumbnail')!.getBoundingClientRect();
          return {
            name: name.toJSON(),
            actions: actions.toJSON(),
            width: row.clientWidth,
            scroll: row.scrollWidth,
          };
        });
        expect(Math.abs(geometry.actions.left - geometry.name.left)).toBeLessThanOrEqual(1);
        expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
      }
      await shot('7-thread-attachment');
    });
