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
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`Remote snapshots, details, focus and settings ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      // The chip and its tooltip are the space home header's; the page header's More menu holds Remote
      // at every width (#4116), so the phone width exercises that path.
      await fixture(page, { home: width === 1440 });
      const header = page.locator('#remote-header-fixture header');
      const cluster = page.locator('#remote-header-fixture .tmt-ui-shell-tools');
      if (width === 1440) {
        const chip = cluster.getByRole('link', { name: 'Remote: 3 live sessions' });
        await expect(chip).toBeVisible();
        await expect.poll(() => readCount(page)).toBe(1);
        await capture(page, `count-3-${width}-${theme}`);
        await chip.hover();
        const tooltip = page.getByRole('tooltip');
        await expect(tooltip).toBeVisible();
        await expect(tooltip).toContainText('This browser · 1 session · active now');
        await expect(tooltip).toContainText('Browser add-on · 2 sessions · 1 min ago');
        await expect(tooltip).not.toContainText('CLI · 0 sessions');
        await expect(tooltip).toContainText(
          'As of 10:42 · counts live sessions; a closed tab can stay listed for about a minute.',
        );
        const rows = await tooltip.locator('.remote-details > div').allTextContents();
        expect(rows[0]).toBe('This browser · 1 session · active now');
        await expect.poll(() => readCount(page)).toBe(2);
        await capture(page, `tooltip-${width}-${theme}`);
        await page.keyboard.press('Escape');
        await expect(tooltip).toBeHidden();
        await page.mouse.move(0, 899);
        await chip.focus();
        await expect(tooltip).toBeVisible();
        await expect.poll(() => readCount(page)).toBe(3);
        await page.keyboard.press('Escape');
        await page.route('**/settings', (route) =>
          route.fulfill({ body: '<h1>Remote settings destination</h1>', contentType: 'text/html' }),
        );
        await chip.click();
        await expect(page).toHaveURL(new URL('/settings', page.url()).href);
      } else {
        await expect(
          page.locator('#remote-header-fixture').getByRole('link', { name: /^Remote:/ }),
        ).toHaveCount(0);
        await header.getByRole('button', { name: 'More', exact: true }).click();
        const row = page.getByRole('menuitem', { name: 'Remote: 3 live sessions' });
        await expect(row).toBeVisible();
        await capture(page, `more-${width}-${theme}`);
        await row.hover();
        await expect(page.getByText('This browser · 1 session · active now')).toBeVisible();
        await expect(page.getByText('CLI · 0 sessions · activity unknown')).toHaveCount(0);
        await expect(page.getByRole('menuitem', { name: 'Open Remote settings' })).toBeVisible();
        const submenu = page.locator('[data-slot="dropdown-menu-sub-content"]');
        const bounds = await submenu.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        // The menu's position rounds to whole pixels while its width keeps the font's fraction.
        expect(bounds!.x + bounds!.width).toBeLessThan(width + 0.5);
        expect(
          await submenu
            .locator('.remote-details')
            .evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true);

        await expect.poll(() => readCount(page)).toBe(2);
        await capture(page, `submenu-${width}-${theme}`);
        await page.keyboard.press('Escape');
        await expect(page.getByText('This browser · 1 session · active now')).toBeHidden();
        await page.keyboard.press('Escape');
        await expect(header.getByRole('button', { name: 'More', exact: true })).toBeFocused();
        await header.getByRole('button', { name: 'More', exact: true }).click();
        const source = page.getByRole('menuitem', { name: 'Source', exact: true });
        await source.evaluate((node: HTMLElement) => node.click());
        await expect(page.getByRole('dialog', { name: 'Source', exact: true })).toHaveCount(0);
        await (await pageAction(page, 'Source')).click();
        await expect(page.getByRole('dialog', { name: 'Source', exact: true })).toBeVisible();
        await page.getByRole('button', { name: 'Close Source', exact: true }).click();
        await expect(header.getByRole('button', { name: 'More', exact: true })).toBeFocused();
      }
    });
    test(`zero, one, unconfirmed and hidden stay distinct ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      for (const count of [0, 1]) {
        await fixture(page, { count, home: true });
        const header = page.locator('#remote-header-fixture header');
        const cluster = page.locator('#remote-header-fixture .tmt-ui-shell-tools');
        if (width === 390) await header.getByRole('button', { name: 'More', exact: true }).click();
        const control =
          width === 390
            ? page.getByRole('menuitem', {
                name: count ? 'Remote: 1 live session' : 'Remote: no live sessions',
              })
            : cluster.getByRole('link', {
                name: count ? 'Remote: 1 live session' : 'Remote: no live sessions',
              });
        await expect(control).toBeVisible();
        await expect(control).toContainText(`Remote · ${count}`);
        await capture(page, `count-${count}-${width}-${theme}`);
      }
      await fixture(page, { unconfirmed: true, home: true });
      const header = page.locator('#remote-header-fixture header');
      const cluster = page.locator('#remote-header-fixture .tmt-ui-shell-tools');
      if (width === 390) await header.getByRole('button', { name: 'More', exact: true }).click();
      const control =
        width === 390
          ? page.getByRole('menuitem', { name: 'Remote: session count unconfirmed' })
          : cluster.getByRole('button', { name: 'Remote: session count unconfirmed' });
      await expect(control).toHaveText('Remote · offline');
      await control.hover();
      await expect(page.getByText("Couldn't confirm live sessions right now.")).toBeVisible();
      await expect(page.getByRole('menuitem', { name: 'Open Remote settings' })).toHaveCount(0);
      await capture(page, `unconfirmed-${width}-${theme}`);
      await fixture(page, { hidden: true, home: true });
      await expect(page.locator('#remote-header-fixture').getByText(/^Remote ·/)).toHaveCount(0);
      expect(await readCount(page)).toBe(0);
      await capture(page, `hidden-${width}-${theme}`);
    });
  }

