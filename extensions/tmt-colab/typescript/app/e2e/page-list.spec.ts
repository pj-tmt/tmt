import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

async function mount(page: Page, size: 'few' | 'bulk' | 'none' | 'unmanaged' = 'few', hash = '/') {
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
const requests = (page: Page) => page.evaluate(() => window.__labelRequests);

test('a label button filters the list, adds a token and removes it again', async ({ page }) => {
  await mount(page);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops']);
  const design = app(page).getByRole('button', { name: 'design, 2 pages' });
  await expect(design).toHaveAttribute('aria-pressed', 'false');
  await design.click();
  await expect(design).toHaveAttribute('aria-pressed', 'true');
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  await expect(app(page).locator('[data-label-filter="design"]')).toBeVisible();
  expect(new URL(page.url()).hash).toContain('label');
  await app(page).getByRole('button', { name: 'Remove label filter design' }).click();
  await expect(titles(page)).toHaveCount(4);
  await expect(design).toHaveAttribute('aria-pressed', 'false');
  // A page's own chip filters too.
  await rows(page).first().getByRole('button', { name: 'q4 plan' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan']);
});

for (const query of [
  'label=design',
  'by=Stu',
  'q=123',
  'sort=1',
  'archived=1',
  'label=%5B%22design%22%2C7%5D&by=%7B%7D',
]) {
  test(`a hand-written list link (${query}) drops what it cannot read and shows the list`, async ({
    page,
  }) => {
    await mount(page, 'few', `/?${query}`);
    await expect(titles(page)).toHaveCount(query.startsWith('label=%5B') ? 2 : 4);
    await expect(app(page).getByText('Something went wrong')).toHaveCount(0);
  });
}

test('keyboard removing a filter token keeps editor focus and applies the token removal', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  const remove = app(page).getByRole('button', { name: 'Remove label filter design' });
  await remove.focus();
  await remove.press('Enter');
  await expect(app(page).locator('[data-label-filter="design"]')).toHaveCount(0);
  await expect(titles(page)).toHaveCount(4);
  await expect(search(page)).toBeFocused();
  await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
  await remove.focus();
  await remove.press('Space');
  await expect(app(page).locator('[data-label-filter="design"]')).toHaveCount(0);
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

test('typing label: suggests names, Enter takes one, Enter again applies free text, Escape clears', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('label:des');
  // The suggestion list is a popover outside the fixture.
  const options = page.getByRole('option');
  await expect(options).toHaveText(['design (2)']);
  await page.keyboard.press('Enter');
  await expect(app(page).locator('[data-label-filter="design"]')).toBeVisible();
  // The token alone is a draft until Enter applies it.
  await expect(titles(page)).toHaveCount(4);
  await page.keyboard.type('beta');
  await page.keyboard.press('Enter');
  await expect(titles(page)).toHaveText(['Beta review']);
  expect(new URL(page.url()).hash).toContain('q=beta');
  await search(page).focus();
  await page.keyboard.press('Escape');
  await expect(titles(page)).toHaveCount(4);
  await expect(app(page).locator('[data-label-filter]')).toHaveCount(0);
  expect(new URL(page.url()).hash).not.toContain('label');
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
  // Creators combine with labels as an AND, and several creators as an OR.
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveCount(0);
  await app(page).getByRole('button', { name: 'Remove label filter ops' }).click();
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
    location.hash = '/today';
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

test('free text matches titles and labels, a miss says so and clears', async ({ page }) => {
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
    location.hash = '/today';
    location.hash = hash;
  }, hash);
  await expect(titles(page)).toHaveCount(2);
  await expect(app(page).locator('[data-label-filter="design"]')).toBeVisible();
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
  // The label row counts only what the toggle shows.
  await expect(app(page).getByRole('button', { name: 'design, 1 page' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'ops, 1 page' })).toHaveCount(0);
});

test('a reader gets search and filters but no label or list menus', async ({ page }) => {
  await mount(page, 'unmanaged');
  await expect(app(page).getByRole('button', { name: 'List options' })).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /^Labels for/ })).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /New label/ })).toHaveCount(0);
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveText(['Delta ops']);
});

