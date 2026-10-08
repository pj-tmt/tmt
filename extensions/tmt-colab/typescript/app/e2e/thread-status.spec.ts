import { expect, test, type Page } from '@playwright/test';

const fixture = '/test/ask-page-browser.tsx';

async function run(page: Page, method: string, argument?: string) {
  return page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );
}
async function open(page: Page) {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live preview');
  await run(page, 'seedThread');
  const host = page.locator('#ask-page-fixture');
  const toggle = host.getByTestId('comments-toggle');
  if (!(await toggle.isVisible()))
    await host.getByRole('button', { name: 'More page actions' }).click();
  await toggle.click();
  return {
    toggle,
    markers: page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
    row: page.getByTestId('annotation-row'),
    status: page.getByTestId('thread-row-status'),
  };
}
const statusActions = async (page: Page) =>
  ((await run(page, 'proof')).actions as string[]).filter((value) => value.startsWith('status:'));

test('resolving hides the margin marker, Reopen restores it, and each click writes once', async ({
  page,
}) => {
  const { markers, row, status } = await open(page);
  await expect(markers).toHaveCount(1);
  await expect(status).toHaveText('Open');
  await row.click();
  const thread = page.getByTestId('comment-thread');
  await thread.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(thread.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(markers).toHaveCount(0);
  await expect(thread).toHaveAttribute('data-anchor', 'resolved');
  await expect(status).toHaveText('Resolved');
  await expect(thread.getByText('Detached')).toHaveCount(0);
  expect(await statusActions(page)).toEqual(['status:resolve']);
  await thread.getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(thread.getByRole('button', { name: 'Resolve', exact: true })).toBeEnabled();
  await expect(markers).toHaveCount(1);
  expect(await statusActions(page)).toEqual(['status:resolve', 'status:reopen']);
});

test('an agent resolution stays unseen until the person opens the thread', async ({ page }) => {
  const { toggle, markers, row, status } = await open(page);
  await run(page, 'agentResolves');
  await expect(markers).toHaveCount(0);
  await expect(toggle).toContainText('New');
  await expect(status).toHaveText('Resolved by Atlas · New');
  // Rendering and the open panel never acknowledge it.
  expect(await run(page, 'seenProof')).toBe(0);
  await row.click();
  await expect(status).toHaveText('Resolved by Atlas');
  await expect(toggle).not.toContainText('New');
  expect(await run(page, 'seenProof')).toBe(1);
  await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-anchor', 'resolved');
});

test('a device without owner-member provenance gets no Resolve or Reopen control', async ({
  page,
}) => {
  const { row } = await open(page);
  await run(page, 'nonOwnerDevice');
  await row.click();
  const thread = page.getByTestId('comment-thread');
  await expect(thread.getByRole('button', { name: 'Close thread', exact: true })).toBeVisible();
  await expect(thread.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
  await run(page, 'agentResolves');
  await expect(thread.getByRole('button', { name: 'Reopen', exact: true })).toHaveCount(0);
  expect(await statusActions(page)).toEqual([]);
});
