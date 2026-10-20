import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';

/**
 * The page's own projects (#4116): the current ones are small chips after the title, and the picker is
 * the list row's menu, opened from the chips, its trigger or Projects… in the More menu.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const requests = (page: Page) =>
  page.evaluate(async (path) => (await import(path)).proof().projects, fixture) as Promise<
    { pageId: string; projects: string[] }[]
  >;
const names = ['alpha', 'beta', 'gamma'];

async function open(
  page: Page,
  width = 1440,
  theme: 'light' | 'dark' = 'light',
  current = ['alpha', 'beta'],
  all = names,
) {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.goto('/');
  await run(page, 'mount', { projects: { names: all, current } });
  await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
  return page.locator('.tmt-ui-header:visible');
}
const trigger = (header: ReturnType<Page['locator']>) =>
  header.getByRole('button', { name: 'Projects for Live shared page' });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the current projects show as chips after the title, and the trigger opens the picker at ${width} ${theme}`, async ({
      page,
    }) => {
      const header = await open(page, width, theme);
      await expect(trigger(header)).toBeVisible();
      const chips = header.locator('.page-header-project-chip');
      // A phone keeps the trigger alone; the picker names every project.
      if (width < 640) await expect(chips.first()).toBeHidden();
      else await expect(chips).toHaveText(['alpha', 'beta']);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`projects-header-${width}-${theme}.png`) });
      await trigger(header).click();
      const rows = page.getByRole('menuitemcheckbox');
      // A checked row ends in its check mark.
      await expect(rows).toHaveText(['alpha✓', 'beta✓', 'gamma']);
      await expect(rows.nth(0)).toHaveAttribute('aria-checked', 'true');
      await expect(rows.nth(1)).toHaveAttribute('aria-checked', 'true');
      await expect(rows.nth(2)).toHaveAttribute('aria-checked', 'false');
      await expect(
        page.getByRole('menuitem', { name: text.pageList.newProjectItem }),
      ).toBeVisible();
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`projects-picker-${width}-${theme}.png`) });
    });

test('choosing a project writes the page set and updates the chips; unchecking removes it', async ({
  page,
}) => {
  const header = await open(page);
  await trigger(header).click();
  await page.getByRole('menuitemcheckbox', { name: 'gamma' }).click();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['alpha', 'beta', 'gamma']);
  expect((await requests(page)).map((request) => request.projects)).toEqual([
    ['alpha', 'beta', 'gamma'],
  ]);
  await expect(header.getByRole('status')).toHaveText(text.pageList.saved);
  await trigger(header).click();
  await page.getByRole('menuitemcheckbox', { name: 'alpha' }).click();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['beta', 'gamma']);
  expect((await requests(page)).at(-1)!.projects).toEqual(['beta', 'gamma']);
});

test('a chip and Projects… in More open the same picker', async ({ page }) => {
  const header = await open(page);
  await header.locator('.page-header-project-chip').first().click();
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(3);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(0);
  await (await pageAction(page.locator('#ask-page-fixture'), 'Projects…')).click();
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(3);
});

test('a page in eight projects cannot take a ninth or a new one', async ({ page }) => {
  const eight = ['p1', 'p2', 'p3', 'p4', 'p5', 'p6', 'p7', 'p8'];
  const header = await open(page, 1440, 'light', eight, [...eight, 'p9']);
  // Three chips and a count of the rest.
  await expect(header.locator('.page-header-project-chip')).toHaveText(['p1', 'p2', 'p3', '+5']);
  await trigger(header).click();
  await expect(page.getByRole('menuitemcheckbox', { name: 'p9' })).toBeDisabled();
  await expect(page.getByRole('menuitem', { name: text.pageList.newProjectItem })).toBeDisabled();
});

test('New project… names a project and puts this page in it', async ({ page }) => {
  const header = await open(page);
  await trigger(header).click();
  await page.getByRole('menuitem', { name: text.pageList.newProjectItem }).click();
  const dialog = page.getByRole('dialog', { name: text.pageList.newProject });
  await dialog.getByLabel(text.pageList.nameField).fill('delta');
  await expect(dialog.getByRole('checkbox')).toBeChecked();
  await dialog.getByRole('button', { name: text.pageList.create, exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['alpha', 'beta', 'delta']);
  expect((await requests(page)).at(-1)!.projects).toEqual(['alpha', 'beta', 'delta']);
});

test('a viewer who cannot write projects sees neither chips, trigger nor Projects…', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', {});
  await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
  const header = page.locator('.tmt-ui-header:visible');
  await expect(header.locator('.page-header-project-chip')).toHaveCount(0);
  await expect(trigger(header)).toHaveCount(0);
  await page.getByRole('button', { name: 'More', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'Projects…' })).toHaveCount(0);
});
