import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

// #4919: the page's two floating actions sit in one capsule, the dock; Chat opens out of the dock and
// closes back into it. The motion is the leaf drawer's own dialog, clipped to the dock's rectangle at one
// end (dock-motion.ts), so these tests read the animation and the dialog, never a fixed delay.
const fixture = '/test/reply-notifications-browser.ts';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const dock = (page: Page) => page.locator('.page-floating-actions');
const chat = (page: Page) => dock(page).getByRole('button', { name: /^Chat(?: · Unread)?$/ });
const drawer = (page: Page) => page.locator('.tmt-ui-panel[data-panel="chat"]');
/** Records each call that starts the dock's motion, as it is made: a motion of 180ms is over before a
 * test can look for it, so the test reads what was started, not what is still running. */
const record = () => {
  const started: unknown[] = [];
  (window as unknown as { chatDockMotions: unknown[] }).chatDockMotions = started;
  const original = Element.prototype.animate;
  Element.prototype.animate = function (this: Element, frames, options) {
    const timing = options as KeyframeAnimationOptions | undefined;
    if (timing?.id === 'dock-motion') {
      const dialog = this as HTMLElement;
      started.push({
        clip: (frames as { clipPath: string[] }).clipPath,
        duration: timing.duration,
        fill: timing.fill,
        open: !dialog.hidden,
        rendered: dialog.getBoundingClientRect().width > 0,
      });
    }
    return original.call(this, frames, options);
  };
};
interface Started {
  clip: [string, string];
  duration: number;
  fill: string;
  open: boolean;
  rendered: boolean;
}
const started = (page: Page) =>
  page.evaluate(() => (window as unknown as { chatDockMotions: Started[] }).chatDockMotions);
