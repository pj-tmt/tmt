import { pageAction, commentsAction, showResolvedOn } from '../test/page-actions.js';
import { capturePath } from './captures.js';
import { expect, test, type Locator, type Page } from '@playwright/test';

const fixture = '/test/ask-page-browser.tsx';

async function run(page: Page, method: string, argument?: unknown) {
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
    toggle: commentsAction(host),
    markers: page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
    // The Discussion view's status line, once a thread is open.
    state: page.getByTestId('discussion-state'),
  };
}
/** Resolve, or Reopen, stands in the Comments panel's header, beside the thread arrows. */
const headerActions = (page: Page) =>
  page.locator('.tmt-ui-panel[data-panel="comments"]:not([hidden]) .tmt-ui-drawer-header-actions');
const statusActions = async (page: Page) =>
  ((await run(page, 'proof')).actions as string[]).filter((value) => value.startsWith('status:'));

test('resolving hides the margin marker, Reopen restores it, and each click writes once', async ({
  page,
}) => {
  // A selected thread stays expanded through resolution.
  await showResolvedOn(page);
  const { markers, state } = await open(page);
  await expect(markers).toHaveCount(1);
  // An open thread has no state line: it says something only when it is resolved or waits for you.
  await expect(state).toHaveCount(0);
  const thread = page.getByTestId('comment-thread');
  await headerActions(page).getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(
    headerActions(page).getByRole('button', { name: 'Reopen', exact: true }),
  ).toBeEnabled();
  await expect(markers).toHaveCount(0);
  await expect(thread).toHaveAttribute('data-anchor', 'resolved');
  await expect(state).toHaveText('Resolved by You');
  await expect(thread.getByText('Detached')).toBeHidden();
  expect(await statusActions(page)).toEqual(['status:resolve']);
  await headerActions(page).getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolve', exact: true }),
  ).toBeEnabled();
  await expect(markers).toHaveCount(1);
  expect(await statusActions(page)).toEqual(['status:resolve', 'status:reopen']);
});

test('an agent resolution stays unseen until the person reopens the selected discussion', async ({
  page,
}) => {
  const { toggle, markers, state } = await open(page);
  await run(page, 'agentResolves');
  await expect(markers).toHaveCount(0);
  await expect(toggle).toHaveAccessibleName(/New/);
  expect(await run(page, 'seenProof')).toBe(1);
  await page.getByRole('button', { name: 'Close Discussion', exact: true }).click();
  await toggle.click();
  await expect(state).toHaveText('Resolved by Atlas');
  await expect(toggle).not.toHaveAccessibleName(/New/);
  expect(await run(page, 'seenProof')).toBe(2);
  await expect(page.getByTestId('comment-thread')).toHaveAttribute('data-anchor', 'resolved');
});

test('a device without owner-member provenance gets no Resolve or Reopen control', async ({
  page,
}) => {
  await open(page);
  await run(page, 'nonOwnerDevice');
  await expect(page.locator('.discussion-position')).toHaveText('1 / 1');
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolve', exact: true }),
  ).toHaveCount(0);
  await run(page, 'agentResolves');
  await expect(
    headerActions(page).getByRole('button', { name: 'Reopen', exact: true }),
  ).toHaveCount(0);
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
  await showResolvedOn(page);
  const { state } = await open(page);
  const thread = page.getByTestId('comment-thread');
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  await expect(state).toHaveCount(0);
  await capture(page, 'open');
  const open_ = await box(thread);
  await run(page, 'holdStatusOnce');
  await headerActions(page).getByRole('button', { name: 'Resolve', exact: true }).click();
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolving…', exact: true }),
  ).toHaveAttribute('aria-busy', 'true');
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolving…', exact: true }),
  ).toBeDisabled();
  // The write is held: the same conversation stays expanded.
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  // The status line shows the same state in the same frame.
  await expect(state).toHaveText('Resolving…');
  await expect(state).toHaveAttribute('data-state', 'resolved');
  await capture(page, 'click-frame');
  const echoed = await box(thread);
  expect(echoed.height).toBe(open_.height);
  expect(await statusActions(page)).toEqual([]);
  await run(page, 'releaseStatus');
  await expect(thread).toHaveAttribute('data-anchor', 'resolved');
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await expect(state).toHaveText('Resolved by You');
  // Focus stays on the control, which now offers Reopen.
  await expect(
    headerActions(page).getByRole('button', { name: 'Reopen', exact: true }),
  ).toBeFocused();
  const verified = await box(thread);
  expect(Math.abs(verified.y - echoed.y)).toBeLessThanOrEqual(1);
  expect(Math.abs(verified.height - echoed.height)).toBeLessThanOrEqual(1);
  expect(await statusActions(page)).toEqual(['status:resolve']);
});

