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

  test(`${surface}: a write failure turns the echo into the error row and restores the full draft`, async ({
    page,
  }) => {
    const input = await mount(page, surface, 'write-failure');
    await input.pressSequentially('@alpha Kept draft.');
    const draft = (await run(page, 'editingProof')).edit;
    expect(draft.mentions).toHaveLength(1);
    await input.press('Enter');
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    await expect(echo(page)).toContainText(text.commentFailed);
    // One announced alert: the composer's send notice, not a second one on the row.
    await expect(page.getByRole('alert')).toHaveCount(1);
    await expect(echo(page).locator('.comment-echo-mark')).toHaveCount(0);
    await expect(input).toHaveText('@alpha Kept draft.', { useInnerText: true });
    // Text and the bound recipient chip both come back.
    expect((await run(page, 'editingProof')).edit).toMatchObject({
      value: draft.value,
      mentions: draft.mentions,
    });
    expect(await run(page, 'proof')).toMatchObject({ writes: 1, preparations: 0, sends: [] });
    await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
    await expect.poll(() => run(page, 'proof')).toMatchObject({ writes: 2 });
    await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
    await expect(input).toHaveText('@alpha Kept draft.', { useInnerText: true });
  });

  for (const [failure, reference] of [
    ['refused', 'S10'],
    ['stale', 'S14'],
  ] as const)
    test(`${surface}: attachment refusal ${reference} restores the draft behind a failed echo`, async ({
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
      await expect(page.locator('.annotation-send-notice')).toContainText(`(ref ${reference})`);
      await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
      await expect(input).toHaveText('Kept attachment draft.', { useInnerText: true });
      expect(await run(page, 'proof')).toMatchObject({ writes: 0, sends: [] });
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

test('anchored dialog: a refused reply becomes the failed row and restores the draft', async ({
  page,
}) => {
  const { dialog, input, page_ } = await openAnchoredThread(page);
  await input.pressSequentially('A refused reply.');
  await page_('failNextReply');
  await input.press('Enter');
  await expect(echo(page)).toHaveAttribute('data-echo', 'failed');
  await expect(echo(page)).toContainText(text.commentFailed);
  await expect(input).toHaveText('A refused reply.', { useInnerText: true });
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(1);
  await echo(page).getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(echo(page)).toHaveCount(0);
  await expect(dialog.getByTestId('comment-entry')).toHaveCount(2);
  await expect(dialog.getByTestId('comment-entry').last()).toContainText('A refused reply.');
});
