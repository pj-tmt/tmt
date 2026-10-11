import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';
const fixture = '/test/ask-page-browser.tsx';
test.beforeEach(async ({ page }) => {
  // This fixture owns the only app root; do not also start the preview app.
  await page.route('**/src/main.tsx', (route) =>
    route.fulfill({
      contentType: 'text/javascript',
      body: '',
    }),
  );
});
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  const toggle = await pageAction(page.locator('#ask-page-fixture'), 'Chat');
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
  await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
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
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  await expect(page.getByTestId('annotation-row')).toHaveCount(0);
  await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
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
  await (await pageAction(page.locator('#ask-page-fixture'), 'Chat')).click();
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
    [
      'TALK_NOT_ENABLED',
      'cannot send to agents. Ask the owner to enable sending for this device in Remote settings, or on the machine that runs Remote: tmt remote devices talk',
    ],
    ['REMOTE_SCOPE_DENIED', 'Your Remote permission does not allow this ask.'],
    ['REMOTE_INPUT_INVALID', 'Remote rejected this message. Write a new message.'],
    ['REMOTE_RATE_LIMITED', 'Remote is busy. Try again later.'],
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
  const more = page.locator('#ask-page-fixture').getByRole('button', { name: /^Discussion \(/ });
  await more.focus();
  await more.press('Enter');
  const toggle = await pageAction(page.locator('#ask-page-fixture'), 'Chat');
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

for (const theme of ['light', 'dark'] as const)
  test(`390px ${theme}: Annotate waits for drag release; C opens without sending and stays out of the composing field`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript((theme) => {
      document.documentElement.dataset.theme = theme;
    }, theme);
    await page.goto('/');
    await run(page, 'mount');
    const frame = page.frameLocator('#ask-page-fixture iframe');
    const quote = frame.locator('#selected');
    await expect(quote).toHaveText('Exact selected text');
    await quote.evaluate((node) => {
      (node as HTMLElement).style.display = 'inline-block';
    });
    const box = (await quote.boundingBox())!;
    await page.mouse.move(box.x + 1, box.y + box.height / 2);
    await page.mouse.down();
    try {
      await page.mouse.move(box.x + box.width - 1, box.y + box.height / 2, { steps: 8 });
      expect(await quote.evaluate(() => getSelection()!.toString())).toBe('Exact selected text');
      await quote.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      await expect(page.getByTestId('selection-ask')).toBeHidden();
    } finally {
      await page.mouse.up();
    }
    const action = page.getByTestId('selection-ask');
    await expect(action).toBeVisible();
    const actionBox = (await action.boundingBox())!;
    expect(actionBox.x).toBeGreaterThan(box.x + box.width);
    expect(actionBox.x + actionBox.width).toBeLessThanOrEqual(390);
    await page.screenshot({ path: capturePath(`selection-finished-390-${theme}.png`) });
    await expect(action).toHaveAttribute('aria-keyshortcuts', 'c Alt+Enter');
    await page.keyboard.press('c');
    const input = page.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toBeFocused();
    await expect(page.locator('.annotation-popover blockquote')).toHaveText('Exact selected text');
    await expect(input).toHaveText('', { useInnerText: true });
    expect((await run(page, 'proof')).sends).toEqual([]);
    // Capture the opened annotation before the separate editing and IME controls type into it.
    await page.screenshot({ path: capturePath(`selection-annotation-390-${theme}.png`) });
    await input.press('c');
    await expect(page.locator('.selection-mode').getByRole('button')).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    await expect(input).toHaveText('c', { useInnerText: true });
    await page.evaluate(() => {
      document.addEventListener(
        'keydown',
        (event) => {
          Object.assign(window, {
            composingKey: { trusted: event.isTrusted, composing: event.isComposing },
          });
        },
        { capture: true, once: true },
      );
    });
    const client = await page.context().newCDPSession(page);
    await client.send('Input.imeSetComposition', {
      text: 'c',
      selectionStart: 1,
      selectionEnd: 1,
      replacementStart: 0,
      replacementEnd: 1,
    });
    await page.keyboard.press('c');
    expect(
      await page.evaluate(() => (window as unknown as { composingKey: unknown }).composingKey),
    ).toEqual({ trusted: true, composing: true });
    await expect(input).toBeFocused();
    await expect(page.locator('.selection-mode').getByRole('button')).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    await expect(page.locator('.annotation-popover')).toHaveCount(1);
    expect((await run(page, 'proof')).sends).toEqual([]);
    await client.detach();
  });

