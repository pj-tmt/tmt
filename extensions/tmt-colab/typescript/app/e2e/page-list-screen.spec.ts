import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/** Home is the one vertical viewport for the website hero, controls and lazy-drawn rows. */
async function mount(page: Page, width: number, theme: 'light' | 'dark') {
  await page.setViewportSize({ width, height: width < 480 ? 844 : 900 });
  await page.emulateMedia({ colorScheme: theme });
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(async (theme) => {
    document.documentElement.dataset.theme = theme;
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('work');
  }, theme);
}
const app = (page: Page) => page.locator('#list-fixture');
const rows = (page: Page) => app(page).locator('.pages [data-page-id]');

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`one page viewport with a scrolling hero and lazy rows at ${width} ${theme}`, async ({
      page,
    }) => {
      await mount(page, width, theme);
      await expect(app(page).locator('.tmt-ui-topbar-title-current')).toHaveText('All pages');
      await expect(app(page).getByRole('tab')).toHaveCount(6);
      for (const name of [
        'All, 75',
        '◆ Needs you, 1 · waiting',
        'Discussion, 25',
        'In progress, 25',
        'Done, 25',
        'Archived, 1',
      ])
        await expect(app(page).getByRole('tab', { name, exact: true })).toBeVisible();
      await expect(app(page).getByRole('tab', { name: 'All, 75' })).toHaveAttribute(
        'aria-selected',
        'true',
      );
      await expect(app(page).locator('.tmt-ui-tabs')).toHaveAttribute('data-variant', 'flat');
      await expect(app(page).locator('.tmt-ui-tabs')).not.toHaveAttribute('data-wrap', '');
      await expect(app(page).getByRole('button', { name: 'New page' })).toBeVisible();
      await expect(rows(page)).toHaveCount(50);
      const count = app(page).locator('[data-page-list-count]');
      await expect(count.locator('span').first()).toHaveText('Showing 50 of 75');

      const top = async () => {
        const box = (selector: string) =>
          app(page)
            .locator(selector)
            .first()
            .boundingBox()
            .then((value) => value!);
        const [search, tabs, list] = await Promise.all([
          box('.page-list-search'),
          box('[role="tablist"]'),
          box('.page-list-scroll'),
        ]);
        return { search, tabs, list };
      };
      const before = await top();
      expect(before.search.y - before.tabs.y - before.tabs.height).toBe(16);
      expect(before.list.y - before.search.y - before.search.height).toBe(16);
      expect(before.search.height).toBe(36);

      if (width < 480) {
        const tops = await app(page)
          .getByRole('tab')
          .evaluateAll((tabs) => tabs.map((tab) => tab.getBoundingClientRect().top));
        expect(Math.max(...tops) - Math.min(...tops)).toBeLessThanOrEqual(1);
        expect(
          await app(page)
            .getByRole('tablist')
            .evaluate((node) => node.scrollWidth > node.clientWidth),
        ).toBe(true);
      }
      await page.screenshot({ path: await capturePath(`4273-list-${width}-${theme}.png`) });

      const list = app(page).locator('[data-page-list]');
      await list.evaluate((node) => (node.scrollTop = node.scrollHeight));
      await expect(rows(page)).toHaveCount(75);
      await expect(count.locator('span').first()).toHaveText('75 pages in this view');
      await list.evaluate((node) => (node.scrollTop = node.scrollHeight / 2));
      expect((await top()).search.y).toBeLessThan(before.search.y);
      expect(
        await app(page)
          .locator('.page-list-scroll')
          .evaluate((node) => node.scrollTop),
      ).toBe(0);
      expect(await page.evaluate(() => document.documentElement.scrollHeight > innerHeight)).toBe(
        false,
      );
      await page.screenshot({
        path: await capturePath(`4273-list-${width}-${theme}-scrolled.png`),
      });
    });

