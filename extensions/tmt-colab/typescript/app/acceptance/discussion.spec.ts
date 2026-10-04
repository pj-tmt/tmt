import { expect, test, type Locator, type Page } from '@playwright/test';
import { pairBrowser, restartColab, startDoor } from './harness/browser.js';
import { askEntry, createPage, freePort, openPage, selectInRenderer } from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function post(page: Page, body: string, selected = false) {
  if ((await page.getByTestId('comments-toggle').getAttribute('aria-expanded')) === 'false')
    await page.getByTestId('comments-toggle').click();
  if (selected) await page.getByTestId('comment-action').click();
  else await page.getByRole('button', { name: 'Comment on page', exact: true }).click();
  const panel = page.getByTestId('comments-panel');
  if (selected) {
    const over = 'é'.repeat(8193);
    await panel.getByLabel('Post comment', { exact: true }).fill(over);
    await panel.getByRole('button', { name: 'Post comment', exact: true }).click();
    await expect(panel.getByRole('alert')).toContainText('Your draft is kept');
    await expect(panel.getByLabel('Post comment', { exact: true })).toHaveValue(over);
    await expect(panel.getByTestId('comment-entry')).toHaveCount(0);
  }
  await panel.getByLabel('Post comment', { exact: true }).fill(body);
  await panel.getByRole('button', { name: 'Post comment', exact: true }).click();
  await expect(panel.locator('.comment-body').filter({ hasText: body })).toBeVisible();
}
async function edit(comment: Locator, body: string) {
  await comment.getByRole('button', { name: 'Edit', exact: true }).click();
  await comment.getByLabel('Edit comment', { exact: true }).fill(body);
  await comment.getByRole('button', { name: 'Save comment', exact: true }).click();
  await expect(comment.locator('.comment-body')).toHaveText(body);
}

