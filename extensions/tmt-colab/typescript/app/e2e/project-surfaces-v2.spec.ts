import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const app = (page: Page) => page.locator('#list-fixture');
async function mount(page: Page, width = 1586, theme: 'light' | 'dark' = 'light') {
  await page.setViewportSize({ width, height: 992 });
  await page.emulateMedia({ colorScheme: theme });
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(async (theme) => {
    document.documentElement.dataset.theme = theme;
    const fixture = '/test/page-list-browser.tsx';
    await (await import(fixture)).mountPageList('few');
  }, theme);
  await app(page)
    .locator('.tmt-ui-rail-sub:visible, .tmt-ui-module-menu:visible')
    .getByRole('link', { name: 'Manage projects…', exact: true })
    .click();
}

test('website manager opens a new project through the existing write owner', async ({ page }) => {
  await mount(page);
  const manager = app(page).getByRole('dialog', { name: 'Manage projects', exact: true });
  await expect(manager.getByRole('button', { name: 'Delete design', exact: true })).toHaveAttribute(
    'data-variant',
    'ghost',
  );
  const createAction = manager.getByRole('button', { name: 'New project', exact: true });
  await expect(createAction).toHaveText('New project');
  await expect(
    createAction.locator('.tmt-ui-action-mark[data-leading][aria-hidden="true"] svg'),
  ).toHaveCount(1);
  await expect(manager).toContainText('Keep related pages together.');
  const design = manager
    .locator('.project-manage li')
    .filter({ has: page.getByRole('button', { name: 'Rename design', exact: true }) });
  await expect(design).toContainText('2 active pages');
  await expect(design).toContainText('1 archived');
  await manager.getByRole('button', { name: 'New project', exact: true }).click();
  const create = app(page).getByRole('dialog', { name: 'New project', exact: true });
  await expect(app(page).locator('dialog:modal')).toHaveCount(1);
  await expect(create.getByRole('textbox', { name: 'Project name' })).toBeFocused();
  await expect(create.getByRole('group', { name: 'Initial pages' })).toBeVisible();
  await create.getByRole('textbox', { name: 'Project name' }).fill('research');
  await create.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(create).toHaveCount(0);
  expect(await page.evaluate(() => window.__projectManage)).toEqual([
    { action: 'create', name: 'research' },
  ]);
});

for (const width of [1586, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`website project surfaces stay contained at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      const manager = app(page).getByRole('dialog', { name: 'Manage projects', exact: true });
      await expect(manager).toBeVisible();
      await page.screenshot({ path: await capturePath(`projects-manager-${width}-${theme}.png`) });
      await manager.getByRole('button', { name: 'New project', exact: true }).click();
      const create = app(page).getByRole('dialog', { name: 'New project', exact: true });
      await expect(create.getByRole('group', { name: 'Initial pages' })).toBeVisible();
      expect(await create.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({ path: await capturePath(`projects-new-${width}-${theme}.png`) });
      await create.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(create).toHaveCount(0);
      await app(page)
        .locator('.tmt-ui-rail-sub:visible, .tmt-ui-module-menu:visible')
        .getByRole('link', { name: 'Manage projects…', exact: true })
        .click();
      await manager.getByRole('button', { name: 'Rename design', exact: true }).click();
      const rename = app(page).getByRole('dialog', { name: 'Rename project', exact: true });
      await rename.getByRole('textbox', { name: 'Project name' }).fill('research');
      await expect(rename).toContainText('3 pages (1 archived)');
      await page.screenshot({ path: await capturePath(`projects-rename-${width}-${theme}.png`) });
      await rename.getByRole('button', { name: 'Cancel', exact: true }).click();
      await manager.getByRole('button', { name: 'Delete design', exact: true }).click();
      const remove = app(page).getByRole('dialog', { name: 'Delete project design?', exact: true });
      await expect(app(page).locator('dialog:modal')).toHaveCount(1);
      await page.screenshot({ path: await capturePath(`projects-delete-${width}-${theme}.png`) });
      await remove.getByRole('button', { name: 'Cancel', exact: true }).click();
    });

test('project confirmations replace the manager and return focus to its card action', async ({
  page,
}) => {
  await mount(page);
  const manager = app(page).getByRole('dialog', { name: 'Manage projects', exact: true });
  await manager.getByRole('button', { name: 'Rename design', exact: true }).click();
  const rename = app(page).getByRole('dialog', { name: 'Rename project', exact: true });
  await expect(rename).toBeVisible();
  await expect(manager).toHaveCount(0);
  await expect(app(page).locator('dialog:modal')).toHaveCount(1);
  await expect(rename.getByRole('textbox', { name: 'Project name' })).toHaveValue('design');
  await expect(rename.getByRole('textbox', { name: 'Project name' })).toBeFocused();
  await rename.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(manager.getByRole('button', { name: 'Rename design', exact: true })).toBeFocused();
  await manager.getByRole('button', { name: 'Delete design', exact: true }).click();
  const remove = app(page).getByRole('dialog', { name: 'Delete project design?', exact: true });
  await expect(remove).toContainText('3 pages (1 archived)');
  await expect(app(page).locator('dialog:modal')).toHaveCount(1);
  await remove.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(manager.getByRole('button', { name: 'Delete design', exact: true })).toBeFocused();
  expect(await page.evaluate(() => window.__projectManage)).toEqual([]);
});

for (const kind of ['rename', 'delete'] as const)
  test(`${kind} confirmation retains modality while its owner write is pending`, async ({
    page,
  }) => {
    await mount(page);
    const manager = app(page).getByRole('dialog', { name: 'Manage projects', exact: true });
    await manager
      .getByRole('button', {
        name: `${kind === 'rename' ? 'Rename' : 'Delete'} design`,
        exact: true,
      })
      .click();
    const confirmation = app(page).getByRole('dialog', {
      name: kind === 'rename' ? 'Rename project' : 'Delete project design?',
      exact: true,
    });
    if (kind === 'rename')
      await confirmation.getByRole('textbox', { name: 'Project name' }).fill('research');
    await page.evaluate(() => {
      window.__holdManage = true;
    });
    await confirmation
      .getByRole('button', { name: kind === 'rename' ? 'Rename' : 'Delete', exact: true })
      .click();
    await expect.poll(() => page.evaluate(() => typeof window.__releaseManage)).toBe('function');
    await expect(confirmation.getByRole('button', { name: 'Cancel', exact: true })).toBeDisabled();
    await confirmation.press('Escape');
    await confirmation
      .getByRole('button', {
        name: `Close ${kind === 'rename' ? 'Rename project' : 'Delete project design?'}`,
        exact: true,
      })
      .click();
    await expect(confirmation).toBeVisible();
    await expect(manager).toHaveCount(0);
    await expect(app(page).locator('dialog:modal')).toHaveCount(1);
    await page.evaluate(() => window.__releaseManage!());
    await expect(confirmation).toHaveCount(0);
    await expect(manager.getByRole('status')).toHaveText('Project updated.');
    await expect(
      manager.getByRole('button', {
        name: kind === 'rename' ? 'Rename research' : 'New project',
        exact: true,
      }),
    ).toBeFocused();
    expect(await page.evaluate(() => window.__projectManage)).toEqual([
      kind === 'rename'
        ? { action: 'rename', name: 'design', to: 'research' }
        : { action: 'delete', name: 'design' },
    ]);
  });
