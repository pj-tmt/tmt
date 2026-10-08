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
  await expect(page.getByTestId('ask-state')).toHaveText('Waiting for Deterministic agent');
  const before = await run(page, 'proof');
  expect(before.sends).toHaveLength(1);
  await page.clock.fastForward(2 * 60 * 60 * 1000 + 1);
  await expect(page.getByTestId('ask-state')).toHaveText('No reply yet from Deterministic agent');
  await expect(page.getByTestId('ask-state').locator('svg.lucide-clock')).toBeVisible();
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
  await expect(page.getByRole('button', { name: 'Check again', exact: true })).toBeEnabled();
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
  await expect(status).toHaveText([
    'No reply yet from Deterministic agent',
    'Waiting for Deterministic agent',
  ]);
  await page.clock.fastForward(60 * 60 * 1000);
  await expect(status).toHaveText([
    'No reply yet from Deterministic agent',
    'No reply yet from Deterministic agent',
  ]);
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
  await expect(page.getByTestId('ask-entry').getByTestId('ask-state')).toContainText(
    'Waiting for approval',
  );
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

const focused = (page: Page) =>
  page.evaluate(() => {
    const active = document.activeElement;
    return active?.getAttribute('role') === 'combobox'
      ? 'composer'
      : active?.textContent === 'Try again'
        ? 'retry'
        : (active?.tagName ?? 'none');
  });

test('a failed directory recovers in place with Try again, keeping focus and the draft (#2014)', async ({
  page,
}) => {
  const input = await mount(page, 'discovery-failure');
  const status = page.getByRole('status');
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  const ask = page.getByRole('button', { name: 'Ask agent', exact: true });
  const picker = page.getByRole('button', { name: 'Choose recipient', exact: true });
  await expect(status).toHaveText('Agents are unavailable. You can still post a comment.');
  await expect(picker).toBeDisabled();
  await input.fill('Draft kept across recovery.');
  await expect(ask).toBeDisabled();
  // Enter on a default comment and the disabled Ask do nothing about an Ask.
  expect((await run(page, 'directoryProof')).reads).toBe(1);
  // A retry that fails again keeps focus on the action, busy while it runs.
  await run(page, 'setDirectory', 'park');
  await retry.focus();
  await retry.press('Enter');
  await expect(status).toHaveText('Checking for agents…');
  await expect(retry).toBeDisabled();
  await expect(retry).toHaveAttribute('aria-busy', 'true');
  await expect(ask).toBeDisabled();
  await run(page, 'releaseDirectory', 'fail');
  await expect(status).toHaveText('Agents are unavailable. You can still post a comment.');
  await expect(retry).toBeEnabled();
  expect(await focused(page)).toBe('retry');
  // The fault clears: one more retry reaches ready without a remount, focus returns to the composer.
  await run(page, 'setDirectory', 'ok');
  await retry.press('Enter');
  await expect(status).toHaveText('Enter sends · Shift+Enter adds a line · Esc closes');
  await expect(retry).toHaveCount(0);
  await expect(picker).toBeEnabled();
  expect(await focused(page)).toBe('composer');
  await expect(input).toHaveText('Draft kept across recovery.', { useInnerText: true });
  expect((await run(page, 'directoryProof')).reads).toBe(3);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, sends: [] });
});

test('after Reconnect Ask is fenced until the fresh directory resolves, then one click sends once (#2066)', async ({
  page,
}) => {
  const input = await mount(page, 'held');
  const status = page.getByRole('status');
  const ask = page.getByRole('button', { name: 'Ask agent', exact: true });
  const picker = page.getByRole('button', { name: /^(Choose|Change) recipient$/ });
  await input.fill('Explain this.');
  await expect(picker).toBeEnabled();
  await input.press('ArrowDown');
  await input.press('Enter');
  await expect(page.locator('.annotation-hint').last()).toContainText('Recipient');
  await expect(ask).toBeEnabled();
  // Reconnect swaps the Ask binding; its first directory read is slow.
  await run(page, 'setDirectory', 'park');
  await run(page, 'reconnectAsk');
  await expect(status).toHaveText('Checking for agents…');
  await expect(ask).toBeDisabled();
  await expect(picker).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Try again' })).toHaveCount(0);
  // Enter and the disabled button do nothing: no preparation, no error, no write.
  await input.press('Enter');
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(await run(page, 'proof')).toMatchObject({ writes: 0, preparations: 0, sends: [] });
  await expect(input).toHaveText('Explain this.', { useInnerText: true });
  // The plain comment path stays available while the directory loads.
  await expect(page.getByRole('button', { name: 'Post comment', exact: true })).toBeEnabled();
  await run(page, 'releaseDirectory');
  await expect(ask).toBeEnabled();
  await expect(picker).toBeEnabled();
  await expect(page.locator('.annotation-hint').last()).toContainText('Recipient');
  await ask.click();
  await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'held');
  const proof = await run(page, 'proof');
  expect(proof.preparations).toBe(1);
  expect(proof.sends).toHaveLength(1);
});

