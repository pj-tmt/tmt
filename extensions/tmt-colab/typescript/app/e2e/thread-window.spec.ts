import { expect, test, type Page } from '@playwright/test';
import { commentsAction } from '../test/page-actions.js';
import { commentsRegion } from '../test/comments-region.js';

async function run(page: Page, method: string, argument?: unknown) {
  return page.evaluate(
    async ({ method, argument }) => {
      const path = '/test/ask-page-browser.tsx';
      return (await import(path))[method](argument);
    },
    { method, argument },
  );
}
async function open(page: Page, count = 2, body = '@ghost-agent Opening note') {
  await page.goto('/');
  await run(page, 'mount');
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await run(page, 'seedThread', body);
  if (count > 1) await run(page, 'seedManyThreads', count - 1);
  await commentsAction(page).click();
  return commentsRegion(page);
}

test('dock order opens one discussion directly, with a static position and explicit stepping', async ({
  page,
}) => {
  const panel = await open(page);
  const dock = page.locator('.page-floating-actions').getByRole('button');
  await expect(dock).toHaveCount(3);
  await expect(dock.nth(0)).toHaveAccessibleName('Selection mode');
  await expect(dock.nth(1)).toHaveAccessibleName('Chat');
  await expect(dock.nth(2)).toHaveAccessibleName('Comments (2)');
  await expect(panel.getByTestId('annotation-row')).toHaveCount(0);
  await expect(panel.getByTestId('comment-thread')).toBeVisible();
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 2');
  await expect(panel.getByRole('button', { name: 'All comments', exact: true })).toHaveCount(0);
  await panel.getByRole('button', { name: 'Next', exact: true }).click();
  await expect(panel.locator('.discussion-position')).toHaveText('2 / 2');
  await expect(panel.getByRole('button', { name: 'Next', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Previous', exact: true }).click();
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 2');
});

test('Resolve retains the selected conversation, composer draft and unknown-recipient warning in Open', async ({
  page,
}) => {
  const panel = await open(page, 1);
  const thread = panel.getByTestId('comment-thread');
  await expect(thread).toBeVisible();
  const input = thread.getByRole('combobox', { name: 'Message', exact: true });
  await input.fill('Unsent draft');
  const inputNode = await input.elementHandle();
  const before = await thread.boundingBox();
  await panel.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await expect(input).toHaveText('Unsent draft');
  expect(await input.evaluate((node, original) => node === original, inputNode)).toBe(true);
  await expect(thread.getByRole('status').filter({ hasText: 'Not notified:' })).toContainText(
    'ghost-agent',
  );
  const after = await thread.boundingBox();
  expect(after?.y).toBe(before?.y);
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 1');
  await expect(panel.getByRole('button', { name: 'Next', exact: true })).toBeDisabled();
});

test('filter choices replace the list tabs and expose the empty Needs you view', async ({
  page,
}) => {
  const panel = await open(page);
  await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'Needs you', exact: true }).click();
  await expect(panel.getByTestId('comments-empty').getByRole('heading')).toHaveText(
    'Nothing waiting here.',
  );
  await expect(panel.getByRole('tab')).toHaveCount(0);
  await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
  await page.getByRole('menuitemradio', { name: 'All', exact: true }).click();
  await expect(panel.getByTestId('comment-thread')).toBeVisible();
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 2');
});

test('held Resolve changes only the state and control while retaining discussion geometry', async ({
  page,
}) => {
  const panel = await open(page, 1, 'Opening note without recipients');
  const thread = panel.getByTestId('comment-thread');
  const parts = [
    thread,
    thread.locator('.discussion-quote'),
    thread.getByTestId('comment-entry').first(),
    thread.locator('.conversation-composer'),
  ];
  await expect(parts[2]).toBeVisible();
  const before = await Promise.all(parts.map((part) => part.boundingBox()));
  await run(page, 'holdStatusOnce');
  await panel.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Resolving…', exact: true })).toBeDisabled();
  await expect(parts[2]).toBeVisible();
  expect(await Promise.all(parts.map((part) => part.boundingBox()))).toEqual(before);
  await run(page, 'releaseStatus');
  await expect(panel.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  expect(await Promise.all(parts.map((part) => part.boundingBox()))).toEqual(before);
});

test('incoming resolution is not acknowledged by rendering, but reopening the selected discussion acknowledges it', async ({
  page,
}) => {
  const panel = await open(page, 1, 'Opening note');
  await run(page, 'agentResolves');
  await expect(commentsAction(page)).toHaveAccessibleName(/New/);
  expect(await run(page, 'seenProof')).toBe(1);
  await panel.getByRole('button', { name: 'Close (Esc)', exact: true }).click();
  await commentsAction(page).click();
  await expect(commentsAction(page)).not.toHaveAccessibleName(/New/);
  expect(await run(page, 'seenProof')).toBe(2);
});

test('passive arrivals in an already open empty view never acknowledge a resolution', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(() => localStorage.setItem('colab.comments.filter', 'all'));
  await run(page, 'mount');
  await commentsAction(page).click();
  const panel = commentsRegion(page);
  await expect(panel.getByTestId('comments-empty')).toBeVisible();
  await page.evaluate(async () => {
    const path = '/test/ask-page-browser.tsx';
    const fixture = await import(path);
    fixture.proposal('detached');
    fixture.agentResolves();
  });
  await expect(panel.getByTestId('discussion-state')).toHaveText('Resolved by Atlas · just now');
  expect(await run(page, 'seenProof')).toBe(0);
  await expect(commentsAction(page)).toHaveAccessibleName(/New/);
  await panel.getByRole('button', { name: 'Close (Esc)', exact: true }).click();
  await commentsAction(page).click();
  expect(await run(page, 'seenProof')).toBe(1);
  await expect(commentsAction(page)).not.toHaveAccessibleName(/New/);
});

test('Resolve all confirms the filter snapshot, preserves Cancel and notifies a mentioned agent only once', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'seedThread', '@Agent 1 First note');
  await run(page, 'seedThread', '@Agent 1 Second note');
  await commentsAction(page).click();
  const panel = commentsRegion(page);
  const action = panel.getByRole('button', { name: 'Resolve all open (2)', exact: true });
  await action.click();
  const confirm = page.getByRole('dialog', { name: 'Resolve 2 open comments?', exact: true });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(action).toBeEnabled();
  expect(
    (await run(page, 'proof')).actions.filter((value: string) => value.startsWith('notify:')),
  ).toEqual([]);
  await action.click();
  await confirm.getByRole('button', { name: 'Resolve all', exact: true }).click();
  await expect(confirm).toBeHidden();
  await expect(
    panel.getByRole('button', { name: 'Resolve all open (0)', exact: true }),
  ).toBeDisabled();
  await expect(panel.getByRole('button', { name: 'Reopen', exact: true })).toBeVisible();
  await expect
    .poll(async () =>
      (await run(page, 'proof')).actions.filter((value: string) => value.startsWith('notify:')),
    )
    .toEqual(['notify:Agent 1']);
});

