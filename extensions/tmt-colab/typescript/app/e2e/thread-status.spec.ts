import { pageAction } from '../test/page-actions.js';
import { capturePath } from './captures.js';
import { expect, test, type Locator, type Page } from '@playwright/test';

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
  await expect(page.locator('#ask-page-fixture .status')).toContainText('Live');
  await run(page, 'seedThread');
  const host = page.locator('#ask-page-fixture');
  const toggle = await pageAction(host, 'Comments');
  await toggle.click();
  return {
    toggle: host.getByRole('button', { name: /^Discussion \(/ }),
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
  await expect(status).toHaveText('1 comment');
  await row.click();
  const thread = page.getByTestId('comment-thread');
  await thread.getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(thread.getByRole('button', { name: 'Reopen', exact: true })).toBeEnabled();
  await expect(markers).toHaveCount(0);
  await expect(thread).toHaveAttribute('data-anchor', 'resolved');
  await expect(status).toHaveText('1 comment · Resolved by You');
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
  await expect(toggle).toHaveAccessibleName(/New/);
  // The thread left the Open view; the Resolved tab carries the news.
  await expect(page.getByRole('tab', { name: 'Open', exact: true })).toBeVisible();
  await page.getByRole('tab', { name: '● Resolved · New', exact: true }).click();
  await expect(status).toHaveText('1 comment · Resolved by Atlas · New');
  // Rendering and the open panel never acknowledge it.
  expect(await run(page, 'seenProof')).toBe(0);
  await row.click();
  await expect(status).toHaveText('1 comment · Resolved by Atlas');
  await expect(toggle).not.toHaveAccessibleName(/New/);
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

/** The look a capture run uses, `COLAB_LOOK=390:dark`; the assertions hold in every look. */
const look = process.env.COLAB_LOOK ?? '1440:light';
const setLook = async (page: Page) => {
  const [width, scheme] = look.split(':');
  await page.setViewportSize({ width: Number(width), height: 900 });
  await page.emulateMedia({ colorScheme: scheme === 'dark' ? 'dark' : 'light' });
};
const capture = async (page: Page, name: string) => {
  if (process.env.COLAB_CAPTURE_DIR)
    await page.screenshot({ path: capturePath(`resolve-${name}-${look.replace(':', '-')}.png`) });
};

const box = async (locator: Locator) => {
  const value = await locator.boundingBox();
  if (!value) throw new Error('The thread has no box');
  return value;
};

test('Resolve shows the resolved thread in the click frame, and the record takes its place without a second change', async ({
  page,
}) => {
  await setLook(page);
  const { row, status } = await open(page);
  await row.click();
  const thread = page.getByTestId('comment-thread');
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  await expect(status).toHaveText('1 comment');
  await capture(page, 'open');
  const open_ = await box(thread);
  await run(page, 'holdStatusOnce');
  await thread.getByRole('button', { name: 'Resolve', exact: true }).click();
  // The write is held: the resolved summary is already there, at its final size.
  await expect(thread.locator('.resolved-summary')).toBeVisible();
  await expect(thread.getByTestId('comment-entry').first()).toBeHidden();
  await expect(thread.getByText('Resolved', { exact: true })).toBeVisible();
  // The Comments list row shows the same state in the same frame.
  await expect(status).toHaveText('1 comment · Resolved');
  await expect(row).toHaveAttribute('data-resolved', 'true');
  await capture(page, 'click-frame');
  const echoed = await box(thread);
  expect(echoed.height).toBeLessThan(open_.height);
  expect(await statusActions(page)).toEqual([]);
  await run(page, 'releaseStatus');
  await expect(thread).toHaveAttribute('data-anchor', 'resolved');
  await expect(thread.locator('.resolved-summary')).toBeVisible();
  await expect(status).toHaveText('1 comment · Resolved by You');
  // Focus stays on the control, which now offers Reopen.
  await expect(thread.getByRole('button', { name: 'Reopen', exact: true })).toBeFocused();
  const verified = await box(thread);
  expect(Math.abs(verified.y - echoed.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(verified.height - echoed.height)).toBeLessThanOrEqual(1);
  expect(await statusActions(page)).toEqual(['status:resolve']);
});

test('a Resolve that fails restores the open thread, reports at once and keeps focus on the control', async ({
  page,
}) => {
  await setLook(page);
  const { row, status } = await open(page);
  await row.click();
  const thread = page.getByTestId('comment-thread');
  const open_ = await box(thread);
  await run(page, 'holdStatusOnce');
  const resolve = thread.getByRole('button', { name: 'Resolve', exact: true });
  await resolve.focus();
  await resolve.click();
  await expect(thread.locator('.resolved-summary')).toBeVisible();
  await page.evaluate(
    async (fixture) => (await import(fixture)).releaseStatus({ fail: true }),
    fixture,
  );
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await expect(status).toHaveText('1 comment');
  await expect(row).not.toHaveAttribute('data-resolved');
  await expect(thread.getByRole('alert')).toBeVisible();
  await expect(thread.getByRole('button', { name: 'Resolve', exact: true })).toBeFocused();
  await capture(page, 'failed');
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  // The thread is open again at its old size or taller by the failure report, never shorter.
  expect((await box(thread)).height).toBeGreaterThanOrEqual(open_.height - 1);
  expect(await statusActions(page)).toEqual([]);
});
