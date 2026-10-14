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
        const original = await select.boundingBox();
        await run(page, 'arrive', { destination: 'Chat', foreign: true });
        await expect(action(page)).toHaveAccessibleName('Chat');
        await run(page, 'arrive', { destination: 'Thread', human: true });
        await expect(action(page)).toHaveAccessibleName('Chat');
        await page.getByRole('button', { name: 'Dismiss Thread reply', exact: true }).click();
        await run(page, 'arrive', { destination: 'Chat', body: 'Unread Chat arrival' });
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        await expect(action(page)).toContainText('Unread');
        await page.getByRole('button', { name: 'Dismiss Chat reply', exact: true }).click();
        await expect(page.locator('.tmt-ui-notification')).toHaveCount(0);
        await expect(action(page)).toHaveAccessibleName('Chat · Unread');
        const after = await select.boundingBox();
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
        await expect(page.locator('.tmt-ui-drawer[data-panel="chat"]')).toHaveJSProperty(
          'open',
          true,
        );
        await expect(page.getByTestId('chat-panel')).toBeVisible();
        await expect(input).toHaveText('Keep this exact unsent draft.');
        expect(await input.evaluate((node, original) => node === original, editor)).toBe(true);
        const box = await action(page).boundingBox();
        if (width > 640) {
          // Exposed desktop location dismisses through the native backdrop only.
          await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2);
        } else {
          // Compact modality covers/inerts the original control, not a duplicate
          // over the composer. Close is the reachable pointer dismissal action.
          expect(
            await action(page).evaluate((button) => {
              button.focus();
              return {
                modal: !!document.querySelector('.tmt-ui-drawer[data-panel="chat"]:modal'),
                floatingFocused: document.activeElement === button,
                focusInside: !!document.activeElement?.closest('.tmt-ui-drawer[data-panel="chat"]'),
              };
            }),
          ).toEqual({ modal: true, floatingFocused: false, focusInside: true });
          // Role locators query DOM semantics, not Chromium's native inertness.
          // The focus attempt above proves containment; no duplicate is mounted.
          await expect(page.getByRole('button', { name: /^Chat(?: · Unread)?$/ })).toHaveCount(1);
          await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
        }
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
