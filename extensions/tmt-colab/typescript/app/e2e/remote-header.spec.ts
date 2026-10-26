import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';

async function fixture(
  page: Page,
  input: {
    count?: number;
    hidden?: boolean;
    home?: boolean;
    unconfirmed?: boolean;
    pending?: boolean;
    management?: boolean;
  } = {},
) {
  await page.goto('/');
  await page.evaluate(async (input) => {
    const path = '/test/remote-header-browser.tsx';
    await (await import(path)).mount(input);
  }, input);
}
async function readCount(page: Page) {
  return page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    return (await import(path)).reads();
  });
}
async function capture(page: Page, name: string) {
  const directory = process.env.COLAB_REMOTE_HEADER_CAPTURES;
  if (!directory) return;
  mkdirSync(directory, { recursive: true });
  await page.screenshot({ path: `${directory}/${name}.png` });
}
/** The shared top bar's Remote link: Colab draws no chip, tooltip or More entry of its own. */
const remoteLink = (page: Page, name: string) =>
  page.locator('#remote-header-fixture .tmt-ui-topbar-common').getByRole('link', { name });
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`the bar's Remote link counts live sessions and opens Remote settings ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await fixture(page, { home: true });
      const link = remoteLink(page, 'Remote: 3 live sessions');
      await expect(link).toBeVisible();
      await expect(link.locator('.tmt-ui-count')).toHaveText('3');
      await expect.poll(() => readCount(page)).toBe(1);
      await capture(page, `count-3-${width}-${theme}`);
      // One Remote control, drawn by the bar: no chip, no tooltip rows, no second link.
      await expect(
        page.locator('#remote-header-fixture').getByRole('link', { name: /^Remote:/ }),
      ).toHaveCount(1);
      await expect(page.locator('#remote-header-fixture .remote-header-chip')).toHaveCount(0);
      await link.hover();
      await expect(page.getByText('This browser · 1 session · active now')).toHaveCount(0);
      expect(await readCount(page)).toBe(1);
      await page.route('**/settings', (route) =>
        route.fulfill({ body: '<h1>Remote settings destination</h1>', contentType: 'text/html' }),
      );
      await link.click();
      await expect(page).toHaveURL(new URL('/settings', page.url()).href);
    });
    test(`zero, one, unconfirmed and hidden stay distinct ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      for (const count of [0, 1]) {
        await fixture(page, { count, home: true });
        const link = remoteLink(
          page,
          count ? 'Remote: 1 live session' : 'Remote: no live sessions',
        );
        await expect(link).toBeVisible();
        // A positive count draws a badge; zero draws only the Remote glyph.
        if (count) await expect(link.locator('.tmt-ui-count')).toHaveText(String(count));
        else await expect(link.locator('.tmt-ui-count')).toBeHidden();
        await capture(page, `count-${count}-${width}-${theme}`);
      }
      // Without a confirmed count (the read failed, or there is no Remote source) the link is the
      // rail's Remote, with no badge and its own name.
      for (const input of [{ unconfirmed: true }, { hidden: true }]) {
        await fixture(page, { ...input, home: true });
        const link = remoteLink(page, 'Remote: session count unconfirmed');
        await expect(link).toBeVisible();
        await expect(link.locator('.tmt-ui-count')).toBeHidden();
        await expect(link).toHaveAttribute('href', /\/remote\/$/);
        await capture(page, `unconfirmed-${Object.keys(input)[0]}-${width}-${theme}`);
      }
      expect(await readCount(page)).toBe(0);
    });
  }

test('mount and visible are the only triggers, and concurrent triggers join one read', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fixture(page, { home: true });
  await expect(remoteLink(page, 'Remote: 3 live sessions')).toBeVisible();
  expect(await readCount(page)).toBe(1);
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    const f = await import(path);
    f.hold();
    f.setCount(1);
    f.visibility(false);
  });
  expect(await readCount(page)).toBe(1);
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    const f = await import(path);
    f.visibility(true);
    // A second visibility change while the read is held joins it.
    f.visibility(true);
  });
  await expect.poll(() => readCount(page)).toBe(2);
  // Hovering the link reads nothing: the bar's link has no tooltip of rows to refresh.
  await remoteLink(page, 'Remote: 3 live sessions').hover();
  expect(await readCount(page)).toBe(2);
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    (await import(path)).resume();
  });
  await expect(remoteLink(page, 'Remote: 1 live session')).toBeVisible();
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    const f = await import(path);
    f.refuse();
    f.visibility(true);
  });
  await expect(remoteLink(page, 'Remote: session count unconfirmed')).toBeVisible();
  expect(await readCount(page)).toBe(3);
});

test('a pending snapshot does not replace the open More menu or its focused row', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await fixture(page, { pending: true });
  const more = page.getByRole('button', { name: 'More', exact: true });
  await more.click();
  const source = page.getByRole('menuitem', { name: 'Source', exact: true });
  await source.focus();
  const original = await source.elementHandle();
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    (await import(path)).resume();
  });
  // The count reaches the bar's link, not the open menu, which has no Remote entry to replace.
  await expect(remoteLink(page, 'Remote: 3 live sessions')).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /^Remote:/ })).toHaveCount(0);
  await expect(source).toBeFocused();
  expect(await original!.evaluate((node) => node.isConnected)).toBe(true);
  expect(await readCount(page)).toBe(1);
});

test('page-action lookup finds an entry of the modal More that is already open', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await fixture(page);
  const bar = page.locator('#remote-header-fixture .tmt-ui-topbar');
  const more = bar.getByRole('button', { name: 'More', exact: true });
  await more.click();
  await expect(await pageAction(page, 'Source')).toBeVisible();
  await expect(
    page.locator('[data-slot="dropdown-menu-content"], .tmt-ui-menu-surface[open]').first(),
  ).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(more).toBeFocused();
});

test('Manage captures the persistent More opener after the shared menu closes', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await fixture(page, { management: true });
  await (await pageAction(page, 'Manage page')).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.getByRole('button', { name: 'More', exact: true })).toBeFocused();
});

for (const width of [1440, 390])
  test(`the page bar keeps no Remote chip and More no Remote entry, and a Remote that cannot be confirmed shows a mark and a word at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await fixture(page);
    const bar = page.locator('#remote-header-fixture .tmt-ui-topbar');
    await expect(remoteLink(page, 'Remote: 3 live sessions')).toBeVisible();
    await expect(bar.locator('.page-attention')).toHaveCount(0);
    await bar.getByRole('button', { name: 'More', exact: true }).click();
    const items = page.getByRole('menuitem');
    await expect(items.nth(0)).toHaveText(/^Share…/);
    await expect(items.nth(1)).toHaveText('Questions…');
    await expect(items.nth(2)).toHaveText('Set creator…');
    await expect(items.nth(3)).toHaveText('Agent status');
    await expect(items.nth(4)).toHaveText('Source');
    await expect(page.getByRole('menuitem', { name: /^Remote:/ })).toHaveCount(0);
    await page.keyboard.press('Escape');
    await fixture(page, { unconfirmed: true });
    const mark = bar.locator('.page-attention[data-attention="remote"]');
    await expect(mark).toContainText('Remote offline');
    await expect(mark.locator('[aria-hidden]')).toHaveText('△');
    if (width < 480)
      expect((await mark.locator('.page-attention-word').boundingBox())!.width).toBeLessThanOrEqual(
        1,
      );
  });
