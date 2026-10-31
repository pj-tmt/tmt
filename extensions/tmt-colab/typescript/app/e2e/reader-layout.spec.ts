import { expect, test, type Page } from '@playwright/test';
import type { PageView } from '../src/transport.js';

const fixture = '/test/reader-metadata-browser.tsx';
const frame = (page: Page) => page.locator('#reader-metadata-fixture iframe');
const html = (page: Page) => page.frameLocator('#reader-metadata-fixture iframe').locator('html');
const layout = (page: Page, next: PageView['layout']) =>
  page.evaluate(async ({ fixture, next }) => (await import(fixture)).layout(next), {
    fixture,
    next,
  });
async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async (fixture) => (await import(fixture)).mount('viewer'), fixture);
  await expect(
    page.frameLocator('#reader-metadata-fixture iframe').getByRole('heading', {
      name: 'Editor metadata',
      exact: true,
    }),
  ).toBeVisible();
}

for (const width of [390, 1440]) {
  for (const kind of ['column', 'full', 'canvas', undefined] as const)
    test(`reader applies ${kind ?? 'absent'} layout at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      await layout(page, kind);
      await expect(frame(page)).toHaveCount(1);
      if (kind === 'column' || kind === 'full')
        await expect(html(page)).toHaveAttribute('data-colab-layout', kind);
      // Canvas and absent (freeform) intentionally retain author layout, as in the page view.
      else await expect(html(page)).not.toHaveAttribute('data-colab-layout', /.+/);
    });

  test(`reader remounts for layout-only changes, not unchanged metadata, at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    const source = await page.evaluate(
      async (fixture) => (await import(fixture)).proof().view.source,
      fixture,
    );
    for (const kind of ['column', 'full', 'canvas', undefined] as const) {
      const previous = (await frame(page).elementHandle())!;
      const previousId = await frame(page).getAttribute('data-render-id');
      await layout(page, kind);
      await expect.poll(() => previous.evaluate((node) => node.isConnected)).toBe(false);
      await expect(frame(page)).toHaveCount(1);
      await expect(frame(page)).not.toHaveAttribute('data-render-id', previousId!);
      if (kind === 'column' || kind === 'full')
        await expect(html(page)).toHaveAttribute('data-colab-layout', kind);
      else await expect(html(page)).not.toHaveAttribute('data-colab-layout', /.+/);
      const current = (await frame(page).elementHandle())!;
      await layout(page, kind);
      expect(await current.evaluate((node) => node.isConnected)).toBe(true);
      expect(
        await page.evaluate(
          async (fixture) => (await import(fixture)).proof().view.source,
          fixture,
        ),
      ).toBe(source);
    }
  });
}
