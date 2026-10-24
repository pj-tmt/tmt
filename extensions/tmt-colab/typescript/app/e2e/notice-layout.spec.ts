import { expect, test } from '@playwright/test';

for (const width of [1586, 390]) {
  test(`state-card spacing does not leak into inline notices at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 992 });
    await page.goto('/');
    await page.evaluate(
      async (fixture) => (await import(fixture)).mount(),
      '/test/notice-layout-browser.tsx',
    );
    const card = page.getByTestId('state-card').locator('.tmt-ui-notice');
    const inline = page.getByTestId('inline-host').locator('.tmt-ui-notice');
    await expect(card).toBeVisible();
    await expect(inline).toBeVisible();
    const layout = await inline.evaluate((node) => ({
      width: node.getBoundingClientRect().width,
      hostWidth: node.parentElement!.getBoundingClientRect().width,
      margin: getComputedStyle(node).marginTop,
    }));
    expect(layout.width).toBeCloseTo(layout.hostWidth, 0);
    expect(layout.margin).toBe('0px');
    expect(
      await card.evaluate((node) => parseFloat(getComputedStyle(node).marginTop)),
    ).toBeGreaterThan(0);
    expect(await card.evaluate((node) => node.getBoundingClientRect().width)).toBeLessThanOrEqual(
      680,
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  });
}
