import { expect, test } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { text } from '../src/strings.js';

for (const width of [1586, 390]) {
  for (const colorScheme of ['light', 'dark'] as const) {
    test(`Source follows the website at ${width} ${colorScheme} without claiming persistence`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme });
      await page.goto('/');
      await page.evaluate(
        async (fixture) => (await import(fixture)).mount(),
        '/test/ask-page-browser.tsx',
      );
      await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
      await (await pageAction(page.locator('#ask-page-fixture'), 'Source')).click();
      const drawer = page.locator('.tmt-ui-drawer[data-panel="source"][open]');
      const save = drawer.getByRole('button', { name: text.save, exact: true });
      await expect(drawer.locator('header')).toContainText('Save');
      await expect(
        drawer.locator('header').getByRole('button', { name: text.save, exact: true }),
      ).toHaveCount(1);
      await expect(save).toHaveAttribute('data-variant', 'primary');
      await expect(save).toHaveText('Save');
      const editor = drawer.getByRole('textbox', { name: 'Page source', exact: true });
      await expect(editor).toBeVisible();
      await expect(drawer).toContainText('Changes remain in the draft until you save.');
      await expect(drawer.getByTestId('source-draft-status')).toHaveText('Draft unchanged');
      await editor.fill('<pre>Draft only: exact bytes</pre>');
      await expect(drawer.getByTestId('source-draft-status')).toHaveText('Draft changed');
      await expect(drawer.getByTestId('source-draft-status')).not.toContainText('Saved');
      await expect(editor).toHaveCSS('resize', 'vertical');
      const geometry = await drawer.evaluate((node) => {
        const body = node.querySelector('.tmt-ui-drawer-body')!;
        const field = node.querySelector('textarea')!;
        return {
          body: body.getBoundingClientRect().width,
          field: field.getBoundingClientRect().width,
          padding: parseFloat(getComputedStyle(body).paddingLeft),
        };
      });
      await drawer.getByRole('button', { name: 'Close Source', exact: true }).click();
      await (await pageAction(page.locator('#ask-page-fixture'), 'Source')).click();
      await expect(editor).toHaveValue('<pre>Draft only: exact bytes</pre>');
      await editor.fill('x'.repeat(1000));
      expect(await editor.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
      expect(geometry.padding).toBeGreaterThan(0);
      expect(geometry.field).toBeGreaterThan(geometry.body - geometry.padding * 2 - 2);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    });
  }
}
