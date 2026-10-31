import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

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

test('a send that fails after its window closed comes back with its draft and Retry records it once', async ({
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
  // The same quote again shows the failed row with its exact draft; nothing was recorded.
  await annotate(page, FIRST);
  const echo = dialog(page).getByTestId('comment-echo');
  await expect(echo).toHaveAttribute('data-echo', 'failed');
  await expect(echo).toContainText('Kept through a failure.');
  expect(await run(page, 'discussionProof')).toHaveLength(0);
  for (const width of [390, 1440])
    for (const theme of ['light', 'dark'] as const) {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.screenshot({
        path: capturePath(`annotation-failed-after-close-${width}-${theme}.png`),
      });
    }
  await echo.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(dialog(page).getByTestId('comment-entry')).toHaveCount(1);
  await expect.poll(async () => (await run(page, 'discussionProof')).length).toBe(1);
  // The other quote's draft is still its own.
  await page.keyboard.press('Escape');
  await annotate(page, SECOND);
  await expect(input(page)).toHaveText('Draft of another quote.', { useInnerText: true });
});
