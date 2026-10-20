import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');
const edges = async (node: Locator) => {
  const box = await node.boundingBox();
  return { left: box!.x, right: box!.x + box!.width, top: box!.y, bottom: box!.y + box!.height };
};
/** Where a passage's text starts: the left edge of its first text node. */
const textLeft = (node: Locator) =>
  node.evaluate((element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let text = walker.nextNode(); text; text = walker.nextNode()) {
      if (!text.textContent?.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(text);
      return range.getClientRects()[0]!.left;
    }
    throw new Error('no text');
  });

async function mount(page: Page, width: number, theme: 'light' | 'dark', layout = 'column') {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, layout });
}
const sizes = [
  [1440, 'light'],
  [1440, 'dark'],
  [1024, 'light'],
  [390, 'light'],
  [390, 'dark'],
  [2000, 'light'],
] as const;

for (const [width, theme] of sizes) {
  test(`a reading band sits on the column of its passage at ${width} ${theme}`, async ({
    page,
  }) => {
    await mount(page, width, theme);
    await run(page, 'reading', { count: 2, darkPage: theme === 'dark' });
    const band = page.locator('[data-inline-reading]').first();
    await expect(band).not.toHaveAttribute('data-detached');
    const passage = await edges(frame(page).locator('tmt-read').first());
    const bar = await edges(band.locator('.needs-you-bar'));
    expect(bar.left).toBeCloseTo(passage.left, 0);
    expect(bar.right).toBeCloseTo(passage.right, 0);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`band-reading-${width}-${theme}.png`) });
  });

  test(`a review band and its Goes to line sit on the column at ${width} ${theme}`, async ({
    page,
  }) => {
    await mount(page, width, theme);
    await run(page, 'review', { sections: 1, darkPage: theme === 'dark' });
    const card = page.locator('.review-card').first();
    await expect(page.locator('[data-inline-review]').first()).not.toHaveAttribute('data-detached');
    const passage = await edges(frame(page).locator('tmt-review').first());
    const bar = await edges(card.locator('.needs-you-bar'));
    expect(bar.left).toBeCloseTo(passage.left, 0);
    expect(bar.right).toBeCloseTo(passage.right, 0);
    // The line under the band starts where the band's own text starts.
    const recipient = card.locator('.review-recipient');
    await expect(recipient).toContainText('Goes to');
    expect(await textLeft(recipient)).toBeCloseTo(
      await textLeft(card.locator('.needs-you-line')),
      0,
    );
    const line = await edges(recipient);
    expect(line.right).toBeLessThanOrEqual(bar.right + 0.5);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`band-review-${width}-${theme}.png`) });
  });

  test(`a compact question block sits on the column of its passage at ${width} ${theme}`, async ({
    page,
  }) => {
    await mount(page, width, theme);
    await run(page, 'chipPage', { count: 1, darkPage: theme === 'dark' });
    const row = page.locator('.question-line').first();
    await expect(row).toBeVisible();
    const passage = await edges(frame(page).locator('#host-text'));
    const block = await edges(page.locator('.question-compact-block'));
    expect(block.left).toBeCloseTo(passage.left, 0);
    expect(block.right).toBeCloseTo(passage.right, 0);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`band-question-${width}-${theme}.png`) });
  });
}

/** The owner's QA page shape: a section whose question sits right under a paragraph. */
const section = `<style>body{font:14px/1.5 sans-serif;margin:0;padding:16px}p{margin:0 0 16px}section{margin:0 0 16px;padding:16px;border:1px solid #ccc}</style>
<section><h2>Pick a default</h2><p id="lead">New pages need a default project scope.</p>
<tmt-question data-id="7a3e1c20-4b1d-4c8e-9f20-000000003781" data-kind="single" data-agent="Agent 1"><p data-question>Which default scope should a new page get?</p><span data-choice>The last chosen project</span><span data-choice>No project</span></tmt-question>
<div id="side">Side decision inside running text: the compact chip sits here
<tmt-question data-id="7a3e1c20-4b1d-4c8e-9f20-000000003782" data-kind="single" data-agent="Agent 1" data-form="chip"><p data-question>Show archived pages?</p><span data-choice>Yes</span><span data-choice>No</span></tmt-question>
and the sentence continues after it.</div></section>`;

for (const layout of ['column', 'freeform'])
  test(`a compact question line follows its passage with only the page's own gap (${layout})`, async ({
    page,
  }) => {
    await mount(page, 1440, 'light', layout);
    await run(page, 'authored', section);
    const rows = page.locator('.question-line');
    await expect(rows).toHaveCount(2);
    await expect(page.locator('[data-inline-question]').first()).not.toHaveAttribute(
      'data-detached',
    );
    const lead = await edges(frame(page).locator('#lead'));
    const first = await edges(rows.nth(0));
    // The paragraph's own 16 px margin and nothing the frame reserves on top of it.
    expect(first.top - lead.bottom).toBeLessThanOrEqual(16.5);
    const side = await edges(frame(page).locator('#side'));
    const second = await edges(rows.nth(1));
    expect(second.top).toBeGreaterThan(first.bottom - 1);
    expect(second.top - first.bottom).toBeLessThanOrEqual(side.bottom - side.top);
    if (process.env.COLAB_CAPTURE_DIR)
      await page.screenshot({ path: capturePath(`band-gap-${layout}.png`) });
  });
