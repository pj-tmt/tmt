import { pageAction } from '../test/page-actions.js';
import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { expect, test, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { destination } from '../test/ask-fixtures.js';
import { capturePath } from './captures.js';
import {
  mount as mountAttachments,
  openChat as openAttachmentChat,
  attach as attachFiles,
  actions as attachmentActions,
  fixture as attachmentFixture,
} from '../test/attachment-page.js';
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

for (const surface of ['reply', 'chat'])
  for (const width of [390, 1440])
    for (const theme of ['light', 'dark'] as const)
      test(`${surface} agent send has a stable turn and no transient problem at ${width} ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme });
        const input = await mount(page, surface);
        await input.pressSequentially('@alpha A stable message.');
        await run(page, 'holdAskPublication');
        await page.evaluate(() => {
          const frames: { y: number; height: number; composerY: number; problem: boolean }[] = [];
          let frame = 0;
          const sample = () => {
            const turn = document.querySelector(
              '[data-testid="comment-echo"], [data-testid="comment-entry"]',
            );
            const composer = document.querySelector('.annotation-compose');
            if (turn && composer) {
              const box = turn.getBoundingClientRect();
              frames.push({
                y: box.y,
                height: box.height,
                composerY: composer.getBoundingClientRect().y,
                problem: !!document.querySelector('[data-tone="problem"]'),
              });
            }
            frame = requestAnimationFrame(sample);
          };
          frame = requestAnimationFrame(sample);
          (window as typeof window & { stopSendFrames?: () => typeof frames }).stopSendFrames =
            () => {
              cancelAnimationFrame(frame);
              return frames;
            };
        });
        await input.press('Enter');
        await expect(echo(page)).toHaveCount(1);
        const before = (await echo(page).boundingBox())!;
        const composeBefore = (await page.locator('.annotation-compose').boundingBox())!;
        await page.screenshot({
          path: capturePath(`send-${surface}-pending-${width}-${theme}.png`),
        });
        await expect.poll(async () => (await run(page, 'proof')).preparations).toBe(1);
        await run(page, 'publishRecorded');
        // The signed intent has arrived but its state publication is still in flight.
        await expect(page.locator('[data-tone="problem"]')).toHaveCount(0);
        await expect(echo(page)).toHaveCount(1);
        await run(page, 'releaseAskPublication');
        await expect(page.getByTestId('comment-entry')).toHaveCount(1);
        await expect(echo(page)).toHaveCount(0);
        await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
        const after = (await page.getByTestId('comment-entry').boundingBox())!;
        const composeAfter = (await page.locator('.annotation-compose').boundingBox())!;
        await page.screenshot({
          path: capturePath(`send-${surface}-settled-${width}-${theme}.png`),
        });
        expect(after.y).toBeCloseTo(before.y, 0);
        expect(after.height).toBeCloseTo(before.height, 0);
        expect(composeAfter.y).toBeCloseTo(composeBefore.y, 0);
        const frames = await page.evaluate(() => {
          const state = window as typeof window & {
            stopSendFrames?: () => {
              y: number;
              height: number;
              composerY: number;
              problem: boolean;
            }[];
          };
          const frames = state.stopSendFrames!();
          delete state.stopSendFrames;
          return frames;
        });
        expect(frames.length).toBeGreaterThan(0);
        for (const frame of frames) {
          expect(frame.problem).toBe(false);
          expect(frame.y).toBeCloseTo(before.y, 0);
          expect(frame.height).toBeCloseTo(before.height, 0);
          expect(frame.composerY).toBeCloseTo(composeBefore.y, 0);
        }
      });

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

/** The anchored annotation dialog on a selection, before its first comment. */
async function openAnnotation(page: Page) {
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
  return { dialog, input, page_ };
}

/** The anchored annotation dialog passes its own composer to the thread window. */
async function openAnchoredThread(page: Page) {
  const { dialog, input, page_ } = await openAnnotation(page);
  await input.pressSequentially('The first comment.');
  await input.press('Enter');
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  return { dialog, input, page_ };
}

for (const width of [1440, 390])
  test(`chat: a failed row's Retry and Edit start flush with the message text: ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 844 });
    const input = await mount(page, 'chat', 'write-failure');
    await input.pressSequentially('Kept draft.');
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    const body = (await echo(page).locator('.comment-body').boundingBox())!;
    // The label is what the eye aligns: the button's own box also holds its padding.
    const label = (name: string) =>
      echo(page).getByRole('button', { name }).locator('.tmt-ui-action-label').boundingBox();
    const retry = (await label(text.chatRetry))!;
    const edit = (await label(text.commentEdit))!;
    if (process.env.COLAB_CAPTURE_DIR) {
      await page.mouse.move(2, 2);
      await page.screenshot({ path: capturePath(`failed-row-actions-${width}.png`) });
    }
    // A text action starts where the text above it starts, not one padding in (#4452).
    expect(Math.abs(retry.x - body.x)).toBeLessThanOrEqual(1);
    // The two keep a gap between their labels.
    expect(edit.x - (retry.x + retry.width)).toBeGreaterThanOrEqual(12);
  });

