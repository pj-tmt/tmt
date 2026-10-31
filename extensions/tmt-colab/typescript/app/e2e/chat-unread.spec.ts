import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/reply-notifications-browser.ts';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
const action = (page: Page) =>
  page.locator('.page-floating-actions').getByRole('button', { name: /^Chat(?: · Unread)?$/ });
async function mount(page: Page) {
  await page.goto('/test/reply-notifications.html');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await expect(action(page)).toBeVisible();
}
for (const width of [320, 390, 1440])
  for (const theme of ['light', 'dark'] as const) {
    test.describe(`${width} ${theme}`, () => {
      test.use({ viewport: { width, height: 900 }, colorScheme: theme });
      test('one press opens at end; unread survives toast dismissal and is cleared by reading, with focus return', async ({
        page,
      }) => {
        await mount(page);
        const select = page.getByRole('button', { name: 'Selection mode', exact: true });
        const original = await page.locator('.page-floating-actions').boundingBox();
        await run(page, 'arrive', { destination: 'Chat', foreign: true });
        await expect(action(page)).toHaveAccessibleName('Chat');
        await run(page, 'arrive', { destination: 'Thread', human: true });
        await expect(action(page)).toHaveAccessibleName('Chat');
        await page.getByRole('button', { name: 'Dismiss Thread reply', exact: true }).click();
        await run(page, 'arrive', { destination: 'Chat', body: 'Unread Chat arrival' });
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        await expect(action(page).locator('.chat-unread')).toHaveAttribute('aria-hidden', 'true');
        await expect(action(page).locator('.chat-unread')).toHaveText('');
        const unreadButton = (await action(page).boundingBox())!;
        const dot = (await action(page).locator('.chat-unread').boundingBox())!;
        expect(unreadButton.width).toBe(40);
        expect(unreadButton.height).toBe(40);
        expect(dot.width).toBe(8);
        expect(dot.height).toBe(8);
        expect(dot.y - unreadButton.y).toBe(8);
        expect(unreadButton.x + unreadButton.width - dot.x - dot.width).toBe(8);
        await expect(action(page).locator('.chat-unread')).toHaveCSS('outline-width', '2px');
        expect(dot.x).toBeGreaterThan(unreadButton.x + unreadButton.width / 2);
        expect(dot.y + dot.height).toBeLessThan(unreadButton.y + unreadButton.height / 2 + 1);
        await action(page).hover();
        await expect(
          page
            .locator('.tmt-ui-icon-action-tooltip:popover-open')
            .filter({ hasText: 'Chat · Unread' }),
        ).toBeVisible();
        await page.getByRole('button', { name: 'Dismiss Chat reply', exact: true }).click();
        await expect(page.locator('.tmt-ui-notification')).toHaveCount(0);
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        // The decorative dot never changes the dock layout.
        const after = await page.locator('.page-floating-actions').boundingBox();
        expect(after!.width).toBeCloseTo(original!.width, 0);
        expect(after!.x).toBeCloseTo(original!.x, 0);
        expect(after!.y).toBeCloseTo(original!.y, 0);
        const group = await page.locator('.page-floating-actions').boundingBox();
        expect(group!.x).toBeGreaterThanOrEqual(0);
        expect(group!.x + group!.width).toBeLessThanOrEqual(width);
        await page.screenshot({ path: capturePath(`chat-unread-${width}-${theme}.png`) });
        await action(page).click();
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        await expect(page.locator('.page-floating-actions .chat-unread')).toHaveCount(0);
        const input = page
          .getByTestId('chat-panel')
          .getByRole('combobox', { name: 'Message', exact: true });
        await input.fill('Keep this exact unsent draft.');
        const editor = await input.elementHandle();
        await page.screenshot({ path: capturePath(`chat-open-${width}-${theme}.png`) });
        await page.keyboard.press('Escape');
        await expect(page.getByTestId('chat-panel')).toBeHidden();
        await expect(action(page)).toBeFocused();
        await page.screenshot({ path: capturePath(`chat-cleared-${width}-${theme}.png`) });
        await action(page).click();
        await expect(page.locator('.tmt-ui-panel[data-panel="chat"]')).toBeVisible();
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        await expect(input).toHaveText('Keep this exact unsent draft.');
        expect(await input.evaluate((node, original) => node === original, editor)).toBe(true);
        // The dock and page remain interactive; closing Chat preserves its draft.
        await expect(action(page)).toBeVisible();
        await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
        await expect(page.getByTestId('chat-panel')).toBeHidden();
        await expect(action(page)).toBeFocused();
        await select.click();
        await expect(action(page)).toHaveCount(0);
        await page.keyboard.press('Escape');
        await expect(action(page)).toBeVisible();
      });
      test('closed retained history and reading older messages cannot clear a new reply; position survives reload', async ({
        page,
      }) => {
        await mount(page);
        await run(page, 'history', 'Chat');
        await action(page).click();
        const history = page.getByTestId('chat-panel').locator('.conversation-messages');
        await expect
          .poll(() =>
            history.evaluate((node) => node.scrollHeight - node.clientHeight - node.scrollTop),
          )
          .toBeLessThanOrEqual(24);
        await history.evaluate((node) => {
          node.scrollTop = 0;
          node.dispatchEvent(new Event('scroll', { bubbles: true }));
        });
        await run(page, 'arrive', {
          destination: 'Chat',
          body: 'Arrived while reading older history',
        });
        await expect(page.locator('.page-floating-actions .chat-unread')).toHaveCount(1);
        await page.keyboard.press('Escape');
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        await run(page, 'arrive', {
          destination: 'Chat',
          body: 'Closed mounted history cannot read this',
        });
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        await action(page).click();
        await expect(page.locator('.page-floating-actions .chat-unread')).toHaveCount(0);
        await page.keyboard.press('Escape');
        await page.reload();
        await run(page, 'mount');
        await expect(action(page)).toBeVisible();
        // Re-deliver the same verified reply identities after reloading the app.
        await run(page, 'arrive', { destination: 'Chat' });
        await run(page, 'arrive', { destination: 'Chat' });
        await expect(action(page)).toHaveAccessibleName('Chat');
        await run(page, 'arrive', { destination: 'Chat', body: 'New identity after reload' });
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        await expect(page.getByTestId('chat-panel')).toHaveCount(0);
      });
    });
  }