test('a directory read for a replaced binding or attempt cannot change the composer', async ({
  page,
}) => {
  await mount(page, 'discovery-failure');
  const status = page.getByRole('status');
  const ask = page.getByRole('button', { name: 'Ask agent', exact: true });
  await expect(status).toContainText('Agents are unavailable');
  // Attempt A is parked; a Reconnect then reads a healthy directory; A fails late.
  await run(page, 'setDirectory', 'park');
  await page.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect(status).toHaveText('Checking for agents…');
  await run(page, 'setDirectory', 'ok');
  await run(page, 'reconnectAsk');
  await expect(status).toHaveText('Enter sends · Shift+Enter adds a line · Esc closes');
  await run(page, 'releaseDirectory', 'fail');
  await expect(status).toHaveText('Enter sends · Shift+Enter adds a line · Esc closes');
  await expect(page.getByRole('button', { name: 'Choose recipient', exact: true })).toBeEnabled();
  await expect(ask).toBeDisabled();
});

test('captures the directory states for review', async ({ page }) => {
  const directory = process.env.COLAB_2014_CAPTURE_DIR;
  test.skip(!directory, 'review captures only');
  mkdirSync(directory!, { recursive: true });
  for (const surface of ['annotation', 'reply', 'chat']) {
    for (const theme of ['light', 'dark']) {
      for (const width of [1440, 390]) {
        await page.goto('/');
        await run(page, 'setSurface', surface);
        await run(page, 'mount', 'discovery-failure');
        await page.setViewportSize({ width, height: 700 });
        await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        const input = page.getByRole('combobox', { name: 'Message', exact: true });
        await input.fill('Draft kept while the directory recovers.');
        const shot = (state: string) =>
          page.screenshot({ path: `${directory}/${surface}-${state}-${width}-${theme}.png` });
        await expect(page.getByRole('status')).toContainText('Agents are unavailable');
        await shot('failed');
        await run(page, 'setDirectory', 'park');
        const retry = page.getByRole('button', { name: 'Try again', exact: true });
        await retry.focus();
        await retry.press('Enter');
        await expect(page.getByRole('status')).toHaveText('Checking for agents…');
        await shot('retry-busy');
        await run(page, 'releaseDirectory', 'fail');
        await expect(retry).toBeEnabled();
        await shot('failed-focused');
        await run(page, 'setDirectory', 'ok');
        await retry.press('Enter');
        await expect(retry).toHaveCount(0);
        await shot('ready');
        await run(page, 'setDirectory', 'park');
        await run(page, 'reconnectAsk');
        await expect(page.getByRole('status')).toHaveText('Checking for agents…');
        await shot('reconnect-loading');
        await run(page, 'releaseDirectory');
        await expect(page.getByRole('status')).toContainText('Enter sends');
        await shot('reconnect-ready');
      }
    }
  }
});

test('a plain comment is available while the directory loads', async ({ page }) => {
  const input = await mount(page, 'held');
  await run(page, 'setDirectory', 'park');
  await run(page, 'reconnectAsk');
  await expect(page.getByRole('status')).toHaveText('Checking for agents…');
  await input.fill('Comment while loading.');
  await page.getByRole('button', { name: 'Post comment', exact: true }).click();
  expect(await run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0, commits: 1 });
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
test('window status controls require owner-member provenance and current admission', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountWindow', 'readonly');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
  // Status is not writer-owned: an owner device controls another device's thread.
  await run(page, 'mountWindow', 'other-writer');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toBeEnabled();
  await run(page, 'mountWindow', 'blocked');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 0, closes: 0 });
});
