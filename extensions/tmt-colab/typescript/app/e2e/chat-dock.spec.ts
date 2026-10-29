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
const drawer = (page: Page) => page.locator('#ask-page-fixture .page-conversation-window');
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
        panel: dialog.getAttribute('data-panel'),
        title: dialog.querySelector('h2')?.textContent,
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
    const dialog = document.querySelector<HTMLElement>(
      '#ask-page-fixture .page-conversation-window',
    )!;
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

      test('one capsule holds Comments, Chat and Select, divided by visible hairlines', async ({
        page,
      }) => {
        await mount(page);
        const group = await dock(page).evaluate((node) => {
          const style = getComputedStyle(node);
          const actions = [...node.children] as HTMLElement[];
          return {
            count: actions.length,
            radius: style.borderTopLeftRadius,
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
        expect(group.radius).toBe('22px');
        expect(group.lineWidth).toEqual(['1px', '1px']);
        expect(group.frames).toEqual(['0px', '0px', '0px']);
        // Each hairline is a real colour that is not the capsule's own surface, in both themes.
        for (const line of group.line) {
          expect(line).not.toBe('rgba(0, 0, 0, 0)');
          expect(line).not.toBe(group.background);
        }
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
        await expect(drawer(page)).toHaveJSProperty('hidden', false);
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
        await expect(drawer(page).locator(':scope > .tmt-ui-drawer-body')).toHaveCSS(
          'padding',
          '0px',
        );
        // The dock remains visible and usable below the non-modal host.
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        await expect(page.locator('.selection-mode')).toHaveCSS('pointer-events', 'auto');
        expect(
          await page.evaluate(() => !!document.activeElement?.closest('.page-conversation-window')),
        ).toBe(true);
        await page.screenshot({ path: capturePath(`dock-open-${width}-${theme}.png`) });
        // Closing: Escape runs the same clip back, on a dialog that is closed but still rendered.
        await page.keyboard.press('Escape');
        await expect.poll(async () => (await started(page)).length).toBe(2);
        const closing = (await started(page))[1];
        expect(closing).toMatchObject({
          open: false,
          rendered: true,
          fill: 'forwards',
          panel: 'chat',
          title: 'Chat',
        });
        expect(closing.clip).toEqual([opening.clip[1], opening.clip[0]]);
        await settled(page);
        await expect(drawer(page)).toHaveJSProperty('hidden', true);
        await expect(chat(page)).toBeFocused();
        await expect(page.locator('.selection-mode')).toHaveCSS('opacity', '1');
        // Opening again starts a fresh motion and the dialog opens as before.
        await chat(page).click();
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        await expect.poll(async () => (await started(page)).length).toBe(3);
        await settled(page);
        await page.keyboard.press('Escape');
        await expect(drawer(page)).toHaveJSProperty('hidden', true);
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
        await expect(drawer(page)).toHaveJSProperty('hidden', true);
        await expect(chat(page)).toBeFocused();
      });
    });
  }