test.afterEach(disposeActiveWorlds);
test('two paired writers retain discussion, anchors and frozen comment Ask through reload and restart', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('discussion-agent');
    const a = await pairBrowser(world, 'discussion-author');
    const b = await pairBrowser(world, 'discussion-replier');
    const html =
      '<h1>Shared review</h1><p id="quote">An &amp; <em>🌍 exact quote</em> for review.</p><p id="next">Another selection.</p><div style="height:2400px">Long-page scroll proof</div>';
    const created = createPage(world, 'Shared review', html);
    const first = await openPage(door, a, created);
    const second = await openPage(door, b, created);
    // The real owner app's CSP also permits trusted content-driven frame sizing.
    await expect
      .poll(async () => (await first.locator('iframe').boundingBox())?.height ?? 0)
      .toBeGreaterThan(2400);
    await expect(first.locator('iframe')).toHaveAttribute('scrolling', 'no');
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      await first.evaluate(() => window.scrollTo(0, 0));
      await first.screenshot({ path: `/tmp/1586-native-${width}-light-long-top.png` });
      await first.evaluate(() => window.scrollTo(0, 1200));
      expect(await first.evaluate(() => window.scrollY)).toBeGreaterThan(1000);
      expect(
        await first
          .frameLocator('iframe')
          .locator('html')
          .evaluate((node) => node.ownerDocument.defaultView!.scrollY),
      ).toBe(0);
      expect((await first.locator('.page-bar').boundingBox())?.y).toBe(0);
      await first.screenshot({ path: `/tmp/1586-native-${width}-light-long-scrolled.png` });
    }
    await first.setViewportSize({ width: 1280, height: 900 });
    await first.evaluate(() => window.scrollTo(0, 0));
    await second.getByTestId('comments-toggle').click();
    await selectInRenderer(first, '#quote');
    await post(first, '<script>plain discussion</script>\nPlease explain this.', true);
    const t1 = first.getByTestId('comment-thread').first();
    const t2 = second.getByTestId('comment-thread').first();
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    const threadId = await t1.getAttribute('data-thread-id');
    const messageId = await t1.getByTestId('comment-entry').getAttribute('data-message-id');
    await expect(t2.getByTestId('comment-entry')).toContainText('discussion-author');
    const byline = t2.getByTestId('comment-entry').locator('.comment-byline');
    await expect(byline).toHaveText('discussion-author · Just now');
    await expect(byline).toHaveAttribute('title', /^[a-f0-9-]{36}$/);
    await expect(byline.locator('time')).toHaveAttribute('datetime', /Z$/);
    await expect(t2.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
    await t2.getByRole('button', { name: 'Reply', exact: true }).click();
    await t2.getByLabel('Post reply', { exact: true }).fill('A reply from another device.');
    await t2.getByRole('button', { name: 'Post reply', exact: true }).click();
    await expect(t1.getByTestId('comment-entry')).toHaveCount(2);
    const reply1 = t1
      .getByTestId('comment-entry')
      .filter({ hasText: 'A reply from another device.' });
    const reply2 = t2
      .getByTestId('comment-entry')
      .filter({ hasText: 'A reply from another device.' });
    await expect(reply1).toContainText('discussion-replier');
    await expect(reply1.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
    const replyId = await reply2.getAttribute('data-message-id');
    await edit(t2.locator(`[data-message-id="${replyId}"]`), 'An edited reply.');
    await expect(t1).toContainText('An edited reply.');
    await expect(t1.locator(`[data-message-id="${replyId}"] .comment-byline`)).toHaveText(
      'discussion-replier · Just now · edited',
    );
    await t1.getByRole('button', { name: 'Resolve', exact: true }).click();
    await expect(t2).toContainText('Resolved thread');
    await t1.getByRole('button', { name: 'Reopen', exact: true }).click();
    await expect(t2).toContainText('Open thread');
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(0);

    const comment = t1.locator(`[data-message-id="${messageId}"]`);
    await comment.getByTestId('comment-ask-action').click();
    await comment
      .locator(`[data-testid=ask-agent-option][data-agent-id="${agent.id}"] input`)
      .check();
    await expect(comment.getByLabel('Question or instruction')).toHaveValue(
      '<script>plain discussion</script>\nPlease explain this.',
    );
    await comment.getByRole('button', { name: 'Ask agent — preview', exact: true }).click();
    const preview = comment.getByTestId('ask-preview');
    await expect(preview).toBeVisible();
    const operationId = (await preview.getAttribute('data-operation-id'))!;
    const exactMessage = await comment.getByTestId('ask-preview-text').textContent();
    await edit(comment, 'A later comment edit.');
    await expect(comment.getByTestId('ask-preview-text')).toHaveText(exactMessage!);
    await comment.getByTestId('ask-send').click();
    await until(() => agent.received().length === 1, 'comment Ask delivered');
    expect(agent.received()[0].message).toBe(exactMessage);
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(1);
    await second.getByTestId('ask-toggle').click();
    await expect(askEntry(second, operationId).getByTestId('ask-reply')).toBeVisible();
    expect(messageId).toMatch(/^[a-f0-9-]{36}$/);
    await comment.getByRole('button', { name: 'Close preview', exact: true }).click();

    // New source surrounding a unique quote keeps attachment; changing it detaches.
    await first.getByRole('button', { name: 'Source', exact: true }).click();
    await first
      .getByRole('textbox', { name: 'Source', exact: true })
      .fill('<p>Inserted above.</p>' + html);
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    await first
      .getByRole('textbox', { name: 'Source', exact: true })
      .fill(html.replace('exact quote', 'changed quote'));
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(t1).toHaveAttribute('data-anchor', 'detached');
    await expect(t1.locator('blockquote').first()).toHaveText('An & 🌍 exact quote for review.');
    await selectInRenderer(first, '#next');
    await first.getByTestId('comments-toggle').click();
    await t1.getByRole('button', { name: 'Reattach to selection', exact: true }).click();
    await t1.getByRole('button', { name: 'Confirm reattach', exact: true }).click();
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    await expect(t2.locator('blockquote').first()).toHaveText('Another selection.');
    await first.screenshot({ path: '/tmp/1427-threads-light.png', fullPage: true });
    await first.getByRole('button', { name: 'Change color theme' }).click();
    await first.screenshot({ path: '/tmp/1427-threads-dark.png', fullPage: true });
    await first.setViewportSize({ width: 390, height: 844 });
    expect(await first.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
    await first.screenshot({ path: '/tmp/1427-threads-mobile.png', fullPage: true });

    await second.getByTestId('comments-toggle').click();
    await t2
      .getByTestId('comment-entry')
      .filter({ hasText: 'An edited reply.' })
      .getByRole('button', { name: 'Delete comment', exact: true })
      .click();
    await expect(t1).toContainText('Comment deleted');
    await t1.getByRole('button', { name: 'Delete thread', exact: true }).click();
    await expect(t2).toContainText('Deleted thread');
    await expect(t2.getByRole('button', { name: 'Reply', exact: true })).toHaveCount(0);
    await post(first, 'Page-wide discussion.');
    await first.reload();
    await expect(
      first
        .getByTestId('comment-thread')
        .filter({ has: first.locator(`[data-message-id="${messageId}"]`) }),
    ).toHaveAttribute('data-thread-id', threadId!);
    await expect(first.getByTestId('comments-panel')).toContainText('Comment deleted');
    await restartColab(world, door);
    await second.reload();
    await expect(second.getByTestId('comments-panel')).toContainText('A later comment edit.');
    await expect(second.getByTestId('comments-panel')).toContainText('Page-wide discussion.');
    await expect(second.getByTestId('comments-panel')).toContainText('Comment deleted');
    await expect(second.getByTestId('comments-panel')).toContainText('Deleted thread');
    await second.getByTestId('ask-toggle').click();
    await expect(askEntry(second, operationId).getByTestId('ask-reply')).toBeVisible();
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((c) => c.operation === 'dispatch.create')).toHaveLength(1);
  });
});
