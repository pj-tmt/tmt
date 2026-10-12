import { expect, test, type Locator, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';
import { PNG, attach, send } from '../test/attachment-page.js';

const fixture = '/test/reply-notifications-browser.ts';
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
  const proof = await run(page, 'proof');
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
  const proof = await run(page, 'proof');
  expect(proof.sends).toEqual([]);
  const snapshot = await run(page, 'snapshot');
  expect(snapshot.threads).toHaveLength(2);
});

for (const destination of ['Chat', 'Thread'] as const) {
  for (const removal of ['expiry', 'Escape', 'Open'] as const) {
    test(`${destination} ${removal} preserves the total and only opening the canonical thread clears Unseen`, async ({
      page,
    }) => {
      if (removal === 'expiry') await page.clock.install();
      await mount(page);
      await run(page, 'secondThread');
      const discussion = page
        .locator('.page-header-actions')
        .getByRole('button', { name: /^Discussion \(/, includeHidden: true });
      await expect(discussion).toHaveAccessibleName('Discussion (2)');
      await run(page, 'agentResolves');
      await expect(discussion).toHaveAccessibleName('Discussion (1) · New');
      if (removal === 'expiry') {
        await page.clock.pauseAt(await page.evaluate(() => new Date(Date.now() + 100)));
      }
      const openDestination = async () => {
        if (destination === 'Chat') await open(page, 'Chat');
        else {
          await open(page, 'Comments');
          await page.getByTestId('annotation-row').filter({ hasText: 'Resolved by Atlas' }).click();
        }
      };
      const body = `Retained ${destination} reply after ${removal}`;
      await run(page, 'arrive', { destination, body });
      await expect(cards(page)).toHaveCount(1);
      await expect(cards(page)).toContainText(body);
      if (removal === 'expiry') {
        await page.mouse.move(0, 800);
        await page.clock.runFor(5999);
        await expect(cards(page)).toHaveCount(1);
        await page.clock.runFor(1);
      } else if (removal === 'Escape') {
        await cards(page)
          .getByRole('button', { name: `Open ${destination}`, exact: true })
          .focus();
        await page.keyboard.press('Escape');
      } else {
        await cards(page)
          .getByRole('button', { name: `Open ${destination}`, exact: true })
          .click();
      }
      await expect(cards(page)).toHaveCount(0);
      if (removal !== 'Open') {
        await expect(discussion).toHaveAccessibleName('Discussion (1) · New');
        expect(await run(page, 'seenProof')).toBe(0);
        // The record remains available even after the transient card is gone.
        const snapshot = await run(page, 'snapshot');
        expect(snapshot.threads).toHaveLength(3);
        expect(snapshot.asks).toHaveLength(1);
        expect(snapshot.asks[0].reply).toBe(body);
        if (removal === 'expiry') await page.clock.resume();
        await openDestination();
      }
      const surface = page.getByTestId(destination === 'Chat' ? 'chat-panel' : 'comment-thread');
      await expect(surface).toBeVisible();
      await expect(surface.getByTestId('ask-reply')).toHaveText(body);
      await expect(discussion).toHaveAccessibleName(
        destination === 'Thread' ? 'Discussion (1)' : 'Discussion (1) · New',
      );
      const proof = await run(page, 'proof');
      expect(proof.sends).toEqual([]);
      expect(proof.actions).toEqual([]);
      expect(await run(page, 'discussionProof')).toHaveLength(3);
      if (destination === 'Chat') await close(page, 'Chat');
      else if ((await surface.getAttribute('data-layout')) === 'anchored') {
        await surface.getByRole('button', { name: 'Collapse thread', exact: true }).click();
      } else await close(page, 'Comments');
      await run(page, 'repeat');
      await expect(cards(page)).toHaveCount(0);
      await expect(discussion).toHaveAccessibleName(
        destination === 'Thread' ? 'Discussion (1)' : 'Discussion (1) · New',
      );
      await openDestination();
      await expect(surface.getByTestId('ask-reply')).toHaveText(body);
      await expect(discussion).toHaveAccessibleName(
        destination === 'Thread' ? 'Discussion (1)' : 'Discussion (1) · New',
      );
      await expect(live(page)).toHaveText('');
    });
  }
}

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

test('Chat New messages attention clears at the history end without changing the discussion total', async ({
  page,
}) => {
  await mount(page);
  await run(page, 'history', 'Chat');
  await open(page, 'Chat');
  const chat = page.getByTestId('chat-panel');
  const history = chat.locator('.conversation-messages');
  const index = await run(page, 'arrive', { destination: 'Chat', pending: true });
  await expect(chat.getByTestId('ask-state')).toBeVisible();
  await history.evaluate(async (node) => {
    node.scrollTop = 100;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
  await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(100);
  await run(page, 'complete', { index, body: 'Retained reply beyond the reading position' });
  const attention = chat.getByRole('button', { name: 'New messages', exact: true });
  await expect(attention).toBeVisible();
  await expect(live(page)).toHaveText('');
  await expect(page.locator('.page-header-count').first()).toHaveText('1');
  await run(page, 'repeat');
  await expect(attention).toBeVisible();
  await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(100);
  await attention.click();
  await expect(attention).toHaveCount(0);
  await expect(history).toBeFocused();
  await expect
    .poll(() => history.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop))
    .toBeLessThanOrEqual(1);
  await expect(chat.getByTestId('ask-reply')).toHaveText(
    'Retained reply beyond the reading position',
  );
  await expect(page.locator('.page-header-count').first()).toHaveText('1');
  expect(await run(page, 'seenProof')).toBe(0);
  expect((await run(page, 'proof')).sends).toEqual([]);
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

test('Answer and Proposal share the sole polite queue and announce once after modal suspension', async ({
  page,
}) => {
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mountBlocks');
  await expect(page.locator('.reply-announcement')).toHaveCount(0);
  await expect(live(page)).toHaveCount(1);
  await expect(live(page)).toHaveAttribute('aria-live', 'polite');
  await page.evaluate(() => {
    const events: string[] = [];
    Object.assign(window, { blockAnnouncements: events });
    new MutationObserver((changes) => {
      if (changes.length) {
        const value = document.querySelector('.tmt-ui-notification-status')?.textContent;
        if (value) events.push(value);
      }
    }).observe(document.querySelector('.tmt-ui-notification-status')!, {
      childList: true,
      characterData: true,
      subtree: true,
    });
  });
  await open(page, 'Agents');
  await run(page, 'arrive', { destination: 'Answer', body: 'Answer notification one' });
  await run(page, 'arrive', {
    destination: 'Proposal',
    human: true,
    body: 'Proposal notification two',
  });
  await expect(live(page)).toHaveText('');
  await close(page, 'Agents');
  await expect(cards(page)).toHaveCount(2);
  await expect(live(page)).toContainText('Proposal notification two');
  await run(page, 'repeat');
  const events = await page.evaluate(
    () => (window as unknown as { blockAnnouncements: string[] }).blockAnnouncements,
  );
  expect(events.filter((value) => value.includes('Answer notification one'))).toHaveLength(1);
  expect(events.filter((value) => value.includes('Proposal notification two'))).toHaveLength(1);
  await test.info().attach('block-live-region-events.json', {
    body: JSON.stringify(events),
    contentType: 'application/json',
  });
});

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    test(`persistent count and attention captures ${width} ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.clock.install();
      await mount(page);
      await run(page, 'secondThread');
      await run(page, 'agentResolves');
      const discussion = page
        .locator('.page-header-actions')
        .getByRole('button', { name: /^Discussion \(/, includeHidden: true });
      await expect(discussion).toHaveAccessibleName('Discussion (1) · New');
      await page.clock.pauseAt(await page.evaluate(() => new Date(Date.now() + 100)));
      for (const destination of ['Chat', 'Thread', 'Chat', 'Thread'] as const) {
        await run(page, 'arrive', { destination });
      }
      await expect(cards(page)).toHaveCount(3);
      await expect(page.locator('.tmt-ui-notification-waiting')).toHaveText('1 more waiting');
      await page.screenshot({ path: capturePath(`counts-queue-${width}-${theme}.png`) });
      await cards(page).first().getByRole('button', { name: 'Open Chat', exact: true }).focus();
      await page.keyboard.press('Escape');
      await expect(cards(page)).toHaveCount(3);
      await expect(discussion).toHaveAccessibleName('Discussion (1) · New');
      await page.screenshot({ path: capturePath(`counts-dismissed-${width}-${theme}.png`) });
      await page.mouse.move(0, 800);
      await page.clock.runFor(6000);
      await expect(cards(page)).toHaveCount(0);
      await expect(discussion).toHaveAccessibleName('Discussion (1) · New');
      expect(await run(page, 'seenProof')).toBe(0);
      await page.screenshot({ path: capturePath(`counts-expired-${width}-${theme}.png`) });
      await page.clock.resume();
      await open(page, 'Comments');
      await page.getByTestId('annotation-row').filter({ hasText: 'Resolved by Atlas' }).click();
      await expect(page.getByTestId('comment-thread').getByTestId('ask-reply')).toHaveText([
        'Thread reply 201\nSecond reply line',
        'Thread reply 203\nSecond reply line',
      ]);
      await expect(discussion).toHaveAccessibleName('Discussion (1)');
      await page.screenshot({ path: capturePath(`counts-thread-open-${width}-${theme}.png`) });
      await close(page, 'Comments');
      await open(page, 'Chat');
      await expect(page.getByTestId('chat-panel').getByTestId('ask-reply')).toHaveText([
        'Chat reply 200\nSecond reply line',
        'Chat reply 202\nSecond reply line',
      ]);
      await expect(discussion).toHaveAccessibleName('Discussion (1)');
      await expect(live(page)).toHaveText('');
      await page.screenshot({ path: capturePath(`counts-chat-open-${width}-${theme}.png`) });
    });
  }
}

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
    await expect(page.locator('.reply-announcement')).toHaveCount(0);
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

for (const [width, theme] of [
  [1440, 'light'],
  [390, 'dark'],
] as const) {
  for (const destination of ['Answer', 'Proposal'] as const) {
    test(`${destination} follow-up opens its block with a pointer-visible arrival cue ${width} ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.goto('/test/reply-notifications.html');
      await run(page, 'mountBlocks');
      const block = page.locator(
        destination === 'Answer' ? '.question-card' : '.proposal-inline .proposal-card',
      );
      await expect(block).toHaveCount(1);
      await expect
        .poll(() => block.evaluate((node) => node.getBoundingClientRect().top))
        .toBeGreaterThan(900);
      await run(page, 'arrive', { destination, body: `${destination} exact follow-up` });
      await expect(cards(page)).toHaveCount(1);
      await expect(cards(page)).toContainText(`${destination} exact follow-up`);
      await cards(page)
        .getByRole('button', { name: `Open ${destination}`, exact: true })
        .click();
      await expect(block).toBeFocused();
      await expect(block).toHaveAttribute('data-arrived', '');
      await expect(block).toContainText(`${destination} exact follow-up`);
      const geometry = await block.evaluate((node) => {
        const css = getComputedStyle(node);
        const token = document.createElement('span');
        token.style.color = 'var(--tmt-ui-color-focus)';
        node.append(token);
        const color = getComputedStyle(token).color;
        token.remove();
        return {
          top: node.getBoundingClientRect().top,
          header: document.querySelector('.tmt-ui-header')!.getBoundingClientRect().bottom,
          width: css.outlineWidth,
          style: css.outlineStyle,
          color: css.outlineColor,
          token: color,
        };
      });
      expect(geometry.top).toBeGreaterThanOrEqual(geometry.header);
      expect(geometry.top).toBeLessThan(900);
      expect(geometry.width).toBe('2px');
      expect(geometry.style).toBe('solid');
      expect(geometry.color).toBe(geometry.token);
      await page.screenshot({
        path: capturePath(
          `notification-${destination.toLowerCase()}-arrival-${width}-${theme}.png`,
        ),
      });
      await page.keyboard.press('Tab');
      await expect(block).not.toHaveAttribute('data-arrived', '');
      await run(page, 'repeat');
      await expect(cards(page)).toHaveCount(0);
      expect((await run(page, 'proof')).sends).toEqual([]);
    });
  }
}

test('visible open Answer and Proposal histories suppress arrivals; folded and detached proposals remain reachable', async ({
  page,
}) => {
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mountBlocks');
  const answer = page.locator('.question-card');
  await answer.scrollIntoViewIfNeeded();
  await run(page, 'arrive', {
    destination: 'Answer',
    human: true,
    body: 'Visible answer follow-up',
  });
  await expect(answer).toContainText('Visible answer follow-up');
  await expect(cards(page)).toHaveCount(0);
  const proposal = page.locator('.proposal-inline .proposal-card');
  await proposal.scrollIntoViewIfNeeded();
  await run(page, 'arrive', {
    destination: 'Proposal',
    human: true,
    body: 'Visible proposal follow-up',
  });
  await expect(proposal).toContainText('Visible proposal follow-up');
  await expect(cards(page)).toHaveCount(0);
  await run(page, 'mountBlocks', { detached: true, folded: true });
  await run(page, 'arrive', {
    destination: 'Proposal',
    human: true,
    body: 'Detached folded follow-up',
  });
  await expect(cards(page)).toHaveCount(1);
  await cards(page).getByRole('button', { name: 'Open Proposal', exact: true }).click();
  const fallback = page.locator('dialog:modal .proposal-card');
  await expect(fallback).toBeFocused();
  await expect(fallback).toHaveAttribute('data-arrived', '');
  await expect(fallback).toContainText('Detached folded follow-up');
});

test('Answer activation selects its inactive tab; a covering modal holds the arrival instead of suppressing it', async ({
  page,
}) => {
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mountBlocks', { grouped: true });
  const block = page.getByRole('region', { name: 'Question for you?', exact: true });
  await expect(block).toBeHidden();
  await run(page, 'arrive', {
    destination: 'Answer',
    human: true,
    body: 'Follow-up for the inactive question',
  });
  await expect(cards(page)).toHaveCount(1);
  await cards(page).getByRole('button', { name: 'Open Answer', exact: true }).click();
  await expect(block).toBeFocused();
  await expect(block).toHaveAttribute('data-arrived', '');
  await expect(block).toContainText('Follow-up for the inactive question');
  await open(page, 'Agents');
  await run(page, 'arrive', { destination: 'Answer', human: true, body: 'Held under the modal' });
  await expect(cards(page)).toHaveCount(0);
  await close(page, 'Agents');
  // The block is visible after close, so it has now become read; never replay it.
  await expect(block).toContainText('Held under the modal');
  await expect(cards(page)).toHaveCount(0);
});
