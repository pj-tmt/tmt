import { readFile, writeFile } from 'node:fs/promises';
import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`pages index lists newest first with title ties and a menu for archived pages: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
      await page.evaluate(async () => {
        const path = '/test/page-index-browser.tsx';
        await (await import(path)).mountPageIndex('few');
      });
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const fixture = page.locator('#index-fixture');
      const rows = fixture.locator('.pages > li');
      await expect(rows).toHaveCount(3);
      expect(
        await rows.evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-page-id'))),
      ).toEqual([
        'c0000002-1111-4111-8111-000000000001',
        'c0000003-1111-4111-8111-000000000001',
        'c0000001-1111-4111-8111-000000000001',
      ]);
      await expect(rows.locator('.tmt-ui-list-title')).toHaveText([
        'Alpha update',
        'Beta update',
        'Older notes',
      ]);
      const first = rows.first();
      await expect(first.locator('time')).toHaveText('1 hour ago');
      await expect(first.locator('time')).toHaveAttribute('datetime', '2026-10-08T23:00:00.000Z');
      await expect(first.locator('time')).toHaveAttribute('title', /2026/);
      await expect(first.locator('.tmt-ui-list-state')).toHaveText('Shared');
      await first.getByRole('link', { name: 'Alpha update' }).focus();
      await page.keyboard.press('Tab');
      await expect(first.getByRole('button', { name: 'Labels for Alpha update' })).toBeFocused();
      await page.keyboard.press('Tab');
      const actions = first.getByRole('button', { name: /^Actions for/ });
      await expect(actions).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(rows.nth(1).getByRole('link', { name: 'Beta update' })).toBeFocused();
      await actions.click();
      await expect(page.getByRole('menuitem', { name: 'Manage page' })).toBeVisible();
      await expect(page.getByRole('menu')).toContainText(
        'Page ID: c0000002-1111-4111-8111-000000000001',
      );
      await expect(page.getByRole('menu').locator('.retention-hint')).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.getByRole('menu')).toHaveCount(0);
      await expect(actions).toBeFocused();
      await page.screenshot({ path: await capturePath(`2168-few-${width}-${theme}.png`) });
      await fixture.getByRole('button', { name: 'List options' }).click();
      await page.getByRole('menuitemcheckbox', { name: 'Show archived' }).click();
      await expect(rows).toHaveCount(2);
      await expect(rows.locator('.tmt-ui-list-state')).toHaveText(['Archived', 'Archived']);
      await expect(rows.getByRole('link')).toHaveCount(2);
      await expect(
        rows.first().getByRole('link', { name: 'Archived notes', exact: true }),
      ).toHaveAttribute('href', '/#/pages/c0000004-1111-4111-8111-000000000001');
      await expect(rows.last().locator('time')).toHaveCount(0);
      await expect(rows.last()).toContainText('Update time unknown');
      await page.screenshot({ path: await capturePath(`2168-archived-${width}-${theme}.png`) });
    });

    test(`200 rows keep one window scroll with only the header fixed: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
      await page.evaluate(async () => {
        const path = '/test/page-index-browser.tsx';
        await (await import(path)).mountPageIndex('many');
      });
      await page.evaluate((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      const rows = page.locator('#index-fixture .pages > li');
      await expect(rows).toHaveCount(200);
      await expect(rows.first().getByRole('link')).toHaveText('Page 001');
      await expect(rows.last().getByRole('link')).toHaveText('Page 200');
      await expect(rows.first()).toHaveCSS('box-shadow', 'none');
      await expect(rows.first()).toHaveCSS('border-bottom-width', '1px');
      await page.screenshot({ path: await capturePath(`2168-many-${width}-${theme}.png`) });
      await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
      await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(0);
      await expect(page.locator('.tmt-ui-header:visible')).toHaveCount(1);
      expect((await page.locator('.tmt-ui-header:visible').boundingBox())!.y).toBe(0);
      const scroll = await page.evaluate(() => {
        const visible = Array.from(document.querySelectorAll<HTMLElement>('body *')).filter(
          (node) => node.getClientRects().length,
        );
        return {
          window: document.scrollingElement === document.documentElement,
          horizontal: document.documentElement.scrollWidth > innerWidth,
          nested: visible
            .filter(
              (node) =>
                /^(auto|scroll)$/.test(getComputedStyle(node).overflowY) &&
                node.scrollHeight > node.clientHeight + 1,
            )
            .map((node) => node.className),
          fixed: visible
            .filter((node) => /^(fixed|sticky)$/.test(getComputedStyle(node).position))
            .map((node) => node.className),
        };
      });
      expect(scroll).toEqual({
        window: true,
        horizontal: false,
        nested: [],
        fixed: ['tmt-ui-header'],
      });
    });
  }

