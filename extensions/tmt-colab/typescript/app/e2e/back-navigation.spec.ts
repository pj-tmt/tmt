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

// The tab and filters live in the history entry (#4052), so a reload of an owner address must not
// find them in it: the list is `#space=…` and never a `path=` the router cannot read back.
for (const start of ['/colab/', `/r/abcdefghijklmnop/x/colab/#space=${'a'.repeat(32)}`])
  test(`the list search never reaches the address (${start.startsWith('/colab/') ? 'public' : 'owner'})`, async ({
    page,
  }) => {
    await mount(page, start);
    const address = (path: string) =>
      page.evaluate(async ({ fixture, path }) => (await import(fixture)).addressOf(path), {
        fixture,
        path,
      });
    const bare = await address('/');
    expect(await address('/?stage=all')).toBe(bare);
    expect(await address('/?archived=true&q=notes&project=design')).toBe(bare);
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

const loads = (page: Page) =>
  page.evaluate(async (fixture) => (await import(fixture)).pageLoads(), fixture);
test('selecting a thread keeps the open page loaded, and entering a page again loads it afresh', async ({
  page,
}) => {
  await mount(page, '/p/c0000001');
  await expect(body(page, 1)).toBeVisible();
  expect(await loads(page)).toBe(1);
  // A thread selection only rewrites the address fragment: it must not replace the page's
  // binding, which would close its writer under a send in flight.
  await page.evaluate(
    async (fixture) => (await import(fixture)).selectThread('00000000-0000-4000-8000-000000000002'),
    fixture,
  );
  await expect(page).toHaveURL(/\/p\/c0000001#t=00000000-0000-4000-8000-000000000002$/);
  expect(await loads(page)).toBe(1);
  await expect(body(page, 1)).toBeVisible();
  await app(page)
    .getByRole('link', { name: /^Colab/ })
    .click();
  await app(page).getByRole('link', { name: 'Page 1' }).click();
  await expect(body(page, 1)).toBeVisible();
  expect(await loads(page)).toBe(2);
});
