import { expect, test, type Page } from '@playwright/test';
import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}

/** The long page the Escape and reveal specs use: the passage sits 1000px down. */
const longPage =
  '<main style="height:1800px"><p id="selected" style="margin-left:calc(100% - 150px);padding-top:1000px">Exact selected text</p></main>';

async function open(page: Page, width: number, height = 900) {
  await page.setViewportSize({ width, height });
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'change', longPage);
  await run(page, 'conversation', { surface: 'thread', state: 'replied', message: 'A note.' });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
}

// The conversation stays a floating card on both sides of the former layout breakpoints.
for (const [width, mode] of [
  [1600, 'card'],
  [1180, 'card'],
  [1179, 'card'],
  [761, 'card'],
  [760, 'card'],
  [390, 'card'],
] as const)
  test(`the Comments panel is a ${mode} at ${width}px`, async ({ page }) => {
    await open(page, width);
    await (await pageAction(page, 'Comments')).click();
    const panel = commentsRegion(page);
    await expect(panel).toBeVisible();
    await expect(panel).toHaveAttribute('data-mode', mode);
    // Conversation opening never adds a page-grid track.
    await expect(panel).toHaveCSS('position', 'fixed');
    const workspace = panel.locator('xpath=..');
    await expect(workspace).not.toHaveAttribute('data-panel-column', /.*/);
  });

// Resizing retains the same floating host without closing it.
test('the open conversation stays floating while the viewport resizes', async ({ page }) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await expect(panel).toHaveAttribute('data-mode', 'card');
  await page.setViewportSize({ width: 1000, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'card');
  await page.setViewportSize({ width: 600, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'card');
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'card');
});

for (const width of [1440, 1024, 390])
  test(`the Comments panel is not modal at ${width}px: no scrim, no trap, a live page`, async ({
    page,
  }) => {
    await open(page, width);
    await (await pageAction(page, 'Comments')).click();
    const panel = commentsRegion(page);
    await expect(panel).toBeVisible();
    // No top layer, scrim or inert page: the header and the page stay live beside or under it.
    expect(await page.locator('dialog:modal').count()).toBe(0);
    await expect(page.locator('#ask-page-fixture iframe')).not.toHaveAttribute('inert', /.*/);
    await expect(page.locator('.tmt-ui-header').first()).not.toHaveAttribute('inert', /.*/);
    await page.evaluate(() => window.scrollTo(0, 300));
    expect(await page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
    await expect(panel).toBeVisible();
  });

// Show in the page scrolls the page to the passage and leaves the panel open.
for (const width of [1440, 390])
  test(`Show in the page scrolls to the passage and keeps the panel at ${width}px`, async ({
    page,
  }) => {
    await open(page, width);
    await (await pageAction(page, 'Comments')).click();
    const panel = commentsRegion(page);
    // Selecting the row already brings the passage into view; the reader then scrolls away.

    await page.evaluate(() => window.scrollTo(0, 0));
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await panel.getByRole('button', { name: /^Show in the page/ }).click();
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(300);
    await expect(panel).toBeVisible();
    await expect(panel.getByRole('heading', { level: 2 })).toHaveText('Discussion');
  });

// Escape closes the same conversation from either focus location.
test('Escape closes Comments from the page and from its header', async ({ page }) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await expect(panel).toBeVisible();
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
  await (await pageAction(page, 'Comments')).click();
  await panel.getByRole('heading', { level: 2 }).focus();
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
});

// One panel at a time: a drawer replaces the panel, and the panel replaces a drawer.
test('Comments and a drawer share the single panel slot', async ({ page }) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await expect(panel).toBeVisible();
  await (await pageAction(page, 'Source')).click();
  await expect(page.locator('.tmt-ui-drawer[data-panel=source][open]')).toBeVisible();
  await expect(panel).toBeHidden();
  await (await pageAction(page, 'Comments')).click();
  await expect(panel).toBeVisible();
  await expect(page.locator('.tmt-ui-drawer[data-panel=source][open]')).toHaveCount(0);
});

// An unsent reply survives closing and reopening the panel.
test('an unsent reply survives closing and reopening the Comments panel', async ({ page }) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);

  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.fill('Kept across a close.');
  await commentsClose(page, 'discussion').click();
  await expect(panel).toBeHidden();
  await (await pageAction(page, 'Comments')).click();
  await expect(panel.getByRole('combobox', { name: 'Message', exact: true })).toHaveText(
    'Kept across a close.',
    { useInnerText: true },
  );
});

