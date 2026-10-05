import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
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
test('an accepted annotation stops saying awaiting at the observation deadline without another send', async ({
  page,
}) => {
  const input = await mount(page, 'accepted');
  await page.clock.install();
  await input.fill('@Deterministic agent Explain this.');
  await input.press('Enter');
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
  await expect(page.getByTestId('ask-state')).toHaveText('waiting');
  const before = await run(page, 'proof');
  expect(before.sends).toHaveLength(1);
  await page.clock.fastForward(2 * 60 * 60 * 1000 + 1);
  await expect(page.getByTestId('ask-state')).toHaveText('no reply yet');
  await expect(page.getByTestId('ask-state').locator('svg.lucide-clock')).toBeVisible();
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
  await expect(page.getByRole('button', { name: 'Re-check delivery', exact: true })).toBeEnabled();
  expect(await run(page, 'proof')).toEqual(before);
  const directory = process.env.COLAB_1699_CAPTURE_DIR;
  if (directory) {
    mkdirSync(directory, { recursive: true });
    for (const theme of ['light', 'dark']) {
      await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        await page.screenshot({ path: `${directory}/timeout-${width}-${theme}.png` });
      }
    }
  }
});
test('each accepted annotation reaches its own deadline while the conversation stays mounted', async ({
  page,
}) => {
  const input = await mount(page, 'accepted');
  await page.clock.install();
  await input.fill('@Deterministic agent First question.');
  await input.press('Enter');
  await expect(page.getByTestId('ask-state')).toHaveCount(1);
  await page.clock.fastForward(60 * 60 * 1000);
  await input.fill('@Deterministic agent Second question.');
  await input.press('Enter');
  const status = page.getByTestId('ask-state');
  await expect(status).toHaveCount(2);
  const before = await run(page, 'proof');
  expect(before.preparations).toBe(2);
  await page.clock.fastForward(60 * 60 * 1000 + 1);
  await expect(status).toHaveText(['no reply yet', 'waiting']);
  await page.clock.fastForward(60 * 60 * 1000);
  await expect(status).toHaveText(['no reply yet', 'no reply yet']);
  expect(await run(page, 'proof')).toEqual(before);
});
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
  await expect(page.locator('.annotation-compose details')).toHaveCount(0);
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
  expect(result.sends[0].message).toContain('Explain this.\nMore detail.');
  await expect(page.getByTestId('ask-entry').locator('.comment-byline')).toContainText(
    'Fixture browser ·',
  );
  await expect(page.getByTestId('ask-entry').getByTestId('ask-state')).toContainText('held');
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

test('input arrows reopen the shared list, skip disabled recipients and retain explicit Enter sends', async ({
  page,
}) => {
  const input = await mount(page, 'multi');
  await input.fill('@');
  await input.press('Escape');
  await expect(input).toHaveAttribute('aria-expanded', 'false');
  await input.press('ArrowDown');
  await expect(input).toHaveAttribute('aria-expanded', 'true');
  await input.press('ArrowDown');
  await expect(input).toHaveAttribute('aria-activedescendant', /option-2$/);
  await input.press('ArrowUp');
  await expect(input).toHaveAttribute('aria-activedescendant', /option-0$/);
  await input.press('End');
  await input.press('Enter');
  await expect(input).toHaveValue('@Other agent ');
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await input.fill('@Deterministic agent Explain this.');
  await input.press('Enter');
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'held');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});