for (const width of [1440, 390]) {
  test(`new replies announce once inside the active modal without moving focus ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    const live = page.locator('.reply-announcement');
    await expect(live).toHaveCount(1);
    await expect(live).toHaveAttribute('role', 'status');
    await expect(live).toHaveAttribute('aria-live', 'polite');
    if (width === 390)
      await expect(page.locator('dialog:modal .reply-announcement')).toHaveCount(1);
    await page.evaluate(() => {
      const events: string[] = [];
      Object.assign(window, { replyAnnouncements: events });
      new MutationObserver((records) => {
        for (const record of records) {
          const target =
            record.target instanceof Element ? record.target : record.target.parentElement;
          const live = target?.closest('.reply-announcement');
          if (live?.textContent) events.push(live.textContent);
        }
      }).observe(document.body, { subtree: true, childList: true, characterData: true });
    });
    const events = () =>
      page.evaluate(
        () => (window as unknown as { replyAnnouncements: string[] }).replyAnnouncements,
      );
    const input = page.getByTestId('chat-panel').getByRole('combobox', { name: 'Message' });
    await input.focus();
    await run(page, 'syncRecords', 'uncertain');
    await expect(live).toHaveText('Agent 2 replied.'); // An empty reply is still a completion.
    await run(page, 'syncRecords', 'accepted');
    await expect(live).toHaveText('Agent 1 replied.');
    await expect(input).toBeFocused();
    await expect.poll(events).toEqual(['Agent 2 replied.', 'Agent 1 replied.']);
    await page.getByRole('button', { name: 'Close Chat', exact: true }).press('Enter');
    await (await pageAction(page, 'Chat')).click();
    await expect(live).toHaveCount(1);
    await expect(live).toHaveText(width === 390 ? '' : 'Agent 1 replied.');
    await run(page, 'syncRecords', 'accepted');
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    expect(await events()).toEqual(['Agent 2 replied.', 'Agent 1 replied.']);
    await page.evaluate(() => {
      location.hash = '/pages/notes';
    });
    await expect(page.getByRole('heading', { name: 'Another fixture page' })).toBeVisible();
    await expect(live).toHaveCount(1);
    await expect(live).toHaveText('');
    await page.evaluate(() => {
      location.hash = '/pages/welcome';
    });
    await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
    await expect(live).toHaveCount(1);
    await expect(live).toHaveText('');
    expect(await events()).toEqual(['Agent 2 replied.', 'Agent 1 replied.']);
    await test.info().attach('reply-live-region-events.json', {
      body: JSON.stringify(await events(), null, 2),
      contentType: 'application/json',
    });
    expect((await run(page, 'proof')).sends).toEqual([]);
    expect(await run(page, 'seenProof')).toBe(0);
  });
}

test('initial admitted reply history stays silent after loading and reopening', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { pastReplies: true });
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  const live = page.locator('.reply-announcement');
  await expect(live).toHaveCount(1);
  await expect(live).toHaveText('');
  await (await pageAction(page, 'Chat')).click();
  await expect(page.getByTestId('ask-reply')).toHaveText('A reply from before this page opened.');
  await expect(live).toHaveText('');
  await page.getByRole('button', { name: 'Close Chat', exact: true }).press('Enter');
  await (await pageAction(page, 'Chat')).click();
  await expect(live).toHaveText('');
  expect((await run(page, 'proof')).sends).toEqual([]);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const) {
    test(`selection mode retains anchored drafts without dispatch ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await page.goto('/');
      await run(page, 'mount');
      const mode = page.locator('.selection-mode').getByRole('button');
      await expect(mode).toHaveAttribute('aria-pressed', 'false');
      await expect(mode).toHaveAttribute('aria-keyshortcuts', 'c');
      await page.screenshot({ path: capturePath(`1923-off-${width}-${theme}.png`) });
      await mode.focus();
      for (const key of ['Shift+c', 'Alt+c', 'Control+c', 'Meta+c']) await page.keyboard.press(key);
      await expect(mode).toHaveAttribute('aria-pressed', 'false');
      await page.keyboard.down('c');
      await page.keyboard.down('c');
      await page.keyboard.up('c');
      await expect(mode).toHaveAttribute('aria-pressed', 'true');
      await expect(page.getByText(text.selectionModeHint, { exact: true })).toBeVisible();
      await expect(page.locator('.page-drawer[open]')).toHaveCount(0);
      // C above is a keyboard guard case. Capture the ordinary pointer activation separately.
      await expect(mode).toBeFocused();
      expect(await mode.evaluate((node) => node.matches(':focus-visible'))).toBe(true);
      // Use the preceding hint as a test-only starting point for native Tab navigation.
      const hint = page.getByText(text.selectionModeHint, { exact: true });
      await hint.evaluate((node: HTMLElement) => {
        node.tabIndex = -1;
        node.focus();
        node.removeAttribute('tabindex');
      });
      await page.keyboard.press('Tab');
      await expect(mode).toBeFocused();
      expect(await mode.evaluate((node) => node.matches(':focus-visible'))).toBe(true);
      // Release this test's forced keyboard focus before exercising the body shortcut.
      await mode.evaluate((node) => node.blur());
      await page.keyboard.press('c');
      await expect(mode).toHaveAttribute('aria-pressed', 'false');
      await page.keyboard.press('c');
      await expect(mode).toHaveAttribute('aria-pressed', 'true');
      expect(await mode.evaluate((node) => getComputedStyle(node).outlineStyle)).toBe('none');
      await mode.click();
      await expect(mode).toHaveAttribute('aria-pressed', 'false');
      await mode.click();
      await expect(mode).toHaveAttribute('aria-pressed', 'true');
      expect(await mode.evaluate((node) => getComputedStyle(node).outlineStyle)).toBe('none');
      await expect(page.locator('.selection-mode .tmt-ui-icon-action-tooltip')).toBeHidden();
      await page.screenshot({ path: capturePath(`1923-on-${width}-${theme}.png`) });
      const frame = page.frameLocator('#ask-page-fixture iframe');
      await frame.locator('#selected').hover();
      await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(1);
      await page.screenshot({ path: capturePath(`1923-hover-${width}-${theme}.png`) });
      await frame.locator('#selected').click();
      const input = page.locator('.annotation-popover').getByRole('combobox', { name: 'Message' });
      await expect(input).toBeFocused();
      await expect(page.locator('.annotation-popover blockquote')).toHaveText(
        'Exact selected text',
      );
      await expect(page.getByText(text.selectionModeHint, { exact: true })).toHaveCount(0);
      await input.fill('Retain this selection draft');
      await input.press('c');
      await expect(mode).toHaveAttribute('aria-pressed', 'true');
      const modeBox = (await mode.boundingBox())!;
      const windowBox = (await page.locator('.annotation-popover').boundingBox())!;
      const headerBox = (await page.locator('#ask-page-fixture .tmt-ui-header').boundingBox())!;
      expect(modeBox.y).toBeGreaterThanOrEqual(headerBox.y + headerBox.height);
      expect(
        modeBox.y + modeBox.height <= windowBox.y ||
          modeBox.y >= windowBox.y + windowBox.height ||
          modeBox.x + modeBox.width <= windowBox.x ||
          modeBox.x >= windowBox.x + windowBox.width,
      ).toBe(true);
      await page.screenshot({ path: capturePath(`1923-discussion-${width}-${theme}.png`) });
      const originalDraft = await input.textContent();
      await page.getByRole('button', { name: 'Close annotation', exact: true }).click();
      await frame.locator('#selected').click();
      await expect(input).toHaveText(originalDraft!);
      expect((await run(page, 'proof')).sends).toEqual([]);
      await page.getByRole('button', { name: 'Close annotation', exact: true }).click();
      if (width === 1440) {
        await (await pageAction(page, 'Source')).click();
        await expect(page.locator('.page-drawer[open]')).toHaveCount(1);
        await expect(mode).toHaveAttribute('aria-pressed', 'true');
        await expect
          .poll(async () => {
            const controlBox = (await mode.boundingBox())!;
            const panelBox = (await page.locator('.page-drawer[open]').boundingBox())!;
            return controlBox.x + controlBox.width <= panelBox.x - 24;
          })
          .toBe(true);
        await page.screenshot({ path: capturePath(`1923-panel-${width}-${theme}.png`) });
        await page.getByRole('button', { name: 'Close Source', exact: true }).click();
      }
      await run(
        page,
        'change',
        '<p id="selected">Exact selected text</p><div id="empty" style="width:80px;height:80px"></div>',
      );
      await expect(frame.locator('#empty')).toBeVisible();
      await frame.locator('#empty').click();
      await expect(frame.getByRole('status')).toHaveText('Select some text to comment');
      await expect(page.locator('.annotation-popover')).toHaveCount(0);
      await page.screenshot({ path: capturePath(`1923-textless-${width}-${theme}.png`) });
      expect((await run(page, 'proof')).sends).toEqual([]);
      await frame.locator('#selected').hover();
      await expect(frame.getByRole('status')).toHaveCount(0);
      await frame.locator('#selected').click();
      await expect(input).toHaveText(originalDraft!);
    });
  }

