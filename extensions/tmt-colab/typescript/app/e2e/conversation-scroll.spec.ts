import { pageAction } from '../test/page-actions.js';
import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/ask-page-browser.tsx';
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
// An existing thread opens as the Discussion view in the Comments drawer, whose single scroller
// comments-panel-scroll.spec.ts covers; the history scroller here is Chat's.
async function mount(page: Page) {
  await page.goto('/');
  await run(page, 'mount', { history: 'chat' });
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live');
  const toggle = await pageAction(host, 'Chat');
  await toggle.click();
  const window = page.getByTestId('chat-panel');
  await expect(window).toBeVisible();
  return window;
}
const distance = (history: Locator) =>
  history.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop);
async function scroll(history: Locator, top: number | 'bottom') {
  await history.evaluate((node, top) => {
    node.scrollTop = top === 'bottom' ? node.scrollHeight : top;
    // Wait for the native scroll event rather than depending on a fixture sleep.
  }, top);
  await history.evaluate(
    () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
}
async function caret(input: Locator) {
  return input.evaluate(() => {
    const selection = getSelection();
    return { anchor: selection?.anchorOffset, focus: selection?.focusOffset };
  });
}

for (const width of [1440, 390]) {
  for (const theme of ['light', 'dark'] as const) {
    for (const surface of ['chat'] as const) {
      test(`${surface} preserves reading and follows arrivals at ${width} ${theme}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme });
        const window = await mount(page);
        const history = window.locator('.conversation-messages');
        const jump = window.getByRole('button', { name: 'New messages', exact: true });
        const input = window.getByRole('combobox', { name: 'Message', exact: true });
        await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
        expect(await history.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
        await run(page, 'scrollArrival');
        await expect(window.getByTestId('comment-entry')).toHaveCount(41);
        await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
        await expect(jump).toHaveCount(0);

        await input.fill('Exact unsent draft');
        await input.press('ArrowLeft');
        const inputId = await input.getAttribute('id');
        const selection = await caret(input);
        await scroll(history, 150);
        const position = await history.evaluate((node) => node.scrollTop);
        await run(page, 'scrollArrival');
        await expect(window.getByTestId('comment-entry')).toHaveCount(42);
        await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(position);
        await expect(jump).toBeVisible();
        await expect(input).toBeFocused();
        await expect(input).toHaveText('Exact unsent draft');
        expect(await input.getAttribute('id')).toBe(inputId);
        expect(await caret(input)).toEqual(selection);
        await input.dispatchEvent('compositionstart');
        const status = window.locator('.conversation-arrivals');
        await expect(status).toHaveAttribute('role', 'status');
        await expect(status).toHaveAttribute('aria-live', 'polite');
        await status.evaluate((node) => {
          (globalThis as typeof globalThis & { arrivalRegion?: Element }).arrivalRegion =
            node.firstElementChild!;
        });
        await run(page, 'scrollArrival', 'reply');
        await expect(window.getByTestId('ask-reply')).toHaveText('A newly admitted answer.');
        expect(await history.evaluate((node) => node.scrollTop)).toBe(position);
        await expect(input).toBeFocused();
        expect(await caret(input)).toEqual(selection);
        await input.dispatchEvent('compositionend');
        expect(
          await status.evaluate(
            (node) =>
              node.firstElementChild ===
              (globalThis as typeof globalThis & { arrivalRegion?: Element }).arrivalRegion,
          ),
        ).toBe(true);
        await run(page, 'unrelatedWindowUpdate');
        await run(page, 'updateWindowReply');
        await expect(window.getByTestId('ask-reply')).toHaveText('Updated associated agent reply.');
        expect(await history.evaluate((node) => node.scrollTop)).toBe(position);
        await jump.focus();
        await jump.press('Enter');
        await expect(jump).toHaveCount(0);
        await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
        await expect(history).toBeFocused();
        await expect(input).toHaveText('Exact unsent draft');

        await scroll(history, 150);
        await run(page, 'scrollArrival');
        await expect(jump).toBeVisible();
        await jump.focus();
        await scroll(history, 'bottom');
        await expect(jump).toHaveCount(0);
        await expect(history).toBeFocused();
        await scroll(history, 150);
        await run(page, 'scrollArrival');
        await expect(jump).toBeVisible();
        await jump.click();
        await expect(jump).toHaveCount(0);
        await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);

        await scroll(history, 150);
        await run(page, 'unrelatedWindowUpdate');
        await run(page, 'updateWindowReply');
        await expect(jump).toHaveCount(0);
        expect(await history.evaluate((node) => node.scrollTop)).toBe(150);

        // Local publication must follow our turn before Remote preparation completes.
        await scroll(history, 150);
        await input.fill('@Agent 1 My own recorded turn');
        await run(page, 'pausePrepare');
        try {
          await input.press('Enter');
          await expect(window.getByTestId('comment-entry').last()).toContainText(
            'My own recorded turn',
          );
          await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
          await expect(jump).toHaveCount(0);
          await expect(input).toBeFocused();
          expect((await run(page, 'proof')).sends).toHaveLength(0);
        } finally {
          await run(page, 'resumePrepare');
        }
        await expect(input).toHaveText('');
        // An agent reply carries the asker's writer, but is not our own Send.
        await scroll(history, 150);
        await run(page, 'scrollArrival', 'reply');
        await expect(jump).toBeVisible();
        expect(await history.evaluate((node) => node.scrollTop)).toBe(150);
      });

      test(`${surface} scroll captures at ${width} ${theme}`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ colorScheme: theme });
        const window = await mount(page);
        const history = window.locator('.conversation-messages');
        await scroll(history, 'bottom');
        await page.screenshot({
          path: capturePath(`scroll-${surface}-${width}-${theme}-bottom.png`),
        });
        await scroll(history, 150);
        await run(page, 'scrollArrival');
        await expect(window.getByTestId('comment-entry')).toHaveCount(41);
        // The before capture also shows the reading position, without the new action.
        await scroll(history, 150);
        await page.screenshot({
          path: capturePath(`scroll-${surface}-${width}-${theme}-reading.png`),
        });
      });
    }
  }
}

test('Chat follows a layout clamp and viewport shrink before the queued scroll', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const window = await mount(page);
  const history = window.locator('.conversation-messages');
  const layout = await history.evaluate((node) => {
    const content = node.firstElementChild as HTMLElement;
    content.style.paddingBottom = '600px';
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event('scroll'));
    const before = node.scrollTop;
    const height = node.clientHeight;
    // Shrinking content clamps the bottom offset, then the viewport shrinks before
    // that native scroll event reaches React or ResizeObserver can follow again.
    content.style.paddingBottom = '553px';
    const clamped = node.scrollTop;
    node.style.maxHeight = `${height - 200}px`;
    const gap = node.scrollHeight - node.clientHeight - node.scrollTop;
    node.dispatchEvent(new Event('scroll'));
    return { before, clamped, gap };
  });
  expect(layout.before - layout.clamped).toBe(47);
  expect(layout.gap).toBe(200);
  await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);

  await history.hover();
  const bottom = await history.evaluate((node) => node.scrollTop);
  await page.mouse.wheel(0, -300);
  await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(bottom - 300);
  const reading = await history.evaluate((node) => node.scrollTop);
  await run(page, 'resizeHistory');
  await expect(window.getByTestId('comment-entry').last()).toContainText('More detail');
  await expect.poll(() => history.evaluate((node) => node.scrollTop)).toBe(reading);
  await expect(window.getByRole('button', { name: 'New messages', exact: true })).toHaveCount(0);
});

for (const surface of ['chat'] as const) {
  test(`${surface} bottom threshold, resize and reopen`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    const window = await mount(page);
    const history = window.locator('.conversation-messages');
    const jump = window.getByRole('button', { name: 'New messages', exact: true });
    for (const offset of [24, 25]) {
      await scroll(history, 'bottom');
      await scroll(history, (await history.evaluate((node) => node.scrollTop)) - offset);
      const position = await history.evaluate((node) => node.scrollTop);
      await run(page, 'scrollArrival');
      if (offset === 24) {
        await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
        await expect(jump).toHaveCount(0);
      } else {
        await expect(jump).toBeVisible();
        expect(await history.evaluate((node) => node.scrollTop)).toBe(position);
      }
    }
    await jump.click();
    await run(page, 'resizeHistory');
    await expect(window.getByTestId('comment-entry').last()).toContainText('More detail');
    await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
    await page.setViewportSize({ width: 1440, height: 700 });
    await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
    await scroll(history, 150);
    await run(page, 'resizeHistory');
    await page.setViewportSize({ width: 1440, height: 800 });
    expect(await history.evaluate((node) => node.scrollTop)).toBe(150);
    await expect(jump).toHaveCount(0);
    await run(page, 'scrollArrival');
    await expect(jump).toBeVisible();
    await scroll(history, 0);
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await expect(window).not.toBeVisible();
    const host = page.locator('#ask-page-fixture');
    const toggle = await pageAction(host, 'Chat');
    await toggle.click();
    await expect(window).toBeVisible();
    await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
    await expect(jump).toHaveCount(0);
    // If the viewport now fits the entire history, every turn is already latest.
    await scroll(history, 0);
    // The viewport is the history's own height plus the drawer's chrome: a fixed 7000px stopped
    // fitting once the turns gained their bubbles on a font with taller lines.
    const fitting = (await history.evaluate((node) => node.scrollHeight)) + 600;
    await page.setViewportSize({ width: 1440, height: fitting });
    await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
    await run(page, 'scrollArrival');
    await expect(window.getByTestId('comment-entry').last()).toContainText('A new turn');
    await expect(jump).toHaveCount(0);
    await page.setViewportSize({ width: 1440, height: 900 });
    await expect.poll(() => distance(history)).toBeLessThanOrEqual(1);
  });
}
