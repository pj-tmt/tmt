import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

async function mount(
  page: Page,
  size: 'few' | 'bulk' | 'none' | 'unmanaged' | 'long' = 'few',
  hash = '/',
) {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(
    async ({ size, hash }) => {
      const path = '/test/page-list-browser.tsx';
      await (await import(path)).mountPageList(size, hash);
    },
    { size, hash },
  );
}
/** The fixture mounts beside the hidden app, which renders the same route; scope to the fixture. */
const app = (page: Page) => page.locator('#list-fixture');
const rows = (page: Page) => app(page).locator('.pages > li');
const titles = (page: Page) => rows(page).locator('.tmt-ui-list-title > a');
const search = (page: Page) => app(page).getByRole('combobox', { name: 'Search pages' });
const requests = (page: Page) => page.evaluate(() => window.__projectRequests);
const manageRequests = (page: Page) => page.evaluate(() => window.__projectManage);

test('a project button filters the list, adds a token and removes it again', async ({ page }) => {
  await mount(page);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops']);
  const design = app(page).getByRole('button', { name: 'design, 2 pages' });
  await expect(design).toHaveAttribute('aria-pressed', 'false');
  await design.click();
  await expect(design).toHaveAttribute('aria-pressed', 'true');
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  await expect(app(page).locator('[data-project-filter="design"]')).toBeVisible();
  expect(new URL(page.url()).hash).toContain('project');
  await app(page).getByRole('button', { name: 'Remove project filter design' }).click();
  await expect(titles(page)).toHaveCount(4);
  await expect(design).toHaveAttribute('aria-pressed', 'false');
  // A page's own chip filters too.
  await rows(page).first().getByRole('button', { name: 'q4 plan' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan']);
});

for (const query of [
  'project=design',
  'by=Stu',
  'q=123',
  'sort=1',
  'archived=1',
  'project=%5B%22design%22%2C7%5D&by=%7B%7D',
]) {
  test(`a hand-written list link (${query}) drops what it cannot read and shows the list`, async ({
    page,
  }) => {
    await mount(page, 'few', `/?${query}`);
    await expect(titles(page)).toHaveCount(query.startsWith('project=%5B') ? 2 : 4);
    await expect(app(page).getByText('Something went wrong')).toHaveCount(0);
  });
}

test('keyboard removing a filter token keeps editor focus and applies the token removal', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  const remove = app(page).getByRole('button', { name: 'Remove project filter design' });
  await remove.focus();
  await remove.press('Enter');
  await expect(app(page).locator('[data-project-filter="design"]')).toHaveCount(0);
  await expect(titles(page)).toHaveCount(4);
  await expect(search(page)).toBeFocused();
  await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
  await remove.focus();
  await remove.press('Space');
  await expect(app(page).locator('[data-project-filter="design"]')).toHaveCount(0);
  await expect(titles(page)).toHaveCount(4);
  await expect(search(page)).toBeFocused();
  // The same native chip contract applies to creator tokens introduced on main.
  for (const key of ['Enter', 'Space']) {
    await page.keyboard.type('by:stu');
    await expect(search(page)).toContainText('by:stu');
    await expect(page.getByRole('option')).toHaveText(['Studio agent (1)']);
    await page.keyboard.press('Enter');
    await expect(app(page).locator('[data-by-filter="Studio agent"]')).toBeVisible();
    await page.keyboard.press('Enter');
    await expect(titles(page)).toHaveText(['Alpha plan']);
    const authorRemove = app(page).getByRole('button', {
      name: 'Remove author filter Studio agent',
    });
    await authorRemove.focus();
    await authorRemove.press(key);
    await expect(app(page).locator('[data-by-filter]')).toHaveCount(0);
    await expect(titles(page)).toHaveCount(4);
    await expect(search(page)).toBeFocused();
  }
});

test('typing project: suggests names, Enter takes one, Enter again applies free text, Escape clears', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('project:des');
  // The suggestion list is a popover outside the fixture.
  const options = page.getByRole('option');
  await expect(options).toHaveText(['design (2)']);
  await page.keyboard.press('Enter');
  await expect(app(page).locator('[data-project-filter="design"]')).toBeVisible();
  // The token alone is a draft until Enter applies it.
  await expect(titles(page)).toHaveCount(4);
  await page.keyboard.type('beta');
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Beta review']);
  expect(new URL(page.url()).hash).toContain('q=beta');
  await search(page).focus();
  await page.keyboard.press('Escape');
  await expect(titles(page)).toHaveCount(4);
  await expect(app(page).locator('[data-project-filter]')).toHaveCount(0);
  expect(new URL(page.url()).hash).not.toContain('project');
});

