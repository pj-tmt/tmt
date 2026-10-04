import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';

function source(long: boolean) {
  return `<!doctype html><style>
  :root{color-scheme:light dark}body{margin:0;font:16px/1.7 system-ui;background:#f6f7f9;color:#263148}
  main{max-width:1040px;margin:auto;padding:clamp(24px,5vw,64px)}h1{font-size:clamp(32px,4vw,52px);line-height:1.15;letter-spacing:-.04em;margin:16px 0 24px}
  .eyebrow{font:12px system-ui;letter-spacing:.12em;color:#5b6b82;text-transform:uppercase}section{padding:32px 0;border-top:1px solid #d7dee8}h2{font-size:24px;line-height:1.3}
  blockquote{margin:24px 0;padding:16px 24px;border-left:3px solid #466dd6;background:#eaf0fc}
  @media(prefers-color-scheme:dark){body{background:#171b24;color:#d6ddea}.eyebrow{color:#a1aec4}section{border-color:#343d4e}blockquote{background:#252f45}}
  </style><main><p class="eyebrow">Product notebook · October 2026</p><h1>Notes on the next release</h1><p>Small changes can make a shared page feel natural. Keep the content close, and bring the tools into view when they are needed.</p><blockquote id="quote">The page should feel like a page.</blockquote>${Array.from({ length: long ? 9 : 1 }, (_, i) => `<section><p class="eyebrow">${String(i + 1).padStart(2, '0')} / Working notes</p><h2>${['A quieter reading space', 'Conversations beside the work', 'A clear path back'][i % 3]}</h2><p>Readers can move through the document without leaving its context. Comments and agent conversations stay available from the Colab bar, while the author controls the page itself.</p><p>Keep the boundaries simple. The renderer fills the available space; tools open alongside it and close without losing the work in progress.</p></section>`).join('')}</main>`;
}
async function mount(page: Page, long: boolean) {
  await page.goto('/');
  await page.evaluate(async (html) => {
    const path = '/test/page-layout-browser.tsx';
    (await import(path)).mount(html);
  }, source(long));
  await expect(page.locator('.page-bar .status')).toContainText('Live preview');
  await expect(page.frameLocator('iframe').locator('h1')).toBeVisible();
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`frameless ${width}px ${theme}: short page, mid-scroll and drawers`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await mount(page, false);
      const bar = page.locator('.page-bar'),
        frame = page.locator('iframe');
      await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
      await expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer');
      await expect(page.getByText('Page content', { exact: true })).toHaveCount(0);
      await expect(page.locator('.canvas')).toHaveCSS('border-top-width', '0px');
      await expect(page.locator('.canvas')).toHaveCSS('box-shadow', 'none');
      const rect = await frame.boundingBox();
      expect(rect?.x).toBe(0);
      const contentWidth = await page.evaluate(() => document.documentElement.clientWidth);
      expect(rect?.width).toBe(contentWidth);
      expect(rect?.y).toBe(56);
      expect(rect?.height).toBeGreaterThanOrEqual(844);
      await expect(frame).toHaveAttribute('scrolling', 'no');
      await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await expect(bar).toHaveCSS('flex-wrap', 'nowrap');
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-short.png` });
      if (width === 390) {
        await expect(
          page.getByRole('heading', { name: 'Release notes', exact: true }),
        ).toBeVisible();
        await page.getByRole('button', { name: 'More page actions' }).click();
        await expect(page.locator('.page-menu-meta')).toContainText('local · Studio Mac');
        await expect(page.locator('.page-menu-meta')).toContainText('Private');
        await expect(
          page.getByRole('button', { name: 'Change color theme' }).locator('.theme-label'),
        ).toHaveText(`Theme: ${theme}`);
        await page.screenshot({ path: `/tmp/1586-${width}-${theme}-menu.png` });
        await page.keyboard.press('Escape');
      }
      await mount(page, true);
      await expect.poll(async () => (await frame.boundingBox())?.height ?? 0).toBeGreaterThan(1500);
      await expect
        .poll(() =>
          page
            .frameLocator('iframe')
            .locator('html')
            .evaluate(
              (node) => node.scrollHeight <= node.ownerDocument.defaultView!.innerHeight + 1,
            ),
        )
        .toBe(true);
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-long-top.png` });
      await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight / 2));
      expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(500);
      expect(
        await page
          .frameLocator('iframe')
          .locator('html')
          .evaluate((node) => node.ownerDocument.defaultView!.scrollY),
      ).toBe(0);
      expect((await bar.boundingBox())?.y).toBe(0);
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-middle.png` });
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-long-scrolled.png` });
      const beforePanel = await frame.boundingBox(),
        scrollBeforePanel = await page.evaluate(() => window.scrollY);
      await expect(page.locator('.page-bar > .page-backend')).toHaveText('local · Studio Mac');
      if (width === 390) await page.getByRole('button', { name: 'More page actions' }).click();
      await page.getByTestId('comments-toggle').click();
      const comments = page.locator('.page-drawer[data-panel=comments]');
      await expect(comments).toBeVisible();
      expect((await frame.boundingBox())?.width).toBe(beforePanel?.width);
      expect((await frame.boundingBox())?.height).toBe(beforePanel?.height);
      expect(await page.evaluate(() => window.scrollY)).toBe(scrollBeforePanel);
      if (width === 390) {
        expect((await comments.boundingBox())?.width).toBe(390);
        expect((await comments.boundingBox())?.y).toBe(0);
      }
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-comments.png` });
      await comments.getByRole('button', { name: 'Close Comments', exact: true }).click();
      await expect(comments).not.toBeVisible();
      if (width === 390) {
        await expect(page.getByRole('button', { name: 'More page actions' })).toBeFocused();
        await page.getByRole('button', { name: 'More page actions' }).click();
      } else await expect(page.getByTestId('comments-toggle')).toBeFocused();
      await page.getByTestId('ask-toggle').click();
      const asks = page.locator('.page-drawer[data-panel=ask]');
      await expect(asks.getByTestId('ask-panel')).toBeVisible();
      await asks.getByRole('button', { name: 'Close Ask agent', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(asks).not.toBeVisible();
      if (width === 390) {
        await page.getByRole('button', { name: 'More page actions' }).click();
      }
      await page.getByRole('button', { name: 'Source', exact: true }).click();
      const editor = page.locator('.page-drawer[data-panel=source]');
      await expect(editor.getByLabel('Source', { exact: true })).toHaveValue(source(true));
      await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
      await editor.getByRole('button', { name: 'Close Source', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(editor).not.toBeVisible();
      if (width === 390)
        await expect(page.getByRole('button', { name: 'More page actions' })).toBeFocused();
      expect((await frame.boundingBox())?.width).toBe(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }

test('narrow desktop menu switches floating panels and keeps information readable', async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await mount(page, true);
  const frame = page.locator('iframe');
  await expect.poll(async () => (await frame.boundingBox())?.height ?? 0).toBeGreaterThan(1500);
  const before = await frame.boundingBox();
  await page.getByRole('button', { name: 'More page actions' }).click();
  await page.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(page.locator('.page-drawer[data-panel=source]')).toBeVisible();
  await page.getByRole('button', { name: 'More page actions' }).click();
  await page.getByTestId('comments-toggle').click();
  await expect(page.locator('.page-drawer[data-panel=comments]')).toBeVisible();
  await expect(page.locator('.page-drawer[data-panel=source]')).not.toBeVisible();
  await page.getByRole('button', { name: 'More page actions' }).click();
  await page.getByText('Page information', { exact: true }).click();
  const information = page.locator('.page-information p');
  await expect(information).toBeVisible();
  const box = (await information.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(900);
  await page.getByRole('button', { name: 'Close page actions' }).click();
  await expect(page.getByRole('button', { name: 'More page actions' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  expect((await frame.boundingBox())?.width).toBe(before?.width);
});
