import { pageAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { destination } from '../test/ask-fixtures.js';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}

/** The reply or Chat composer inside its real host, which shows a sent message at once. */
async function mount(page: Page, surface: string, mode = 'accepted') {
  await page.goto('/');
  await run(page, 'setSurface', surface);
  await run(page, 'setAgents', [{ ...destination(), agentName: 'alpha' }]);
  await run(page, 'setCreator', false);
  await run(page, 'setDirectory', 'ok');
  await run(page, 'setEchoHost', true);
  await run(page, 'mount', mode);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await expect(page.locator('.annotation-hint[role="status"]')).not.toContainText('Checking');
  return input;
}
const echo = (page: Page) => page.getByTestId('comment-echo');
const send = (page: Page) => page.getByRole('button', { name: /^Send(?: · to .+)?$/ });

for (const surface of ['reply', 'chat']) {
  test(`${surface}: the sent message shows in the frame the composer clears, with no busy words or grey`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await input.fill('Echo me now.');
    await run(page, 'holdWrite');
    // Record the first frame in which the echo exists, before Playwright can poll.
    await page.evaluate(() => {
      const seen = new MutationObserver(() => {
        if (!document.querySelector('[data-testid="comment-echo"]')) return;
        seen.disconnect();
        requestAnimationFrame(() => {
          const composer = document.querySelector('[role="combobox"]')!;
          document.body.dataset.echoFrame = JSON.stringify({
            composer: (composer as HTMLElement).innerText.trim(),
            echo: document.querySelector('[data-testid="comment-echo"] .comment-body')!.textContent,
          });
        });
      });
      seen.observe(document.body, { childList: true, subtree: true });
    });
    await input.press('Enter');
    await expect
      .poll(() => page.evaluate(() => document.body.dataset.echoFrame))
      .toBe(JSON.stringify({ composer: '', echo: 'Echo me now.' }));
    // The echo is the only busy signal: no Sending words, no Send busy mark, no grey composer.
    await expect(page.getByText('Sending…')).toHaveCount(0);
    await expect(send(page)).not.toHaveAttribute('aria-busy', 'true');
    await expect(input).not.toHaveAttribute('aria-disabled', 'true');
    await expect(input).toHaveAttribute('contenteditable', 'true');
    const mark = echo(page).locator('.comment-echo-mark');
    await expect(mark).toHaveCSS('visibility', 'hidden');
    await expect(mark).toHaveCSS('visibility', 'visible');
    await expect(mark).toHaveText('');
    await run(page, 'releaseWrite');
  });

  test(`${surface}: the verified record replaces the echo in its place`, async ({ page }) => {
    const input = await mount(page, surface);
    await input.fill('Replace me in place.');
    await run(page, 'holdWrite');
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
    const before = (await echo(page).boundingBox())!;
    await run(page, 'releaseWrite');
    await expect.poll(async () => (await run(page, 'editingProof')).captured).toHaveLength(1);
    // Recorded but not yet in the verified view: the echo still holds the place.
    await expect(echo(page)).toHaveCount(1);
    await expect(page.getByTestId('comment-entry')).toHaveCount(0);
    await run(page, 'publishRecorded');
    await expect(echo(page)).toHaveCount(0);
    const entry = page.getByTestId('comment-entry');
    await expect(entry).toHaveCount(1);
    await expect(entry).toContainText('Replace me in place.');
    const after = (await entry.boundingBox())!;
    expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
    expect(Math.abs(after.height - before.height)).toBeLessThanOrEqual(1);
  });

  test(`${surface}: a write failure leaves one failed row that holds the draft until Retry or Edit`, async ({
    page,
  }) => {
    const input = await mount(page, surface, 'write-failure');
    const look = () =>
      page.locator('.annotation-compose').evaluate((compose) =>
        ['.rich-input-surface', '.rich-input-field'].map((selector) => {
          const style = getComputedStyle(compose.querySelector(selector)!);
          return [style.color, style.backgroundColor, style.borderTopColor];
        }),
      );
    const empty = await look();
    await input.pressSequentially('@alpha Kept draft.');
    const draft = (await run(page, 'editingProof')).edit;
    expect(draft.mentions).toHaveLength(1);
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    // The row is the one place: one alert, '✗ Not sent.' with its reference.
    await expect(page.getByRole('alert')).toHaveCount(1);
    await expect(echo(page).getByRole('alert')).toHaveText(
      `✗ ${text.messageUnsent} ${text.messageSendReference('S12')}`,
    );
    await expect(echo(page).locator('.comment-echo-mark')).toHaveCount(0);
    // The composer stays empty and inert, not grey, and says why.
    await expect(input).toHaveText('', { useInnerText: true });
    await expect(input).toHaveAttribute('aria-disabled', 'true');
    await expect(page.getByText(text.messageUnsentFirst)).toBeVisible();
    await expect(send(page)).toBeDisabled();
    // Inert, not grey: the composer looks exactly as when empty; only the hint changed.
    expect(await look()).toEqual(empty);
    await input.pressSequentially('typed while unsent');
    await expect(input).toHaveText('', { useInnerText: true });
    await send(page).click({ force: true });
    await expect(page.getByText(text.messageUnsentFirst)).toBeVisible();
    // The kept draft is the row's: closing now would keep text and chip.
    expect((await run(page, 'editingProof')).edit).toMatchObject({
      value: draft.value,
      mentions: draft.mentions,
    });
    await input.press('Enter');
    expect(await run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0, sends: [] });
    // Retry re-sends exactly that draft once; it fails again into a fresh row.
    await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
    await expect.poll(async () => (await run(page, 'editingProof')).captured).toHaveLength(2);
    expect((await run(page, 'editingProof')).captured.at(-1)).toMatchObject({ body: draft.value });
    await expect(echo(page)).toHaveCount(1);
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    // Edit moves the full draft back into the composer and removes the row.
    await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
    await expect(echo(page)).toHaveCount(0);
    await expect(input).toHaveText('@alpha Kept draft.', { useInnerText: true });
    await expect(input).toBeFocused();
    await expect(input).toHaveAttribute('aria-disabled', 'false');
    expect((await run(page, 'editingProof')).edit).toMatchObject({
      value: draft.value,
      mentions: draft.mentions,
    });
  });

  for (const [failure, reference] of [
    ['refused', 'S10'],
    ['stale', 'S14'],
  ] as const)
    test(`${surface}: attachment refusal ${reference} is reported on the failed row that holds the draft`, async ({
      page,
    }) => {
      const input = await mount(page, surface);
      await input.fill('Kept attachment draft.');
      await page.evaluate(async (failure) => {
        const path = '/src/attachment-draft.ts';
        const { AttachmentDraft } = await import(path);
        const original = AttachmentDraft.prototype.prepare;
        AttachmentDraft.prototype.prepare = async () => {
          AttachmentDraft.prototype.prepare = original;
          if (failure === 'refused') return { ok: false, why: 'refused' };
          const service = '/src/attachment-service.ts';
          const { AttachmentStaleError } = await import(service);
          throw new AttachmentStaleError([]);
        };
      }, failure);
      await input.press('Enter');
      await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
      await expect(echo(page).getByRole('alert')).toContainText(`(ref ${reference})`);
      await expect(page.locator('.annotation-send-notice')).toHaveCount(0);
      await expect(input).toHaveText('', { useInnerText: true });
      await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
      await expect(input).toHaveText('Kept attachment draft.', { useInnerText: true });
      expect(await run(page, 'proof')).toMatchObject({ writes: 0, sends: [] });
    });

  test(`${surface}: a record landing just after the mark shows keeps the mark for its hold`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await input.fill('Landing at 450 ms.');
    await run(page, 'holdWrite');
    const sentAt = await page.evaluate(() => performance.now());
    await input.press('Enter');
    const mark = echo(page).locator('.comment-echo-mark');
    await expect(mark).toHaveAttribute('data-shown');
    await page.waitForFunction((at) => performance.now() - at >= 450, sentAt);
    await run(page, 'releaseWrite');
    await expect.poll(async () => (await run(page, 'editingProof')).captured).toHaveLength(1);
    // Record the moment the echo gives way, and that the record never shows beside it.
    await page.evaluate(() => {
      const seen = new MutationObserver(() => {
        const entry = document.querySelector('[data-testid="comment-entry"]');
        const echoRow = document.querySelector('[data-testid="comment-echo"]');
        if (entry && echoRow) document.body.dataset.echoBoth = 'true';
        if (echoRow) return;
        seen.disconnect();
        document.body.dataset.echoGone = String(performance.now());
      });
      seen.observe(document.body, { childList: true, subtree: true });
    });
    await run(page, 'publishRecorded');
    await expect(echo(page)).toHaveCount(0);
    await expect(page.getByTestId('comment-entry')).toContainText('Landing at 450 ms.');
    const gone = Number(await page.evaluate(() => document.body.dataset.echoGone));
    // Shown at 400 ms, the mark holds 400 ms: the echo stays until at least 800 ms.
    expect(gone - sentAt).toBeGreaterThanOrEqual(800);
    expect(await page.evaluate(() => document.body.dataset.echoBoth)).toBeUndefined();
  });

  test(`${surface}: a failure after the record keeps the echo for the verified record`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await input.fill('Recorded once.');
    await page.evaluate(async () => {
      const path = '/src/attachment-draft.ts';
      const { AttachmentDraft } = await import(path);
      const original = AttachmentDraft.prototype.committed;
      AttachmentDraft.prototype.committed = () => {
        AttachmentDraft.prototype.committed = original;
        throw new Error('post-record private failure');
      };
    });
    await input.press('Enter');
    await expect(page.locator('.annotation-send-notice')).toContainText('(ref S13)');
    // Recorded: no Retry and no restored draft, which would send it twice.
    await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
    await expect(echo(page).getByRole('button', { name: 'Retry' })).toHaveCount(0);
    await expect(input).toHaveText('');
    await run(page, 'publishRecorded');
    await expect(echo(page)).toHaveCount(0);
    await expect(page.getByTestId('comment-entry')).toContainText('Recorded once.');
    expect(await run(page, 'proof')).toMatchObject({ writes: 1 });
  });

  test(`${surface}: a second Send while one is pending records nothing and keeps the new text`, async ({
    page,
  }) => {
    const input = await mount(page, surface);
    await input.fill('First message.');
    await run(page, 'holdWrite');
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
    await input.pressSequentially('Second message.');
    await input.press('Enter');
    await expect(input).toHaveText('Second message.', { useInnerText: true });
    await expect(echo(page)).toHaveCount(1);
    await expect(echo(page)).toContainText('First message.');
    await run(page, 'releaseWrite');
    await expect.poll(async () => (await run(page, 'editingProof')).captured).toHaveLength(1);
    // The completed first send leaves the new draft alone.
    await expect(input).toHaveText('Second message.', { useInnerText: true });
  });
}

