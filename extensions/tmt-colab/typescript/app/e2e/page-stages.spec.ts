import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * The page list's stage tabs (#4052): In progress, Discussion, Done, Archived and All, each with a
 * count; a row's progress and who works on it; and `stage:` as one state with the tab.
 */
type Size = 'stages' | 'quiet' | 'cold';
async function mount(page: Page, size: Size = 'stages', hash = '/') {
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
const app = (page: Page) => page.locator('#list-fixture');
const rows = (page: Page) => app(page).locator('.pages > li');
const titles = (page: Page) => rows(page).locator('.tmt-ui-list-title > a');
const search = (page: Page) => app(page).getByRole('combobox', { name: 'Search pages' });
const tab = (page: Page, name: string | RegExp) => app(page).getByRole('tab', { name });
const hash = (page: Page) => new URL(page.url()).hash;

test('the tabs are In progress, Discussion, Done, Archived and All, counting what each holds', async ({
  page,
}) => {
  await mount(page);
  await expect(app(page).getByRole('tab')).toHaveText([
    'In progress 2',
    'Discussion 1',
    'Done 1',
    'Archived 1',
    'All',
  ]);
  // An archived page counts only under Archived, and a page that has not reported under no stage.
  await expect(tab(page, 'All')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText([
    'Alpha plan',
    'Beta review',
    'Gamma notes',
    'Delta ops',
    'Epsilon draft',
  ]);
  const shown = async (name: string, expected: string[]) => {
    await tab(page, name).click();
    await expect(tab(page, name)).toHaveAttribute('aria-selected', 'true');
    await expect(titles(page)).toHaveText(expected);
  };
  await shown('In progress 2', ['Alpha plan', 'Beta review']);
  expect(hash(page)).toContain('stage=in-progress');
  await shown('Discussion 1', ['Gamma notes']);
  await shown('Done 1', ['Delta ops']);
  await shown('Archived 1', ['Archived spec']);
  expect(hash(page)).toContain('archived=true');
  expect(hash(page)).not.toContain('stage=');
  await shown('All', ['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops', 'Epsilon draft']);
  expect(hash(page)).not.toContain('stage=');
  expect(hash(page)).not.toContain('archived');
});

test('a copied link lands on its stage, and one it cannot read lands on the default', async ({
  page,
}) => {
  await mount(page, 'stages', '/?stage=done');
  await expect(tab(page, 'Done 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Delta ops']);
  await mount(page, 'stages', '/?stage=blocked');
  await expect(tab(page, 'All')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveCount(5);
  // Archived wins over a stage in the same link.
  await mount(page, 'stages', '/?stage=done&archived=true');
  await expect(tab(page, 'Archived 1')).toHaveAttribute('aria-selected', 'true');
});

test('a row shows its progress in words and who works on it', async ({ page }) => {
  await mount(page);
  const alpha = rows(page).filter({ hasText: 'Alpha plan' });
  await expect(alpha.getByRole('progressbar', { name: '7 of 10 tasks done' })).toHaveAttribute(
    'value',
    '7',
  );
  await expect(alpha).toContainText('7/10');
  const working = alpha.getByRole('list', { name: 'Working' });
  await expect(working.getByRole('listitem')).toHaveText(['feat-1', 'Ada']);
  // Only three are named; the rest are counted.
  const beta = rows(page).filter({ hasText: 'Beta review' });
  await expect(beta.getByRole('list', { name: 'Working' }).getByRole('listitem')).toHaveText([
    'feat-1',
    'feat-2',
    'remote-1',
    '+2',
  ]);
  // A page with no task has no bar, and a finished one says so in the same words.
  await expect(rows(page).filter({ hasText: 'Gamma notes' }).getByRole('progressbar')).toHaveCount(
    0,
  );
  await expect(
    rows(page)
      .filter({ hasText: 'Delta ops' })
      .getByRole('progressbar', { name: '4 of 4 tasks done' }),
  ).toBeVisible();
});

test('a typed stage: selects the tab and leaves the search, as a tab click does', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('stage:done');
  await page.keyboard.press('Enter');
  await expect(tab(page, 'Done 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Delta ops']);
  expect(hash(page)).toContain('stage=done');
  // One state: nothing of the stage stays in the search, and no separate chip.
  await expect(search(page)).not.toContainText('stage');
  expect(hash(page)).not.toContain('q=');
  // Free text beside it is still a filter on that tab.
  await search(page).click();
  await page.keyboard.type('stage:in-progress alpha');
  await page.keyboard.press('Enter');
  await expect(tab(page, 'In progress 2')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Alpha plan']);
  expect(hash(page)).toContain('q=alpha');
});

test('stage: suggests the stage names with their counts; picking one selects the tab', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('stage:');
  await expect(page.getByRole('option')).toHaveText([
    'in-progress (2)',
    'discussion (1)',
    'done (1)',
    'all (5)',
  ]);
  await page.keyboard.type('dis');
  await expect(page.getByRole('option')).toHaveText(['discussion (1)']);
  await page.keyboard.press('Enter');
  await expect(tab(page, 'Discussion 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Gamma notes']);
  await expect(search(page)).not.toContainText('stage');
});

test('a tab click removes a stage: still typed in the draft, so there is one stage', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('stage:done');
  await tab(page, 'Discussion 1').click();
  await expect(tab(page, 'Discussion 1')).toHaveAttribute('aria-selected', 'true');
  await expect(search(page)).not.toContainText('stage');
  await expect(titles(page)).toHaveText(['Gamma notes']);
});

test('the project filters and the scope count what the stage tab holds', async ({ page }) => {
  await mount(page);
  await tab(page, 'In progress 2').click();
  await expect(app(page).getByRole('button', { name: 'design, 2 pages' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: /^ops/ })).toHaveCount(0);
  await tab(page, 'Done 1').click();
  await expect(app(page).getByRole('button', { name: 'ops, 1 page' })).toBeVisible();
  await expect(app(page).getByRole('button', { name: /^design/ })).toHaveCount(0);
});

test('an empty In progress says where pages without tasks are and links the way out', async ({
  page,
}) => {
  await mount(page, 'quiet', '/?stage=in-progress');
  await expect(tab(page, 'In progress 0')).toHaveAttribute('aria-selected', 'true');
  const empty = app(page).locator('.page-list-empty');
  await expect(empty).toHaveText(
    'Nothing in progress. Pages without tasks are under Discussion; All lists every page.',
  );
  // No tab opens by itself; a link does.
  await expect(titles(page)).toHaveCount(0);
  await empty.getByRole('button', { name: 'Discussion' }).click();
  await expect(tab(page, 'Discussion 2')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Gamma notes', 'Zeta notes']);
  await tab(page, 'In progress 0').click();
  await empty.getByRole('button', { name: 'All' }).click();
  await expect(tab(page, 'All')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveCount(2);
  await tab(page, 'Done 0').click();
  await expect(empty).toHaveText('Nothing done yet.');
});

test('a stage tab in a project with nothing at that stage names the project', async ({ page }) => {
  await mount(page, 'stages', '/?stage=done&scope=design');
  await expect(app(page).locator('.page-list-empty')).toHaveText('Nothing done in design.');
});

test('before any page has reported its tasks a stage tab says it is reading, not that it is empty', async ({
  page,
}) => {
  await mount(page, 'cold', '/?stage=in-progress');
  await expect(app(page).locator('.page-list-empty')).toHaveText('Reading page details…');
  await expect(tab(page, 'In progress 0')).toBeVisible();
  // All still lists the pages, so the list is never blank.
  await tab(page, 'All').click();
  await expect(titles(page)).toHaveText(['Gamma notes', 'Zeta notes']);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the stage tabs keep one scroll axis and capture their states: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page, 'stages', '/?stage=in-progress');
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const flat = () =>
        page.evaluate(
          () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
        );
      await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`4052-in-progress-${width}-${theme}.png`) });
      await tab(page, 'All').click();
      await expect(titles(page)).toHaveCount(5);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`4052-all-${width}-${theme}.png`) });
      await search(page).click();
      await page.keyboard.type('stage:');
      await expect(page.getByRole('option')).toHaveCount(4);
      await page.screenshot({ path: await capturePath(`4052-stage-token-${width}-${theme}.png`) });
      await page.keyboard.press('Escape');
      await mount(page, 'quiet', '/?stage=in-progress');
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await expect(app(page).locator('.page-list-empty')).toContainText('Nothing in progress.');
      expect(await flat()).toBe(true);
      await page.screenshot({
        path: await capturePath(`4052-in-progress-empty-${width}-${theme}.png`),
      });
    });
