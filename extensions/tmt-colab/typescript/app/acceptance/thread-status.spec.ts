import { commentsRegion } from '../test/comments-region.js';
import { pageAction, commentsAction } from '../test/page-actions.js';
import { expect, test, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { text } from '../src/strings.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run, selectInRenderer } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function comments(page: Page) {
  await expect(commentsAction(page)).toHaveAccessibleName(/^Comments \(\d+\)/);
  if (!(await commentsRegion(page).isVisible())) await (await pageAction(page, 'Comments')).click();
  await expect(commentsRegion(page)).toBeVisible();
}
const threadAction = (page: Page, name: string) =>
  commentsRegion(page).getByRole('button', { name, exact: true });
const markers = (page: Page) => page.frameLocator('iframe').locator('[data-colab-thread]');
const state = (page: Page) => page.getByTestId('discussion-state');
async function annotate(page: Page, body: string, selector = '#quote') {
  await selectInRenderer(page, selector);
  await page.getByTestId('selection-ask').click();
  const compose = page.getByRole('dialog', { name: 'Annotate selection' });
  const input = compose.getByRole('combobox', { name: 'Message', exact: true });
  await expect(input).toBeEnabled();
  await input.fill(body);
  if (await page.getByRole('listbox').isVisible()) await input.press('Escape');
  await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
  const window = compose.getByTestId('comment-thread');
  await expect(window).toBeVisible();
  const id = (await window.getAttribute('data-thread-id'))!;
  await window.getByRole('button', { name: text.threadCollapse, exact: true }).click();
  return id;
}
async function reopenSelected(page: Page) {
  await threadAction(page, 'Close (Esc)').click();
  await commentsAction(page).click();
}

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
        threads: { resolved: boolean }[];
      };
    const first = await openPage(door, author, created);
    const threadId = await annotate(first, 'Please check this passage.');
    const second = await openPage(door, viewer, created);
    await comments(second);
    const discussion = second.getByTestId('comment-thread');
    await expect(discussion).toContainText('Please check this passage.');
    await expect(markers(second)).toHaveCount(1);
    await expect(second.getByTestId('annotation-row')).toHaveCount(0);
    await comments(first);
    await threadAction(first, text.threadResolve).click();
    await expect(threadAction(first, text.threadReopen)).toBeEnabled();
    await expect(state(second)).toContainText(text.threadResolvedBy('status-author'));
    await expect(markers(first)).toHaveCount(0);
    await expect(markers(second)).toHaveCount(0);
    await expect.poll(() => listing().threads[0].resolved).toBe(true);
    await threadAction(first, text.threadReopen).click();
    await expect(markers(first)).toHaveCount(1);
    await expect(markers(second)).toHaveCount(1);
    await expect.poll(() => listing().threads[0].resolved).toBe(false);

    const resolved = JSON.parse(
      colab(['threads', 'resolve', created.pageId, threadId, '--json'], agent.pane),
    );
    expect(resolved.changed).toBe(true);
    expect(resolved.operationId).toBeTruthy();
    expect(resolved.action.actor).toBe('agent');
    await expect(state(second)).toContainText(text.threadResolvedBy(agent.name));
    await expect(markers(second)).toHaveCount(0);
    await expect(markers(first)).toHaveCount(0);
    await expect(commentsAction(second)).toHaveAccessibleName(/New/);
    const again = JSON.parse(
      colab(['threads', 'resolve', created.pageId, threadId, '--json'], agent.pane),
    );
    expect(again.changed).toBe(false);
    expect(again.operationId).toBeNull();

    // A fragment restores the selected discussion without acknowledging a new winning status.
    await second.reload();
    await expect(commentsAction(second)).toHaveAccessibleName(/New/);
    await expect(commentsRegion(second)).toBeVisible();
    await expect(second.getByTestId('comment-thread')).toHaveAttribute('data-anchor', 'resolved');
    await reopenSelected(second);
    await expect(commentsAction(second)).not.toHaveAccessibleName(/New/);
    await second.reload();
    await expect(commentsAction(second)).toHaveAccessibleName('Comments (0)');
    await expect(commentsAction(first)).toHaveAccessibleName(/New/);
    const reopened = JSON.parse(
      colab(['threads', 'reopen', created.pageId, threadId, '--json'], agent.pane),
    );
    expect(reopened.changed).toBe(true);
    await expect(markers(first)).toHaveCount(1);
    await expect(markers(second)).toHaveCount(1);
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
      '<p id="quote">Quoted passage for status.</p><p id="quote2">Second status passage.</p>',
      agent.pane,
    );
    const colab = (args: string[]) => run(world, world.binaries.colab, args, undefined, agent.pane);
    const shoot = async (page: Page, label: string) => {
      await expect(
        page.locator('.annotation-helper').filter({ hasText: 'Checking for agents' }),
      ).toHaveCount(0);
      for (const [width, theme] of variants) {
        await page.setViewportSize({ width, height: 900 });
        await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await page.mouse.move(width - 1, 899);
        await page.screenshot({
          path: `${captureDir}/${label}-${width}-${theme}.png`,
          animations: 'disabled',
        });
      }
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.evaluate(() => (document.documentElement.dataset.theme = 'light'));
    };
    const second = await openPage(door, viewer, created);
    await expect(commentsAction(second)).toHaveAccessibleName('Comments (0)');
    await comments(second);
    await expect(second.getByTestId('comments-empty')).toBeVisible();
    await shoot(second, 'empty');
    const first = await openPage(door, author, created);
    const threadId = await annotate(first, 'Please check @ghost-agent this passage.');
    await expect(second.getByTestId('comment-thread')).toContainText('Please check');
    await shoot(second, 'open');
    await threadAction(second, text.commentsFilter).click();
    await shoot(second, 'filter');
    await second.keyboard.press('Escape');
    await comments(first);
    const t1 = first.getByTestId('comment-thread');
    const input = t1.getByRole('combobox', { name: 'Message', exact: true });
    await input.fill('Retained unsent reply');
    const original = await input.elementHandle();
    await threadAction(first, text.threadResolve).click();
    await expect(threadAction(first, text.threadReopen)).toBeEnabled();
    await expect(t1.getByTestId('comment-entry').first()).toBeVisible();
    await expect(t1.locator('.resolved-summary')).toHaveCount(0);
    const warning = t1.getByRole('status').filter({ hasText: text.threadNotNotified });
    await expect(warning).toBeVisible();
    await expect(warning.locator('.agent-chip')).toHaveText('ghost-agent');
    await expect(input).toHaveText('Retained unsent reply');
    expect(await input.evaluate((node, original) => node === original, original)).toBe(true);
    await expect(state(second)).toContainText(text.threadResolvedBy('status-author'));
    await shoot(first, 'person-resolved-not-notified');
    await shoot(second, 'person-resolved');
    await threadAction(first, text.threadReopen).click();
    await threadAction(first, 'Close (Esc)').click();
    await annotate(first, 'A second, open discussion.', '#quote2');
    await expect(commentsAction(second)).toHaveAccessibleName('Comments (2)');
    JSON.parse(colab(['threads', 'resolve', created.pageId, threadId, '--json']));
    await expect(state(second)).toContainText(text.threadResolvedBy(agent.name));
    await expect(commentsAction(second)).toHaveAccessibleName('Comments (1) · New');
    await shoot(second, 'agent-unseen');
    await threadAction(second, 'Close (Esc)').click();
    await shoot(second, 'agent-unseen-dock');
    await commentsAction(second).click();
    await expect(commentsAction(second)).toHaveAccessibleName('Comments (1)');
    await shoot(second, 'agent-seen');
    for (const name of [text.threadReopen, text.discussionNext]) {
      await threadAction(second, name).focus();
      await second.mouse.move(0, 899);
      await second.screenshot({
        path: `${captureDir}/focus-${name === text.threadReopen ? 'reopen' : 'next'}.png`,
      });
    }
  });
});

