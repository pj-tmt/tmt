import { expect, test, type Page } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import { text } from '../src/strings.js';
const fixture = '/test/short-links-browser.tsx';
async function capture(page: Page, state: 'chooser' | 'deleted') {
  const directory = process.env.COLAB_SHORT_LINK_CAPTURE_DIR;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  for (const theme of ['light', 'dark']) {
    await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
      await page.screenshot({
        path: `${directory}/${state}-${width}-${theme}.png`,
        fullPage: true,
      });
    }
  }
}
test('an old short link opens a chooser after a new page collides and never guesses a page', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678'), fixture);
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await expect(page.getByRole('link', { name: /Original proposal/ })).toBeVisible();
  await expect(page.getByRole('link', { name: /Later proposal/ })).toBeVisible();
  await expect(page.locator('iframe')).toHaveCount(0);
  await expect(page.locator('#short-fixture header')).toHaveCount(1);
  // Inside the shell the rail carries the TMT mark; the header keeps its mark markup hidden.
  const mark = page.locator('#short-fixture .tmt-ui-header svg.tmt-ui-mark');
  await expect(mark).toBeHidden();
  await expect(mark).toHaveAttribute('aria-hidden', 'true');
  await expect(mark).toHaveText('');
  await expect(page.getByTestId('short-page-choice').locator('section')).toHaveAttribute(
    'role',
    'status',
  );
  await expect(
    page.getByTestId('short-page-choice').locator('.tmt-ui-notice-mark svg.lucide-diamond'),
  ).toBeVisible();
  const original = page.getByRole('link', { name: /Original proposal/ });
  await expect(original.locator('svg.lucide-arrow-up-right')).toBeVisible();
  await original.focus();
  await expect(original).toBeFocused();
  await expect(original.locator('.short-page-title')).toHaveCSS(
    'text-decoration-line',
    'underline',
  );
  await original.blur();
  await capture(page, 'chooser');
  await page.getByRole('link', { name: /Original proposal/ }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/12345678-0000-4000-8000-000000000001/);
});
test('a unique prefix canonicalizes to the full page route and an unknown prefix fails visibly', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678', false), fixture);
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/12345678-0000-4000-8000-000000000001/);
  await page.evaluate(async (path) => (await import(path)).mount('99999999'), fixture);
  await expect(page.getByRole('alert')).toContainText('Page unavailable');
  await expect(page.locator('#short-fixture iframe')).toHaveCount(0);
});

test('deleting the original page reserves old prefixes and never exposes its title or rebinds its link', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async (path) => (await import(path)).mount('12345678', false, true), fixture);
  await expect(page.getByRole('heading', { name: 'This page was deleted' })).toBeVisible();
  await expect(page.getByText('Original proposal')).toHaveCount(0);
  await expect(page.locator('[aria-disabled="true"]')).toContainText('Deleted page');
  await expect(page.getByTestId('short-page-choice').locator('section')).toHaveAttribute(
    'role',
    'alert',
  );
  await expect(
    page.getByTestId('short-page-choice').locator('.tmt-ui-notice-mark svg.lucide-x'),
  ).toBeVisible();
  await expect(page.locator('#short-fixture .tmt-ui-header')).toHaveCount(1);
  await expect(
    page.getByText('Its owner deleted it. Ask them for a new link if you still need it.'),
  ).toBeVisible();
  await capture(page, 'deleted');
  await page.getByRole('link', { name: 'Back to pages' }).click();
  await expect(page.locator('#short-fixture .home')).toBeVisible();
  await page.evaluate(async (path) => (await import(path)).mount('12345678', true, true), fixture);
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await expect(page.getByRole('link', { name: /Original proposal/ })).toHaveCount(0);
  await expect(page.getByRole('link', { name: /Later proposal/ })).toBeVisible();
  await expect(page.locator('[aria-disabled="true"]')).toContainText('Deleted page');
  await expect(page.locator('#short-fixture iframe')).toHaveCount(0);
});

