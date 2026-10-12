import { expect, test, type Page } from '@playwright/test';

const SPACE = 'a'.repeat(32);
const first = '00000000-0000-4000-8000-000000000001';
const second = '00000000-0000-4000-8000-000000000002';

async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/recently-opened-browser.tsx';
    await (await import(path)).mountOpenedPages();
  });
}
const open = (page: Page, id: string) =>
  page.evaluate(
    ([space, id]) => {
      location.hash = `space=${space}&path=${encodeURIComponent(`/pages/${id}`)}`;
    },
    [SPACE, id],
  );
const recorded = (page: Page) =>
  page.evaluate(async (space) => {
    const path = '/src/recently-opened.js';
    const { readRecentlyOpened } = await import(path);
    return Object.fromEntries(await readRecentlyOpened(space));
  }, SPACE);

test('opening a page on this device records it, newest last opened, and nothing else', async ({
  page,
}) => {
  await mount(page);
  expect(await recorded(page)).toEqual({});
  const before = Date.now();
  await open(page, first);
  await expect.poll(() => recorded(page)).toHaveProperty(first);
  await open(page, second);
  await expect.poll(async () => Object.keys(await recorded(page)).sort()).toEqual([first, second]);
  const times = await recorded(page);
  expect(times[first]).toBeGreaterThanOrEqual(before - 5000);
  expect(times[second]).toBeGreaterThanOrEqual(times[first]);
  // The record is this device's alone: one IndexedDB key for the space, no other storage.
  const keys = await page.evaluate(
    () =>
      new Promise<string[]>((resolve, reject) => {
        const request = indexedDB.open('tmt-colab', 1);
        request.onsuccess = () => {
          const read = request.result.transaction('keys').objectStore('keys').getAllKeys();
          read.onsuccess = () => resolve(read.result.map(String));
          read.onerror = () => reject(read.error);
        };
        request.onerror = () => reject(request.error);
      }),
  );
  expect(keys.filter((key) => key.startsWith('recent-pages:'))).toEqual([`recent-pages:${SPACE}`]);
  expect(await page.evaluate(() => Object.keys(localStorage))).toEqual([]);
});

test('a switcher opened after the space home loaded shows that list on its first render', async ({
  page,
}) => {
  await mount(page);
  const state = () =>
    page.evaluate(async () => {
      const path = '/test/recently-opened-browser.tsx';
      return (await import(path)).switcherFirstRender();
    });
  await expect.poll(state).toEqual({ state: 'ready', pages: [first, second] });
});