test('leaf row insets, height and controls stay equal while metadata stacks at 390', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    await (await import(path)).mountPageIndex('few');
  });
  const row = page.locator('#index-fixture .pages > li').first();
  await expect(row).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const measures = [];
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    measures.push(
      await row.evaluate((node) => {
        const style = getComputedStyle(node);
        const title = node.querySelector('.tmt-ui-list-title')!.getBoundingClientRect();
        const meta = node.querySelector('.tmt-ui-list-meta')!.getBoundingClientRect();
        const action = node.querySelector('.tmt-ui-menu button')!.getBoundingClientRect();
        return {
          height: node.getBoundingClientRect().height,
          padding: [style.paddingTop, style.paddingRight, style.paddingBottom, style.paddingLeft],
          rule: style.borderBottomWidth,
          action: [action.width, action.height],
          stacked: meta.top >= title.bottom,
        };
      }),
    );
  }
  await writeFile(capturePath('2168-row-geometry.json'), JSON.stringify(measures, null, 2));
  expect(measures[0].padding).toEqual(['10px', '14px', '10px', '14px']);
  expect(measures[0].stacked).toBe(false);
  expect(measures[1].stacked).toBe(true);
  expect({ ...measures[0], stacked: true }).toEqual(measures[1]);
});

test('static toggle markup keeps its symmetric inset when the obsolete host token is set', async ({
  page,
}) => {
  const leaf = new URL('../../../../../design/browser-ui/', import.meta.url);
  const html = await readFile(new URL('test/fixtures/static.html', leaf), 'utf8');
  await page.route('**/leaf-controls/', (route) =>
    route.fulfill({ contentType: 'text/html', body: html }),
  );
  await page.route('**/generated/static.css', async (route) =>
    route.fulfill({
      contentType: 'text/css',
      body: await readFile(new URL('generated/static.css', leaf), 'utf8'),
    }),
  );
  await page.route('**/leaf-controls/controls.css', async (route) =>
    route.fulfill({
      contentType: 'text/css',
      body: await readFile(new URL('test/fixtures/controls.css', leaf), 'utf8'),
    }),
  );
  await page.route('**/leaf-controls/static.js', (route) =>
    route.fulfill({ contentType: 'text/javascript', body: '' }),
  );
  await page.goto('/leaf-controls/');
  await page.evaluate(() => {
    document.documentElement.style.setProperty('--tmt-ui-host-toggle-padding', '0');
  });
  for (const toggle of await page.locator('.tmt-ui-toggle').all()) {
    for (const side of ['top', 'right', 'bottom', 'left'])
      await expect(toggle).toHaveCSS(`padding-${side}`, '8px');
  }
  await expect(page.locator('.tmt-ui-toggle')).toHaveCount(2);
});

test('metadata order follows update timestamps before English title ties', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    await (await import(path)).mountPageIndex('few');
  });
  const rows = page.locator('#index-fixture .pages > li');
  await expect(rows).toHaveCount(3);
  expect(
    await rows.evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-page-id'))),
  ).toEqual([
    'c0000002-1111-4111-8111-000000000001',
    'c0000003-1111-4111-8111-000000000001',
    'c0000001-1111-4111-8111-000000000001',
  ]);
});

