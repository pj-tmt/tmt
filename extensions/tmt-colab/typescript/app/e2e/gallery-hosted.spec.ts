import { expect, test } from '@playwright/test';

test('the gallery hosted-shell frame stays on its own page and never reloads', async ({ page }) => {
  await page.goto('/');
  // The frame is a page of the gallery, at the gallery's own path, not at the app's `/`.
  await page.evaluate(() => history.replaceState(null, '', '/design/?frame=hosted&theme=light'));
  let navigations = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) navigations++;
  });
  await page.evaluate(async () => {
    const path = '/test/gallery-hosted-browser.tsx';
    await (await import(path)).mountHostedFrame();
  });
  await expect(page.getByText('A route module mounted by the host')).toBeVisible();
  await page.waitForTimeout(3000);
  expect(navigations).toBe(0);
  expect(await page.evaluate(() => location.pathname + location.search)).toBe(
    '/design/?frame=hosted&theme=light',
  );
});
