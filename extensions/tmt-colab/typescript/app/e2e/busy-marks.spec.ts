import { expect, test, type Locator, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';

/**
 * Work that used to swap a label or insert a status paragraph now follows the shared busy mark
 * (gui-style.md, Transient states): nothing changes for the first 400 ms, then one mark shows on
 * the action at work, and the other actions are fenced without turning grey. The page's fake
 * clock makes every window exact.
 */
const START = new Date('2026-10-20T00:00:00Z');
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

/** The infinite animations on a mark: every shown LoaderCircle turns by the one shared rule (#3985). */
const turns = (mark: Locator) =>
  mark.evaluate((node) => ({
    name: getComputedStyle(node.querySelector('svg') ?? node).animationName,
    running: (node.querySelector('svg') ?? node)
      .getAnimations()
      .filter((animation) => animation.effect?.getComputedTiming().iterations === Infinity).length,
  }));

/** The look a capture run uses, `COLAB_LOOK=390:dark`; the assertions hold in every look. */
const look = process.env.COLAB_LOOK ?? '1440:light';
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`busy-${name}-${look.replace(':', '-')}.png`) });
};

/** Starts the fake clock, mounts the fixture page, then stops the clock for the test to drive. */
async function mount(page: Page, setup: (page: Page) => Promise<void>) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.clock.install({ time: START });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await setup(page);
  await page.clock.pauseAt(new Date(START.getTime() + 60_000));
  // The most marks that were visible at once from here on.
  await page.evaluate(() => {
    const seen = window as unknown as { peakMarks: number };
    seen.peakMarks = 0;
    const count = () => {
      seen.peakMarks = Math.max(
        seen.peakMarks,
        document.querySelectorAll('[data-busy="true"] svg, .comment-echo-mark[data-shown] svg')
          .length,
      );
    };
    new MutationObserver(count).observe(document.body, {
      attributes: true,
      subtree: true,
      childList: true,
      attributeFilter: ['data-busy', 'data-shown'],
    });
  });
}
const peak = (page: Page) =>
  page.evaluate(() => (window as unknown as { peakMarks: number }).peakMarks);
/**
 * Settles a held operation while the clock runs, since the fixture waits two frames to render. It
 * steps one frame at a time, so a fast case's 200 ms head start leaves a wide margin below 400 ms.
 */
async function settle(page: Page, release: () => Promise<unknown>) {
  let done = false;
  const released = release().then(() => (done = true));
  for (let frames = 0; !done && frames < 150; frames++) await page.clock.runFor(16);
  await released;
}
const fenced = async (action: Locator) => {
  await expect(action).toHaveAttribute('aria-busy', 'true');
  await expect(action).toHaveAttribute('aria-disabled', 'true');
  // Fenced, not greyed: it keeps its look, so it is not natively disabled.
  expect(await action.evaluate((node) => (node as HTMLButtonElement).disabled)).toBe(false);
};
const markOf = (action: Locator) => action.locator('.tmt-ui-action-mark');