test('a confirmed Resolve-all snapshot excludes an arrival while the confirmation is open', async ({
  page,
}) => {
  const panel = await open(page, 2, 'First note');
  await panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Resolve 2 open comments?', exact: true });
  await run(page, 'seedThread', 'Arrived after confirmation opened');
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Resolve all', exact: true }).click();
  await expect(
    panel.getByRole('button', { name: 'Resolve all open (1)', exact: true }),
  ).toBeEnabled();
});

test('Resolve all remains busy until original notification attempts settle, including after a new arrival', async ({
  page,
}) => {
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, holdStatusNotice: true });
  await run(page, 'seedThread', '@Agent 1 First note');
  await commentsAction(page).click();
  const panel = commentsRegion(page);
  await panel.getByRole('button', { name: 'Resolve all open (1)', exact: true }).click();
  await page
    .getByRole('dialog', { name: 'Resolve 1 open comments?', exact: true })
    .getByRole('button', { name: 'Resolve all', exact: true })
    .click();
  await expect(panel.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await run(page, 'seedThread', 'Arrived while notification is pending');
  const action = panel.getByRole('button', { name: 'Resolve all open (1)', exact: true });
  await expect(action).toBeDisabled();
  await run(page, 'finishStatusNotice');
  await expect(action).toBeEnabled();
  expect(
    (await run(page, 'proof')).actions.filter((value: string) => value.startsWith('notify:')),
  ).toEqual(['notify:Agent 1']);
});

