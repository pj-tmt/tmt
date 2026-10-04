import { expect, test, type Locator, type Page } from '@playwright/test';
import { pairBrowser, restartColab, startDoor } from './harness/browser.js';
import {
  annotationInput as inputFor,
  createPage,
  freePort,
  openPage,
  selectInRenderer,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

async function edit(found: Locator, body: string) {
  // The text filter stops matching once the draft replaces the body, so pin the row by its message id.
  const id = await found.getAttribute('data-message-id');
  const comment = found.page().locator(`[data-testid=comment-entry][data-message-id="${id}"]`);
  await comment.hover();
  await comment.getByRole('button', { name: 'Message actions', exact: true }).click();
  await comment.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await comment.getByLabel('Edit comment', { exact: true }).fill(body);
  await comment.getByRole('button', { name: 'Save comment', exact: true }).click();
  await expect(comment.locator('.comment-body')).toHaveText(body);
}
async function comments(page: Page) {
  if ((await page.getByTestId('comments-toggle').getAttribute('aria-expanded')) === 'false')
    await page.getByTestId('comments-toggle').click();
}

test.afterEach(disposeActiveWorlds);
test('paired writers retain anchored annotation conversations, direct exact sends and one window scroll through restart', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('discussion-agent');
    const a = await pairBrowser(world, 'discussion-author');
    const b = await pairBrowser(world, 'discussion-replier');
    const paragraphs = Array.from(
      { length: 40 },
      (_, index) =>
        `<section><h2>Paragraph ${index + 1}</h2><p>This numbered paragraph is visible page content. The browser window moves it beneath the fixed Colab header.</p></section>`,
    ).join('');
    const html =
      '<script>const every=Array.prototype.every;window.anchorTraffic=[];Array.prototype.every=function(callback,...args){if(this[0]?.selector&&this[0]?.id)window.anchorTraffic.push(JSON.stringify(this));return every.call(this,callback,...args)};</script><style>body{margin:0;padding:24px 48px 24px 24px;font:16px/1.6 sans-serif}p{max-width:70ch}h2{margin-top:32px}</style><h1>Shared review</h1><p id="quote">An &amp; <em>🌍 exact quote</em> for review.</p><p id="next">Another selection.</p>' +
      paragraphs +
      '<h2 id="scroll-end">END OF PAGE</h2>';
    const created = createPage(world, 'Shared review', html, agent.pane);
    const first = await openPage(door, a, created);
    const second = await openPage(door, b, created);
    await expect
      .poll(async () => (await first.locator('iframe').boundingBox())?.height ?? 0)
      .toBeGreaterThan(2400);
    await expect(first.locator('iframe')).toHaveAttribute('scrolling', 'no');
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      await expect
        .poll(() =>
          first
            .frameLocator('iframe')
            .locator('html')
            .evaluate(
              (node) => node.scrollHeight <= node.ownerDocument.defaultView!.innerHeight + 1,
            ),
        )
        .toBe(true);
      await first.evaluate(() => window.scrollTo(0, 0));
      await first.screenshot({ path: `/tmp/1587-native-${width}-light-long-top.png` });
      await first.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
      const windowScrollTop = await first.evaluate(() => document.scrollingElement!.scrollTop);
      const frameScrollTop = await first
        .frameLocator('iframe')
        .locator('html')
        .evaluate((node) => node.ownerDocument.scrollingElement!.scrollTop);
      expect(windowScrollTop).toBeGreaterThan(1000);
      expect(frameScrollTop).toBe(0);
      await expect(first.frameLocator('iframe').locator('#scroll-end')).toBeInViewport();
      console.log(
        JSON.stringify({ width, windowScrollTop, frameScrollTop, marker: 'END OF PAGE' }),
      );
      expect((await first.locator('.colab-header').boundingBox())?.y).toBe(0);
      await first.screenshot({ path: `/tmp/1587-native-${width}-light-long-scrolled.png` });
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => window.scrollTo(0, 0));
    await selectInRenderer(first, '#quote');
    await expect(first.getByTestId('selection-ask')).toBeVisible();
    await first.screenshot({ path: '/tmp/1587-native-1440-light-selection.png' });
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await selectInRenderer(first, '#quote');
        await first.getByTestId('selection-ask').click();
        const popover = first.getByRole('dialog', { name: 'Annotate selection' });
        const prefilled = popover.getByRole('combobox', { name: 'Message to agent' });
        await expect(prefilled).toHaveValue(`@${agent.name} `);
        await expect(prefilled).toBeFocused();
        expect(await prefilled.evaluate((node: HTMLTextAreaElement) => node.selectionStart)).toBe(
          agent.name.length + 2,
        );
        await expect(first.getByTestId('comments-toggle')).toHaveAttribute(
          'aria-expanded',
          'false',
        );
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-popover.png` });
        await prefilled.fill('@');
        await expect(first.getByRole('listbox')).toBeVisible();
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-autocomplete.png` });
        await prefilled.press('Escape');
        await prefilled.press('Escape');
        await expect(popover).toHaveCount(0);
        await expect(first.getByTestId('selection-ask')).toBeVisible();
        expect(agent.received()).toHaveLength(0);
      }
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => (document.documentElement.dataset.theme = 'light'));
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const compose = first.locator('.annotation-new');
    const input = compose.getByRole('combobox', { name: 'Message to agent' });
    await expect(input).toHaveValue(`@${agent.name} `);
    const over = `@${agent.name} ${'é'.repeat(8193)}`;
    await input.fill(over);
    await input.press('Enter');
    await expect(compose.getByRole('alert')).toContainText('could not be recorded');
    await expect(input).toHaveValue(over);
    expect(agent.received()).toHaveLength(0);
    const opening = `@${agent.name} <script>plain discussion</script>\nPlease explain this.`;
    await input.fill(opening);
    await expect(compose.locator('details')).toHaveCount(0);
    await input.press('Enter');
    await until(() => agent.received().length === 1, 'opening annotation delivered');
    expect(agent.received()[0].message).toContain('[remote: discussion-author]\n');
    expect(agent.received()[0].message).toContain(opening.slice(agent.name.length + 2));
    const t1 = first.getByTestId('comment-thread').first();
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    const threadId = (await t1.getAttribute('data-thread-id'))!;
    const messageId = await t1.getByTestId('comment-entry').first().getAttribute('data-message-id');
    await comments(second);
    const row2 = second
      .getByTestId('annotation-row')
      .filter({ hasText: 'An & 🌍 exact quote for review.' });
    await row2.click();
    const t2 = second.getByTestId('comment-thread').first();
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    await expect(t2.getByTestId('comment-entry').first().locator('.comment-byline')).toHaveText(
      'discussion-author · Just now',
    );
    await expect(t2.getByRole('button', { name: 'Resolve', exact: true })).toHaveCount(0);
    await expect(t1.getByTestId('ask-reply')).toBeVisible();
    await expect(t2.getByTestId('ask-reply')).toBeVisible();
    await expect(t1.getByTestId('ask-reply-attribution')).toContainText(agent.name);
    const firstReply = await t1.getByTestId('ask-reply').textContent();
    const follow = await inputFor(t2, agent.name);
    await follow.fill(`@${agent.name} A follow-up from another device.`);
    await follow.press('Shift+Enter');
    await expect(follow).toHaveValue(`@${agent.name} A follow-up from another device.\n`);
    await follow.type('One more line.');
    await follow.press('Enter');
    await until(() => agent.received().length === 2, 'follow-up annotation delivered');
    expect(agent.received()[1].message).toContain(opening);
    expect(agent.received()[1].message).toContain(firstReply!);
    expect(agent.received()[1].message).toContain('Earlier conversation (quoted data):');
    await expect(t1.getByTestId('comment-entry')).toHaveCount(2);
    const replyId = await t2
      .getByTestId('comment-entry')
      .filter({ hasText: 'A follow-up from another device.' })
      .getAttribute('data-message-id');
    // Pinned by id: the text filter would stop matching once the body is edited.
    const reply1 = t1.locator(`[data-testid=comment-entry][data-message-id="${replyId}"]`);
    const reply2 = t2.locator(`[data-testid=comment-entry][data-message-id="${replyId}"]`);
    await expect(reply1.getByRole('button', { name: 'Message actions', exact: true })).toHaveCount(
      0,
    );
    await edit(reply2, 'An edited reply.');
    await expect(reply1.locator('.comment-byline')).toHaveText(
      'discussion-replier · Just now · edited',
    );
    await t1.getByRole('button', { name: 'Resolve', exact: true }).click();
    await expect(row2).toContainText('Resolved thread');
    await t1.getByRole('button', { name: 'Reopen', exact: true }).click();
    await expect(row2).toContainText('Open thread');
    const marker = first.frameLocator('iframe').locator('[data-colab-thread]');
    await expect(marker).toHaveCount(1);
    const traffic = await first
      .frameLocator('iframe')
      .locator('html')
      .evaluate(
        (node) =>
          (node.ownerDocument.defaultView as unknown as { anchorTraffic: string[] }).anchorTraffic,
      );
    expect(traffic.length).toBeGreaterThan(0);
    for (const batch of traffic) {
      expect(batch).not.toContain(opening.split('\n')[0].slice(0, 32));
      for (const anchor of JSON.parse(batch))
        expect(Object.keys(anchor).sort()).toEqual(['id', 'selector']);
    }
    await expect(first.getByTestId('annotation-row').first()).toHaveAttribute(
      'title',
      opening.split('\n')[0].slice(0, 32),
    );
    await expect(marker).toHaveAttribute('title', 'An & 🌍 exact quote for review.');
    await first.getByRole('button', { name: 'Close Comments', exact: true }).click();
    await marker.click();
    await expect(t1).toBeVisible();
    const frameWidth = (await first.locator('iframe').boundingBox())!.width;
    expect(frameWidth).toBe(1440);
    await first
      .frameLocator('iframe')
      .locator('body')
      .evaluate((node) => node.ownerDocument.getSelection()?.removeAllRanges());
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      await first.evaluate(() => window.scrollTo(0, 0));
      await expect(t1).toHaveAttribute('data-anchor', 'attached');
      await first.getByRole('button', { name: 'Close Comments', exact: true }).click();
      await first.screenshot({ path: `/tmp/1587-native-${width}-light-markers.png` });
      await marker.click();
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        await t1.getByRole('combobox', { name: 'Message to agent', exact: true }).press('Escape');
        await first.locator('.page-drawer[open] .drawer-body').evaluate((node) => {
          node.scrollTop = 0;
        });
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-threads.png` });
        await first.locator(`[data-testid="annotation-row"][data-thread-id="${threadId}"]`).click();
        await expect(t1).toHaveAttribute('data-anchor', 'attached');
        await first.locator('.page-drawer[open] .drawer-body').evaluate((node) => {
          node.scrollTop = 0;
        });
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-thread.png` });
        // Comment order within a thread is not fixed; take the one this browser wrote.
        const own = t1.getByTestId('comment-entry').filter({ hasText: 'You ·' }).first();
        const menu = own.getByRole('button', { name: 'Message actions', exact: true });
        await own.hover();
        await menu.click();
        await expect(own.getByRole('menuitem')).toHaveText(['Edit', 'Delete']);
        await first.screenshot({ path: `/tmp/1690-native-${width}-${theme}-comment-menu.png` });
        await menu.press('Escape');
        await expect(own.getByRole('menuitem')).toHaveCount(0);
        await t1
          .getByRole('combobox', { name: 'Message to agent', exact: true })
          .scrollIntoViewIfNeeded();
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-input.png` });
      }
      await first.evaluate(() => {
        document.documentElement.dataset.theme = 'light';
      });
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.getByRole('button', { name: 'Source', exact: true }).click();
    await first
      .getByRole('textbox', { name: 'Source', exact: true })
      .fill('<p style="height:300px">Inserted above.</p>' + html);
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    await first
      .getByRole('textbox', { name: 'Source', exact: true })
      .fill(html.replace('exact quote', 'changed quote'));
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(t1).toHaveAttribute('data-anchor', 'detached');
    await expect(marker).toHaveCount(0);
    await first.getByRole('button', { name: 'Close Source', exact: true }).click();
    await selectInRenderer(first, '#next');
    await comments(first);
    await t1.getByRole('button', { name: 'Reattach to selection', exact: true }).click();
    await t1.getByRole('button', { name: 'Confirm reattach', exact: true }).click();
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    await expect(t2.locator('blockquote').first()).toHaveText('Another selection.');
    await reply2.hover();
    await reply2.getByRole('button', { name: 'Message actions', exact: true }).click();
    await reply2.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    await expect(t1).toContainText('Comment deleted');
    await t1.getByRole('button', { name: 'Delete thread', exact: true }).click();
    await expect(t2).toContainText('Deleted thread');
    await expect(t2.getByRole('combobox')).toHaveCount(0);
    await first.getByRole('button', { name: '+ Comment on page', exact: true }).click();
    await first.getByLabel('Post comment', { exact: true }).fill('Page-wide discussion.');
    await first.getByRole('button', { name: 'Post comment', exact: true }).click();
    await expect(first.getByTestId('annotation-row')).toHaveCount(2);
    await first.reload();
    await comments(first);
    await first.getByTestId('annotation-row').filter({ hasText: 'Deleted thread' }).click();
    await expect(first.getByTestId('comment-thread')).toHaveAttribute('data-thread-id', threadId);
    await expect(first.getByTestId('comment-thread')).toContainText('Comment deleted');
    await restartColab(world, door);
    await second.reload();
    await comments(second);
    await expect(second.getByTestId('annotation-row')).toHaveCount(2);
    await second.getByTestId('annotation-row').filter({ hasText: 'Deleted thread' }).click();
    await expect(second.getByTestId('comment-thread')).toContainText('Comment deleted');
    await expect(
      second.getByTestId('comment-thread').getByTestId('ask-reply').first(),
    ).toBeVisible();
    expect(messageId).toMatch(/^[a-f0-9-]{36}$/);
    expect(agent.received()).toHaveLength(2);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      2,
    );
  });
});