test('anchored dialog: the first comment shows at once as the thread, and its record takes its place', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnnotation(page);
  await input.pressSequentially('A first comment.');
  const popover = page.locator('.annotation-popover');
  // Every change of the dialog's height from here on, with the step it came in.
  await page.evaluate(() => {
    const watch = window as unknown as { steps: { step: string; height: number }[]; step: string };
    watch.steps = [];
    watch.step = 'send';
    const node = () => document.querySelector('.annotation-popover');
    let last = Math.round(node()!.getBoundingClientRect().height);
    const tick = () => {
      const height = Math.round(node()?.getBoundingClientRect().height ?? last);
      if (Math.abs(height - last) > 1)
        watch.steps.push({ step: watch.step, height: height - last });
      last = height;
      requestAnimationFrame(tick);
    };
    tick();
  });
  await page_('holdReply');
  await input.press('Enter');
  // The thread's layout stands in the frame the composer clears: the comment as an echo.
  await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
  await expect(echo(page)).toContainText('A first comment.');
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(page.getByText('Sending…')).toHaveCount(0);
  const held = { echo: (await echo(page).boundingBox())!, dialog: (await popover.boundingBox())! };
  await page.evaluate(() => {
    (window as unknown as { step: string }).step = 'record';
  });
  await page_('releaseReply');
  const entry = dialog.getByTestId('comment-entry');
  await expect(entry).toHaveCount(1);
  await expect(echo(page)).toHaveCount(0);
  await page.waitForTimeout(400);
  // The record stands where the echo did, and the dialog moved once: at Send, none after.
  const after = (await entry.boundingBox())!;
  expect(Math.abs(after.y - held.echo.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(after.height - held.echo.height)).toBeLessThanOrEqual(1);
  const grown = (await popover.boundingBox())!;
  expect(Math.abs(grown.height - held.dialog.height)).toBeLessThanOrEqual(1);
  const steps = await page.evaluate(
    () => (window as unknown as { steps: { step: string; height: number }[] }).steps,
  );
  expect(
    steps.map(({ step }) => step),
    JSON.stringify(steps),
  ).toEqual(['send']);
  expect(steps[0].height).toBeGreaterThan(0);
});

test('anchored dialog: a first comment that fails holds its draft with Retry and Edit', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnnotation(page);
  await input.pressSequentially('Keep this first comment.');
  await page_('holdReply');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
  await page_('releaseReply');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page)).toContainText('Keep this first comment.');
  await expect(page.getByRole('alert')).toHaveCount(1);
  await expect(input).toHaveText('', { useInnerText: true });
  await expect(input).toHaveAttribute('aria-disabled', 'true');
  expect(await page_('discussionProof')).toHaveLength(0);
  // Edit returns the draft to the composer; sending it records the comment once.
  await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(echo(page)).toHaveCount(0);
  await expect(input).toHaveText('Keep this first comment.', { useInnerText: true });
  await input.press('Enter');
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  expect(await page_('discussionProof')).toHaveLength(1);
});

test('anchored dialog: Retry on a failed first comment records it once', async ({ page }) => {
  const { dialog, input, page_ } = await openAnnotation(page);
  await input.pressSequentially('Retry this first comment.');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  await expect(echo(page)).toHaveCount(0);
  const threads = (await page_('discussionProof')) as { comments: unknown[] }[];
  expect(threads).toHaveLength(1);
  expect(threads[0].comments).toHaveLength(1);
});

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

  // The thread's composer remounts, so the row's draft comes back in it.
  const reopened = commentsRegion(page);
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

  const panel = commentsRegion(page);
  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.pressSequentially('Closed while sending.');
  await page_('holdReply');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
  // Closing the thread itself waits for the send; closing Comments hides it mid-send.
  await commentsClose(page, 'discussion').click();
  await expect(panel).toHaveCount(0);
  await page_('releaseReply');
  await (await pageAction(fixtureRoot, 'Comments')).click();
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page)).toContainText('Closed while sending.');
  await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(input).toHaveText('Closed while sending.', { useInnerText: true });
  await expect(panel.getByTestId('comment-entry')).toHaveCount(1);
});

