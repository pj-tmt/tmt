import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

/**
 * A page that opens slowly shows a calm frame, never the old screen, and a fast open shows no frame
 * at all. The router's pending timers run on the page's fake clock, so every window is exact.
 */
const START = new Date('2026-10-09T00:00:00Z');

async function mount(page: Page) {
  await page.clock.install({ time: START });
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/page-reentry-browser.tsx';
    (await import(/* @vite-ignore */ path)).mount();
  });
  await expect(app(page).getByRole('link', { name: 'Alpha plan' })).toBeVisible();
}
const app = (page: Page) => page.locator('#reentry-fixture');
const frame = (page: Page) => app(page).locator('[data-page-frame]');
/** Holds the next page load until `release()`, and records whether a frame was ever on screen. */
async function holdNextOpen(page: Page) {
  await page.evaluate(() => {
    const seen = { frame: false };
    (window as unknown as { frameSeen: typeof seen }).frameSeen = seen;
    new MutationObserver(() => {
      if (document.querySelector('#reentry-fixture [data-page-frame]')) seen.frame = true;
    }).observe(document.body, { subtree: true, childList: true });
    window.holdPages = new Promise<void>((resolve) => {
      (window as unknown as { releasePages: () => void }).releasePages = resolve;
    });
  });
}
const release = (page: Page) =>
  page.evaluate(() => (window as unknown as { releasePages: () => void }).releasePages());
const frameSeen = (page: Page) =>
  page.evaluate(() => (window as unknown as { frameSeen: { frame: boolean } }).frameSeen.frame);
/** Opens Alpha from the list with the clock stopped, so the test decides how long the load takes. */
async function openAlpha(page: Page) {
  await holdNextOpen(page);
  await page.clock.pauseAt(new Date(START.getTime() + 60_000));
  await app(page).getByRole('link', { name: 'Alpha plan' }).click();
  await expect.poll(() => page.evaluate(() => window.pageOpens.length)).toBe(1);
}
const body = (page: Page) =>
  page.frameLocator('#reentry-fixture iframe').getByText('Body of Alpha plan.');

test('a fast open shows no frame', async ({ page }) => {
  await mount(page);
  await openAlpha(page);
  await page.clock.runFor(100);
  await release(page);
  await page.clock.runFor(500);
  await expect(body(page)).toBeVisible();
  expect(await frameSeen(page)).toBe(false);
});

test('a held open shows the frame by 150 ms and keeps it for at least 300 ms', async ({ page }) => {
  await mount(page);
  await openAlpha(page);
  await page.clock.runFor(140);
  await expect(frame(page)).toHaveCount(0);
  await page.clock.runFor(20);
  await expect(frame(page)).toBeVisible();

  // The real header with the title the list already held, four quiet lines, nothing of the page.
  await expect(app(page).getByRole('heading', { level: 1, name: 'Alpha plan' })).toBeVisible();
  await expect(frame(page).locator('[data-frame-line]')).toHaveCount(4);
  await expect(frame(page).locator('[data-frame-line]').first()).not.toHaveCSS(
    'background-color',
    'rgba(0, 0, 0, 0)',
  );
  await expect(app(page).locator('iframe')).toHaveCount(0);
  await expect(
    app(page).getByRole('button', {
      name: /comments|files|share|export|manage|chat|more actions/i,
    }),
  ).toHaveCount(0);
  const animations = await frame(page).evaluate(
    (element) =>
      [element, ...element.querySelectorAll('*')].flatMap((node) => node.getAnimations()).length,
  );
  expect(animations).toBe(0);

  // The load ends 40 ms after the frame appeared; the frame stays until 300 ms have passed.
  await release(page);
  await page.clock.runFor(100);
  await expect(frame(page)).toBeVisible();
  await expect(app(page).locator('iframe')).toHaveCount(0);
  await page.clock.runFor(200);
  await expect(body(page)).toBeVisible();
  await expect(frame(page)).toHaveCount(0);
});

test('a page the list never held opens on the neutral word, not a guessed title', async ({
  page,
}) => {
  await mount(page);
  await holdNextOpen(page);
  await page.clock.pauseAt(new Date(START.getTime() + 60_000));
  await page.evaluate(() => {
    location.hash = '/pages/c0000009-1111-4111-8111-000000000001';
  });
  await page.clock.runFor(200);
  await expect(frame(page)).toBeVisible();
  await expect(app(page).getByRole('heading', { level: 1, name: 'Opening page…' })).toBeVisible();
  // Leave before the unknown page answers, so its refusal never reaches the log.
  await page.evaluate(() => {
    location.hash = '/';
  });
  await page.clock.runFor(400);
  await release(page);
});

test('going back to the list while the open is held works without an error', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await mount(page);
  await openAlpha(page);
  await page.clock.runFor(200);
  await expect(frame(page)).toBeVisible();
  await page.goBack();
  await page.clock.runFor(400);
  await expect(app(page).getByRole('link', { name: 'Alpha plan' })).toBeVisible();
  await expect(frame(page)).toHaveCount(0);
  await release(page);
  await page.clock.runFor(500);
  await expect(app(page).getByRole('link', { name: 'Alpha plan' })).toBeVisible();
  await expect(app(page).locator('iframe')).toHaveCount(0);
  expect(errors).toEqual([]);
});

for (const scheme of ['light', 'dark'] as const) {
  for (const width of [1440, 390]) {
    test(`the frame fits ${width} px wide in ${scheme} without sideways scrolling`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, scheme);
      await page.setViewportSize({ width, height: 900 });
      await mount(page);
      await openAlpha(page);
      await page.clock.runFor(200);
      await expect(frame(page)).toBeVisible();
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
      await page.screenshot({ path: capturePath(`page-frame-${width}-${scheme}.png`) });
    });
  }
}
