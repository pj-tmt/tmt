import { readFile, writeFile } from 'node:fs/promises';
import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`pages index has symmetric toggle inset and newest-first title ties: ${width}px ${theme}`, async ({
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
      const toggle = fixture.getByRole('button', { name: 'Show archived', exact: true });
      await page.evaluate(() => {
        document.documentElement.style.setProperty('--tmt-ui-host-toggle-padding', '1px 2px 3px 0');
      });
      for (const side of ['top', 'right', 'bottom', 'left'])
        await expect(toggle).toHaveCSS(`padding-${side}`, '8px');
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
      await toggle.focus();
      await page.keyboard.press('Tab');
      await expect(first.getByRole('link', { name: 'Alpha update' })).toBeFocused();
      await page.keyboard.press('Tab');
      const actions = first.locator('summary');
      await expect(actions).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(rows.nth(1).getByRole('link', { name: 'Beta update' })).toBeFocused();
      await actions.click();
      await expect(first.getByRole('button', { name: 'Manage page' })).toBeVisible();
      await expect(first).toContainText('Page ID: c0000002-1111-4111-8111-000000000001');
      await expect(first.locator('.retention-hint')).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(first.locator('details')).not.toHaveAttribute('open');
      await expect(actions).toBeFocused();
      await page.screenshot({ path: await capturePath(`2168-few-${width}-${theme}.png`) });
      await toggle.click();
      await expect(rows).toHaveCount(2);
      await expect(rows.locator('.tmt-ui-list-state')).toHaveText(['Archived', 'Archived']);
      await expect(rows.getByRole('link')).toHaveCount(0);
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
        const action = node.querySelector('summary')!.getBoundingClientRect();
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
  await page.getByRole('button', { name: 'Show archived', exact: true }).click();
  const row = page.locator('#index-fixture [data-page-id="c0000005-1111-4111-8111-000000000001"]');
  await expect(row.locator('.tmt-ui-list-updated')).toHaveText('Update time unknown');
  await expect(row.locator('time')).toHaveCount(0);
  await expect(row.getByRole('link')).toHaveCount(0);
  await expect(row.getByText('Untitled page', { exact: true })).toBeVisible();
  await expect(row.locator('summary')).toHaveAccessibleName('Actions for Untitled page');
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
