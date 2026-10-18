import { expect, test } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const surface of ['annotation', 'chat'] as const) {
      test(`conversation scale: ${surface} ${width}px ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme });
        await page.addInitScript((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        await page.goto('/');
        await page.evaluate(async (surface) => {
          const fixture = '/test/conversation-scale-browser.tsx';
          (await import(fixture)).mount(surface);
        }, surface);
        const window = page.getByTestId(surface === 'annotation' ? 'comment-thread' : 'chat-panel');
        await expect(window.getByRole('combobox', { name: 'Message', exact: true })).toContainText(
          'Please check the spacing.',
        );
        await expect(window.locator('.agent-chip[data-variant="chip"]')).toHaveCount(1);
        await expect(window.locator('[data-turn-role="agent"]')).toHaveCount(1);
        await page.evaluate(() => document.fonts.ready);
        const metrics = await window.evaluate((window) => {
          const selectors = {
            header: '.conversation-window-bar',
            title: '.conversation-title',
            human: '[data-turn-role="user"]',
            message: '[data-turn-role="user"] .conversation-body',
            agent: '[data-turn-role="agent"]',
            agentMessage: '[data-turn-role="agent"] .conversation-body',
            byline: '.comment-byline',
            composer: '.conversation-composer',
            editor: '.rich-input-field',
            mention: '.agent-chip[data-variant="chip"]',
            status: '.annotation-status-row',
            hint: '.annotation-hint',
            anchor: '.annotation-anchor-note',
          };
          return Object.fromEntries(
            Object.entries(selectors).map(([name, selector]) => {
              const element = window.querySelector(selector);
              if (!element) return [name, null];
              const style = getComputedStyle(element);
              return [
                name,
                Object.fromEntries(
                  [
                    'fontSize',
                    'lineHeight',
                    'padding',
                    'margin',
                    'gap',
                    'minHeight',
                    'maxHeight',
                  ].map((property) => [property, style[property as keyof CSSStyleDeclaration]]),
                ),
              ];
            }),
          );
        });
        const pill = await page.getByTestId('scale-annotate').evaluate((element) => {
          const style = getComputedStyle(element);
          return {
            fontSize: style.fontSize,
            lineHeight: style.lineHeight,
            padding: style.padding,
            gap: style.gap,
          };
        });
        const directory = process.env.COLAB_CAPTURE_DIR;
        if (directory) {
          await mkdir(directory, { recursive: true });
          const name = `${surface}-${width}-${theme}`;
          await writeFile(
            join(directory, `${name}.json`),
            JSON.stringify({ width, theme, surface, metrics, pill }, null, 2),
          );
          await page.screenshot({ path: join(directory, `${name}.png`), fullPage: true });
        }
        {
          await expect(window.locator('.rich-input-field')).toHaveCSS('font-size', '14px');
          await expect(window.locator('.rich-input-field')).toHaveCSS('line-height', '21px');
          await expect(window.locator('.conversation-body').first()).toHaveCSS('font-size', '14px');
          await expect(window.locator('[data-turn-role="agent"] .conversation-body')).toHaveCSS(
            'font-size',
            '14px',
          );
          // The bordered chip is caption size; its text beside it stays at the message size.
          await expect(window.locator('.agent-chip[data-variant="chip"]')).toHaveCSS(
            'font-size',
            '11px',
          );
          await expect(window.locator('.comment-byline').first()).toHaveCSS('font-size', '11px');
          await expect(window.locator('.comment-byline').first()).toHaveCSS(
            'line-height',
            '16.5px',
          );
          await expect(window.locator('.conversation-composer')).toHaveCSS(
            'padding',
            '12px 16px 16px',
          );
          if (surface === 'annotation') {
            await expect(window.locator('.annotation-anchor-note')).toHaveCSS('font-size', '12px');
            await expect(window.locator('.conversation-window-bar')).toHaveCSS(
              'padding',
              '8px 16px',
            );
          }
          await page.evaluate(async () => {
            const fixture = '/test/conversation-scale-browser.tsx';
            (await import(fixture)).mountInline();
          });
          await expect(page.locator('#scale-fixture .conversation-turn')).toHaveCSS(
            'padding',
            '8px 16px 8px 13px',
          );
        }
      });
    }
