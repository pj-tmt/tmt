import { expect, test } from '@playwright/test';

// The flat strip's opt-in wrap: every tab shows in rows inside a narrow host, nothing scrolls or is
// clipped, and the DOM (so the arrow-key) order is the reading order.
test('a wrapping flat tab strip shows every tab in rows without scrolling', async ({ page }) => {
  await page.goto('/#tabs');
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
