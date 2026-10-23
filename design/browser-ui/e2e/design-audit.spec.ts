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
  for (const theme of ['light', 'dark'])
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