test('Resolve all commits two native statuses and notifies each agent once in the retained window', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const alpha = await world.startAgent('batch-alpha');
    const beta = await world.startAgent('batch-beta');
    const author = await pairBrowser(world, 'batch-author', { talk: true });
    const created = createPage(
      world,
      'Batch status acceptance',
      '<p id="quote">First batch passage.</p><p id="quote2">Second batch passage.</p>',
    );
    const page = await openPage(door, author, created);
    await annotate(page, '@batch-alpha @batch-beta First batch comment.');
    await annotate(page, '@batch-alpha @ghost-agent Second batch comment.', '#quote2');
    // These comments dispatch ordinary Asks; count batch notifications separately.
    await expect.poll(() => alpha.received().length).toBe(2);
    await expect.poll(() => beta.received().length).toBe(1);
    const notices = (agent: typeof alpha) =>
      agent
        .received()
        .filter((value) => String(value.message).includes('Thread resolved by batch-author.'));
    await comments(page);
    const panel = commentsRegion(page);
    const action = panel.getByRole('button', { name: 'Resolve all open (2)', exact: true });
    await action.click();
    const confirmation = page.getByRole('dialog', { name: 'Resolve 2 open comments?' });
    await expect(confirmation.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
    await confirmation.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(action).toBeEnabled();
    expect(notices(alpha)).toEqual([]);
    expect(notices(beta)).toEqual([]);
    await action.click();
    await confirmation.getByRole('button', { name: 'Resolve all', exact: true }).click();
    await expect.poll(() => notices(alpha).length).toBe(1);
    await expect.poll(() => notices(beta).length).toBe(1);
    for (const agent of [alpha, beta]) {
      expect(notices(agent)[0].message).toContain('Thread resolved by batch-author.');
      expect(notices(agent)[0].message).toContain('First batch comment.');
    }
    const listing = () =>
      JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])) as {
        threads: { resolved: boolean }[];
      };
    expect(listing().threads).toHaveLength(2);
    expect(listing().threads.every((thread) => thread.resolved)).toBe(true);
    await expect(panel.getByRole('status').filter({ hasText: 'Not notified' })).toContainText(
      'ghost-agent',
    );
  });
});