test('Needs you is a tab: it lists the pages waiting on you and leaves with another tab', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  await app(page)
    .getByRole('tab', { name: /Needs you, 1/ })
    .click();
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).locator('.tmt-ui-list-title > a')).toHaveText('Review 2');
  expect(new URL(page.url()).hash).toContain('needs=true');
  await app(page).getByRole('tab', { name: 'Discussion, 25' }).click();
  await expect(rows(page)).toHaveCount(25);
  expect(new URL(page.url()).hash).not.toContain('needs');
});

test('New page gives the command that creates a page on the machine', async ({ page }) => {
  await mount(page, 1440, 'light');
  await app(page).getByRole('button', { name: 'New page' }).click();
  const dialog = page.getByRole('dialog', { name: 'New page' });
  await expect(dialog).toContainText('tmt colab page create --title "Notes" --layout column');
  await expect(dialog).toContainText('Return to your TMT workspace after creating the page.');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: 'New page' })).toBeFocused();
  await expect(rows(page)).toHaveCount(50);
  await app(page).getByRole('button', { name: 'New page' }).click();
  await dialog.getByRole('button', { name: 'Close New page' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: 'New page' })).toBeFocused();
});

for (const width of [1586, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`website page dialogs remain contained at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      await page.setViewportSize({ width, height: 992 });
      const opener = app(page).getByRole('button', { name: 'New page', exact: true });
      await opener.click();
      const create = app(page).getByRole('dialog', { name: 'New page', exact: true });
      await expect(
        create.locator('footer').getByRole('button', { name: 'Close', exact: true }),
      ).toBeVisible();
      await expect(
        create
          .getByRole('button', { name: 'Copy', exact: true })
          .locator('svg[aria-hidden="true"]'),
      ).toHaveCount(1);
      expect(await create.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      await page.screenshot({ path: await capturePath(`new-page-${width}-${theme}.png`) });
      await create.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(opener).toBeFocused();
      const menu = rows(page)
        .first()
        .getByRole('button', { name: /^Actions for/ });
      await menu.click();
      await page.getByRole('menuitem', { name: 'Archive…', exact: true }).click();
      const archive = app(page).getByRole('dialog');
      await expect(archive).toContainText(
        'An archived page stays readable, but nobody can edit it.',
      );
      await expect(archive.locator('footer')).toHaveCSS('border-top-width', '1px');
      const cancel = archive.locator('footer').getByRole('button', { name: 'Cancel', exact: true });
      await expect(cancel).toHaveCSS('border-top-width', '1px');
      expect(await archive.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({ path: await capturePath(`archive-${width}-${theme}.png`) });
      await cancel.click();
      await expect(archive).toHaveCount(0);
      await expect(menu).toBeFocused();
      expect(await page.evaluate(() => window.__pageOperations)).toEqual([]);
    });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`website list density and truthful metadata at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      const home = app(page).locator('[data-page-list]');
      await expect(home).toHaveClass(/tmt-ui-page/);
      await expect(home).toHaveCSS('padding-left', width === 1440 ? '36px' : '18px');
      await expect(rows(page).first().locator('progress')).toHaveCount(0);
      await expect(rows(page).first().locator('.page-comments')).toHaveCount(0);
      const sort = app(page).getByRole('button', { name: 'Sort: Recently updated' });
      if (width === 390) {
        await expect(sort).toHaveAccessibleName('Sort: Recently updated');
        await expect(sort.locator('svg.lucide-arrow-down-up')).toHaveCount(1);
      } else await expect(sort).toHaveText('Recently updated');
      const search = app(page).getByRole('combobox', { name: 'Search pages' });
      await expect(search).toHaveCSS('font-size', '14px');
      expect((await search.boundingBox())!.height).toBe(34);
      const ordinary = rows(page)
        .filter({ has: page.locator('[data-page-stage="discussion"]') })
        .first();
      // This fixture's Discussion pages still carry four tasks, so they retain a task line.
      if (width === 1440) expect((await ordinary.boundingBox())!.height).toBe(70);
      expect((await ordinary.locator('.page-stage').boundingBox())!.height).toBe(20);
      expect((await rows(page).locator('.page-waiting').first().boundingBox())!.height).toBe(20);
      expect(await home.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      if (width === 390) {
        const first = rows(page).first();
        // A compact row flows its cells' contents: the details wrap on a line under the title.
        await expect(first).toHaveCSS('display', 'flex');
        const title = (await first.locator('.page-row-title').boundingBox())!;
        const detail = (await first.locator('.page-row-projects').boundingBox())!;
        expect(detail.y).toBeGreaterThanOrEqual(title.y + title.height);
      }

      expect(
        await app(page)
          .locator('.page-list-status')
          .evaluate((node) => getComputedStyle(node).position),
      ).toBe('absolute');
      await expect(rows(page).locator('.page-project-group-chip').first()).toHaveAttribute(
        'data-project-tone',
        /blue|violet|amber|green/,
      );
      await page.evaluate(async () => {
        const path = '/test/page-list-browser.tsx';
        await (await import(path)).mountPageList('quiet');
      });
      if (width === 1440) expect((await rows(page).first().boundingBox())!.height).toBe(64);
    });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`file icons stay inside their clipping boxes at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      await page.evaluate(async () => {
        const path = '/test/page-list-browser.tsx';
        await (await import(path)).mountPageList('quiet');
      });
      await expect(rows(page)).toHaveCount(2);
      await page.screenshot({ path: await capturePath(`4883-icons-${width}-${theme}.png`) });
      const geometry = await rows(page)
        .first()
        .locator('.page-row-icon')
        .evaluate((icon) => {
          const svg = icon.querySelector('svg')!;
          const box = svg.getBoundingClientRect();
          const clips: {
            name: string;
            top: number;
            bottom: number;
            left: number;
            right: number;
          }[] = [];
          for (let node: Element | null = icon; node; node = node.parentElement) {
            const style = getComputedStyle(node);
            if (
              node === icon ||
              /(hidden|clip|scroll|auto)/.test(`${style.overflowX} ${style.overflowY}`)
            ) {
              const bounds = node.getBoundingClientRect();
              clips.push({
                name: node.className.toString(),
                top: bounds.top,
                bottom: bounds.bottom,
                left: bounds.left,
                right: bounds.right,
              });
            }
          }
          const artwork = svg.getBBox();
          const stroke = parseFloat(getComputedStyle(svg).strokeWidth) / 2;
          return {
            box: { top: box.top, bottom: box.bottom, left: box.left, right: box.right },
            clips,
            artwork: {
              left: artwork.x - stroke,
              top: artwork.y - stroke,
              right: artwork.x + artwork.width + stroke,
              bottom: artwork.y + artwork.height + stroke,
            },
            view: { width: svg.viewBox.baseVal.width, height: svg.viewBox.baseVal.height },
          };
        });
      await test.info().attach('icon-geometry', {
        body: JSON.stringify(geometry),
        contentType: 'application/json',
      });
      for (const clip of geometry.clips) {
        expect(geometry.box.top, clip.name).toBeGreaterThanOrEqual(clip.top - 0.5);
        expect(geometry.box.bottom, clip.name).toBeLessThanOrEqual(clip.bottom + 0.5);
        expect(geometry.box.left, clip.name).toBeGreaterThanOrEqual(clip.left - 0.5);
        expect(geometry.box.right, clip.name).toBeLessThanOrEqual(clip.right + 0.5);
      }
      expect(geometry.artwork.left).toBeGreaterThanOrEqual(0);
      expect(geometry.artwork.top).toBeGreaterThanOrEqual(0);
      expect(geometry.artwork.right).toBeLessThanOrEqual(geometry.view.width);
      expect(geometry.artwork.bottom).toBeLessThanOrEqual(geometry.view.height);
      if (width === 1440) {
        const first = rows(page).first();
        const title = (await first.locator('.page-row-title > a').boundingBox())!;
        const tag = (await first.locator('.page-project-group-chip').first().boundingBox())!;
        expect(Math.abs(title.y + title.height / 2 - tag.y - tag.height / 2)).toBeLessThanOrEqual(
          1,
        );
      }
    });

    test(`row titles and project tags align at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      const first = rows(page)
        .filter({ has: page.locator('.page-project-group-chip') })
        .first();
      await page.screenshot({ path: await capturePath(`4883-alignment-${width}-${theme}.png`) });
      const title = (await first.locator('.page-row-title > a').boundingBox())!;
      const tag = (await first.locator('.page-project-group-chip').first().boundingBox())!;
      await test.info().attach('title-tag-geometry', {
        body: JSON.stringify({ title, tag }),
        contentType: 'application/json',
      });
      if (width === 1440)
        expect(Math.abs(title.y + title.height / 2 - tag.y - tag.height / 2)).toBeLessThanOrEqual(
          1,
        );
      else {
        const detail = (await first.locator('.page-row-projects').boundingBox())!;
        const titleCell = (await first.locator('.page-row-title').boundingBox())!;
        expect(detail.y).toBeGreaterThanOrEqual(titleCell.y + titleCell.height);
      }
      expect(
        await app(page)
          .locator('[data-page-list]')
          .evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
      expect(await page.evaluate(() => window.__pageOperations)).toEqual([]);
      await page.evaluate(async () => {
        const path = '/test/page-list-browser.tsx';
        await (await import(path)).mountPageList('long');
      });
      const longRow = rows(page).filter({ hasText: 'Long names' });
      await expect(longRow).toHaveCount(1);
      expect(await longRow.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      if (width === 390) {
        const titleCell = (await longRow.locator('.page-row-title').boundingBox())!;
        const detail = (await longRow.locator('.page-row-projects').boundingBox())!;
        expect(detail.y).toBeGreaterThanOrEqual(titleCell.y + titleCell.height);
      }
    });
  }