test('a row shows the creator, the comment count and a proposal waiting for you', async ({
  page,
}) => {
  await mount(page);
  const metadata = (title: string) =>
    rows(page).filter({ hasText: title }).locator('.page-metadata');
  await expect(metadata('Alpha plan')).toContainText('by Studio agent');
  await expect(metadata('Alpha plan')).toContainText('3 comments');
  await expect(metadata('Beta review')).toContainText('1 comment');
  await expect(metadata('Beta review')).not.toContainText('1 comments');
  await expect(metadata('Delta ops')).toContainText('0 comments');
  // A separator leads the part after it, so a wrap can start a line with a dot and never end one.
  const separators = await metadata('Alpha plan')
    .locator('> *')
    .evaluateAll((parts) =>
      parts.map((part) => [
        getComputedStyle(part, '::before').content,
        getComputedStyle(part, '::after').content,
      ]),
    );
  expect(separators.map(([, after]) => after)).toEqual(['none', 'none', 'none']);
  expect(separators.map(([before]) => before !== 'none')).toEqual([false, true, true]);
  // The mark is the word with the diamond; only the page with a proposal awaiting the owner has it.
  await expect(app(page).locator('[data-waiting]')).toHaveCount(1);
  await expect(rows(page).filter({ hasText: 'Alpha plan' }).locator('[data-waiting]')).toHaveText(
    '◆ Waiting for you',
  );
  // A page the owner has not summarized yet shows what the list already knows, with no placeholder.
  const gamma = metadata('Gamma notes');
  await expect(gamma).not.toContainText('comment');
  await expect(gamma).not.toContainText('by ');
  await expect(gamma.locator('time')).toBeVisible();
});

test('typing by: suggests creators, a token filters by creator and free text matches them too', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('by:');
  await expect(page.getByRole('option')).toHaveText(['Ada (1)', 'Studio agent (1)']);
  await page.keyboard.type('stu');
  await expect(page.getByRole('option')).toHaveText(['Studio agent (1)']);
  await page.keyboard.press('Enter');
  await expect(app(page).locator('[data-by-filter="Studio agent"]')).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  expect(new URL(page.url()).hash).toContain('by');
  // Creators combine with projects as an AND, and several creators as an OR.
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveCount(0);
  await app(page).getByRole('button', { name: 'Remove project filter ops' }).click();
  await app(page).getByRole('button', { name: 'Remove author filter Studio agent' }).click();
  await expect(titles(page)).toHaveCount(4);
  await expect(app(page).locator('[data-by-filter]')).toHaveCount(0);
  await search(page).click();
  await page.keyboard.type('ada');
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Beta review']);
});

