import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

// #3927: the layout and component references teach markup for a `column` page. Every sample must
// render there: each top-level block on the column, and nothing wider than the window.
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

const samples = ['layout', 'components'].flatMap((name) => {
  const text = readFileSync(
    new URL(`../../../skills/tmt-colab/references/${name}.md`, import.meta.url),
    'utf8',
  );
  return text
    .split('```html\n')
    .slice(1)
    .map((block, index) => ({ name: `${name}.md #${index + 1}`, source: block.split('```')[0] }));
});

test('the references carry their samples', () => {
  expect(samples.length).toBeGreaterThanOrEqual(11);
});

for (const width of [1440, 390])
  for (const { name, source } of samples)
    test(`${name} lays out on the column at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await run(page, 'mount', { statusAgents: true, layout: 'column' });
      await run(page, 'authored', source);
      await expect(frame(page).locator('html')).toHaveAttribute('data-colab-layout', 'column');
      const blocks = await frame(page)
        .locator('body')
        .evaluate((body) => {
          const skipped = 'script, style, template, noscript, link, meta, [hidden]';
          return (
            [...body.children]
              .filter((element) => !element.matches(skipped))
              // The frame's own spacers and markers, which it does not treat as blocks either.
              .filter(
                (element) =>
                  !element.matches(
                    '[data-colab-highlight], [data-colab-proposal-slot], [data-colab-chip-slot]',
                  ),
              )
              .map((element) => {
                const { left, right, height } = element.getBoundingClientRect();
                return {
                  tag: element.tagName.toLowerCase(),
                  full: element.getAttribute('data-width') === 'full',
                  left,
                  right,
                  height,
                  overflow: element.scrollWidth - element.clientWidth,
                };
              })
          );
        });
      expect(blocks.length, 'the sample has at least one top-level block').toBeGreaterThan(0);
      const size = await page.locator('#ask-page-fixture iframe').evaluate((n) => n.clientWidth);
      const column = Math.min(1280, size - 32);
      for (const block of blocks) {
        const label = `<${block.tag}>`;
        // Question lines are drawn by the parent on the same column (column-bands.spec.ts): the
        // question elements in the frame are anchors without a box of their own.
        if (block.tag.startsWith('tmt-question')) continue;
        expect.soft(block.height, `${label} is drawn`).toBeGreaterThan(0);
        expect
          .soft(block.left, `${label} left`)
          .toBeCloseTo(block.full ? 0 : (size - column) / 2, 0);
        expect
          .soft(block.right, `${label} right`)
          .toBeCloseTo(block.full ? size : (size + column) / 2, 0);
        expect.soft(block.overflow, `${label} does not scroll sideways`).toBeLessThanOrEqual(0);
      }
      const overflow = await frame(page)
        .locator('html')
        .evaluate((html) => html.scrollWidth - innerWidth);
      expect(overflow).toBeLessThanOrEqual(0);
    });
