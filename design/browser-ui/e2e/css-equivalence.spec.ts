import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
// @ts-expect-error The generator is a plain development script.
import { authoredStaticCss } from '../scripts/generate-static-css.mjs';

const shipped = readFileSync(new URL('../generated/static.css', import.meta.url), 'utf8');
for (const theme of ['light', 'dark'] as const) {
  test(`compiled CSS preserves source paint and layout (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await page.clock.install({ time: 0 });
    await page.clock.pauseAt(0);
    for (const [width, route] of [
      [1440, ''],
      [390, ''],
      [1440, '?frame=dialog'],
      [390, '?frame=dialog&presentation=confirm'],
      [1440, '?frame=drawer'],
      [390, '?frame=drawer'],
      [1440, '?frame=panel'],
      [1024, '?frame=panel'],
      [390, '?frame=panel'],
    ] as const) {
      await page.setViewportSize({ width, height: 992 });
      await page.goto(`/${route}${route ? '&' : '?'}theme=${theme}`);
      await expect(page.locator('#root')).not.toBeEmpty();
      await page.evaluate(() => document.fonts.ready);
      await page.clock.runFor(500);
      const differences = await page.evaluate(
        async ({ source, compiled }) => {
          const style = Array.from(
            document.querySelectorAll<HTMLStyleElement>('style[data-vite-dev-id]'),
          ).find((node) => node.dataset.viteDevId?.endsWith('/generated/static.css'));
          if (!style) throw new Error('Missing actual static stylesheet');
          const properties = [
            'display',
            'position',
            'width',
            'height',
            'minWidth',
            'maxWidth',
            'minHeight',
            'maxHeight',
            'top',
            'right',
            'bottom',
            'left',
            'color',
            'backgroundColor',
            'backgroundImage',
            'fontFamily',
            'fontSize',
            'fontWeight',
            'lineHeight',
            'letterSpacing',
            'textAlign',
            'textDecorationLine',
            'paddingTop',
            'paddingRight',
            'paddingBottom',
            'paddingLeft',
            'marginTop',
            'marginRight',
            'marginBottom',
            'marginLeft',
            'borderTopWidth',
            'borderRightWidth',
            'borderBottomWidth',
            'borderLeftWidth',
            'borderTopStyle',
            'borderRightStyle',
            'borderBottomStyle',
            'borderLeftStyle',
            'borderTopColor',
            'borderRightColor',
            'borderBottomColor',
            'borderLeftColor',
            'borderTopLeftRadius',
            'borderTopRightRadius',
            'borderBottomRightRadius',
            'borderBottomLeftRadius',
            'outlineStyle',
            'outlineWidth',
            'outlineColor',
            'outlineOffset',
            'boxShadow',
            'overflowX',
            'overflowY',
            'gap',
            'flexDirection',
            'gridTemplateColumns',
            'opacity',
            'visibility',
            'transform',
            'translate',
            'scale',
            'pointerEvents',
            'backdropFilter',
            'fill',
            'stroke',
          ];
          const canvas = document.createElement('canvas');
          canvas.width = canvas.height = 1;
          const context = canvas.getContext('2d')!;
          function normalize(property: string, value: string) {
            if (
              (property === 'color' ||
                property.endsWith('Color') ||
                property === 'fill' ||
                property === 'stroke') &&
              !/url\(|none/.test(value)
            ) {
              context.clearRect(0, 0, 1, 1);
              context.fillStyle = '#0000';
              context.fillStyle = value;
              context.fillRect(0, 0, 1, 1);
              return Array.from(context.getImageData(0, 0, 1, 1).data).join(',');
            }
            return value;
          }
          async function snapshot(css: string) {
            style!.textContent = css;
            // getComputedStyle flushes this stylesheet synchronously; the frozen clock
            // keeps busy-delay and animation state identical for both snapshots.
            const rows: string[][] = [];
            for (const node of document.querySelectorAll('[class*="tmt-ui-"], [data-tmt-ui] *')) {
              for (const pseudo of [
                null,
                '::before',
                '::after',
                ...(node.tagName === 'DIALOG' ? ['::backdrop'] : []),
              ]) {
                const computed = getComputedStyle(node, pseudo);
                if (pseudo && pseudo !== '::backdrop' && computed.content === 'none') continue;
                rows.push([
                  `${node.tagName}.${node.getAttribute('class')}${pseudo ?? ''}`,
                  ...properties.map((property) =>
                    normalize(property, computed[property as keyof CSSStyleDeclaration] as string),
                  ),
                ]);
              }
            }
            return rows;
          }
          const before = await snapshot(source);
          const after = await snapshot(compiled);
          const out: string[] = [];
          if (before.length !== after.length)
            out.push(`DOM row count ${before.length} != ${after.length}`);
          for (let i = 0; i < Math.min(before.length, after.length); i++) {
            for (let j = 1; j < before[i]!.length; j++)
              if (before[i]![j] !== after[i]![j]) {
                out.push(
                  `${before[i]![0]} ${properties[j - 1]}: ${before[i]![j]} != ${after[i]![j]}`,
                );
                if (out.length >= 12) return out;
              }
          }
          return out;
        },
        { source: authoredStaticCss, compiled: shipped },
      );
      expect(differences, `${width}px ${route || 'full gallery'}`).toEqual([]);
    }
  });
}
