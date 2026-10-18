import { expect, test, type Page } from '@playwright/test';

const ALPHA = 'c0000001-1111-4111-8111-000000000001';

async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-reentry-browser.tsx';
    (await import(/* @vite-ignore */ path)).mount();
  });
}
const app = (page: Page) => page.locator('#reentry-fixture');

test('entering a page again never shows the previous visit as Live while it loads', async ({
  page,
}) => {
  await mount(page);
  const link = app(page).getByRole('link', { name: 'Alpha plan' });
  await link.click();
  await expect(page).toHaveURL(new RegExp(`#/pages/${ALPHA}`));
  await expect(
    page.frameLocator('#reentry-fixture iframe').getByText('Body of Alpha plan.'),
  ).toBeVisible();
  await page.goBack();
  await expect(link).toBeVisible();

  // The second open is slow. The page that closed when the reader left must not stand in for it.
  let release!: () => void;
  await page.evaluate(() => {
    window.holdPages = new Promise<void>((resolve) => {
      (window as unknown as { releasePages: () => void }).releasePages = resolve;
    });
  });
  await link.click();
  await expect.poll(() => page.evaluate(() => window.pageOpens.length)).toBe(2);
  await page.waitForTimeout(300);
  await expect(page.locator('#reentry-fixture iframe')).toHaveCount(0);
  await expect(app(page).getByText('Private · Live')).toHaveCount(0);
  release = () =>
    page.evaluate(() => (window as unknown as { releasePages: () => void }).releasePages());
  await release();
  await expect(
    page.frameLocator('#reentry-fixture iframe').getByText('Body of Alpha plan.'),
  ).toBeVisible();
  await expect(app(page).getByText('Private · Live')).toBeVisible();
});
