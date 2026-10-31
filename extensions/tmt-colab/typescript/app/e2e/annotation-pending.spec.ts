import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { text } from '../src/strings.js';
import { commentsClose } from '../test/comments-region.js';
import { pageAction } from '../test/page-actions.js';

// #5816: an annotation's send belongs to that annotation, so its window closes while the send
// runs, another annotation opens beside it, and a failure after closing is never lost.
const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
/** Selects `[start, end)` of the fixture's one paragraph, as a reader's drag would. */
async function select(page: Page, start: number, end: number) {
  await page
    .frameLocator('#ask-page-fixture iframe')
    .locator('#selected')
    .evaluate(
      (node, [start, end]) => {
        const range = node.ownerDocument.createRange();
        range.setStart(node.firstChild!, start);
        range.setEnd(node.firstChild!, end);
        const selection = node.ownerDocument.getSelection()!;
        selection.removeAllRanges();
        selection.addRange(range);
        node.ownerDocument.dispatchEvent(new Event('selectionchange'));
      },
      [start, end],
    );
}
// "Exact selected text": two quotes of the same paragraph.
const FIRST = [0, 5] as const;
const SECOND = [6, 14] as const;
const dialog = (page: Page) =>
  page.getByRole('dialog', { name: 'Annotate selection', exact: true });
const input = (page: Page) => dialog(page).getByRole('combobox', { name: 'Message', exact: true });
async function annotate(page: Page, [start, end]: readonly [number, number], body?: string) {
  await select(page, start, end);
  await page.getByRole('button', { name: 'Annotate', exact: true }).click();
  await expect(dialog(page)).toBeVisible();
  if (body === undefined) return;
  await input(page).pressSequentially(body);
  await expect(input(page)).toHaveText(body, { useInnerText: true });
}
const send = (page: Page) => dialog(page).getByRole('button', { name: /^Send(?: · to .+)?$/ });

for (const close of ['×', 'Escape', 'outside'] as const)
  test(`a pending annotation closes by ${close}; a second one opens, keeps its text and sends`, async ({
    page,
  }) => {
    await page.goto('/');
    await run(page, 'mount');
    await annotate(page, FIRST, 'First comment, held.');
    await run(page, 'holdReply');
    await send(page).click();
    await expect(dialog(page).getByTestId('comment-echo')).toBeVisible();
    if (close === '×')
      await dialog(page)
        .getByRole('button', { name: /^(Close|Collapse)/ })
        .first()
        .click();
    else if (close === 'Escape') await page.keyboard.press('Escape');
    else await page.mouse.click(4, 600);
    await expect(dialog(page)).toHaveCount(0);
    // The second annotation is not fenced by the first send: a fresh, enabled composer.
    await annotate(page, SECOND);
    await expect(input(page)).toBeEnabled();
    await expect(input(page)).toHaveText('', { useInnerText: true });
    await input(page).pressSequentially('Second comment, while the first is held.');
    await expect(send(page)).toBeEnabled();
    await send(page).click();
    await run(page, 'releaseReply');
    // Both sends finish, each once.
    await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(2);
  });

test('a send that fails after its window closed is said on the page; Open brings back its draft and Retry records it once', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await annotate(page, FIRST, 'Kept through a failure.');
  await run(page, 'holdReply');
  await run(page, 'failNextReply');
  await send(page).click();
  await page.keyboard.press('Escape');
  await expect(dialog(page)).toHaveCount(0);
  // Another annotation opens and closes meanwhile without touching the held one.
  await annotate(page, SECOND, 'Draft of another quote.');
  await page.keyboard.press('Escape');
  await run(page, 'releaseReply');
  // The failure is said on the page without reopening anything; nothing was recorded.
  const notice = page.locator('.page-unsent').getByRole('listitem');
  await expect(page.locator('.page-unsent')).toHaveRole('alert');
  await expect(notice).toHaveCount(1);
  await expect(notice).toHaveText(/^Your comment on “Exact” was not sent\.\s*Open$/);
  await expect(dialog(page)).toHaveCount(0);
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  for (const width of [390, 1440])
    for (const theme of ['light', 'dark'] as const) {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.screenshot({
        path: capturePath(`annotation-unsent-notice-${width}-${theme}.png`),
      });
    }
  // Open shows that window with the failed row, its exact draft and Retry.
  await notice.getByRole('button', { name: 'Open', exact: true }).click();
  const echo = dialog(page).getByTestId('comment-echo');
  await expect(echo).toHaveAttribute('data-echo', 'failed');
  await expect(echo).toContainText('Kept through a failure.');
  await expect(notice).toHaveCount(0);
  for (const width of [390, 1440])
    for (const theme of ['light', 'dark'] as const) {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.screenshot({
        path: capturePath(`annotation-failed-after-close-${width}-${theme}.png`),
      });
    }
  // Closing it again names it again, and the same quote opens it too.
  await page.keyboard.press('Escape');
  await expect(notice).toHaveCount(1);
  await annotate(page, FIRST);
  await expect(echo).toHaveAttribute('data-echo', 'failed');
  await echo.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(dialog(page).getByTestId('comment-entry')).toHaveCount(1);
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
  await expect(page.locator('.page-unsent')).toHaveCount(0);
  // The other quote's draft is still its own.
  await page.keyboard.press('Escape');
  await annotate(page, SECOND);
  await expect(input(page)).toHaveText('Draft of another quote.', { useInnerText: true });
});

test('discarding the kept draft of a send that failed after closing releases its window and alert row', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount');
  await annotate(page, FIRST, 'Discard this one.');
  await run(page, 'holdReply');
  await run(page, 'failNextReply');
  await send(page).click();
  await page.keyboard.press('Escape');
  await run(page, 'releaseReply');
  await expect(page.locator('.page-unsent').getByRole('listitem')).toHaveCount(1);
  // The failed comment's draft is listed with the page's kept drafts; Discard there drops it all.
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  const saved = page.getByTestId('saved-draft');
  await expect(saved).toHaveCount(1);
  await expect(saved.locator('.saved-draft-text')).toHaveText('Discard this one.');
  await saved.getByRole('button', { name: text.savedDraftDiscard }).click();
  await expect(saved).toHaveCount(0);
  await expect(page.locator('.page-unsent')).toHaveCount(0);
  await commentsClose(page, 'empty').click();
  // The same quote opens a fresh composer: no failed row, no draft, nothing recorded.
  await annotate(page, FIRST);
  await expect(dialog(page).getByTestId('comment-echo')).toHaveCount(0);
  await expect(input(page)).toBeEnabled();
  await expect(input(page)).toHaveText('', { useInnerText: true });
  expect(await run(page, 'discussionProof')).toHaveLength(0);
});
