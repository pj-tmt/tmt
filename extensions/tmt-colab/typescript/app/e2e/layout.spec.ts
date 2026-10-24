import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { pageAction } from '../test/page-actions.js';
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
  await expect(page.locator('.tmt-ui-header:visible .status')).toContainText('Live');
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
      const bar = page.locator('.tmt-ui-header:visible'),
        frame = page.locator('iframe');
      await expect(frame).toHaveAttribute('sandbox', 'allow-scripts');
      await expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer');
      await expect(page.getByText('Page content', { exact: true })).toHaveCount(0);
      await expect(page.locator('.canvas')).toHaveCSS('border-top-width', '0px');
      await expect(page.locator('.canvas')).toHaveCSS('box-shadow', 'none');
      // Inside the TMT web shell the page fills the main column beside the rail (the module's pages
      // nest in it)
      // and starts under the bar, plus the module menu's tab row on a phone.
      const rect = await frame.boundingBox();
      const column = (await page.locator('.tmt-ui-shell-main').boundingBox())!;
      const lead = await page.evaluate(() =>
        Number.parseFloat(
          getComputedStyle(document.querySelector('.tmt-ui-shell-bar')!).getPropertyValue(
            '--tmt-ui-shell-lead',
          ),
        ),
      );
      expect(lead).toBe(width < 480 ? 44 : 0);
      expect(rect?.x).toBe(column.x);
      expect(rect?.x).toBe(width < 480 ? 0 : railWidth);
      expect(rect?.width).toBe(column.width);
      expect(rect?.width).toBe(
        (await page.evaluate(() => document.documentElement.clientWidth)) - column.x,
      );
      const header = (await bar.boundingBox())!;
      expect(rect?.y).toBe(header.y + header.height + lead);
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
      await expect(bar).toHaveCSS('flex-wrap', 'nowrap');
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-short.png` });
      const heading = page.getByRole('heading', { name: 'Release notes', exact: true });
      // The header holds no access chip at any width (#4116), so the title keeps its room.
      await expect(heading).toBeVisible();
      if (width < 480) await expect(heading).toHaveAttribute('title', 'Release notes');
      // The shell's rail carries the TMT mark; the bar keeps none (a phone's menu button opens the rail).
      await expect(bar.locator('.tmt-ui-mark')).toBeHidden();
      if (width >= 480)
        await expect(page.locator('.tmt-ui-shell-brand .tmt-brand-mark')).toBeVisible();
      else await expect(page.locator('.tmt-ui-shell-open:visible')).toBeVisible();
      // The bar draws no module crumb (the rail names the module), so the title keeps its row; the
      // module and the connection identity stay in the header's accessible name.
      await expect(bar.locator('.tmt-ui-wordmark, .tmt-ui-brand')).toHaveCount(0);
      await expect(bar).toHaveAttribute('aria-label', 'Colab · local · Studio Mac · Release notes');
      // The connection words stay in the document for assistive technology; nothing shows.
      await expect(bar.locator('.page-connection')).toHaveText('Private · Live');
      await expect(bar.locator('.page-attention')).toHaveCount(0);
      // Discussion and More at every width; Agent status is a writer-only More entry.
      await expect(bar.locator('.page-header-actions button')).toHaveCount(2);
      await page.getByRole('button', { name: 'More', exact: true }).click();
      await expect(
        page.getByRole('menuitemradio', { name: 'Theme: System', exact: true }),
      ).toHaveAttribute('aria-checked', 'true');
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-menu.png` });
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
      await (await pageAction(page, 'Comments')).click();
      const comments = commentsRegion(page);
      await expect(comments).toBeVisible();
      expect((await frame.boundingBox())?.width).toBe(beforePanel?.width);
      expect((await frame.boundingBox())?.height).toBe(beforePanel?.height);
      expect(await page.evaluate(() => window.scrollY)).toBe(scrollBeforePanel);
      if (width === 390) {
        // #4641: regular phone drawers retain the approved 12px viewport allowance.
        expect((await comments.boundingBox())?.width).toBe(378);
        expect((await comments.boundingBox())?.y).toBe(headerHeight(width) + 4);
      }
      await page.screenshot({ path: `/tmp/1586-${width}-${theme}-comments.png` });
      await commentsClose(page, 'list').click();
      await expect(comments).not.toBeVisible();
      await expect(page.getByRole('button', { name: /^Discussion \(/ })).toBeFocused();
      await (await pageAction(page, 'Chat')).click();
      const asks = page.locator('.tmt-ui-drawer[data-panel=chat]');
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
    // The same two entries at every width; Agent status, Files, Source and Export live in More.
    const names = [/^Discussion \(0\)$/, /^More$/];
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
    const discussion = actions.getByRole('button', { name: /^Discussion \(/ });
    await discussion.evaluate((node: HTMLButtonElement) => node.click());
    await expect(actions.getByRole('menu')).toHaveCount(0);
    await discussion.press('ArrowDown');
    const chat = actions.getByRole('menuitem', { name: 'Chat', exact: true });
    const comments = actions.getByRole('menuitem', { name: 'Comments 0', exact: true });
    await expect(chat).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await expect(comments).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(chat).toBeFocused();
    await page.keyboard.press('End');
    await expect(comments).toBeFocused();
    await page.keyboard.press('Home');
    await expect(chat).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(discussion).toBeFocused();
    await discussion.press('Space');
    await chat.press('Enter');
    const drawer = page.locator('.tmt-ui-drawer[data-panel=chat][open]');
    await expect(drawer).toBeVisible();
    const close = drawer.getByRole('button', { name: 'Close Chat', exact: true });
    await close.focus();
    const closeTooltip = close.locator('..').locator('.tmt-ui-icon-action-tooltip');
    await expect(closeTooltip).toBeVisible();
    // One Escape dismisses the tooltip and closes its containing drawer.
    await page.keyboard.press('Escape');
    await expect(drawer).toHaveCount(0);
    await expect(discussion).toBeFocused();
    await expect(page.locator('.tmt-ui-icon-action-tooltip:popover-open')).toHaveCount(0);
    const frame = page.locator('#ask-page-fixture iframe');
    const render = await frame.getAttribute('data-render-id');
    await (await pageAction(page, 'Theme: Dark')).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await (await pageAction(page, 'Theme: System')).click();
    await expect(page.locator('html')).not.toHaveAttribute('data-theme');
    await expect(page.locator('html')).toHaveCSS('color-scheme', 'light');
    await page.emulateMedia({ colorScheme: 'dark' });
    await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
    await expect(frame).toHaveAttribute('data-render-id', render!);
    await (await pageAction(page, 'Theme: System')).focus();
    await expect(
      actions.getByRole('menuitemradio', { name: 'Theme: System', exact: true }),
    ).toHaveAttribute('aria-checked', 'true');
    await page.keyboard.press('Tab');
    await expect(actions.getByRole('menu')).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`drawer edge ${width}px ${theme}: gutters preserve header and page geometry`, async ({
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
      const before = await frame.boundingBox();
      const scroll = await page.evaluate(() => window.scrollY);
      await (await pageAction(page, 'Comments')).click();
      const drawer = commentsRegion(page);
      await expect(drawer).toBeVisible();
      const edge = (await drawer.boundingBox())!;
      const gutter = width <= 480 ? 4 : 8;
      expect(edge.x + edge.width).toBe(width - gutter);
      expect(edge.y + edge.height).toBe(900 - gutter);
      expect(edge.y).toBe(headerHeight(width) + gutter);
      await expect(drawer).toHaveCSS('border-right-width', '1px');
      await expect(page.locator('.frame-host:visible')).toHaveCSS('clip-path', 'none');
      const capture = process.env.COLAB_DRAWER_CAPTURE_DIR;
      if (capture) {
        mkdirSync(capture, { recursive: true });
        await page.screenshot({ path: `${capture}/comments-${width}-${theme}.png` });
      }
      expect((await frame.boundingBox())?.width).toBe(before?.width);
      expect((await frame.boundingBox())?.height).toBe(before?.height);
      expect(await page.evaluate(() => window.scrollY)).toBe(scroll);
      const box = (await marker.boundingBox())!;
      const exposed = { x: box.x + box.width - 2, y: box.y + box.height / 2 };
      const frameAt = () =>
        page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.tagName === 'IFRAME', exposed);
      // The 8 px page edge is visible; the annotation marker sits farther in and remains covered.
      if (width > 640) await expect.poll(frameAt).toBe(false);
      await commentsClose(page, 'list').click();
      await expect(drawer).not.toBeVisible();
      await expect.poll(frameAt).toBe(true);
      await marker.click();
      await expect(page.getByTestId('comment-thread')).toBeVisible();
    });
    test(`export actions ${width}px ${theme}: shared downloads and one close`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/ask-page-browser.tsx';
        await (await import(path)).mount({ exportAttachments: true });
      });
      await (await pageAction(page, 'Export page')).click();
      const drawer = page.locator('.tmt-ui-drawer[data-panel=export][open]');
      await expect(drawer).toBeVisible();
      for (const name of ['page.html', 'conversations.json', 'conversations.md', 'manifest.json']) {
        const action = drawer.getByRole('button', { name: `Download ${name}`, exact: true });
        await expect(action).toBeEnabled();
        await expect(action).toHaveClass(/tmt-ui-action/);
        // The first file is the group's one primary; the rest are secondary.
        await expect(action).toHaveAttribute(
          'data-variant',
          name === 'page.html' ? 'primary' : 'secondary',
        );
        await expect(action).toHaveCSS('border-top-width', '1px');
      }
      await expect(drawer.getByRole('button', { name: /^Close/ })).toHaveCount(1);
      const capture = process.env.COLAB_DRAWER_CAPTURE_DIR;
      if (capture) {
        mkdirSync(capture, { recursive: true });
        await page.screenshot({ path: `${capture}/export-${width}-${theme}.png` });
      }
      const pending = page.waitForEvent('download');
      await drawer.getByRole('button', { name: 'Download page.html', exact: true }).click();
      expect((await pending).suggestedFilename()).toBe('page.html');
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
  const header = page.locator('.tmt-ui-header:visible');
  const heading = header.getByRole('heading', { name: title, exact: true });
  await expect(heading).toBeVisible();
  await expect(header).toHaveAttribute('aria-label', `Colab · local · ${deviceName} · ${title}`);
  await expect(header.locator('.tmt-ui-caption')).toBeVisible();
  await expect(header.locator('.tmt-ui-mark')).toBeHidden();
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
    await expect(header.locator('.tmt-ui-mark')).toBeHidden();
    await expect(
      page.locator(
        width > 900 ? '.tmt-ui-shell-brand .tmt-brand-mark:visible' : '.tmt-ui-shell-open:visible',
      ),
    ).toBeVisible();
    await expect(heading).toHaveAttribute('title', title);
    await expect(header).toHaveCSS('height', `${headerHeight(width)}px`);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    expect(await iframe.evaluate((node, saved) => node === saved, frameHandle)).toBe(true);
    expect(await heading.evaluate((node, saved) => node === saved, titleHandle)).toBe(true);
    await expect(heading).toBeVisible();
    const actions = header.getByRole('navigation', { name: 'Page actions' });
    await expect(actions.getByRole('button')).toHaveCount(2);
    const more = actions.getByRole('button', { name: 'More', exact: true });
    await more.press('ArrowDown');
    // Share… waits on page management and a page that holds no file shows no Files entry; the first
    // enabled entry is Agent status, then Source and Export page.
    await expect(actions.getByRole('menuitem', { name: /^Share…/ })).toBeDisabled();
    await expect(actions.getByRole('menuitem', { name: /^Files/ })).toHaveCount(0);
    await expect(
      actions.getByRole('menuitem', { name: 'Agent status', exact: true }),
    ).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect(actions.getByRole('menuitem', { name: 'Source', exact: true })).toBeFocused();
    await expect(actions.getByRole('menuitem', { name: 'Export page', exact: true })).toBeEnabled();
    await page.keyboard.press('ArrowDown');
    await expect(actions.getByRole('menuitem', { name: 'Export page', exact: true })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(more).toBeFocused();
  }
  await page.evaluate(async () => {
    const path = '/test/page-layout-browser.tsx';
    (await import(path)).mount('<h1>Stable content</h1>', {}, {});
  });
  await expect(header).toHaveAttribute('aria-label', /^Colab · local/);
  await expect(header.locator('.tmt-ui-caption')).toHaveCount(0);
});
