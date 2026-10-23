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

// One view keeps the shard model's budget: the narrow dark one.
for (const width of [390]) {
  for (const theme of ['dark'] as const) {
    test(`choice rows are the shared row in one wrapped line, picked by a digit that no cap draws ${width} ${theme}`, async ({
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
      // The kind line and the instruction head the group.
      await expect(card.locator('.question-kind')).toHaveText('Multiple choice · Other available');
      await expect(card.locator('.tmt-ui-choices-legend')).toHaveText(
        'Choose any that apply, or write your own.',
      );
      // No row draws a key cap: the digit that picks a row is announced to assistive technology and
      // as the row control's hint, and the control is named by its label alone.
      await expect(card.locator('kbd')).toHaveCount(0);
      for (const [index, name] of ['Keep it', 'Change it', 'Other'].entries()) {
        const control = rows.nth(index).getByRole('checkbox', { name, exact: true });
        await expect(control).toHaveCount(1);
        await expect(control).toHaveAttribute('aria-keyshortcuts', String(index + 1));
        await expect(control).toHaveAttribute('title', `Press ${index + 1} to pick`);
      }
      // The old private row is gone.
      await expect(card.locator('.question-option, .question-option-key')).toHaveCount(0);
      // The Other field is not drawn until Other is picked; then it sits under the choices.
      const field = card.getByLabel('Your other answer', { exact: true });
      await expect(field).toHaveCount(0);
      await card.getByRole('checkbox', { name: 'Other', exact: true }).check();
      await expect(field).toBeVisible();
      await expect(field).toHaveAttribute('placeholder', 'Tell us what is missing…');
      const [fieldBox, otherBox] = [
        await field.boundingBox(),
        await card.getByRole('checkbox', { name: 'Other', exact: true }).boundingBox(),
      ];
      expect(fieldBox!.y).toBeGreaterThan(otherBox!.y + otherBox!.height);
      await field.fill('Something else');
      await card.getByRole('checkbox', { name: 'Keep it', exact: true }).check();
      await expect(card.getByRole('checkbox', { name: 'Keep it', exact: true })).toBeChecked();
      await expect(card.getByRole('checkbox', { name: 'Other', exact: true })).toBeChecked();
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
  await expect(card.getByLabel('Your other answer', { exact: true })).toBeVisible();
  // A digit also works while a choice has focus, and the focus follows the pick; in a text field
  // it is just a character.
  await card.getByRole('radio', { name: 'Change it', exact: true }).focus();
  await page.keyboard.press('1');
  await expect(card.getByRole('radio', { name: 'Keep it', exact: true })).toBeChecked();
  await expect(card.getByRole('radio', { name: 'Keep it', exact: true })).toBeFocused();
  await expect(card.getByLabel('Your other answer', { exact: true })).toHaveCount(0);
  await card.getByRole('radio', { name: 'Other', exact: true }).check();
  await card.getByLabel('Your other answer', { exact: true }).focus();
  await page.keyboard.type('2');
  await expect(card.getByLabel('Your other answer', { exact: true })).toHaveValue('2');
});