for (const width of [1440, 390])
  test(`row Projects menu and tag-area plus share the picker at ${width}`, async ({ page }) => {
    await mount(page, width, 'light');
    const row = rows(page)
      .filter({ has: page.locator('.page-project-group-chip') })
      .first();
    const entry = row.locator('.page-project-group');
    await expect(entry).toBeVisible();
    await row.hover();
    await expect(entry).toBeVisible();
    const access = row.locator('.page-row-access > span');
    await expect(access).toHaveAttribute('title', 'Private');
    await expect(access).toHaveCSS('text-overflow', 'ellipsis');
    if (width === 1440) {
      const label = (await access.boundingBox())!;
      const actions = (await row.locator('.tmt-ui-list-actions').boundingBox())!;
      expect(label.x + label.width).toBeLessThanOrEqual(actions.x - 3);
    }
    const add = row.locator('.page-row-projects').getByRole('button', { name: /^Projects for/ });
    await expect(add).toBeVisible();
    await add.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('menuitemcheckbox').first()).toBeVisible();
    expect(await page.evaluate(() => window.__projectRequests)).toEqual([]);
    await page.keyboard.press('Escape');
    await expect(add).toBeFocused();
    await row.getByRole('button', { name: /^Actions for/ }).click();
    await page.getByRole('menuitem', { name: 'Projects…', exact: true }).click();
    await expect(page.getByRole('menuitemcheckbox').first()).toBeVisible();
    await expect(page.locator('[role="menu"]:visible')).toHaveCount(1);
    await page.keyboard.press('Escape');
    await expect(add).toBeFocused();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    const geometry = await row.locator('.page-row-projects').evaluate((node) => {
      const tag = Array.from(node.querySelectorAll('.page-project-group-chip'))
        .at(-1)!
        .getBoundingClientRect();
      const trigger = node.querySelector('.page-project-group-add')!.getBoundingClientRect();
      return {
        gap: trigger.left - tag.right,
        overlap: trigger.top < tag.bottom && trigger.bottom > tag.top,
      };
    });
    expect(geometry.gap).toBeGreaterThanOrEqual(0);
    expect(geometry.gap).toBeLessThanOrEqual(10);
    expect(geometry.overlap).toBe(true);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`sticky controls retain search and commands at ${width} ${theme}`, async ({ page }) => {
      await mount(page, width, theme);
      const home = app(page).locator('[data-page-list]');
      const controls = home.locator('.page-list-controls');
      const search = home.getByRole('combobox', { name: 'Search pages' });
      await search.fill('unfinished query');
      const editor = await search.elementHandle();
      await home.evaluate((node) => (node.scrollTop = 520));
      await expect(home).toHaveAttribute('data-compact', '');
      await expect(search).toHaveText('unfinished query');
      expect(await search.evaluate((node, previous) => node === previous, editor)).toBe(true);
      const geometry = await controls.evaluate((node) => {
        const root = node.closest('[data-page-list]')!;
        const box = node.getBoundingClientRect();
        const search = node.querySelector('.page-list-search')!.getBoundingClientRect();
        const action = node.querySelector('.page-list-compact-create')!.getBoundingClientRect();
        return {
          top: box.top - root.getBoundingClientRect().top,
          height: box.height,
          searchTop: search.top,
          searchBottom: search.bottom,
          actionTop: action.top,
          actionBottom: action.bottom,
          overflow: root.scrollWidth > root.clientWidth,
        };
      });
      expect(Math.abs(geometry.top)).toBeLessThan(1);
      expect(geometry.height).toBe(53);
      expect(geometry.searchTop).toBeLessThan(geometry.actionBottom);
      expect(geometry.actionTop).toBeLessThan(geometry.searchBottom);
      expect(geometry.overflow).toBe(false);
      await expect(controls.locator('.page-scope-filters')).toHaveCount(0);
      await expect(controls.locator('.page-list-sort')).toBeVisible();
      const heights = await controls.evaluate((node) =>
        [
          '.page-list-search-box',
          '.page-list-filter-menu button',
          '.page-list-sort button',
          '.page-list-compact-create button',
        ].map((selector) => node.querySelector(selector)!.getBoundingClientRect().height),
      );
      expect(heights).toEqual([36, 36, 36, 36]);
      await page.screenshot({ path: await capturePath(`5818-sticky-${width}-${theme}.png`) });
      await search.focus();
      await expect(search).toBeFocused();
      const create = home.getByRole('button', { name: 'New page', exact: true });
      await create.click();
      const dialog = page.getByRole('dialog', { name: 'New page', exact: true });
      await expect(dialog).toBeVisible();
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      await expect(create).toBeFocused();
      await home.evaluate((node) => (node.scrollTop = 0));
      await expect(home).not.toHaveAttribute('data-compact');
      await expect(search).toHaveText('unfinished query');
      expect(await search.evaluate((node, previous) => node === previous, editor)).toBe(true);
      await expect(home.getByRole('button', { name: 'New page', exact: true })).toBeFocused();
      expect(
        await page.evaluate(() => ({
          projects: window.__projectRequests,
          manage: window.__projectManage,
          pins: window.__pinRequests,
          operations: window.__pageOperations,
        })),
      ).toEqual({ projects: [], manage: [], pins: [], operations: [] });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    });