async function attachmentPage(page: Page, method: string) {
  return page.evaluate(async ({ path, method }) => (await import(path))[method](), {
    path: attachmentFixture,
    method,
  });
}

async function attachmentComposer(page: Page, surface: 'chat' | 'anchored') {
  await mountAttachments(page);
  if (surface === 'chat') return openAttachmentChat(page);
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
  const panel = page.getByRole('dialog', { name: 'Annotate selection' });
  return { panel, input: panel.getByRole('combobox', { name: 'Message', exact: true }) };
}

async function pickEchoAttachments(page: Page) {
  // Larger than the unloaded single-image placeholder: catches shrink/growth at record swap.
  const png = await page.evaluate(() => {
    const canvas = document.createElement('canvas');
    canvas.width = 160;
    canvas.height = 80;
    canvas.getContext('2d')!.fillRect(0, 0, 160, 80);
    return canvas.toDataURL('image/png').split(',')[1];
  });
  await attachFiles(page, [
    { name: 'diagram.png', mimeType: 'image/png', buffer: Buffer.from(png, 'base64') },
    { name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('The attached notes.') },
  ]);
  await expect(page.getByTestId('attachment-chip')).toHaveCount(2);
}

type AttachmentFrame = {
  kind: string;
  rows: number;
  y: number;
  height: number;
  tiles: number;
  boxes: { x: number; y: number; width: number; height: number }[];
  composer: string;
  chips: number;
};
type AttachmentFrames = { frames: AttachmentFrame[]; stop: boolean };

