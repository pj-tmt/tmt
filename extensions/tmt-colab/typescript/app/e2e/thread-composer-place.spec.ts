import { expect, test, type Locator, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

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