test('a held annotation closes; a second one opens, keeps its text and sends, and Resolve all works meanwhile (#5816)', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const alpha = await world.startAgent('held-alpha');
    const beta = await world.startAgent('held-beta');
    const author = await pairBrowser(world, 'held-author', { talk: true });
    const created = createPage(
      world,
      'Held annotation acceptance',
      '<p id="quote">First held passage.</p><p id="quote2">Second held passage.</p>',
    );
    const page = await openPage(door, author, created);
    const compose = page.getByRole('dialog', { name: 'Annotate selection' });
    const input = compose.getByRole('combobox', { name: 'Message', exact: true });
    const write = async (selector: string, body: string) => {
      await selectInRenderer(page, selector);
      await page.getByTestId('selection-ask').click();
      await expect(input).toBeEnabled();
      await input.fill(body);
      if (await page.getByRole('listbox').isVisible()) await input.press('Escape');
      await expect(input).toHaveText(body);
    };
    const send = () =>
      compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    // Annotation 1: its first dispatch is created and then held, so its send stays pending.
    world.armNextBarrier('after');
    await write('#quote', '@held-alpha @held-beta First held comment.');
    await send();
    const parked = await world.barrierEntered();
    expect(typeof parked.operationId).toBe('string');
    world.armBarrier(parked.operationId as string, 'after');
    // The message is recorded and its dispatch is held: closing never waits for the send.
    await page.keyboard.press('Escape');
    await expect(compose).toHaveCount(0);
    // Annotation 2 opens, keeps exactly what was typed, and sends while annotation 1 is held. The
    // held dispatch keeps the core busy, so the agent directory cannot be read: it is a comment.
    await write('#quote2', 'Second comment, typed while the first is held.');
    await send();
    await expect(compose.getByTestId('comment-thread')).toBeVisible();
    await expect.poll(() => alpha.received().length).toBe(1);
    expect(beta.received()).toHaveLength(0);
    await page.keyboard.press('Escape');
    await expect(compose).toHaveCount(0);
    // Resolve all is not fenced by the held send.
    await comments(page);
    const panel = commentsRegion(page);
    const action = panel.getByRole('button', { name: 'Resolve all open (2)', exact: true });
    await expect(action).toBeEnabled();
    await action.click();
    await page
      .getByRole('dialog', { name: 'Resolve 2 open comments?' })
      .getByRole('button', { name: 'Resolve all', exact: true })
      .click();
    // The confirmation is accepted at once; the statuses commit through the core, which the held
    // dispatch keeps busy until it is released.
    await expect(page.getByRole('dialog', { name: 'Resolve 2 open comments?' })).toHaveCount(0);
    // Releasing annotation 1 finishes its delivery exactly once per recipient.
    world.releaseBarrier();
    const listing = () =>
      JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])) as {
        threads: { resolved: boolean }[];
      };
    await expect.poll(() => listing().threads.every((thread) => thread.resolved)).toBe(true);
    expect(listing().threads).toHaveLength(2);
    const ordinary = (agent: typeof alpha) =>
      agent.received().filter((value) => !String(value.message).includes('Thread resolved by'));
    await expect.poll(() => ordinary(beta).length).toBe(1);
    expect(ordinary(alpha)).toHaveLength(1);
    // Two ordinary Asks and one resolve notice per agent, each operation created once.
    const notices = (agent: typeof alpha) =>
      agent.received().filter((value) => String(value.message).includes('Thread resolved by'));
    await expect.poll(() => notices(alpha).length).toBe(1);
    await expect.poll(() => notices(beta).length).toBe(1);
    const dispatched = world
      .coreCalls()
      .filter((call) => call.operation === 'dispatch.create')
      .map((call) => call.operationId);
    expect(dispatched).toHaveLength(4);
    expect(new Set(dispatched).size).toBe(4);
  });
});
