import { expect, test, type Page } from '@playwright/test';
const control = (page: Page, action: string) =>
  page.evaluate(async ({ path, action }) => (await import(path)).control(action), {
    path: '/test/reply-reveal-browser.tsx',
    action,
  });
async function click(page: Page) {
  await page.goto('/test/reply-reveal.html');
  await expect(page.locator('#activations')).toHaveText('0');
  await control(page, 'arrive');
  await page.getByRole('button', { name: 'Open Chat', exact: true }).click();
  await expect(page.locator('.tmt-ui-notification')).toHaveCount(0);
  await expect(page.locator('#activations')).toHaveText('0');
}
test('a busy click is retained and activates exactly once when its host becomes ready', async ({
  page,
}) => {
  await click(page);
  await control(page, 'ready');
  await expect(page.locator('#activations')).toHaveText('1');
  await expect(page.locator('#reveal')).not.toHaveText('');
  await control(page, 'target');
  await expect(page.locator('#target')).toBeFocused();
  await expect(page.locator('#target')).toHaveAttribute('data-arrived', '');
  await expect(page.locator('#reveal')).toHaveText('');
  await expect(page.locator('#activations')).toHaveText('1');
});
for (const cancellation of ['page', 'withdraw'])
  test(`a ${cancellation} change cancels the busy reveal without visible effects`, async ({
    page,
  }) => {
    await click(page);
    await control(page, cancellation);
    await control(page, 'ready');
    await control(page, 'target');
    await expect(page.locator('#target')).toBeVisible();
    // Let the queued activation and its destination frame run before asserting
    // absence; an immediate zero can precede an incorrectly retained click.
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
        ),
    );
    await expect(page.locator('#activations')).toHaveText('0');
    await expect(page.locator('#reveal')).toHaveText('');
    await expect(page.locator('[data-arrived]')).toHaveCount(0);
    await expect(page.locator('#target')).not.toBeFocused();
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  });
test('busy activation and an unmounted target expire after the bounded wait', async ({ page }) => {
  await page.clock.install();
  await click(page);
  await page.clock.runFor(5001);
  await control(page, 'ready');
  await page.clock.runFor(32);
  await expect(page.locator('#activations')).toHaveText('0');
  await expect(page.locator('#reveal')).toHaveText('');
  await page.reload();
  await expect(page.locator('#activations')).toHaveText('0');
  await control(page, 'ready');
  await control(page, 'arrive');
  await page.getByRole('button', { name: 'Open Chat', exact: true }).click();
  await page.clock.runFor(32);
  await expect(page.locator('#activations')).toHaveText('1');
  await expect(page.locator('#reveal')).not.toHaveText('');
  await page.clock.runFor(5001);
  await expect(page.locator('#reveal')).toHaveText('');
  await control(page, 'target');
  await page.clock.runFor(32);
  await expect(page.locator('[data-arrived]')).toHaveCount(0);
  await expect(page.locator('#target')).not.toBeFocused();
});

test('an expired reveal cannot act before its delayed cleanup timer runs', async ({ page }) => {
  await click(page);
  await control(page, 'ready');
  await expect(page.locator('#activations')).toHaveText('1');
  await expect(page.locator('#reveal')).not.toHaveText('');
  // Advance the observation clock without executing timers: background throttling
  // must not let a resumed frame apply the stale reveal first.
  await page.evaluate(() => {
    const later = performance.now() + 6000;
    Object.defineProperty(performance, 'now', { configurable: true, value: () => later });
  });
  await control(page, 'target');
  await expect(page.locator('#target')).toBeVisible();
  await expect(page.locator('#reveal')).toHaveText('');
  await expect(page.locator('[data-arrived]')).toHaveCount(0);
  await expect(page.locator('#target')).not.toBeFocused();
});
