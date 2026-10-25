import { expect, test, type Locator, type Page } from '@playwright/test';
import { destination } from '../test/ask-fixtures.js';
import { capturePath } from './captures.js';
import { commentsRegion } from '../test/comments-region.js';
import { pageAction, showResolvedOn } from '../test/page-actions.js';

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

      // An 8px gap at least separates the badge from the row's text column.
      const gap = await row.locator('.annotation-row-text').evaluate((text) => {
        const badge = text
          .parentElement!.querySelector('.annotation-number')!
          .getBoundingClientRect();
        return text.getBoundingClientRect().left - badge.right;
      });
      expect(gap).toBeGreaterThanOrEqual(8);
      expect(gap).toBeLessThanOrEqual(9);
      await app.getByTestId('comments-panel').screenshot({
        path: await capturePath(`3898-comments-badge-${width}-${theme}.png`),
      });
    });

test('the Comments list is the website list: views, rows and an empty state per view', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await run(page, 'seedThread');
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  const panel = commentsRegion(page);
  // Three views in one strip (no Proposals view without a proposal), and the add action under it.
  const views = panel.getByRole('tablist', { name: 'Comment views' });
  await expect(views.getByRole('tab')).toHaveText(['Open', 'Needs you', 'Resolved']);
  await expect(views.getByRole('tab', { name: 'Open', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  const add = panel.getByRole('button', { name: '+ Comment on page', exact: true });
  const [strip, action, frame] = [
    await views.boundingBox(),
    await add.boundingBox(),
    await panel.locator('.tmt-ui-tabs').boundingBox(),
  ];
  // Under the strip, ending at the panel's right edge, with the list right below it.
  expect(action!.y).toBeGreaterThanOrEqual(strip!.y + strip!.height - 1);
  expect(Math.abs(action!.x + action!.width - (frame!.x + frame!.width))).toBeLessThanOrEqual(2);
  const firstRow = await panel.getByTestId('annotation-row').first().boundingBox();
  expect(firstRow!.y - (action!.y + action!.height)).toBeLessThanOrEqual(16);
  // A row is the number, the title, the quoted passage and one small line; no byline or time.
  const row = panel.getByTestId('annotation-row');
  await expect(row).toHaveCount(1);
  await expect(row.locator('.annotation-number')).toHaveText('1');
  await expect(row.locator('strong')).not.toBeEmpty();
  await expect(row.locator('.annotation-row-quote')).toHaveText('Exact selected text');
  await expect(row.getByTestId('thread-row-status')).toHaveText('1 comment');
  await expect(row.locator('time')).toHaveCount(0);
  // The website's list: a 1px rule closes the view strip and the row, which has 18px of vertical
  // padding after the first row's tighter top; the quote and meta are the caption size, the
  // title the body size; the selected view tab carries the selection tint.
  await expect(panel.locator('.tmt-ui-tabs-header')).toHaveCSS('border-bottom-width', '1px');
  await expect(row).toHaveCSS('border-bottom-width', '1px');
  await expect(row).toHaveCSS('padding-bottom', '18px');
  await expect(row.locator('.annotation-row-quote')).toHaveCSS('font-size', '11px');
  await expect(row.getByTestId('thread-row-status')).toHaveCSS('font-size', '11px');
  await expect(row.locator('strong')).toHaveCSS('font-size', '14px');
  expect(
    await views
      .getByRole('tab', { name: 'Open', exact: true })
      .evaluate((node) => getComputedStyle(node).backgroundColor),
  ).not.toBe('rgba(0, 0, 0, 0)');
  // The other views are empty here, and say so.
  await views.getByRole('tab', { name: 'Needs you', exact: true }).click();
  await expect(panel.getByTestId('annotation-row')).toHaveCount(0);
  const empty = panel.getByTestId('comments-empty');
  await expect(empty.getByRole('heading')).toHaveText('Nothing waiting here.');
  await expect(empty).toContainText('You are caught up in this view.');
  await views.getByRole('tab', { name: 'Resolved', exact: true }).click();
  await expect(empty).toContainText('Resolved threads will appear here.');
});

test('a row opens the Discussion view: header, title passage, turns, reply, Resolve in the header', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  // The resolved thread stays in view, to be reopened, only with Show resolved on.
  await showResolvedOn(page);
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await run(page, 'seedThread');
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  const drawer = commentsRegion(page);
  await expect(drawer.getByRole('heading', { level: 2 })).toHaveText('Comments');
  await drawer.getByTestId('annotation-row').click();
  // The list is replaced; the drawer reads Discussion with the thread's position.
  await expect(drawer.getByRole('heading', { level: 2 })).toHaveText('Discussion');
  await expect(drawer.locator('.tmt-ui-drawer-caption')).toHaveText('1 / 1');
  await expect(drawer.getByTestId('annotation-row')).toHaveCount(0);
  // An open thread has no state line, and no row of its own to leave by: the position, which is the
  // way back to the list, and the thread arrows stand in the header.
  await expect(drawer.getByTestId('discussion-state')).toHaveCount(0);
  await expect(drawer.getByRole('button', { name: 'All comments', exact: true })).toHaveText(
    '1 / 1',
  );
  await expect(drawer.getByRole('button', { name: 'Previous thread', exact: true })).toBeDisabled();
  await expect(drawer.getByRole('button', { name: 'Next thread', exact: true })).toBeDisabled();
  const thread = drawer.getByTestId('comment-thread');
  // The title is the passage: its number and the quoted text, no heading of the first message.
  await expect(thread.locator('.discussion-title')).toHaveCount(0);
  const quote = thread.locator('.discussion-quote');
  await expect(quote.locator('.annotation-number')).toHaveText('1');
  await expect(quote.locator('q')).toHaveText('Exact selected text');
  await expect(thread.getByTestId('comment-entry')).toHaveCount(1);
  const reply = thread.getByRole('combobox', { name: 'Message', exact: true });
  await expect(reply).toBeVisible();
  // The reply field is one box flush with the content column's edges: the text, then its
  // toolbar with the round Send at the end, and the helper line under the box.
  const column = (await quote.boundingBox())!;
  const box = (await thread.locator('.annotation-compose .rich-input-surface').boundingBox())!;
  const field = (await thread.locator('.annotation-compose .rich-input-field').boundingBox())!;
  // At rest it is 44px high: a line and a half of text under 10px of padding above and 4px below.
  expect(Math.abs(field.height - 44)).toBeLessThanOrEqual(1);
  expect(Math.abs(box.x - column.x)).toBeLessThanOrEqual(1);
  const send = thread.getByRole('button', { name: 'Send', exact: true });
  await expect(send).toHaveText('');
  const sendBox = (await send.boundingBox())!;
  expect(sendBox.y).toBeGreaterThanOrEqual(field.y + field.height - 1);
  expect(sendBox.y + sendBox.height).toBeLessThanOrEqual(box.y + box.height + 1);
  expect(Math.abs(sendBox.x + sendBox.width - (box.x + box.width))).toBeLessThanOrEqual(8);
  const helper = (await thread.locator('.annotation-helper').boundingBox())!;
  expect(helper.y).toBeGreaterThanOrEqual(box.y + box.height);
  // There is no footer: Resolve stands in the header, flips the status line and offers Reopen.
  await expect(thread.locator('.discussion-footer')).toHaveCount(0);
  const actions = drawer.locator('.tmt-ui-drawer-header-actions');
  await actions.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(drawer.getByTestId('discussion-state')).toContainText('Resolved');
  await expect(actions.getByRole('button', { name: 'Reopen', exact: true })).toBeVisible();
  // All comments returns to the list, and the drawer to Comments.
  await drawer.getByRole('button', { name: 'All comments', exact: true }).click();
  await expect(drawer.getByRole('heading', { level: 2 })).toHaveText('Comments');
  await expect(drawer.getByRole('tablist', { name: 'Comment views' })).toBeVisible();
  // Show in the page scrolls to the passage; the non-modal panel stays beside or over it.
  await drawer.getByRole('tab', { name: 'Resolved', exact: true }).click();
  await drawer.getByTestId('annotation-row').click();
  await drawer.getByRole('button', { name: /^Show in the page/ }).click();
  await expect(commentsRegion(page)).toBeVisible();
  await expect(drawer.getByRole('heading', { level: 2 })).toHaveText('Discussion');
});