test('a multi-word creator keeps its space in the token and the address, and an unknown one finds nothing', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('by:stu');
  await page.keyboard.press('Enter');
  const token = app(page).locator('[data-by-filter="Studio agent"]');
  await expect(token).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  // The name travels through the address whole, so a reload restores the same token and rows.
  const hash = new URL(page.url()).hash;
  expect(new URLSearchParams(hash.slice(hash.indexOf('?') + 1)).get('by')).toBe('["Studio agent"]');
  await page.evaluate((hash) => {
    location.hash = '/blocked';
    location.hash = hash;
  }, hash);
  await expect(app(page).locator('[data-by-filter="Studio agent"]')).toBeVisible();
  await expect(titles(page)).toHaveText(['Alpha plan']);
  // A creator no page has is the ordinary empty result, not an error.
  await app(page).getByRole('button', { name: 'Remove author filter Studio agent' }).click();
  await search(page).click();
  await page.keyboard.type('by:nobody');
  await page.keyboard.press('Enter');
  await page.keyboard.press('Enter');
  await expect(rows(page)).toHaveCount(0);
  await expect(app(page).getByText('No pages match.')).toBeVisible();
});

test('free text matches titles and projects, a miss says so and clears', async ({ page }) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('q4');
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  await search(page).click();
  await page.keyboard.type(' nothing');
  await page.keyboard.press('Enter');
  await expect(rows(page)).toHaveCount(0);
  await expect(app(page).getByText('No pages match.')).toBeVisible();
  await app(page).getByRole('button', { name: 'Clear filters' }).click();
  await expect(titles(page)).toHaveCount(4);
  await expect(search(page)).toHaveText('');
});

test('the list keeps its filter in the address and restores it on reload', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
  await expect(titles(page)).toHaveCount(2);
  const hash = new URL(page.url()).hash;
  await page.evaluate((hash) => {
    location.hash = '/blocked';
    location.hash = hash;
  }, hash);
  await expect(titles(page)).toHaveCount(2);
  await expect(app(page).locator('[data-project-filter="design"]')).toBeVisible();
});

test('the home is the list: no Today or view tabs, and an old /all link lands on it with its search', async ({
  page,
}) => {
  await mount(page);
  await expect(app(page).locator('section.home h1')).toHaveText('Studio pages');
  await expect(app(page).getByRole('navigation', { name: 'Home views' })).toHaveCount(0);
  await expect(
    app(page).getByRole('heading', { name: /^Good (morning|afternoon|evening)$/ }),
  ).toHaveCount(0);
  await mount(page, 'few', `/all?project=${encodeURIComponent(JSON.stringify(['design']))}`);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  const hash = new URL(page.url()).hash;
  expect(hash).toMatch(/^#\/\?/);
  expect(hash).toContain('project');
  // Search it cannot read is dropped and the address is the plain home.
  await mount(page, 'few', '/all?project=design&q=123');
  await expect(titles(page)).toHaveCount(4);
  expect(new URL(page.url()).hash).toBe('#/');
});

test('the needs-you chip keeps the pages waiting on the viewer, combines with a project and survives reload', async ({
  page,
}) => {
  await mount(page);
  const needs = app(page).getByRole('button', { name: 'Needs you, 1 page' });
  await expect(needs).toHaveAttribute('aria-pressed', 'false');
  await needs.click();
  await expect(needs).toHaveAttribute('aria-pressed', 'true');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  expect(new URL(page.url()).hash).toContain('needs=true');
  // Pressed is neutral, even under the pointer that just clicked it; colour stays on the diamond.
  const look = await needs.evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      text: style.color,
      border: style.borderTopColor,
      mark: getComputedStyle(node.querySelector('.needs-filter-mark')!).color,
    };
  });
  expect(look.border).toBe(look.text);
  expect(look.mark).not.toBe(look.text);
  // It combines with the project filter: the waiting page is not in `ops`.
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveCount(0);
  await expect(app(page).getByText('No pages match.')).toBeVisible();
  await app(page).getByRole('button', { name: 'Remove project filter ops' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan']);
  const hash = new URL(page.url()).hash;
  await page.evaluate((hash) => {
    location.hash = '/blocked';
    location.hash = hash;
  }, hash);
  await expect(titles(page)).toHaveText(['Alpha plan']);
  await expect(needs).toHaveAttribute('aria-pressed', 'true');
  await needs.click();
  await expect(titles(page)).toHaveCount(4);
  expect(new URL(page.url()).hash).not.toContain('needs');
});