test('a Resolve that fails restores the open thread, reports at once and keeps focus on the control', async ({
  page,
}) => {
  await setLook(page);
  const { state } = await open(page);
  const thread = page.getByTestId('comment-thread');
  const open_ = await box(thread);
  await run(page, 'holdStatusOnce');
  const resolve = headerActions(page).getByRole('button', { name: 'Resolve', exact: true });
  await resolve.focus();
  await resolve.click();
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await page.evaluate(
    async (fixture) => (await import(fixture)).releaseStatus({ fail: true }),
    fixture,
  );
  await expect(thread.locator('.resolved-summary')).toHaveCount(0);
  await expect(state).toHaveCount(0);
  await expect(thread.getByRole('alert')).toBeVisible();
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolve', exact: true }),
  ).toBeFocused();
  await capture(page, 'failed');
  await expect(thread.getByTestId('comment-entry').first()).toBeVisible();
  // The thread is open again at its old size or taller by the failure report, never shorter.
  expect((await box(thread)).height).toBeGreaterThanOrEqual(open_.height - 1);
  expect(await statusActions(page)).toEqual([]);
});

test('Reopen stays busy until confirmation and restores the resolved thread on failure', async ({
  page,
}) => {
  await showResolvedOn(page);
  const { state } = await open(page);
  await run(page, 'agentResolves');
  await run(page, 'holdStatusOnce');
  await headerActions(page).getByRole('button', { name: 'Reopen', exact: true }).click();
  const reopening = headerActions(page).getByRole('button', { name: 'Reopening…', exact: true });
  await expect(reopening).toBeDisabled();
  await expect(reopening).toHaveAttribute('aria-busy', 'true');
  await capture(page, 'reopening');
  await page.evaluate(
    async (fixture) => (await import(fixture)).releaseStatus({ fail: true }),
    fixture,
  );
  await expect(state).toHaveText('Resolved by Atlas');
  await expect(page.getByTestId('comment-thread').getByRole('alert')).toBeVisible();
  await expect(
    headerActions(page).getByRole('button', { name: 'Reopen', exact: true }),
  ).toBeEnabled();
  expect(await statusActions(page)).toEqual([]);
  await run(page, 'holdStatusOnce');
  await headerActions(page).getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(reopening).toBeDisabled();
  await run(page, 'releaseStatus');
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolve', exact: true }),
  ).toBeEnabled();
  expect(await statusActions(page)).toEqual(['status:reopen']);
});

test('a page with 500 threads can immediately Reopen while Resolve notifications are outstanding', async ({
  page,
}) => {
  await showResolvedOn(page);
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true, holdStatusNotice: true });
  await run(page, 'seedManyThreads');
  await (await pageAction(page.locator('#ask-page-fixture'), 'Comments')).click();
  await expect(page.locator('.discussion-position')).toHaveText('1 / 500');
  await expect(page.getByTestId('annotation-row')).toHaveCount(0);
  await expect(page.getByTestId('comment-thread')).toContainText('@Agent 1 Opening note');
  await headerActions(page).getByRole('button', { name: 'Resolve', exact: true }).click();
  // The notification has started but cannot finish until the explicit fixture release below.
  await expect(
    headerActions(page).getByRole('button', { name: 'Reopen', exact: true }),
  ).toBeEnabled();
  expect((await run(page, 'proof')).actions).toContain('notify:Agent 1');
  expect(await statusActions(page)).toEqual(['status:resolve']);
  await headerActions(page).getByRole('button', { name: 'Reopen', exact: true }).click();
  await expect(
    headerActions(page).getByRole('button', { name: 'Resolve', exact: true }),
  ).toBeEnabled();
  await expect(page.getByTestId('comment-thread').getByRole('alert')).toHaveCount(0);
  expect(await statusActions(page)).toEqual(['status:resolve', 'status:reopen']);
  await run(page, 'finishStatusNotice');
});