test('selection uses admitted comment authority without Ask; a read-only page has no mode or C action', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { commentOnly: true });
  const mode = page.locator('.selection-mode').getByRole('button');
  await expect(mode).toHaveAttribute('aria-pressed', 'false');
  await mode.click();
  await page.frameLocator('#ask-page-fixture iframe').locator('#selected').click();
  const input = page.locator('.annotation-popover').getByRole('combobox', { name: 'Message' });
  await expect(input).toBeFocused();
  await input.fill('A comment without Ask authority');
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => (await run(page, 'discussionProof'))?.length).toBe(1);
  expect((await run(page, 'proof')).sends).toEqual([]);
  await page.evaluate(() => {
    location.hash = '/pages/notes';
  });
  const readonly = page.frameLocator('#ask-page-fixture iframe').locator('#other-page');
  await expect(readonly).toHaveText('Another fixture page');
  await expect(mode).toHaveCount(0);
  await readonly.click();
  await page.keyboard.press('c');
  await expect(mode).toHaveCount(0);
  await expect(page.locator('.annotation-popover')).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
});

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    test(`highlight opens its retained discussion without publishing ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      await page.goto('/');
      await run(page, 'mount');
      await run(page, 'conversation', {
        surface: 'thread',
        state: 'empty',
        message: 'Existing anchored discussion',
      });
      const frame = page.frameLocator('#ask-page-fixture iframe');
      await expect(frame.locator('[data-colab-thread]')).toHaveCount(1);
      const before = await run(page, 'discussionProof');
      const proof = await run(page, 'proof');
      const rect = await frame.locator('#selected').evaluate((node) => {
        const range = node.ownerDocument.createRange();
        range.selectNodeContents(node);
        const box = range.getBoundingClientRect();
        return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
      });
      const iframe = (await page.locator('#ask-page-fixture iframe').boundingBox())!;
      const point = { x: iframe.x + rect.x, y: iframe.y + rect.y };
      await page.mouse.move(point.x, point.y);
      await expect(frame.locator('[data-colab-selection-outline]')).toHaveCount(1);
      await page.screenshot({ path: capturePath(`2586-hover-${width}-${theme}.png`) });
      await page.mouse.click(point.x, point.y);
      const window = page.locator('.annotation-popover');
      await expect(window).toBeVisible();
      await expect(window.getByText('Existing anchored discussion', { exact: true })).toBeVisible();
      await page.screenshot({ path: capturePath(`2586-open-${width}-${theme}.png`) });
      const input = window.getByRole('combobox', { name: 'Message' });
      await input.fill('Keep this discussion draft');
      await page.getByRole('button', { name: 'Close thread', exact: true }).click();
      const mode = page.locator('.selection-mode').getByRole('button');
      await mode.click();
      await page.mouse.click(point.x, point.y);
      await expect(input).toHaveText('Keep this discussion draft');
      await expect(window.getByText('Existing anchored discussion', { exact: true })).toBeVisible();
      expect(await run(page, 'discussionProof')).toEqual(before);
      expect(await run(page, 'proof')).toEqual(proof);
    });

test('a live archive preserves Chat text for Copy and removes publication controls', async ({
  page,
}) => {
  await mount(page);
  const input = await compose(page);
  const draft = '@Agent 1 Explain exactly';
  await expect(input).toHaveText(draft, { useInnerText: true });
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          (window as any).copiedArchiveDraft = value;
        },
      },
    });
  });
  await run(page, 'archive');
  await expect(page.locator('#ask-page-fixture .status')).toContainText(text.archiveReadOnly);
  const chat = page.getByTestId('chat-panel');
  await expect(chat.getByText(text.unsentDraft, { exact: true })).toBeVisible();
  await expect(chat.locator('.archive-unsent-draft pre')).toHaveText(draft);
  await expect(chat.getByRole('combobox')).toHaveCount(0);
  await expect(chat.getByRole('button', { name: 'Send', exact: true })).toHaveCount(0);
  await expect(chat.getByRole('button', { name: /Attach/ })).toHaveCount(0);
  await chat.getByRole('button', { name: text.copyDraft, exact: true }).click();
  expect(await page.evaluate(() => (window as any).copiedArchiveDraft)).toBe(draft);
  await expect(page.getByRole('button', { name: 'Agents', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Selection mode', exact: true })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

test('archive keeps an open anchored draft mounted as read-only Copy text', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  const quote = page.frameLocator('iframe').locator('#selected');
  await quote.evaluate((node) => {
    const range = document.createRange();
    range.selectNodeContents(node);
    const selection = node.ownerDocument.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
  });
  await expect(page.getByTestId('selection-ask')).toBeVisible();
  await page.getByTestId('selection-ask').click();
  const annotation = page.getByRole('dialog', { name: 'Annotate selection' });
  await annotation
    .getByRole('combobox', { name: 'Message', exact: true })
    .fill('Keep this anchored draft');
  await run(page, 'archive');
  await expect(annotation).toBeVisible();
  await expect(annotation.locator('.archive-unsent-draft pre')).toHaveText(
    'Keep this anchored draft',
  );
  await expect(annotation.getByRole('button', { name: 'Copy', exact: true })).toBeVisible();
  await expect(annotation.getByRole('combobox')).toHaveCount(0);
  await expect(annotation.getByRole('button', { name: 'Send', exact: true })).toHaveCount(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
  expect((await run(page, 'proof')).actions).toEqual([]);
});

for (const [width, height, theme] of [
  [1440, 900, 'light'],
  [390, 900, 'dark'],
  [1440, 500, 'light'],
  [390, 300, 'dark'],
  [720, 450, 'light'],
] as const) {
  test(`Escape steps the retained thread ladder and keeps the toggle at the content edge ${width} ${height} ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    const scale = width === 720 ? await page.context().newCDPSession(page) : undefined;
    if (scale) {
      // The layout/DPR combination of a 1440x900 display at 200% zoom; this is
      // an explicit metrics control, not manipulation of browser chrome.
      await scale.send('Emulation.setDeviceMetricsOverride', {
        width,
        height,
        deviceScaleFactor: 2,
        mobile: false,
      });
      expect(await page.evaluate(() => devicePixelRatio)).toBe(2);
    }
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    await page.goto('/');
    await run(page, 'mount');
    await run(
      page,
      'change',
      '<main style="height:1800px"><p id="selected" style="margin-left:calc(100% - 150px);padding-top:1000px">Exact selected text</p></main>',
    );
    await run(page, 'conversation', {
      surface: 'thread',
      state: 'replied',
      message: 'Existing discussion. '.repeat(45),
    });
    const mode = page.locator('.selection-mode').getByRole('button');
    await mode.click();
    await expect(mode).toHaveAccessibleName(text.stopSelecting);
    await expect(mode).toHaveAttribute('aria-keyshortcuts', 'c Escape');
    const frame = page.frameLocator('#ask-page-fixture iframe');
    await frame.locator('[data-colab-thread]').click();
    const window = page.locator('.annotation-popover');
    await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-layout', 'anchored');
    const input = window.getByRole('combobox', { name: 'Message' });
    await input.fill('Draft retained @');
    await expect(page.getByRole('option').first()).toBeVisible();
    await input.press('Escape');
    await expect(page.getByRole('option')).toHaveCount(0);
    await expect(window).toBeVisible();
    await expect(mode).toHaveAttribute('aria-pressed', 'true');
    await expect(input).toHaveText('Draft retained @');
    for (const key of ['Shift+Escape', 'Alt+Escape', 'Control+Escape', 'Meta+Escape'])
      await input.press(key);
    await expect(window).toBeVisible();
    await expect
      .poll(async () => {
        const toggle = (await mode.boundingBox())!;
        const thread = (await window.boundingBox())!;
        const header = (await page.locator('#ask-page-fixture .tmt-ui-header').boundingBox())!;
        return (
          toggle.y >= header.height &&
          Math.abs(toggle.y + toggle.height - (height - (width <= 640 ? 16 : 24))) <= 1 &&
          thread.y + thread.height <= toggle.y - 8
        );
      })
      .toBe(true);
    await expect(mode).toBeVisible();
    await page.screenshot({ path: capturePath(`2719-thread-${width}-${height}-${theme}.png`) });
    if (height <= 500) {
      const send = window.getByRole('button', { name: 'Send', exact: true });
      await send.scrollIntoViewIfNeeded();
      await expect
        .poll(async () => {
          const action = (await send.boundingBox())!;
          const shell = (await window.boundingBox())!;
          return action.y >= shell.y && action.y + action.height <= shell.y + shell.height;
        })
        .toBe(true);
      await page.screenshot({ path: capturePath(`2719-composer-${width}-${height}-${theme}.png`) });
    }
    await input.press('Escape');
    await expect(window).toHaveCount(0);
    await expect(mode).toHaveAttribute('aria-pressed', 'true');
    // Collapse returns focus to the opaque frame; its cosmetic Escape notification
    // must reach the same parent owner, not a second selection-mode ladder.
    await expect(page.locator('#ask-page-fixture iframe')).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(mode).toHaveAttribute('aria-pressed', 'false');
    await expect(mode).toHaveAccessibleName(text.selectionMode);
    await frame.locator('[data-colab-thread]').click();
    await expect(input).toHaveText('Draft retained @');
    expect((await run(page, 'proof')).sends).toEqual([]);
    expect((await run(page, 'proof')).actions).toEqual([]);
    await input.press('Escape');
    await mode.click();
    await mode.evaluate((node) => node.blur());
    for (const key of ['Shift+Escape', 'Alt+Escape', 'Control+Escape', 'Meta+Escape'])
      await page.keyboard.press(key);
    await page.evaluate(() =>
      document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
    );
    await expect(mode).toHaveAttribute('aria-pressed', 'true');
    await page.keyboard.down('Escape');
    await page.keyboard.down('Escape');
    await page.keyboard.up('Escape');
    await expect(mode).toHaveAttribute('aria-pressed', 'false');
    await scale?.detach();
  });
}