test('a copied needs-you link filters at once, and a value it cannot read shows every page', async ({
  page,
}) => {
  await mount(page, 'few', '/?needs=true');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  await mount(page, 'few', '/?needs=nope');
  await expect(titles(page)).toHaveCount(4);
  // With nothing waiting there is nothing to filter, so the chip stays out of the way.
  await mount(page, 'none');
  await expect(app(page).locator('[data-needs-filter]')).toHaveCount(0);
});

test('sort orders by title and the list menu shows archived pages', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'Sort: Recently updated' }).click();
  await app(page).getByRole('menuitemradio', { name: 'Title' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Delta ops', 'Gamma notes']);
  await app(page).getByRole('button', { name: 'Sort: Title' }).click();
  await app(page).getByRole('menuitemradio', { name: 'Recently updated' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops']);
  await app(page).getByRole('button', { name: 'List options' }).click();
  const archived = app(page).getByRole('menuitemcheckbox', { name: 'Show archived' });
  await expect(archived).toHaveAttribute('aria-checked', 'false');
  // Closed, the menu leaves no mark, so the status line says which pages the list shows.
  await expect(app(page).locator('.page-list-status')).toHaveText('');
  await archived.click();
  await expect(titles(page)).toHaveText(['Archived spec']);
  await expect(app(page).locator('.page-list-status')).toHaveText('Archived pages');
  // The project row counts only what the toggle shows.
  await expect(app(page).getByRole('button', { name: 'design, 1 page' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'ops, 1 page' })).toHaveCount(0);
});

test('a reader gets search and filters but no project or list menus', async ({ page }) => {
  await mount(page, 'unmanaged');
  await expect(app(page).getByRole('button', { name: 'List options' })).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /^Projects for/ })).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /New project/ })).toHaveCount(0);
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveText(['Delta ops']);
});

test('empty spaces and filters say what is missing', async ({ page }) => {
  await mount(page, 'none');
  await expect(app(page).getByText('No pages in this space yet.')).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'Clear filters' })).toHaveCount(0);
});

test('the page menu adds and removes a project with one whole-set request each', async ({
  page,
}) => {
  await mount(page);
  const gamma = rows(page).filter({ hasText: 'Gamma notes' });
  await gamma.getByRole('button', { name: 'Projects for Gamma notes' }).click();
  const design = page.getByRole('menuitemcheckbox', { name: 'design' });
  await expect(design).toHaveAttribute('aria-checked', 'false');
  await design.click();
  await expect(gamma.getByRole('button', { name: 'design', exact: true })).toBeVisible();
  await expect(app(page).getByRole('status')).toHaveText('Projects saved.');
  expect(await requests(page)).toEqual([
    {
      page: 'c0000003-1111-4111-8111-000000000001',
      changes: [{ pageId: 'c0000003-1111-4111-8111-000000000001', projects: ['design'] }],
    },
  ]);
  await gamma.getByRole('button', { name: 'Projects for Gamma notes' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'design' }).click();
  await expect(gamma.locator('.page-project')).toHaveCount(0);
  expect((await requests(page)).at(-1)!.changes[0].projects).toEqual([]);
});

