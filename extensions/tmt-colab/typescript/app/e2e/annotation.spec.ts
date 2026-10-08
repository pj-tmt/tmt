import { expect, test, type Page } from '@playwright/test';
const fixture = '/test/annotation-browser.tsx';
async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function mount(page: Page, mode = 'held') {
  await page.goto('/');
  await run(page, 'mount', mode);
  const input = page.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toHaveText('', { useInnerText: true });
  return input;
}
test('a directory read Remote refused keeps the generic line and shows its code as a reference (#2170)', async ({
  page,
}) => {
  // REMOTE_STATE_UNAVAILABLE covers any Remote storage fault (its journal being full is only
  // one), so the line stays generic and the code is the reference, as for terminal failures.
  await mount(page, 'discovery-failure');
  const status = page.getByRole('status');
  const retry = page.getByRole('button', { name: 'Try again', exact: true });
  // An unattributed failure shows no reference.
  await expect(status).toHaveText('Agents are unavailable. You can still post a comment.');
  await expect(status.locator('[data-failure-reference]')).toHaveCount(0);
  await run(page, 'setDirectory', 'refuse');
  await retry.click();
  await expect(status).toContainText('Agents are unavailable. You can still post a comment.');
  await expect(status.locator('[data-failure-reference]')).toHaveText('REMOTE_STATE_UNAVAILABLE');
  await expect(status).toContainText('Code:');
  // The reference goes away with the failure.
  await run(page, 'setDirectory', 'ok');
  await retry.click();
  await expect(status).toHaveText('Posts as a comment.');
  await expect(status.locator('[data-failure-reference]')).toHaveCount(0);
});


test('the shared window awaits one admitted status action without replacing its draft or quote', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountWindow');
  const window = page.getByTestId('comment-thread');
  const input = page.getByRole('combobox', { name: 'Window draft', exact: true });
  await input.focus();
  await input.evaluate((node) => {
    (node as HTMLDivElement).dataset.retained = 'yes';
  });
  await window.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(window.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
  await expect(window.getByRole('button', { name: 'Close thread', exact: true })).toBeDisabled();
  await input.focus();
  await input.press('End');
  await input.pressSequentially(' continues');
  await expect(input).toBeFocused();
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  await expect(input).toHaveAttribute('data-retained', 'yes');
  await expect(window.locator('blockquote')).toHaveText('Frozen original quote');
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 1, closes: 0 });
  await run(page, 'finishWindowStatus');
  await expect(window.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(input).toBeFocused();
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  await window.getByRole('button', { name: 'Reopen', exact: true }).click();
  await run(page, 'finishWindowStatus', 'failed');
  await expect(window.getByRole('alert')).toBeVisible();
  await expect(window.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(input).toHaveAttribute('data-retained', 'yes');
  await expect(input).toHaveText('Unsent draft continues', { useInnerText: true });
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 2, closes: 0 });
});
test('window status controls require owner-member provenance and current admission', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mountWindow', 'readonly');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
  // Status is not writer-owned: an owner device controls another device's thread.
  await run(page, 'mountWindow', 'other-writer');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toBeEnabled();
  await run(page, 'mountWindow', 'blocked');
  await expect(page.getByRole('button', { name: 'Resolve', exact: true })).toBeDisabled();
  expect(await run(page, 'windowProof')).toEqual({ statusCalls: 0, closes: 0 });
});
