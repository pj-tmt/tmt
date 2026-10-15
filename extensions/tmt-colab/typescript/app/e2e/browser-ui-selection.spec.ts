import { test, expect, type Page } from '@playwright/test';
async function gallery(page: Page) {
  await page.goto('/test/browser-ui-foundation.html');
  await page.getByRole('button', { name: 'Load isolated UI CSS' }).click();
  await expect(page.locator('[data-css-loaded=true]')).toHaveText('UI CSS loaded');
  await expect(page.getByRole('heading', { name: 'Slice4 selection qualification' })).toBeVisible();
}
test('original Enter and pointer selection each commit once without Send', async ({ page }) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Enter');
  await expect(page.getByLabel('Selection value', { exact: true })).toHaveText('alpha');
  await expect(page.getByLabel('Selection callback count')).toHaveText('1');
  await expect(page.getByLabel('Selection original event')).toHaveText('keydown:true:true');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
  await input.press('ArrowDown');
  await page.getByRole('option', { name: 'Beta', exact: true }).click();
  await expect(page.getByLabel('Selection value', { exact: true })).toHaveText('beta');
  await expect(page.getByLabel('Selection callback count')).toHaveText('2');
  await expect(page.getByLabel('Selection original event')).toHaveText('click:true:true');
  await expect(page.getByLabel('Selection send count')).toHaveText('0');
});
test('Tab and reverse Tab admit once, preserve input identity and continue native focus', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.evaluate((node) => node.setAttribute('data-retained-node', 'yes'));
  await input.focus();
  await expect(page.getByRole('option', { name: 'Alpha', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await input.press('Tab');
  await expect(page.getByRole('button', { name: 'After selection', exact: true })).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toHaveAttribute('data-retained-node', 'yes');
  await expect(page.getByLabel('Selection callback count')).toHaveText('1');
  await expect(page.getByLabel('Selection original event')).toHaveText('keydown:true:true');
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('Shift+Tab');
  await expect(page.getByRole('button', { name: 'Before selection', exact: true })).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('2');
  await expect(input).toHaveAttribute('data-retained-node', 'yes');
});
test('Tab close veto preserves actual open state and makes no selection callback', async ({
  page,
}) => {
  await gallery(page);
  await page.getByRole('button', { name: 'Close veto off', exact: true }).click();
  const input = page.getByRole('combobox', { name: 'Selection search' });
  await input.focus();
  await input.press('Tab');
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
  await input.press('Shift+Tab');
  await expect(input).toBeFocused();
  await expect(page.getByLabel('Selection callback count')).toHaveText('0');
});

test('combobox outside pointer admits the underlying action and keeps its native focus', async ({
  page,
}) => {
  await gallery(page);
  await page.getByRole('combobox', { name: 'Selection search' }).focus();
  const outside = page.getByRole('button', { name: 'Selection outside action', exact: true });
  await outside.click();
  await expect(page.getByLabel('Selection outside count')).toHaveText('1');
  await expect(outside).toBeFocused();
  await expect(page.getByRole('combobox', { name: 'Selection search' })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});
test('chips first outside pointer dismisses only; a subsequent press activates the action', async ({
  page,
}) => {
  await gallery(page);
  const input = page.getByRole('combobox', { name: 'Chips search' });
  await input.focus();
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  const outside = page.getByRole('button', { name: 'Chips outside action', exact: true });
  await outside.click();
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByLabel('Chips outside count')).toHaveText('0');
  await outside.click();
  await expect(page.getByLabel('Chips outside count')).toHaveText('1');
});
