import { expect, test, type Locator, type Page } from '@playwright/test';
import { destination } from '../test/ask-fixtures.js';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const colors = (badge: Locator) =>
  badge.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor };
  });
/** WCAG contrast of two opaque `rgb()` colours. */
function contrast(foreground: string, background: string) {
  const luminance = (rgb: string) => {
    const [r, g, b] = (rgb.match(/\d+(\.\d+)?/g) ?? []).slice(0, 3).map((part) => {
      const value = Number(part) / 255;
      return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [light, dark] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`the Comments list badge matches the page badge and clears the title: ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await page.goto('/');
      await run(page, 'mount', {
        creator: true,
        agents: [{ ...destination(), agentName: 'Atlas', presence: 'active' }],
      });
      await run(page, 'annotationLook');
      const app = page.locator('#ask-page-fixture');
      const pageBadge = page
        .frameLocator('#ask-page-fixture iframe')
        .locator('.annotation-number')
        .first();
      await expect(pageBadge).toBeVisible();
      await (await pageAction(app, 'Comments')).click();
      const panel = page.getByTestId('comments-panel');
      const row = panel.locator('.annotation-row').first();
      const badge = row.locator('.annotation-number');
      await expect(badge).toBeVisible();

      // The list row projects the one badge specification: same colours as the page's own badge.
      // The page renders its badge in a frame that may re-render, so compare until both settle.
      await expect
        .poll(async () => {
          const [onPage, inPanel] = [await colors(pageBadge), await colors(badge)];
          return onPage.color !== '' && JSON.stringify(onPage) === JSON.stringify(inPanel);
        })
        .toBe(true);
      const inPanel = await colors(badge);
      expect(contrast(inPanel.color, inPanel.background)).toBeGreaterThanOrEqual(4.5);

      // An 8px gap separates the badge from the title, as in the thread header row.
      const gap = await row.locator('strong').evaluate((strong) => {
        const badge = strong.querySelector('.annotation-number')!.getBoundingClientRect();
        const title = [...strong.childNodes].find(
          (node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim(),
        )!;
        const range = document.createRange();
        range.selectNodeContents(title);
        return range.getClientRects()[0].left - badge.right;
      });
      expect(gap).toBeGreaterThanOrEqual(8);
      expect(gap).toBeLessThanOrEqual(9);
      await app.getByTestId('comments-panel').screenshot({
        path: await capturePath(`3898-comments-badge-${width}-${theme}.png`),
      });
    });