test('compact project condition preserves draft and focus across viewport changes', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  const home = app(page).locator('[data-page-list]');
  const search = home.getByRole('combobox', { name: 'Search pages' });
  await search.fill('a long unfinished query that remains a draft');
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  await home.getByRole('button', { name: 'Filter', exact: true }).click();
  await page.getByRole('menuitemcheckbox', { name: 'blog', exact: true }).click();
  await expect(home).not.toHaveAttribute('data-compact');
  await expect(app(page).locator('.tmt-ui-topbar-title-current')).toHaveText('blog');
  const chip = home.getByRole('button', { name: 'Remove project: blog', exact: true });
  await expect(chip).toBeVisible();
  await expect(search).toHaveText('a long unfinished query that remains a draft');
  expect(new URL(page.url()).hash).toContain('scope=blog');
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  await chip.focus();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(chip).toBeFocused();
  await expect(home.locator('.page-scope-filters')).toHaveCount(0);
  await expect(home.locator('.page-list-controls')).toHaveCSS('animation-name', 'none');
  expect(await home.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(chip).toBeFocused();
  await expect(search).toHaveText('a long unfinished query that remains a draft');
  await search.fill('');
  await search.press('Escape');
  await page.keyboard.press('/');
  await expect(search).toBeFocused();
});

