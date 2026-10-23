import { expect, test } from '@playwright/test';
import { capturePath } from './captures.js';

for (const theme of ['light', 'dark'] as const)
  for (const width of [1440, 390])
    test(`shared control frames retain strong state and focus: ${theme} ${width}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((theme) => {
        document.documentElement.dataset.theme = theme;
      }, theme);
      await page.goto('/');
      await page.evaluate(async () => {
        const path = '/test/control-frames-browser.tsx';
        (await import(path)).mountControlFrames();
      });
      const fixture = page.locator('#control-frames');
      const composer = fixture.getByRole('combobox', { name: 'Colab composer' });
      await expect(composer).toBeVisible();
      // The composer focuses itself on mount; inspect resting frames after leaving it.
      await fixture.getByRole('button', { name: 'Action', exact: true }).focus();
      await expect(composer).not.toBeFocused();
      // A focused control recolours its own border (#3998), so the resting frames are read with
      // nothing focused.
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
      const color = (token: string) =>
        fixture.evaluate((node, token) => {
          const probe = document.createElement('span');
          probe.style.color = `var(${token})`;
          node.append(probe);
          const value = getComputedStyle(probe).color;
          probe.remove();
          return value;
        }, token);
      const rule = await color('--tmt-ui-surface-rule');
      const edge = await color('--tmt-ui-color-edge');
      const focus = await color('--tmt-ui-color-focus');
      const accentEdge = await color('--tmt-ui-color-accent-strong');
      expect(rule).not.toBe(edge);
      const frames: { selector: string; color: string; enabledPrimary?: boolean }[] = [];
      await page.screenshot({
        path: capturePath(`3299-controls-${width}-${theme}.png`),
        fullPage: true,
      });
      for (const selector of [
        '.tmt-ui-action[data-variant="primary"]',
        '.tmt-ui-action[data-variant="secondary"]',
        '.tmt-ui-field-control:not(.rich-input-field)',
        '.tmt-ui-toggle',
        '.tmt-ui-chip',
        '.tmt-ui-chips-add',
        '.tmt-ui-command',
        '.tmt-ui-command-copy',
        '.rich-input-surface[data-accessory]',
      ]) {
        const controls = fixture.locator(selector);
        expect(await controls.count(), selector).toBeGreaterThan(0);
        for (const control of await controls.all()) {
          await expect(control).toHaveCSS('border-top-width', '1px');
          frames.push({
            selector,
            color: await control.evaluate((node) => getComputedStyle(node).borderTopColor),
            enabledPrimary: selector.includes('primary') && (await control.isEnabled()),
          });
        }
      }
      for (const indicator of await fixture.locator('.tmt-ui-toggle-indicator').all())
        await expect(indicator).toHaveCSS('border-top-color', edge);
      const field = fixture.getByRole('textbox', { name: 'Shared field' });
      const before = await field.boundingBox();
      await field.focus();
      await field.press('Tab');
      await page.keyboard.press('Shift+Tab');
      await expect(field).toHaveCSS('border-top-color', focus);
      // Focus recolours the field's own edge; it draws no second outline (one frame).
      await expect(field).toHaveCSS('border-top-width', '1px');
      await expect(field).toHaveCSS('outline-style', 'none');
      await expect(field).toHaveCSS('outline-width', '0px');
      expect(await field.boundingBox()).toEqual(before);
      await page.screenshot({
        path: capturePath(`3299-focus-${width}-${theme}.png`),
        fullPage: true,
      });
      await fixture.getByRole('button', { name: 'Add label', exact: true }).click();
      const popup = page.locator('.tmt-ui-chips-surface');
      await expect(popup).toBeVisible();
      frames.push({
        selector: '.tmt-ui-chips-surface',
        color: await popup.evaluate((node) => getComputedStyle(node).borderTopColor),
      });
      await page.screenshot({
        path: capturePath(`3299-chips-${width}-${theme}.png`),
        fullPage: true,
      });
      await page.keyboard.press('Escape');
      await fixture.getByRole('button', { name: 'Icon action', exact: true }).hover();
      const tooltip = page.locator('.tmt-ui-icon-action-tooltip-label:visible');
      await expect(tooltip).toHaveText('Icon action');
      frames.push({
        selector: '.tmt-ui-icon-action-tooltip-label',
        color: await tooltip.evaluate((node) => getComputedStyle(node).borderTopColor),
      });
      await page.screenshot({
        path: capturePath(`3299-tooltip-${width}-${theme}.png`),
        fullPage: true,
      });
      // An enabled primary action is accent-filled and draws its edge in the strong accent; every
      // other frame, a disabled primary included, is the rule.
      for (const frame of frames)
        expect(frame.color, frame.selector).toBe(frame.enabledPrimary ? accentEdge : rule);
    });