test('empty spaces and filters say what is missing', async ({ page }) => {
  await mount(page, 'none');
  await expect(app(page).getByText('No pages in this space yet.')).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'Clear filters' })).toHaveCount(0);
});

test('the page menu adds and removes a label with one whole-set request each', async ({ page }) => {
  await mount(page);
  const gamma = rows(page).filter({ hasText: 'Gamma notes' });
  await gamma.getByRole('button', { name: 'Labels for Gamma notes' }).click();
  const design = page.getByRole('menuitemcheckbox', { name: 'design' });
  await expect(design).toHaveAttribute('aria-checked', 'false');
  await design.click();
  await expect(gamma.getByRole('button', { name: 'design', exact: true })).toBeVisible();
  await expect(app(page).getByRole('status')).toHaveText('Labels saved.');
  expect(await requests(page)).toEqual([
    {
      page: 'c0000003-1111-4111-8111-000000000001',
      changes: [{ pageId: 'c0000003-1111-4111-8111-000000000001', labels: ['design'] }],
    },
  ]);
  await gamma.getByRole('button', { name: 'Labels for Gamma notes' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'design' }).click();
  await expect(gamma.locator('.page-label')).toHaveCount(0);
  expect((await requests(page)).at(-1)!.changes[0].labels).toEqual([]);
});

test('a ninth label is refused before any request', async ({ page }) => {
  await mount(page);
  const alpha = rows(page).filter({ hasText: 'Alpha plan' });
  for (const name of ['a1', 'a2', 'a3', 'a4', 'a5', 'a6']) {
    await alpha.getByRole('button', { name: 'Labels for Alpha plan' }).click();
    await page.getByRole('menuitem', { name: 'New label…' }).click();
    await app(page).getByRole('textbox', { name: 'Label name' }).fill(name);
    await app(page).getByRole('checkbox', { name: 'Alpha plan' }).check();
    await app(page).getByRole('button', { name: 'Create' }).click();
    await expect(app(page).getByRole('dialog')).toHaveCount(0);
  }
  expect((await requests(page)).length).toBe(6);
  await expect(alpha.locator('.page-label')).toHaveCount(8);
  await alpha.getByRole('button', { name: 'Labels for Alpha plan' }).click();
  await expect(page.getByRole('menuitem', { name: 'New label…' })).toBeDisabled();
  await expect(page.getByRole('menuitemcheckbox', { name: 'ops' })).toBeDisabled();
  expect((await requests(page)).length).toBe(6);
});

test('a new label needs a name and a page, then lands on every ticked page in one request', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: '+ New label' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'New label' });
  const create = dialog.getByRole('button', { name: 'Create' });
  await expect(create).toBeDisabled();
  await expect(dialog).toContainText('Tick at least one page to create the label.');
  const name = dialog.getByRole('textbox', { name: 'Label name' });
  await name.fill(' bad');
  await expect(dialog).toContainText('Use single spaces inside the name');
  await name.fill('fresh start');
  await expect(create).toBeDisabled();
  await dialog.getByRole('checkbox', { name: 'Beta review' }).check();
  await dialog.getByRole('checkbox', { name: 'Gamma notes' }).check();
  await expect(create).toBeEnabled();
  await create.click();
  await expect(dialog).toHaveCount(0);
  const sent = await requests(page);
  expect(sent).toHaveLength(1);
  expect(sent[0].page).toBe('c0000002-1111-4111-8111-000000000001');
  expect(sent[0].changes).toEqual([
    { pageId: 'c0000002-1111-4111-8111-000000000001', labels: ['design', 'fresh start'] },
    { pageId: 'c0000003-1111-4111-8111-000000000001', labels: ['fresh start'] },
  ]);
  await expect(app(page).getByRole('button', { name: 'fresh start, 2 pages' })).toBeVisible();
});