test('compact list keeps row keys, bulk selection, lazy drawing and pin changes with their owners', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  const home = app(page).locator('[data-page-list]');
  await home.evaluate((node) => (node.scrollTop = node.scrollHeight));
  await expect(rows(page)).toHaveCount(75);
  await expect(home).toHaveAttribute('data-compact', '');
  const link = rows(page).last().locator('.page-row-title > a');
  await link.focus();
  await page.keyboard.press('k');
  await expect(rows(page).nth(73).locator('.page-row-title > a')).toBeFocused();
  await page.keyboard.press('x');
  await expect(home.getByRole('region', { name: 'Bulk actions' })).toContainText('1');
  await page.keyboard.press('l');
  await expect(page.getByRole('menuitemcheckbox').first()).toBeVisible();
  await page.keyboard.press('Escape');
  expect(await page.evaluate(() => window.__projectRequests)).toEqual([]);
  await home.evaluate((node) => (node.scrollTop = 0));
  await expect(home).not.toHaveAttribute('data-compact');
  await home.getByRole('button', { name: 'Clear selection', exact: true }).click();
  const first = rows(page).first();
  await first.getByRole('button', { name: /^Pin / }).click();
  await expect(home.getByRole('list', { name: 'Pinned pages' }).getByRole('link')).toHaveCount(1);
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  await home.evaluate((node) => (node.scrollTop = 0));
  await expect(home).not.toHaveAttribute('data-compact');
  expect(await page.evaluate(() => window.__pinRequests)).toHaveLength(1);
});