/** The anchored annotation dialog passes its own composer to the thread window. */
async function openAnchoredThread(page: Page) {
  const page_ = (method: string) =>
    page.evaluate(async (method) => {
      const path = '/test/ask-page-browser.tsx';
      return (await import(path))[method]();
    }, method);
  await page.goto('/');
  await page_('mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('#selected')
    .evaluate((node) => {
      const range = node.ownerDocument.createRange();
      range.selectNodeContents(node);
      const selection = node.ownerDocument.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
    });
  await page.getByTestId('selection-ask').click();
  const dialog = page.getByRole('dialog', { name: 'Annotate selection' });
  const input = dialog.getByRole('combobox', { name: 'Message', exact: true });
  await input.pressSequentially('The first comment.');
  await input.press('Enter');
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  return { dialog, input, page_ };
}

test('anchored dialog: a reply shows at once and its verified record takes its place', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnchoredThread(page);
  await input.pressSequentially('A held reply.');
  await page_('holdReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
  await expect(echo(page)).toContainText('A held reply.');
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(input).toHaveAttribute('contenteditable', 'true');
  const before = (await echo(page).boundingBox())!;
  await page_('releaseReply');
  await expect(echo(page)).toHaveCount(0);
  const entry = dialog.getByTestId('comment-entry').last();
  await expect(entry).toContainText('A held reply.');
  const after = (await entry.boundingBox())!;
  expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.height - before.height)).toBeLessThanOrEqual(1);
});

