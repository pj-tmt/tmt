import { commentsClose, commentsRegion } from '../test/comments-region.js';
import { pageAction, commentsAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { text } from '../src/strings.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run, selectInRenderer } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function comments(page: Page) {
  // After reload the dock's Comments action precedes the restored panel; its count is absent
  // until that projection is ready, so do not toggle before it and close the panel.
  await expect(commentsAction(page)).toHaveAccessibleName(/^Comments \(\d+\)/);
  if (!(await commentsRegion(page).isVisible())) await (await pageAction(page, 'Comments')).click();
  await expect(commentsRegion(page)).toBeVisible();
}
/** The Comments list shows resolved threads only while Show resolved is on: `Resolved` turns it on and
 * `Open` leaves it off. */
async function view(page: Page, name: 'Open' | 'Resolved') {
  const toggle = commentsRegion(page).getByRole('button', { name: text.showResolved, exact: true });
  if (((await toggle.getAttribute('aria-pressed')) === 'true') !== (name === 'Resolved'))
    await toggle.click();
}
/** Resolve and Reopen are the open thread's actions in the Comments panel header. */
const threadAction = (page: Page, name: string) =>
  commentsRegion(page).getByRole('button', { name, exact: true });
const markers = (page: Page) => page.frameLocator('iframe').locator('[data-colab-thread]');

test.afterEach(disposeActiveWorlds);
test('thread status reaches the other device and the CLI, and an agent resolution stays unseen until opened', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('status-agent');
    const author = await pairBrowser(world, 'status-author');
    const viewer = await pairBrowser(world, 'status-viewer');
    const created = createPage(
      world,
      'Status acceptance',
      '<p id="quote">Quoted passage for status.</p>',
      agent.pane,
    );
    const colab = (args: string[], pane?: string) =>
      run(world, world.binaries.colab, args, undefined, pane);
    const listing = () =>
      JSON.parse(colab(['threads', created.pageId, '--json'])) as {
        threads: { threadId?: string; id?: string; resolved: boolean }[];
      };

    // The author saves a plain annotation; no agent is asked.
    const first = await openPage(door, author, created);
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const compose = first.getByRole('dialog', { name: 'Annotate selection' });
    const input = compose.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toBeEnabled();
    await input.fill('Please check this passage.');
    await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    const window = compose.getByTestId('comment-thread');
    await expect(window).toBeVisible();
    const threadId = (await window.getAttribute('data-thread-id'))!;
    await window.getByRole('button', { name: text.threadCollapse, exact: true }).click();
    await expect(markers(first)).toHaveCount(1);

    const second = await openPage(door, viewer, created);
    await comments(second);
    const row2 = second.getByTestId('annotation-row').filter({ hasText: 'Quoted passage' });
    await expect(row2).toContainText(text.commentCount(1));
    await expect(markers(second)).toHaveCount(1);

    // A person resolves in the first browser: the marker leaves both pages, the
    // second device and the CLI read the same state, and Reopen restores them.
    await comments(first);
    await first.locator(`[data-testid=annotation-row][data-thread-id="${threadId}"]`).click();
    await threadAction(first, text.threadResolve).click();
    await expect(threadAction(first, text.threadReopen)).toBeEnabled();
    await expect(markers(first)).toHaveCount(0);
    await view(second, 'Resolved');
    await expect(row2).toContainText(text.threadResolved);
    await expect(markers(second)).toHaveCount(0);
    await expect.poll(() => listing().threads[0].resolved).toBe(true);
    await threadAction(first, text.threadReopen).click();
    await expect(markers(first)).toHaveCount(1);
    await expect(markers(second)).toHaveCount(1);
    await view(second, 'Open');
    await expect(row2).toContainText(text.commentCount(1));
    await expect.poll(() => listing().threads[0].resolved).toBe(false);
    await first.getByRole('button', { name: text.allComments, exact: true }).click();

    // An agent resolves through the native CLI publication.
    const resolved = JSON.parse(
      colab(['threads', 'resolve', created.pageId, threadId, '--json'], agent.pane),
    ) as { changed: boolean; operationId: string | null; action: { actor: string } };
    expect(resolved.changed).toBe(true);
    expect(resolved.operationId).toBeTruthy();
    expect(resolved.action.actor).toBe('agent');
    const unseen = `${text.threadResolvedBy(agent.name)} · ${text.threadUnseen}`;
    await view(second, 'Resolved');
    await expect(row2).toContainText(unseen);
    await expect(markers(second)).toHaveCount(0);
    await expect(markers(first)).toHaveCount(0);
    await expect(commentsAction(second)).toHaveAccessibleName(new RegExp(text.threadUnseen));
    // The same resolution again is a no-op: no second action is published.
    const again = JSON.parse(
      colab(['threads', 'resolve', created.pageId, threadId, '--json'], agent.pane),
    );
    expect(again.changed).toBe(false);
    expect(again.operationId).toBeNull();

    // Reload never acknowledges it; only an explicit open does, per device.
    await second.reload();
    await comments(second);
    await view(second, 'Resolved');
    const reloaded = second.getByTestId('annotation-row').filter({ hasText: 'Quoted passage' });
    await expect(reloaded).toContainText(unseen);
    await reloaded.click();
    const t2 = second.getByTestId('comment-thread').first();
    await expect(second.getByTestId('discussion-state')).toContainText(
      text.threadResolvedBy(agent.name),
    );
    await expect(t2).toHaveAttribute('data-anchor', 'resolved');
    // The panel shows the opened thread; its row reads as seen back in the list.
    await second.getByRole('button', { name: text.allComments, exact: true }).click();
    await expect(reloaded).not.toContainText(text.threadUnseen);
    await expect(commentsAction(second)).not.toHaveAccessibleName(new RegExp(text.threadUnseen));
    await second.reload();
    await comments(second);
    await view(second, 'Resolved');
    // Reload restores the Discussion view of the thread last opened; its row is in the list.
    const all = second.getByRole('button', { name: text.allComments, exact: true });
    if (await all.isVisible()) await all.click();
    await expect(
      second.getByTestId('annotation-row').filter({ hasText: 'Quoted passage' }),
    ).not.toContainText(text.threadUnseen);
    // The first device never opened it, so it is still unseen there.
    await expect(commentsAction(first)).toHaveAccessibleName(new RegExp(text.threadUnseen));

    // The agent reopens it: the marker returns on both pages.
    const reopened = JSON.parse(
      colab(['threads', 'reopen', created.pageId, threadId, '--json'], agent.pane),
    ) as { changed: boolean };
    expect(reopened.changed).toBe(true);
    await expect(markers(first)).toHaveCount(1);
    await expect.poll(() => listing().threads[0].resolved).toBe(false);
  });
});

