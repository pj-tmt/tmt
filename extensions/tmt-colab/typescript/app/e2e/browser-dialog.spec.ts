import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`content-fitting modal traps focus, dismisses without click-through and restores focus ${width} ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.evaluate(async () => {
      const fixture = '/test/browser-dialog.tsx';
      (await import(/* @vite-ignore */ fixture)).render();
    });
    await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
    const opener = page.getByRole('button', { name: 'Open search', exact: true });
    const input = page.getByRole('textbox', { name: 'Search pages', exact: true });
    const dialog = page.getByRole('dialog', { name: 'Search pages', exact: true });
    await opener.click();
    await expect(dialog).toBeVisible();
    await expect(input).toBeFocused();
    await input.fill('Retained during this opening');
    // Chromium's native forward cycle may visit browser chrome. Backward from
    // the first body control stays in the modal rather than reaching its opener.
    await page.keyboard.press('Shift+Tab');
    await expect(
      dialog.getByRole('button', { name: 'Close Search pages', exact: true }),
    ).toBeFocused();
    await opener.evaluate((node) => node.focus());
    await expect(
      dialog.getByRole('button', { name: 'Close Search pages', exact: true }),
    ).toBeFocused();
    const header = await opener.boundingBox();
    await page.mouse.click(header!.x + header!.width / 2, header!.y + header!.height / 2);
    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId('dialog-reason')).toHaveText('backdrop');
    await expect(page.getByTestId('header-presses')).toHaveText('0');
    await expect(opener).toBeFocused();
    await opener.click();
    const frame = await page.locator('iframe').boundingBox();
    // This point is outside the centered modal, inside the covered author frame.
    await page.mouse.click(frame!.x + 1, frame!.y + 1);
    await expect(dialog).toHaveCount(0);
    expect(
      await page.frameLocator('iframe').locator('body').getAttribute('data-presses'),
    ).toBeNull();
    await opener.click();
    await page.evaluate(() => (document.activeElement as HTMLElement).blur());
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(opener).toBeFocused();
    await opener.click();
    await expect(input).toBeFocused();
    await page.keyboard.press('Shift+Escape');
    await expect(dialog).toBeVisible();
    await page.getByRole('button', { name: 'Remove opener', exact: true }).click();
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Header action', exact: true })).toBeFocused();
  });
}
