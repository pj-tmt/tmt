import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { commentsAction } from '../test/page-actions.js';
import { commentsClose, commentsRegion } from '../test/comments-region.js';

// #5192/#5519: Comments follows Annotate and Chat in the dock, with the open-thread count and
// the needs-you colour and bullet. It opens the Comments panel out of the dock's rectangle as Chat does
// (dock-motion.ts): the host is clipped to the dock rectangle while the dock stays visible.
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
  { mode: 'card', width: 390 },
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
        // The dock remains visible and reachable below the floating host.
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        await expect(page.locator('.selection-mode')).toHaveCSS('pointer-events', 'auto');
        expect(await page.evaluate(() => !!document.activeElement?.closest('.tmt-ui-panel'))).toBe(
          true,
        );
        await page.screenshot({ path: capturePath(`comments-dock-open-${width}-${theme}.png`) });
        // Escape from inside the panel closes it. A card and a sheet run the clip back on a panel that
        // is hidden but still rendered; a column leaves the grid at once and has no closing motion.
        await page.keyboard.press('Escape');
        await expect(panel).toHaveCount(0);
        await expect(action).toBeFocused();
        await expect(action).toHaveAttribute('aria-expanded', 'false');
        {
          await expect.poll(async () => (await started(page)).length).toBe(2);
          const closing = (await started(page))[1];
          expect(closing).toMatchObject({ panel: 'comments', fill: 'forwards', hidden: true });
          expect(closing.rendered).toBe(true);
          expect(closing.clip).toEqual([opening.clip[1], opening.clip[0]]);
          await settled(page);
        }
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

test('the count is the open threads and a thread with news turns it to the needs-you colour', async ({
  page,
}) => {
  await mount(page);
  const action = commentsAction(page);
  const count = page.locator('.dock-count').first();
  await run(page, 'secondThread');
  await expect(action).toHaveAccessibleName('Comments (2)');
  await expect(count).toHaveText('2');
  await expect(count).not.toHaveAttribute('data-unseen', /.*/);
  const quiet = await count.evaluate((node) => getComputedStyle(node).color);
  await run(page, 'agentResolves');
  // One thread is open now and the resolution is news the viewer has not seen.
  await expect(action).toHaveAccessibleName('Comments (1) · New');
  await expect(count).toContainText('1');
  await expect(count).toContainText('•');
  await expect(count).toHaveAttribute('data-unseen', 'true');
  const waiting = await page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.color = 'var(--c-waiting)';
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
  await expect(count).toHaveCSS('color', waiting);
  expect(waiting).not.toBe(quiet);
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
    // One row: the same top and height, left to right in the order Annotate, Chat, Comments.
    for (const box of boxes) {
      expect(box.top).toBe(boxes[0].top);
      expect(box.height).toBe(44);
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
