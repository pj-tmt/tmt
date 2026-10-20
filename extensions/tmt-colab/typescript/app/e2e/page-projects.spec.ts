import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';

/**
 * The page's own projects (#4116): the current ones are one button of small chips after the title,
 * which opens the list row's picker. With no project, and on a phone, More → Projects… opens it.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const requests = (page: Page) =>
  page.evaluate(async (path) => (await import(path)).projectProof().requests, fixture) as Promise<
    { pageId: string; projects: string[] }[]
  >;
const reads = (page: Page) =>
  page.evaluate(
    async (path) => (await import(path)).projectProof().spaceHomeReads,
    fixture,
  ) as Promise<number>;
const names = ['alpha', 'beta', 'gamma'];

async function open(
  page: Page,
  width = 1440,
  theme: 'light' | 'dark' = 'light',
  options: { current?: string[]; all?: string[]; fail?: boolean; deepLink?: boolean } = {},
) {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.goto('/');
  await run(page, 'mount', {
    projects: {
      names: options.all ?? names,
      current: options.current ?? ['alpha', 'beta'],
      fail: options.fail,
      deepLink: options.deepLink,
    },
  });
  await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
  return page.locator('.tmt-ui-header:visible');
}
const chips = (header: ReturnType<Page['locator']>) =>
  header.getByRole('button', { name: /^Projects for Live shared page: / });
/** The chips where they show; More → Projects… with no project and on a phone. */
async function openPicker(page: Page, header: ReturnType<Page['locator']>) {
  if (await chips(header).isVisible()) await chips(header).click();
  else await (await pageAction(page.locator('#ask-page-fixture'), 'Projects…')).click();
  await expect(page.getByRole('menuitemcheckbox').first()).toBeVisible();
}
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR) await page.screenshot({ path: capturePath(name) });
};
const noOverflow = (page: Page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`two projects show as chips that open the picker, with no trigger of their own at ${width} ${theme}`, async ({
      page,
    }) => {
      const header = await open(page, width, theme);
      if (width < 640) await expect(chips(header)).toBeHidden();
      else {
        await expect(chips(header).locator('.page-header-project-chip')).toHaveText([
          'alpha',
          'beta',
        ]);
        await expect(chips(header)).toHaveAttribute('aria-haspopup', 'menu');
      }
      // No standalone trigger is shown, so the title keeps its room.
      await expect(header.locator('.page-header-projects-anchor button')).toBeHidden();
      expect(await noOverflow(page)).toBe(true);
      await capture(page, `projects-two-chips-${width}-${theme}.png`);
      await openPicker(page, header);
      const rows = page.getByRole('menuitemcheckbox');
      // A checked row ends in its check mark.
      await expect(rows).toHaveText(['alpha✓', 'beta✓', 'gamma']);
      await expect(rows.nth(0)).toHaveAttribute('aria-checked', 'true');
      await expect(rows.nth(2)).toHaveAttribute('aria-checked', 'false');
      await expect(
        page.getByRole('menuitem', { name: text.pageList.newProjectItem }),
      ).toBeVisible();
      await capture(page, `projects-picker-${width}-${theme}.png`);
    });

    test(`a page in no project shows nothing in the header and still assigns through More at ${width} ${theme}`, async ({
      page,
    }) => {
      const header = await open(page, width, theme, { current: [] });
      await expect(chips(header)).toHaveCount(0);
      await expect(header.locator('.page-header-project-chip')).toHaveCount(0);
      expect(await noOverflow(page)).toBe(true);
      await capture(page, `projects-none-${width}-${theme}.png`);
      await openPicker(page, header);
      await page.getByRole('menuitemcheckbox', { name: 'gamma' }).click();
      await expect(header.locator('.page-header-project-chip')).toHaveText(['gamma']);
      expect((await requests(page)).map((request) => request.projects)).toEqual([['gamma']]);
    });

    test(`a failed write shows a mark with the failure words only while it holds at ${width} ${theme}`, async ({
      page,
    }) => {
      const header = await open(page, width, theme, { fail: true });
      await openPicker(page, header);
      await page.getByRole('menuitemcheckbox', { name: 'gamma' }).click();
      const mark = header.locator('.page-attention[data-attention="projects"]');
      await expect(mark).toContainText(text.pageList.failures.UNKNOWN);
      await expect(mark).toHaveAttribute('role', 'alert');
      // The chips are what the page holds: nothing was written.
      expect(await requests(page)).toEqual([]);
      expect(await noOverflow(page)).toBe(true);
      await capture(page, `projects-failure-${width}-${theme}.png`);
    });
  }

