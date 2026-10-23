import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';

/**
 * Comment and Chat bodies render a safe Markdown subset (#4112): structure for the supported syntax,
 * literal text for everything else, and never markup or a navigable unsafe link.
 */
const fixture = '/test/ask-page-browser.tsx';
const reply = [
  '**Done.** The plan is below, and `npm test` passes:',
  '',
  '- first step with *emphasis*',
  '- second step: ask **@Atlas**',
  '',
  '> a quoted note',
  '',
  '```sh',
  'echo "<b>kept</b>" # a long line that scrolls inside its own block rather than widening the window ......................',
  '```',
  '',
  '[the page](/p/deadbeef) · [outside](https://example.com/x) · [bad](javascript:alert(1)) <script>alert(1)</script>',
].join('\n');
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
async function open(page: Page, surface: 'chat' | 'thread', width: number, theme: string) {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({ colorScheme: theme as 'light' | 'dark' });
  await page.addInitScript((value) => {
    document.documentElement.dataset.theme = value;
  }, theme);
  await page.goto('/');
  await run(page, 'mount');
  await run(page, 'conversation', { surface, state: 'replied', reply });
  const host = page.locator('#ask-page-fixture');
  await expect(host.locator('.status')).toContainText('Live');
  if (surface === 'thread') {
    await page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]').click();
    await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-layout', 'discussion');
    return page.getByTestId('comment-thread');
  }
  await (await pageAction(host, 'Chat')).click();
  return page.getByTestId('chat-panel');
}

for (const width of [1440, 390])
  for (const theme of ['light', 'dark'] as const)
    for (const surface of ['chat', 'thread'] as const)
      test(`a Markdown reply renders safely in ${surface} at ${width} ${theme}`, async ({
        page,
      }) => {
        const window = await open(page, surface, width, theme);
        const body = window.locator('[data-turn-role="agent"]').getByTestId('ask-reply');
        await expect(body.locator('strong').first()).toHaveText('Done.');
        await expect(body.locator('code').first()).toHaveText('npm test');
        await expect(body.locator('ul > li')).toHaveCount(2);
        await expect(body.locator('li em')).toHaveText('emphasis');
        await expect(body.locator('blockquote')).toHaveText('a quoted note');
        // Fenced code is literal text and scrolls inside its own block.
        const code = body.locator('pre.markdown-code');
        await expect(code).toContainText('echo "<b>kept</b>"');
        await expect(code.locator('b')).toHaveCount(0);
        await expect(code).toHaveCSS('overflow-x', 'auto');
        // A mention still becomes the agent chip, inside the bold text.
        await expect(body.locator('strong .agent-chip')).toHaveCount(1);
        // One link, to a page of this space; the outside URL is words, the unsafe one is text.
        await expect(body.locator('a')).toHaveCount(1);
        await expect(body.locator('a')).toHaveAttribute('href', '/p/deadbeef');
        await expect(body).toContainText('outside (https://example.com/x)');
        await expect(body).toContainText('[bad](javascript:alert(1))');
        await expect(body).toContainText('<script>alert(1)</script>');
        await expect(window.locator('script, img, [href^="javascript"]')).toHaveCount(0);
        // The body never widens its window.
        expect(await window.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(
          true,
        );
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
          true,
        );
        if (process.env.COLAB_CAPTURE_DIR)
          await page.screenshot({ path: capturePath(`markdown-${surface}-${width}-${theme}.png`) });
      });

test('a page link opens in the app without reloading; a modified click stays the browser’s', async ({
  page,
}) => {
  const window = await open(page, 'chat', 1440, 'light');
  const link = window.getByTestId('ask-reply').locator('a.markdown-link');
  await expect(link).toHaveAttribute('href', '/p/deadbeef');
  await page.evaluate(() => {
    Object.assign(globalThis, { appMarker: 'still here' });
    // A click the app handled is already prevented when it bubbles to the document. The listener
    // records that, then cancels the browser's own action so a modified click opens nothing.
    document.addEventListener('click', (event) => {
      Object.assign(globalThis, { clickPrevented: event.defaultPrevented });
      event.preventDefault();
    });
  });
  await link.click({ modifiers: ['Shift'] });
  expect(
    await page.evaluate(() => (globalThis as { clickPrevented?: boolean }).clickPrevented),
  ).toBe(false);
  await link.click();
  expect(
    await page.evaluate(() => (globalThis as { clickPrevented?: boolean }).clickPrevented),
  ).toBe(true);
  // The document was not replaced: the marker survives and the route moved to the short link.
  await expect
    .poll(() => page.evaluate(() => (globalThis as { appMarker?: string }).appMarker))
    .toBe('still here');
  await expect
    .poll(() => page.evaluate(() => location.pathname + location.hash))
    .toContain('deadbeef');
});
