import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, mode = 'held') {
  await page.goto('/');
  await run(page, 'mount', mode);
  const input = page.getByRole('combobox', { name: 'Message to agent', exact: true });
  await expect(input).toHaveValue('@Deterministic agent ');
  return input;
}
test('the shared input listbox consumes recipient Enter; explicit message Enter sends once and held stays inline', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('@');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('End');
  await expect(input).toHaveAttribute('aria-activedescendant', /option-0$/);
  await input.press('Enter');
  await expect(input).toHaveValue('@Deterministic agent ');
  expect((await run(page, 'proof')).writes).toBe(0);
  await input.fill('@Deterministic agent Explain this.');
  await input.press('Shift+Enter');
  await expect(input).toHaveValue('@Deterministic agent Explain this.\n');
  await input.type('More detail.');
  await page.getByText('Show exactly what is sent', { exact: true }).click();
  const exact = await page.getByTestId('annotation-exact-bytes').textContent();
  await input.evaluate((node) =>
    node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })),
  );
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await input.press('Enter');
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'held');
  const result = await run(page, 'proof');
  expect(result.writes).toBe(1);
  expect(result.preparations).toBe(1);
  expect(result.commits).toBe(1);
  expect(result.sends).toHaveLength(1);
  expect(`[remote: Fixture browser]\n${result.sends[0].message}`).toBe(exact);
  await expect(page.getByTestId('ask-entry')).toContainText('Deterministic agent');
  await expect(page.getByTestId('ask-preview')).toHaveCount(0);
  await expect(page.locator('dialog')).toHaveCount(0);
});
test('Esc first closes autocomplete, then cancels without recording or sending', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('@');
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await expect(input).toBeVisible();
  await input.press('Escape');
  await expect(input).toHaveCount(0);
  expect(await run(page, 'proof')).toEqual({
    writes: 0,
    preparations: 0,
    closes: 1,
    commits: 0,
    sends: [],
  });
});
test('a saved turn whose preparation fails keeps its inline error and cannot silently create another turn', async ({
  page,
}) => {
  const input = await mount(page, 'prepare-failure');
  await input.fill('@Deterministic agent Explain this.');
  await input.press('Enter');
  await expect(page.getByRole('alert')).toContainText(
    'turn was recorded, but delivery is unavailable or uncertain',
  );
  await expect(input).toBeDisabled();
  expect(await run(page, 'proof')).toEqual({
    writes: 1,
    preparations: 1,
    closes: 0,
    commits: 0,
    sends: [],
  });
  await page.getByRole('button', { name: 'Open recorded thread', exact: true }).click();
  expect((await run(page, 'proof')).commits).toBe(1);
  expect((await run(page, 'proof')).sends).toHaveLength(0);
});
