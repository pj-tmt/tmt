import { expect, test, type Page } from '@playwright/test';

// The list reads again while pages lack a summary: every 2 s, at most 15 times, only in a visible
// tab, and never after the pages are all in or the list is left. The fake clock keeps it exact.
async function open(page: Page, missing: number) {
  const start = new Date('2026-10-21T00:00:00Z');
  await page.clock.install({ time: start });
  await page.goto('/');
  // Paused before the list mounts, so only `runFor` moves its timers.
  await page.clock.pauseAt(new Date(start.getTime() + 60_000));
  await page.evaluate(async (count) => {
    const path = '/test/home-fill-browser.tsx';
    (await import(path)).mount(count);
  }, missing);
  await expect(page.locator('#state')).toHaveText(`${missing} missing`);
}
const reads = (page: Page) => page.evaluate(() => window.fillReads);
// One 2 s step at a time: the next read is scheduled by the render the previous one causes.
async function readTimes(page: Page, from: number, count: number) {
  for (let read = from + 1; read <= from + count; read += 1) {
    await page.clock.runFor(2000);
    await expect.poll(() => reads(page)).toBe(read);
  }
}
const setVisibility = (page: Page, state: 'visible' | 'hidden') =>
  page.evaluate((value) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => value });
    document.dispatchEvent(new Event('visibilitychange'));
  }, state);

test('reads every 2 s while pages are missing and stops when they are all in', async ({ page }) => {
  await open(page, 3);
  await expect(page.locator('#state')).toHaveAttribute('data-reading', 'true');
  await page.clock.runFor(1999);
  expect(await reads(page)).toBe(0);
  await page.clock.runFor(1);
  expect(await reads(page)).toBe(1);
  // The fold finishes before the second read.
  await page.evaluate(() => {
    window.fillMissing = 0;
  });
  await page.clock.runFor(2000);
  expect(await reads(page)).toBe(2);
  await expect(page.locator('#state')).toHaveText('0 missing');
  await expect(page.locator('#state')).toHaveAttribute('data-reading', 'false');
  await page.clock.runFor(60_000);
  expect(await reads(page)).toBe(2);
});

test('gives up after 15 reads and says it is no longer reading', async ({ page }) => {
  await open(page, 2);
  await readTimes(page, 0, 15);
  await expect(page.locator('#state')).toHaveAttribute('data-reading', 'false');
  await page.clock.runFor(60_000);
  expect(await reads(page)).toBe(15);
});

test('a hidden tab does not read and a visible one picks up again', async ({ page }) => {
  await open(page, 2);
  await page.clock.runFor(2000);
  expect(await reads(page)).toBe(1);
  await setVisibility(page, 'hidden');
  await page.clock.runFor(30_000);
  expect(await reads(page)).toBe(1);
  await setVisibility(page, 'visible');
  await page.clock.runFor(2000);
  expect(await reads(page)).toBe(2);
});

test('leaving the list stops the reads', async ({ page }) => {
  await open(page, 2);
  await page.clock.runFor(2000);
  expect(await reads(page)).toBe(1);
  await page.getByRole('button', { name: 'Leave' }).click();
  await expect(page.locator('#left')).toBeVisible();
  await page.clock.runFor(60_000);
  expect(await reads(page)).toBe(1);
});

test('a page that loses its summary later gets a fresh set of tries', async ({ page }) => {
  await open(page, 1);
  await readTimes(page, 0, 15);
  // Everything arrives, then a page is edited and is missing again.
  await page.evaluate(() => {
    window.fillMissing = 0;
  });
  await page.getByRole('button', { name: 'Sync' }).click();
  await expect(page.locator('#state')).toHaveText('0 missing');
  await page.evaluate(() => {
    window.fillMissing = 1;
  });
  await page.getByRole('button', { name: 'Sync' }).click();
  await expect(page.locator('#state')).toHaveAttribute('data-reading', 'true');
  await page.clock.runFor(2000);
  expect(await reads(page)).toBe(16);
});
