import { expect, test, type Locator, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';

const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

type Mounted = { layout?: 'column' | 'full' | 'canvas'; layoutReason?: string };
async function mount(page: Page, options: Mounted = {}, width = 1440) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, ...options });
  await run(page, 'authored', '<p id="only">A page.</p>');
  await expect(frame(page).locator('#only')).toBeVisible();
}
async function about(page: Page): Promise<Locator> {
  const drawer = page.locator('#ask-page-fixture .tmt-ui-drawer[data-panel=about]');
  if (!(await drawer.isVisible())) await (await pageAction(page, 'About this page')).click();
  await expect(drawer).toBeVisible();
  return drawer.locator('.page-layout-info');
}
const box = (node: Locator) =>
  node.evaluate((element) => {
    const { left, right, top, bottom, height } = element.getBoundingClientRect();
    return { left, right, top, bottom, height };
  });
const reason = 'A status board draws its own cards and needs the whole window';
const long = Array.from({ length: 40 }, (_, index) => `reason-${index}`).join(' ');

for (const [name, options, value] of [
  ['column', { layout: 'column' }, 'Column · max 1280 px'],
  ['full', { layout: 'full' }, 'Full width'],
  ['canvas', { layout: 'canvas', layoutReason: reason }, 'Canvas · author layout'],
  ['unset', {}, 'Freeform · author layout'],
] as const)
  for (const width of [1440, 390])
    test(`About names a ${name} page's layout at ${width}`, async ({ page }) => {
      await mount(page, options, width);
      const section = await about(page);
      await expect(section.getByRole('heading', { name: 'Layout' })).toBeVisible();
      await expect(section.locator('.page-layout-value')).toContainText(value);
      const label = await box(section.getByRole('heading', { name: 'Layout' }));
      const text = await box(section.locator('.page-layout-value'));
      // The value sits under its heading, flush with it, at every width.
      expect(text.top).toBeGreaterThanOrEqual(label.bottom - 1);
      expect(Math.abs(text.left - label.left)).toBeLessThan(1);
      if ('layoutReason' in options) {
        await expect(section.locator('.page-layout-reason')).toHaveText(reason);
        // A short reason needs no expansion.
        await expect(section.getByRole('button')).toHaveCount(0);
      } else await expect(section.locator('.page-layout-reason')).toHaveCount(0);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`layout-info-${name}-${width}.png`) });
    });

for (const width of [1440, 390])
  test(`a long canvas reason is clamped to three lines and expands in place at ${width}`, async ({
    page,
  }) => {
    await mount(page, { layout: 'canvas', layoutReason: long }, width);
    const section = await about(page);
    const line = section.locator('.page-layout-reason');
    const lineHeight = await line.evaluate((node) => parseFloat(getComputedStyle(node).lineHeight));
    const clamped = await box(line);
    expect(clamped.height).toBeLessThanOrEqual(lineHeight * 3 + 1);
    await expect(line).toHaveAttribute('title', long);
    const show = section.getByRole('button', { name: 'Show all' });
    await expect(show).toBeVisible();
    // The action sits flush with the reason text, not indented by the button's own padding.
    expect(Math.abs((await box(show)).left - clamped.left)).toBeLessThan(1);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`layout-info-long-clamped-${width}.png`) });
    await show.click();
    await expect.poll(async () => (await box(line)).height).toBeGreaterThan(lineHeight * 3 + 1);
    await expect(line).toHaveText(long);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`layout-info-long-expanded-${width}.png`) });
    const less = section.getByRole('button', { name: 'Show less' });
    await less.click();
    await expect.poll(async () => (await box(line)).height).toBeLessThanOrEqual(lineHeight * 3 + 1);
    await expect(section.getByRole('button', { name: 'Show all' })).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });

// The drawer reads as headed sections divided by rules, as the website's About does.
for (const width of [1440, 390])
  test(`About is headed sections divided by rules at ${width}`, async ({ page }) => {
    await mount(page, { layout: 'column' }, width);
    await about(page);
    const drawer = page.locator('#ask-page-fixture .tmt-ui-drawer[data-panel=about]');
    await expect(drawer.getByRole('heading', { level: 3 })).toHaveText([
      'Author and publisher',
      'Layout',
      'Page scripts',
    ]);
    const sections = drawer.locator('section');
    await expect(sections).toHaveCount(3);
    await expect(sections.first()).toHaveCSS('border-top-width', '0px');
    for (const index of [1, 2])
      await expect(sections.nth(index)).toHaveCSS('border-top-width', '1px');
    // The attribution is name and value side by side.
    const name = await box(drawer.locator('.page-attribution dt').first());
    const value = await box(drawer.locator('.page-attribution dd').first());
    expect(value.left).toBeGreaterThan(name.right - 1);
    expect(Math.abs(value.top - name.top)).toBeLessThan(2);
  });

test('a live metadata-only layout change reframes the page and updates About without a new source', async ({
  page,
}) => {
  await mount(page, { layout: 'column' });
  const html = frame(page).locator('html');
  await expect(html).toHaveAttribute('data-colab-layout', 'column');
  const section = await about(page);
  await expect(section.locator('.page-layout-value')).toContainText('Column · max 1280 px');
  const title = await page.title();

  await run(page, 'setLayout', { layout: 'canvas', layoutReason: reason });
  await expect(frame(page).locator('#only')).toBeVisible();
  await expect(html).not.toHaveAttribute('data-colab-layout', /.+/);
  await expect(section.locator('.page-layout-value')).toContainText('Canvas · author layout');
  await expect(section.locator('.page-layout-reason')).toHaveText(reason);

  await run(page, 'setLayout', { layout: 'full' });
  await expect(html).toHaveAttribute('data-colab-layout', 'full');
  await expect(section.locator('.page-layout-value')).toContainText('Full width');
  await expect(section.locator('.page-layout-reason')).toHaveCount(0);

  await run(page, 'setLayout', {});
  await expect(html).not.toHaveAttribute('data-colab-layout', /.+/);
  await expect(section.locator('.page-layout-value')).toContainText('Freeform · author layout');
  expect(await page.title()).toBe(title);
  await expect(frame(page).locator('#only')).toHaveText('A page.');
});
