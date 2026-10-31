import { expectAvailableDockWindow } from '../test/floating-panel.js';
import { expect, test, type Page } from '@playwright/test';
import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { commentsAction, pageAction, chooseThreadFilter } from '../test/page-actions.js';

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
      for (const name of ['Previous', 'Next']) {
        const button = panel.getByRole('button', { name, exact: true });
        await expect(button).toBeVisible();
        await expect(button).toHaveCSS('width', '26px');
        await expect(button).toHaveCSS('height', '26px');
        const icon = await button.locator('svg').boundingBox();
        expect(icon).not.toBeNull();
        expect(icon!.width).toBe(16);
        expect(icon!.height).toBe(16);
      }
      await expect(panel.getByRole('button', { name: 'Previous', exact: true })).toBeDisabled();
      await panel.getByRole('button', { name: 'Next', exact: true }).click();
      await expect(panel.getByRole('button', { name: 'Previous', exact: true })).toBeEnabled();
      await page.screenshot({ path: test.info().outputPath(`discussion-${width}-${theme}.png`) });
    });

test('Discussion never offers page-level comment creation', async ({ page }) => {
  const { panel } = await openFirst(page);
  await expect(panel.getByRole('button', { name: '+ Comment on page' })).toHaveCount(0);
  await chooseThreadFilter(page, 'Needs you');
  await expect(panel.getByRole('button', { name: '+ Comment on page' })).toHaveCount(0);
});

test('Discussion uses twelve-pixel phone gutters above the dock', async ({ page }) => {
  const { panel } = await openFirst(page, 390);
  const frame = (await panel.boundingBox())!;
  const dock = (await page.locator('#ask-page-fixture .page-floating-actions').boundingBox())!;
  expect(frame.x).toBeCloseTo(12, 0);
  expect(frame.x + frame.width).toBeCloseTo(378, 0);
  const header = (await page.locator('#ask-page-fixture .tmt-ui-topbar').boundingBox())!;
  expect(frame.y).toBeCloseTo(header.y + header.height + 12, 0);
  expect(frame.y + frame.height).toBeLessThanOrEqual(dock.y);
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 1280, height: 720 },
  { width: 390, height: 900 },
])
  test(`Discussion fills the available height at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, viewport.width);
    await page.setViewportSize(viewport);
    const topbar = page.locator('#ask-page-fixture .tmt-ui-topbar');
    const bar = (await topbar.boundingBox())!;
    expect((await panel.boundingBox())!.y - bar.y - bar.height).toBeCloseTo(12, 0);
    const frame = (await panel.boundingBox())!;
    const dock = (await page.locator('#ask-page-fixture .page-floating-actions').boundingBox())!;
    expect(frame.y + frame.height).toBeCloseTo(dock.y - 16, 0);
    const composer = panel.locator('.conversation-composer');
    const before = (await composer.boundingBox())!;
    await panel.locator('.conversation-history').evaluate((node) => {
      node.scrollTop = 0;
    });
    expect((await composer.boundingBox())!.y).toBe(before.y);
  });

test('Chat shares the non-modal Discussion frame and keeps the page interactive', async ({
  page,
}) => {
  const { frame, panel } = await openFirst(page, 1440);
  const discussion = (await panel.boundingBox())!;
  await page
    .locator('#ask-page-fixture .page-floating-actions')
    .getByRole('button', { name: 'Chat', exact: true })
    .click();
  const chat = page.locator('#ask-page-fixture .tmt-ui-panel[data-panel="chat"]:visible');
  await expect(chat).toBeVisible();
  const chatBox = (await chat.boundingBox())!;
  expect(chatBox).toEqual(discussion);
  await expect(
    page.locator('#ask-page-fixture .tmt-ui-drawer[data-panel="chat"][open]'),
  ).toHaveCount(0);
  await frame.getByText('Keep the preview focused.', { exact: true }).click();
  await expect(chat).toBeVisible();
  await frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ }).click();
  await expect(chat).toBeHidden();
  await expect(commentsRegion(page).getByRole('heading', { level: 2 })).toHaveText('Discussion');
});

const highlights = (frame: ReturnType<Page['frameLocator']>) =>
  frame.locator('html').evaluate(() => ({
    quiet: CSS.highlights.has('colab-quiet'),
    selected: CSS.highlights.has('colab-selected'),
  }));

for (const width of [1440, 390])
  test(`the header arrows walk numbered threads and the position stays static at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    // All includes the resolved fifth thread.
    await chooseThreadFilter(page, 'All');
    const position = panel.locator('.discussion-position');
    const previous = panel.getByRole('button', { name: 'Previous', exact: true });
    const next = panel.getByRole('button', { name: 'Next', exact: true });
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
    await expect(position).toHaveJSProperty('tagName', 'SPAN');
    await expect(panel.getByTestId('annotation-row')).toHaveCount(0);
  });