test('choosing a project writes the page set and updates the chips; unchecking removes it', async ({
  page,
}) => {
  const header = await open(page);
  await openPicker(page, header);
  await page.getByRole('menuitemcheckbox', { name: 'gamma' }).click();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['alpha', 'beta', 'gamma']);
  expect((await requests(page)).map((request) => request.projects)).toEqual([
    ['alpha', 'beta', 'gamma'],
  ]);
  await expect(header.locator('.page-header-projects-status')).toHaveText(text.pageList.saved);
  await openPicker(page, header);
  await page.getByRole('menuitemcheckbox', { name: 'alpha' }).click();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['beta', 'gamma']);
  expect((await requests(page)).at(-1)!.projects).toEqual(['beta', 'gamma']);
});

test('the chips and Projects… in More open the same picker', async ({ page }) => {
  const header = await open(page);
  await chips(header).click();
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(3);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(0);
  await (await pageAction(page.locator('#ask-page-fixture'), 'Projects…')).click();
  await expect(page.getByRole('menuitemcheckbox')).toHaveCount(3);
});

for (const width of [1440, 390])
  test(`the picker opened from More → Projects… sits right-aligned under More at ${width}`, async ({
    page,
  }) => {
    const header = await open(page, width, 'light');
    await (await pageAction(page.locator('#ask-page-fixture'), 'Projects…')).click();
    const picker = page.locator('dialog[open]:has([role="menuitemcheckbox"])');
    await expect(picker).toBeVisible();
    const more = (await header.getByRole('button', { name: 'More', exact: true }).boundingBox())!;
    const box = (await picker.boundingBox())!;
    // Under the More button, flush with its right edge, never floating under the title.
    expect(box.y).toBeGreaterThanOrEqual(more.y + more.height - 2);
    expect(box.x + box.width).toBeLessThanOrEqual(more.x + more.width + 2);
    expect(box.x + box.width).toBeGreaterThanOrEqual(more.x + more.width - 12);
    await capture(page, `projects-picker-from-more-${width}-light.png`);
  });

test('a page in eight projects cannot take a ninth or a new one', async ({ page }) => {
  const eight = ['p1', 'p2', 'p3', 'p4', 'p5', 'p6', 'p7', 'p8'];
  const header = await open(page, 1440, 'light', { current: eight, all: [...eight, 'p9'] });
  // Three chips and a count of the rest.
  await expect(header.locator('.page-header-project-chip')).toHaveText(['p1', 'p2', 'p3', '+5']);
  await openPicker(page, header);
  await expect(page.getByRole('menuitemcheckbox', { name: 'p9' })).toBeDisabled();
  await expect(page.getByRole('menuitem', { name: text.pageList.newProjectItem })).toBeDisabled();
});

test('New project… names a project and puts this page in it', async ({ page }) => {
  const header = await open(page);
  await openPicker(page, header);
  await page.getByRole('menuitem', { name: text.pageList.newProjectItem }).click();
  const dialog = page.getByRole('dialog', { name: text.pageList.newProject });
  await dialog.getByLabel(text.pageList.nameField).fill('delta');
  await expect(dialog.getByRole('checkbox')).toBeChecked();
  await dialog.getByRole('button', { name: text.pageList.create, exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(header.locator('.page-header-project-chip')).toHaveText(['alpha', 'beta', 'delta']);
  expect((await requests(page)).at(-1)!.projects).toEqual(['alpha', 'beta', 'delta']);
});

test('opening a page reads no list; a page opened by a link shows no chips until the picker is used', async ({
  page,
}) => {
  const header = await open(page, 1440, 'light', { deepLink: true });
  await expect(chips(header)).toHaveCount(0);
  expect(await reads(page)).toBe(0);
  await openPicker(page, header);
  // The picker opened once the list was read, with this page's projects checked.
  const rows = page.getByRole('menuitemcheckbox');
  await expect(rows).toHaveText(['alpha✓', 'beta✓', 'gamma']);
  expect(await reads(page)).toBe(1);
  await page.keyboard.press('Escape');
  await expect(chips(header).locator('.page-header-project-chip')).toHaveText(['alpha', 'beta']);
});

test('arriving from the list shows the chips with no read, and the picker refreshes behind what it shows', async ({
  page,
}) => {
  const header = await open(page);
  await expect(chips(header).locator('.page-header-project-chip')).toHaveText(['alpha', 'beta']);
  expect(await reads(page)).toBe(0);
  await openPicker(page, header);
  await expect.poll(() => reads(page)).toBe(1);
});

test('a viewer who cannot write projects sees neither chips nor Projects…', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount', {});
  await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toBeVisible();
  const header = page.locator('.tmt-ui-header:visible');
  await expect(chips(header)).toHaveCount(0);
  await expect(header.locator('.page-header-project-chip')).toHaveCount(0);
  await page.getByRole('button', { name: 'More', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'Projects…' })).toHaveCount(0);
});
