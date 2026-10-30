import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

// #5073: the owner's pinned projects, written from Manage projects and a project's list heading.
// Since #5662 the side menu is a fixed list and lists no project, whatever the owner pins. The
// fixture reports what the owner would.
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
}
const app = (page: Page) => page.locator('#list-fixture');
const moduleMenu = (page: Page) => app(page).locator('.tmt-ui-rail-sub');
/** Scope the list to a project from its filter. */
const scopeTo = (page: Page, name: string) =>
  app(page)
    .locator('.page-scope-filters .project-filter-button')
    .filter({ has: page.locator('.page-scope-name', { hasText: new RegExp(`^${name}$`) }) })
    .click();
const projectActions = (page: Page) =>
  app(page).getByRole('button', { name: 'Project actions', exact: true });
/** All projects… from the ⋯ beside the project filter. */
async function openManager(page: Page) {
  await projectActions(page).click();
  await page.getByRole('menuitem', { name: 'All projects…', exact: true }).click();
}
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

      test('the side menu lists no project, pinned, recent or scoped', async ({ page }) => {
        await mount(page, 'projects', ['Project 03', 'Project 01'], '/?scope=Project%2011');
        await expect(moduleMenu(page).locator('a[data-id="all"]')).toHaveCount(1);
        await expect(moduleMenu(page).locator('a[data-id^="project:"]')).toHaveCount(0);
        await expect(moduleMenu(page)).not.toContainText('Project');
        await mount(page, 'projects', 'automatic');
        await expect(moduleMenu(page)).not.toContainText('Recent projects');
        await expect(moduleMenu(page)).not.toContainText('Project');
      });

      test('until the owner writes a list the five most recent projects are pinned, marked automatic', async ({
        page,
      }) => {
        await mount(page, 'projects', 'automatic');
        // The first explicit change keeps what was pinned: unpinning one leaves the other four.
        await openManager(page);
        await expect(manager(page)).toBeVisible();
        await expect(manager(page).getByText(/because you used it recently/)).toBeVisible();
        await manager(page).getByRole('button', { name: 'Pin Project 02 to sidebar' }).click();
        await expect
          .poll(() => writes(page))
          .toEqual([
            { action: 'pins', names: ['Project 01', 'Project 03', 'Project 04', 'Project 05'] },
          ]);
        await manager(page).getByRole('button', { name: 'Close Manage projects' }).click();
        // Pinning when automatic with nothing else changed still writes the shown list.
        await mount(page, 'projects', 'automatic');
        await openManager(page);
        await manager(page).getByRole('button', { name: 'Pin Project 06 to sidebar' }).click();
        await expect
          .poll(() => writes(page))
          .toEqual([{ action: 'pins', names: [...names(5), 'Project 06'] }]);
      });

      test('Manage projects pins, unpins and reorders with buttons, pinned first, and keeps focus on the control', async ({
        page,
      }) => {
        await mount(page, 'projects', ['Project 04', 'Project 02']);
        await openManager(page);
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
      });

      test('a 21st pin is refused before any request, and a refused write changes nothing', async ({
        page,
      }) => {
        await mount(page, 'projects', names(20));
        await openManager(page);
        await manager(page).getByRole('button', { name: 'Pin Project 21 to sidebar' }).click();
        await expect(manager(page).getByRole('alert')).toContainText('at most 20 projects');
        expect(await writes(page)).toEqual([]);
        // A write the machine refuses keeps the list and says so.
        await manager(page).getByRole('button', { name: 'Pin Project 20 to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(1);
        await mount(page, 'projects', names(3));
        await page.evaluate(() => (window.__failManage = true));
        await openManager(page);
        await manager(page).getByRole('button', { name: 'Pin Project 02 to sidebar' }).click();
        await expect(manager(page).getByRole('alert')).toContainText('not saved');
      });

      test('the heading of a project pins it to the menu and unpins it again', async ({ page }) => {
        await mount(page, 'projects', ['Project 03']);
        await scopeTo(page, 'Project 03');
        const unpin = app(page).getByRole('button', { name: 'Unpin from sidebar' });
        await expect(unpin).toBeVisible();
        await unpin.click();
        await expect.poll(() => writes(page)).toEqual([{ action: 'pins', names: [] }]);
        // Unpinned, the heading offers the pin again.
        await expect(app(page).getByRole('button', { name: 'Pin to sidebar' })).toBeVisible();
        await app(page).getByRole('button', { name: 'Pin to sidebar' }).click();
        await expect.poll(async () => (await writes(page)).length).toBe(2);
        expect((await writes(page))[1]).toEqual({ action: 'pins', names: ['Project 03'] });
        await expect(app(page).getByRole('button', { name: 'Unpin from sidebar' })).toBeVisible();
        await page.screenshot({ path: await capturePath(`5073-heading-${width}-${theme}.png`) });
      });

      test('a read-only viewer sees the owner pins and no way to change them', async ({ page }) => {
        await mount(page, 'unmanaged', ['design']);
        await expect(projectActions(page)).toHaveCount(0);
        await scopeTo(page, 'design');
        await expect(app(page).getByRole('button', { name: /sidebar/ })).toHaveCount(0);
      });
    });
  }