test('an open thread draws the selected look on its passage and a resolved one only a quiet dashed mark', async ({
  page,
}) => {
  const { frame, panel } = await openFirst(page);
  await chooseThreadFilter(page, 'All');
  await expect.poll(() => highlights(frame)).toEqual({ quiet: false, selected: true });
  // Walking to the resolved thread: its passage keeps only the dashed mark, no selected look.
  const next = panel.getByRole('button', { name: 'Next', exact: true });
  for (let index = 0; index < 4; index++) await next.click();
  await expect(panel.locator('.discussion-position')).toHaveText('5 / 5');
  await expect(panel.getByTestId('discussion-state')).toContainText('Resolved');
  await expect.poll(() => highlights(frame)).toEqual({ quiet: true, selected: false });
  // And back to an open thread: the dashed mark goes, the selected look returns.
  await panel.getByRole('button', { name: 'Previous', exact: true }).click();
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
  test(`Resolve and Reopen are icon actions that fit the header at ${width}px`, async ({
    page,
  }) => {
    const { panel } = await openFirst(page, width);
    const header = panel.locator('.tmt-ui-drawer-header');
    const resolve = panel.getByRole('button', { name: 'Resolve', exact: true });
    await expect(resolve).toBeVisible();
    await expect(resolve).toHaveText('');
    await expect(resolve).toHaveClass(/tmt-ui-icon-action-control/);
    // The title follows the compact mock typography.
    const title = panel.getByRole('heading', { level: 2 });
    await expect(title).toHaveCSS('font-size', '13px');
    await expect(title).toHaveCSS('font-weight', '620');
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
    await chooseThreadFilter(page, 'All');
    await resolve.click();
    const reopen = panel.getByRole('button', { name: 'Reopen', exact: true });
    await expect(reopen).toHaveText('');
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
    const mode = width === 1440 ? 'card' : 'sheet';
    await expect(panel).toHaveAttribute('data-mode', mode);
    // Both the floating card and the sheet draw a single edge on all sides.
    const edge = '1px';
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
    // The composer is on the panel's sheet surface, never a band of another colour.
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
  test(`Open excludes resolved threads and All walks them at ${width}px`, async ({ page }) => {
    const { panel } = await openFirst(page, width);
    const position = panel.locator('.discussion-position');
    await expect(position).toHaveText('1 / 4');
    await chooseThreadFilter(page, 'All');
    await expect(position).toHaveText('1 / 5');
    for (let index = 0; index < 4; index++)
      await panel.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(panel.locator('.discussion-quote q')).toHaveText('old decision text');
    await expect(panel.getByTestId('discussion-state')).toContainText('Resolved');
    await chooseThreadFilter(page, 'Open');
    await expect(position).toHaveText('1 / 4');
    await expect(commentsAction(page)).toHaveAccessibleName('Comments (4)');
  });

test('the All preference survives remount and does not depend on successful browser storage', async ({
  page,
}) => {
  await openFirst(page);
  await chooseThreadFilter(page, 'All');
  expect(await page.evaluate(() => localStorage.getItem('colab.comments.filter'))).toBe('all');
  await page.reload();
  await page.evaluate(async () => {
    const path = '/test/ask-page-browser.tsx';
    const fixture = await import(path);
    await fixture.mount({ creator: true });
    await fixture.annotationLook();
  });
  await (await pageAction(page, 'Comments')).click();
  await expect(commentsRegion(page).locator('.discussion-position')).toHaveText('1 / 5');
  await page.evaluate(() => {
    Storage.prototype.setItem = () => {
      throw new Error('quota');
    };
  });
  await chooseThreadFilter(page, 'Open');
  await expect(commentsRegion(page).locator('.discussion-position')).toHaveText('1 / 4');
});

test('resolving the final thread stays expanded until explicit filter navigation', async ({
  page,
}) => {
  const { panel } = await openFirst(page);
  const resolve = panel.getByRole('button', { name: 'Resolve', exact: true });
  for (let left = 4; left > 0; left--) {
    await resolve.click();
    await expect(panel.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
    await expect(commentsAction(page)).toHaveAccessibleName(`Comments (${left - 1})`);
    await expect(panel.getByTestId('comment-entry').first()).toBeVisible();
    if (left > 1) await panel.getByRole('button', { name: 'Next', exact: true }).click();
  }
  await expect(panel.getByTestId('comments-empty')).toHaveCount(0);
  await chooseThreadFilter(page, 'Open');
  await expect(panel.getByTestId('comments-empty')).toContainText('Nothing waiting here.');
  await chooseThreadFilter(page, 'All');
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 5');
});

test('closing Discussion clears its selected passage by button and Escape from the page', async ({
  page,
}) => {
  const { frame } = await openFirst(page);
  const selected = () => highlights(frame);
  await expect.poll(selected).toMatchObject({ selected: true });
  await commentsClose(page, 'discussion').click();
  await expect(commentsRegion(page)).toHaveCount(0);
  await expect.poll(selected).toMatchObject({ selected: false });

  await frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ }).click();
  await expect.poll(selected).toMatchObject({ selected: true });
  // Escape must work with focus back in the page, not only inside the panel.
  await page.evaluate(() => {
    document.body.tabIndex = -1;
    document.body.focus();
  });
  await page.keyboard.press('Escape');
  await expect(commentsRegion(page)).toHaveCount(0);
  await expect.poll(selected).toMatchObject({ selected: false });
});

test('Discussion header follows Mock 1 order and 26px icon geometry', async ({ page }) => {
  const { panel } = await openFirst(page, 1440);
  await expect(panel).toHaveAccessibleName('Discussion');
  const header = panel.locator('.tmt-ui-drawer-header');
  const title = header.getByRole('heading', { name: 'Discussion' });
  const position = header.locator('.discussion-position');
  const filter = header.getByRole('button', { name: 'Thread filter' });
  const previous = header.getByRole('button', { name: 'Previous', exact: true });
  const next = header.getByRole('button', { name: 'Next', exact: true });
  const resolve = header.getByRole('button', { name: 'Resolve', exact: true });
  const all = header.getByRole('button', { name: /Resolve all open/ });
  const close = header.getByRole('button', { name: 'Close (Esc)', exact: true });
  const items = [title, position, filter, previous, next, resolve, all, close];
  const boxes = await Promise.all(items.map((item) => item.boundingBox()));
  for (const box of boxes) expect(box).not.toBeNull();
  for (let i = 1; i < boxes.length; i++) {
    expect(boxes[i]!.x).toBeGreaterThanOrEqual(boxes[i - 1]!.x + boxes[i - 1]!.width - 1);
    expect(
      Math.abs(boxes[i]!.y + boxes[i]!.height / 2 - (boxes[0]!.y + boxes[0]!.height / 2)),
    ).toBeLessThanOrEqual(1);
  }
  await expect(title).toHaveCSS('font-size', '13px');
  await expect(title).toHaveCSS('font-weight', '620');
  await expect(position).toHaveCSS('font-size', '11px');
  await expect(filter).toHaveCSS('border-top-width', '1px');
  for (const action of [previous, next, resolve, all, close]) {
    await expect(action).toHaveCSS('width', '26px');
    await expect(action).toHaveCSS('height', '26px');
    await expect(action.locator('svg')).toHaveCSS('width', '16px');
    await expect(action.locator('svg')).toHaveCSS('stroke-linecap', 'round');
  }
  await expect(header.locator('.discussion-separator')).toHaveCount(2);
  await page.screenshot({ path: test.info().outputPath('discussion-mock1-1440-light.png') });
});

test('wide Discussion floats 16px above the dock without reflowing the page', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { creator: true });
  await run(page, 'annotationLook');
  const frame = page.frameLocator('#ask-page-fixture iframe');
  const pageFrame = page.locator('#ask-page-fixture iframe');
  const before = (await pageFrame.boundingBox())!;
  await frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ }).click();
  const panel = commentsRegion(page);
  const dock = page.locator('#ask-page-fixture .page-floating-actions');
  await expect(panel).toBeVisible();
  await expect(dock).toBeVisible();
  const windowBox = (await panel.boundingBox())!;
  const dockBox = (await dock.boundingBox())!;
  const after = (await pageFrame.boundingBox())!;
  expect(windowBox.width).toBe(360);
  await expectAvailableDockWindow(page, panel);
  expect(Math.abs(windowBox.x + windowBox.width - dockBox.x - dockBox.width)).toBeLessThanOrEqual(
    1,
  );
  expect(Math.abs(dockBox.y - (windowBox.y + windowBox.height) - 16)).toBeLessThanOrEqual(1);
  expect(after.width).toBe(before.width);
  expect(after.x).toBe(before.x);
});

