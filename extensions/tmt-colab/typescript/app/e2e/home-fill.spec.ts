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

// The router scrolls the window to the top when a load renders; a re-read must not move a reader
// who is part-way down the list (#4226: it reset the page-scroll checks of the chrome spec too).
for (const width of [1440, 390, 320])
  test(`a re-read keeps the reader where they scrolled: ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 });
    await page.goto('/');
    await page.evaluate(async () => {
      const path = '/test/page-list-browser.tsx';
      // 130 pages that have not reported: the list is long and reads again every 2 s.
      await (await import(path)).mountPageList('bulk', '/?stage=all');
    });
    const list = page.locator('#list-fixture');
    await expect(list.locator('.pages > li').first()).toBeVisible();
    await page.evaluate(() => {
      scrollTo(0, (document.documentElement.scrollHeight - innerHeight) / 2);
      // The lowest offset a scroll event ever reports: a reset to the top would show as 0.
      const seen = { lowest: scrollY };
      window.addEventListener('scroll', () => (seen.lowest = Math.min(seen.lowest, scrollY)));
      (window as unknown as { seen: typeof seen }).seen = seen;
    });
    const before = await page.evaluate(() => scrollY);
    expect(before).toBeGreaterThan(100);
    const reads = await page.evaluate(() => window.__listReads);
    // The first re-read is done once the second one starts: it is scheduled after the render.
    await expect
      .poll(() => page.evaluate(() => window.__listReads), { timeout: 15_000 })
      .toBeGreaterThanOrEqual(reads + 2);
    expect(
      await page.evaluate(() => (window as unknown as { seen: { lowest: number } }).seen.lowest),
    ).toBe(before);
    expect(await page.evaluate(() => scrollY)).toBe(before);
  });