test('a ninth project is refused before any request', async ({ page }) => {
  await mount(page);
  const alpha = rows(page).filter({ hasText: 'Alpha plan' });
  for (const name of ['a1', 'a2', 'a3', 'a4', 'a5', 'a6']) {
    await alpha.getByRole('button', { name: 'Projects for Alpha plan' }).click();
    await page.getByRole('menuitem', { name: 'New project…' }).click();
    await app(page).getByRole('textbox', { name: 'Project name' }).fill(name);
    await app(page).getByRole('checkbox', { name: 'Alpha plan' }).check();
    await app(page).getByRole('button', { name: 'Create' }).click();
    await expect(app(page).getByRole('dialog')).toHaveCount(0);
  }
  expect((await requests(page)).length).toBe(6);
  await expect(alpha.locator('.page-project')).toHaveCount(8);
  await alpha.getByRole('button', { name: 'Projects for Alpha plan' }).click();
  await expect(page.getByRole('menuitem', { name: 'New project…' })).toBeDisabled();
  await expect(page.getByRole('menuitemcheckbox', { name: 'ops' })).toBeDisabled();
  expect((await requests(page)).length).toBe(6);
});

test('a new project needs only a name; ticked pages land in it in one request', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: '+ New project' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'New project' });
  const create = dialog.getByRole('button', { name: 'Create' });
  await expect(create).toBeDisabled();
  await expect(dialog).toContainText('Optional: tick the pages to put in it.');
  const name = dialog.getByRole('textbox', { name: 'Project name' });
  await name.fill(' bad');
  await expect(dialog).toContainText('Use single spaces inside the name');
  await name.fill('design');
  await create.click();
  await expect(dialog.getByRole('alert')).toHaveText('That project already exists.');
  await name.fill('fresh start');
  await expect(create).toBeEnabled();
  await dialog.getByRole('checkbox', { name: 'Beta review' }).check();
  await dialog.getByRole('checkbox', { name: 'Gamma notes' }).check();
  await create.click();
  await expect(dialog).toHaveCount(0);
  const sent = await requests(page);
  expect(sent).toHaveLength(1);
  expect(sent[0].page).toBe('c0000002-1111-4111-8111-000000000001');
  expect(sent[0].changes).toEqual([
    { pageId: 'c0000002-1111-4111-8111-000000000001', projects: ['design', 'fresh start'] },
    { pageId: 'c0000003-1111-4111-8111-000000000001', projects: ['fresh start'] },
  ]);
  expect(await manageRequests(page)).toEqual([]);
  await expect(app(page).getByRole('button', { name: 'fresh start, 2 pages' })).toBeVisible();
});

test('a project with no page is one request, is listed everywhere, and takes pages later', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: '+ New project' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'New project' });
  await dialog.getByRole('textbox', { name: 'Project name' }).fill('backlog');
  await page.evaluate(() => {
    window.__failManage = true;
  });
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Projects were not saved');
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await manageRequests(page)).toEqual([
    { action: 'create', name: 'backlog' },
    { action: 'create', name: 'backlog' },
  ]);
  expect(await requests(page)).toEqual([]);
  // No page is in it, so the filter row does not list it; the page menu and the manager do.
  await expect(app(page).getByRole('button', { name: /^backlog/ })).toHaveCount(0);
  const gamma = app(page).locator('[data-page-id="c0000003-1111-4111-8111-000000000001"]');
  await gamma.getByRole('button', { name: 'Projects for Gamma notes' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'backlog' }).click();
  await expect(app(page).getByRole('button', { name: 'backlog, 1 page' })).toBeVisible();
  expect((await requests(page)).at(-1)!.changes).toEqual([
    { pageId: 'c0000003-1111-4111-8111-000000000001', projects: ['backlog'] },
  ]);
});

test('a new project save in flight vetoes Cancel, Escape and Close, then closes its own opening once', async ({
  page,
}) => {
  await mount(page);
  const open = app(page).getByRole('button', { name: '+ New project' });
  await open.click();
  const dialog = app(page).getByRole('dialog', { name: 'New project' });
  await dialog.getByRole('textbox', { name: 'Project name' }).fill('held');
  await page.evaluate(() => {
    window.__holdManage = true;
  });
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect.poll(() => page.evaluate(() => typeof window.__releaseManage)).toBe('function');
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  await page.keyboard.press('Escape');
  await dialog.getByRole('button', { name: 'Close New project' }).click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('textbox', { name: 'Project name' })).toHaveValue('held');
  await page.evaluate(() => window.__releaseManage!());
  await expect(dialog).toHaveCount(0);
  expect(await manageRequests(page)).toEqual([{ action: 'create', name: 'held' }]);
  await open.click();
  await expect(dialog.getByRole('textbox', { name: 'Project name' })).toHaveValue('');
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
});

