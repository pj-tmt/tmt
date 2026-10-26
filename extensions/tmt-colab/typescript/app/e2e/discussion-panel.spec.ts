import { expect, test, type Page } from '@playwright/test';
import { commentsRegion } from '../test/comments-region.js';
import { commentsAction, pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}

/** Five numbered threads; the fifth is resolved. A marker opens the first one's Discussion view. */
async function openFirst(page: Page, width = 1440) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { creator: true });
  await run(page, 'annotationLook');
  const frame = page.frameLocator('#ask-page-fixture iframe');
  await frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ }).click();
  const panel = commentsRegion(page);
  await expect(panel.getByRole('heading', { level: 2 })).toHaveText('Discussion');
  return { frame, panel };
}
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`Discussion chevrons retain their icon box at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: theme });
      const { panel } = await openFirst(page, width);
      for (const name of ['Previous thread', 'Next thread']) {
        const button = panel.getByRole('button', { name, exact: true });
        await expect(button).toBeVisible();
        await expect(button).toHaveCSS('width', '24px');
        await expect(button).toHaveCSS('height', '24px');
        const icon = await button.locator('svg').boundingBox();
        expect(icon).not.toBeNull();
        expect(icon!.width).toBe(14);
        expect(icon!.height).toBe(14);
      }
      await expect(
        panel.getByRole('button', { name: 'Previous thread', exact: true }),
      ).toBeDisabled();
      await panel.getByRole('button', { name: 'Next thread', exact: true }).click();
      await expect(
        panel.getByRole('button', { name: 'Previous thread', exact: true }),
      ).toBeEnabled();
      await page.screenshot({ path: test.info().outputPath(`discussion-${width}-${theme}.png`) });
    });

const highlights = (frame: ReturnType<Page['frameLocator']>) =>
  frame.locator('html').evaluate(() => ({
    quiet: CSS.highlights.has('colab-quiet'),
    selected: CSS.highlights.has('colab-selected'),
  }));

for (const width of [1440, 390])
  test(`the header arrows walk the threads in the order of the position, and the position is the way back at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    // Resolved threads are walked once Show resolved is on: the fifth thread is resolved.
    await panel.getByRole('button', { name: 'Show resolved', exact: true }).click();
    const position = panel.getByRole('button', { name: 'All comments', exact: true });
    const previous = panel.getByRole('button', { name: 'Previous thread', exact: true });
    const next = panel.getByRole('button', { name: 'Next thread', exact: true });
    const title = panel.locator('.discussion-quote q');
    const quotes = [
      'Check the mobile layout before release.',
      'Make sure the Remote door restarts cleanly after the upgrade on every supported platform, including the slower CI runners.',
      'first range',
      'second range',
      'old decision text',
    ];
    await expect(position).toHaveText('1 / 5');
    await expect(previous).toBeDisabled();
    await expect(title).toHaveText(quotes[0]!);
    // The title is the passage and its number; the position shows the same number.
    await expect(panel.locator('.discussion-quote .annotation-number')).toHaveText('1');
    for (const [index, quote] of quotes.entries()) {
      if (index > 0) await next.click();
      await expect(position).toHaveText(`${index + 1} / 5`);
      await expect(title).toHaveText(quote);
      await expect(panel.locator('.discussion-quote .annotation-number')).toHaveText(
        String(index + 1),
      );
    }
    await expect(next).toBeDisabled();
    await previous.click();
    await expect(position).toHaveText('4 / 5');
    await expect(title).toHaveText(quotes[3]!);
    // The position returns to the list: the drawer reads Comments and the arrows are gone.
    await position.click();
    await expect(panel.getByRole('heading', { level: 2 })).toHaveText('Comments');
    await expect(previous).toHaveCount(0);
    await expect(next).toHaveCount(0);
    await expect(panel.locator('.discussion-position')).toHaveCount(0);
  });

