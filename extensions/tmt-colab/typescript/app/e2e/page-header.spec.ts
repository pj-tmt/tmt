import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * The lean page header (#4116): Search, Chat (Discussion) and Agents stay; a needs-you count and an
 * attention mark show only while their state holds; everything else is in More.
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
/** The header's entries from left to right, by accessible name. */
const names = (page: Page) =>
  page.evaluate(() => {
    // The fixture keeps a hidden earlier router beside the live one.
    const header = [...document.querySelectorAll('.tmt-ui-header')].find(
      (node) => node.getBoundingClientRect().width > 0,
    )!;
    const entries = [
      ...header.querySelectorAll<HTMLElement>(
        '.needs-you-header-trigger, .page-attention, .quick-switcher-control button, .page-header-actions button',
      ),
    ];
    return entries
      .filter((node) => node.getBoundingClientRect().width > 1)
      .sort((a, b) => a.getBoundingClientRect().left - b.getBoundingClientRect().left)
      .map((node) => node.getAttribute('aria-label') ?? node.textContent!.trim());
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`idle: only Search, Chat, Agents and More, with no state shown at ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme);
      const header = page.locator('.tmt-ui-header:visible');
      await expect(header.locator('.page-attention')).toHaveCount(0);
      await expect(header.locator('.needs-you-header-trigger')).toHaveCount(0);
      // Nothing waits and nothing shows; no Remote chip and no access chip.
      await expect(header.getByText('Nothing waits')).toBeHidden();
      await expect(header.locator('.remote-header-chip')).toHaveCount(0);
      // Search sits in the shell bar's cluster, to the right of the page's own actions.
      expect(await names(page)).toEqual(['Discussion (0)', 'Agents', 'More']);
      await expect(
        page
          .locator('.tmt-ui-shell-tools')
          .getByRole('button', { name: 'Search pages', exact: true }),
      ).toBeVisible();
      await expect(header.locator('.page-connection')).toHaveText('Private · Live');
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-idle-${width}-${theme}.png`) });
    });

    test(`needs-you 2, shared and Remote offline show as marks, in that order at ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme, { shared: true, remoteOffline: true, waiting: true });
      const header = page.locator('.tmt-ui-header:visible');
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
      expect(order.slice(-3)).toEqual(['Discussion (0)', 'Agents', 'More']);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-waiting-${width}-${theme}.png`) });
    });

    test(`More holds Share…, Remote, Source and Export, then the existing items at ${width} ${theme}`, async ({
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
        'Remote · offline',
        'Source',
        'Export page',
        'Manage page',
        'Theme: Light',
        'Theme: Dark',
        'Theme: System',
        'About this page',
      ]);
      await expect(page.getByRole('menuitem', { name: /^Files/ })).toHaveCount(0);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`header-more-${width}-${theme}.png`) });
    });
  }

/**
 * The shell's bar (#4558): the page's title with no module crumb, then the shell's own cluster, the
 * search and the Remote presence chip. The rail names the module; the bar's accessible name keeps it.
 */
test.describe('the shell bar', () => {
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
  // The ask-page fixture keeps a hidden earlier router beside the live one.
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

    test(`the bar is the page title with no module crumb at ${width} ${scheme}`, async ({
      page,
    }) => {
      await look(page);
      await mountPage(page);
      const header = bar(page).locator('header');
      await expect(header.getByRole('heading', { level: 1 })).toHaveText('Live shared page');
      await expect(bar(page).locator('.tmt-ui-brand, .tmt-ui-wordmark, .tmt-ui-mark')).toHaveCount(
        0,
      );
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

  test('at 390 the presence chip is not drawn and the search stays in the bar', async ({
    page,
  }) => {
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
});
