import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

test.use({ timezoneId: 'UTC' });

async function mount(page: Page, kind = 'many', hash = '/') {
  await page.goto('/');
  await page.clock.setFixedTime(new Date('2026-10-17T09:30:00Z'));
  await page.evaluate(
    async ({ kind, hash }) => {
      const path = '/test/today-browser.tsx';
      await (await import(/* @vite-ignore */ path)).mountToday(kind, hash);
    },
    { kind, hash },
  );
}
const app = (page: Page) => page.locator('#today-fixture');
const link = (page: Page, title: string) =>
  app(page).locator('.today-chapter').getByRole('link', { name: title, exact: true });
const panel = (page: Page) => app(page).locator('main[data-chapter="panel"]');
const full = (page: Page) => app(page).locator('main[data-chapter="full"]');
const bar = (page: Page) => app(page).getByRole('region', { name: 'Page preview' });
const held = (page: Page) => app(page).locator('.chapter-held');
const hash = (page: Page) => new URL(page.url()).hash;
const scrollY = (page: Page) => page.evaluate(() => Math.round(window.scrollY));
const wide = (page: Page) => page.setViewportSize({ width: 1440, height: 900 });
/** Counts View Transitions, running each update callback like the browser does. */
async function countTransitions(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as { transitions: number };
    w.transitions = 0;
    document.startViewTransition = ((update: () => Promise<void>) => {
      w.transitions++;
      const done = Promise.resolve().then(update);
      return { updateCallbackDone: done, finished: done, ready: done, skipTransition() {} };
    }) as never;
  });
}
const transitions = (page: Page) =>
  page.evaluate(() => (window as unknown as { transitions: number }).transitions);
/** Scrolls Today down, opens a chapter in the panel and returns the offset it was left at. */
async function openFromScrolledToday(page: Page, title = 'Page 05') {
  await link(page, title).scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollBy(0, 120));
  const offset = await scrollY(page);
  expect(offset).toBeGreaterThan(200);
  await link(page, title).click();
  await expect(panel(page)).toBeVisible();
  // Focus moves into the panel once it has opened.
  await expect(bar(page)).toBeFocused();
  return offset;
}
async function expectBackAtToday(page: Page, offset: number, title: string) {
  await expect(held(page)).toHaveCount(0);
  await expect(app(page).locator('.today h1')).toBeVisible();
  expect(hash(page)).toBe('#/');
  await expect(link(page, title)).toBeFocused();
  expect(Math.abs((await scrollY(page)) - offset)).toBeLessThanOrEqual(1);
}