test('an open thread draws the selected look on its passage and a resolved one only a quiet dashed mark', async ({
  page,
}) => {
  const { frame, panel } = await openFirst(page);
  await panel.getByRole('button', { name: 'Show resolved', exact: true }).click();
  await expect.poll(() => highlights(frame)).toEqual({ quiet: false, selected: true });
  // Walking to the resolved thread: its passage keeps only the dashed mark, no selected look.
  const next = panel.getByRole('button', { name: 'Next thread', exact: true });
  for (let index = 0; index < 4; index++) await next.click();
  await expect(panel.getByRole('button', { name: 'All comments', exact: true })).toHaveText(
    '5 / 5',
  );
  await expect(panel.getByTestId('discussion-state')).toContainText('Resolved');
  await expect.poll(() => highlights(frame)).toEqual({ quiet: true, selected: false });
  // And back to an open thread: the dashed mark goes, the selected look returns.
  await panel.getByRole('button', { name: 'Previous thread', exact: true }).click();
  await expect.poll(() => highlights(frame)).toEqual({ quiet: false, selected: true });
});

test('the anchor title shows its passage in the page and an open thread has no state line or footer', async ({
  page,
}) => {
  const { panel } = await openFirst(page);
  await expect(panel.getByTestId('discussion-state')).toHaveCount(0);
  await expect(panel.locator('.discussion-footer')).toHaveCount(0);
  await expect(panel.locator('.discussion-title')).toHaveCount(0);
  // The title is one control: its name says what it does and which passage it is about.
  const anchor = panel.getByRole('button', { name: /^Show in the page: Check the mobile/ });
  await expect(anchor).toBeVisible();
  await expect(anchor).toHaveAttribute('title', 'Check the mobile layout before release.');
});

for (const width of [1440, 800, 390])
  test(`Resolve and Reopen are secondary text buttons that fit the header at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    const header = panel.locator('.tmt-ui-drawer-header');
    const resolve = panel.getByRole('button', { name: 'Resolve', exact: true });
    await expect(resolve).toBeVisible();
    await expect(resolve).toHaveText('Resolve');
    await expect(resolve).toHaveCSS('font-size', '12px');
    await expect(resolve).toHaveCSS('border-top-left-radius', '8px');
    // The title is 15px bold.
    const title = panel.getByRole('heading', { level: 2 });
    await expect(title).toHaveCSS('font-size', '15px');
    await expect(title).toHaveCSS('font-weight', '700');
    // Nothing is cut or covered: every control sits inside the header and none overlaps another.
    const row = async () =>
      header.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const items = [...node.querySelectorAll<HTMLElement>('h2, button')].map((item) => {
          const rect = item.getBoundingClientRect();
          return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
        });
        const overlap = (a: (typeof items)[number], b: (typeof items)[number]) =>
          a.left < b.right - 0.5 &&
          b.left < a.right - 0.5 &&
          a.top < b.bottom - 0.5 &&
          b.top < a.bottom - 0.5;
        return {
          fits: node.scrollWidth <= node.clientWidth + 1,
          inside: items.every((item) => item.left >= box.left - 1 && item.right <= box.right + 1),
          apart: items.every((item, index) =>
            items.slice(index + 1).every((other) => !overlap(item, other)),
          ),
        };
      });
    expect(await row()).toEqual({ fits: true, inside: true, apart: true });
    // In the 286px column row 1 is the title and Close; row 2 holds the position with both arrows as
    // one group on the left and Resolve on the right. Wider, they all share one row.
    const place = async () =>
      header.evaluate((node) => {
        const rect = (selector: string) => {
          const box = node.querySelector(selector)!.getBoundingClientRect();
          return { top: box.top, bottom: box.bottom, left: box.left, right: box.right };
        };
        return {
          header:
            node.getBoundingClientRect().right - parseFloat(getComputedStyle(node).paddingRight),
          title: rect('h2'),
          close: rect('button[aria-label^="Close"]'),
          position: rect('.discussion-position'),
          previous: rect('button[aria-label="Previous thread"]'),
          next: rect('button[aria-label="Next thread"]'),
          resolve: rect('.discussion-resolve'),
        };
      });
    const at = await place();
    const level = (a: { top: number; bottom: number }, b: { top: number; bottom: number }) =>
      Math.abs((a.top + a.bottom) / 2 - (b.top + b.bottom) / 2) < 4;
    expect(at.position.right).toBeLessThanOrEqual(at.previous.left + 1);
    expect(at.previous.right).toBeLessThanOrEqual(at.next.left + 1);
    expect(at.next.right).toBeLessThanOrEqual(at.resolve.left + 1);
    expect(level(at.position, at.previous) && level(at.next, at.resolve)).toBe(true);
    // The column and the card are two rows: the title and Close, then the position, the arrows and
    // Show resolved on the left and Resolve on the right. The phone sheet is one.
    if (width !== 390) {
      expect(level(at.title, at.close)).toBe(true);
      expect(at.title.bottom).toBeLessThanOrEqual(at.position.top + 1);
      expect(Math.abs(at.resolve.right - at.header)).toBeLessThanOrEqual(1);
    } else {
      expect(level(at.title, at.position) && level(at.position, at.close)).toBe(true);
    }
    expect(await row()).toEqual({ fits: true, inside: true, apart: true });
    // With Show resolved on the resolved thread stays in view, to be reopened; the toggle is one of the
    // header's controls and fits with the rest.
    await panel.getByRole('button', { name: 'Show resolved', exact: true }).click();
    await resolve.click();
    const reopen = panel.getByRole('button', { name: 'Reopen', exact: true });
    await expect(reopen).toHaveText('Reopen');
    expect(await row()).toEqual({ fits: true, inside: true, apart: true });
    // Its turns' avatars are 24px.
    await expect(panel.locator('.tmt-ui-avatar').first()).toHaveCSS('width', '24px');
  });

for (const width of [1440, 800, 390])
  test(`a wider font shortens the title and overlaps nothing in the header at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    await page.addStyleTag({
      content:
        '.tmt-ui-panel[data-panel="comments"] .tmt-ui-drawer-header * { letter-spacing: 1.5px !important; }',
    });
    const overlap = await panel.locator('.tmt-ui-drawer-header').evaluate((node) => {
      const box = node.getBoundingClientRect();
      const items = [...node.querySelectorAll<HTMLElement>('h2, button')].map((item) =>
        item.getBoundingClientRect(),
      );
      const crossed = items.some((a, index) =>
        items
          .slice(index + 1)
          .some(
            (b) =>
              a.left < b.right - 0.5 &&
              b.left < a.right - 0.5 &&
              a.top < b.bottom - 0.5 &&
              b.top < a.bottom - 0.5,
          ),
      );
      return {
        crossed,
        inside: items.every((rect) => rect.left >= box.left - 1 && rect.right <= box.right + 1),
      };
    });
    expect(overlap).toEqual({ crossed: false, inside: true });
  });

