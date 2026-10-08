import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  const toggle = page.locator('#ask-page-fixture').getByTestId('chat-toggle');
  if (!(await toggle.isVisible()))
    await page
      .locator('#ask-page-fixture')
      .getByRole('button', { name: 'More page actions' })
      .click();
  await toggle.click();
  await expect(page.getByTestId('chat-panel')).toBeVisible();
}
async function compose(page: Page) {
  const input = page.getByTestId('chat-panel').getByRole('combobox', { name: 'Message' });
  await input.fill('@');
  await expect(page.getByRole('option')).toHaveCount(5);
  await page.getByRole('option').first().click();
  await input.fill('@Agent 1 Explain exactly');
  return input;
}
test('Chat retains drafts across close and live edits; only trusted Enter freezes and sends the current text without a confirmation', async ({
  page,
}) => {
  await mount(page);
  const input = await compose(page);
  await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.locator('#ask-page-fixture').getByTestId('chat-toggle').click();
  await expect(input).toHaveText('@Agent 1 Explain exactly', { useInnerText: true });
  await run(page, 'change', '<p id="selected">Changed selected text</p>');
  await expect(page.getByRole('heading', { name: 'Changed live title' })).toBeVisible();
  await input.evaluate((node) =>
    node.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })),
  );
  expect((await run(page, 'proof')).sends).toEqual([]);
  await input.press('Enter');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  const sent = (await run(page, 'proof')).sends[0];
  expect(sent.message).toContain('Page: Changed live title');
  expect(sent.message).toContain('@Agent 1 Explain exactly');
  expect(sent.message).not.toContain('Changed selected text');
  await expect(page.getByTestId('ask-preview')).toHaveCount(0);
  await expect(input).toHaveText('', { useInnerText: true });
  await page.locator('#ask-page-fixture').getByTestId('comments-toggle').click();
  await expect(page.getByTestId('annotation-row')).toHaveCount(0);
  await page.locator('#ask-page-fixture').getByTestId('chat-toggle').click();
  await run(page, 'syncRecords');
  await expect(page.locator('[data-turn-role="user"] .conversation-body > pre').first()).toHaveText(
    '<script>inert ask</script>',
  );
  await expect(page.locator('.chat-panel script,.chat-panel img')).toHaveCount(0);
  await expect(page.getByText('The agent returned an empty reply.')).toBeVisible();
  await expect(page.getByTestId('ask-reply-attribution')).toContainText('Agent 2');
  await page.getByRole('button', { name: 'Check again' }).click();
  await page.getByRole('button', { name: 'Abandon tracking' }).click();
  expect((await run(page, 'proof')).actions).toEqual([
    'recheck:00000000-0000-4000-8000-000000000021',
    'abandon:00000000-0000-4000-8000-000000000021',
  ]);
  expect((await run(page, 'proof')).sends).toHaveLength(1);
});
test('closing a pending explicit send retains it and never dispatches again on reopen; connection loss disables new turns', async ({
  page,
}) => {
  await mount(page);
  const input = await compose(page);
  await run(page, 'pausePrepare');
  await input.press('Enter');
  await expect(input).toHaveAttribute('contenteditable', 'false');
  await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
  await run(page, 'resumePrepare');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  await page.locator('#ask-page-fixture').getByTestId('chat-toggle').click();
  await expect(input).toHaveText('', { useInnerText: true });
  expect((await run(page, 'proof')).sends).toHaveLength(1);
  await run(page, 'block');
  await expect(input).toHaveAttribute('contenteditable', 'false');
  await expect(page.getByTestId('ask-preview')).toHaveCount(0);
});
test('Chat shows held, pending, replied and display-only reply timeout without changing the ledger or sending', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await page.clock.install();
  await mount(page);
  await page.evaluate(() => (document.documentElement.dataset.theme = 'dark'));
  await run(page, 'syncRecords', 'held');
  // Every state is a Lucide mark plus its word, the mark colored by the state's role.
  const state = (tone: string) =>
    page.getByTestId('ask-state').first().locator(`.ask-state[data-tone="${tone}"]`);
  await expect(page.getByTestId('ask-state').first()).toContainText('Waiting for approval');
  await expect(state('held').locator('svg.lucide')).toBeVisible();
  await page.screenshot({ path: '/tmp/1730-390-dark-held.png' });
  await run(page, 'pending');
  await expect(page.getByTestId('ask-state').first()).toContainText('Waiting for Agent 1');
  await expect(state('waiting').locator('svg.lucide')).toBeVisible();
  await page.screenshot({ path: '/tmp/1730-390-dark-waiting.png' });
  await page.clock.fastForward(2 * 60 * 60 * 1000 + 1);
  await expect(page.getByTestId('ask-state').first()).toContainText('No reply yet from Agent 1');
  await expect(state('waiting').locator('svg.lucide')).toBeVisible();
  await expect(page.getByTestId('ask-state').first()).toHaveAttribute('data-state', 'accepted');
  await page.screenshot({ path: '/tmp/1645-chromium-390-dark-timeout.png' });
  await run(page, 'syncRecords', 'accepted');
  await expect(page.getByTestId('ask-state')).toHaveCount(0);
  await expect(page.getByTestId('ask-reply')).toHaveCount(2);
  await expect(page.getByTestId('ask-reply-attribution').first()).toContainText(
    `Agent 1 · ${text.conversationAgent}`,
  );
  await page.screenshot({ path: '/tmp/1730-390-dark-replied.png' });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.evaluate(() => (document.documentElement.dataset.theme = 'light'));
  await page.screenshot({ path: '/tmp/1730-1440-light-replied.png' });
  expect((await run(page, 'proof')).sends).toEqual([]);
});
test('verified refusal reasons use actionable copy without exposing a resend', async ({ page }) => {
  await mount(page);
  for (const [reason, copy] of [
    ['REMOTE_SCOPE_DENIED', 'Your Remote permission does not allow this ask.'],
    ['REMOTE_INPUT_INVALID', 'Remote rejected this message. Write a new message.'],
    ['REMOTE_RATE_LIMITED', 'Remote is busy. Try a new message later.'],
    [
      'REMOTE_INTENT_CONFLICT',
      'This operation already has a different message. Write a new message.',
    ],
    ['REMOTE_CLOSED', 'Remote is closed. Reconnect before writing another message.'],
    [
      'REMOTE_SESSION_ENDED',
      'Your Remote session ended. Reconnect before writing another message.',
    ],
  ]) {
    await run(page, 'syncRefusal', reason);
    await expect(page.getByTestId('ask-state').first()).toContainText('Not delivered');
    await expect(page.getByTestId('ask-entry').first()).toContainText(copy);
    await expect(
      page.getByTestId('ask-state').first().locator('.ask-state[data-tone="problem"] svg.lucide'),
    ).toBeVisible();
    await expect(page.getByRole('button', { name: 'Abandon tracking' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Check again' })).toHaveCount(0);
  }
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.screenshot({
    path: capturePath('ask-refused-session.png'),
    fullPage: true,
  });
});

test('read refusals show ephemeral copy without changing the admitted operation or dispatching', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'syncRecords');
  for (const [code, copy] of [
    ['REMOTE_INPUT_TOO_LARGE', 'Remote rejected the message size. Write a shorter message.'],
    ['REMOTE_STATE_UNAVAILABLE', 'Remote cannot read this operation yet. Check again later.'],
    ['REMOTE_CORE_UNAVAILABLE', 'The agent service is unavailable. Check again later.'],
  ]) {
    await run(page, 'refuseRead', code);
    await page.getByRole('button', { name: 'Check again' }).click();
    await expect(page.locator('.ask-panel [role=alert]')).toContainText(copy);
    await expect(page.getByTestId('ask-state').first()).toHaveAttribute('data-state', 'uncertain');
    expect((await run(page, 'proof')).sends).toEqual([]);
  }
  await page.screenshot({
    path: capturePath('ask-read-refused.png'),
    fullPage: true,
  });
});

test('mobile modal Chat keeps the shared autocomplete visible and clickable in its top layer', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page);
  const drawer = page.locator('.page-drawer[data-panel=chat][open]');
  expect(await drawer.evaluate((node) => node.matches(':modal'))).toBe(true);
  const input = drawer.getByRole('combobox', { name: 'Message' });
  await input.fill('@');
  const option = drawer.getByRole('option').first();
  await expect(option).toBeInViewport();
  await expect
    .poll(() =>
      option.evaluate((node) => {
        const box = node.getBoundingClientRect();
        return node.contains(
          document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
        );
      }),
    )
    .toBe(true);
  await option.click();
  await expect(input).toHaveText('@Agent 1 ', { useInnerText: true });
  await expect(input).toBeFocused();
  expect((await run(page, 'proof')).sends).toEqual([]);
});