test('anchored dialog: a refused reply becomes the failed row and Retry records it once', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnchoredThread(page);
  await input.pressSequentially('A refused reply.');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page).getByRole('alert')).toContainText(text.messageUnsent);
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(echo(page)).toHaveCount(0);
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(2);
  await expect(dialog.getByTestId('comment-entry').last()).toContainText('A refused reply.');
});

test('chat: a failed message survives closing and reopening Chat', async ({ page }) => {
  const page_ = (method: string) =>
    page.evaluate(async (method) => {
      const path = '/test/ask-page-browser.tsx';
      return (await import(path))[method]();
    }, method);
  await page.goto('/');
  await page_('mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  const fixtureRoot = page.locator('#ask-page-fixture');
  await (await pageAction(fixtureRoot, 'Chat')).click();
  const chat = page.getByTestId('chat-panel');
  const input = chat.getByRole('combobox', { name: 'Message' });
  await input.pressSequentially('Keep this unsent message.');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(input).toHaveText('', { useInnerText: true });
  await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
  await (await pageAction(fixtureRoot, 'Chat')).click();
  // Chat stays mounted while closed, so the row still holds the draft; Edit brings it back.
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page)).toContainText('Keep this unsent message.');
  await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(input).toHaveText('Keep this unsent message.', { useInnerText: true });
  await expect(echo(page)).toHaveCount(0);
});