for (const width of [1440, 390])
  test(`the panel has one edge, one separator under its header and a composer on its own surface at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    const mode = width === 1440 ? 'column' : 'sheet';
    await expect(panel).toHaveAttribute('data-mode', mode);
    // The edge between the page and a column is the page's own: the column draws no rule, so no
    // second vertical line stands inside the panel. A sheet draws the leaf's edge once, all round.
    const edge = width === 1440 ? '0px' : '1px';
    for (const side of ['top', 'right', 'bottom', 'left'])
      await expect(panel).toHaveCSS(`border-${side}-width`, edge);
    // One horizontal separator between the header and the thread: the header's own. The last turn
    // ends the thread and the composer starts without a rule above it.
    const composer = panel.locator('.conversation-composer');
    await expect(panel.locator('.tmt-ui-drawer-header')).toHaveCSS('border-bottom-width', '1px');
    await expect(panel.locator('.conversation-turn').last()).toHaveCSS(
      'border-bottom-width',
      '0px',
    );
    await expect(composer).toHaveCSS('border-top-width', '0px');
    // The composer is on its panel's surface: the page's paper behind a column, the sheet in a
    // sheet; never a band of another colour.
    const surface = await page.evaluate(
      ({ mode }) =>
        getComputedStyle(
          mode === 'column'
            ? document.body
            : document.querySelector('.tmt-ui-panel[data-panel="comments"]')!,
        ).backgroundColor,
      { mode },
    );
    await expect(composer).toHaveCSS('background-color', surface);
    // The field alone keeps its border: some box around the editor inside the composer draws it.
    const bordered = await composer.locator('.rich-input-field').evaluate((node) => {
      for (let box = node.parentElement; box; box = box.parentElement) {
        if (getComputedStyle(box).borderTopWidth === '1px') return true;
        if (box.classList.contains('conversation-composer')) return false;
      }
      return false;
    });
    expect(bordered).toBe(true);
  });

for (const width of [1440, 390])
  test(`resolved threads are left out of the position and the arrows until Show resolved is on at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    const position = panel.getByRole('button', { name: 'All comments', exact: true });
    const next = panel.getByRole('button', { name: 'Next thread', exact: true });
    const toggle = panel.getByRole('button', { name: 'Show resolved', exact: true });
    // Four open threads and one resolved: the position counts the four.
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(position).toHaveText('1 / 4');
    for (let index = 0; index < 3; index++) await next.click();
    await expect(position).toHaveText('4 / 4');
    await expect(panel.locator('.discussion-quote q')).toHaveText('second range');
    await expect(next).toBeDisabled();
    // Turning it on adds the resolved thread, in its place by number; turning it off takes it out.
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await expect(position).toHaveText('4 / 5');
    await expect(next).toBeEnabled();
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await expect(position).toHaveText('4 / 4');
    // The dock's count is the open threads, whichever way the toggle is.
    await expect(commentsAction(page)).toHaveAccessibleName('Comments (4)');
    await toggle.click();
    await expect(commentsAction(page)).toHaveAccessibleName('Comments (4)');
  });