const variants = [
  [1440, 'light'],
  [1440, 'dark'],
  [390, 'light'],
  [390, 'dark'],
] as const;
test('status states captured for the UX look', async () => {
  const captureDir = process.env.COLAB_STATUS_CAPTURE_DIR ?? test.info().outputPath('captures');
  mkdirSync(captureDir, { recursive: true });
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('status-agent');
    const author = await pairBrowser(world, 'status-author');
    const viewer = await pairBrowser(world, 'status-viewer');
    const created = createPage(
      world,
      'Status captures',
      '<p id="quote">Quoted passage for status.</p>',
      agent.pane,
    );
    const colab = (args: string[]) => run(world, world.binaries.colab, args, undefined, agent.pane);
    // Every state in the four looks, with the pointer parked away from the page so a
    // stray hover never reads as a pressed or selected state.
    const shoot = async (page: Page, state: string, options: { header?: boolean } = {}) => {
      for (const [width, theme] of variants) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await page.mouse.move(width - 1, 899);
        if (options.header && width < 600) {
          // The 390px sheet covers the dock: shoot its Comments action and its count with it closed.
          await commentsClose(page, 'list').click();
          await page.mouse.move(width - 1, 899);
          await page.screenshot({ path: `${captureDir}/${state}-dock-${width}-${theme}.png` });
          await comments(page);
        }
        await page.mouse.move(width - 1, 899);
        await page.screenshot({ path: `${captureDir}/${state}-${width}-${theme}.png` });
      }
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.evaluate(() => (document.documentElement.dataset.theme = 'light'));
    };
    const toggleCopy = async (page: Page, expected: string, state: string) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.mouse.move(0, 899);
      await expect(commentsAction(page)).toHaveAccessibleName(
        expected === text.comments
          ? 'Comments (0)'
          : expected.replace(/^Comments (\d+)/, 'Comments ($1)'),
      );
      await page.screenshot({
        path: `${captureDir}/toggle-${state}.png`,
        clip: { x: 480, y: 0, width: 960, height: 56 },
      });
    };

    const second = await openPage(door, viewer, created);
    await expect(commentsAction(second)).toBeVisible();
    await toggleCopy(second, text.comments, 'none');
    await comments(second);

    // An open thread whose text names an agent that does not exist.
    const first = await openPage(door, author, created);
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const compose = first.getByRole('dialog', { name: 'Annotate selection' });
    const input = compose.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toBeEnabled();
    await input.fill('Please check @ghost-agent this passage.');
    // Escape would cancel the whole annotation unless a candidate list is open.
    if (await first.getByRole('listbox').isVisible()) await input.press('Escape');
    await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    const window = compose.getByTestId('comment-thread');
    await expect(window).toBeVisible();
    const threadId = (await window.getAttribute('data-thread-id'))!;
    await window.getByRole('button', { name: text.threadCollapse, exact: true }).click();
    const row2 = second.getByTestId('annotation-row').filter({ hasText: 'Quoted passage' });
    await expect(row2).toContainText(text.commentCount(1));
    await toggleCopy(second, `${text.comments} 1`, 'open');
    await shoot(second, 'open');

    // A person resolves: admitted actor metadata is displayed and the unknown mention is reported.
    await comments(first);
    await first.locator(`[data-testid=annotation-row][data-thread-id="${threadId}"]`).click();
    const t1 = first.getByTestId('comment-thread').first();
    await threadAction(first, text.threadResolve).click();
    await expect(threadAction(first, text.threadReopen)).toBeEnabled();
    // A resolved thread shows its summary; the conversation, with the notice, opens on request.
    await first.getByRole('button', { name: 'Show conversation', exact: true }).click();
    const notNotified = t1.getByRole('status').filter({ hasText: text.threadNotNotified });
    await expect(notNotified).toBeVisible();
    await expect(notNotified.locator('.agent-chip')).toHaveText('ghost-agent');
    await view(second, 'Resolved');
    await expect(row2).toContainText(text.threadResolved);
    await expect(row2).toContainText(text.threadResolvedBy('status-author'));
    await shoot(first, 'person-resolved-not-notified');
    await shoot(second, 'person-resolved');
    await threadAction(first, text.threadReopen).click();
    await view(second, 'Open');
    await expect(row2).toContainText(text.commentCount(1));

    // One more open thread (a page comment), then the agent resolves the first one.
    await first.getByRole('button', { name: text.allComments, exact: true }).click();
    await first.getByRole('button', { name: '+ Comment on page', exact: true }).click();
    await first
      .getByRole('combobox', { name: 'Post comment', exact: true })
      .fill('A second, open discussion.');
    await first.getByRole('button', { name: 'Post comment', exact: true }).click();
    await expect(second.getByTestId('annotation-row')).toHaveCount(2);
    JSON.parse(colab(['threads', 'resolve', created.pageId, threadId, '--json']));
    await view(second, 'Resolved');
    await expect(row2).toContainText(`${text.threadResolvedBy(agent.name)} · ${text.threadUnseen}`);
    await toggleCopy(second, `${text.comments} 1 · ${text.threadUnseen}`, 'open-and-new');
    await shoot(second, 'agent-unseen', { header: true });
    await row2.click();
    await expect(second.getByTestId('discussion-state')).toContainText(
      text.threadResolvedBy(agent.name),
    );
    await toggleCopy(second, `${text.comments} 1`, 'open-after-seen');
    await shoot(second, 'agent-seen');
    // Keyboard focus on the panel header's own controls, shot with a margin for the ring.
    await second.keyboard.press('Shift');
    for (const name of [text.threadReopen, text.allComments]) {
      const control = threadAction(second, name);
      await control.focus();
      await second.mouse.move(0, 899);
      const box = (await control.boundingBox())!;
      await second.screenshot({
        clip: { x: box.x - 8, y: box.y - 8, width: box.width + 16, height: box.height + 16 },
        path: `${captureDir}/focus-${name === text.threadReopen ? 'reopen' : 'close'}.png`,
      });
    }
  });
});