for (const collision of [false, true])
  test(`an owner reading hint survives ${collision ? 'the prefix chooser' : 'unique prefix resolution'} and only highlights its passage`, async ({
    page,
  }) => {
    const read = '10000000-0000-4000-8000-000000000401';
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, collision, read }) =>
        (await import(fixture)).mount('12345678', collision, false, false, read),
      { fixture, collision, read },
    );
    if (collision) {
      await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
      await page.getByRole('link', { name: /Original proposal/ }).click();
    }
    const frame = page.frameLocator('#short-fixture iframe');
    await expect(frame.getByRole('heading', { name: 'Recommended passage' })).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`&read=${read}$`));
    await expect
      .poll(() => frame.locator('body').evaluate(() => CSS.highlights.has('colab-reading')))
      .toBe(true);
  });
// The public URL stays short while resolution uses the fresh admitted catalog.
test('public aliases resolve a freshly loaded catalog and disambiguate without a redirect loop', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(
    async (path) => (await import(path)).mount('12345678', true, false, true),
    fixture,
  );
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await page.getByRole('link', { name: /Original proposal/ }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/p\/12345678-0$/);
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
  await page.getByRole('link', { name: /Later proposal/ }).click();
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Later content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/p\/12345678-1$/);
});
test('a unique public alias resolves after startup without turning the address into a full page route', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(
    async (path) => (await import(path)).mount('12345678', false, false, true),
    fixture,
  );
  await expect(
    page.frameLocator('iframe').getByRole('heading', { name: 'Original content' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/p\/12345678$/);
});
for (const collision of [false, true])
  test(`a public reading hint survives ${collision ? 'the prefix chooser' : 'unique prefix resolution'} and only highlights its passage`, async ({
    page,
  }) => {
    const read = '10000000-0000-4000-8000-000000000401';
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, collision, read }) =>
        (await import(fixture)).mount('12345678', collision, false, true, read),
      { fixture, collision, read },
    );
    if (collision) {
      await expect(page.getByRole('heading', { name: 'Choose a page' })).toBeVisible();
      await page.getByRole('link', { name: /Original proposal/ }).click();
    }
    const frame = page.frameLocator('#short-fixture iframe');
    await expect(frame.getByRole('heading', { name: 'Recommended passage' })).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`/p/12345678${collision ? '-0' : ''}#read=${read}$`));
    await expect
      .poll(() => frame.locator('body').evaluate(() => CSS.highlights.has('colab-reading')))
      .toBe(true);
  });
/** A public link that names a page by a plain #id (the web's own form) scrolls to that element,
 * says so when it is not there, and a fragment that names nothing never blocks the page (#4014). */
for (const [name, fragment, element] of [
  ['a plain #id', '#digest-matrix', true],
  ['an #anchor= fragment', '#anchor=digest-matrix', true],
  ['a plain #id that is not on the page', '#missing-id', false],
] as const)
  test(`${name} opens the page${element ? ' at that element' : ' with a note'}`, async ({
    page,
  }) => {
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, fragment }) =>
        (await import(fixture)).mount('12345678', false, false, true, undefined, fragment),
      { fixture, fragment },
    );
    const frame = page.frameLocator('#short-fixture iframe');
    await expect(frame.getByRole('heading', { name: 'Original content' })).toBeVisible();
    if (element) {
      await expect(frame.locator('#digest-matrix')).toBeVisible();
      await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(2000);
      await expect(page.getByText(text.anchorMissing, { exact: true })).toHaveCount(0);
    } else {
      await expect(page.getByText(text.anchorMissing, { exact: true })).toBeVisible();
      expect(await page.evaluate(() => window.scrollY)).toBe(0);
    }
  });
for (const fragment of ['#%%%garbage', '#a=1&b=2', '#t=bad', '#read=bad'])
  test(`the fragment ${fragment} names nothing: the page opens at the top`, async ({ page }) => {
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, fragment }) =>
        (await import(fixture)).mount('12345678', false, false, true, undefined, fragment),
      { fixture, fragment },
    );
    await expect(
      page.frameLocator('#short-fixture iframe').getByRole('heading', { name: 'Original content' }),
    ).toBeVisible();
    await expect(page.getByText(text.anchorMissing, { exact: true })).toHaveCount(0);
    await expect(page.getByRole('alert')).toHaveCount(0);
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  });
