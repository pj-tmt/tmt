import { expect, test, type Page } from '@playwright/test';
import { openModuleNavigation } from '../test/home-list.js';
import { capturePath } from './captures.js';

// #5073: the Colab side menu lists the projects the owner pins, in the owner's order. Until the owner
// writes the list it shows the five most recently used projects; Manage projects and the heading of
// a project's list pin, unpin and reorder. The fixture reports what the owner would.
type Size = 'few' | 'unmanaged' | 'projects';
async function mount(page: Page, size: Size, pinned?: 'automatic' | string[], hash = '/') {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(
    async ({ size, hash, pinned }) => {
      const path = '/test/page-list-browser.tsx';
      await (await import(path)).mountPageList(size, hash, pinned);
    },
    { size, hash, pinned },
  );
  await openModuleNavigation(page, app(page));
}
const app = (page: Page) => page.locator('#list-fixture');
const moduleMenu = (page: Page) => app(page).locator('.tmt-ui-rail-sub:visible');
/** The menu's project entries, in order, by name. */
const listed = async (page: Page) => {
  const menu = await openModuleNavigation(page, app(page));
  return menu.locator('a[data-id^="project:"] .tmt-ui-module-menu-label').allTextContents();
};
const menuEntry = (page: Page, name: string) =>
  moduleMenu(page)
    .locator('a')
    .filter({
      has: page.locator('.tmt-ui-module-menu-label', { hasText: new RegExp(`^${name}$`) }),
    });
const manager = (page: Page) =>
  app(page).getByRole('dialog', { name: 'Manage projects', exact: true });
