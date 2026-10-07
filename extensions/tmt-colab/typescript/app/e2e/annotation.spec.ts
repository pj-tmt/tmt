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
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('', { useInnerText: true });
  return input;
}
test('an accepted annotation stops saying awaiting at the observation deadline without another send', async ({
  page,
}) => {
  const input = await mount(page, 'accepted');
  await page.clock.install();
  await input.fill('@Deterministic agent Explain this.');
  await page.getByRole('button', { name: 'Choose recipient', exact: true }).click();
  await input.press('Enter');
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
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
  await page.getByRole('button', { name: 'Choose recipient', exact: true }).click();
  await input.press('Enter');
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
  await expect(page.getByTestId('ask-state')).toHaveCount(1);
  await page.clock.fastForward(60 * 60 * 1000);
  await input.fill('Second question.');
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
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
  await expect(input).toHaveText('@Deterministic agent ', { useInnerText: true });
  expect((await run(page, 'proof')).writes).toBe(0);
  await input.fill('@Deterministic agent Explain this.');
  await input.press('Shift+Enter');
  await expect(input).toHaveText('@Deterministic agent Explain this.\n', { useInnerText: true });
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
  await page.getByRole('button', { name: 'Choose recipient', exact: true }).click();
  await input.press('Enter');
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText(
    'turn was recorded, but delivery is unavailable or uncertain',
  );
  await expect(input).toHaveAttribute('contenteditable', 'false');
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
  await expect(input).toHaveText('@Other agent ', { useInnerText: true });
  expect((await run(page, 'proof')).sends).toHaveLength(0);
  await input.fill('@Deterministic agent Explain this.');
  await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'held');
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});

test('failed discovery still permits one admitted plain comment without preparing an Ask', async ({
  page,
}) => {
  const input = await mount(page, 'discovery-failure');
  await expect(page.getByRole('status')).toContainText('Agents are unavailable');
  await input.fill('A plain note.\n\nKeep this line.');
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  expect(await run(page, 'proof')).toEqual({
    writes: 1,
    preparations: 0,
    closes: 0,
    commits: 1,
    sends: [],
  });
  const editing = await run(page, 'editingProof');
  expect(editing.captured).toEqual([
    {
      body: 'A plain note.\n\nKeep this line.',
      anchor: { exact: 'Selected text', prefix: '', suffix: '' },
    },
  ]);
});

test('explicit Ask uses exact no-prefix bytes once, while candidate selection alone has no effects', async ({
  page,
}) => {
  const input = await mount(page);
  await input.fill('Explain this.\n\nKeep 😀 and this trailing line.\n');
  await input.press('ArrowDown');
  await input.press('Enter');
  expect(await run(page, 'proof')).toEqual({
    writes: 0,
    preparations: 0,
    closes: 0,
    commits: 0,
    sends: [],
  });
  const before = await run(page, 'editingProof');
  expect(before.draft).toBe('Explain this.\n\nKeep 😀 and this trailing line.\n');
  await input.press('Enter');
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'held');
  const proof = await run(page, 'proof');
  expect(proof.writes).toBe(1);
  expect(proof.preparations).toBe(1);
  expect(proof.sends).toHaveLength(1);
  expect(proof.sends[0].message).toContain(before.draft);
  const after = await run(page, 'editingProof');
  expect(after.captured[0].body).toBe(before.draft);
  expect(after.captured[1].comment).toBe(before.draft);
  expect(after.captured[1].destination.agent).toBe('00000000-0000-4000-8000-000000000006');
});

test('a refused content write preserves exact draft and never prepares or dispatches', async ({
  page,
}) => {
  const input = await mount(page, 'write-failure');
  await input.fill('Keep this unsent note.');
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveText('The turn could not be recorded.');
  expect((await run(page, 'editingProof')).draft).toBe('Keep this unsent note.');
  expect(await run(page, 'proof')).toEqual({
    writes: 1,
    preparations: 0,
    closes: 0,
    commits: 0,
    sends: [],
  });
});

test('the shared window awaits one admitted status action without replacing its draft or quote', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountWindow');
  const window = page.getByTestId('comment-thread');
  const input = page.getByRole('combobox', { name: 'Window draft', exact: true });
  await input.focus();
  await input.evaluate((node) => {
    (node as HTMLDivElement).dataset.retained = 'yes';
  });
  await window.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(window.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
  await expect(window.getByRole('button', { name: 'Close thread', exact: true })).toBeDisabled();
  await input.focus();
  await input.press('End');
  await input.pressSequentially(' continues');
  await expect(input).toBeFocused();
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  await expect(input).toHaveAttribute('data-retained', 'yes');
  await expect(window.locator('blockquote')).toHaveText('Frozen original quote');
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 1, closes: 0 });
  await run(page, 'finishWindowStatus');
  await expect(window.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(input).toBeFocused();
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  await window.getByRole('button', { name: 'Reopen', exact: true }).click();
  await run(page, 'finishWindowStatus', 'failed');
  await expect(window.getByRole('alert')).toBeVisible();
  await expect(window.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(input).toHaveAttribute('data-retained', 'yes');
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 2, closes: 0 });
});
test('window status controls require writer ownership and current admission', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mountWindow', 'readonly');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
  await run(page, 'mountWindow', 'blocked');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 0, closes: 0 });
});
