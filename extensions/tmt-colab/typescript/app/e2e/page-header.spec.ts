import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * The lean page actions in the shared top bar: Search (the bar's own) and the Discussion dock stay; a
 * needs-you count and an attention mark show only while their state holds; everything else is in More.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

async function open(
  page: Page,
  width: number,
  theme: 'light' | 'dark',
  options: { shared?: boolean; remoteOffline?: boolean; waiting?: boolean } = {},
) {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.goto('/');
  await run(page, 'mount', { shared: options.shared, remoteOffline: options.remoteOffline });
  if (options.waiting) {
    await run(page, 'review', { sections: 2 });
    await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
  } else {
    await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
  }
}
/** The bar's page entries from left to right, by accessible name. */
const names = (page: Page) =>
  page.evaluate(() => {
    // The fixture keeps a hidden earlier router beside the live one.
    const bar = [...document.querySelectorAll('.tmt-ui-topbar')].find(
      (node) => node.getBoundingClientRect().width > 0,
    )!;
    const entries = [
      ...bar.querySelectorAll<HTMLElement>(
        '.needs-you-header-trigger, .page-attention, .page-header-actions button',
      ),
    ];
    return entries
      .filter((node) => node.getBoundingClientRect().width > 1)
      .sort((a, b) => a.getBoundingClientRect().left - b.getBoundingClientRect().left)
      .map((node) => node.getAttribute('aria-label') ?? node.textContent!.trim());
  });

test('More groups the admitted page actions with the proposal labels', async ({ page }) => {
  await open(page, 1440, 'light', { shared: true });
  await page.getByRole('button', { name: 'More', exact: true }).click();
  const menu = page.locator('.tmt-ui-menu-surface[open]');
  await expect(menu).toBeVisible();
  for (const label of ['Share', 'Organize', 'Content', 'Page']) {
    await expect(menu.getByText(label, { exact: true })).toBeVisible();
  }
  await expect(menu.getByRole('menuitem', { name: /^Creator/ })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: /^Theme/ })).toHaveCount(0);
  await expect(menu.getByRole('menuitem', { name: /^Remote/ })).toHaveCount(0);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`idle: only Search, Discussion and More, with no state shown at ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme);
      const header = page.locator('.tmt-ui-topbar:visible');
      await expect(header.locator('.page-attention')).toHaveCount(0);
      await expect(header.locator('.needs-you-header-trigger')).toHaveCount(0);
      // Nothing waits and nothing shows; no Remote chip and no access chip.
      await expect(header.getByText('Nothing waits')).toBeHidden();
      await expect(header.locator('.remote-header-chip')).toHaveCount(0);
      // Search is the bar's own control, first at the left, before the page's actions.
      expect(await names(page)).toEqual(['More']);
      await expect(page.getByRole('button', { name: 'Search TMT', exact: true })).toBeVisible();
      await expect(header.locator('.page-connection')).toHaveText('Private · Live');
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-idle-${width}-${theme}.png`) });
    });

    test(`needs-you 2, shared and Remote offline show as marks, in that order at ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme, { shared: true, remoteOffline: true, waiting: true });
      const header = page.locator('.tmt-ui-topbar:visible');
      await expect(header.getByRole('button', { name: 'Needs you: 2', exact: true })).toBeVisible();
      const offline = header.locator('.page-attention[data-attention="remote"]');
      const shared = header.locator('.page-attention[data-attention="shared"]');
      await expect(offline).toContainText('Remote offline');
      await expect(shared).toContainText('Shared');
      // A phone keeps the glyph and hides the word visually, never from the accessible name.
      if (width < 480)
        for (const mark of [offline, shared])
          expect(
            (await mark.locator('.page-attention-word').boundingBox())!.width,
          ).toBeLessThanOrEqual(1);
      else
        for (const mark of [offline, shared])
          expect((await mark.locator('.page-attention-word').boundingBox())!.width).toBeGreaterThan(
            1,
          );
      const order = await names(page);
      expect(order[0]).toBe('Needs you: 2');
      expect(order.slice(1, 3).join(' ')).toMatch(/^△.*◉/s);
      expect(order.slice(-1)).toEqual(['More']);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-waiting-${width}-${theme}.png`) });
    });

    test(`More groups page actions without Theme or Remote at ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme, { shared: true, remoteOffline: true });
      await page.getByRole('button', { name: 'More', exact: true }).click();
      const menu = page.locator('[data-slot="dropdown-menu-content"], .tmt-ui-menu-surface[open]');
      await expect(menu.first()).toBeVisible();
      const rows = await page
        .getByRole('menuitem')
        .or(page.getByRole('menuitemradio'))
        .allInnerTexts();
      expect(rows.map((row) => row.replace(/\s+/g, ' ').trim())).toEqual([
        'Share… Link · Live',
        'Questions…',
        'Creator',
        'Source',
        'Export page',
        'Agent status',
        'Manage page',
        'About this page',
      ]);
      await expect(page.getByRole('menuitem', { name: /^Files/ })).toHaveCount(0);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-more-${width}-${theme}.png`) });
    });
  }

