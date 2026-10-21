import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

async function mount(
  page: Page,
  size: 'few' | 'bulk' | 'none' | 'unmanaged' | 'long' | 'full' = 'few',
  hash = '/',
  remembered?: string,
) {
  await page.goto('/');
  if (remembered !== undefined)
    await page.evaluate(
      (name) => localStorage.setItem('tmt-colab-project-scope-v1:', name),
      remembered,
    );
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
/** The header's project crumb: it names its value, and its menu holds the project actions. */
const scopeTrigger = (page: Page, value = 'All pages') =>
  app(page).getByRole('button', { name: `Project: ${value}` });
/** The crumb whatever its value, to open the menu from any scope. */
const anyScopeTrigger = (page: Page) => app(page).getByRole('button', { name: /^Project: / });
async function fromScopeMenu(page: Page, name: string) {
  await anyScopeTrigger(page).click();
  await page.getByRole('menuitem', { name }).click();
}
/** Scope the list to a project (or All pages) from the header menu. */
async function scopeTo(page: Page, name: string) {
  await anyScopeTrigger(page).click();
  await page.getByRole('menuitemradio', { name: new RegExp(`^${name}`) }).click();
}
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
    '◆ Waiting for you · 1',
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

test('sort orders by title; the Archived tab shows archived pages and counts them', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('button', { name: 'Sort: Recently updated' }).click();
  await app(page).getByRole('menuitemradio', { name: 'Title' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Delta ops', 'Gamma notes']);
  await app(page).getByRole('button', { name: 'Sort: Title' }).click();
  await app(page).getByRole('menuitemradio', { name: 'Recently updated' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops']);
  const all = app(page).getByRole('tab', { name: 'All', exact: true });
  const archived = app(page).getByRole('tab', { name: 'Archived 1' });
  await expect(all).toHaveAttribute('aria-selected', 'true');
  await expect(archived).toHaveAttribute('aria-selected', 'false');
  await archived.click();
  await expect(archived).toHaveAttribute('aria-selected', 'true');
  expect(new URL(page.url()).hash).toContain('archived=true');
  await expect(titles(page)).toHaveText(['Archived spec']);
  // The project row counts only what the tab shows.
  await expect(app(page).getByRole('button', { name: 'design, 1 page' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'ops, 1 page' })).toHaveCount(0);
  await all.click();
  await expect(titles(page)).toHaveCount(4);
  expect(new URL(page.url()).hash).not.toContain('archived');
});

const operations = (page: Page) => page.evaluate(() => window.__pageOperations);
const confirmDialog = (page: Page, name: string) => page.getByRole('dialog', { name });
const status = (page: Page) => app(page).locator('.page-list-status');

test('a page waiting on the viewer shows the ◆ in the waiting colour with the word and its count', async ({
  page,
}) => {
  await mount(page);
  const waiting = rows(page).filter({ hasText: 'Alpha plan' }).locator('[data-waiting]');
  await expect(waiting).toHaveText('◆ Waiting for you · 1');
  const colours = await app(page).evaluate((root) => {
    const colour = (selector: string) => getComputedStyle(root.querySelector(selector)!).color;
    return {
      rowMark: colour('[data-waiting] .page-waiting-mark'),
      word: colour('[data-waiting]'),
      chipMark: colour('.needs-filter-mark'),
    };
  });
  // The row's mark is the chip's mark; the word beside it is plain text colour.
  expect(colours.rowMark).toBe(colours.chipMark);
  expect(colours.word).not.toBe(colours.rowMark);
  // A row that waits is not otherwise marked: no rule, nothing that reads as a selection.
  await expect(rows(page).filter({ hasText: 'Alpha plan' })).not.toHaveAttribute('data-needs-you');
  await expect(rows(page).filter({ hasText: 'Beta review' }).locator('[data-waiting]')).toHaveCount(
    0,
  );
});

test('row actions show on hover and focus; a row opens in a new tab without leaving the list', async ({
  page,
}) => {
  await mount(page);
  const row = rows(page).filter({ hasText: 'Beta review' });
  const actions = row.locator('.tmt-ui-list-actions');
  await expect(actions).toHaveCSS('opacity', '0');
  await row.hover();
  await expect(actions).toHaveCSS('opacity', '1');
  await page.mouse.move(0, 0);
  await expect(actions).toHaveCSS('opacity', '0');
  await row.getByRole('link', { name: 'Beta review' }).focus();
  await expect(actions).toHaveCSS('opacity', '1');
  await row.getByRole('button', { name: 'Open Beta review in a new tab' }).click();
  expect(await page.evaluate(() => window.__opened)).toEqual([
    {
      url: expect.stringMatching(/#\/pages\/c0000002-1111-4111-8111-000000000001$/),
      target: '_blank',
      features: 'noopener',
    },
  ]);
  await expect(titles(page)).toHaveCount(4);
});

test('archiving from a row confirms, explains restoration, and sends one statement', async ({
  page,
}) => {
  await mount(page);
  const row = rows(page).filter({ hasText: 'Gamma notes' });
  await expect(row.getByRole('button', { name: 'Archive Gamma notes' })).toHaveAttribute(
    'aria-haspopup',
    'dialog',
  );
  await row.getByRole('button', { name: 'Archive Gamma notes' }).click();
  const dialog = confirmDialog(page, 'Archive “Gamma notes”?');
  await expect(dialog).toContainText(
    'An archived page stays readable, but nobody can edit it. You can restore it from Archived.',
  );
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
  expect(await operations(page)).toEqual([]);
  await row.getByRole('button', { name: 'Archive Gamma notes' }).click();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  expect(await operations(page)).toEqual([]);
  await row.getByRole('button', { name: 'Archive Gamma notes' }).click();
  await dialog.getByRole('button', { name: 'Archive page' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(status(page)).toHaveText('Archived “Gamma notes”.');
  expect(await operations(page)).toEqual([
    { operation: 'page.archive', pageId: 'c0000003-1111-4111-8111-000000000001' },
  ]);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Delta ops']);
  await expect(app(page).getByRole('tab', { name: 'Archived 2' })).toBeVisible();
});

test('deleting from the menu names the page, says it cannot be undone, and removes it', async ({
  page,
}) => {
  await mount(page);
  const row = rows(page).filter({ hasText: 'Delta ops' });
  await row.getByRole('button', { name: 'Actions for Delta ops' }).click();
  await expect(page.getByRole('menuitem', { name: 'Archive…' })).toBeVisible();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  const dialog = confirmDialog(page, 'Delete “Delta ops”?');
  await expect(dialog).toContainText(
    'The page, its comments and its attachments are deleted. This cannot be undone.',
  );
  await expect(dialog).not.toContainText('archive');
  await dialog.getByRole('button', { name: 'Delete page' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(status(page)).toHaveText('Deleted “Delta ops”.');
  expect(await operations(page)).toEqual([
    { operation: 'page.delete', pageId: 'c0000004-1111-4111-8111-000000000001' },
  ]);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review', 'Gamma notes']);
});

test('an archived row offers Restore and Delete while keeping archive unavailable', async ({
  page,
}) => {
  await mount(page);
  await app(page)
    .getByRole('tab', { name: /^Archived/ })
    .click();
  const row = rows(page).filter({ hasText: 'Archived spec' });
  await expect(row.getByRole('button', { name: /^Archive / })).toHaveCount(0);
  await row.getByRole('button', { name: 'Actions for Archived spec' }).click();
  await expect(page.getByRole('menuitem')).toHaveText(['Manage page', 'Restore…', 'Delete…']);
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  await confirmDialog(page, 'Delete “Archived spec”?')
    .getByRole('button', { name: 'Delete page' })
    .click();
  await expect(status(page)).toHaveText('Deleted “Archived spec”.');
  await expect(app(page).getByText('No archived pages.')).toBeVisible();
  await expect(app(page).getByRole('tab', { name: 'Archived 0' })).toBeVisible();
});

test('a refused statement changes nothing and says why; an unverified one is not called done', async ({
  page,
}) => {
  await mount(page);
  const row = rows(page).filter({ hasText: 'Gamma notes' });
  await page.evaluate(() => {
    window.__operationMode = 'refuse';
  });
  await row.getByRole('button', { name: 'Archive Gamma notes' }).click();
  await confirmDialog(page, 'Archive “Gamma notes”?')
    .getByRole('button', { name: 'Archive page' })
    .click();
  await expect(status(page)).toHaveText(
    'This device is not allowed to manage pages. Reopen your paired session.',
  );
  await expect(status(page)).toHaveAttribute('data-tone', 'error');
  expect(await operations(page)).toEqual([]);
  await expect(titles(page)).toHaveCount(4);
  await page.evaluate(() => {
    window.__operationMode = 'unverified';
  });
  await row.getByRole('button', { name: 'Archive Gamma notes' }).click();
  await confirmDialog(page, 'Archive “Gamma notes”?')
    .getByRole('button', { name: 'Archive page' })
    .click();
  await expect(status(page)).toContainText('not verified yet');
  await expect(status(page)).not.toHaveText(/^Archived/);
});

test('a project with only active pages says its archived view is empty, not that it has no pages', async ({
  page,
}) => {
  await mount(page, 'few', '/?scope=ops');
  await expect(titles(page)).toHaveText(['Delta ops']);
  await app(page)
    .getByRole('tab', { name: /^Archived/ })
    .click();
  await expect(app(page).getByText('No archived pages in ops.')).toBeVisible();
  await expect(app(page).getByText('No pages in ops yet.')).toHaveCount(0);
});

const selectRow = (page: Page, title: string) =>
  app(page).getByRole('checkbox', { name: `Select ${title}` });
const bulkBar = (page: Page) => app(page).getByRole('region', { name: 'Bulk actions' });

test('selecting pages swaps the search row for a bulk bar; Escape and Clear bring it back', async ({
  page,
}) => {
  await mount(page);
  await expect(bulkBar(page)).toHaveCount(0);
  await selectRow(page, 'Alpha plan').check();
  await expect(bulkBar(page).getByRole('status')).toHaveText('1 page selected');
  await selectRow(page, 'Beta review').check();
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
  // The bar stands where the search row stood, and the draft search survives it.
  await expect(search(page)).toHaveCount(0);
  await expect(rows(page).filter({ hasText: 'Alpha plan' })).toHaveAttribute('data-selected', '');
  await page.keyboard.press('Escape');
  await expect(bulkBar(page)).toHaveCount(0);
  await expect(search(page)).toBeVisible();
  await expect(selectRow(page, 'Alpha plan')).not.toBeChecked();
  await selectRow(page, 'Gamma notes').check();
  await bulkBar(page).getByRole('button', { name: 'Clear selection' }).click();
  await expect(bulkBar(page)).toHaveCount(0);
});

test('a filter that hides a selected page drops it from the selection', async ({ page }) => {
  await mount(page);
  await selectRow(page, 'Alpha plan').check();
  await selectRow(page, 'Delta ops').check();
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
  await scopeTo(page, 'design');
  await expect(bulkBar(page).getByRole('status')).toHaveText('1 page selected');
  await scopeTo(page, 'All pages');
  // Delta ops is shown again but is not selected again: what was hidden was dropped.
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
});

test('bulk add and remove send one whole-set request for the selected pages that change', async ({
  page,
}) => {
  await mount(page);
  await selectRow(page, 'Beta review').check();
  await selectRow(page, 'Gamma notes').check();
  await bulkBar(page).getByRole('button', { name: 'Add to project' }).click();
  await page.getByRole('menuitem', { name: 'q4 plan' }).click();
  await expect(status(page)).toHaveText('Added 2 pages to q4 plan.');
  expect(await requests(page)).toEqual([
    {
      page: 'c0000002-1111-4111-8111-000000000001',
      changes: [
        { pageId: 'c0000002-1111-4111-8111-000000000001', projects: ['design', 'q4 plan'] },
        { pageId: 'c0000003-1111-4111-8111-000000000001', projects: ['q4 plan'] },
      ],
    },
  ]);
  // Adding it where it already is changes nothing, and says so.
  await bulkBar(page).getByRole('button', { name: 'Add to project' }).click();
  await page.getByRole('menuitem', { name: 'q4 plan' }).click();
  await expect(status(page)).toHaveText('Nothing to change.');
  expect(await requests(page)).toHaveLength(1);
  await bulkBar(page).getByRole('button', { name: 'Remove from project' }).click();
  await page.getByRole('menuitem', { name: 'q4 plan' }).click();
  await expect(status(page)).toHaveText('Removed 2 pages from q4 plan.');
  expect((await requests(page)).at(-1)!.changes.map((c) => c.projects)).toEqual([['design'], []]);
});

test('a bulk add leaves a full page out and says so', async ({ page }) => {
  await mount(page, 'full');
  await selectRow(page, 'Full page').check();
  await selectRow(page, 'Gamma notes').check();
  await bulkBar(page).getByRole('button', { name: 'Add to project' }).click();
  await page.getByRole('menuitem', { name: 'extra' }).click();
  await expect(status(page)).toHaveText(
    'Added 1 page to extra. 1 page is in 8 projects already and was skipped.',
  );
  expect((await requests(page)).at(-1)!.changes).toHaveLength(1);
});

test('bulk archive confirms with the count and titles, then sends one statement per page', async ({
  page,
}) => {
  await mount(page);
  await selectRow(page, 'Beta review').check();
  await selectRow(page, 'Gamma notes').check();
  await bulkBar(page).getByRole('button', { name: 'Archive…' }).click();
  const dialog = confirmDialog(page, 'Archive 2 pages?');
  await expect(dialog.getByRole('listitem')).toHaveText(['Beta review', 'Gamma notes']);
  await expect(dialog).toContainText('You can restore them from Archived.');
  await dialog.getByRole('button', { name: 'Archive pages' }).click();
  await expect(status(page)).toHaveText('Archived 2 pages.');
  expect(await operations(page)).toEqual([
    { operation: 'page.archive', pageId: 'c0000002-1111-4111-8111-000000000001' },
    { operation: 'page.archive', pageId: 'c0000003-1111-4111-8111-000000000001' },
  ]);
  await expect(bulkBar(page)).toHaveCount(0);
  await expect(titles(page)).toHaveText(['Alpha plan', 'Delta ops']);
});

test('bulk delete in the Archived tab offers no archive and confirms the count', async ({
  page,
}) => {
  await mount(page);
  await app(page)
    .getByRole('tab', { name: /^Archived/ })
    .click();
  await selectRow(page, 'Archived spec').check();
  await expect(bulkBar(page).getByRole('button', { name: 'Archive…' })).toHaveCount(0);
  await bulkBar(page).getByRole('button', { name: 'Delete…' }).click();
  const dialog = confirmDialog(page, 'Delete “Archived spec”?');
  await expect(dialog).toContainText('This cannot be undone.');
  await dialog.getByRole('button', { name: 'Delete page' }).click();
  await expect(status(page)).toHaveText('Deleted “Archived spec”.');
});

test('a refusal stops the batch, names the page and keeps the rest selected', async ({ page }) => {
  await mount(page);
  await selectRow(page, 'Beta review').check();
  await selectRow(page, 'Gamma notes').check();
  await selectRow(page, 'Delta ops').check();
  await page.evaluate(() => {
    window.__operationMode = 'refuse-second';
  });
  await bulkBar(page).getByRole('button', { name: 'Delete…' }).click();
  await confirmDialog(page, 'Delete 3 pages?')
    .getByRole('button', { name: 'Delete pages' })
    .click();
  await expect(status(page)).toHaveText(
    'Updated 1 of 3 pages. “Gamma notes” was not changed: This device is not allowed to manage pages. Reopen your paired session.',
  );
  await expect(status(page)).toHaveAttribute('data-tone', 'error');
  expect(await operations(page)).toEqual([
    { operation: 'page.delete', pageId: 'c0000002-1111-4111-8111-000000000001' },
  ]);
  // The page that went is gone from the list; the two that did not are still selected.
  await expect(titles(page)).toHaveText(['Alpha plan', 'Gamma notes', 'Delta ops']);
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
});

test('list keys move, select, archive and open menus, and stay out of text fields, menus and dialogs', async ({
  page,
}) => {
  await mount(page);
  const link = (title: string) => rows(page).getByRole('link', { name: title, exact: true });
  // The search starts focused: a key typed there writes to it, and Escape on an empty search
  // hands the keys back to the list.
  await expect(search(page)).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(search(page)).not.toBeFocused();
  await page.keyboard.press('j');
  await expect(link('Alpha plan')).toBeFocused();
  await page.keyboard.press('j');
  await expect(link('Beta review')).toBeFocused();
  await page.keyboard.press('k');
  await expect(link('Alpha plan')).toBeFocused();
  await page.keyboard.press('k');
  await expect(link('Alpha plan')).toBeFocused();
  await page.keyboard.press('x');
  await expect(selectRow(page, 'Alpha plan')).toBeChecked();
  await page.keyboard.press('j');
  await page.keyboard.press('x');
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
  // With a selection, e archives it; the dialog is modal, so no key reaches the list.
  await page.keyboard.press('e');
  const dialog = confirmDialog(page, 'Archive 2 pages?');
  await expect(dialog).toBeVisible();
  await page.keyboard.press('x');
  await page.keyboard.press('j');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(bulkBar(page).getByRole('status')).toHaveText('2 pages selected');
  expect(await operations(page)).toEqual([]);
  await page.keyboard.press('Escape');
  await expect(bulkBar(page)).toHaveCount(0);
  // Without a selection, e archives the focused row, and l opens its projects menu.
  await link('Gamma notes').focus();
  await page.keyboard.press('l');
  await expect(page.getByRole('menuitemcheckbox', { name: 'design' })).toBeVisible();
  await page.keyboard.press('j');
  await page.keyboard.press('x');
  await expect(bulkBar(page)).toHaveCount(0);
  await page.keyboard.press('Escape');
  await link('Gamma notes').focus();
  await page.keyboard.press('e');
  await expect(confirmDialog(page, 'Archive “Gamma notes”?')).toBeVisible();
  await page.keyboard.press('Escape');
  // `/` focuses the search; typing a key there writes to it and selects nothing.
  await page.keyboard.press('/');
  await expect(search(page)).toBeFocused();
  await page.keyboard.type('xj');
  await expect(bulkBar(page)).toHaveCount(0);
  await expect(selectRow(page, 'Gamma notes')).not.toBeChecked();
});

test('a reader gets search, filters and the scope, but no project writes or archived tab', async ({
  page,
}) => {
  await mount(page, 'unmanaged');
  await expect(app(page).getByRole('tab')).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /^Projects for/ })).toHaveCount(0);
  await app(page).getByRole('button', { name: 'ops, 1 page' }).click();
  await expect(titles(page)).toHaveText(['Delta ops']);
  await scopeTrigger(page).click();
  await expect(page.getByRole('menuitemradio')).toHaveCount(4);
  await expect(page.getByRole('menuitem', { name: 'New project…' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Manage projects…' })).toHaveCount(0);
});

test('the header scope lists All pages and each project with its counts and needs-you', async ({
  page,
}) => {
  await mount(page);
  await expect(scopeTrigger(page)).toBeVisible();
  await scopeTrigger(page).click();
  const rows = page.getByRole('menuitemradio');
  await expect(rows).toHaveCount(4);
  // Active pages shown: All 4; design 2 (Alpha waits on the viewer); ops 1; q4 plan 1 (also waits).
  await expect(rows.nth(0)).toContainText('All pages');
  await expect(rows.nth(0)).toContainText('4');
  await expect(rows.nth(0)).toContainText('◆ 1');
  await expect(rows.nth(1)).toHaveText(/design\s*2\s*◆ 1/);
  await expect(rows.nth(2)).toHaveText(/ops\s*1\s*$/);
  await expect(rows.nth(3)).toHaveText(/q4 plan\s*1\s*◆ 1/);
  await expect(rows.nth(0)).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByRole('menuitem', { name: 'New project…' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Manage projects…' })).toBeVisible();
});

test('choosing a project scopes the list, the filter chips and the needs-you chip', async ({
  page,
}) => {
  await mount(page);
  await scopeTo(page, 'ops');
  await expect(scopeTrigger(page, 'ops')).toBeVisible();
  expect(new URL(page.url()).hash).toContain('scope=ops');
  await expect(titles(page)).toHaveText(['Delta ops']);
  // Only the scope's pages feed the chips: no design chip, and nothing waits in ops.
  await expect(app(page).getByRole('button', { name: /^design/ })).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: /^Needs you/ })).toHaveCount(0);
  await scopeTo(page, 'design');
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  await expect(app(page).getByRole('button', { name: 'Needs you, 1 page' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: 'q4 plan, 1 page' })).toBeVisible();
  // A project: token filters inside the scope; a project outside it matches nothing.
  await app(page).getByRole('button', { name: 'q4 plan, 1 page' }).click();
  await expect(titles(page)).toHaveText(['Alpha plan']);
  await search(page).fill('ops');
  await search(page).press('Enter');
  await expect(app(page).getByText('No pages match.')).toBeVisible();
  await scopeTo(page, 'All pages');
  await expect(scopeTrigger(page)).toBeVisible();
  expect(new URL(page.url()).hash).not.toContain('scope');
});

test('the scope is remembered on this device; All pages forgets it; a missing project opens All pages', async ({
  page,
}) => {
  await mount(page);
  await scopeTo(page, 'ops');
  expect(await page.evaluate(() => localStorage.getItem('tmt-colab-project-scope-v1:'))).toBe(
    'ops',
  );
  // Opening the list without a scope applies the remembered one, and the address shows it.
  await mount(page, 'few', '/', 'ops');
  await expect(scopeTrigger(page, 'ops')).toBeVisible();
  expect(new URL(page.url()).hash).toContain('scope=ops');
  await expect(titles(page)).toHaveText(['Delta ops']);
  await scopeTo(page, 'All pages');
  expect(await page.evaluate(() => localStorage.getItem('tmt-colab-project-scope-v1:'))).toBeNull();
  await mount(page);
  await expect(scopeTrigger(page)).toBeVisible();
  await expect(titles(page)).toHaveCount(4);
  // A remembered project that no longer exists opens All pages.
  await mount(page, 'few', '/', 'gone');
  await expect(scopeTrigger(page)).toBeVisible();
  await expect(titles(page)).toHaveCount(4);
  expect(new URL(page.url()).hash).not.toContain('scope');
});

test('a link to a scope the space no longer has shows All pages', async ({ page }) => {
  await mount(page, 'few', '/?scope=gone');
  await expect(scopeTrigger(page)).toBeVisible();
  await expect(titles(page)).toHaveCount(4);
});

test('a scoped project with no page says so, and the scope names it', async ({ page }) => {
  await mount(page);
  await fromScopeMenu(page, 'New project…');
  const dialog = app(page).getByRole('dialog', { name: 'New project' });
  await dialog.getByRole('textbox', { name: 'Project name' }).fill('backlog');
  await dialog.getByRole('button', { name: 'Create' }).click();
  await expect(dialog).toHaveCount(0);
  await scopeTo(page, 'backlog');
  await expect(scopeTrigger(page, 'backlog')).toBeVisible();
  await expect(app(page).getByText('No pages in backlog yet.')).toBeVisible();
});

test('a page opened from a scoped list shows the project it came from and links back to it', async ({
  page,
}) => {
  await mount(page);
  await scopeTo(page, 'design');
  await rows(page).first().getByRole('link', { name: 'Alpha plan' }).click();
  const crumb = app(page).getByRole('link', { name: 'design', exact: true });
  await expect(crumb).toBeVisible();
  await crumb.click();
  await expect(scopeTrigger(page, 'design')).toBeVisible();
  expect(new URL(page.url()).hash).toContain('scope=design');
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  // From the unscoped list, or by a copied link, no crumb: the page link stays clean.
  await scopeTo(page, 'All pages');
  await rows(page).first().getByRole('link', { name: 'Alpha plan' }).click();
  await expect(app(page).getByRole('link', { name: 'design', exact: true })).toHaveCount(0);
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
    await app(page).getByRole('checkbox', { name: 'Alpha plan', exact: true }).check();
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
  await fromScopeMenu(page, 'New project…');
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
  await fromScopeMenu(page, 'New project…');
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
  const open = () => fromScopeMenu(page, 'New project…');
  await open();
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
  await open();
  await expect(dialog.getByRole('textbox', { name: 'Project name' })).toHaveValue('');
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
});

test('rename is one request for every page in the project, archived included, and retries after a failure', async ({
  page,
}) => {
  await mount(page, 'bulk');
  await fromScopeMenu(page, 'Manage projects…');
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
  await fromScopeMenu(page, 'Manage projects…');
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
  await fromScopeMenu(page, 'Manage projects…');
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
      const beta = rows(page).filter({ hasText: 'Beta review' });
      await beta.hover();
      await page.screenshot({ path: await capturePath(`3746-row-actions-${width}-${theme}.png`) });
      await beta.getByRole('button', { name: 'Actions for Beta review' }).click();
      await expect(page.getByRole('menuitem', { name: 'Delete…' })).toBeVisible();
      await page.screenshot({ path: await capturePath(`3746-row-menu-${width}-${theme}.png`) });
      await page.getByRole('menuitem', { name: 'Delete…' }).click();
      const confirm = confirmDialog(page, 'Delete “Beta review”?');
      await expect(confirm).toBeVisible();
      expect(await flat()).toBe(true);
      await page.screenshot({
        path: await capturePath(`3746-delete-confirm-${width}-${theme}.png`),
      });
      await confirm.getByRole('button', { name: 'Cancel' }).click();
      await scopeTo(page, 'design');
      await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3747-scoped-list-${width}-${theme}.png`) });
      await rows(page).first().getByRole('link', { name: 'Alpha plan' }).click();
      const crumb = app(page).getByRole('link', { name: 'design', exact: true });
      if (width > 390) await expect(crumb).toBeVisible();
      else await expect(crumb).toBeHidden();
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3747-page-crumb-${width}-${theme}.png`) });
      if (width > 390) await crumb.click();
      else await page.goBack();
      await scopeTo(page, 'All pages');
      await expect(titles(page)).toHaveCount(4);
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
      await fromScopeMenu(page, 'New project…');
      await app(page).getByRole('textbox', { name: 'Project name' }).fill('review');
      await page.screenshot({ path: await capturePath(`2588-new-project-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await app(page)
        .getByRole('tab', { name: /^Archived/ })
        .click();
      await expect(titles(page)).toHaveText(['Archived spec']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3746-archived-tab-${width}-${theme}.png`) });
      await app(page).getByRole('tab', { name: 'All', exact: true }).click();
      await scopeTrigger(page).click();
      await page.screenshot({ path: await capturePath(`3747-scope-menu-${width}-${theme}.png`) });
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

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`the bulk bar, its menu and its confirmation stay inside ${width}px ${theme}`, async ({
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
      await selectRow(page, 'Alpha plan').check();
      await selectRow(page, 'Beta review').check();
      const phone = width < 480;
      const selected = phone ? '2 selected' : '2 pages selected';
      await expect(bulkBar(page).getByRole('status')).toHaveText(selected);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3746-bulk-bar-${width}-${theme}.png`) });
      if (phone) {
        // One row: the count, an Actions menu and Clear.
        const bar = await bulkBar(page).boundingBox();
        expect(bar!.height).toBeLessThan(80);
        await bulkBar(page).getByRole('button', { name: 'Actions' }).click();
        await expect(page.getByRole('menuitem')).toHaveText([
          'Add to project',
          'Remove from project',
          'Archive…',
          'Delete…',
        ]);
      } else {
        await bulkBar(page).getByRole('button', { name: 'Add to project' }).click();
        await expect(page.getByRole('menuitem', { name: 'New project…' })).toBeVisible();
      }
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3746-bulk-menu-${width}-${theme}.png`) });
      if (phone) {
        await page.getByRole('menuitem', { name: 'Add to project' }).click();
        const choice = page.getByRole('dialog', { name: 'Add to project' });
        await expect(choice.getByRole('button')).toContainText([
          'design',
          'ops',
          'q4 plan',
          'New project…',
        ]);
        expect(await flat()).toBe(true);
        await page.screenshot({
          path: await capturePath(`3746-bulk-choice-${width}-${theme}.png`),
        });
        await page.keyboard.press('Escape');
        await bulkBar(page).getByRole('button', { name: 'Actions' }).click();
        await page.getByRole('menuitem', { name: 'Delete…' }).click();
      } else {
        await page.keyboard.press('Escape');
        await bulkBar(page).getByRole('button', { name: 'Delete…' }).click();
      }
      const confirm = confirmDialog(page, 'Delete 2 pages?');
      await expect(confirm).toBeVisible();
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`3746-bulk-delete-${width}-${theme}.png`) });
      await confirm.getByRole('button', { name: 'Cancel' }).click();
      await expect(bulkBar(page).getByRole('status')).toHaveText(selected);
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

for (const width of [1440, 390, 320])
  for (const theme of ['light', 'dark'] as const)
    test(`the project crumb, its menu and the page crumb stay inside ${width}px ${theme} with the longest names`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'long');
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const inside = () =>
        page.evaluate(() => {
          const viewport = document.documentElement.clientWidth;
          const crumb = document
            .querySelector('#list-fixture button[aria-haspopup="menu"][aria-label^="Project: "]')!
            .getBoundingClientRect();
          return {
            scroll: document.documentElement.scrollWidth - viewport,
            crumbInside: crumb.left >= -0.5 && crumb.right <= viewport + 0.5,
          };
        });
      for (const long of ['W'.repeat(32), '漢'.repeat(32)]) {
        await anyScopeTrigger(page).click();
        const row = page.getByRole('menuitemradio', { name: new RegExp(`^${long}`) });
        await expect(row).toBeVisible();
        if (long.startsWith('W'))
          await page.screenshot({
            path: await capturePath(`3747-scope-long-${width}-${theme}.png`),
          });
        const box = await row.boundingBox();
        expect(box!.x + box!.width).toBeLessThanOrEqual(width + 0.5);
        await row.click();
        await expect(scopeTrigger(page, long)).toBeVisible();
        expect(await inside()).toEqual({ scroll: 0, crumbInside: true });
        await rows(page).first().getByRole('link', { name: 'Long names' }).click();
        // A heading too narrow for the crumb and the title leaves the crumb out; the title stays.
        const crumb = app(page).getByRole('link', { name: long, exact: true });
        if (width > 390) await expect(crumb).toBeVisible();
        else await expect(crumb).toBeHidden();
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
          ),
        ).toBe(0);
        if (long.startsWith('W'))
          await page.screenshot({
            path: await capturePath(`3747-page-crumb-long-${width}-${theme}.png`),
          });
        if (width > 390) await crumb.click();
        else await page.goBack();
        await expect(scopeTrigger(page, long)).toBeVisible();
      }
    });

