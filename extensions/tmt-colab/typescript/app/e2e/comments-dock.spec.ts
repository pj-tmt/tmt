import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { commentsAction } from '../test/page-actions.js';
import { commentsClose, commentsRegion } from '../test/comments-region.js';

// #5519: Comments follows Annotate and Chat with a plain open-thread count. The dock stays
// visible when Discussion is open, with the open action highlighted.
const fixture = '/test/reply-notifications-browser.ts';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const dock = (page: Page) => page.locator('.page-floating-actions');
/** Records each call that starts the dock's motion, as it is made (see chat-dock.spec.ts). */
const record = () => {
  const started: unknown[] = [];
  (window as unknown as { dockMotions: unknown[] }).dockMotions = started;
  const original = Element.prototype.animate;
  Element.prototype.animate = function (this: Element, frames, options) {
    const timing = options as KeyframeAnimationOptions | undefined;
    if (timing?.id === 'dock-motion') {
      started.push({
        panel: this.getAttribute('data-panel'),
        clip: (frames as { clipPath: string[] }).clipPath,
        fill: timing.fill,
        hidden: (this as HTMLElement).hidden,
        rendered: this.getBoundingClientRect().width > 0,
        rect: this.getBoundingClientRect().toJSON(),
        dock: document.querySelector('.page-floating-actions')!.getBoundingClientRect().toJSON(),
      });
    }
    return original.call(this, frames, options);
  };
};
interface Started {
  panel: string | null;
  clip: [string, string];
  fill: string;
  hidden: boolean;
  rendered: boolean;
  rect: { x: number; y: number; width: number; height: number };
  dock: { x: number; y: number; width: number; height: number };
}
const started = (page: Page) =>
  page.evaluate(() => (window as unknown as { dockMotions: Started[] }).dockMotions);
async function mount(page: Page) {
  await page.addInitScript(record);
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await expect(commentsAction(page)).toBeVisible();
}
/** The dock's own motion has run to its end. */
const settled = (page: Page) =>
  page.evaluate(async () => {
    const panel = document.querySelector<HTMLElement>('.tmt-ui-panel[data-panel="comments"]')!;
    await Promise.all(
      panel
        .getAnimations()
        .filter((animation) => animation.id === 'dock-motion')
        .map((animation) => animation.finished),
    );
  });

