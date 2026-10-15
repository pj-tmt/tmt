import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

test.use({ timezoneId: 'Asia/Tokyo' });

// This fixture verifies actual chrome/rendering, not authority or provider behavior. The embedded
// hosted-bundle acceptance separately enforces the composed CSP with the real SDK bytes.
for (const width of [390, 1280]) {
  test(`hosted read-only states and renderer at ${width}px`, async ({ page, baseURL }) => {
    const origin = 'https://demo-tmt-hosted.web.app';
    const outside: string[] = [];
    await page.setViewportSize({ width, height: 844 });
    await page.clock.setFixedTime(new Date('2026-11-02T00:00:00Z'));
    await page.routeWebSocket('**/*', (socket) => socket.close());
    await page.route('**/*', async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== origin) {
        outside.push(url.origin);
        await route.abort();
      } else {
        const response = await route.fetch({ url: `${baseURL}${url.pathname}${url.search}` });
        await route.fulfill({ response });
      }
    });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    for (const state of [
      'sign-in',
      'verifying',
      'recovering',
      'paused',
      'ready',
      'reshare',
      'budget',
      'rate-limited',
      'denied',
      'missing',
    ]) {
      await page.goto(`${origin}/test/hosted-browser.html?state=${state}`);
      if (state === 'ready') {
        await expect(
          page.frameLocator('iframe').getByRole('heading', { name: 'Verified shared page' }),
        ).toBeVisible();
        await expect(page.locator('iframe')).toHaveAttribute('src', `${origin}/renderer.html`);
        await expect(page.getByText('Live', { exact: true })).toBeVisible();
        await expect(
          page.getByText('This browser is close to its daily reading allowance.', {
            exact: false,
          }),
        ).toBeVisible();
        await expect(page.getByRole('button', { name: /^(Save|Ask|Share)$/ })).toHaveCount(0);
        await page.getByLabel('Info', { exact: true }).click();
        const panel = page.locator('.reader-information-panel');
        await expect(panel).toBeVisible();
        const box = (await panel.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        await page.getByLabel('Info', { exact: true }).click();
      } else {
        await expect(page.getByTestId('hosted-entry')).toBeVisible();
        await expect(page.locator('iframe')).toHaveCount(0);
        await expect(page.getByText('Live', { exact: true })).toHaveCount(0);
        if (state === 'sign-in')
          await expect(page.getByRole('button', { name: 'Sign in', exact: true })).toBeVisible();
        if (state === 'reshare')
          await expect(
            page.getByText(
              'This page changed its sharing since this link was made. Ask the owner to share it again.',
            ),
          ).toBeVisible();
        if (state === 'budget') {
          await expect(page.getByText('The allowance resets at 17:00.')).toBeVisible();
          await expect(page.getByText('Limit reached', { exact: true })).toBeVisible();
          await expect(page.getByText('failed', { exact: true })).toHaveCount(0);
          await expect(page.getByRole('button')).toHaveCount(0);
        }
      }
      if (state === 'rate-limited') {
        await expect(
          page.getByRole('heading', { name: 'Too many requests', exact: true }),
        ).toBeVisible();
        await expect(page.getByText('Paused', { exact: true })).toBeVisible();
        await expect(page.getByText('failed', { exact: true })).toHaveCount(0);
        await expect(page.getByRole('button')).toHaveCount(0);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      await page.screenshot({ path: capturePath(`hosted-${state}-${width}.png`), fullPage: true });
    }
    expect(errors).toEqual([]);
    expect(outside).toEqual([]);
  });
}