for (const width of [1440, 390]) {
  test(`Escape closes Chat after suggestions, keeps its draft and fences a busy send ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await mount(page);
    const chat = page.getByTestId('chat-panel');
    const input = chat.getByRole('combobox', { name: 'Message' });
    await input.fill('Chat draft @');
    await expect(page.getByRole('option').first()).toBeVisible();
    await input.press('Escape');
    await expect(page.getByRole('option')).toHaveCount(0);
    await expect(chat).toBeVisible();
    const close = chat.getByRole('button', { name: 'Close Chat', exact: true });
    await close.focus();
    for (const key of ['Shift+Escape', 'Alt+Escape', 'Control+Escape', 'Meta+Escape'])
      await page.keyboard.press(key);
    await expect(chat).toBeVisible();
    await input.press('Escape');
    await expect(chat).toBeHidden();
    await expect(page.locator('.page-drawer[open]')).toHaveCount(0);
    await expect(page.locator('.page-header-actions button:focus')).toHaveCount(1);
    await (await pageAction(page, 'Chat')).click();
    await expect(input).toHaveText('Chat draft @');
    expect((await run(page, 'proof')).sends).toEqual([]);
    await compose(page);
    await run(page, 'pausePrepare');
    await input.press('Enter');
    await expect(chat.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
    await chat.getByRole('button', { name: 'Close Chat', exact: true }).focus();
    await page.keyboard.press('Escape');
    await expect(chat).toBeVisible();
    expect((await run(page, 'proof')).sends).toEqual([]);
    await run(page, 'resumePrepare');
    await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
  });
}

test('Escape does not collapse an anchored thread during its explicit send', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'conversation', {
    surface: 'thread',
    state: 'replied',
    message: 'Existing discussion',
  });
  await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
  const window = page.locator('.annotation-popover');
  const input = window.getByRole('combobox', { name: 'Message' });
  await input.fill('@');
  await page.getByRole('option').first().click();
  await input.fill('@Agent 1 Anchored busy turn');
  await run(page, 'pausePrepare');
  await input.press('Enter');
  await expect(window.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Close thread', exact: true }).focus();
  await page.keyboard.press('Escape');
  await expect(window).toBeVisible();
  expect((await run(page, 'proof')).sends).toEqual([]);
  await run(page, 'resumePrepare');
  await expect.poll(async () => (await run(page, 'proof')).sends.length).toBe(1);
});