test('rename is one request for every page in the project, archived included, and retries after a failure', async ({
  page,
}) => {
  await mount(page, 'bulk');
  await app(page).getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitem', { name: 'Manage projects…' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'Manage projects' });
  await expect(dialog).toContainText('old');
  await dialog.getByRole('button', { name: 'Rename old' }).click();
  await dialog.getByRole('textbox', { name: 'New name' }).fill('current');
  // The count names every page, the archived one included.
  await expect(dialog).toContainText('Rename old to current on 131 pages (1 archived).');
  await page.evaluate(() => {
    window.__failManage = true;
  });
  await dialog.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Projects were not saved');
  expect(await manageRequests(page)).toHaveLength(1);
  await dialog.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(dialog.getByRole('status')).toHaveText('Project updated.');
  expect(await manageRequests(page)).toEqual([
    { action: 'rename', name: 'old', to: 'current' },
    { action: 'rename', name: 'old', to: 'current' },
  ]);
  expect(await requests(page)).toEqual([]);
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(app(page).getByRole('button', { name: 'current, 130 pages' })).toBeVisible();
});

test('renaming onto an existing project says it merges', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitem', { name: 'Manage projects…' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'Manage projects' });
  await dialog.getByRole('button', { name: 'Rename ops' }).click();
  await dialog.getByRole('textbox', { name: 'New name' }).fill('design');
  await expect(dialog).toContainText(
    'A project named design already exists. Merge ops into design? 1 page moves; nothing is deleted.',
  );
  await expect(dialog.getByRole('button', { name: 'Rename', exact: true })).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Merge', exact: true }).click();
  await expect(dialog.getByRole('status')).toHaveText('Project updated.');
  expect(await manageRequests(page)).toEqual([{ action: 'rename', name: 'ops', to: 'design' }]);
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(app(page).getByRole('button', { name: 'design, 3 pages' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: /^ops/ })).toHaveCount(0);
});

test('delete removes the project in one request and keeps the pages', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitem', { name: 'Manage projects…' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'Manage projects' });
  // The list shows the active pages (2 are in design); the dialog counts the archived one too.
  await expect(dialog.locator('.project-manage li').first()).toContainText('3 pages · 1 archived');
  await expect(dialog.locator('.project-manage li').nth(1)).toContainText('ops');
  await dialog.getByRole('button', { name: 'Delete design' }).click();
  await expect(dialog).toContainText(
    'Delete the project design, now on 3 pages (1 archived). The pages stay.',
  );
  await dialog.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(dialog.getByRole('status')).toHaveText('Project updated.');
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(app(page).getByRole('button', { name: /^design/ })).toHaveCount(0);
  await expect(titles(page)).toHaveCount(4);
  expect(await manageRequests(page)).toEqual([{ action: 'delete', name: 'design' }]);
  expect(await requests(page)).toEqual([]);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the list v2 keeps one scroll axis and captures its states: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const flat = () =>
        page.evaluate(
          () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
        );
      await expect(titles(page)).toHaveCount(4);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`2588-list-${width}-${theme}.png`) });
      await app(page).getByRole('button', { name: 'Needs you, 1 page' }).click();
      await expect(titles(page)).toHaveCount(1);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3647-needs-${width}-${theme}.png`) });
      await app(page).getByRole('button', { name: 'Needs you, 1 page' }).click();
      await expect(titles(page)).toHaveCount(4);
      await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
      await app(page).getByRole('combobox', { name: 'Search pages' }).click();
      await page.keyboard.type('project:o');
      await expect(page.getByRole('option')).toHaveText(['ops (1)']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`2588-search-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      for (const _ of 'project:o') await page.keyboard.press('Backspace');
      await page.keyboard.type('by:');
      await expect(page.getByRole('option')).toHaveText(['Ada (1)', 'Studio agent (1)']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`2588-search-by-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: 'Projects for Alpha plan' }).click();
      await expect(page.getByRole('menuitemcheckbox', { name: 'design' })).toBeVisible();
      await page.screenshot({
        path: await capturePath(`2588-projects-menu-${width}-${theme}.png`),
      });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: '+ New project' }).click();
      await app(page).getByRole('textbox', { name: 'Project name' }).fill('review');
      await page.screenshot({ path: await capturePath(`2588-new-project-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: 'List options' }).click();
      await expect(page.getByRole('menuitemcheckbox', { name: 'Show archived' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`2588-gear-menu-${width}-${theme}.png`) });
      await page.getByRole('menuitem', { name: 'Manage projects…' }).click();
      await expect(app(page).getByRole('dialog', { name: 'Manage projects' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`2588-manage-${width}-${theme}.png`) });
      // Renaming onto an existing project says the two merge.
      const manager = app(page).getByRole('dialog', { name: 'Manage projects' });
      await manager.getByRole('button', { name: 'Rename ops' }).click();
      await manager.getByRole('textbox', { name: 'New name' }).fill('design');
      await expect(manager).toContainText('A project named design already exists.');
      await page.screenshot({ path: await capturePath(`2588-merge-${width}-${theme}.png`) });
    });

