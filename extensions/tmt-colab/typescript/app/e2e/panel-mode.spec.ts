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

// The mode follows the viewport at both sides of each width panel-mode.ts names.
for (const [width, mode] of [
  [1600, 'column'],
  [1180, 'column'],
  [1179, 'card'],
  [761, 'card'],
  [760, 'sheet'],
  [390, 'sheet'],
] as const)
  test(`the Comments panel is a ${mode} at ${width}px`, async ({ page }) => {
    await open(page, width);
    await (await pageAction(page, 'Comments')).click();
    const panel = commentsRegion(page);
    await expect(panel).toBeVisible();
    await expect(panel).toHaveAttribute('data-mode', mode);
    // A column is a track of the page grid; a card and a sheet float over the page.
    await expect(panel).toHaveCSS('position', mode === 'column' ? 'sticky' : 'fixed');
    const workspace = panel.locator('xpath=..');
    if (mode === 'column') await expect(workspace).toHaveAttribute('data-panel-column', '');
    else await expect(workspace).not.toHaveAttribute('data-panel-column', /.*/);
  });

// Resizing across a width moves the open panel to the other mode without closing it.
test('the open Comments panel changes mode as the viewport crosses a width', async ({ page }) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await expect(panel).toHaveAttribute('data-mode', 'column');
  await page.setViewportSize({ width: 1000, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'card');
  await page.setViewportSize({ width: 600, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'sheet');
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(panel).toHaveAttribute('data-mode', 'column');
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
    await panel.getByTestId('annotation-row').first().click();
    await page.evaluate(() => window.scrollTo(0, 0));
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await panel.getByRole('button', { name: 'Show in the page', exact: true }).click();
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(300);
    await expect(panel).toBeVisible();
    await expect(panel.getByRole('heading', { level: 2 })).toHaveText('Discussion');
  });

// Escape on the page leaves the panel open; Escape inside it steps back, then closes it.
test('Escape on the page leaves the Comments panel open; inside it, it closes it', async ({
  page,
}) => {
  await open(page, 1440);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await expect(panel).toBeVisible();
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('Escape');
  await expect(panel).toBeVisible();
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
  await panel.getByTestId('annotation-row').first().click();
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
test('a focused reply field stays on screen in a sheet over a short viewport', async ({ page }) => {
  await open(page, 390, 420);
  await (await pageAction(page, 'Comments')).click();
  const panel = commentsRegion(page);
  await panel.getByTestId('annotation-row').first().click();
  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.click();
  await expect(input).toBeFocused();
  const [field, sheet] = await Promise.all([input.boundingBox(), panel.boundingBox()]);
  expect(field!.y).toBeGreaterThanOrEqual(sheet!.y);
  expect(field!.y + field!.height).toBeLessThanOrEqual(sheet!.y + sheet!.height);
  expect(sheet!.y + sheet!.height).toBeLessThanOrEqual(420);
  await expect(input).toBeInViewport({ ratio: 1 });
});
