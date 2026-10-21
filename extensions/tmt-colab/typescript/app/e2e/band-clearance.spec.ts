import { expect, test, type Page } from '@playwright/test';
import { needsYouSamplePage } from '../test/needs-you-sample-page.js';

/** Parent bands sit in spacers the frame reserves, so a band never covers the passage around it
 * or another band, at any width (#3968). Measure the settled layout: a `fullPage` screenshot of
 * this page shows a layout that differs from the live one and from scrolled-viewport captures. */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

/** Every band's box and every line of the page's own text, in viewport coordinates. */
async function layout(page: Page) {
  const frame = page.locator('#ask-page-fixture iframe');
  const origin = await frame.boundingBox();
  const lines = await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('body')
    .evaluate(() => {
      const found: { text: string; top: number; bottom: number }[] = [];
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const content = node.textContent?.trim();
        if (!content || node.parentElement?.closest('style')) continue;
        const range = document.createRange();
        range.selectNodeContents(node);
        for (const rect of range.getClientRects())
          if (rect.height > 0)
            found.push({ text: content.slice(0, 24), top: rect.top, bottom: rect.bottom });
      }
      return found;
    });
  const bands = await page.locator('[data-inline-slot]').evaluateAll((nodes) =>
    nodes
      .map((node) => {
        const box = node.getBoundingClientRect();
        return {
          id: node.getAttribute('data-inline-slot')!.slice(-4),
          top: box.top,
          bottom: box.bottom,
        };
      })
      .sort((a, b) => a.top - b.top),
  );
  return {
    bands,
    lines: lines.map((line) => ({
      ...line,
      top: line.top + origin!.y,
      bottom: line.bottom + origin!.y,
    })),
  };
}

type Layout = Awaited<ReturnType<typeof layout>>;

/** What covers what: text lines a band's box crosses, and bands that start above the previous
 * band's end. A pixel of slack absorbs sub-pixel rounding between the frame and the parent. */
function collisions({ bands, lines }: Layout) {
  return [
    ...bands.flatMap((band) =>
      lines
        .filter((line) => line.top < band.bottom - 1 && line.bottom > band.top + 1)
        .map((line) => `band ${band.id} covers "${line.text}"`),
    ),
    ...bands.flatMap((band, index) =>
      index && band.top < bands[index - 1].bottom - 1
        ? [`band ${band.id} starts inside band ${bands[index - 1].id}`]
        : [],
    ),
  ];
}

/** Nothing covers anything once the layout has caught up. A width change reflows the frame first
 * and the bands a few frames later (about 100-200 ms, with the two layers measuring each other),
 * so the check waits for the clear state and then holds it across more frames. */
async function clear(page: Page) {
  await expect.poll(async () => collisions(await layout(page))).toEqual([]);
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve))),
      ),
  );
  expect(collisions(await layout(page))).toEqual([]);
}

for (const layoutName of ['freeform', 'column'])
  for (const width of [390, 1024, 1440])
    test(`no band covers the page text or another band at ${width} in a ${layoutName} page`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
      await page.goto('/');
      await run(page, 'mount', { statusAgents: true, layout: layoutName });
      await run(page, 'authored', needsYouSamplePage);
      const bands = page.locator('[data-inline-slot]');
      await expect(bands).toHaveCount(6);
      for (const band of await bands.all()) await expect(band).not.toHaveAttribute('data-detached');
      await clear(page);
      const before = (await layout(page)).bands.map((band) => band.top);
      // The same page after a live width change: the spacers follow the new wrap.
      await page.setViewportSize({ width: width === 390 ? 1440 : 390, height: 900 });
      await expect
        .poll(async () => (await layout(page)).bands.map((band) => band.top))
        .not.toEqual(before);
      await clear(page);
    });
