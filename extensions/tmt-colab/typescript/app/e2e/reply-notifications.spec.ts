import { expect, test, type Locator, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';
import { PNG, attach, send } from '../test/attachment-page.js';

const fixture = '/test/reply-notifications-browser.ts';
const pageFixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
async function mount(page: Page) {
  // One app owner: hiding the preview app would leave its portalled live region mounted.
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await expect(page.frameLocator('#ask-page-fixture iframe').locator('#selected')).toHaveText(
    'Exact selected text',
  );
}
const cards = (page: Page) => page.locator('.tmt-ui-notification');
const live = (page: Page) => page.locator('.tmt-ui-notification-status');
async function open(page: Page, name: string) {
  await (await pageAction(page.locator('#ask-page-fixture'), name)).click();
}
async function close(page: Page, name: string) {
  await page.getByRole('button', { name: `Close ${name}`, exact: true }).click();
}
async function revealVisible(history: Locator, record: Locator) {
  const bounds = await history.boundingBox();
  const target = await record.boundingBox();
  expect(bounds).not.toBeNull();
  expect(target).not.toBeNull();
  expect(target!.y).toBeGreaterThanOrEqual(bounds!.y - 1);
  expect(target!.y).toBeLessThan(bounds!.y + bounds!.height);
}

test('own replies notify once; foreign conversations and re-delivery do not notify or dispatch', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'arrive', { destination: 'Chat', foreign: true });
  await expect(cards(page)).toHaveCount(0);
  await run(page, 'arrive', {
    destination: 'Chat',
    body: 'Exact admitted Chat reply\nSecond line',
  });
  await expect(cards(page)).toHaveCount(1);
  await expect(cards(page)).toContainText('Exact admitted Chat reply');
  await expect(cards(page)).not.toContainText('Second line');
  await run(page, 'repeat');
  await expect(cards(page)).toHaveCount(1);
  await cards(page).getByRole('button', { name: 'Dismiss Chat reply' }).click();
  await run(page, 'repeat');
  await expect(cards(page)).toHaveCount(0);
  await run(page, 'foreignConversation');
  await run(page, 'arrive', { destination: 'Thread', human: true, foreign: true });
  await expect(cards(page)).toHaveCount(0);
  const proof = await page.evaluate(async (path) => (await import(path)).proof(), pageFixture);
  expect(proof.sends).toEqual([]);
});

test('Chat activation opens its existing Drawer and reveals the exact reply rather than the latest turn', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'history', 'Chat');
  await run(page, 'arrive', { destination: 'Chat', body: 'The exact notification target' });
  await expect(cards(page)).toHaveCount(1);
  await cards(page).getByRole('button', { name: 'Open Chat', exact: true }).click();
  const chat = page.getByTestId('chat-panel');
  await expect(chat).toBeVisible();
  const target = chat
    .locator('[data-conversation-record]')
    .filter({ hasText: 'The exact notification target' });
  await expect(target).toBeVisible();
  await expect
    .poll(async () => {
      const a = await target.boundingBox();
      const b = await chat.locator('.conversation-messages').boundingBox();
      return !!a && !!b && a.y >= b.y - 1 && a.y < b.y + b.height;
    })
    .toBe(true);
  await revealVisible(chat.locator('.conversation-messages'), target);
  await expect(chat.locator('.conversation-messages')).toBeFocused();
  await close(page, 'Chat');
  await run(page, 'repeat');
  await expect(cards(page)).toHaveCount(0);
});