const writes = (page: Page) => page.evaluate(() => window.__projectManage);
const names = (count: number, from = 1) =>
  Array.from({ length: count }, (_, n) => `Project ${String(n + from).padStart(2, '0')}`);

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`${width} ${theme}`, () => {
      test.use({ viewport: { width, height: 900 }, colorScheme: theme });
      test.beforeEach(async ({ page }) => {
        await page.emulateMedia({ colorScheme: theme });
      });

      test('the menu lists the pinned projects in the owner order with their counts, then All projects and New project', async ({
        page,
      }) => {
        await mount(page, 'projects', ['Project 03', 'Project 01', 'Project 07']);
        await expect.poll(() => listed(page)).toEqual(['Project 03', 'Project 01', 'Project 07']);
        // Each entry keeps its page count.
        await expect(menuEntry(page, 'Project 03')).toContainText('1');
        // The entries that follow open Manage projects and the new-project dialog.
        const links = await moduleMenu(page).locator('a').allTextContents();
        const after = links.map((text) => text.trim());
        expect(after.some((text) => text.startsWith('All projects…'))).toBe(true);
        expect(after.some((text) => text.startsWith('New project…'))).toBe(true);
        expect(after.some((text) => text.startsWith('Manage projects…'))).toBe(false);
        // The 19 others are not in the menu, and nothing overflows the window.
        await expect(moduleMenu(page).locator('a[data-id^="project:"]')).toHaveCount(3);
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
          ),
        ).toBe(0);
        await page.screenshot({ path: await capturePath(`5073-rail-3-${width}-${theme}.png`) });
      });

      test('twelve pinned projects fill the menu and an empty chosen list lists none', async ({
        page,
      }) => {
        await mount(page, 'projects', names(12));
        await expect.poll(() => listed(page)).toEqual(names(12));
        await page.screenshot({ path: await capturePath(`5073-rail-12-${width}-${theme}.png`) });
        await mount(page, 'projects', []);
        await expect(moduleMenu(page).locator('a[data-id="all"]')).toBeVisible();
        await expect(moduleMenu(page).locator('a[data-id^="project:"]')).toHaveCount(0);
        await expect(moduleMenu(page).getByRole('link', { name: 'All projects…' })).toBeVisible();
        await page.screenshot({ path: await capturePath(`5073-rail-0-${width}-${theme}.png`) });
      });

      test('until the owner writes a list the menu shows the five most recent projects, marked automatic', async ({
        page,
      }) => {
        await mount(page, 'projects', 'automatic');
        await expect.poll(() => listed(page)).toEqual(names(5));
        if (width > 650)
          await expect(
            moduleMenu(page).getByText('Recent projects', { exact: true }),
          ).toBeVisible();
        // The first explicit change keeps what was showing: unpinning one leaves the other four.
        await menuEntry(page, 'All projects…').click();
        await expect(manager(page)).toBeVisible();
        await expect(manager(page).getByText(/because you used it recently/)).toBeVisible();
        await manager(page).getByRole('button', { name: 'Pin Project 02 to sidebar' }).click();
        await expect
          .poll(() => writes(page))
          .toEqual([
            { action: 'pins', names: ['Project 01', 'Project 03', 'Project 04', 'Project 05'] },
          ]);
        await manager(page).getByRole('button', { name: 'Close Manage projects' }).click();
        await expect
          .poll(() => listed(page))
          .toEqual(['Project 01', 'Project 03', 'Project 04', 'Project 05']);
        // It is the owner's list now: no longer marked automatic.
        if (width > 650)
          await expect(moduleMenu(page).getByText('Recent projects', { exact: true })).toHaveCount(
            0,
          );
        // Pinning when automatic with nothing else changed still writes the shown list.
        await mount(page, 'projects', 'automatic');
        await menuEntry(page, 'All projects…').click();
        await manager(page).getByRole('button', { name: 'Pin Project 06 to sidebar' }).click();
        await expect
          .poll(() => writes(page))
          .toEqual([{ action: 'pins', names: [...names(5), 'Project 06'] }]);
      });

      test('Manage projects pins, unpins and reorders with buttons, pinned first, and keeps focus on the control', async ({
        page,
      }) => {
        await mount(page, 'projects', ['Project 04', 'Project 02']);
        await menuEntry(page, 'All projects…').click();
        const rows = manager(page).locator('.project-manage > li');
        await expect(rows.first().locator('.project-manage-name')).toHaveText('Project 04');
        await expect(rows.nth(1).locator('.project-manage-name')).toHaveText('Project 02');
        await expect(rows.nth(2).locator('.project-manage-name')).toHaveText('Project 01');
        await expect(rows.first()).toContainText('Sidebar place 1 of 2');
        // The ends cannot move past themselves.
        await expect(
          manager(page).getByRole('button', { name: 'Move Project 04 up in the sidebar' }),
        ).toBeDisabled();
        await expect(
          manager(page).getByRole('button', { name: 'Move Project 02 down in the sidebar' }),
        ).toBeDisabled();
        // Move the second up: the request is the whole list, and the rows follow it.
        const up = manager(page).getByRole('button', { name: 'Move Project 02 up in the sidebar' });
        await up.click();
        await expect
          .poll(() => writes(page))
          .toEqual([{ action: 'pins', names: ['Project 02', 'Project 04'] }]);
        await expect(rows.first().locator('.project-manage-name')).toHaveText('Project 02');
        // It reached the top, so its Up is off and the focus moved to its Down.
        await expect(
          manager(page).getByRole('button', { name: 'Move Project 02 down in the sidebar' }),
        ).toBeFocused();
        // A pinned row's control is a pressed toggle and an unpinned row's is not; both keep one
        // name, so the state is carried by aria-pressed alone and no label changes.
        const toggle = (name: string) => manager(page).getByRole('button', { name });
        await expect(toggle('Pin Project 02 to sidebar')).toHaveAttribute('aria-pressed', 'true');
        await expect(toggle('Pin Project 09 to sidebar')).toHaveAttribute('aria-pressed', 'false');
        await expect(toggle('Pin Project 02 to sidebar')).toHaveText('');
        // The pressed look is the leaf's pressed state, which differs from an unpressed row's.
        const fill = (name: string) =>
          toggle(name).evaluate((node) => getComputedStyle(node).backgroundColor);
        expect(await fill('Pin Project 02 to sidebar')).not.toBe(
          await fill('Pin Project 09 to sidebar'),
        );
        // Pin another: it goes last.
        await manager(page).getByRole('button', { name: 'Pin Project 09 to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(2);
        expect((await writes(page))[1]).toEqual({
          action: 'pins',
          names: ['Project 02', 'Project 04', 'Project 09'],
        });
        await expect(
          manager(page).getByRole('button', { name: 'Pin Project 09 to sidebar' }),
        ).toBeFocused();
        // Unpin the first: the others keep their order.
        await manager(page).getByRole('button', { name: 'Pin Project 02 to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(3);
        expect((await writes(page))[2]).toEqual({
          action: 'pins',
          names: ['Project 04', 'Project 09'],
        });
        await page.screenshot({ path: await capturePath(`5073-manage-${width}-${theme}.png`) });
        await manager(page).getByRole('button', { name: 'Close Manage projects' }).click();
        await expect.poll(() => listed(page)).toEqual(['Project 04', 'Project 09']);
      });

      test('a 21st pin is refused before any request, and a refused write changes nothing', async ({
        page,
      }) => {
        await mount(page, 'projects', names(20));
        await menuEntry(page, 'All projects…').click();
        await manager(page).getByRole('button', { name: 'Pin Project 21 to sidebar' }).click();
        await expect(manager(page).getByRole('alert')).toContainText('at most 20 projects');
        expect(await writes(page)).toEqual([]);
        // A write the machine refuses keeps the list and says so.
        await manager(page).getByRole('button', { name: 'Pin Project 20 to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(1);
        await mount(page, 'projects', names(3));
        await page.evaluate(() => (window.__failManage = true));
        await menuEntry(page, 'All projects…').click();
        await manager(page).getByRole('button', { name: 'Pin Project 02 to sidebar' }).click();
        await expect(manager(page).getByRole('alert')).toContainText('not saved');
        await manager(page).getByRole('button', { name: 'Close Manage projects' }).click();
        await expect.poll(() => listed(page)).toEqual(names(3));
      });

      test('the current scope shows as a temporary row after the pins and leaves when you do', async ({
        page,
      }) => {
        await mount(page, 'projects', ['Project 03']);
        await expect.poll(() => listed(page)).toEqual(['Project 03']);
        // Open a project that is not pinned: it joins the menu, current, after the pinned ones.
        await mount(page, 'projects', ['Project 03'], '/?scope=Project%2011');
        await expect.poll(() => listed(page)).toEqual(['Project 03', 'Project 11']);
        await expect(menuEntry(page, 'Project 11')).toHaveAttribute('aria-current', 'page');
        await menuEntry(page, 'All pages').click();
        await expect.poll(() => listed(page)).toEqual(['Project 03']);
      });

      test('the heading of a project pins it to the menu and unpins it again', async ({ page }) => {
        await mount(page, 'projects', ['Project 03']);
        await menuEntry(page, 'Project 03').click();
        const unpin = app(page).getByRole('button', { name: 'Unpin from sidebar' });
        await expect(unpin).toBeVisible();
        await unpin.click();
        await expect.poll(() => writes(page)).toEqual([{ action: 'pins', names: [] }]);
        // Unpinned, it stays as the temporary row while it is the scope.
        await expect(app(page).getByRole('button', { name: 'Pin to sidebar' })).toBeVisible();
        await app(page).getByRole('button', { name: 'Pin to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(2);
        expect((await writes(page))[1]).toEqual({ action: 'pins', names: ['Project 03'] });
        await expect(app(page).getByRole('button', { name: 'Unpin from sidebar' })).toBeVisible();
        await page.screenshot({ path: await capturePath(`5073-heading-${width}-${theme}.png`) });
      });

      test('a read-only viewer sees the owner pins and no way to change them', async ({ page }) => {
        await mount(page, 'unmanaged', ['design']);
        await expect.poll(() => listed(page)).toEqual(['design']);
        await expect(moduleMenu(page).getByRole('link', { name: 'All projects…' })).toHaveCount(0);
        await menuEntry(page, 'design').click();
        await expect(app(page).getByRole('button', { name: /sidebar/ })).toHaveCount(0);
      });
    });
  }
