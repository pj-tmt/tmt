import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';
import { commentsClose } from '../test/comments-region.js';

/**
 * A reply that arrives while the reader is in a thread never moves the composer under them
 * (UX ruling on #4144): the window grows upward and the new message appears above the composer.
 * Draft, caret and focus are never disturbed.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const look = process.env.COLAB_LOOK ?? '1440:light';

async function open(page: Page) {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
  await page.goto('/');
  await run(page, 'mount');
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
  const window = page.getByTestId('annotation-window');
  await expect(window).toBeVisible();
  const input = window.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeVisible();
  await input.fill('@');
  await page.getByRole('option').first().click();
  await input.fill('@Agent 1 Please read this');
  await input.press('Enter');
  await expect(page.getByTestId('comment-entry')).toHaveCount(1);
  return page.getByTestId('comment-thread');
}
const place = (target: Locator) =>
  target.evaluate((node) => {
    const box = node.getBoundingClientRect();
    return { y: Math.round(box.y), height: Math.round(box.height) };
  });

test('a reply that arrives grows the thread above a composer that stays where it is', async ({
  page,
}) => {
  const thread = await open(page);
  const composer = thread.locator('.conversation-composer');
  const input = thread.getByRole('combobox', { name: 'Message', exact: true });
  await input.click();
  await input.pressSequentially('A draft');
  const before = { composer: await place(composer), window: await place(thread) };
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`composer-before-${look.replace(':', '-')}.png`) });
  await run(page, 'windowReply');
  await expect(thread.getByText('Exact associated agent reply.')).toBeVisible();
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`composer-after-${look.replace(':', '-')}.png`) });
  const after = { composer: await place(composer), window: await place(thread) };
  expect(after.composer.y).toBe(before.composer.y);
  expect(after.composer.height).toBe(before.composer.height);
  // The window's lower edge holds too: it grew upward, never past where it settled.
  expect(
    Math.abs(after.window.y + after.window.height - (before.window.y + before.window.height)),
  ).toBeLessThanOrEqual(1);
  expect(after.window.height).toBeGreaterThan(before.window.height);
  await expect(input).toBeFocused();
  await expect(input).toHaveText('A draft', { useInnerText: true });
});

const marker = (page: Page, index: number) =>
  page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').nth(index);
async function twoThreads(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'largePage', { paragraphs: 3, threads: 2 });
  // The second thread is taller than the first one was when its reply lifted the window.
  await run(page, 'largePageReply', 1);
  await expect(marker(page, 1)).toBeVisible();
}

test('a Discussion view switched to another thread starts from that thread’s own place', async ({
  page,
}) => {
  // The control: the second thread opened with nothing before it.
  await twoThreads(page);
  await marker(page, 1).click();
  const control = await place(page.getByTestId('comment-thread'));
  // A reply lifted the first thread's Discussion view; the switch must not carry that over.
  await twoThreads(page);
  await marker(page, 0).click();
  const first = page.getByTestId('comment-thread');
  await expect(first.getByTestId('comment-entry')).toHaveCount(1);
  await run(page, 'largePageReply', 0);
  await expect(first.getByTestId('comment-entry')).toHaveCount(2);
  await commentsClose(page, 'discussion').click();
  await marker(page, 1).click();
  await expect(page.getByTestId('comment-thread')).toHaveAttribute(
    'data-thread-id',
    await marker(page, 1).evaluate((node) =>
      (node.getAttribute('data-colab-thread') ?? '').split(':').at(-1)!,
    ),
  );
  const switched = await place(page.getByTestId('comment-thread'));
  expect(Math.abs(switched.y - control.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(switched.height - control.height)).toBeLessThanOrEqual(1);
});

async function history(page: Page) {
  await page.goto('/');
  await run(page, 'mount', { history: 'thread' });
  await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
  const thread = page.getByTestId('comment-thread');
  await expect(thread).toBeVisible();
  return thread;
}
const distance = (messages: Locator) =>
  messages.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop);

test('in a long thread a reader at the bottom stays pinned and the composer holds', async ({
  page,
}) => {
  const thread = await history(page);
  const messages = thread.locator('.thread-messages');
  const composer = thread.locator('.conversation-composer');
  await expect.poll(() => distance(messages)).toBeLessThanOrEqual(1);
  const before = await place(composer);
  await run(page, 'scrollArrival');
  await expect(thread.getByTestId('comment-entry')).toHaveCount(41);
  await expect.poll(() => distance(messages)).toBeLessThanOrEqual(1);
  expect(await place(composer)).toEqual(before);
});

test('in a long thread a reader scrolled up is held and the composer holds', async ({ page }) => {
  const thread = await history(page);
  const messages = thread.locator('.thread-messages');
  const composer = thread.locator('.conversation-composer');
  await messages.evaluate((node) => {
    node.scrollTop = 150;
  });
  await messages.evaluate(
    () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  const top = await messages.evaluate((node) => node.scrollTop);
  const before = await place(composer);
  await run(page, 'scrollArrival');
  await expect(thread.getByTestId('comment-entry')).toHaveCount(41);
  await expect.poll(() => messages.evaluate((node) => node.scrollTop)).toBe(top);
  expect(await place(composer)).toEqual(before);
});
