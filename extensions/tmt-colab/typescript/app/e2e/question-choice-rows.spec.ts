import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { pageAction } from '../test/page-actions.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, question: object) {
  await page.goto('/');
  await run(page, 'mount', {});
  await run(page, 'question', question);
  await expect(page.locator('[data-inline-question]')).not.toHaveAttribute('data-detached');
  const card = page.locator('.question-group-card');
  await card.locator('.question-line').click();
  return card;
}

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`choice rows are the shared row with a digit key hint ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      const card = await mount(page, {
        kind: 'multiple',
        choices: ['Keep it', 'Change it'],
        other: true,
      });
      if (theme === 'dark')
        await (await pageAction(page.locator('#ask-page-fixture'), 'Theme: Dark')).click();
      const rows = card.locator('.tmt-ui-choice');
      await expect(rows).toHaveCount(3);
      // Each row ends with its digit as a key cap that assistive technology skips.
      for (const [index, name] of ['Keep it', 'Change it', 'Other'].entries()) {
        const row = rows.nth(index);
        const key = row.locator('kbd.tmt-ui-choice-hint');
        await expect(key).toHaveText(String(index + 1));
        await expect(key).toHaveAttribute('aria-hidden', 'true');
        // The control is named by its label alone, with no digit in the name.
        await expect(row.getByRole('checkbox', { name, exact: true })).toHaveCount(1);
        const [rowBox, keyBox] = [await row.boundingBox(), await key.boundingBox()];
        // The key sits at the row's end and the row text does not run under it.
        expect(keyBox!.x + keyBox!.width).toBeLessThanOrEqual(rowBox!.x + rowBox!.width + 0.5);
        const text = await row.locator('.tmt-ui-choice-text').boundingBox();
        expect(text!.x + text!.width).toBeLessThanOrEqual(keyBox!.x + 0.5);
      }
      // The old private row is gone.
      await expect(card.locator('.question-option, .question-option-key')).toHaveCount(0);
      // The Other row opens its text field; the other rows still pick as before.
      await expect(card.getByLabel('Other answer', { exact: true })).toHaveCount(0);
      await card.getByRole('checkbox', { name: 'Other', exact: true }).check();
      await expect(card.getByLabel('Other answer', { exact: true })).toBeVisible();
      await card.getByRole('checkbox', { name: 'Keep it', exact: true }).check();
      await expect(card.getByRole('checkbox', { name: 'Keep it', exact: true })).toBeChecked();
      await page.screenshot({ path: capturePath(`question-rows-${width}-${theme}.png`) });
    });
  }
}

test('a single question picks its row with the digit and the Other row clears the choice', async ({
  page,
}) => {
  const card = await mount(page, {
    kind: 'single',
    choices: ['Keep it', 'Change it'],
    other: true,
  });
  await card.locator('.question-card:visible').first().focus();
  await page.keyboard.press('2');
  await expect(card.getByRole('radio', { name: 'Change it', exact: true })).toBeChecked();
  await page.keyboard.press('3');
  await expect(card.getByRole('radio', { name: 'Other', exact: true })).toBeChecked();
  await expect(card.getByRole('radio', { name: 'Change it', exact: true })).not.toBeChecked();
  await expect(card.getByLabel('Other answer', { exact: true })).toBeVisible();
});