for (const surface of ['chat', 'anchored'] as const) {
  for (const width of [1440, 390]) {
    for (const colorScheme of ['light', 'dark'] as const) {
      test(`attachment echo ${surface} ${width} ${colorScheme}: tiles occupy their final size from Send through verification`, async ({
        page,
      }, testInfo) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme });
        const { panel, input } = await attachmentComposer(page, surface);
        await pickEchoAttachments(page);
        await input.fill('Here are the attachments.');
        await attachmentPage(page, 'holdReply');
        await page.evaluate(() => {
          const watch: AttachmentFrames = { frames: [], stop: false };
          (window as unknown as { attachmentFrames: AttachmentFrames }).attachmentFrames = watch;
          const tick = () => {
            if (watch.stop) return;
            const rows = document.querySelectorAll(
              '[data-testid="comment-echo"], [data-testid="comment-entry"]',
            );
            const row = rows[0];
            if (row || watch.frames.length) {
              const box = row?.getBoundingClientRect();
              watch.frames.push({
                kind: row?.getAttribute('data-testid') ?? 'missing',
                rows: rows.length,
                y: box?.y ?? 0,
                height: box?.height ?? 0,
                tiles: row?.querySelectorAll('[data-testid="message-attachment"]').length ?? 0,
                boxes: Array.from(
                  row?.querySelectorAll('[data-testid="message-attachment"]') ?? [],
                ).map((tile) => {
                  const rect = tile.getBoundingClientRect();
                  return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
                }),
                composer: (
                  document.querySelector('[role="combobox"]') as HTMLElement
                ).innerText.trim(),
                chips: document.querySelectorAll('[data-testid="attachment-chip"]').length,
              });
            }
            requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        });
        try {
          await input.press('Enter');
          await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
          // The DOM may commit before the next animation frame under a busy full suite.
          // Wait for the sampler, retaining its very first frame rather than taking a later one.
          await page.waitForFunction(
            () =>
              (window as unknown as { attachmentFrames: AttachmentFrames }).attachmentFrames.frames
                .length > 0,
          );
          const first = await page.evaluate(
            () =>
              (window as unknown as { attachmentFrames: AttachmentFrames }).attachmentFrames
                .frames[0],
          );
          expect(first).toMatchObject({ kind: 'comment-echo', tiles: 2, composer: '', chips: 0 });
          await expect(
            echo(page).getByRole('button', { name: 'Preview diagram.png' }),
          ).toBeDisabled();
          await expect(
            echo(page).getByRole('button', { name: text.attachmentDownload, exact: true }),
          ).toBeDisabled();
          expect(await attachmentActions(page)).not.toContain(
            'message:Here are the attachments.:2',
          );
          expect(
            (await attachmentActions(page)).filter((action) => action.startsWith('attach:open:')),
          ).toEqual([]);
          if (process.env.COLAB_CAPTURE_DIR)
            await panel.screenshot({
              path: `${process.env.COLAB_CAPTURE_DIR}/echo-${surface}-${width}-${colorScheme}.png`,
            });
          await attachmentPage(page, 'releaseReply');
          await expect(echo(page)).toHaveCount(0);
          const recorded = panel.getByTestId('comment-entry');
          await expect(recorded).toHaveCount(1);
          await expect(recorded.getByTestId('message-attachment')).toHaveCount(2);
          await expect(recorded.locator('.attachment-thumbnail img')).toBeVisible();
          await expect
            .poll(() =>
              recorded
                .locator('.attachment-thumbnail img')
                .evaluate((img) => (img as HTMLImageElement).naturalWidth),
            )
            .toBe(160);
          const frames = await page.evaluate(async () => {
            // Observe two painted frames after the authenticated image has loaded too.
            await new Promise<void>((resolve) =>
              requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
            );
            const watch = (window as unknown as { attachmentFrames: AttachmentFrames })
              .attachmentFrames;
            watch.stop = true;
            return watch.frames;
          });
          await testInfo.attach('attachment-frames', {
            body: JSON.stringify(frames),
            contentType: 'application/json',
          });
          expect(frames.some((frame) => frame.kind === 'comment-entry')).toBe(true);
          for (const frame of frames) {
            expect(frame.rows).toBe(1);
            expect(frame.tiles).toBe(2);
            expect(Math.abs(frame.y - first.y)).toBeLessThanOrEqual(1);
            expect(Math.abs(frame.height - first.height)).toBeLessThanOrEqual(1);
            frame.boxes.forEach((box, index) => {
              for (const dimension of ['x', 'y', 'width', 'height'] as const)
                expect(
                  Math.abs(box[dimension] - first.boxes[index][dimension]),
                ).toBeLessThanOrEqual(1);
            });
          }
          expect(
            (await attachmentActions(page)).filter((action) => action.startsWith('message:')),
          ).toEqual(['message:Here are the attachments.:2']);
          if (process.env.COLAB_CAPTURE_DIR)
            await panel.screenshot({
              path: `${process.env.COLAB_CAPTURE_DIR}/record-${surface}-${width}-${colorScheme}.png`,
            });
        } finally {
          await page.evaluate(() => {
            (window as unknown as { attachmentFrames: AttachmentFrames }).attachmentFrames.stop =
              true;
          });
          await attachmentPage(page, 'releaseReply');
        }
      });
    }
  }

  test(`attachment echo ${surface}: failed row retains files through Retry and Edit`, async ({
    page,
  }) => {
    const { input } = await attachmentComposer(page, surface);
    await pickEchoAttachments(page);
    await input.fill('Keep these files.');
    await attachmentPage(page, 'failNextReply');
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    await expect(echo(page).getByTestId('message-attachment')).toHaveCount(2);
    const uploads = (await attachmentActions(page)).filter((action) =>
      action.startsWith('attach:upload:'),
    );
    await attachmentPage(page, 'holdReply');
    await attachmentPage(page, 'failNextReply');
    await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(echo(page)).toHaveAttribute('data-echo', 'pending');
    await expect(echo(page).getByTestId('message-attachment')).toHaveCount(2);
    await attachmentPage(page, 'releaseReply');
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    await expect(echo(page).getByTestId('message-attachment')).toHaveCount(2);
    expect(
      (await attachmentActions(page)).filter((action) => action.startsWith('attach:upload:')),
    ).toEqual(uploads);
    await echo(page).getByRole('button', { name: 'Edit', exact: true }).click();
    await expect(echo(page)).toHaveCount(0);
    await expect(input).toHaveText('Keep these files.', { useInnerText: true });
    await expect(page.getByTestId('attachment-chip')).toHaveCount(2);
    await input.press('Enter');
    await expect(echo(page)).toHaveCount(0);
    await expect(page.getByTestId('comment-entry').getByTestId('message-attachment')).toHaveCount(
      2,
    );
    expect(
      (await attachmentActions(page)).filter((action) => action.startsWith('attach:upload:')),
    ).toEqual(uploads);
    expect(
      (await attachmentActions(page)).filter((action) => action.startsWith('message:')),
    ).toEqual(['message:Keep these files.:2']);
  });
}