for (const kind of ['fast', 'slow'] as const) {
  test(`a review action that takes ${kind === 'fast' ? 'under' : 'over'} 400 ms ${kind === 'fast' ? 'changes nothing' : 'shows one mark on the pressed action only'}`, async ({
    page,
  }) => {
    let card!: Locator;
    await mount(page, async () => {
      await run(page, 'review');
      card = page.locator('.review-card').first();
      await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute(
        'data-detached',
      );
    });
    const confirm = card.getByRole('button', { name: 'Confirm and close', exact: true });
    const note = card.getByRole('button', { name: 'Add a note', exact: true });
    await run(page, 'holdStatusOnce');
    await confirm.click();
    await fenced(confirm);
    await fenced(note);
    // The label never swaps and no status paragraph is inserted.
    await expect(card.getByText('Saving…')).toHaveCount(0);
    await page.clock.runFor(kind === 'fast' ? 200 : 400);
    if (kind === 'fast') {
      await settle(page, () => run(page, 'releaseStatus'));
      await page.clock.runFor(2000);
      expect(await peak(page)).toBe(0);
    } else {
      await expect(markOf(confirm)).toHaveAttribute('data-busy', 'true');
      await expect(markOf(confirm).locator('svg')).toHaveCount(1);
      // The other actions reserve the same slot but draw nothing in it.
      await expect(markOf(note).locator('svg')).toHaveCount(0);
      expect(await peak(page)).toBe(1);
      await capture(page, 'review');
      await settle(page, () => run(page, 'releaseStatus'));
    }
    await expect(card).toHaveAttribute('data-review-state', 'closed');
  });

  test(`Mark read that takes ${kind === 'fast' ? 'under' : 'over'} 400 ms ${kind === 'fast' ? 'changes nothing' : 'shows its mark alone'}`, async ({
    page,
  }) => {
    let band!: Locator;
    await mount(page, async () => {
      await run(page, 'reading', { count: 1 });
      band = page.locator('[data-inline-reading]').first();
      await expect(band).not.toHaveAttribute('data-detached');
    });
    const mark = band.getByRole('button', { name: 'Mark read', exact: true });
    const next = band.getByRole('button', { name: /^Next/ });
    await run(page, 'holdStatusOnce');
    await mark.click();
    await fenced(mark);
    await expect(next).toHaveAttribute('aria-busy', 'true');
    await expect(band.getByText('Saving…')).toHaveCount(0);
    await page.clock.runFor(kind === 'fast' ? 200 : 400);
    if (kind === 'fast') {
      await settle(page, () => run(page, 'releaseStatus'));
      await page.clock.runFor(2000);
      expect(await peak(page)).toBe(0);
    } else {
      await expect(markOf(mark)).toHaveAttribute('data-busy', 'true');
      expect(await peak(page)).toBe(1);
      await capture(page, 'reading');
      await settle(page, () => run(page, 'releaseStatus'));
    }
    await expect(band).toHaveAttribute('data-reading-state', 'read');
  });

  test(`a Source save that takes ${kind === 'fast' ? 'under' : 'over'} 400 ms keeps the label Save and ${kind === 'fast' ? 'shows no mark' : 'shows the mark'}`, async ({
    page,
  }) => {
    // The drawer opens and the draft changes before the clock stops: only the Save is timed.
    await mount(page, async () => {
      await (await pageAction(page, 'Source')).click();
      await page
        .locator('#source-edit:visible')
        .fill('<p id="selected">Exact selected text, edited</p>');
    });
    const save = page.getByRole('button', { name: 'Save source', exact: true });
    await run(page, 'holdEdit');
    await save.click();
    await fenced(save);
    await expect(page.getByRole('button', { name: 'Saving…' })).toHaveCount(0);
    await page.clock.runFor(kind === 'fast' ? 200 : 400);
    if (kind === 'fast') {
      await settle(page, () => run(page, 'releaseEdit'));
      await page.clock.runFor(2000);
      expect(await peak(page)).toBe(0);
    } else {
      await expect(markOf(save)).toHaveAttribute('data-busy', 'true');
      expect(await peak(page)).toBe(1);
      await capture(page, 'source');
      await settle(page, () => run(page, 'releaseEdit'));
    }
    await expect(save).not.toHaveAttribute('aria-busy', 'true');
    // The mark that has shown stays 400 ms, then goes; a Save of an unchanged draft is not offered.
    await page.clock.runFor(1000);
    await expect(markOf(save)).toHaveAttribute('data-busy', 'false');
  });
}

test('a dispatching ask is a wordless mark that shows after 400 ms', async ({ page }) => {
  await mount(page, async () => {
    await run(page, 'conversation', { surface: 'thread', state: 'dispatching' });
  });
  // The thread opens with the clock stopped, so the mark's delay starts here.
  await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
  const state = page.getByTestId('comment-thread').getByTestId('ask-state');
  await expect(state).toHaveText('');
  const mark = state.locator('.comment-echo-mark');
  await expect(mark).not.toHaveAttribute('data-shown');
  await page.clock.runFor(399);
  await expect(mark).not.toHaveAttribute('data-shown');
  await page.clock.runFor(1);
  await expect(mark).toHaveAttribute('data-shown', 'true');
  await expect(state).toHaveText('');
  await capture(page, 'dispatching');
});

test('a shown mark turns by the shared spinner and stands still under reduced motion', async ({
  page,
}) => {
  let card!: Locator;
  await mount(page, async () => {
    await run(page, 'review');
    card = page.locator('.review-card').first();
    await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
  });
  const confirm = card.getByRole('button', { name: 'Confirm and close', exact: true });
  await run(page, 'holdStatusOnce');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await confirm.click();
  await page.clock.runFor(400);
  const mark = markOf(confirm);
  await expect(mark).toHaveAttribute('data-busy', 'true');
  expect(await turns(mark)).toEqual({ name: 'tmt-ui-spin', running: 1 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await turns(mark)).toEqual({ name: 'none', running: 0 });
  // The mark stays on screen; only the turning stops.
  await expect(mark.locator('svg')).toBeVisible();
  await settle(page, () => run(page, 'releaseStatus'));
});

test('a dispatching ask turns its mark by the shared spinner and stands still under reduced motion', async ({
  page,
}) => {
  await mount(page, async () => {
    await run(page, 'conversation', { surface: 'thread', state: 'dispatching' });
  });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
  const mark = page
    .getByTestId('comment-thread')
    .getByTestId('ask-state')
    .locator('.comment-echo-mark');
  await expect(mark).not.toHaveAttribute('data-shown');
  await page.clock.runFor(399);
  await page.clock.runFor(1);
  await expect(mark).toHaveAttribute('data-shown', 'true');
  expect(await turns(mark)).toEqual({ name: 'tmt-ui-spin', running: 1 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await turns(mark)).toEqual({ name: 'none', running: 0 });
  await expect(mark.locator('svg')).toBeVisible();
});