const modes = [
  { mode: 'card', width: 1440 },
  { mode: 'card', width: 1000 },
  { mode: 'sheet', width: 390 },
] as const;
for (const { mode, width } of modes)
  for (const theme of ['light', 'dark'] as const)
    test.describe(`${mode} ${width} ${theme}`, () => {
      test.use({ viewport: { width, height: 900 }, colorScheme: theme });

      test('Comments opens out of the dock and closes with focus back on its action', async ({
        page,
      }) => {
        // The suite runs with reduced motion; the motion is what this test is about.
        await page.emulateMedia({ reducedMotion: 'no-preference' });
        await mount(page);
        const action = commentsAction(page);
        const box = (await dock(page).boundingBox())!;
        await expect(action).toHaveAttribute('aria-expanded', 'false');
        await action.click();
        const panel = commentsRegion(page);
        await expect(panel).toBeVisible();
        await expect(panel).toHaveAttribute('data-mode', mode);
        await expect(action).toHaveAttribute('aria-expanded', 'true');
        const [opening] = await started(page);
        expect(opening).toMatchObject({ panel: 'comments', fill: 'backwards', rendered: true });
        expect(opening.clip[0]).toMatch(/^inset\(.+ round /);
        expect(opening.clip[1]).toMatch(/^inset\(0px 0px 0px 0px round /);
        // The first frame shows the dock's rectangle: the clip's insets are the dock's own edges.
        const rect = (await panel.boundingBox())!;
        const [top, right, bottom, left] = opening.clip[0]
          .match(/inset\((.+?) round/)![1]
          .split(' ')
          .map(parseFloat);
        expect(rect.x + left).toBeCloseTo(box.x, 0);
        expect(rect.y + top).toBeCloseTo(box.y, 0);
        expect(rect.x + rect.width - right).toBeCloseTo(box.x + box.width, 0);
        expect(rect.y + rect.height - bottom).toBeCloseTo(box.y + box.height, 0);
        await settled(page);
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        await expect(page.locator('.selection-mode')).toHaveCSS('pointer-events', 'auto');
        expect(await page.evaluate(() => !!document.activeElement?.closest('.tmt-ui-panel'))).toBe(
          true,
        );
        await page.screenshot({
          path: capturePath(`comments-dock-open-${mode}-${width}-${theme}.png`),
        });
        // Escape closes the panel; the retained surface clips back to the dock.
        await page.keyboard.press('Escape');
        await expect(panel).toHaveCount(0);
        await expect(action).toBeFocused();
        await expect(action).toHaveAttribute('aria-expanded', 'false');
        await expect.poll(async () => (await started(page)).length).toBe(2);
        const closing = (await started(page))[1];
        expect(closing).toMatchObject({ panel: 'comments', fill: 'forwards', hidden: true });
        expect(closing.rendered).toBe(true);
        expect(closing.clip[0]).toEqual(opening.clip[1]);
        const [closeTop, closeRight, closeBottom, closeLeft] = closing.clip[1]
          .match(/inset\((.+?) round/)![1]
          .split(' ')
          .map(parseFloat);
        expect(closing.rect.x + closeLeft).toBeCloseTo(closing.dock.x, 0);
        expect(closing.rect.y + closeTop).toBeCloseTo(closing.dock.y, 0);
        expect(closing.rect.x + closing.rect.width - closeRight).toBeCloseTo(
          closing.dock.x + closing.dock.width,
          0,
        );
        expect(closing.rect.y + closing.rect.height - closeBottom).toBeCloseTo(
          closing.dock.y + closing.dock.height,
          0,
        );
        await settled(page);
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        // The close button does the same, and the dock's action opens it again.
        await action.click();
        await expect(commentsRegion(page)).toBeVisible();
        await commentsClose(page, 'discussion').click();
        await expect(commentsRegion(page)).toHaveCount(0);
        await expect(action).toBeFocused();
      });

      test('under reduced motion Comments opens and closes with no animation', async ({ page }) => {
        await page.emulateMedia({ reducedMotion: 'reduce' });
        await mount(page);
        await commentsAction(page).click();
        await expect(commentsRegion(page)).toBeVisible();
        expect(await started(page)).toEqual([]);
        await page.keyboard.press('Escape');
        await expect(commentsRegion(page)).toHaveCount(0);
        expect(await started(page)).toEqual([]);
        await expect(commentsAction(page)).toBeFocused();
      });
    });

test('a panel that is already open when the page mounts does not play the dock motion', async ({
  page,
}) => {
  await page.goto('/');
  const call = (method: string, argument?: unknown) =>
    page.evaluate(
      async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
      { fixture: '/test/dock-motion-browser.tsx', method, argument },
    );
  await call('mount', 'chat');
  expect(await call('recorded')).toEqual([]);
  // Closing and opening it again are the dock's motion, so the hook still plays.
  await call('show', null);
  await call('show', 'chat');
  expect(await call('recorded')).toEqual(['chat', 'chat']);
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`the count marks unseen thread news at ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await mount(page);
      const action = commentsAction(page);
      const count = page.locator('.dock-count').first();
      await run(page, 'secondThread');
      await expect(action).toHaveAccessibleName('Comments (2)');
      await expect(count).toHaveText('2');
      await expect(count).not.toHaveAttribute('data-unseen', /.*/);
      const quiet = await count.evaluate((node) => getComputedStyle(node).color);
      await run(page, 'agentResolves');
      await expect(action).toHaveAccessibleName('Comments (1) · New');
      await expect(count).toHaveText('1•');
      await expect(count).toHaveAttribute('data-unseen', 'true');
      await expect(count).toHaveCSS('color', quiet);
      await page.screenshot({ path: capturePath(`comments-dock-unseen-${width}-${theme}.png`) });
    });

for (const width of [320, 390, 1440])
  test(`the three-action capsule fits at ${width}px without wrapping`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 });
    await mount(page);
    const actions = dock(page).getByRole('button');
    await expect(actions).toHaveCount(3);
    const boxes = await actions.evaluateAll((nodes) =>
      nodes.map((node) => node.getBoundingClientRect().toJSON()),
    );
    // #5813: equal dock actions use the approved floating-action-size role.
    const actionSize = await dock(page).evaluate((node) =>
      parseFloat(getComputedStyle(node).getPropertyValue('--tmt-ui-floating-action-size')),
    );
    // One row: the same token-sized actions, left to right: Annotate, Chat, Comments.
    for (const box of boxes) {
      expect(box.top).toBe(boxes[0].top);
      expect(box.height).toBe(actionSize);
    }
    expect(boxes[0].right).toBeLessThanOrEqual(boxes[1].left + 1);
    expect(boxes[1].right).toBeLessThanOrEqual(boxes[2].left + 1);
    const capsule = (await dock(page).boundingBox())!;
    expect(capsule.x).toBeGreaterThanOrEqual(0);
    expect(capsule.x + capsule.width).toBeLessThanOrEqual(width);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(width);
    await expect(actions.nth(0)).toHaveAccessibleName('Selection mode');
    await expect(actions.nth(1)).toHaveAccessibleName(/^Chat/);
    await expect(actions.nth(2)).toHaveAccessibleName(/^Comments/);
    await page.screenshot({
      path: capturePath(`comments-dock-closed-${width}.png`),
      clip: { x: Math.max(0, width - 360), y: 560, width: Math.min(360, width), height: 140 },
    });
  });

test('dock keeps the mock-sized round icons and highlights its open Discussion action', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mount(page);
  const actions = dock(page).getByRole('button');
  await expect(actions).toHaveCount(3);
  const roles = await dock(page).evaluate((node) => {
    const css = getComputedStyle(node);
    const probe = document.createElement('span');
    probe.style.background = 'var(--tmt-ui-surface-selection)';
    node.append(probe);
    const selection = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return {
      icon: css.getPropertyValue('--tmt-ui-header-icon-size').trim(),
      stroke: `${parseFloat(css.getPropertyValue('--tmt-ui-header-icon-stroke'))}px`,
      selection,
    };
  });
  for (const icon of await dock(page).locator('button svg').all()) {
    await expect(icon).toHaveCSS('width', roles.icon);
    await expect(icon).toHaveCSS('height', roles.icon);
    await expect(icon).toHaveCSS('stroke-width', roles.stroke);
    await expect(icon).toHaveCSS('stroke-linecap', 'round');
    await expect(icon).toHaveCSS('stroke-linejoin', 'round');
  }
  const comments = commentsAction(page);
  await comments.click();
  await expect(dock(page)).toBeVisible();
  await expect(comments).toHaveAttribute('aria-expanded', 'true');
  await expect(comments).toHaveCSS('background-color', roles.selection);
});