test('a wide chapter click opens a right panel over a held Today', async ({ page }) => {
  await wide(page);
  await mount(page);
  await link(page, 'Budget review').click();
  expect(hash(page)).toMatch(/^#\/pages\/d0000001/);
  await expect(panel(page)).toBeVisible();
  await expect(bar(page).getByRole('heading', { name: 'Budget review' })).toBeVisible();
  await expect(bar(page)).toBeFocused();
  await expect(held(page).locator('.today-chapters')).toBeVisible();
  const box = await panel(page).boundingBox();
  expect(box!.width).toBeCloseTo(560, 0);
  expect(box!.x + box!.width).toBeCloseTo(1440, 0);
  await expect(panel(page)).toHaveCSS('border-left-width', '1px');
  await expect(panel(page)).toHaveCSS('box-shadow', 'none');
});

test('a narrow chapter click opens the full page with a way back', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mount(page);
  await link(page, 'Budget review').click();
  expect(hash(page)).toMatch(/^#\/pages\/d0000001/);
  await expect(full(page)).toBeVisible();
  await expect(held(page)).toHaveCount(0);
  await expect(app(page).getByRole('button', { name: '← Today' })).toBeVisible();
  await expect(app(page).locator('main[data-chapter="panel"]')).toHaveCount(0);
});

test('a direct visit to a page is the plain page, with no chapter chrome', async ({ page }) => {
  await wide(page);
  await mount(page, 'many', '/pages/d0000001-1111-4111-8111-000000000001');
  await expect(app(page).locator('.page')).toBeVisible();
  await expect(app(page).locator('main[data-chapter]')).toHaveCount(0);
  await expect(bar(page)).toHaveCount(0);
  await expect(held(page)).toHaveCount(0);
});

for (const way of ['Close', 'Escape', 'outside', 'Back'] as const)
  test(`${way} closes the panel back to the same chapter, focused, at the same scroll`, async ({
    page,
  }) => {
    await wide(page);
    await mount(page);
    const entries = await page.evaluate(() => history.length);
    const offset = await openFromScrolledToday(page);
    if (way === 'Close') await bar(page).getByRole('button', { name: 'Close' }).click();
    else if (way === 'Escape') await page.keyboard.press('Escape');
    else if (way === 'Back') await page.goBack();
    else await held(page).click({ position: { x: 24, y: 400 } });
    await expectBackAtToday(page, offset, 'Page 05');
    // Closing steps back; it never pushes another entry.
    expect(await page.evaluate(() => history.length)).toBe(entries + 1);
  });

test('Escape closes what the page has open first, then the panel', async ({ page }) => {
  await wide(page);
  await mount(page);
  const offset = await openFromScrolledToday(page);
  await app(page).getByRole('button', { name: 'Source and export' }).click();
  const drawer = page.getByRole('dialog');
  await expect(drawer).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();
  await expect(panel(page)).toBeVisible();
  expect(hash(page)).toMatch(/^#\/pages\//);
  await page.keyboard.press('Escape');
  await expectBackAtToday(page, offset, 'Page 05');
});

test('another chapter switches the panel in place; Back then closes it', async ({ page }) => {
  await wide(page);
  await mount(page);
  const offset = await openFromScrolledToday(page, 'Page 05');
  const length = await page.evaluate(() => history.length);
  await held(page).getByRole('link', { name: 'Page 06', exact: true }).click();
  await expect(bar(page).getByRole('heading', { name: 'Page 06' })).toBeVisible();
  expect(await page.evaluate(() => history.length)).toBe(length);
  await page.goBack();
  // Focus returns to the chapter that was open when the panel closed.
  await expectBackAtToday(page, offset, 'Page 06');
});

test('Open full page expands in place and ← Today returns to the chapter', async ({ page }) => {
  await wide(page);
  await mount(page);
  const offset = await openFromScrolledToday(page);
  const before = hash(page);
  await bar(page).getByRole('button', { name: 'Open full page →' }).click();
  await expect(full(page)).toBeVisible();
  await expect(held(page)).toHaveCount(0);
  expect(hash(page)).toBe(before);
  await app(page).getByRole('button', { name: '← Today' }).click();
  await expectBackAtToday(page, offset, 'Page 05');
});

test('a reload on the panel URL shows the panel over Today again', async ({ page }) => {
  await wide(page);
  await mount(page);
  await link(page, 'Budget review').click();
  await expect(panel(page)).toBeVisible();
  const url = hash(page);
  await page.reload();
  await page.evaluate(
    async ({ hash }) => {
      const path = '/test/today-browser.tsx';
      await (await import(/* @vite-ignore */ path)).mountToday('many', hash);
    },
    { hash: url.slice(1) },
  );
  await expect(panel(page)).toBeVisible();
  await expect(held(page).locator('.today-chapters')).toBeVisible();
  await bar(page).getByRole('button', { name: 'Close' }).click();
  await expect(app(page).locator('.today h1')).toBeVisible();
  expect(hash(page)).toBe('#/');
});

test('a modified click keeps the link: no panel is opened', async ({ page }) => {
  await wide(page);
  await mount(page);
  const [popup] = await Promise.all([
    page.context().waitForEvent('page'),
    link(page, 'Budget review').click({ modifiers: ['ControlOrMeta'] }),
  ]);
  await popup.close();
  expect(hash(page)).toBe('#/');
  await expect(app(page).locator('main[data-chapter]')).toHaveCount(0);
});

test('open and close each start one View Transition; reduced motion and no support start none', async ({
  page,
}) => {
  await wide(page);
  await mount(page);
  await countTransitions(page);
  await link(page, 'Budget review').click();
  await expect(panel(page)).toBeVisible();
  expect(await transitions(page)).toBe(1);
  await bar(page).getByRole('button', { name: 'Close' }).click();
  await expect(app(page).locator('.today h1')).toBeVisible();
  expect(await transitions(page)).toBe(2);

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await link(page, 'Budget review').click();
  await expect(panel(page)).toBeVisible();
  await bar(page).getByRole('button', { name: 'Close' }).click();
  await expect(app(page).locator('.today h1')).toBeVisible();
  expect(await transitions(page)).toBe(2);

  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.evaluate(() => {
    (document as unknown as { startViewTransition?: unknown }).startViewTransition = undefined;
  });
  await link(page, 'Budget review').click();
  await expect(panel(page)).toBeVisible();
  await bar(page).getByRole('button', { name: 'Close' }).click();
  await expect(app(page).locator('.today h1')).toBeVisible();
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`chapter look ${width} ${theme}: ${width === 1440 ? 'panel over Today' : 'full page'}, no overflow`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
      // The look is the settled state; the transition has its own test.
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await mount(page, 'few');
      await link(page, 'Budget review').click();
      await (width === 390 ? full(page) : panel(page)).waitFor();
      await page.evaluate((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const geometry = await page.evaluate(() => {
        const main = document.querySelector('#today-fixture main[data-chapter]')!;
        const rect = main.getBoundingClientRect();
        const heldBox = document
          .querySelector('#today-fixture .chapter-held')
          ?.getBoundingClientRect();
        return {
          overflow: document.documentElement.scrollWidth > innerWidth,
          left: rect.left,
          width: rect.width,
          heldTop: heldBox?.top,
          headerBottom: document.querySelector('.tmt-ui-header:not([hidden])')
            ? Math.max(
                ...[...document.querySelectorAll('#today-fixture .tmt-ui-header')].map(
                  (header) => header.getBoundingClientRect().bottom,
                ),
              )
            : 0,
        };
      });
      expect(geometry.overflow).toBe(false);
      if (width === 1440) {
        expect(geometry.width).toBeCloseTo(560, 0);
        // The held Today starts below the header, like the panel.
        expect(geometry.heldTop).toBeCloseTo(geometry.headerBottom, 0);
      } else expect(geometry.width).toBeCloseTo(390, 0);
      await page.screenshot({
        path: capturePath(
          `chapter/chapter-${width === 390 ? 'full' : 'panel'}-${width}-${theme}.png`,
        ),
      });
    });

/** The running View Transition's animations, as the browser reports them once it is ready. */
async function transitionAnimations(page: Page) {
  await page.waitForFunction(
    () => (window as unknown as { lastTransition?: unknown }).lastTransition,
  );
  return page.evaluate(async () => {
    await (window as unknown as { lastTransition: { ready: Promise<void> } }).lastTransition.ready;
    return document
      .getAnimations()
      .filter((animation) => (animation.effect as KeyframeEffect).pseudoElement)
      .map((animation) => {
        const effect = animation.effect as KeyframeEffect;
        // A CSS animation's curve is the keyframes' timing function, not the effect's.
        return {
          name: effect.pseudoElement,
          duration: effect.getTiming().duration,
          easing: effect.getKeyframes()[0]?.easing,
        };
      });
  });
}
async function recordTransitions(page: Page) {
  await page.evaluate(() => {
    const original = document.startViewTransition.bind(document);
    document.startViewTransition = ((update: never) => {
      const transition = original(update);
      (window as unknown as { lastTransition: unknown }).lastTransition = transition;
      return transition;
    }) as never;
  });
}

test('the transition follows the design: shared title .45s, panel in .35s, out .3s', async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name !== 'chromium', 'View Transitions are checked in Chromium');
  await wide(page);
  await mount(page, 'few');
  await recordTransitions(page);
  await link(page, 'Budget review').click();
  const opening = await transitionAnimations(page);
  const curve = 'cubic-bezier(0.2, 0.8, 0.2, 1)';
  expect(opening).toContainEqual({
    name: '::view-transition-group(chapter-title)',
    duration: 450,
    easing: curve,
  });
  expect(opening).toContainEqual({
    name: '::view-transition-new(chapter-panel)',
    duration: 350,
    easing: curve,
  });
  expect(opening).toContainEqual(
    expect.objectContaining({ name: '::view-transition-new(root)', duration: 350 }),
  );
  // Mid-way through the open: every transition animation held at the same instant.
  await page.evaluate(() => {
    for (const animation of document.getAnimations()) {
      if (!(animation.effect as KeyframeEffect).pseudoElement) continue;
      animation.pause();
      animation.currentTime = 160;
    }
  });
  await page.screenshot({ path: capturePath('chapter/chapter-opening-1440-light.png') });
  await page.evaluate(() => {
    for (const animation of document.getAnimations()) animation.finish();
  });
  await page.evaluate(async () => {
    const w = window as unknown as { lastTransition?: { finished: Promise<void> } };
    await w.lastTransition!.finished;
    w.lastTransition = undefined;
  });
  await bar(page).getByRole('button', { name: 'Close' }).click();
  const closing = await transitionAnimations(page);
  expect(closing).toContainEqual({
    name: '::view-transition-old(chapter-panel)',
    duration: 300,
    easing: 'ease-in',
  });
  expect(closing).toContainEqual(
    expect.objectContaining({ name: '::view-transition-group(chapter-title)', duration: 450 }),
  );
});
