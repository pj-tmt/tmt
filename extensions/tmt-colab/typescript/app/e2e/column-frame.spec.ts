import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

type Layout = 'column' | 'full' | 'canvas' | 'freeform' | undefined;
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
const edges = (node: Locator) =>
  node.evaluate((element) => {
    const { left, right, top, bottom } = element.getBoundingClientRect();
    return { left, right, top, bottom };
  });

/** What an agent typically writes: its own container width, margins and section rhythm. */
const authored = (extra = '') => `<style>
  body { max-width: 900px; margin: 0 auto; padding: 24px; font: 16px/1.5 sans-serif; background: #fff; color: #222 }
  p { margin: 0 0 16px }
  section { margin: 40px 0; max-width: 700px; padding: 16px; border: 1px solid #ccc }
  table { width: 100%; border-collapse: collapse }
  #stubborn { max-width: 900px !important; margin: 40px 0 !important; width: 500px !important }
  #grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px }
  ${extra}
</style>
<h1 id="b1">Weekly plan</h1>
<p id="b2">A paragraph is not narrowed inside the column: its line length follows the column, however wide that is, so a long sentence keeps running until the column ends.</p>
<section id="b3"><h2>Section</h2><p>Inside a section.</p></section>
<table id="b4"><tr><td>One</td><td>Two</td></tr></table>
<section id="b5" data-gap="tight"><p>Close to the table above.</p></section>
<section id="b6" data-gap="loose"><p>Far from the section above.</p></section>
<div id="grid"><div>Left cell</div><div>Right cell</div></div>
<div id="b7" data-width="full" style="background:#eee;max-width:500px;margin:0 60px;padding:0 3px;border:2px solid #999"><h2 id="full-title">Full width</h2><p id="full-text">Text of a full-bleed block stays on the column.</p></div>
<section id="stubborn" style="max-width: 600px; margin-left: 40px"><p>This block insists on its own box.</p></section>
<p id="b9">Last paragraph.</p>`;

async function mount(page: Page, layout: Layout, width = 1440, source = authored()) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, ...(layout ? { layout } : {}) });
  await run(page, 'authored', source);
  await expect(frame(page).locator('#b9')).toBeVisible();
}
const frameWidth = (page: Page) =>
  page.locator('#ask-page-fixture iframe').evaluate((node) => node.clientWidth);
/** Every page reserves a 28 px marker lane at the right edge; the column is centred in what is left. */
const LANE = 28;
const column = (width: number) => {
  const usable = width - LANE;
  const size = Math.min(1280, usable - 32);
  return { left: (usable - size) / 2, right: (usable + size) / 2 };
};
const blocks = ['b1', 'b2', 'b3', 'b4', 'b5', 'b6', 'grid', 'stubborn', 'b9'];

for (const width of [1440, 390])
  for (const layout of ['column', 'full'] as const)
    test(`${layout} pages put every top-level block on one column at ${width}`, async ({
      page,
    }) => {
      await mount(page, layout, width);
      const size = await frameWidth(page);
      const want = column(size);
      for (const id of blocks) {
        const box = await edges(frame(page).locator(`#${id}`));
        expect.soft(box.left, `${id} left`).toBeCloseTo(want.left, 0);
        expect.soft(box.right, `${id} right`).toBeCloseTo(want.right, 0);
      }
      // The line length follows the column; no sideways scroll in the frame or the page.
      const pageOverflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      const frameOverflow = await frame(page)
        .locator('html')
        .evaluate((html) => html.scrollWidth - innerWidth);
      expect(pageOverflow).toBeLessThanOrEqual(0);
      expect(frameOverflow).toBeLessThanOrEqual(0);
      if (process.env.COLAB_CAPTURE_DIR)
        await page.screenshot({
          path: capturePath(`column-frame-${layout}-${width}.png`),
          fullPage: true,
        });
    });

for (const width of [1440, 390])
  test(`gaps between blocks are 16, tight 8 and loose 40 px at ${width}`, async ({ page }) => {
    await mount(page, 'column', width);
    const gap = async (before: string, after: string) =>
      (await edges(frame(page).locator(`#${after}`))).top -
      (await edges(frame(page).locator(`#${before}`))).bottom;
    expect(await gap('b1', 'b2')).toBeCloseTo(16, 0);
    expect(await gap('b2', 'b3')).toBeCloseTo(16, 0);
    // The author's 40 px section margins and the paragraph's own bottom margin change nothing.
    expect(await gap('b4', 'b5')).toBeCloseTo(8, 0);
    expect(await gap('b5', 'b6')).toBeCloseTo(40, 0);
    expect(await gap('b6', 'grid')).toBeCloseTo(16, 0);
  });

test('a full block touches both edges, the marker lane included, and keeps its text on the column', async ({
  page,
}) => {
  for (const width of [1440, 390]) {
    await mount(page, 'column', width);
    const size = await frameWidth(page);
    const want = column(size);
    const block = await edges(frame(page).locator('#b7'));
    expect(block.left).toBeCloseTo(0, 0);
    expect(block.right).toBeCloseTo(size, 0);
    for (const id of ['full-title', 'full-text'])
      expect((await edges(frame(page).locator(`#${id}`))).left).toBeCloseTo(want.left, 0);
    expect((await edges(frame(page).locator('#full-text'))).right).toBeLessThanOrEqual(
      want.right + 0.5,
    );
  }
});

test('author width, max-width and horizontal margins cannot change a block', async ({ page }) => {
  await mount(page, 'column');
  const want = column(await frameWidth(page));
  const block = await edges(frame(page).locator('#stubborn'));
  expect(block.left).toBeCloseTo(want.left, 0);
  expect(block.right).toBeCloseTo(want.right, 0);
  // Colours and inner layout stay the author's: the grid keeps two cells.
  const cells = frame(page).locator('#grid > div');
  const [first, second] = [await edges(cells.nth(0)), await edges(cells.nth(1))];
  expect(second.left).toBeGreaterThan(first.right - 1);
  expect(Math.abs(second.top - first.top)).toBeLessThan(1);
});

for (const layout of ['freeform', 'canvas', undefined] as const)
  test(`${layout ?? 'unset'} pages keep the author's own layout`, async ({ page }) => {
    await mount(page, layout);
    const size = await frameWidth(page);
    // body { max-width: 900px; margin: auto; padding: 24px } is a 948 px box, and the section keeps
    // its 700 px cap (734 with its padding and border).
    const heading = await edges(frame(page).locator('#b1'));
    expect(heading.left).toBeCloseTo((size - LANE - 948) / 2 + 24, 0);
    const section = await edges(frame(page).locator('#b3'));
    expect(section.right - section.left).toBeCloseTo(734, 0);
    const stubborn = await edges(frame(page).locator('#stubborn'));
    expect(stubborn.right - stubborn.left).toBeCloseTo(534, 0);
    expect(await frame(page).locator('html').getAttribute('data-colab-layout')).toBeNull();
  });