test('mount, visible and tooltip are the only triggers, and concurrent triggers join one read', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await fixture(page, { home: true });
  const chip = page.getByRole('link', { name: 'Remote: 3 live sessions' });
  await expect(chip).toBeVisible();
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
    (await import(path)).visibility(true);
  });
  await expect.poll(() => readCount(page)).toBe(2);
  await chip.hover();
  await expect(page.getByRole('tooltip')).toBeVisible();
  expect(await readCount(page)).toBe(2);
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    (await import(path)).resume();
  });
  await expect(page.getByRole('link', { name: 'Remote: 1 live session' })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.mouse.move(0, 899);
  await page.evaluate(async () => {
    const path = '/test/remote-header-browser.tsx';
    (await import(path)).refuse();
  });
  await page.getByRole('link', { name: 'Remote: 1 live session' }).hover();
  await expect(
    page.getByRole('button', { name: 'Remote: session count unconfirmed' }),
  ).toBeVisible();
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
  await expect(page.getByRole('menuitem', { name: 'Remote: 3 live sessions' })).toBeVisible();
  await expect(source).toBeFocused();
  expect(await original!.evaluate((node) => node.isConnected)).toBe(true);
  expect(await readCount(page)).toBe(1);
});

test('page-action lookup dismisses the modal Remote More before opening Discussion', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await fixture(page);
  const header = page.locator('#remote-header-fixture header');
  await header.getByRole('button', { name: 'More', exact: true }).click();
  await expect(await pageAction(page, 'Source')).toBeVisible();
  await expect(await pageAction(page, 'Chat')).toBeVisible();
  await expect(page.locator('[data-slot="dropdown-menu-content"]')).toBeHidden();
  await page.keyboard.press('Escape');
  await expect(header.getByRole('button', { name: /^Discussion \(/ })).toBeFocused();
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
  test(`the page header keeps no Remote chip: Remote is in More after Share…, and a Remote that cannot be confirmed shows a mark and a word at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await fixture(page);
    const header = page.locator('#remote-header-fixture header');
    await expect(
      page.locator('#remote-header-fixture').getByRole('link', { name: /^Remote:/ }),
    ).toHaveCount(0);
    await expect(header.locator('.page-attention')).toHaveCount(0);
    await header.getByRole('button', { name: 'More', exact: true }).click();
    const items = page.getByRole('menuitem');
    await expect(items.nth(0)).toHaveText(/^Share…/);
    await expect(items.nth(1)).toHaveAccessibleName('Remote: 3 live sessions');
    await expect(items.nth(2)).toHaveText('Agent status');
    await expect(items.nth(3)).toHaveText('Source');
    await page.keyboard.press('Escape');
    await fixture(page, { unconfirmed: true });
    const mark = header.locator('.page-attention[data-attention="remote"]');
    await expect(mark).toContainText('Remote offline');
    await expect(mark.locator('[aria-hidden]')).toHaveText('△');
    if (width < 480)
      expect((await mark.locator('.page-attention-word').boundingBox())!.width).toBeLessThanOrEqual(
        1,
      );
  });
