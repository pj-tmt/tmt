import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import {
  controlBoundaries,
  controlRadii,
  focusFrames,
  oneSidedBorders,
  textAtSurfaceEdge,
} from './audits';

/** The gallery's own pages in a real browser: every demo of every theme, at desktop and phone
 * width, and each frame page on its own. */
async function gallery(page: Page, width: number) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await expect(page.locator('.gallery-main #chips')).toBeAttached();
}

for (const width of [1440, 390]) {
  test(`gallery demos: no one-sided border, no text on a surface edge, one radius (${width}px)`, async ({
    page,
  }) => {
    await gallery(page, width);
    expect(await oneSidedBorders(page, '.gallery-main')).toEqual([]);
    expect(await textAtSurfaceEdge(page, '.gallery-main')).toEqual([]);
    const radii = await controlRadii(page, '.gallery-main');
    expect(radii.token).toBe('8px');
    expect(radii.off).toEqual([]);
  });
}

test('gallery demos: a control drawn by its border keeps 3:1 against its surface', async ({
  page,
}) => {
  await gallery(page, 1440);
  const { borders } = await controlBoundaries(page, '.gallery-main');
  expect(borders.measured).toBeGreaterThan(10);
  expect(borders.misses).toEqual([]);
});

// Fields keep the soft v2 edge by decision (gui-style.md "Fields"): they are exempt from the 3:1
// border rule, named in `fieldsKeepSoftEdge`, and must fill differently from their surface and
// show a label or a value.
test('gallery demos: a field with a soft edge fills differently and is labelled', async ({
  page,
}) => {
  await gallery(page, 1440);
  const { fields } = await controlBoundaries(page, '.gallery-main');
  expect(fields.measured).toBeGreaterThan(5);
  expect(fields.sameFill).toEqual([]);
  expect(fields.unlabelled).toEqual([]);
});

test('gallery demos: each focus frame is drawn once and never clipped', async ({ page }) => {
  await gallery(page, 1440);
  const found = await focusFrames(page, '.gallery-main');
  expect(found.checked).toBeGreaterThan(20);
  // A ring a rounded overflow ancestor would clip is that ancestor's own border.
  expect(found.clipped).toEqual([]);
  // A control inside a focus-carrying ancestor adds no outline, border, fill or shadow.
  expect(found.doubled).toEqual([]);
});

const frames: [string, string][] = [
  ['dialog', ''],
  ['drawer', ''],
  ['drawer', '&width=wide'],
  ['source', ''],
  ['source', '&busy=true'],
  ['menu', ''],
  ['notifications', ''],
  ['shell', ''],
  ['shell', '&rail=collapsed'],
  ['hosted', ''],
  ...['dialog', 'confirm', 'sheet', 'sheet-bottom', 'menu', 'popover', 'tooltip'].map(
    (kind): [string, string] => ['ui-overlay', `&kind=${kind}`],
  ),
  ...['', 'filtered', 'empty', 'busy', 'disabled'].map((state): [string, string] => [
    'ui-overlay',
    `&kind=combobox&state=${state}`,
  ]),
];
for (const [frame, params] of frames)
  for (const theme of ['light', 'dark'] as const)
    test(`frame ${frame}${params} (${theme}): the same audits`, async ({ page }) => {
      await page.setViewportSize({ width: frame === 'ui-overlay' ? 560 : 1080, height: 620 });
      await page.goto(`/?frame=${frame}&theme=${theme}${params}`);
      await expect(page.locator('#root')).not.toBeEmpty();
      await page.evaluate(async () => {
        await document.fonts.ready;
        await Promise.all(
          document
            .getAnimations()
            .filter((animation) => Number.isFinite(animation.effect?.getComputedTiming().endTime))
            .map((animation) => animation.finished.catch(() => {})),
        );
      });
      expect(await oneSidedBorders(page, 'body')).toEqual([]);
      expect(await textAtSurfaceEdge(page, 'body')).toEqual([]);
      expect((await controlRadii(page, 'body')).off).toEqual([]);
    });