// A sheet over a short viewport (the on-screen keyboard) never covers the field being typed in.
test('a focused reply field stays on screen in a floating window over a short viewport', async ({
  page,
}) => {
  await open(page, 390, 420);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);

  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.click();
  await expect(input).toBeFocused();
  const [field, sheet] = await Promise.all([input.boundingBox(), panel.boundingBox()]);
  expect(field!.y).toBeGreaterThanOrEqual(sheet!.y);
  expect(field!.y + field!.height).toBeLessThanOrEqual(sheet!.y + sheet!.height);
  expect(sheet!.y + sheet!.height).toBeLessThanOrEqual(420);
  await expect(input).toBeInViewport({ ratio: 1 });
});

for (const width of [390, 1440])
  test(`Chat and Comments share one floating window above the visible dock at ${width}px`, async ({
    page,
  }, testInfo) => {
    await open(page, width);
    await run(
      page,
      'change',
      '<main style="height:2000px"><button id="page-control">Page control</button><p id="selected">Exact selected text</p><p id="selectable">Unmarked selectable text.</p></main>',
    );
    const frame = page.frameLocator('#ask-page-fixture iframe');
    const before = await page.locator('#ask-page-fixture iframe').boundingBox();
    await (await pageAction(page, 'Comments')).click();
    const window = page.locator('#ask-page-fixture .page-conversation-window');
    await expect(window).toBeVisible();
    const close = window.getByRole('button', { name: 'Close Discussion', exact: true });
    await expect
      .poll(() =>
        close.evaluate((button) => {
          const box = button.getBoundingClientRect();
          return button.contains(
            document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
          );
        }),
      )
      .toBe(true);

    await expect(window).toHaveCSS('position', 'fixed');
    const original = await window.elementHandle();
    const commentsBox = await window.boundingBox();
    await page.screenshot({ path: testInfo.outputPath('comments.png') });
    expect((await page.locator('#ask-page-fixture iframe').boundingBox())!.width).toBe(
      before!.width,
    );
    const dock = page.locator('#ask-page-fixture .page-floating-actions');
    await expect(dock).toBeVisible();
    const dockBox = await dock.boundingBox();
    expect(commentsBox!.y + commentsBox!.height).toBeLessThanOrEqual(dockBox!.y - 7);
    expect(await page.locator('dialog:modal').count()).toBe(0);
    const commentInput = window.getByRole('combobox', { name: 'Message', exact: true });
    await commentInput.fill('Keep the comment draft');
    await frame.locator('#page-control').click();
    await expect(frame.locator('#page-control')).toBeFocused();
    await frame.locator('#selectable').dblclick({ position: { x: 10, y: 10 } });
    expect(await frame.locator('body').evaluate(() => getSelection()?.toString())).toBeTruthy();
    await dock.getByRole('button', { name: /^Chat/ }).click();
    await expect(window.getByTestId('chat-panel')).toBeVisible();
    expect(await window.evaluate((node, original) => node === original, original)).toBe(true);
    expect(await window.boundingBox()).toEqual(commentsBox);
    const chatInput = window.getByRole('combobox', { name: 'Message', exact: true });
    await chatInput.fill('Keep the Chat draft');
    const selectionControl = page.locator('#ask-page-fixture [data-testid="selection-ask"]');
    await expect(selectionControl).toBeVisible();
    const controlBox = (await selectionControl.boundingBox())!;
    const hostBox = (await window.boundingBox())!;
    expect(
      controlBox.x >= hostBox.x + hostBox.width ||
        controlBox.x + controlBox.width <= hostBox.x ||
        controlBox.y >= hostBox.y + hostBox.height ||
        controlBox.y + controlBox.height <= hostBox.y,
    ).toBe(true);
    await page.screenshot({ path: testInfo.outputPath('chat.png') });
    await dock.getByRole('button', { name: /^Comments/ }).click();
    await expect(commentInput).toHaveText('Keep the comment draft');
    expect(await window.boundingBox()).toEqual(commentsBox);
    await frame.locator('#page-control').press('Escape');
    await expect(window).toBeHidden();
    await expect(dock.getByRole('button', { name: /^Comments/ })).toBeFocused();
    await dock.getByRole('button', { name: /^Chat/ }).click();
    await expect(chatInput).toHaveText('Keep the Chat draft');
    await page.locator('body').click({ position: { x: 1, y: 1 } });
    await page.keyboard.press('Escape');
    await expect(window).toBeHidden();
    await expect(dock.getByRole('button', { name: /^Chat/ })).toBeFocused();
  });
