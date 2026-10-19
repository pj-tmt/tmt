import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/back-browser.tsx';
async function mount(page: Page, start: string, scope?: string) {
  await page.goto('/');
  await page.evaluate(
    async ({ fixture, start, scope }) => (await import(fixture)).mount(start, scope),
    { fixture, start, scope },
  );
}
const app = (page: Page) => page.locator('#back-fixture');
const body = (page: Page, n: number) =>
  page.frameLocator('#back-fixture iframe').getByText(`Body of page ${n}`);

for (const scope of [undefined, 'design'])
  test(`Back from a page lands on the list (${scope ? 'a remembered scope' : 'no scope'})`, async ({
    page,
  }) => {
    await mount(page, '/colab/', scope);
    await app(page).getByRole('link', { name: 'Page 1' }).click();
    await expect(body(page, 1)).toBeVisible();
    await expect(page).toHaveURL(/\/p\/c0000001$/);
    await page.goBack();
    await expect(page).toHaveURL(/\/colab\/$/);
    await expect(app(page).getByRole('link', { name: 'Page 1' })).toBeVisible();
    await expect(app(page).getByRole('alert')).toHaveCount(0);
  });

test('Back from a list opened through a page entry keeps the whole trail', async ({ page }) => {
  await mount(page, '/p/c0000001');
  await expect(body(page, 1)).toBeVisible();
  await app(page)
    .getByRole('link', { name: /^Colab/ })
    .click();
  await app(page).getByRole('link', { name: 'Page 2' }).click();
  await expect(body(page, 2)).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/\/colab\/$/);
  await expect(app(page).getByRole('link', { name: 'Page 2' })).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/\/p\/c0000001$/);
  await expect(body(page, 1)).toBeVisible();
  await expect(app(page).getByRole('alert')).toHaveCount(0);
});