for (const width of [1440, 390])
  test(`heatmaps use wide short token-radius bars and retain keyboard tips at ${width}`, async ({
    page,
  }) => {
    // Isolate keyboard focus from the full gallery's lazy-loading overlay demonstrations.
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?frame=charts');
    const maps = page.locator('.tmt-ui-chart[data-layout="heatmap"]');
    await expect(maps).toHaveCount(4);
    for (const map of await maps.all()) {
      const cell = map.locator('.tmt-ui-chart-slot:not(.tmt-ui-chart-slot-none)').first();
      await cell.scrollIntoViewIfNeeded();
      const box = (await cell.boundingBox())!;
      expect(box.width / box.height).toBeGreaterThanOrEqual(1.9);
      await expect(cell).toHaveCSS('border-radius', '6px');
      await cell.focus();
      await expect(cell).toBeFocused();
      await cell.dispatchEvent('pointerleave');
      await expect(map.locator('.tmt-ui-chart-tip')).toBeVisible();
      await page.keyboard.press('ArrowRight');
      expect(
        await map
          .locator('.tmt-ui-chart-slot')
          .evaluateAll((nodes) => nodes.some((node) => node === document.activeElement)),
      ).toBe(true);
    }
  });

for (const scheme of ['light', 'dark'] as const) {
  test(`theme roles survive nested theme switches and ${scheme} system preference`, async ({
    page,
  }) => {
    const tokens = JSON.parse(
      readFileSync(new URL('../../tokens/tokens.json', import.meta.url), 'utf8'),
    );
    await page.emulateMedia({ colorScheme: scheme });
    await gallery(page, 1440);
    const roles = Object.entries({
      color: tokens.browser.color,
      surface: tokens.browser.surface,
      shadow: tokens.browser.shadow,
      'brand-mark': tokens.brand.mark,
    }).flatMap(([group, values]) =>
      Object.entries(values as Record<string, { light: string; dark: string }>)
        .filter(([name]) => !name.startsWith('$'))
        .map(([name, value]) => ({ name: `--tmt-ui-${group}-${name}`, value })),
    );
    const actual = await page.evaluate((roles) => {
      document.documentElement.removeAttribute('data-theme');
      const outer = document.createElement('div');
      outer.dataset.theme = 'light';
      const inner = document.createElement('div');
      inner.dataset.theme = 'dark';
      const last = document.createElement('div');
      last.dataset.theme = 'light';
      document.body.append(outer);
      outer.append(inner);
      inner.append(last);
      const values = [document.documentElement, outer, inner, last].map((node) =>
        roles.map(({ name }) => getComputedStyle(node).getPropertyValue(name).trim().toLowerCase()),
      );
      outer.remove();
      return values;
    }, roles);
    expect(actual).toEqual(
      [scheme, 'light', 'dark', 'light'].map((theme) =>
        roles.map(({ value }) => value[theme as 'light' | 'dark'].toLowerCase()),
      ),
    );
  });
}
for (const width of [1586, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`website shell metrics at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 992 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      const fontRequests: string[] = [];
      page.on('request', (request) => {
        if (
          request.resourceType() === 'font' ||
          /fonts\.(googleapis|gstatic)\.com/.test(request.url())
        )
          fontRequests.push(request.url());
      });
      await page.goto('/?frame=shell&theme=' + theme);
      await expect(page.locator('.tmt-ui-page-title')).toBeVisible();
      await expect(page.locator('.tmt-ui-header')).toHaveCSS(
        'height',
        width === 390 ? '56px' : '62px',
      );
      await expect(page.locator('.tmt-ui-page')).toHaveCSS(
        'padding',
        width === 390 ? '24px 18px' : '33px 36px',
      );
      await expect(page.locator('.tmt-ui-page-title')).toHaveCSS(
        'font-size',
        width === 390 ? '29px' : '37px',
      );
      await expect(page.locator('.tmt-ui-card-title')).toHaveCSS('font-size', '19px');
      if (width === 1586) {
        await expect(page.locator('.tmt-ui-rail')).toHaveCSS('width', '232px');
        await expect(page.locator('.tmt-ui-rail')).toHaveCSS('padding', '22px 12px 17px');
        await expect(page.locator('.tmt-ui-rail-item').first()).toHaveCSS('height', '45px');
        await expect(page.locator('.tmt-ui-rail-sub-item').first()).toHaveCSS('height', '33px');
        const indent = await page.evaluate(() => {
          const parent = document.querySelector(
            '.tmt-ui-rail-item[aria-current="page"] .tmt-ui-shell-icon',
          )!;
          const label = document.querySelector('.tmt-ui-rail-sub-item .tmt-ui-module-menu-label')!;
          return label.getBoundingClientRect().x - parent.getBoundingClientRect().x;
        });
        expect(indent).toBeCloseTo(8, 1);
        const icon = page.locator('.tmt-ui-rail-item').first().locator('svg');
        const expandedX = (await icon.boundingBox())!.x;
        await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
        await expect(page.locator('.tmt-ui-rail')).toHaveCSS('width', '56px');
        await expect.poll(async () => (await icon.boundingBox())!.x).toBe(expandedX);
        const railBox = (await page.locator('.tmt-ui-rail').boundingBox())!;
        const iconBox = (await icon.boundingBox())!;
        expect(iconBox.x + iconBox.width).toBeLessThan(railBox.x + railBox.width);
        const badgeInset = await page
          .locator('.tmt-ui-rail-item .tmt-ui-shell-count')
          .first()
          .evaluate((badge) => {
            const list = badge.closest('ul')!.getBoundingClientRect();
            const range = document.createRange();
            range.selectNodeContents(badge);
            const glyph = range.getBoundingClientRect();
            return { left: glyph.left - list.left, right: list.right - glyph.right };
          });
        expect(badgeInset.left).toBeGreaterThanOrEqual(3);
        expect(badgeInset.right).toBeGreaterThanOrEqual(3);
      }
      expect(fontRequests).toEqual([]);
    });

test('small avatar initials retain an inset with platform fallback fonts', async ({ page }) => {
  await page.goto('/#avatar');
  for (const family of ['Arial, sans-serif', 'Verdana, sans-serif']) {
    await page.locator('.tmt-ui-avatar').evaluateAll((nodes, font) => {
      for (const node of nodes) (node as HTMLElement).style.fontFamily = font;
    }, family);
    expect(await textAtSurfaceEdge(page, '#avatar')).toEqual([]);
  }
});

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 390])
    for (const selected of ['pages', 'project']) {
      test(`side-menu hierarchy ${selected} ${width} ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 992 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await page.goto(`/?frame=shell&counts&theme=${theme}&page=${selected}`);
        const root = page.locator('.tmt-ui-rail-sub');
        if (width === 390) {
          await expect(root).toHaveCount(0);
          const tabs = page.locator('.tmt-ui-module-menu-item');
          await expect(tabs).toHaveCount(5);
          await tabs.first().focus();
          await page.keyboard.press('Tab');
          await expect(tabs.nth(1)).toBeFocused();
          await expect(page.locator('.tmt-ui-module-menu [aria-current="page"]')).toHaveAttribute(
            'href',
            selected === 'project' ? /\/colab\/p\/release$/ : /\/colab\/pages$/,
          );
          return;
        }
        const group = root.getByRole('list', { name: 'Projects', exact: true });
        await expect(group.getByRole('link')).toHaveCount(2);
        expect(await group.evaluate((node) => node.parentElement?.tagName)).toBe('LI');
        const all = root.locator('a[href$="/colab/pages"]');
        const project = group.locator('a[href$="/colab/p/release"]');
        const settings = root.locator('a[href$="/colab/settings"]');
        for (const flat of [all, settings])
          expect(await flat.evaluate((node) => node.parentElement?.parentElement?.className)).toBe(
            'tmt-ui-rail-sub',
          );
        const labelX = (link: typeof all) =>
          link
            .locator('.tmt-ui-module-menu-label')
            .evaluate((node) => node.getBoundingClientRect().x);
        expect((await labelX(project)) - (await labelX(all))).toBe(8);
        expect(await labelX(settings)).toBe(await labelX(all));
        const allBox = (await all.boundingBox())!;
        const projectBox = (await project.boundingBox())!;
        expect(projectBox.x + projectBox.width).toBe(allBox.x + allBox.width);
        const current = root.locator('[aria-current="page"]');
        await expect(current).toHaveAttribute(
          'href',
          selected === 'project' ? /\/colab\/p\/release$/ : /\/colab\/pages$/,
        );
        await current.focus();
        const colors = await current.evaluate((node) => {
          const style = getComputedStyle(node);
          const luminance = (color: string) =>
            color
              .match(/[\d.]+/g)!
              .slice(0, 3)
              .map((n) => Number(n) / 255)
              .map((n) => (n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4))
              .reduce((sum, n, i) => sum + n * [0.2126, 0.7152, 0.0722][i]!, 0);
          const a = luminance(style.color);
          const b = luminance(style.backgroundColor);
          return {
            ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05),
            outline: style.outlineColor,
            ink: style.color,
            marker: getComputedStyle(node, '::before').content,
          };
        });
        expect(colors.ratio).toBeGreaterThanOrEqual(4.5);
        expect(colors.outline).toBe(colors.ink);
        expect(colors.marker).toBe('none');
        expect(
          await page
            .locator('.tmt-ui-rail-item[aria-current="page"]')
            .evaluate((node) => getComputedStyle(node, '::before').content),
        ).not.toBe('none');
        const frames = await focusFrames(page, '.tmt-ui-rail');
        expect(frames.clipped).toEqual([]);
        expect(frames.doubled).toEqual([]);
        await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
        await expect(root).toBeHidden();
        await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
        await expect(group).toBeVisible();
      });
    }
