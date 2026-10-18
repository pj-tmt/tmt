import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';

async function mount(page: Page, waiting: boolean) {
  await page.evaluate(async (waiting) => {
    const path = '/test/reading-connecting-browser.tsx';
    (await import(/* @vite-ignore */ path)).mount(waiting);
  }, waiting);
}

test('a short wait shows nothing, a long one shows Connecting beside Mark read without moving anything', async ({
  page,
}) => {
  await page.goto('/');
  await page.clock.install();
  await page.clock.pauseAt(Date.now() + 60_000);
  await mount(page, false);
  const mark = page.getByRole('button', { name: 'Mark read', exact: true });
  await expect(mark).toBeEnabled();
  await expect(page.getByRole('status')).toHaveCount(0);
  await mount(page, true);
  await expect(mark).toBeDisabled();
  const status = page.getByRole('status');
  const passage = page.getByText('The passage below the band stays where it is.');
  const before = await passage.boundingBox();
  // Under 400 ms the live region exists but says nothing.
  await page.clock.runFor(399);
  await expect(status).toBeEmpty();
  await page.clock.runFor(2);
  await expect(status).toContainText(text.readingConnecting);
  // The cue sits on the Mark read row, assistive text says what it waits for, nothing moved.
  await expect(status).toContainText(text.readingNotReady);
  const [cue, button] = await Promise.all([
    status.locator('[aria-hidden="true"]').boundingBox(),
    mark.boundingBox(),
  ]);
  expect(Math.abs(cue!.y + cue!.height / 2 - (button!.y + button!.height / 2))).toBeLessThan(
    button!.height,
  );
  expect(cue!.x).toBeGreaterThan(button!.x + button!.width - 1);
  expect(await passage.boundingBox()).toEqual(before);
  await page.screenshot({ path: await capturePath('3744-connecting-390-dark.png') });
  // Ready again: the cue goes and Mark read is enabled.
  await mount(page, false);
  await expect(status).toHaveCount(0);
  await expect(mark).toBeEnabled();
});

for (const [width, theme] of [
  [390, 'dark'],
  [1440, 'light'],
] as const)
  test(`the waiting cue at ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 400 });
    await page.emulateMedia({ colorScheme: theme });
    await page.goto('/');
    await page.evaluate((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.clock.install();
    await page.clock.pauseAt(Date.now() + 60_000);
    await mount(page, true);
    await page.clock.runFor(500);
    await expect(page.getByRole('status')).toContainText(text.readingConnecting);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    await page.screenshot({ path: await capturePath(`3744-connecting-${width}-${theme}.png`) });
  });
