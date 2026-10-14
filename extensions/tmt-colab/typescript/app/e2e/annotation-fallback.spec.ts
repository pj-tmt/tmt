import { mkdirSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { captureDirectory } from './captures.js';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`opaque fallback preserves authored text and range-end numbering ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        document.getElementById('root')?.setAttribute('hidden', '');
        const path = '/src/renderer.ts';
        const { mountRenderer } = await import(path);
        const host = document.createElement('div');
        host.id = 'fallback-fixture';
        document.body.append(host);
        const handle = await mountRenderer(
          host,
          '<style>body{margin:40px;background:white;color:#343434;font:24px/1.6 sans-serif;max-width:700px}html[data-theme="dark"] body{background:#080808;color:#ddd}</style><p id="quote"><strong>Check the mobile layout before release.</strong></p><p id="wrapped">Keep this long range wrapped over several lines on mobile, with its number placed on the final line.</p><script>window.Highlight=undefined;</script>',
          {
            signal: new AbortController().signal,
            onState: (state: string) => (host.dataset.state = state),
            onAnchors: (ids: string[]) => (host.dataset.anchors = JSON.stringify(ids)),
            onOpenThread: (id: string) => (host.dataset.opened = id),
          },
        );
        const frame = host.querySelector('iframe')!;
        frame.style.width = '100%';
        frame.style.border = '0';
        Object.assign(window, { fallbackHandle: handle });
      });
      const host = page.locator('#fallback-fixture');
      await expect(host).toHaveAttribute('data-state', 'ready');
      const frame = page.frameLocator('#fallback-fixture iframe');
      const before = await frame.locator('body').evaluate((body) => ({
        html: body.innerHTML,
        color: getComputedStyle(body.querySelector('strong')!).color,
        weight: getComputedStyle(body.querySelector('strong')!).fontWeight,
      }));
      await page.evaluate(() =>
        (
          window as unknown as { fallbackHandle: { highlight(anchors: unknown[]): void } }
        ).fallbackHandle.highlight([
          {
            id: 'one',
            ordinal: 1,
            selector: { exact: 'Check the mobile layout before release.', prefix: '', suffix: '' },
          },
          {
            id: 'two',
            ordinal: 2,
            selector: {
              exact:
                'Keep this long range wrapped over several lines on mobile, with its number placed on the final line.',
              prefix: '',
              suffix: '',
            },
          },
        ]),
      );
      await expect(frame.locator('[data-colab-highlight]:not([data-colab-markers])')).toHaveCount(
        1,
      );
      await expect(frame.locator('[data-colab-thread]')).toHaveText(['1', '2']);
      const after = await frame.locator('body').evaluate((body) => {
        const clone = body.cloneNode(true) as HTMLElement;
        clone.querySelectorAll('[data-colab-highlight]').forEach((node) => node.remove());
        return {
          html: clone.innerHTML,
          color: getComputedStyle(body.querySelector('strong')!).color,
          weight: getComputedStyle(body.querySelector('strong')!).fontWeight,
        };
      });
      expect(after).toEqual(before);
      mkdirSync(captureDirectory(), { recursive: true });
      await page.screenshot({
        path: `${captureDirectory()}/annotation-fallback-${width}-${theme}.png`,
      });
      await page.evaluate(() =>
        (
          window as unknown as { fallbackHandle: { highlight(anchors: unknown[]): void } }
        ).fallbackHandle.highlight(
          Array.from({ length: 12 }, (_, index) => ({
            id: `collision-${index + 1}`,
            ordinal: index + 1,
            selector: { exact: 'Check the mobile layout before release.', prefix: '', suffix: '' },
          })),
        ),
      );
      const markers = frame.locator('[data-colab-thread]');
      await expect(markers).toHaveCount(12);
      const boxes = await markers.evaluateAll((nodes) =>
        nodes.map((node) => node.getBoundingClientRect().toJSON()),
      );
      for (const [index, first] of boxes.entries()) {
        expect(first.width).toBe(first.height);
        for (const second of boxes.slice(index + 1))
          expect(
            first.right <= second.left ||
              second.right <= first.left ||
              first.bottom <= second.top ||
              second.bottom <= first.top,
          ).toBe(true);
      }
      for (let ordinal = 1; ordinal <= 12; ordinal++) {
        await markers.filter({ hasText: new RegExp(`^${ordinal}$`) }).click();
        await expect(host).toHaveAttribute('data-opened', `collision-${ordinal}`);
      }
      await page.screenshot({
        path: `${captureDirectory()}/annotation-collisions-${width}-${theme}.png`,
      });
    });
  }