test('the list shows open threads by default, resolved ones with Show resolved, and the choice stays on this device', async ({
  page,
}) => {
  const { panel } = await openFirst(page);
  const toggle = panel.getByRole('button', { name: 'Show resolved', exact: true });
  await panel.getByRole('button', { name: 'All comments', exact: true }).click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect(panel.getByTestId('annotation-row')).toHaveCount(4);
  await expect(panel.locator('.annotation-list-item[data-resolved]')).toHaveCount(0);
  await toggle.click();
  await expect(panel.getByTestId('annotation-row')).toHaveCount(5);
  await expect(panel.locator('.annotation-list-item[data-resolved]')).toHaveCount(1);
  // Kept on this device: a reload opens with it on, and storage that refuses leaves it off.
  expect(await page.evaluate(() => localStorage.getItem('colab.comments.showResolved'))).toBe('1');
  await toggle.click();
  expect(await page.evaluate(() => localStorage.getItem('colab.comments.showResolved'))).toBeNull();
  await toggle.click();
  await page.reload();
  await page.evaluate(async () => {
    const path = '/test/ask-page-browser.tsx';
    const fixture = await import(path);
    await fixture.mount({ creator: true });
    await fixture.annotationLook();
  });
  await (await pageAction(page, 'Comments')).click();
  await expect(
    commentsRegion(page).getByRole('button', { name: 'Show resolved', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
});

test('with every thread resolved and Show resolved off the panel says there is no open thread', async ({
  page,
}) => {
  const { panel } = await openFirst(page);
  const toggle = panel.getByRole('button', { name: 'Show resolved', exact: true });
  await panel.getByRole('button', { name: 'All comments', exact: true }).click();
  // Resolve the four open threads from their Discussion views; each moves on to the next open one.
  const resolve = panel.getByRole('button', { name: 'Resolve', exact: true });
  await panel.getByTestId('annotation-row').first().click();
  for (let left = 4; left > 1; left--) {
    await resolve.click();
    await expect(commentsAction(page)).toHaveAccessibleName(`Comments (${left - 1})`);
    await expect(panel.getByRole('button', { name: 'All comments', exact: true })).toHaveText(
      `1 / ${left - 1}`,
    );
  }
  // The last one: resolving it leaves the Discussion view for the list's empty state.
  await resolve.click();
  await expect(panel.getByTestId('comments-empty')).toContainText('No open threads');
  await expect(panel.getByTestId('annotation-row')).toHaveCount(0);
  await expect(toggle).toBeVisible();
  await expect(commentsAction(page)).toHaveAccessibleName('Comments (0)');
  await toggle.click();
  await expect(panel.getByTestId('annotation-row')).toHaveCount(5);
});
