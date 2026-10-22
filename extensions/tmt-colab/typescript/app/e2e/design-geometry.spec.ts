import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

/** Busy-mark centring and the chip remove segment, measured on the real design-system CSS. */
async function mount(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const path = '/test/design-geometry-browser.tsx';
    await (await import(path)).mountDesignGeometry();
  });
  // A busy mark shows only after the busy delay.
  await expect(page.locator('#geometry-fixture .tmt-ui-action-mark[data-busy="true"]')).toHaveCount(
    6,
  );
}

test('a busy action centres its mark on the label for every variant and theme', async ({
  page,
}) => {
  await mount(page);
  const offsets = await page.$$eval('#geometry-fixture section > .tmt-ui-action', (actions) =>
    actions.map((action) => {
      const centre = (element: Element) => {
        const box = element.getBoundingClientRect();
        return box.top + box.height / 2;
      };
      const mark = action.querySelector('.tmt-ui-action-mark > .tmt-ui-busy-mark')!;
      const label = action.querySelector('.tmt-ui-action-label')!;
      return {
        name: action.textContent,
        variant: action.getAttribute('data-variant'),
        offset: Math.abs(centre(mark) - centre(label)),
      };
    }),
  );
  expect(offsets.map((entry) => entry.variant).sort()).toEqual([
    'destructive',
    'destructive',
    'primary',
    'primary',
    'text',
    'text',
  ]);
  for (const entry of offsets) expect(entry.offset, entry.name ?? '').toBeLessThanOrEqual(0.5);
});

test('a chip remove control is a segment flush with the chip inner edges', async ({ page }) => {
  await mount(page);
  const chips = await page.$$eval(
    '#geometry-fixture .tmt-ui-chip, #geometry-fixture [data-slot="badge"]',
    (chips) =>
      chips.map((chip) => {
        const outer = chip.getBoundingClientRect();
        const inner = {
          top: outer.top + chip.clientTop,
          right: outer.left + chip.clientLeft + chip.clientWidth,
          bottom: outer.top + chip.clientTop + chip.clientHeight,
        };
        const remove = chip.querySelector('button')!.getBoundingClientRect();
        return {
          name: chip.textContent,
          top: Math.abs(remove.top - inner.top),
          right: Math.abs(remove.right - inner.right),
          bottom: Math.abs(remove.bottom - inner.bottom),
        };
      }),
  );
  expect(chips).toHaveLength(4);
  for (const chip of chips) {
    expect(chip.top, `${chip.name} top`).toBeLessThanOrEqual(0.5);
    expect(chip.right, `${chip.name} right`).toBeLessThanOrEqual(0.5);
    expect(chip.bottom, `${chip.name} bottom`).toBeLessThanOrEqual(0.5);
  }
});

/** The tab icon at 16/24/32 px and the full mark at 256 px, rasterised from the generated files
 * that `generate-brand.mjs` serialises from the one mark definition (its `--check` keeps them equal). */
test('the generated favicon and the full mark render the aurora beam, the dark panel and the prompt', async ({
  page,
}) => {
  const read = (name: string) =>
    `data:image/svg+xml;base64,${readFileSync(
      new URL(`../../../../../design/browser-ui/generated/${name}`, import.meta.url),
    ).toString('base64')}`;
  await page.goto('/');
  const renders = await page.evaluate(
    async (sources) => {
      const out: Record<string, { beam: number; dark: number; light: number }> = {};
      for (const [name, src, size] of sources) {
        const image = new Image();
        image.src = src;
        await image.decode();
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = size;
        const context = canvas.getContext('2d')!;
        context.drawImage(image, 0, 0, size, size);
        const data = context.getImageData(0, 0, size, size).data;
        let beam = 0;
        let dark = 0;
        let light = 0;
        for (let i = 0; i < data.length; i += 4) {
          const [r, g, b, a] = [data[i]!, data[i + 1]!, data[i + 2]!, data[i + 3]!];
          if (a < 200) continue;
          const max = Math.max(r, g, b);
          const min = Math.min(r, g, b);
          if (max > 150 && (max - min) / max > 0.35 && b + g > 2 * r) beam++;
          else if (max < 60) dark++;
          else if (min > 220) light++;
        }
        out[name] = { beam, dark, light };
      }
      return out;
    },
    [
      ['favicon-16', read('favicon.svg'), 16],
      ['favicon-24', read('favicon.svg'), 24],
      ['favicon-32', read('favicon.svg'), 32],
      ['mark-256', read('app-icon-light.svg'), 256],
    ] as [string, string, number][],
  );
  for (const [name, { beam, dark, light }] of Object.entries(renders)) {
    const size = Number(name.split('-')[1]);
    // A visible aurora beam, a dark panel and tile, and a white prompt at every size.
    expect(beam, name).toBeGreaterThan(size * size * 0.04);
    expect(dark, name).toBeGreaterThan(size * size * 0.3);
    expect(light, name).toBeGreaterThan(0);
  }
});