test('mobile Chat preserves an open autocomplete inside its dialog across keyboard close and reopen', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 900 });
  await mount(page);
  const drawer = page.locator('.page-drawer[data-panel=chat][open]');
  const input = drawer.getByRole('combobox', { name: 'Message' });
  await input.fill('@');
  await expect(drawer.getByRole('option')).toHaveCount(5);
  const close = drawer.getByRole('button', { name: 'Close Chat' });
  await close.focus();
  await close.press('Enter');
  await expect(page.locator('.page-drawer[data-panel=chat][open]')).toHaveCount(0);
  const more = page.locator('#ask-page-fixture').getByRole('button', { name: 'More page actions' });
  await more.focus();
  await more.press('Enter');
  const toggle = page.locator('#ask-page-fixture').getByTestId('chat-toggle');
  await toggle.focus();
  await toggle.press('Enter');
  await expect(input).toHaveText('@', { useInnerText: true });
  const option = drawer.getByRole('option').first();
  await expect(option).toBeInViewport();
  await expect
    .poll(() =>
      option.evaluate((node) => {
        const box = node.getBoundingClientRect();
        return node.contains(
          document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
        );
      }),
    )
    .toBe(true);
  await option.click();
  await expect(input).toHaveText('@Agent 1 ', { useInnerText: true });
  expect((await run(page, 'proof')).sends).toEqual([]);
});