/**
 * The shared top bar (#4841, #5237): Search first at the left, then the page's actions, then Theme and
 * the Remote link. The page's title is in the bar's title slot; its status band is pinned under it.
 */
test.describe('the shell bar', () => {
  async function mountPage(page: Page, options: { subpage?: boolean } = {}) {
    await page.goto('/');
    await run(page, 'mount', options);
    await expect(page.locator('.tmt-ui-shell-bar:visible .tmt-ui-topbar')).toBeVisible();
  }
  async function mountHome(page: Page, count = 3) {
    await page.goto('/');
    await page.evaluate(async (count) => {
      const path = '/test/remote-header-browser.tsx';
      await (await import(path)).mount({ home: true, count });
    }, count);
    await expect(page.locator('.tmt-ui-shell-bar:visible .tmt-ui-topbar')).toBeVisible();
  }
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
    await expect(page.locator('.tmt-ui-shell-bar:visible .tmt-ui-topbar')).toBeVisible();
  }
  // The ask-page fixture keeps a hidden earlier router beside the live one.
  const bar = (page: Page) => page.locator('.tmt-ui-shell-bar:visible');
  const search = (page: Page) => page.getByRole('button', { name: 'Search TMT', exact: true });
  const common = (page: Page) => bar(page).locator('.tmt-ui-topbar-common');
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

    test(`the bar's title slot names the page and the status band stays pinned under it at ${width} ${scheme}`, async ({
      page,
    }) => {
      await look(page);
      await mountPage(page, { subpage: true });
      // The page's one heading is in the bar's left group; no title row or brand repeats it.
      const title = bar(page).locator('.tmt-ui-topbar-title');
      await expect(title.getByRole('heading', { level: 1 })).toHaveCount(1);
      await expect(title.locator('.tmt-ui-topbar-title-current')).toHaveText('Live shared page');
      await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1);
      await expect(page.locator('.colab-title-row')).toHaveCount(0);
      await expect(bar(page).locator('.tmt-ui-brand, .tmt-ui-wordmark, .tmt-ui-mark')).toHaveCount(
        0,
      );
      // The status band follows the bar with no phone tab strip, and stays at its bottom
      // after scrolling, collapsed and expanded.
      const band = page.locator('.page-main .work-band:visible');
      const gap = async () =>
        (await band.boundingBox())!.y -
        ((await bar(page).boundingBox())!.y + (await bar(page).boundingBox())!.height);
      if (width <= 900)
        await expect(page.locator('#ask-page-fixture .tmt-ui-module-menu')).toBeHidden();
      await expect.poll(gap).toBeCloseTo(0, 0);
      await page.evaluate(() => {
        document.querySelector<HTMLElement>('#ask-page-fixture .canvas')!.style.minHeight =
          '3000px';
        window.scrollTo(0, 400);
      });
      await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(400);
      await expect.poll(gap).toBeCloseTo(0, 0);
      const toggle = band.locator('.work-toggle');
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-expanded', 'true');
      await expect.poll(gap).toBeCloseTo(0, 0);
      const room = await page.evaluate(() => window.innerHeight);
      expect(
        (await band.boundingBox())!.y + (await band.boundingBox())!.height,
      ).toBeLessThanOrEqual(room + 1);
    });

    test(`the search opens from the shortcut once, with its key hint, and Escape returns focus at ${width} ${scheme}`, async ({
      page,
    }) => {
      await look(page);
      await mountSwitcher(page);
      const control = search(page);
      await expect(control).toBeVisible();
      // The bar owns the one shortcut and the one Search control.
      await expect(page.getByRole('button', { name: 'Search TMT', exact: true })).toHaveCount(1);
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

  test('Search leads the rail and Theme precedes Remote at 1440', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await mountHome(page);
    const control = search(page);
    const remote = common(page).getByRole('button', { name: 'Remote: 3 live sessions' });
    const theme = common(page).getByRole('button', { name: /^Theme:/ });
    const home = page.locator('nav.tmt-ui-rail:visible a[data-id="home"]');
    const [searchBox, homeBox, themeBox, remoteBox] = await Promise.all(
      [control, home, theme, remote].map((node) => node.boundingBox()),
    );
    expect(searchBox!.y + searchBox!.height).toBeLessThan(homeBox!.y);
    expect(themeBox!.x + themeBox!.width).toBeLessThanOrEqual(remoteBox!.x);
    await expect(page.locator('.remote-header-chip:visible')).toHaveCount(0);
    await expect(page.getByRole('button', { name: /^Remote:/ })).toHaveCount(1);
  });

  test('the rail mark paints from its own paint servers while a hidden shell shares the document', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    // The home fixture leaves the real app mounted (hidden) beside the visible shell: two rails.
    await mountHome(page);
    const result = await page.evaluate(() => {
      const marks = [...document.querySelectorAll<SVGElement>('.tmt-ui-shell-brand svg')];
      const visible = marks.find((mark) => mark.getBoundingClientRect().width > 0)!;
      const references = [...visible.innerHTML.matchAll(/url\(#([^)]+)\)/g)].map((m) => m[1]);
      return {
        marks: marks.length,
        references: references.length,
        outside: references.filter((id) => document.getElementById(id)?.closest('svg') !== visible),
        shared: marks
          .map((mark) => mark.querySelector('[id]')?.id)
          .filter((id, index, ids) => ids.indexOf(id) !== index),
      };
    });
    expect(result.marks).toBeGreaterThan(1);
    expect(result.references).toBeGreaterThan(0);
    expect(result.outside).toEqual([]);
    expect(result.shared).toEqual([]);
  });

  test('at 390 the Remote link and Search both stay in the bar', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await mountHome(page);
    const remote = common(page).getByRole('button', { name: 'Remote: 3 live sessions' });
    await expect(remote).toBeVisible();
    const box = (await search(page).boundingBox())!;
    expect(box.x).toBeGreaterThan(0);
    expect(box.x + box.width).toBeLessThanOrEqual(390);
    const remoteBox = (await remote.boundingBox())!;
    expect(remoteBox.x + remoteBox.width).toBeLessThanOrEqual(390);
    // No search hint on a phone: the control is its icon alone.
    await expect(search(page).locator('kbd')).toBeHidden();
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
});

test('the busiest page status and global controls fit at 320', async ({ page }) => {
  await open(page, 320, 'light', { shared: true, remoteOffline: true, waiting: true });
  const bar = page.locator('.tmt-ui-topbar:visible');
  const controls = [
    bar.getByRole('button', { name: 'Open navigation', exact: true }),
    bar.getByRole('button', { name: 'Search TMT', exact: true }),
    bar.locator('.page-header-meta'),
    bar.getByRole('button', { name: /^Theme:/ }),
    bar.getByRole('button', { name: /^Remote:/ }),
    bar.getByRole('button', { name: 'More', exact: true }),
  ];
  const boxes = await Promise.all(controls.map((control) => control.boundingBox()));
  for (let i = 1; i < boxes.length; i++)
    expect(boxes[i]!.x).toBeGreaterThanOrEqual(boxes[i - 1]!.x + boxes[i - 1]!.width);
  expect(boxes.at(-1)!.x + boxes.at(-1)!.width).toBeLessThanOrEqual(320);
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`grouped More review capture ${width} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await page.goto('/');
      await run(page, 'mount', {
        creator: true,
        attachments: true,
        projects: { names: ['design'], current: ['design'] },
      });
      await expect(
        page.frameLocator('#ask-page-fixture iframe').locator('#selected'),
      ).toBeVisible();
      await page.getByRole('button', { name: 'More', exact: true }).click();
      const menu = page.getByRole('menu', { name: 'More', exact: true });
      for (const label of ['Share', 'Organize', 'Content', 'Page'])
        await expect(menu.getByRole('group', { name: label, exact: true })).toBeVisible();
      await expect(menu.getByRole('menuitem', { name: 'Files 0', exact: true })).toBeVisible();
      for (const name of ['Share…', 'Files 0']) {
        const item = menu.getByRole('menuitem', { name, exact: true });
        const icon = await item.locator('.tmt-ui-menu-leading-icon > svg').boundingBox();
        const label = await item.locator('.tmt-ui-menu-label > span').first().boundingBox();
        expect(icon).not.toBeNull();
        expect(label).not.toBeNull();
        expect(icon!.width).toBe(16);
        expect(icon!.height).toBe(16);
        expect(
          Math.abs(icon!.y + icon!.height / 2 - label!.y - label!.height / 2),
          `${name}: icon ${JSON.stringify(icon)} label ${JSON.stringify(label)}`,
        ).toBeLessThanOrEqual(2);
      }
      const heading = menu.getByRole('group', { name: 'Organize' }).locator('.tmt-ui-menu-heading');
      await expect(heading).toHaveCSS('margin-top', '4px');
      await expect(heading).toHaveCSS('padding-top', '16px');
      await page.screenshot({ path: info.outputPath('open.png') });
      const files = menu.getByRole('menuitem', { name: 'Files 0', exact: true });
      await files.hover();
      await page.screenshot({ path: info.outputPath('hover.png') });
      await files.focus();
      await page.screenshot({ path: info.outputPath('focus.png') });
    });
