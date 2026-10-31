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
const rows = (page: Page) => app(page).locator('.pages [data-page-id]');
const titles = (page: Page) => rows(page).locator('.tmt-ui-list-title > a');
const search = (page: Page) => app(page).getByRole('combobox', { name: 'Search pages' });
const tab = (page: Page, name: string | RegExp) => app(page).getByRole('tab', { name });
const hash = (page: Page) => new URL(page.url()).hash;

test('the tabs are All, Needs you, Discussion, In progress, Done and Archived, counting what each holds', async ({
  page,
}) => {
  await mount(page);
  for (const [index, name] of [
    'All, 5',
    'Needs you, 0',
    'Discussion, 1',
    'In progress, 2',
    'Done, 1',
    'Archived, 1',
  ].entries())
    await expect(app(page).getByRole('tab').nth(index)).toHaveAccessibleName(name);
  // The bare link opens on All, and the address stays plain: that is the default.
  await expect(tab(page, 'All, 5')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText([
    'Alpha plan',
    'Beta review',
    'Gamma notes',
    'Delta ops',
    'Epsilon draft',
  ]);
  expect(hash(page)).not.toContain('stage=');
  const shown = async (name: string, expected: string[]) => {
    await tab(page, name).click();
    await expect(tab(page, name)).toHaveAttribute('aria-selected', 'true');
    await expect(titles(page)).toHaveText(expected);
  };
  await shown('In progress, 2', ['Alpha plan', 'Beta review']);
  expect(hash(page)).toContain('stage=in-progress');
  await shown('Discussion, 1', ['Gamma notes']);
  expect(hash(page)).toContain('stage=discussion');
  await shown('Done, 1', ['Delta ops']);
  await shown('Archived, 1', ['Archived spec']);
  expect(hash(page)).toContain('archived=true');
  expect(hash(page)).not.toContain('stage=');
  // An archived page counts only under Archived, and a page that has not reported under no stage.
  await shown('All, 5', ['Alpha plan', 'Beta review', 'Gamma notes', 'Delta ops', 'Epsilon draft']);
  expect(hash(page)).not.toContain('stage=');
  expect(hash(page)).not.toContain('archived');
});

test('a copied link lands on its stage, and one it cannot read lands on the default', async ({
  page,
}) => {
  await mount(page, 'stages', '/?stage=done');
  await expect(tab(page, 'Done, 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Delta ops']);
  // A link to In progress keeps working, and so does an older one that names the default, All.
  await mount(page, 'stages', '/?stage=in-progress');
  await expect(tab(page, 'In progress, 2')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
  await mount(page, 'stages', '/?stage=all');
  await expect(tab(page, 'All, 5')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveCount(5);
  await mount(page, 'stages', '/?stage=blocked');
  await expect(tab(page, 'All, 5')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveCount(5);
  // Archived wins over a stage in the same link.
  await mount(page, 'stages', '/?stage=done&archived=true');
  await expect(tab(page, 'Archived, 1')).toHaveAttribute('aria-selected', 'true');
});

test('a row shows its sub-page progress in words, and no workers', async ({ page }) => {
  await mount(page, 'stages', '/?stage=all');
  const alpha = rows(page).filter({ hasText: 'Alpha plan' });
  await expect(alpha).toContainText('7 of 10 sub-pages done');
  await expect(alpha.getByRole('progressbar')).toHaveCount(0);
  await expect(page.getByRole('list', { name: 'Working' })).toHaveCount(0);
  // A page without sub-pages has no progress count, and a finished one uses the same words.
  await expect(rows(page).filter({ hasText: 'Gamma notes' }).getByRole('progressbar')).toHaveCount(
    0,
  );
  await expect(
    rows(page).filter({ hasText: 'Delta ops' }).getByText('4 of 4 sub-pages done', { exact: true }),
  ).toBeVisible();
});

test('a typed stage: selects the tab and leaves the search, as a tab click does', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('stage:done');
  await page.keyboard.press('Enter');
  await expect(tab(page, 'Done, 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Delta ops']);
  expect(hash(page)).toContain('stage=done');
  // One state: nothing of the stage stays in the search, and no separate chip.
  await expect(search(page)).not.toContainText('stage');
  expect(hash(page)).not.toContain('q=');
  // Free text beside it is still a filter on that tab.
  await search(page).click();
  await page.keyboard.type('stage:in-progress alpha');
  await page.keyboard.press('Enter');
  await expect(tab(page, 'In progress, 2')).toHaveAttribute('aria-selected', 'true');
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
    'all (5)',
    'discussion (1)',
    'in-progress (2)',
    'done (1)',
  ]);
  await page.keyboard.type('dis');
  await expect(page.getByRole('option')).toHaveText(['discussion (1)']);
  await page.keyboard.press('Enter');
  await expect(tab(page, 'Discussion, 1')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Gamma notes']);
  await expect(search(page)).not.toContainText('stage');
});

test('a tab click removes a stage: still typed in the draft, so there is one stage', async ({
  page,
}) => {
  await mount(page);
  await search(page).click();
  await page.keyboard.type('stage:done');
  // The suggestions open over the tab strip, which stays put under them: a click outside them (the
  // list's count line) closes them and keeps the draft.
  await app(page).locator('[data-page-list-count]').click();
  await expect(app(page).getByRole('listbox')).toBeHidden();
  await tab(page, 'Discussion, 1').click();
  await expect(tab(page, 'Discussion, 1')).toHaveAttribute('aria-selected', 'true');
  await expect(search(page)).not.toContainText('stage');
  await expect(titles(page)).toHaveText(['Gamma notes']);
});

test('the project suggestions count what the stage tab holds', async ({ page }) => {
  await mount(page);
  const suggested = async (expected: string[]) => {
    await search(page).click();
    await page.keyboard.type('project:');
    await expect(page.getByRole('option')).toHaveText(expected);
    // Escape closes the suggestions, then clears the draft.
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
  };
  await tab(page, 'In progress, 2').click();
  await suggested(['design (2)']);
  await tab(page, 'Done, 1').click();
  await suggested(['ops (1)']);
});

test('an empty In progress says where pages without tasks are and links the way out', async ({
  page,
}) => {
  await mount(page, 'quiet', '/?stage=in-progress');
  await expect(tab(page, 'In progress, 0')).toHaveAttribute('aria-selected', 'true');
  const empty = app(page).locator('.page-list-empty');
  await expect(empty).toHaveText(
    'Nothing in progress. Pages without sub-pages are under Discussion; All lists every page.',
  );
  // No tab opens by itself; a link does.
  await expect(titles(page)).toHaveCount(0);
  await empty.getByRole('button', { name: 'Discussion' }).click();
  await expect(tab(page, 'Discussion, 2')).toHaveAttribute('aria-selected', 'true');
  await expect(titles(page)).toHaveText(['Gamma notes', 'Zeta notes']);
  await tab(page, 'In progress, 0').click();
  await empty.getByRole('button', { name: 'All' }).click();
  await expect(tab(page, /^All, \d+\+?$/)).toHaveAttribute('aria-selected', 'true');
  expect(hash(page)).not.toContain('stage=');
  await expect(titles(page)).toHaveCount(2);
  await tab(page, 'Done, 0').click();
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
  await expect(tab(page, 'In progress, 0')).toBeVisible();
  // All still lists the pages, so the list is never blank.
  await tab(page, /^All, \d+\+?$/).click();
  await expect(titles(page)).toHaveText(['Gamma notes', 'Zeta notes']);
});

test('a cold start reads again and fills In progress as the pages report, with no click', async ({
  page,
}) => {
  await mount(page, 'cold', '/?stage=in-progress');
  await expect(app(page).locator('.page-list-empty')).toHaveText('Reading page details…');
  await page.evaluate(() => window.__reportTasks!());
  // The list reads again every 2 s: the page that reports work appears, the other is a discussion.
  await expect(titles(page)).toHaveText(['Gamma notes'], { timeout: 10_000 });
  for (const [index, name] of [
    'All, 2',
    'Needs you, 0',
    'Discussion, 1',
    'In progress, 1',
    'Done, 0',
    'Archived, 0',
  ].entries())
    await expect(app(page).getByRole('tab').nth(index)).toHaveAccessibleName(name);
  await expect(app(page).locator('.page-list-empty')).toHaveCount(0);
});

// Phone tabs scroll horizontally while every stage remains keyboard reachable.
for (const width of [1440, 390, 320])
  test(`every stage tab is keyboard reachable in the strip: ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    const strip = app(page).getByRole('tablist');
    const names = [/^All/, /^Needs you/, /^Discussion/, /^In progress/, /^Done/, /^Archived/];
    if (width <= 390)
      expect(await strip.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
    expect(await strip.evaluate((node) => getComputedStyle(node).maskImage)).toBe('none');
    const tops = await app(page)
      .getByRole('tab')
      .evaluateAll((nodes) => nodes.map((node) => node.getBoundingClientRect().top));
    expect(Math.max(...tops) - Math.min(...tops)).toBeLessThanOrEqual(1);
    await tab(page, names[0]).focus();
    for (const [index, name] of names.entries()) {
      if (index > 0) await page.keyboard.press('ArrowRight');
      await expect(tab(page, name)).toHaveAttribute('aria-selected', 'true');
      await expect(tab(page, name)).toBeFocused();
      await expect(tab(page, name)).toBeInViewport();
      await expect
        .poll(() =>
          tab(page, name).evaluate((node) => {
            const bounds = node.closest('[role="tablist"]')!.getBoundingClientRect();
            const box = node.getBoundingClientRect();
            return box.left >= bounds.left - 1 && box.right <= bounds.right + 1;
          }),
        )
        .toBe(true);
    }
    await page.keyboard.press('Home');
    await expect(tab(page, names[0])).toHaveAttribute('aria-selected', 'true');
    await expect(tab(page, names[0])).toBeFocused();
    expect(await strip.evaluate((node) => node.scrollLeft)).toBe(0);
  });

// A linked stage is brought into view in the horizontal strip on load.
for (const width of [390, 320])
  for (const link of [
    '/?stage=discussion',
    '/?stage=in-progress',
    '/?stage=done',
    '/?archived=true',
  ])
    test(`a link to ${link} opens with its tab inside the strip: ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await mount(page, 'stages', link);
      const selected = app(page).getByRole('tab', { selected: true });
      await expect(selected).toHaveCount(1);
      await expect
        .poll(() =>
          selected.evaluate((node) => {
            const bounds = node.closest('[role="tablist"]')!.getBoundingClientRect();
            const box = node.getBoundingClientRect();
            return box.left >= bounds.left - 1 && box.right <= bounds.right + 1;
          }),
        )
        .toBe(true);
    });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the stage tabs keep one scroll axis and capture their states: ${width}px ${theme}`, async ({
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
      await expect(titles(page)).toHaveCount(5);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`4052-all-${width}-${theme}.png`) });
      await tab(page, /^In progress/).click();
      await expect(titles(page)).toHaveText(['Alpha plan', 'Beta review']);
      expect(await flat()).toBe(true);
      await page.screenshot({ path: await capturePath(`4052-in-progress-${width}-${theme}.png`) });
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