test('rename reaches every page that carries the name in ascending 64-page requests and retries what is left', async ({
  page,
}) => {
  await mount(page, 'bulk');
  await app(page).getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitem', { name: 'Manage labels…' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'Manage labels' });
  await expect(dialog).toContainText('old');
  await dialog.getByRole('button', { name: 'Rename old' }).click();
  await dialog.getByRole('textbox', { name: 'New name' }).fill('current');
  // The count names every page, the archived one included.
  await expect(dialog).toContainText('Rename old to current on 131 pages (1 archived).');
  await page.evaluate(() => {
    window.__failLabelRequest = 1;
  });
  await dialog.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Updated 64 of 131 pages.');
  let sent = await requests(page);
  expect(sent.map((request) => request.changes.length)).toEqual([64, 64]);
  await dialog.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(dialog.getByRole('status')).toHaveText('Updated 67 pages.');
  sent = await requests(page);
  // The retry planned only the pages still carrying the name, as one 64 and one 3 request.
  expect(sent.map((request) => request.changes.length)).toEqual([64, 64, 64, 3]);
  for (const request of sent) {
    const ids = request.changes.map((change) => change.pageId);
    expect(ids).toEqual([...ids].sort());
    expect(request.page).toBe(ids[0]);
  }
  const everyPage = sent.flatMap((request) => request.changes);
  expect(new Set(everyPage.map((change) => change.pageId)).size).toBe(131);
  expect(
    everyPage.find((change) => change.pageId === 'c0000131-1111-4111-8111-000000000001'),
  ).toEqual({ pageId: 'c0000131-1111-4111-8111-000000000001', labels: ['current', 'z'] });
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(app(page).getByRole('button', { name: 'current, 130 pages' })).toBeVisible();
});

test('delete removes the name from every page and keeps the pages', async ({ page }) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitem', { name: 'Manage labels…' }).click();
  const dialog = app(page).getByRole('dialog', { name: 'Manage labels' });
  // The list shows the active pages (2 carry design); the dialog counts the archived one too.
  await expect(dialog.locator('.label-manage li').first()).toContainText('3 pages · 1 archived');
  await expect(dialog.locator('.label-manage li').nth(1)).toContainText('ops');
  await dialog.getByRole('button', { name: 'Delete design' }).click();
  await expect(dialog).toContainText('Delete design from 3 pages (1 archived). The pages stay.');
  await dialog.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(dialog.getByRole('status')).toHaveText('Updated 3 pages.');
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(app(page).getByRole('button', { name: /^design/ })).toHaveCount(0);
  await expect(titles(page)).toHaveCount(4);
  const sent = await requests(page);
  expect(sent).toHaveLength(1);
  expect(sent[0].changes.map((change) => change.labels)).toEqual([['q4 plan'], [], []]);
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
      await app(page).getByRole('button', { name: 'design, 2 pages' }).click();
      await app(page).getByRole('combobox', { name: 'Search pages' }).click();
      await page.keyboard.type('label:o');
      await expect(page.getByRole('option')).toHaveText(['ops (1)']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`2588-search-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      for (const _ of 'label:o') await page.keyboard.press('Backspace');
      await page.keyboard.type('by:');
      await expect(page.getByRole('option')).toHaveText(['Ada (1)', 'Studio agent (1)']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`2588-search-by-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: 'Labels for Alpha plan' }).click();
      await expect(page.getByRole('menuitemcheckbox', { name: 'design' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`2588-labels-menu-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: '+ New label' }).click();
      await app(page).getByRole('textbox', { name: 'Label name' }).fill('review');
      await page.screenshot({ path: await capturePath(`2588-new-label-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page).getByRole('button', { name: 'List options' }).click();
      await expect(page.getByRole('menuitemcheckbox', { name: 'Show archived' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`2588-gear-menu-${width}-${theme}.png`) });
      await page.getByRole('menuitem', { name: 'Manage labels…' }).click();
      await expect(app(page).getByRole('dialog', { name: 'Manage labels' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`2588-manage-${width}-${theme}.png`) });
    });