test('Thread activation opens the canonical anchored window and quote without creating a conversation', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'arrive', {
    destination: 'Thread',
    human: true,
    body: 'A participant replied at this quote',
  });
  await expect(cards(page)).toHaveCount(1);
  await expect(cards(page).locator('blockquote')).toHaveText('Exact selected text');
  await cards(page).getByRole('button', { name: 'Open Thread', exact: true }).click();
  const thread = page.getByTestId('comment-thread');
  await expect(thread).toBeVisible();
  await expect(thread).toHaveAttribute('data-layout', 'anchored');
  await expect(thread.locator('blockquote').first()).toHaveText('Exact selected text');
  await expect(
    thread.getByTestId('comment-entry').filter({ hasText: 'A participant replied at this quote' }),
  ).toBeVisible();
  const proof = await page.evaluate(async (path) => (await import(path)).proof(), pageFixture);
  expect(proof.sends).toEqual([]);
  const snapshot = await run(page, 'snapshot');
  expect(snapshot.threads).toHaveLength(2);
});

test('an open Chat at the end suppresses the toast after closing; scrolled-away arrivals wait for its modal', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'history', 'Chat');
  await open(page, 'Chat');
  const history = page.getByTestId('chat-panel').locator('.conversation-messages');
  await expect
    .poll(() => history.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
    .toBeLessThanOrEqual(1);
  await run(page, 'arrive', { destination: 'Chat', body: 'Already visible at the end' });
  await expect(page.getByTestId('chat-panel').getByTestId('ask-reply')).toHaveText(
    'Already visible at the end',
  );
  await close(page, 'Chat');
  await expect(cards(page)).toHaveCount(0);
  await open(page, 'Chat');
  // The outgoing turn exists before the reader leaves the end. Its final reply
  // must not be mistaken for a newly sent own turn that intentionally follows.
  const index = await run(page, 'arrive', { destination: 'Chat', pending: true });
  await expect(page.getByTestId('chat-panel').getByTestId('ask-state')).toBeVisible();
  await history.evaluate(async (node) => {
    node.scrollTop = 100;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
  await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(100);
  await run(page, 'complete', { index, body: 'Not at the reader position' });
  await expect(live(page)).toHaveText('');
  await expect(
    page.getByTestId('chat-panel').getByRole('button', { name: 'New messages' }),
  ).toBeVisible();
  await close(page, 'Chat');
  await expect(cards(page)).toHaveCount(1);
  await expect(cards(page)).toContainText('Not at the reader position');
});

test('Menu and Manage suspension recheck withdrawn records before display and announcement', async ({
  page,
}) => {
  await mount(page);
  const header = page.locator('#ask-page-fixture .page-header-actions');
  await header.getByRole('button', { name: 'More', exact: true }).click();
  await run(page, 'arrive', { destination: 'Chat', body: 'Queued behind a Menu' });
  await expect(page.locator('.tmt-ui-notifications')).not.toBeVisible();
  await expect(live(page)).toHaveText('');
  await page.getByRole('menuitem', { name: 'Manage page', exact: true }).click();
  await expect(page.locator('.management-dialog')).toBeVisible();
  await run(page, 'arrive', { destination: 'Thread', body: 'Withdrawn while Manage is open' });
  await run(page, 'withdraw', 'Thread');
  await expect(live(page)).toHaveText('');
  await page
    .locator('.management-dialog')
    .getByRole('button', { name: 'Close', exact: true })
    .click();
  await expect(cards(page)).toHaveCount(1);
  await expect(cards(page)).toContainText('Queued behind a Menu');
  await expect(live(page)).not.toContainText('Withdrawn');
});

test('a nested attachment modal cannot resume while its Chat Drawer remains open', async ({
  page,
}) => {
  await mount(page);
  await open(page, 'Chat');
  const chat = page.getByTestId('chat-panel');
  await chat.getByRole('combobox', { name: 'Message', exact: true }).fill('Preview this image');
  await attach(page, [{ name: 'notification-preview.png', mimeType: 'image/png', buffer: PNG }]);
  await send(page).click();
  const image = chat.getByRole('button', { name: 'Preview notification-preview.png', exact: true });
  await expect(image).toBeVisible();
  await image.click();
  await expect(page.locator('.attachment-viewer')).toBeVisible();
  await run(page, 'arrive', { destination: 'Thread', body: 'Reply behind two modal owners' });
  await expect(live(page)).toHaveText('');
  await page.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(page.locator('.attachment-viewer')).toHaveCount(0);
  await expect(chat).toBeVisible();
  await expect(page.locator('.tmt-ui-notifications')).not.toBeVisible();
  await expect(live(page)).toHaveText('');
  await close(page, 'Chat');
  await expect(cards(page)).toHaveCount(1);
  await expect(cards(page)).toContainText('Reply behind two modal owners');
});

test('typing focus and caret survive arrivals; hidden-document resume announces only new displayed replies', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'arrive', { destination: 'Thread', body: 'Open this conversation' });
  await cards(page).getByRole('button', { name: 'Open Thread', exact: true }).click();
  const field = page
    .getByTestId('comment-thread')
    .getByRole('combobox', { name: 'Message', exact: true });
  await field.fill('Retained unsent draft');
  await field.press('ArrowLeft');
  const identity = await field.getAttribute('id');
  const caret = () =>
    field.evaluate(() => ({
      anchor: getSelection()?.anchorOffset,
      focus: getSelection()?.focusOffset,
    }));
  const selection = await caret();
  await page.evaluate(() => {
    const regions = [
      ...document.querySelectorAll('.tmt-ui-notification-status, .reply-announcement'),
    ];
    const events: { region: string; text: string }[] = [];
    Object.assign(window, { notificationAnnouncements: events, notificationHidden: true });
    for (const region of regions) {
      let previous = region.textContent ?? '';
      new MutationObserver(() => {
        const value = region.textContent ?? '';
        if (value && value !== previous) events.push({ region: region.className, text: value });
        previous = value;
      }).observe(region, { subtree: true, characterData: true, childList: true });
    }
    Object.defineProperty(document, 'hidden', {
      configurable: true,
      get: () => (window as unknown as { notificationHidden: boolean }).notificationHidden,
    });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await run(page, 'arrive', { destination: 'Chat', body: 'New reply during hidden document' });
  await expect(live(page)).toHaveText('');
  await expect(field).toBeFocused();
  await page.evaluate(() => {
    Object.assign(window, { notificationHidden: false });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  // The active anchored window also holds the queue after document visibility resumes.
  await expect(live(page)).toHaveText('');
  await expect(page.locator('.tmt-ui-notifications')).not.toBeVisible();
  await expect(field).toBeFocused();
  await expect(field).toHaveText('Retained unsent draft');
  expect(await field.getAttribute('id')).toBe(identity);
  expect(await caret()).toEqual(selection);
  await page
    .getByTestId('comment-thread')
    .getByRole('button', { name: 'Collapse thread', exact: true })
    .click();
  await expect(live(page)).toContainText('New reply during hidden document');
  await run(page, 'repeat');
  const events = await page.evaluate(
    () =>
      (window as unknown as { notificationAnnouncements: { region: string; text: string }[] })
        .notificationAnnouncements,
  );
  expect(events.filter((event) => event.text.includes('New reply during hidden document'))).toEqual(
    [{ region: 'tmt-ui-notification-status', text: 'Chat: New reply during hidden document\n' }],
  );
});

test('staged Answer and Proposal replies remain polite-only once, disjoint from Chat/Thread', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'arrive', {
    destination: 'Chat',
    body: 'Chat belongs to the notification region',
  });
  await expect(live(page)).toContainText('Chat belongs to the notification region');
  await expect(page.locator('.reply-announcement')).toHaveText('');
  const other = (method: string, argument?: unknown) =>
    page.evaluate(async ({ path, method, argument }) => (await import(path))[method](argument), {
      path: pageFixture,
      method,
      argument,
    });
  await other('question', { kind: 'short' });
  const question = page.locator('.question-card');
  await expect(question.getByLabel('Your answer', { exact: true })).toBeVisible();
  await question.getByLabel('Your answer', { exact: true }).fill('My recorded answer');
  await question.getByRole('button', { name: /^Send answer/ }).click();
  await expect(question.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
  await other('questionAsk', 'replied');
  await expect(question.getByTestId('ask-reply')).toHaveText('I read your answer in this thread.');
  await expect(page.locator('.reply-announcement')).toHaveText('Deterministic agent replied.');
  await expect(live(page)).not.toContainText('I read your answer');
  await expect(cards(page)).toHaveCount(0);
  const legacy = await page.locator('.reply-announcement').evaluate((node) => {
    let changes = 0;
    const observer = new MutationObserver(() => {
      if (node.textContent) changes++;
    });
    observer.observe(node, { subtree: true, characterData: true, childList: true });
    Object.assign(window, { legacyChanges: () => changes });
    return node.textContent;
  });
  await other('questionAsk', 'replied');
  await expect(page.locator('.reply-announcement')).toHaveText(legacy!);
  expect(
    await page.evaluate(() => (window as unknown as { legacyChanges(): number }).legacyChanges()),
  ).toBe(0);
  await other('proposal', 'inline');
  // The normal proposal input records the follow-up before its admitted reply.
  const proposal = page.locator('.proposal-card');
  await expect(proposal).toBeVisible();
  await proposal.getByRole('button', { name: 'Follow up', exact: true }).click();
  const input = proposal.getByRole('combobox', { name: 'Message', exact: true });
  // Keep the existing admitted recipient chip; replacing the field would create only a comment.
  await input.press('End');
  await input.pressSequentially('A proposal follow-up');
  await proposal.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => (await other('proof')).sends.length).toBe(2);
  await other('proposalAsk', 'replied');
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { legacyChanges(): number }).legacyChanges()),
    )
    .toBe(1);
  await expect(page.locator('.reply-announcement')).toHaveText('Deterministic agent replied.');
  await expect(live(page)).not.toContainText('The supporting notes');
  await expect(cards(page)).toHaveCount(0);
});

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  test(`real parent mixed reply queue and activation capture ${width} ${theme}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    await mount(page);
    for (const destination of ['Chat', 'Thread', 'Chat', 'Thread', 'Chat'] as const) {
      await run(page, 'arrive', { destination });
    }
    await expect(cards(page)).toHaveCount(3);
    await expect(page.locator('.tmt-ui-notification-waiting')).toHaveText('2 more waiting');
    await expect(page.locator('.reply-announcement')).toHaveText('');
    const count = await page.locator('.page-header-count').first().textContent();
    await page.screenshot({ path: capturePath(`replies-queue-${width}-${theme}.png`) });
    await cards(page)
      .filter({ has: page.getByRole('button', { name: 'Open Thread', exact: true }) })
      .getByRole('button', { name: 'Open Thread', exact: true })
      .click();
    await expect(page.getByTestId('comment-thread')).toBeVisible();
    await expect(page.locator('.tmt-ui-notifications')).not.toBeVisible();
    await expect(live(page)).toHaveText('');
    await expect(page.locator('.page-header-count').first()).toHaveText(count!);
    await page.screenshot({ path: capturePath(`replies-thread-${width}-${theme}.png`) });
    await run(page, 'arrive', { destination: 'Chat', body: 'Held while reading the thread' });
    await expect(page.locator('.tmt-ui-notifications')).not.toBeVisible();
    await expect(live(page)).toHaveText('');
    await page
      .getByTestId('comment-thread')
      .getByRole('button', { name: 'Collapse thread', exact: true })
      .click();
    await expect(cards(page)).toHaveCount(3);
    // Reply 203 was already waiting before activation; opening the window
    // holds that entry rather than changing its original place in the FIFO.
    await expect(cards(page)).toContainText([
      'Chat reply 200',
      'Chat reply 202',
      'Thread reply 203',
    ]);
    await expect(page.locator('.tmt-ui-notification-waiting')).toHaveText('2 more waiting');
    await expect(live(page)).toContainText('Thread reply 203');
    await expect(live(page)).not.toContainText('Chat reply 200');
    await expect(live(page)).not.toContainText('Chat reply 202');
  });
}
