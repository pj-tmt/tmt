import { expect, test, type Page } from '@playwright/test';

/**
 * The floating page actions (Chat, selection mode) sit in a viewport corner. They never cover a
 * control of the page chrome: at a phone's width the expanded work band ends with its Add row,
 * which the group used to sit over (#4588).
 */
const views = [
  { width: 390, height: 898 },
  { width: 1440, height: 900 },
] as const;

async function openWorkBand(page: Page, tall = false) {
  await page.goto('/');
  await page.evaluate(async (tall) => {
    const path = '/test/work-page-browser.tsx';
    (await import(path)).mountWorkPage(true, 'tree', tall);
  }, tall);
  const band = page.getByRole('region', { name: 'Work on this page' });
  await band.getByRole('button', { name: 'Expand work panel' }).click();
  await band.getByRole('button', { name: 'Expand Remote pages v2' }).click();
  await expect(
    band.locator('.work-add-row').getByRole('button', { name: 'Add', exact: true }),
  ).toBeVisible();
  // On a page taller than the viewport the open band can fill the room under the bar; the group
  // then hides rather than covering anything.
  if (!tall)
    await expect(page.locator('.page-floating-actions button:visible').first()).toBeVisible();
}

/** The controls of the page chrome that the floating group covers right now. */
const covered = (page: Page) =>
  page.evaluate(() => {
    // A group hidden for lack of room covers nothing.
    const group = [...document.querySelectorAll('.page-floating-actions')]
      .filter((node) => getComputedStyle(node.parentElement!).visibility !== 'hidden')
      .map((node) => node.getBoundingClientRect())
      .find((box) => box.width > 0);
    if (!group) return [];
    const controls = document.querySelectorAll<HTMLElement>(
      'button, a[href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])',
    );
    return [...controls]
      .filter(
        (node) =>
          !node.closest('.selection-mode, .tmt-ui-visually-hidden') &&
          node.getClientRects().length > 0 &&
          getComputedStyle(node).visibility !== 'hidden',
      )
      .filter((node) => {
        const box = node.getBoundingClientRect();
        return (
          box.width > 0 &&
          box.height > 0 &&
          box.left < group.right &&
          box.right > group.left &&
          box.top < group.bottom &&
          box.bottom > group.top
        );
      })
      .map((node) => (node.getAttribute('aria-label') ?? node.textContent ?? node.tagName).trim());
  });

for (const { width, height } of views) {
  test(`the floating actions stay off the work band's controls at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await openWorkBand(page);
    await expect.poll(() => covered(page)).toEqual([]);
    // The group moved up and stayed in view, rather than being hidden to avoid the row.
    const group = page.locator('.page-floating-actions:visible').first();
    await expect(group).toBeVisible();
    const box = await group.boundingBox();
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height).toBeLessThanOrEqual(height);
  });

  test(`wider controls do not bring the group back over them at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    // A wider font or label makes a row's last button reach the group's column.
    await page.addInitScript(() => {
      addEventListener('DOMContentLoaded', () => {
        const style = document.createElement('style');
        style.textContent =
          '.work-band :is(.tmt-ui-action, .tmt-ui-menu-trigger, .work-add-text) { padding-inline: 14px; }';
        document.head.append(style);
      });
    });
    await openWorkBand(page);
    await expect.poll(() => covered(page)).toEqual([]);
  });

  test(`the last control at the page end is clear of the floating actions at ${width}`, async ({
    page,
  }) => {
    // Keep this case about reaching the page end by scrolling a page taller than the viewport.
    await page.setViewportSize({ width, height: 480 });
    await openWorkBand(page, true);
    await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
    await expect.poll(() => page.evaluate(() => Math.round(scrollY))).toBeGreaterThan(0);
    await expect.poll(() => covered(page)).toEqual([]);
  });
}
