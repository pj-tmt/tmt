import { expect, test, type Page } from '@playwright/test';
import { commentsRegion } from '../test/comments-region.js';
import { pageAction } from '../test/page-actions.js';

/**
 * An admitted update on a large page re-renders what changed. A reply to one thread updates that
 * thread; the document, its highlights and the other threads are not touched again.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const frame = (page: Page) => page.frameLocator('#ask-page-fixture iframe');

/** What the parent sends the document frame, by type: the cost of "re-rendering the page". */
async function countFrameMessages(page: Page) {
  await page.addInitScript(() => {
    const counts: Record<string, number> = {};
    Object.defineProperty(window, 'frameMessages', { value: counts });
    const post = MessagePort.prototype.postMessage;
    MessagePort.prototype.postMessage = function (
      this: MessagePort,
      message: unknown,
      ...rest: never[]
    ) {
      const type = (message as { type?: string } | null)?.type;
      if (type?.startsWith('colab.render.')) counts[type] = (counts[type] ?? 0) + 1;
      return post.call(this, message, ...(rest as []));
    } as typeof post;
  });
}
const frames = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
/** Returns the counts once the frame has been sent nothing for several frames in a row. */
async function quiet(page: Page) {
  let last = '';
  for (let steady = 0, tries = 0; steady < 5; tries++) {
    if (tries > 200) throw new Error('The page never went quiet');
    await frames(page);
    const now = JSON.stringify(await messages(page));
    steady = now === last ? steady + 1 : 0;
    last = now;
  }
  return JSON.parse(last) as Record<string, number>;
}
const messages = (page: Page) =>
  page.evaluate(() => ({
    ...(window as unknown as { frameMessages: Record<string, number> }).frameMessages,
  }));

test('a reply on a large page updates its thread and leaves the document alone', async ({
  page,
}) => {
  await countFrameMessages(page);
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'largePage', { paragraphs: 400, threads: 120 });
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(120);
  await (await pageAction(page, 'Comments')).click();
  const drawer = commentsRegion(page);
  await drawer.getByRole('button', { name: /^8 Comment on paragraph 7\./ }).click();
  await expect(
    drawer.getByTestId('comment-entry').getByText('Comment on paragraph 7.'),
  ).toBeVisible();
  // Let the opening work settle, then count only what the reply causes.
  const before = await quiet(page);
  await run(page, 'largePageReply', 7);
  await expect(drawer.getByText('Reply 1 on paragraph 7.')).toBeVisible();
  const after = await quiet(page);
  const caused = Object.fromEntries(
    Object.keys({ ...before, ...after })
      .map((type) => [type, (after[type] ?? 0) - (before[type] ?? 0)])
      .filter(([, n]) => n),
  );
  expect(caused).toEqual({});
});

test('an update that changes no thread leaves the document alone on a large page', async ({
  page,
}) => {
  await countFrameMessages(page);
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'largePage', { paragraphs: 400, threads: 120 });
  await expect(frame(page).locator('[data-colab-thread]')).toHaveCount(120);
  const before = await quiet(page);
  // Live publishes a fresh copy of the whole projection; here only the title differs.
  await run(page, 'unrelatedWindowUpdate');
  await expect(page).toHaveTitle(/Unrelated live title/);
  expect(await quiet(page)).toEqual(before);
});