test('restoring an Archived row confirms and returns it to the active list after verification', async ({
  page,
}) => {
  await mount(page);
  await app(page).getByRole('tab', { name: 'Archived 1' }).click();
  const row = rows(page).filter({ hasText: 'Archived spec' });
  await row.getByRole('button', { name: 'Restore “Archived spec”', exact: true }).click();
  const dialog = confirmDialog(page, 'Restore “Archived spec”?');
  await expect(dialog).toContainText(
    'This page returns to your active list and can be edited again. Its expiry starts over from today.',
  );
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect(await operations(page)).toEqual([]);
  await row.getByRole('button', { name: 'Actions for Archived spec' }).click();
  await page.getByRole('menuitem', { name: 'Restore…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Restore page', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(status(page)).toHaveText('Restored “Archived spec”.');
  expect(await operations(page)).toEqual([
    { operation: 'page.unarchive', pageId: 'c0000005-1111-4111-8111-000000000001' },
  ]);
  await expect(titles(page)).toHaveCount(0);
  await app(page).getByRole('tab', { name: 'All', exact: true }).click();
  await expect(titles(page)).toHaveCount(5);
  await expect(titles(page).filter({ hasText: 'Archived spec' })).toBeVisible();
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'])
    test(`restore confirmation stays readable at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await app(page).getByRole('tab', { name: 'Archived 1' }).click();
      const row = rows(page).filter({ hasText: 'Archived spec' });
      await row.getByRole('button', { name: 'Actions for Archived spec' }).click();
      await page.getByRole('menuitem', { name: 'Restore…', exact: true }).click();
      const dialog = confirmDialog(page, 'Restore “Archived spec”?');
      await expect(dialog.getByRole('button', { name: 'Restore page', exact: true })).toBeVisible();
      expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({ path: await capturePath(`3833-restore-${width}-${theme}.png`) });
      await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
      expect(await operations(page)).toEqual([]);
    });
