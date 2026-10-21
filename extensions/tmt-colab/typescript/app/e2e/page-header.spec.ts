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
      expect(await names(page)).toEqual(['Search pages', 'Discussion (0)', 'Agents', 'More']);
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
