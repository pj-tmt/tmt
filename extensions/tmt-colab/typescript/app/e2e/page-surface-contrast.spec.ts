import { mkdirSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

/**
 * Colab draws its cards over the author's page (#3959): whatever the page paints, and whatever
 * the app theme is, a card's text stays readable against what is really behind it. A card is the
 * page sheet of the active theme with a 1px light rule, not the needs-you band and not a dark edge
 * (#4068). The render frame's canvas is white, so a page that sets no background is a white page in
 * both themes.
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

/** The text contrast, and the surface and boundary, of the parent-drawn cards on the page. */
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
    const cards: { selector: string; fill: string; rule: string; other: string }[] = [];
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
        // A card's own surface and boundary: the question block and the proposal card.
        if (element.matches('.question-compact-block, .proposal-card')) {
          const style = getComputedStyle(element);
          // The block draws its rule as an outline inside its box (nothing moves), the proposal
          // card as a border; the other of the two must be absent.
          const block = element.matches('.question-compact-block');
          const outline = `${style.outlineWidth} ${style.outlineStyle} ${style.outlineColor}`;
          const border = `${style.borderTopWidth} ${style.borderTopStyle} ${style.borderTopColor}`;
          cards.push({
            selector: element.className.split(' ')[0],
            fill: style.backgroundColor,
            rule: block ? outline : border,
            other: block ? style.borderTopStyle : style.outlineStyle,
          });
        }
      }
    }
    return { text, cards };
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
          const { text, cards } = await measure(page, backdrop);
          if (process.env.CONTRAST_DEBUG)
            console.log(
              name,
              paint,
              theme,
              JSON.stringify(text.map((t) => [t.text, t.ratio.toFixed(2), t.ink, t.surface])),
              JSON.stringify(cards),
            );
          expect(text.length).toBeGreaterThan(0);
          // A measurement that is not a number would pass every bound below.
          expect(text.every((entry) => Number.isFinite(entry.ratio))).toBe(true);
          // Text against the surface it sits on: 4.5:1.
          expect(text.filter((entry) => entry.ratio < 4.5)).toEqual([]);
          // Every card is the theme's sheet with a 1px light rule and no dark edge; the 3:1 boundary
          // floor does not apply to it, by Ben's decision (#4068, as for floating panels in #4025).
          const sheet = theme === 'dark' ? 'rgb(16, 16, 16)' : 'rgb(255, 255, 255)';
          const rule = theme === 'dark' ? 'rgb(48, 48, 48)' : 'rgb(208, 208, 208)';
          // A reading and a review band hold no card of these two kinds.
          if (name === 'question group' || name === 'question and proposal')
            expect(cards.length).toBeGreaterThan(0);
          expect(
            cards.filter(
              (card) =>
                card.fill !== sheet || card.rule !== `1px solid ${rule}` || card.other !== 'none',
            ),
          ).toEqual([]);
          const captures = process.env.COLAB_CONTRAST_CAPTURE_DIR;
          if (captures) {
            mkdirSync(captures, { recursive: true });
            await page.screenshot({
              fullPage: true,
              path: `${captures}/${name.replaceAll(' ', '-')}-${paint}-${theme}-${width}.png`,
            });
          }
        });
