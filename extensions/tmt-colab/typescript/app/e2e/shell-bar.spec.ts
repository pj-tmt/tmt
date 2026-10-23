import { expect, test, type Page } from '@playwright/test';

/**
 * The shell's bar (#4558): the page's title with no module crumb, then the shell's own cluster, the
 * search and the Remote presence chip. The rail names the module; the bar's accessible name keeps it.
 */
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ method, argument }) => {
      const path = '/test/ask-page-browser.tsx';
      return (await import(path))[method](argument);
    },
    { method, argument },
  );
async function mountPage(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('.tmt-ui-shell-bar:visible header')).toBeVisible();
}
async function mountHome(page: Page, count = 3) {
  await page.goto('/');
  await page.evaluate(async (count) => {
    const path = '/test/remote-header-browser.tsx';
    await (await import(path)).mount({ home: true, count });
  }, count);
  await expect(page.locator('.tmt-ui-shell-bar:visible header')).toBeVisible();
}
// The ask-page fixture keeps a hidden earlier router beside the live one.
// The quick-switcher fixture mounts the header with a search that is allowed to open.
async function mountSwitcher(page: Page) {
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/quick-switcher-browser.tsx';
    await (await import(path)).mount('page');
  });
  await expect(page.locator('.tmt-ui-shell-bar:visible header')).toBeVisible();
}
const bar = (page: Page) => page.locator('.tmt-ui-shell-bar:visible');
const cluster = (page: Page) => page.locator('.tmt-ui-shell-tools:visible');
const search = (page: Page) =>
  cluster(page).getByRole('button', { name: 'Search pages', exact: true });
const dialog = (page: Page) => page.getByRole('dialog', { name: 'Search pages', exact: true });
const views = [
  { width: 1440, scheme: 'light' },
  { width: 390, scheme: 'dark' },
] as const;

for (const { width, scheme } of views) {
  const look = async (page: Page) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  };

  test(`the bar is the page title with no module crumb at ${width} ${scheme}`, async ({ page }) => {
    await look(page);
    await mountPage(page);
    const header = bar(page).locator('header');
    await expect(header.getByRole('heading', { level: 1 })).toHaveText('Live shared page');
    await expect(bar(page).locator('.tmt-ui-brand, .tmt-ui-wordmark, .tmt-ui-mark')).toHaveCount(0);
    // The rail's head carries the brand at a wide screen; the module is named by the rail and by
    // the header's accessible name, not drawn before the title.
    await expect(header).toHaveAttribute('aria-label', /^Colab · (.+ · )?Live shared page$/);
    expect((await bar(page).innerText()).trim().split('\n')[0]).toBe('Live shared page');
  });

  test(`the search opens from the shortcut once, with its key hint, and Escape returns focus at ${width} ${scheme}`, async ({
    page,
  }) => {
    await look(page);
    await mountSwitcher(page);
    const control = search(page);
    await expect(control).toBeVisible();
    await expect(control).toHaveAttribute('aria-keyshortcuts', /^(Meta|Control)\+K$/);
    const hint = control.locator('kbd');
    if (width === 1440) await expect(hint).toHaveText(/^(⌘K|Ctrl K)$/);
    else await expect(hint).toBeHidden();
    const shortcut = await page.evaluate(() =>
      /Mac|iPhone|iPad|iPod/.test(navigator.platform) ? 'Meta+k' : 'Control+k',
    );
    // From anywhere: with nothing focused the shortcut opens one dialog, and Escape closes it.
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await page.keyboard.press(shortcut);
    await expect(dialog(page)).toHaveCount(1);
    await expect(dialog(page)).toBeVisible();
    await expect(control).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('Escape');
    await expect(dialog(page)).toHaveCount(0);
    // Opened from the control, Escape returns focus to it.
    await control.focus();
    await page.keyboard.press(shortcut);
    await expect(dialog(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(dialog(page)).toHaveCount(0);
    await expect(control).toBeFocused();
  });
}

test('the search leads the cluster and the presence chip follows it, clear of the page actions at 1440', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mountHome(page);
  const control = search(page);
  const chip = cluster(page).getByRole('link', { name: 'Remote: 3 live sessions' });
  await expect(control).toBeVisible();
  await expect(chip).toBeVisible();
  const [box, chipBox, barBox] = await Promise.all([
    control.boundingBox(),
    chip.boundingBox(),
    bar(page).boundingBox(),
  ]);
  expect(box!.x).toBeLessThan(chipBox!.x);
  expect(chipBox!.x + chipBox!.width).toBeLessThanOrEqual(barBox!.x + barBox!.width);
  // The header leaves the cluster's room: none of its controls reaches under it.
  const clusterLeft = (await cluster(page).boundingBox())!.x;
  const reach = await bar(page).evaluate((node) =>
    Math.max(
      ...Array.from(node.querySelectorAll('header button, header a')).map(
        (item) => item.getBoundingClientRect().right,
      ),
    ),
  );
  expect(reach).toBeLessThanOrEqual(clusterLeft);
});

test('at 390 the presence chip is not drawn and the search stays in the bar', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mountHome(page);
  await expect(cluster(page).getByRole('link', { name: 'Remote: 3 live sessions' })).toBeHidden();
  const box = (await search(page).boundingBox())!;
  expect(box.x).toBeGreaterThan(0);
  expect(box.x + box.width).toBeLessThanOrEqual(390);
});

for (const mount of [mountPage, mountHome])
  test(`the bar does not overflow at 320 (${mount.name})`, async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 900 });
    await mount(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      320,
    );
    expect(await bar(page).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    const box = (await search(page).boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(320);
  });
