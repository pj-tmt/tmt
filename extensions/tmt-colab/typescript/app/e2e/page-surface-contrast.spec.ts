import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

/**
 * Colab draws its cards over the author's page (#3959): whatever the page paints, and whatever
 * the app theme is, a card's text and edge stay readable against what is really behind them. The
 * render frame's canvas is white, so a page that sets no background is a white page in both themes.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

const pages = {
  light: { options: { darkPage: false }, backdrop: '#ffffff' },
  dark: { options: { darkPage: true }, backdrop: '#181818' },
  bare: { options: { bare: true }, backdrop: '#ffffff' },
} as const;

const cases = {
  'question group': async (page: Page, options: object) => {
    await run(page, 'questionGroup', options);
    await page.locator('[data-question-group] .question-line').first().click();
  },
  'question and proposal': (page: Page, options: object) =>
    run(page, 'question', { mixed: true, other: true, ...options }),
  reading: (page: Page, options: object) => run(page, 'reading', { count: 1, ...options }),
  review: (page: Page, options: object) => run(page, 'review', { sections: 1, ...options }),
} as const;

/** The least text contrast and edge contrast among the parent-drawn cards on the page. */
const measure = (page: Page, backdrop: string) =>
  page.evaluate((backdrop) => {
    const canvas = document.createElement('canvas').getContext('2d', { willReadFrequently: true })!;
    const rgba = (value: string): [number, number, number, number] => {
      canvas.clearRect(0, 0, 1, 1);
      canvas.fillStyle = '#000';
      canvas.fillStyle = value;
      canvas.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = canvas.getImageData(0, 0, 1, 1).data;
      return [r, g, b, a / 255];
    };
    const over = (top: number[], bottom: number[]): [number, number, number, number] => {
      const alpha = top[3] + bottom[3] * (1 - top[3]);
      const channel = (i: number) =>
        (top[i] * top[3] + bottom[i] * bottom[3] * (1 - top[3])) / (alpha || 1);
      return [channel(0), channel(1), channel(2), alpha];
    };
    const luminance = ([r, g, b]: number[]) => {
      const lin = (c: number) => {
        const s = c / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
    };
    const ratio = (a: number[], b: number[]) => {
      const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
      return (hi + 0.05) / (lo + 0.05);
    };
    const page = rgba(backdrop);
    /** What is drawn behind an element: its ancestors' fills over the author's page. */
    const behind = (element: Element) => {
      const layers: number[][] = [];
      for (let node: Element | null = element; node; node = node.parentElement) {
        const fill = rgba(getComputedStyle(node).backgroundColor);
        if (fill[3] > 0) layers.push(fill);
        if (fill[3] === 1) break;
      }
      return layers.reduceRight((below, layer) => over(layer, below), page as number[]);
    };
    const visible = (element: Element) => {
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect();
      return (
        style.visibility === 'visible' &&
        box.width > 1 &&
        box.height > 1 &&
        !element.closest('[hidden]')
      );
    };
    const text: { text: string; ratio: number; ink: string; surface: string }[] = [];
    const edges: { selector: string; ratio: number }[] = [];
    for (const root of document.querySelectorAll('.proposal-inline')) {
      if (!visible(root)) continue;
      for (const element of [root, ...root.querySelectorAll('*')]) {
        // An inactive control is exempt from the text bound, as in WCAG.
        if (!visible(element) || element.closest(':disabled, [aria-disabled="true"]')) continue;
        const own = [...element.childNodes]
          .filter((node) => node.nodeType === Node.TEXT_NODE)
          .map((node) => node.textContent?.trim() ?? '')
          .join(' ')
          .trim();
        if (own) {
          const surface = behind(element);
          const ink = over(rgba(getComputedStyle(element).color), surface);
          text.push({
            text: own.slice(0, 40),
            ratio: ratio(ink, surface),
            ink: `rgb(${ink.slice(0, 3).map(Math.round)})`,
            surface: `rgb(${surface.slice(0, 3).map(Math.round)})`,
          });
        }
        // A card's boundary: an open block or a proposal card. A closed line is text on the band,
        // identified by its text, which the bound above covers.
        if (element.matches('.question-compact-block[data-expanded], .proposal-card')) {
          const style = getComputedStyle(element);
          // A block draws its edge as an outline inside its box, a proposal card as a border.
          const outlined = style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) >= 1;
          const edge = rgba(outlined ? style.outlineColor : style.borderTopColor);
          const drawn = outlined || (parseFloat(style.borderTopWidth) >= 1 && edge[3] > 0);
          // The boundary a reader sees against the author's page is the fill or the edge,
          // whichever stands out more (WCAG 1.4.11).
          edges.push({
            selector: element.className.split(' ')[0],
            ratio: Math.max(
              ratio(behind(element), page),
              drawn ? ratio(over(edge, page), page) : 1,
            ),
          });
        }
      }
    }
    return { text, edges };
  }, backdrop);

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const [paint, { options, backdrop }] of Object.entries(pages))
      for (const [name, mount] of Object.entries(cases))
        test(`${name} stays readable on a ${paint} page in the ${theme} theme at ${width}`, async ({
          page,
        }) => {
          await page.setViewportSize({ width, height: 1100 });
          await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
          await page.goto('/');
          await run(page, 'mount');
          await mount(page, options);
          await expect(page.locator('.proposal-inline:not([data-detached])').first()).toBeVisible();
          await expect(
            page.frameLocator('#ask-page-fixture iframe').locator('html'),
          ).toHaveAttribute('data-theme', theme);
          const { text, edges } = await measure(page, backdrop);
          if (process.env.CONTRAST_DEBUG)
            console.log(
              name,
              paint,
              theme,
              JSON.stringify(text.map((t) => [t.text, t.ratio.toFixed(2), t.ink, t.surface])),
              JSON.stringify(edges),
            );
          expect(text.length).toBeGreaterThan(0);
          // A measurement that is not a number would pass every bound below.
          expect([...text, ...edges].every((entry) => Number.isFinite(entry.ratio))).toBe(true);
          // Text against the surface it sits on: 4.5:1.
          expect(text.filter((entry) => entry.ratio < 4.5)).toEqual([]);
          // A card's boundary (its fill or its edge) against the author's page: 3:1.
          expect(edges.filter((entry) => entry.ratio < 3)).toEqual([]);
          const captures = process.env.COLAB_CONTRAST_CAPTURE_DIR;
          if (captures) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({
              fullPage: true,
              path: `${captures}/${name.replaceAll(' ', '-')}-${paint}-${theme}-${width}.png`,
            });
          }
        });