async function mount(page: Page) {
  await page.addInitScript(record);
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await expect(chat(page)).toBeVisible();
}
/** The dock's own motion has run to its end (the leaf's transitions on the dialog come and go with it). */
const settled = (page: Page) =>
  page.evaluate(async () => {
    const dialog = document.querySelector<HTMLElement>('.tmt-ui-panel[data-panel="chat"]')!;
    await Promise.all(
      dialog
        .getAnimations()
        .filter((animation) => animation.id === 'dock-motion')
        .map((animation) => animation.finished),
    );
  });

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`${width} ${theme}`, () => {
      test.use({ viewport: { width, height: 900 }, colorScheme: theme });

      test('one capsule holds Comments, Chat and Select with token spacing', async ({ page }) => {
        await mount(page);
        const group = await dock(page).evaluate((node) => {
          const style = getComputedStyle(node);
          const actions = [...node.children] as HTMLElement[];
          return {
            count: actions.length,
            radius: style.borderTopLeftRadius,
            radiusRole: style.getPropertyValue('--tmt-ui-radius-pill').trim(),
            gap: style.gap,
            padding: style.padding,
            space: style.getPropertyValue('--tmt-ui-space-xs').trim(),
            background: style.backgroundColor,
            line: actions.slice(1).map((action) => getComputedStyle(action).borderLeftColor),
            lineWidth: actions.slice(1).map((action) => getComputedStyle(action).borderLeftWidth),
            // The actions carry no frame of their own.
            frames: actions.map(
              (action) => getComputedStyle(action.querySelector('button')!).borderTopWidth,
            ),
          };
        });
        expect(group.count).toBe(3);
        // #5813 mock replaces the old divided capsule with token spacing and a pill surface.
        expect(group.radius).toBe(group.radiusRole);
        expect(group.gap).toBe(group.space);
        expect(group.padding).toBe(group.space);
        expect(group.lineWidth).toEqual(['0px', '0px']);
        expect(group.frames).toEqual(['0px', '0px', '0px']);
        await page.screenshot({
          path: capturePath(`dock-closed-${width}-${theme}.png`),
          clip: { x: Math.max(0, width - 360), y: 780, width: Math.min(360, width), height: 120 },
        });
      });

      test('Chat opens out of the dock and closes back into it, with focus and Escape as before', async ({
        page,
      }) => {
        // The suite runs with reduced motion; the motion is what this test is about.
        await page.emulateMedia({ reducedMotion: 'no-preference' });
        await mount(page);
        const box = (await dock(page).boundingBox())!;
        // Opening: the dialog is clipped to the dock's rectangle first and ends on its own.
        await chat(page).click();
        await expect(drawer(page)).toBeVisible();
        const [opening] = await started(page);
        expect(opening).toMatchObject({ open: true, rendered: true, fill: 'backwards' });
        expect(opening.duration).toBeGreaterThan(0);
        expect(opening.clip[0]).toMatch(/^inset\(.+ round /);
        expect(opening.clip[1]).toMatch(/^inset\(0px 0px 0px 0px round /);
        // The first frame shows the dock's rectangle: the clip's insets are the dock's own edges.
        const dialog = (await drawer(page).boundingBox())!;
        const [top, right, bottom, left] = opening.clip[0]
          .match(/inset\((.+?) round/)![1]
          .split(' ')
          .map(parseFloat);
        expect(dialog.x + left).toBeCloseTo(box.x, 0);
        expect(dialog.y + top).toBeCloseTo(box.y, 0);
        expect(dialog.x + dialog.width - right).toBeCloseTo(box.x + box.width, 0);
        expect(dialog.y + dialog.height - bottom).toBeCloseTo(box.y + box.height, 0);
        await settled(page);
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        // The dock stays visible and usable below the non-modal panel.
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        await expect(page.locator('.selection-mode')).not.toHaveCSS('pointer-events', 'none');
        expect(
          await page.evaluate(
            () => !!document.activeElement?.closest('.tmt-ui-panel[data-panel=chat]'),
          ),
        ).toBe(true);
        await page.screenshot({ path: capturePath(`dock-open-${width}-${theme}.png`) });
        // Closing: Escape runs the same clip back, on a dialog that is closed but still rendered.
        await page.keyboard.press('Escape');
        await expect.poll(async () => (await started(page)).length).toBe(2);
        const closing = (await started(page))[1];
        expect(closing).toMatchObject({ open: false, rendered: true, fill: 'forwards' });
        expect(closing.clip).toEqual([opening.clip[1], opening.clip[0]]);
        await settled(page);
        await expect(drawer(page)).toBeHidden();
        await expect(chat(page)).toBeFocused();
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        // Opening again starts a fresh motion and the dialog opens as before.
        await chat(page).click();
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        await expect.poll(async () => (await started(page)).length).toBe(3);
        await settled(page);
        await page.keyboard.press('Escape');
        await expect(drawer(page)).toBeHidden();
        await settled(page);
        await expect(chat(page)).toBeFocused();
      });

      test('under reduced motion Chat opens and closes with no animation', async ({ page }) => {
        await page.emulateMedia({ reducedMotion: 'reduce' });
        await mount(page);
        await chat(page).click();
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        expect(await started(page)).toEqual([]);
        await page.keyboard.press('Escape');
        expect(await started(page)).toEqual([]);
        await expect(drawer(page)).toBeHidden();
        await expect(chat(page)).toBeFocused();
      });
    });
  }

// #5651: every dock action shares one above-dock help position, independent of the hit target.
for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const name of ['Selection mode', 'Chat', 'Comments'] as const)
      test(`dock tooltip ${name} centered with token gap ${width} ${theme}`, async ({
        page,
      }, info) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
        await mount(page);
        const action = dock(page).getByRole('button', {
          name: new RegExp(`^${name}(?: \\(| ·|$)`),
        });
        const accessibleName = await action.getAttribute('aria-label');
        for (const input of ['hover', 'keyboard'] as const) {
          await page.mouse.move(0, 0);
          if (input === 'hover') await action.hover();
          else {
            await page.keyboard.press('Tab');
            await action.focus();
            await expect(action).toBeFocused();
          }
          const tooltip = dock(page).locator('.tmt-ui-icon-action-tooltip:popover-open');
          await expect(tooltip).toHaveCount(1);
          const label = tooltip.locator('.tmt-ui-icon-action-tooltip-label');
          await expect(label).toBeVisible();
          const [group, help] = await Promise.all([dock(page).boundingBox(), label.boundingBox()]);
          expect(help!.x + help!.width / 2).toBeCloseTo(group!.x + group!.width / 2, 0);
          const offset = await tooltip.evaluate((node) =>
            parseFloat(getComputedStyle(node).getPropertyValue('--tmt-ui-toggle-gap')),
          );
          expect(offset).toBeGreaterThan(0);
          expect(group!.y - help!.y - help!.height).toBeGreaterThanOrEqual(offset - 0.5);
          await expect(action).toHaveAccessibleName(accessibleName!);
          await page.screenshot({
            path: info.outputPath(
              `dock-${name.replaceAll(' ', '-')}-${input}-${width}-${theme}.png`,
            ),
          });
          await page.keyboard.press('Escape');
          await expect(tooltip).toHaveCount(0);
        }
      });