test('anchored dialog: a failed reply survives closing the thread as the composer draft', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnchoredThread(page);
  await input.pressSequentially('Keep this unsent reply.');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(input).toHaveText('', { useInnerText: true });
  await page.getByRole('button', { name: 'Collapse thread', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  await page.getByTestId('annotation-row').first().click();
  // The thread's composer remounts, so the row's draft comes back in it.
  const reopened = page.getByRole('dialog', { name: 'Comments' });
  const composer = reopened.getByRole('combobox', { name: 'Message', exact: true });
  await expect(composer).toHaveText('Keep this unsent reply.', { useInnerText: true });
  await expect(echo(page)).toHaveCount(0);
  await expect(reopened.getByTestId('comment-entry')).toHaveCount(1);
});

test('comments: a reply that fails after Comments closed mid-send still holds its draft', async ({
  page,
}) => {
  const { page_ } = await openAnchoredThread(page);
  await page.getByRole('button', { name: 'Collapse thread', exact: true }).click();
  const fixtureRoot = page.locator('#ask-page-fixture');
  await (await pageAction(fixtureRoot, 'Comments')).click();
  await page.getByTestId('annotation-row').first().click();
  const panel = page.getByRole('dialog', { name: 'Comments' });
  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.pressSequentially('Closed while sending.');
  await page_('holdReply');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
  // Closing the thread itself waits for the send; closing Comments hides it mid-send.
  await page.getByRole('button', { name: 'Close Comments', exact: true }).click();
  await expect(panel).toHaveCount(0);
  await page_('releaseReply');
  await (await pageAction(fixtureRoot, 'Comments')).click();
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page)).toContainText('Closed while sending.');
  await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(input).toHaveText('Closed while sending.', { useInnerText: true });
  await expect(panel.getByTestId('comment-entry')).toHaveCount(1);
});