test('Escape returns focus to the marker that opened Discussion', async ({ page }) => {
  const { frame } = await openFirst(page);
  const marker = frame.locator('[data-colab-thread]').filter({ hasText: /^1$/ });
  await page.keyboard.press('Escape');
  await expect(commentsRegion(page)).toHaveCount(0);
  await expect(marker).toBeFocused();
  await marker.click();
  await expect(commentsRegion(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(commentsRegion(page)).toHaveCount(0);
  await expect(marker).toBeFocused();
});

test('Escape returns focus to the grouped marker that opened Discussion', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { creator: true });
  await run(page, 'annotationLook');
  const group = page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-group]');
  await group.click();
  await expect(commentsRegion(page)).toBeVisible();
  await expect(commentsRegion(page).getByTestId('comment-thread')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(commentsRegion(page)).toHaveCount(0);
  await expect(group).toBeFocused();
});

for (const theme of ['light', 'dark'] as const) {
  test(`Resolve-all tooltip is below the Discussion header in ${theme}`, async ({ page }, info) => {
    await page.emulateMedia({ colorScheme: theme });
    const { panel } = await openFirst(page, 1440);
    const action = panel.getByRole('button', { name: 'Resolve all open (4)', exact: true });
    await action.hover();
    const tooltip = panel.locator('.tmt-ui-icon-action-tooltip:popover-open');
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toHaveText('Resolve all open (4)');
    await expect(tooltip.locator('.tmt-ui-icon-action-tooltip-label')).toHaveCSS(
      'background-color',
      'rgb(38, 57, 63)',
    );
    const buttonBox = (await action.boundingBox())!;
    const tooltipBox = (await tooltip.boundingBox())!;
    expect(tooltipBox.y).toBeGreaterThanOrEqual(buttonBox.y + buttonBox.height);
    await page.screenshot({ path: info.outputPath(`mock1-tooltip-1440-${theme}.png`) });
  });
}
