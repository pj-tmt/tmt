import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
const app = (page: Page) => page.locator('#list-fixture');
async function mount(page: Page, size: 'work' | 'few' | 'unmanaged' = 'work') {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(async (size) => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList(size);
  }, size);
}
const row = (page: Page, title: string) =>
  app(page)
    .locator('.pages > li')
    .filter({ has: page.getByRole('link', { name: title, exact: true }) });
async function pin(page: Page, title: string) {
  const target = row(page, title);
  await target.scrollIntoViewIfNeeded();
  await target.getByRole('button', { name: `Pin ${title}` }).focus();
  await target.getByRole('button', { name: `Pin ${title}` }).click();
}
test('concept17 shows real pins in owner order, wraps after three and shares the list scope', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1586, height: 992 });
  await mount(page);
  await expect(app(page).getByRole('heading', { name: 'A shared place for ideas.' })).toBeVisible();
  await expect(app(page).locator('h1:visible')).toHaveCount(1);
  const header = app(page).locator('.tmt-ui-header:visible');
  await expect(header).toHaveAttribute('aria-label', /Colab.*Pages/);
  await expect(header.locator('h1, .tmt-ui-title')).toHaveCount(0);

  await expect(app(page).getByRole('list', { name: 'Pinned pages' })).toHaveCount(0);
  for (const title of ['Review 2', 'Spec 1', 'Plan 3', 'Notes 4']) await pin(page, title);
  const pins = app(page).getByRole('list', { name: 'Pinned pages' });
  await expect(pins.getByRole('link')).toHaveText(['Review 2', 'Spec 1', 'Plan 3', 'Notes 4']);
  const firstPin = pins.getByRole('link').first();
  const beforeFocus = await firstPin.boundingBox();
  await firstPin.focus();
  await expect(firstPin).toBeFocused();
  const frame = await firstPin.evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      width: parseFloat(style.outlineWidth),
      offset: parseFloat(style.outlineOffset),
      border: parseFloat(style.borderWidth),
      shadow: style.boxShadow,
    };
  });
  expect(frame.width).toBeGreaterThan(0);
  expect(frame.offset).toBeLessThanOrEqual(-frame.width);
  expect(frame.border).toBe(0);
  expect(frame.shadow).toBe('none');
  expect(await firstPin.boundingBox()).toEqual(beforeFocus);

  const bounds = await pins
    .locator('> li')
    .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().top));
  expect(bounds[0]).toBe(bounds[2]);
  expect(bounds[3]).toBeGreaterThan(bounds[0]);
  await app(page).getByRole('button', { name: 'Scope to blog', exact: true }).click();
  await expect(
    app(page).getByRole('button', { name: 'Scope to blog', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await expect(pins.getByRole('link')).toHaveText(['Review 2']);
  await expect(app(page).getByRole('heading', { name: 'blog', exact: true })).toBeVisible();
  await expect(app(page).locator('h1:visible')).toHaveCount(1);
  await expect(header).not.toContainText('blog');

  await expect(row(page, 'Spec 1')).toHaveCount(0);
});
for (const width of [1586, 1440, 1150, 901, 900, 651, 650, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`concept17 stays readable at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      for (const title of ['Spec 1', 'Review 2', 'Plan 3']) await pin(page, title);
      const section = app(page).locator('[data-page-list]');
      await expect(
        app(page).getByRole('list', { name: 'Pinned pages' }).getByRole('link'),
      ).toHaveCount(3);
      expect(await section.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await section.evaluate((node) => {
        node.scrollTop = 0;
      });
      await page.locator('body').click({ position: { x: width - 4, y: 12 } });
      await page.mouse.move(0, 0);
      await page.screenshot({ path: await capturePath(`concept17-after-${width}-${theme}.png`) });
    });
test('a viewer gets no pin controls and unreported metadata stays unknown', async ({ page }) => {
  await mount(page, 'unmanaged');
  await expect(app(page).getByRole('button', { name: /^Pin |^Unpin / })).toHaveCount(0);
  await mount(page, 'few');
  const unknown = row(page, 'Gamma notes');
  await expect(unknown.locator('[data-page-stage]')).toHaveCount(0);
  await expect(unknown.locator('.page-author')).toHaveCount(0);
  await expect(unknown.locator('.page-comments')).toHaveCount(0);
});

test('the header selection includes matching pages not drawn yet and clears as one selection', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1586, height: 992 });
  await mount(page);
  await expect(app(page).locator('.pages > li')).toHaveCount(50);
  await app(page).getByRole('checkbox', { name: 'Select all matching pages' }).check();
  await expect(app(page).getByRole('region', { name: 'Bulk actions' })).toContainText('75');
  await app(page).getByRole('button', { name: 'Clear selection', exact: true }).click();
  await expect(
    app(page).getByRole('checkbox', { name: 'Select all matching pages' }),
  ).not.toBeChecked();
});