test('the current filter defaults to Open without reading the removed Show-resolved preference', async ({
  page,
}) => {
  await page.addInitScript(() => localStorage.setItem('colab.comments.showResolved', '1'));
  const panel = await open(page);
  await expect(panel.getByRole('button', { name: 'Thread filter', exact: true })).toHaveText(
    'Open',
  );
  await expect(panel.locator('.discussion-position')).toHaveText('1 / 2');
});

test('nested filter and batch confirmation consume Escape before the conversation', async ({
  page,
}) => {
  const panel = await open(page);
  const input = panel.getByRole('combobox', { name: 'Message', exact: true });
  await input.fill('Retained through nested Escape');
  await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
  await expect(page.getByRole('menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menu')).toBeHidden();
  await expect(panel).toBeVisible();
  await panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }).click();
  const confirmation = page.getByRole('dialog', { name: 'Resolve 2 open comments?' });
  await expect(confirmation).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(confirmation).toBeHidden();
  await expect(panel).toBeVisible();
  await expect(input).toHaveText('Retained through nested Escape');
  await expect(
    panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }),
  ).toBeEnabled();
});

test('Resolve all reports a partial failure in the existing Discussion status line', async ({
  page,
}) => {
  const panel = await open(page, 2, 'First note');
  await run(page, 'failStatusOnce');
  await panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }).click();
  await page
    .getByRole('dialog', { name: 'Resolve 2 open comments?' })
    .getByRole('button', { name: 'Resolve all', exact: true })
    .click();
  await expect(panel.locator('.discussion-status')).toContainText(
    '1 comment could not be resolved.',
  );
  await expect(panel.locator('.resolve-all-report')).toHaveCount(0);
  await expect(
    panel.getByRole('button', { name: 'Resolve all open (1)', exact: true }),
  ).toBeEnabled();
});

for (const width of [390, 1440])
  for (const theme of ['light', 'dark'] as const)
    test(`Discussion review states ${width} ${theme}`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.dataset.theme = value;
      }, theme);
      const panel = await open(page, 2, 'First note');
      await expect(panel.getByTestId('comment-thread')).toBeVisible();
      await page.screenshot({ path: info.outputPath('discussion.png') });
      await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
      await page.screenshot({ path: info.outputPath('filter.png') });
      await page.keyboard.press('Escape');
      await panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }).click();
      const confirm = page.getByRole('dialog', { name: 'Resolve 2 open comments?', exact: true });
      await expect(confirm).toBeVisible();
      await page.screenshot({ path: info.outputPath('confirm.png') });
      await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
      await panel.getByRole('button', { name: 'Resolve', exact: true }).click();
      await expect(panel.getByTestId('discussion-state')).toContainText('Resolved');
      await page.screenshot({ path: info.outputPath('resolved.png') });
      await panel.getByRole('button', { name: 'Reopen', exact: true }).click();
      await run(page, 'failStatusOnce');
      await panel.getByRole('button', { name: 'Resolve all open (2)', exact: true }).click();
      await page
        .getByRole('dialog', { name: 'Resolve 2 open comments?' })
        .getByRole('button', { name: 'Resolve all', exact: true })
        .click();
      await expect(panel.locator('.discussion-status')).toContainText(
        '1 comment could not be resolved.',
      );
      await page.screenshot({ path: info.outputPath('error.png') });
      await panel.getByRole('button', { name: 'Thread filter', exact: true }).click();
      await page.getByRole('menuitemradio', { name: 'Needs you', exact: true }).click();
      await expect(panel.getByTestId('comments-empty')).toBeVisible();
      await page.screenshot({ path: info.outputPath('empty.png') });
      await panel.getByRole('button', { name: 'Close (Esc)', exact: true }).click();
      await page.screenshot({ path: info.outputPath('dock.png') });
    });