/** Nothing on the page reaches past the viewport, and the filter rows stay inside their column. */
async function insideViewport(page: Page) {
  const report = await page.evaluate(() => {
    const width = document.documentElement.clientWidth;
    const column = document
      .querySelector('#list-fixture .page-list-controls')!
      .getBoundingClientRect();
    const outside = [...document.querySelectorAll('#list-fixture *, [role="listbox"]')]
      .map((node) => ({ node, box: node.getBoundingClientRect() }))
      .filter(({ box }) => box.width > 0 && (box.right > width + 0.5 || box.left < -0.5))
      .map(({ node }) => `${node.tagName}.${(node as HTMLElement).className}`);
    const rows = [
      ...document.querySelectorAll('#list-fixture .project-filters, #list-fixture .page-projects'),
    ]
      .flatMap((row) => [...row.querySelectorAll('button')].map((b) => b.getBoundingClientRect()))
      .filter((box) => box.right > column.right + 0.5 || box.left < column.left - 0.5).length;
    return { outside, rows, scroll: document.documentElement.scrollWidth - width };
  });
  expect(report).toEqual({ outside: [], rows: 0, scroll: 0 });
}

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`the search field, its tokens and the project rows stay inside ${width}px ${theme} with the longest names`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'long');
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await expect(titles(page)).toHaveCount(5);
      // The long projects are filter buttons and row chips before any filter is typed.
      for (const name of ['W'.repeat(32), '漢'.repeat(32)])
        await expect(app(page).getByRole('button', { name: `${name}, 1 page` })).toBeVisible();
      await insideViewport(page);
      await search(page).click();
      for (const [typed, option] of [
        ['by:W', 'W'.repeat(120)],
        ['project:W', 'W'.repeat(32)],
        ['project:漢', '漢'.repeat(32)],
      ] as const) {
        await page.keyboard.type(typed);
        await expect(page.getByRole('option').filter({ hasText: option })).toBeVisible();
        await insideViewport(page);
        await page.keyboard.press('Enter');
      }
      await page.keyboard.type(` ${'z'.repeat(120)}`);
      await expect(app(page).locator('[data-project-filter]')).toHaveCount(2);
      await insideViewport(page);
      await page.screenshot({ path: await capturePath(`3745-search-${width}-${theme}.png`) });
    });