test('an out-of-range update has the same unknown copy and no fabricated time element', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    await (await import(path)).mountPageIndex('few', Number.MAX_SAFE_INTEGER);
  });
  await page.getByRole('button', { name: 'List options' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'Show archived' }).click();
  const row = page.locator('#index-fixture [data-page-id="c0000005-1111-4111-8111-000000000001"]');
  await expect(row.locator('.tmt-ui-list-updated')).toHaveText('Update time unknown');
  await expect(row.locator('time')).toHaveCount(0);
  await expect(row.getByRole('link', { name: 'Untitled page', exact: true })).toHaveAttribute(
    'href',
    '/#/pages/c0000005-1111-4111-8111-000000000001',
  );
  await expect(row.getByText('Untitled page', { exact: true })).toBeVisible();
  await expect(row.getByRole('button', { name: /^Actions for/ })).toHaveAccessibleName(
    'Actions for Untitled page',
  );
});

test('the first committed index render uses the actual time snapshot', async ({ page }) => {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-09T00:00:00Z'));
  await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    await (await import(path)).mountPageIndex('few');
  });
  await expect(page.locator('#index-fixture .pages > li')).toHaveCount(3);
  const initialLabels = await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    return (await import(path)).firstUpdateLabels();
  });
  expect(initialLabels).toEqual(['1 hour ago', '1 hour ago', '3 days ago']);
});

test('Home fills only uncached visible title rows, including under StrictMode', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 600 });
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-index-browser.tsx';
    const { mountPageIndex } = await import(path);
    await mountPageIndex('missing');
  });
  const rows = page.locator('#index-fixture [data-page-id]');
  await expect(rows).toHaveCount(30);
  await expect(rows.first().getByRole('link')).toHaveText('Cached title');
  const reads = () =>
    page.evaluate(async () => {
      const path = '/test/page-index-browser.tsx';
      return (await import(path)).titleReadCalls() as { id: string; visible: boolean }[];
    });
  await expect.poll(async () => (await reads()).length).toBeGreaterThan(0);
  const initial = await reads();
  const cachedId = await rows.first().getAttribute('data-page-id');
  const lastId = await rows.last().getAttribute('data-page-id');
  expect(initial.every((read) => read.visible)).toBe(true);
  expect(initial.some((read) => read.id === cachedId || read.id === lastId)).toBe(false);
  await rows.last().scrollIntoViewIfNeeded();
  await expect(rows.last().getByRole('link')).toHaveText(`Loaded ${lastId}`);
  const completed = await reads();
  expect(completed.every((read) => read.visible && read.id !== cachedId)).toBe(true);
  expect(new Set(completed.map((read) => read.id)).size).toBe(completed.length);
  expect(completed.length).toBeLessThanOrEqual(24);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`an unresolved list title and null update keep neutral placeholders until resolution ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/page-index-browser.tsx';
        await (await import(path)).mountPageIndex('pending');
      });
      const row = page.locator('#index-fixture .pages > li');
      await expect(row).toHaveCount(1);
      await expect(row.getByRole('link')).toHaveText('Untitled page');
      await expect(row.locator('.page-metadata')).toHaveText('Update time unknown');
      await expect(row.locator('time')).toHaveCount(0);
      await expect(row).not.toContainText(/undefined|null/);
      const firstRows = await page.evaluate(async () => {
        const path = '/test/page-index-browser.tsx';
        return (await import(path)).firstRowTexts();
      });
      expect(firstRows).toHaveLength(1);
      expect(firstRows![0]).toContain('Untitled page');
      expect(firstRows![0]).toContain('Update time unknown');
      expect(firstRows![0]).not.toMatch(/undefined|null/);
      await expect(
        row.getByRole('button', { name: 'Actions for Untitled page', exact: true }),
      ).toBeVisible();
      await expect
        .poll(() =>
          page.evaluate(async () => {
            const path = '/test/page-index-browser.tsx';
            return (await import(path)).hasPendingTitleRead();
          }),
        )
        .toBe(true);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({ path: capturePath(`3218-pending-${width}-${theme}.png`) });
      await page.evaluate(async () => {
        const path = '/test/page-index-browser.tsx';
        (await import(path)).resolvePendingTitle();
      });
      await expect(row.getByRole('link')).toHaveText('Resolved title');
      await expect(row.locator('.page-metadata')).toHaveText('Update time unknown');
      await expect(row).not.toContainText(/undefined|null/);
    });
  }