test('short and empty lists stay full; viewers omit empty project lists and writers keep the picker', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  for (const size of ['none', 'unmanaged', 'few'] as const) {
    await page.evaluate(async (size) => {
      const path = '/test/page-list-browser.tsx';
      await (await import(path)).mountPageList(size);
    }, size);
    const home = app(page).locator('[data-page-list]');
    await expect(home).toBeVisible();
    await home.evaluate((node) => (node.scrollTop = node.scrollHeight));
    await expect(home).not.toHaveAttribute('data-compact');
    if (size !== 'none') {
      const row = rows(page).filter({ hasText: 'Gamma notes' });
      await expect(row.locator('ul[aria-label="Projects"]')).toHaveCount(0);
      await expect(row.locator('.page-project-group')).toHaveCount(size === 'few' ? 1 : 0);
    }
  }
});

test('list replacement disconnects its scroll observers', async ({ page }) => {
  await page.goto('/');
  const observers = await page.evaluateHandle(() => {
    const records: { root: Element | Document | null; disconnected: boolean }[] = [];
    const Original = IntersectionObserver;
    window.IntersectionObserver = class extends Original {
      private record: (typeof records)[number];
      constructor(callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
        super(callback, options);
        this.record = { root: options?.root ?? null, disconnected: false };
        records.push(this.record);
      }
      override disconnect() {
        this.record.disconnected = true;
        super.disconnect();
      }
    };
    const OriginalResize = ResizeObserver;
    window.ResizeObserver = class extends OriginalResize {
      private records: typeof records = [];
      override observe(target: Element, options?: ResizeObserverOptions) {
        const record = { root: target, disconnected: false };
        this.records.push(record);
        records.push(record);
        super.observe(target, options);
      }
      override unobserve(target: Element) {
        for (const record of this.records) if (record.root === target) record.disconnected = true;
        super.unobserve(target);
      }
      override disconnect() {
        for (const record of this.records) record.disconnected = true;
        super.disconnect();
      }
    };
    return records;
  });
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('work');
  });
  await expect(app(page).locator('[data-page-list]')).toBeVisible();
  const count = await observers.evaluate(
    (records) =>
      records.filter(
        (record) => record.root instanceof Element && record.root.matches('[data-page-list]'),
      ).length,
  );
  expect(count).toBeGreaterThanOrEqual(3);
  await page.evaluate(async () => {
    const path = '/test/page-list-browser.tsx';
    await (await import(path)).mountPageList('none');
  });
  await expect
    .poll(() =>
      observers.evaluate((records) =>
        records
          .filter((record) => record.root instanceof Element && !record.root.isConnected)
          .every((record) => record.disconnected),
      ),
    )
    .toBe(true);
});

