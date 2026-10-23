import { readdirSync, readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

// #3927: the layout and component references teach markup for a `column` page. Every sample must
// render there: each top-level block on the column, and nothing wider than the window.
// #3928: every page template renders the same way in the layout it sets, with its blocks on the
// frame; its captures are the design review of the starting pages.
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

type Sample = { name: string; source: string; layout: 'column' | 'full'; capture?: string };

const references: Sample[] = ['layout', 'components'].flatMap((name) => {
  const text = readFileSync(
    new URL(`../../../skills/tmt-colab/references/${name}.md`, import.meta.url),
    'utf8',
  );
  return text
    .split('```html\n')
    .slice(1)
    .map((block, index) => ({
      name: `${name}.md #${index + 1}`,
      source: block.split('```')[0],
      layout: 'column' as const,
    }));
});

// A template names its layout on its first line; `page create --template` sets that layout, puts
// the page-style starter (without its body frame rule) where `{{style}}` stands and a fresh UUID
// where each `{{uuid:NAME}}` stands (cli_template.rs renders and checks all three).
const pageStyle = readFileSync(
  new URL('../../../skills/tmt-colab/references/page-style.md', import.meta.url),
  'utf8',
);
const baseStyle = pageStyle
  .split('<!-- BEGIN generated page style -->')[1]
  .split('```html\n')[1]
  .split('```')[0]
  .replace('    max-width: 1280px;\n    margin: 0 auto;\n    padding: var(--gap);\n', '')
  .trimEnd();
const templateDirectory = new URL('../../../rust/tmt-colab/templates/', import.meta.url);
const templates: Sample[] = readdirSync(templateDirectory)
  .filter((file) => file.endsWith('.html'))
  .sort()
  .map((file) => {
    const text = readFileSync(new URL(file, templateDirectory), 'utf8').replace(
      '{{style}}',
      () => baseStyle,
    );
    const names = [...new Set([...text.matchAll(/\{\{uuid:([a-z-]+)\}\}/g)].map((m) => m[1]))];
    const source = names.reduce(
      (rendered, name, index) =>
        rendered.replaceAll(`{{uuid:${name}}}`, `00000000-0000-4000-8000-00000000010${index}`),
      text,
    );
    const layout = /^<!-- layout: (column|full)\./.exec(text)?.[1] as 'column' | 'full';
    const name = file.replace(/\.html$/, '');
    return { name: `template ${name}`, source, layout, capture: name };
  });

const samples = [...references, ...templates];

test('the references carry their samples', () => {
  expect(references.length).toBeGreaterThanOrEqual(11);
});

test('the templates are the four documented starting pages', () => {
  expect(templates.map((t) => [t.capture, t.layout])).toEqual([
    ['board', 'full'],
    ['compare', 'column'],
    ['report', 'column'],
    ['review', 'column'],
  ]);
  for (const { source } of templates) expect(source).not.toContain('{{');
});

for (const width of [1440, 390])
  for (const { name, source, layout, capture } of samples)
    test(`${name} lays out on the ${layout} frame at ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await run(page, 'mount', { statusAgents: true, layout });
      await run(page, 'authored', source);
      await expect(frame(page).locator('html')).toHaveAttribute('data-colab-layout', layout);
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
      // The frame keeps a 28 px marker lane at the right edge; the column is centred in the rest and a
      // full block runs over the lane.
      const usable = size - 28;
      const column = Math.min(1280, usable - 32);
      for (const block of blocks) {
        const label = `<${block.tag}>`;
        // Question lines are drawn by the parent on the same column (column-bands.spec.ts): the
        // question elements in the frame are anchors without a box of their own.
        if (block.tag.startsWith('tmt-question')) continue;
        expect.soft(block.height, `${label} is drawn`).toBeGreaterThan(0);
        expect
          .soft(block.left, `${label} left`)
          .toBeCloseTo(block.full ? 0 : (usable - column) / 2, 0);
        expect
          .soft(block.right, `${label} right`)
          .toBeCloseTo(block.full ? size : (usable + column) / 2, 0);
        expect.soft(block.overflow, `${label} does not scroll sideways`).toBeLessThanOrEqual(0);
      }
      const overflow = await frame(page)
        .locator('html')
        .evaluate((html) => html.scrollWidth - innerWidth);
      expect(overflow).toBeLessThanOrEqual(0);
      if (capture)
        await page.screenshot({
          path: capturePath(`3928-template-${capture}-${width}.png`),
          fullPage: true,
        });
    });
