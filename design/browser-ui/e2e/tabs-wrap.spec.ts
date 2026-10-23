import { expect, test } from '@playwright/test';
import { controlBoundaries, focusFrames, textAtSurfaceEdge } from './audits';

// The flat strip's opt-in wrap: every tab shows in rows inside a narrow host, nothing scrolls or is
// clipped, and the DOM (so the arrow-key) order is the reading order.
test('a wrapping flat tab strip shows every tab in rows without scrolling', async ({ page }) => {
  await page.goto('/#tabs');
  // Both flat examples share a document; each tab must reference its own retained panel.
  const ids = await page.locator('#tabs [id]').evaluateAll((nodes) => nodes.map((node) => node.id));
  expect(new Set(ids).size).toBe(ids.length);
  for (const theme of ['light', 'dark']) {
    const list = page
      .locator('.gallery-pair', { has: page.locator('h3', { hasText: 'flat, wrapping' }) })
      .locator(`.gallery-column[data-theme="${theme}"] [role="tablist"]`);
    await list.scrollIntoViewIfNeeded();
    const measured = await list.evaluate((node) => {
      const bounds = node.getBoundingClientRect();
      const tabs = [...node.querySelectorAll<HTMLElement>('[role="tab"]')].map((tab) => {
        const box = tab.getBoundingClientRect();
        return { left: box.left, right: box.right, top: box.top };
      });
      return {
        scrolls: node.scrollWidth > node.clientWidth,
        inside: tabs.every((box) => box.left >= bounds.left && box.right <= bounds.right + 0.5),
        rows: new Set(tabs.map((box) => Math.round(box.top))).size,
        ordered: tabs.every((box, index) => index === 0 || box.top > tabs[index - 1]!.top - 1),
      };
    });
    expect(measured.scrolls).toBe(false);
    expect(measured.inside).toBe(true);
    expect(measured.rows).toBeGreaterThan(1);
    expect(measured.ordered).toBe(true);
  }
});

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 390]) {
    test(`flat selection stays legible and follows keyboard selection at ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
      await page.goto('/?theme=' + theme + '#tabs');
      const scope =
        '#tabs .gallery-pair:has(.tmt-ui-tabs[data-wrap]) .gallery-column[data-theme="' +
        theme +
        '"]';
      const list = page.locator(scope).getByRole('tablist');
      const selected = list.locator('[aria-selected="true"]');
      await selected.scrollIntoViewIfNeeded();
      const contrast = () =>
        selected.evaluate((node) => {
          const luminance = (colour: string) => {
            const channels = (colour.match(/[\d.]+/g) ?? [])
              .slice(0, 3)
              .map(Number)
              .map((value) => {
                const s = value / 255;
                return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
              });
            return channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722;
          };
          const style = getComputedStyle(node);
          const background = luminance(style.backgroundColor);
          const ratio = (colour: string) => {
            const ink = luminance(colour);
            return (Math.max(ink, background) + 0.05) / (Math.min(ink, background) + 0.05);
          };
          return {
            label: ratio(style.color),
            count: ratio(getComputedStyle(node.querySelector('.tmt-ui-tab-count')!).color),
            focus: ratio(style.borderTopColor),
            fill: style.backgroundColor,
          };
        });
      expect((await contrast()).label).toBeGreaterThanOrEqual(4.5);
      expect((await contrast()).count).toBeGreaterThanOrEqual(4.5);
      const fill = (await contrast()).fill;
      await expect(selected).toHaveCSS('border-top-color', 'rgba(0, 0, 0, 0)');
      const resting = (await selected.boundingBox())!;
      await selected.focus();
      await expect(selected).toBeFocused();
      const focused = (await selected.boundingBox())!;
      expect(focused.width).toBe(resting.width);
      expect(focused.height).toBe(resting.height);
      await page.keyboard.press('ArrowRight');
      await expect(selected).toBeFocused();
      await expect(selected).toHaveAccessibleName(/Answered/);
      expect((await contrast()).label).toBeGreaterThanOrEqual(4.5);
      expect((await contrast()).count).toBeGreaterThanOrEqual(4.5);
      expect((await contrast()).focus).toBeGreaterThanOrEqual(3);
      expect((await contrast()).fill).toBe(fill);
      await expect(list.locator('[aria-selected="false"]').first()).not.toHaveCSS(
        'background-color',
        fill,
      );
      const boundaries = await controlBoundaries(page, scope);
      expect(boundaries.borders.misses).toEqual([]);
      expect(await textAtSurfaceEdge(page, scope)).toEqual([]);
      const frames = await focusFrames(page, scope);
      expect(frames.checked).toBeGreaterThan(0);
      expect(frames.clipped).toEqual([]);
      expect(frames.doubled).toEqual([]);
    });
  }
