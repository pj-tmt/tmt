import { expect, test, type Page } from '@playwright/test';
import { capturePath } from './captures.js';

const fixture = '/test/send-identity-browser.tsx';
const anonymous = {
  state: 'anonymous',
  reason: 'USER_NAME_UNAVAILABLE',
  message:
    'USER_NAME_UNAVAILABLE: no login name; set one with tmt config set user.name <name> and send again.',
};
const renamed = { state: 'person', name: 'ben-user', renamedFrom: 'ben' };
async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
const line = (page: Page) => page.getByTestId('send-identity');

test('a refused name is told once: the command is selectable code and Dismiss removes the line', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await run(page, 'mountChat', anonymous);
  await expect(line(page)).toContainText(
    'Sent without your name. USER_NAME_UNAVAILABLE: no login name;',
  );
  await expect(line(page).locator('code')).toHaveText('tmt config set user.name <name>');
  await expect(line(page)).toHaveAttribute('role', 'status');
  await line(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: capturePath('send-identity-anonymous-390.png') });
  // A later send in the same page session does not bring it back, even after a dismissal.
  await run(page, 'sendAgain', renamed);
  await expect(line(page)).toHaveCount(1);
  await line(page).getByRole('button', { name: 'Dismiss' }).click();
  await expect(line(page)).toHaveCount(0);
  await run(page, 'sendAgain', anonymous);
  await expect(line(page)).toHaveCount(0);
});

test('a name an agent already holds is told without claiming a rename', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await run(page, 'mountChat', renamed);
  await expect(line(page)).toContainText('Sent as ben-user: an agent already uses ben.');
  await expect(line(page)).not.toContainText('was');
  await line(page).scrollIntoViewIfNeeded();
  await page.screenshot({ path: capturePath('send-identity-renamed-390.png') });
});

test('an older Remote says nothing, so the page shows nothing', async ({ page }) => {
  await page.goto('/');
  await run(page, 'mountChat');
  await expect(page.getByTestId('chat-panel')).toBeVisible();
  await expect(line(page)).toHaveCount(0);
});