test('a scroll transition never steals dialog focus and dismissal returns to the visible create action', async ({
  page,
}) => {
  await mount(page, 1440, 'light');
  const home = app(page).locator('[data-page-list]');
  const create = home.getByRole('button', { name: 'New page', exact: true });
  const dialog = page.getByRole('dialog', { name: 'New page', exact: true });
  await create.click();
  await expect(dialog).toBeVisible();
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  expect(await dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(create).toBeFocused();
  await create.click();
  await home.evaluate((node) => (node.scrollTop = 0));
  await expect(home).not.toHaveAttribute('data-compact');
  expect(await dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(create).toBeFocused();
});

for (const width of [1440, 390])
  test(`sticky mode stays stable just across its boundary at ${width}`, async ({ page }) => {
    await mount(page, width, 'light');
    const home = app(page).locator('[data-page-list]');
    const transitions = await home.evaluate(async (node) => {
      const events: boolean[] = [];
      const observer = new MutationObserver(() => events.push(node.hasAttribute('data-compact')));
      observer.observe(node, { attributes: true, attributeFilter: ['data-compact'] });
      const boundary = node.querySelector('.page-list-controls-boundary')!;
      const threshold =
        boundary.getBoundingClientRect().bottom - node.getBoundingClientRect().top + node.scrollTop;
      node.scrollTop = threshold + 2;
      // Frames let the real IntersectionObserver and scroll anchoring settle. No timer advances state.
      for (let frame = 0; frame < 30; frame++) await new Promise(requestAnimationFrame);
      observer.disconnect();
      return {
        events,
        compact: node.hasAttribute('data-compact'),
        scroll: node.scrollTop,
        threshold,
      };
    });
    expect(transitions.events).toEqual([true]);
    expect(transitions.compact).toBe(true);
    expect(Math.abs(transitions.scroll - transitions.threshold - 2)).toBeLessThan(1);
    await home.evaluate((node) => (node.scrollTop = 0));
    await expect(home).not.toHaveAttribute('data-compact');
  });

test('motion preferences control both directions of the compact transition', async ({ page }) => {
  await mount(page, 1440, 'light');
  const home = app(page).locator('[data-page-list]');
  const controls = home.locator('.page-list-controls');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(controls).toHaveCSS('animation-name', 'page-list-full-in');
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  await expect(controls).toHaveCSS('animation-name', 'page-list-compact-in');
  await home.evaluate((node) => (node.scrollTop = 0));
  await expect(home).not.toHaveAttribute('data-compact');
  await expect(controls).toHaveCSS('animation-name', 'page-list-full-in');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(controls).toHaveCSS('animation-name', 'none');
  await home.evaluate((node) => (node.scrollTop = 520));
  await expect(home).toHaveAttribute('data-compact', '');
  await expect(controls).toHaveCSS('animation-name', 'none');
});

for (const width of [1440, 390])
  test(`instant scroll and short viewport geometry restore the full header at ${width}`, async ({
    page,
  }) => {
    await mount(page, width, 'light');
    await page.setViewportSize({ width, height: 180 });
    const home = app(page).locator('[data-page-list]');
    const outside = await home.evaluate(
      (node) =>
        node.querySelector('.page-list-controls-boundary')!.getBoundingClientRect().top >
        node.getBoundingClientRect().bottom,
    );
    expect(outside).toBe(true);
    await home.evaluate((node) => (node.scrollTop = 520));
    await expect(home).toHaveAttribute('data-compact', '');
    await home.evaluate((node) => (node.scrollTop = 0));
    await expect(home).not.toHaveAttribute('data-compact');
    await home.evaluate((node) => (node.scrollTop = 520));
    await expect(home).toHaveAttribute('data-compact', '');
    if (width === 1440) {
      await home.getByRole('button', { name: 'Filter', exact: true }).click();
      await page.getByRole('menuitemcheckbox', { name: 'blog' }).click();
      await expect(home).not.toHaveAttribute('data-compact');
      expect(await home.evaluate((node) => node.scrollTop)).toBe(0);
    }
    await page.setViewportSize({ width, height: 1000 });
    await home.evaluate((node) => (node.scrollTop = 0));
    await expect(home).not.toHaveAttribute('data-compact');
    expect(await page.evaluate(() => window.__pageOperations)).toEqual([]);
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`row project groups keep zero, one and four memberships at ${width} ${theme}`, async ({
      page,
    }) => {
      await mount(page, width, theme);
      await page.evaluate(async () => {
        const fixture = '/test/page-list-browser.tsx';
        await (await import(fixture)).mountPageList('groups');
      });
      const alpha = rows(page).filter({ hasText: 'Alpha plan' });
      const beta = rows(page).filter({ hasText: 'Beta review' });
      const gamma = rows(page).filter({ hasText: 'Gamma notes' });
      await expect(alpha.locator('.page-project-group-chip')).toHaveText(['design', '+3']);
      expect(
        await alpha
          .locator('.page-project-group-chip > span')
          .first()
          .evaluate((node) => {
            const range = document.createRange();
            range.selectNodeContents(node);
            return range.getBoundingClientRect().width <= node.getBoundingClientRect().width;
          }),
      ).toBe(true);

      await expect(beta.locator('.page-project-group-chip')).toHaveText(['design']);
      await expect(gamma.locator('.page-project-group-chip')).toHaveCount(0);
      await expect(gamma.locator('.page-project-group')).toHaveAccessibleName('Add to project');
      await expect(alpha.locator('.page-project-group')).toHaveAccessibleName(
        'Projects for Alpha plan: design, q4 plan, research, ops',
      );
      for (const row of [alpha, beta, gamma]) {
        const group = row.locator('.page-project-group');
        await expect(group).toBeVisible();
        await expect(group.locator('.page-project-group-add')).toBeVisible();
        await group.focus();
        await group.press('Space');
        await expect(page.getByRole('menuitemcheckbox').first()).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(group).toBeFocused();
      }
      const metrics = await alpha
        .locator('.page-project-group-chip')
        .first()
        .evaluate((node) => {
          const style = getComputedStyle(node);
          return {
            height: node.getBoundingClientRect().height,
            font: style.fontSize,
            weight: style.fontWeight,
            start: style.paddingLeft,
            end: style.paddingRight,
          };
        });
      expect(metrics).toEqual({
        height: 22,
        font: '12px',
        weight: '500',
        start: '9px',
        end: '9px',
      });
      if (width === 1440)
        expect(
          (await alpha.locator('.page-row-projects').boundingBox())!.width,
        ).toBeGreaterThanOrEqual(150);
      await expect(rows(page)).toHaveCount(4);
      expect(await page.evaluate(() => window.__projectRequests)).toEqual([]);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    });
