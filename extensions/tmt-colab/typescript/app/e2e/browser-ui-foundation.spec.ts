import { test, expect, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
const tokens = JSON.parse(
  readFileSync(new URL('../../../../../design/tokens/tokens.json', import.meta.url), 'utf8'),
) as {
  browser: {
    color: Record<string, Record<'light' | 'dark', string>>;
    surface: Record<string, Record<'light' | 'dark', string>>;
    metric: Record<string, string>;
    ui: Record<string, string>;
  };
};

async function oldStyles(page: Page) {
  return page
    .locator('#legacy [class*="tmt-ui"], #outside, #unscoped-input, h2')
    .evaluateAll((nodes) =>
      nodes.map((node) => {
        const style = getComputedStyle(node);
        return Object.fromEntries(
          [
            'color',
            'background-color',
            'font-family',
            'font-size',
            'padding',
            'margin',
            'border',
            'border-radius',
            'box-shadow',
            'outline',
            'display',
            'box-sizing',
          ].map((key) => [key, style.getPropertyValue(key)]),
        );
      }),
    );
}
function rgb(hex: string) {
  return `rgb(${[1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16)).join(', ')})`;
}
async function rootVariables(page: Page) {
  return page.locator('html').evaluate((node) => {
    const style = getComputedStyle(node);
    return Object.fromEntries(
      Array.from(style)
        .filter((key) => key.startsWith('--tmt-ui-'))
        .map((key) => [key, style.getPropertyValue(key)]),
    );
  });
}
for (const width of [320, 390, 1440]) {
  for (const system of ['light', 'dark'] as const) {
    test(`foundation islands and portals ${width} system-${system}`, async ({ page }, testInfo) => {
      const errors: string[] = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: system });
      await page.goto('/test/browser-ui-foundation.html');
      await expect(
        page.getByRole('heading', {
          name: 'Slice1 infrastructure only; no shadcn components implemented',
        }),
      ).toBeVisible();
      const baseline = await oldStyles(page);
      const variables = await rootVariables(page);
      await page.getByRole('button', { name: 'Load isolated UI CSS' }).click();
      await expect(page.locator('[data-css-loaded=true]')).toHaveText('UI CSS loaded');
      expect(await oldStyles(page)).toEqual(baseline);
      expect(await rootVariables(page)).toEqual(variables);
      for (const outer of ['light', 'dark', 'system'] as const) {
        await page.getByLabel('Outer theme').selectOption(outer);
        const themes = {
          outer: outer === 'system' ? system : outer,
          'nested-light': 'light',
          'nested-dark': 'dark',
          'nested-system': system,
        } as const;
        for (const [name, theme] of Object.entries(themes)) {
          for (const suffix of ['', '-portal']) {
            const probe = page.locator(`[data-probe="${name}${suffix}"]`);
            await expect(probe).toHaveCSS(
              'background-color',
              rgb(tokens.browser.surface.paper[theme]),
            );
            await expect(probe).toHaveCSS('color', rgb(tokens.browser.color.text[theme]));
            await expect(probe).toHaveCSS('border-color', rgb(tokens.browser.color.edge[theme]));
            await expect(probe).toHaveCSS('padding', '8px');
            await expect(probe).toHaveCSS('border-width', tokens.browser.metric['rule-width']);
            await expect(probe).toHaveCSS('border-radius', '0px');
            await expect(probe).toHaveCSS('box-shadow', 'none');
            await expect(probe).toHaveCSS('z-index', tokens.browser.ui['z-layer']);
            if (suffix)
              expect(
                await probe.evaluate(
                  (node) =>
                    !!node.closest('[data-tmt-ui-portal]') && !!node.closest('[data-tmt-ui]'),
                ),
              ).toBe(true);
          }
        }
      }
      const focus = page.locator('[data-probe="outer"]');
      await focus.focus();
      await expect(focus).toHaveCSS('outline-width', tokens.browser.metric['focus-width']);
      await expect(focus).toHaveCSS('outline-color', rgb(tokens.browser.color.focus[system]));
      for (const selector of ['html', '#outside', '#unscoped-input'])
        expect(
          await page
            .locator(selector)
            .evaluate((node) =>
              getComputedStyle(node).getPropertyValue('--tmt-foundation-background'),
            ),
        ).toBe('');
      expect(await oldStyles(page)).toEqual(baseline);
      await page.screenshot({
        path: testInfo.outputPath(`foundation-${width}-${system}.png`),
        fullPage: true,
      });
      expect(errors).toEqual([]);
    });
  }
}