for (const selected of [false, true]) {
  test(`Chat keeps focused ${selected ? 'selected text' : 'mid-text caret'} across both drawer breakpoints`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 900 });
    await mount(page);
    const drawer = page.locator('.page-drawer[data-panel=chat][open]');
    const input = drawer.getByRole('combobox', { name: 'Message', exact: true });
    const draft = 'Before draft after';
    await input.fill(draft);
    await expect(input).toHaveText(draft, { useInnerText: true });
    const editorId = await input.getAttribute('id');
    const selection = () =>
      input.evaluate((node) => {
        const range = node.ownerDocument.getSelection()!;
        return {
          anchor: range.anchorNode?.textContent,
          anchorOffset: range.anchorOffset,
          focus: range.focusNode?.textContent,
          focusOffset: range.focusOffset,
          inside: node.contains(range.anchorNode) && node.contains(range.focusNode),
        };
      });
    for (const width of [1440, 390]) {
      await input.focus();
      await input.press('Home');
      for (let index = 0; index < 7; index++) await input.press('ArrowRight');
      if (selected) for (let index = 0; index < 5; index++) await input.press('Shift+ArrowRight');
      const before = await selection();
      expect(before).toEqual({
        anchor: draft,
        anchorOffset: 7,
        focus: draft,
        focusOffset: selected ? 12 : 7,
        inside: true,
      });
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate((width) => {
        document.documentElement.dataset.theme = width === 390 ? 'dark' : 'light';
      }, width);
      // Wait for the real native mode transition, not just updated CSS geometry.
      await expect
        .poll(() => drawer.evaluate((node) => node.matches(':modal')))
        .toBe(width === 390);
      await expect(input).toBeFocused();
      await expect(input).toHaveAttribute('id', editorId!);
      await expect(input).toHaveText(draft, { useInnerText: true });
      expect(await selection()).toEqual(before);
    }
    await page.keyboard.type('kept');
    await expect(input).toHaveText(selected ? 'Before kept after' : 'Before keptdraft after', {
      useInnerText: true,
    });
    expect((await run(page, 'proof')).sends).toEqual([]);
    expect(await run(page, 'discussionProof')).toEqual([]);
    await input.press('End');
    await input.pressSequentially(' @');
    await drawer.getByRole('option').first().click();
    await input.press('Enter');
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
    expect((await run(page, 'proof')).sends[0].message).toContain(
      selected ? 'Before kept after' : 'Before keptdraft after',
    );
    await expect(input).toHaveText('', { useInnerText: true });
    expect((await run(page, 'proof')).sends).toHaveLength(1);
  });
}
