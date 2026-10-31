import { expectAvailableDockWindow } from '../test/floating-panel.js';
import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { chatAction, commentsAction, pageAction } from '../test/page-actions.js';
import { expect, test } from '@playwright/test';
import type { Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { tokens, headerHeight } from './tokens.js';

/** The rail is the only column left of the page on a wide screen. */
const railWidth = Number.parseFloat(tokens.browser.metric['rail-width']!);

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
  await expect(page.locator('.tmt-ui-topbar:visible .status')).toContainText('Live');
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
      const bar = page.locator('.tmt-ui-topbar:visible'),
        title = bar.locator('.tmt-ui-topbar-title'),
        frame = page.locator('iframe');
      await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
      await expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer');
      await expect(page.getByText('Page content', { exact: true })).toHaveCount(0);
      await expect(page.locator('.canvas')).toHaveCSS('border-top-width', '0px');
      await expect(page.locator('.canvas')).toHaveCSS('box-shadow', 'none');
      // Inside the TMT web shell the page fills the main column beside the rail (the module's pages
      // nest in it)
      // and starts under the bar, plus the module menu's tab row on a phone and the page's status band.
      const rect = await frame.boundingBox();
      const column = (await page.locator('.tmt-ui-shell-main').boundingBox())!;
      const lead = await page.evaluate(() =>
        Number.parseFloat(
          getComputedStyle(document.querySelector('.tmt-ui-shell-bar')!).getPropertyValue(
            '--tmt-ui-shell-lead',
          ),
        ),
      );
      expect(lead).toBe(0);
      expect(rect?.x).toBe(column.x);
      expect(rect?.x).toBe(width < 480 ? 0 : railWidth);
      expect(rect?.width).toBe(column.width);
      expect(rect?.width).toBe(
        (await page.evaluate(() => document.documentElement.clientWidth)) - column.x,
      );
      const header = (await bar.boundingBox())!;
      const panel = (await page.locator('.work-band:visible').boundingBox())!;
      expect(rect?.y).toBe(header.y + header.height + lead + panel.height);
      expect(rect?.height).toBeGreaterThanOrEqual(900 - rect!.y);
      // The frame ends the document: a page taller than the room grows it (window scroll mode), so
      // the bottom is the frame's, not a fixed 900 that depends on the runner's fonts. The frame
      // reports its height asynchronously, so both sides are measured again until they settle.
      await expect
        .poll(async () => {
          const box = (await frame.boundingBox())!;
          const bottom = Math.max(900, Math.round(box.y + box.height));
          return (await page.evaluate(() => document.documentElement.scrollHeight)) - bottom;
        })
        .toBe(0);
      await expect(frame).toHaveAttribute('scrolling', 'no');
      await expect(frame).toHaveAttribute('data-scroll-mode', 'window');
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await expect(title).toHaveCSS('flex-wrap', 'nowrap');
      await page.screenshot({ path: test.info().outputPath(`1586-${width}-${theme}-short.png`) });
      const heading = page.getByRole('heading', { name: 'Release notes', exact: true });
      // The header holds no access chip at any width (#4116), so the title keeps its room.
      await expect(heading).toBeVisible();
      if (width < 480)
        await expect(title.locator('.tmt-ui-topbar-title-current')).toHaveAttribute(
          'title',
          'Release notes',
        );
      // The shell's rail carries the TMT mark; the bar keeps none (a phone's menu button opens the rail).
      await expect(bar.locator('.tmt-ui-mark')).toBeHidden();
      if (width >= 480)
        await expect(page.locator('.tmt-ui-shell-brand .tmt-brand-mark')).toBeVisible();
      else await expect(page.locator('.tmt-ui-shell-open:visible')).toBeVisible();
      // The bar draws no brand (the rail names the module); its title slot holds the page's one
      // heading and no title row repeats it.
      await expect(bar.locator('.tmt-ui-wordmark, .tmt-ui-brand')).toHaveCount(0);
      await expect(title.getByRole('heading', { level: 1 })).toHaveCount(1);
      await expect(page.locator('.colab-title-row')).toHaveCount(0);
      // The module and the connection identity stay in the title group's accessible name.
      await expect(title.locator('.colab-topbar-identity')).toHaveAttribute(
        'aria-label',
        'Colab · local · Studio Mac · Release notes',
      );
      // The connection words stay in the document for assistive technology; nothing shows.
      await expect(bar.locator('.page-connection')).toHaveText('Private · Live');
      await expect(bar.locator('.page-attention')).toHaveCount(0);
      // More alone at every width (Comments and Chat are the dock's); Agent status is a writer-only
      // More entry.
      await expect(bar.locator('.page-header-actions button')).toHaveCount(1);
      await page.getByRole('button', { name: 'More', exact: true }).click();
      // Theme is the shared bar's: More keeps no copy of it.
      await expect(page.getByRole('menuitemradio')).toHaveCount(0);
      await page.screenshot({ path: test.info().outputPath(`1586-${width}-${theme}-menu.png`) });
      await page.keyboard.press('Escape');
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
      await page.screenshot({
        path: test.info().outputPath(`1586-${width}-${theme}-long-top.png`),
      });
      await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight / 2));
      expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(500);
      expect(
        await page
          .frameLocator('iframe')
          .locator('html')
          .evaluate((node) => node.ownerDocument.defaultView!.scrollY),
      ).toBe(0);
      expect((await bar.boundingBox())?.y).toBe(0);
      await page.screenshot({ path: test.info().outputPath(`1586-${width}-${theme}-middle.png`) });
      await page.screenshot({
        path: test.info().outputPath(`1586-${width}-${theme}-long-scrolled.png`),
      });
      const beforePanel = await frame.boundingBox(),
        scrollBeforePanel = await page.evaluate(() => window.scrollY);
      await (await pageAction(page, 'Comments')).click();
      const comments = commentsRegion(page);
      await expect(comments).toBeVisible();
      expect(await page.evaluate(() => window.scrollY)).toBe(scrollBeforePanel);
      if (width === 1440) {
        // The wide card floats above the dock and does not reflow the page.
        await expect(comments).toHaveAttribute('data-mode', 'card');
        expect((await frame.boundingBox())?.width).toBe(beforePanel!.width);
        expect((await frame.boundingBox())?.height).toBe(beforePanel!.height);
        const card = (await comments.boundingBox())!;
        const dock = (await page.locator('.page-floating-actions:visible').boundingBox())!;
        expect(card.width).toBe(360);
        await expectAvailableDockWindow(page, comments);
        expect(Math.abs(dock.y - card.y - card.height - 16)).toBeLessThanOrEqual(1);
      } else {
        // The phone card floats above the dock with 12px side gutters; the page does not reflow.
        await expect(comments).toHaveAttribute('data-mode', 'sheet');
        expect((await frame.boundingBox())?.width).toBe(beforePanel?.width);
        expect((await frame.boundingBox())?.height).toBe(beforePanel?.height);
        const box = (await comments.boundingBox())!;
        const dock = (await page.locator('.page-floating-actions:visible').boundingBox())!;
        expect(box.x).toBe(12);
        expect(box.width).toBe(width - 24);
        await expectAvailableDockWindow(page, comments);
        expect(Math.abs(dock.y - box.y - box.height - 16)).toBeLessThanOrEqual(1);
      }
      await page.screenshot({
        path: test.info().outputPath(`1586-${width}-${theme}-comments.png`),
      });
      await commentsClose(page, 'empty').click();
      await expect(comments).not.toBeVisible();
      await expect(commentsAction(page)).toBeFocused();
      await (await pageAction(page, 'Chat')).click();
      const asks = page.locator('.tmt-ui-panel[data-panel=chat]');
      await expect(asks.getByTestId('chat-panel')).toBeVisible();
      await page.getByRole('button', { name: 'Close Chat', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(asks).not.toBeVisible();
      await (await pageAction(page, 'Source')).click();
      const editor = page.locator('.tmt-ui-drawer[data-panel=source]');
      await expect(editor.getByLabel('Page source', { exact: true })).toHaveValue(source(true));
      await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
      await editor.getByRole('button', { name: 'Close Source', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(editor).not.toBeVisible();
      await expect(page.getByRole('button', { name: 'More', exact: true })).toBeFocused();
      expect((await frame.boundingBox())?.width).toBe(column.width);
    });
  }

test('narrow desktop menus switch floating panels and keep About readable', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await mount(page, true);
  const frame = page.locator('iframe');
  await expect.poll(async () => (await frame.boundingBox())?.height ?? 0).toBeGreaterThan(1500);
  const before = await frame.boundingBox();
  await (await pageAction(page, 'Source')).click();
  await expect(page.locator('.tmt-ui-drawer[data-panel=source]')).toBeVisible();
  await (await pageAction(page, 'Comments')).click();
  await expect(commentsRegion(page)).toBeVisible();
  await expect(page.locator('.tmt-ui-drawer[data-panel=source]')).not.toBeVisible();
  await (await pageAction(page, 'About this page')).click();
  const information = page.locator('.tmt-ui-drawer[data-panel=about]');
  await expect(information).toBeVisible();
  const box = (await information.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(900);
  await page.getByRole('button', { name: 'Close About this page' }).click();
  await expect(page.getByRole('button', { name: 'More', exact: true })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  expect((await frame.boundingBox())?.width).toBe(before?.width);
});

for (const width of [1440, 390])
  test(`header tooltips, menu keys and System theme preserve the page at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: 'light' });
    await page.goto('/');
    await page.evaluate(async () => {
      const path = '/test/ask-page-browser.tsx';
      await (await import(path)).mount({ attachments: true });
    });
    await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
    const actions = page.getByRole('navigation', { name: 'Page actions' });
    // The header holds More alone at every width (Comments and Chat are the dock's); Agent status,
    // Files, Source and Export live in More.
    const names = [/^More$/];
    for (const name of names) {
      const button = actions.getByRole('button', { name });
      await expect(button).toBeInViewport();
      await button.hover();
      const tooltip = button.locator('..').locator('.tmt-ui-icon-action-tooltip');
      await expect(tooltip).toBeVisible();
      await expect(tooltip).toHaveText(name);
      await page.mouse.move(0, 899);
      await button.focus();
      await page.keyboard.press('Tab');
      await page.keyboard.press('Shift+Tab');
      await expect(button).toBeFocused();
      await expect(tooltip).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(tooltip).toBeHidden();
      await expect(button).toBeFocused();
    }
    // The dock opens Chat; Escape closes it and focus returns to the action that opened it.
    const dockChat = chatAction(page);
    await dockChat.press('Enter');
    const drawer = page.locator('.tmt-ui-panel[data-panel=chat]:not([hidden])');
    await expect(drawer).toBeVisible();
    const close = drawer.getByRole('button', { name: 'Close Chat', exact: true });
    await close.focus();
    const closeTooltip = close.locator('..').locator('.tmt-ui-icon-action-tooltip');
    await expect(closeTooltip).toBeVisible();
    // One Escape dismisses the tooltip and closes its containing drawer.
    await page.keyboard.press('Escape');
    await expect(drawer).toHaveCount(0);
    await expect(dockChat).toBeFocused();
    await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
    const frame = page.locator('#ask-page-fixture iframe');
    const render = await frame.getAttribute('data-render-id');
    // Theme is the shared bar's menu; the page's More keeps no copy of it.
    const theme = page.locator('.tmt-ui-topbar-common').getByRole('button', { name: /^Theme:/ });
    await theme.click();
    await page.getByRole('menuitemradio', { name: 'Dark', exact: true }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await theme.click();
    await page.getByRole('menuitemradio', { name: 'System', exact: true }).click();
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    await expect(page.locator('html')).toHaveCSS('color-scheme', 'light');
    await page.emulateMedia({ colorScheme: 'dark' });
    await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
    await expect(frame).toHaveAttribute('data-render-id', render!);
    await theme.click();
    await expect(page.getByRole('menuitemradio', { name: 'System', exact: true })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await page.keyboard.press('Escape');
    await expect(page.getByRole('menu')).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`Comments panel edge ${width}px ${theme}: its mode keeps header and page geometry`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/ask-page-browser.tsx';
        const fixture = await import(path);
        await fixture.mount();
        fixture.conversation({
          surface: 'thread',
          state: 'replied',
          message: 'Review this point.',
        });
      });
      const frame = page.locator('#ask-page-fixture iframe');
      const marker = page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]');
      await expect(marker).toBeVisible();
      // The page starts under the status band, which stays pinned under the bar: give it room and
      // keep it at the top, so the marker stays clear of both.
      const rowHeight = 0;
      await page.evaluate((top) => {
        document.querySelector<HTMLElement>('#ask-page-fixture .canvas')!.style.minHeight =
          '3000px';
        window.scrollTo(0, top);
      }, rowHeight);
      await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(rowHeight);
      const before = await frame.boundingBox();
      const scroll = await page.evaluate(() => window.scrollY);
      await (await pageAction(page, 'Comments')).click();
      const panel = commentsRegion(page);
      await expect(panel).toBeVisible();
      const edge = (await panel.boundingBox())!;
      // After close, the original page marker is reachable again.
      const frameAt = async () => {
        const box = await marker.boundingBox();
        return (
          !!box &&
          page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.tagName === 'IFRAME', {
            x: box.x + box.width - 2,
            y: box.y + box.height / 2,
          })
        );
      };
      if (width === 1440) {
        // The card has its own edge, stays above the dock and leaves the page width intact.
        await expect(panel).toHaveAttribute('data-mode', 'card');
        const dock = (await page
          .locator('#ask-page-fixture .page-floating-actions')
          .boundingBox())!;
        expect(edge.width).toBe(360);
        await expectAvailableDockWindow(page, panel);
        expect(Math.abs(dock.y - edge.y - edge.height - 16)).toBeLessThanOrEqual(1);
        await expect(panel).toHaveCSS('border-left-width', '1px');
        await expect(panel).toHaveCSS('border-right-width', '1px');
        expect((await frame.boundingBox())?.width).toBe(before!.width);
      } else {
        // The phone card keeps its side gutters and floats above the dock, over the page.
        await expect(panel).toHaveAttribute('data-mode', 'sheet');
        const dock = (await page
          .locator('#ask-page-fixture .page-floating-actions')
          .boundingBox())!;
        expect(edge.x).toBe(12);
        expect(edge.x + edge.width).toBe(width - 12);
        await expectAvailableDockWindow(page, panel);
        expect(Math.abs(dock.y - edge.y - edge.height - 16)).toBeLessThanOrEqual(1);
        await expect(panel).toHaveCSS('border-right-width', '1px');
        expect((await frame.boundingBox())?.width).toBe(before?.width);
        expect((await frame.boundingBox())?.height).toBe(before?.height);
      }
      await expect(page.locator('.frame-host:visible')).toHaveCSS('clip-path', 'none');
      const capture = process.env.COLAB_DRAWER_CAPTURE_DIR;
      if (capture) {
        mkdirSync(capture, { recursive: true });
        await page.screenshot({ path: `${capture}/comments-${width}-${theme}.png` });
      }
      expect(await page.evaluate(() => window.scrollY)).toBe(scroll);
      await commentsClose(page, 'discussion').click();
      await expect(panel).not.toBeVisible();
      await expect.poll(frameAt).toBe(true);
      expect((await frame.boundingBox())?.width).toBe(before?.width);
      await marker.click();
      await expect(page.getByTestId('comment-thread')).toBeVisible();
    });
    test(`export rows ${width}px ${theme}: named downloads and one close`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      const attachmentName = `LongExportAttachment${'x'.repeat(140)}.txt`;
      await page.evaluate(async (exportAttachmentName) => {
        const path = '/test/ask-page-browser.tsx';
        await (await import(path)).mount({ exportAttachments: true, exportAttachmentName });
      }, attachmentName);
      await (await pageAction(page, 'Export page')).click();
      const drawer = page.locator('.tmt-ui-drawer[data-panel=export][open]');
      await expect(drawer).toBeVisible();
      for (const name of ['page.html', 'conversations.json', 'conversations.md', 'manifest.json']) {
        const action = drawer.getByRole('button', { name: `Download ${name}`, exact: true });
        await expect(action).toBeEnabled();
        await expect(action).toHaveClass(/tmt-ui-action/);
        await expect(action).toHaveText('Download');
        await expect(action).toHaveAttribute('data-variant', 'secondary');
        const row = drawer
          .getByRole('list', { name: 'Page files', exact: true })
          .getByRole('listitem')
          .filter({ hasText: name });
        await expect(row).toContainText(name);
        await expect(row.locator('svg').first()).toHaveAttribute('aria-hidden', 'true');
        await expect(row.getByRole('button')).toHaveCount(1);
        await expect(action).toHaveCSS('border-top-width', '1px');
      }
      await expect(drawer.getByRole('button', { name: /^Close/ })).toHaveCount(1);
      await expect(drawer.getByRole('heading', { name: 'Page files', exact: true })).toBeVisible();
      await expect(drawer.getByRole('region', { name: 'Page files', exact: true })).toHaveCount(0);
      await expect(
        drawer.getByRole('list', { name: 'Page files', exact: true }).getByRole('listitem'),
      ).toHaveCount(4);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await test.info().attach(`export-rows-${width}-${theme}`, {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      const attachments = drawer.getByRole('list', { name: 'Attachments', exact: true });
      await attachments.scrollIntoViewIfNeeded();
      await expect(attachments.getByRole('listitem')).toHaveCount(4);
      for (const name of [attachmentName, 'gone.bin', 'revoked.bin', 'huge.bin']) {
        const row = attachments.getByRole('listitem').filter({ hasText: name });
        await expect(row.locator('.tmt-ui-list-label')).toHaveText(name);
        await expect(row.locator('.tmt-ui-list-description')).not.toBeEmpty();
        expect(
          await row.evaluate((node) => {
            const title = node.querySelector('.tmt-ui-list-title')!;
            const description = node.querySelector('.tmt-ui-list-description')!;
            const action = node.querySelector('button');
            const bounds = node.getBoundingClientRect();
            const box = title.getBoundingClientRect();
            return (
              title.scrollWidth <= title.clientWidth + 1 &&
              description.scrollWidth <= description.clientWidth + 1 &&
              box.left >= bounds.left &&
              box.right <= bounds.right &&
              (!action || box.right < action.getBoundingClientRect().left)
            );
          }),
          name,
        ).toBe(true);
        await expect(row.getByRole('button')).toHaveCount(name === attachmentName ? 1 : 0);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await test.info().attach(`export-attachments-${width}-${theme}`, {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      const pending = page.waitForEvent('download');
      await drawer.getByRole('button', { name: 'Download page.html', exact: true }).click();
      expect((await pending).suggestedFilename()).toBe('page.html');
      await expect(drawer.getByRole('status').first()).toContainText('Some files requested');
      await expect(
        drawer
          .getByRole('list', { name: 'Page files', exact: true })
          .getByRole('listitem')
          .first()
          .locator('.export-download > svg'),
      ).toHaveCount(1);
      await drawer.getByRole('button', { name: 'Close Export page', exact: true }).click();
      await expect(drawer).not.toBeVisible();
    });
  }

test('one-row fitting retains full labels, stable content and compact action authority', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  const title = 'A very long release title '.repeat(8);
  const deviceName = 'A very long device name '.repeat(8).trim();
  await page.evaluate(
    async ({ title, deviceName }) => {
      const path = '/test/page-layout-browser.tsx';
      (await import(path)).mount(
        '<h1>Stable content</h1>',
        { originalAuthor: 'Original writer', publisherAgent: 'Latest writer' },
        { title, deviceName },
      );
    },
    { title, deviceName },
  );
  const bar = page.locator('.tmt-ui-topbar:visible');
  const header = bar.locator('.tmt-ui-topbar-title');
  const heading = header.getByRole('heading', { name: title, exact: true });
  await expect(heading).toBeVisible();
  await expect(header.locator('.tmt-ui-caption')).toBeVisible();
  await expect(bar.locator('.tmt-ui-mark')).toBeHidden();
  const iframe = page.locator('#layout-fixture iframe');
  await expect(
    page.frameLocator('#layout-fixture iframe').getByRole('heading', { name: 'Stable content' }),
  ).toBeVisible();
  const frameHandle = await iframe.elementHandle();
  const titleHandle = await heading.elementHandle();
  for (const width of [800, 640, 390, 320, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(header.locator('.tmt-ui-caption'))[width <= 900 ? 'toBeHidden' : 'toBeVisible']();
    // The shell carries the mark at every width (the rail, or the phone's menu button), not the bar.
    await expect(bar.locator('.tmt-ui-mark')).toBeHidden();
    await expect(
      page.locator(
        width > 900 ? '.tmt-ui-shell-brand .tmt-brand-mark:visible' : '.tmt-ui-shell-open:visible',
      ),
    ).toBeVisible();
    await expect(header.locator('.tmt-ui-topbar-title-current')).toHaveAttribute('title', title);
    await expect(bar).toHaveCSS('height', `${headerHeight(width)}px`);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await iframe.evaluate((node, saved) => node === saved, frameHandle)).toBe(true);
    expect(await heading.evaluate((node, saved) => node === saved, titleHandle)).toBe(true);
    await expect(heading).toBeVisible();
    const actions = bar.getByRole('navigation', { name: 'Page actions' });
    await expect(actions.getByRole('button')).toHaveCount(1);
    const more = actions.getByRole('button', { name: 'More', exact: true });
    await more.press('ArrowDown');
    // Share… waits on page management and an empty page has no Files entry. Metadata inspection
    // remains reachable before Agent status, Source and Export page.
    await expect(actions.getByRole('menuitem', { name: /^Share…/ })).toBeDisabled();
    await expect(actions.getByRole('menuitem', { name: /^Files/ })).toHaveCount(0);
    await expect(actions.getByRole('menuitem', { name: 'Questions…', exact: true })).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(actions.getByRole('menuitem', { name: 'Creator', exact: true })).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(actions.getByRole('menuitem', { name: 'Source', exact: true })).toBeFocused();
    await expect(actions.getByRole('menuitem', { name: 'Export page', exact: true })).toBeEnabled();
    await page.keyboard.press('ArrowDown');
    await expect(actions.getByRole('menuitem', { name: 'Export page', exact: true })).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(
      actions.getByRole('menuitem', { name: 'Agent status', exact: true }),
    ).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(more).toBeFocused();
  }
  await page.evaluate(async () => {
    const path = '/test/page-layout-browser.tsx';
    (await import(path)).mount('<h1>Stable content</h1>', {}, {});
  });
  await expect(header.getByRole('heading', { level: 1 })).toHaveCount(1);
  await expect(header.locator('.tmt-ui-caption')).toHaveCount(0);
});
