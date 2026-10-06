import { expect, test, type Locator, type Page } from '@playwright/test';
import { text } from '../src/strings.js';
import { mkdirSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
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
test('a same-request annotation reply submitted while observation is paused is recovered without another send', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('late-annotation-agent', { gated: true });
    const browser = await pairBrowser(world, 'late-annotation-author');
    const created = createPage(
      world,
      'Delayed annotation',
      '<p id="quote">Quoted passage.</p>',
      agent.pane,
    );
    const page = await openPage(door, browser, created);
    await selectInRenderer(page, '#quote');
    await page.getByTestId('selection-ask').click();
    const input = await inputFor(
      page.getByRole('dialog', { name: 'Annotate selection' }),
      agent.name,
    );
    await expect(input).toHaveText('', { useInnerText: true });
    await input.fill('Explain this passage.');
    await page.getByRole('button', { name: 'Ask agent', exact: true }).click();
    await until(() => agent.received().length === 1, 'annotation delivery');
    await expect(page.getByTestId('ask-state')).toHaveAttribute('data-state', 'accepted');
    const requestId = agent.received()[0].requestId as string;
    // Drive the observer's visibility input, leaving Remote and its signed clock untouched.
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    writeFileSync(path.join(agent.gate, `${requestId}.release`), '');
    await until(
      () => agent.rows().some((row) => row.event === 'replied' && row.requestId === requestId),
      'same-request durable reply',
    );
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    const expected = agent.rows().find((row) => row.event === 'replied')!.body as string;
    await expect(page.getByTestId('ask-reply')).toHaveText(expected);
    await expect(page.getByTestId('ask-reply-attribution')).toContainText(agent.name);
    await expect(page.getByTestId('ask-reply-attribution')).not.toContainText(
      'late-annotation-author',
    );
    await expect(page.getByTestId('ask-state')).toHaveCount(0);
    await page.reload();
    await comments(page);
    await page.getByTestId('annotation-row').filter({ hasText: 'Quoted passage.' }).click();
    await expect(page.getByTestId('ask-reply')).toHaveText(expected);
    await expect(page.getByTestId('ask-reply-attribution')).toContainText(agent.name);
    await expect(page.getByTestId('ask-reply-attribution')).not.toContainText(
      'late-annotation-author',
    );
    await expect(page.getByTestId('ask-state')).toHaveCount(0);
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
    const directory = process.env.COLAB_1699_CAPTURE_DIR;
    if (directory) {
      mkdirSync(directory, { recursive: true });
      for (const theme of ['light', 'dark']) {
        await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: 900 });
          await page.screenshot({ path: `${directory}/native-reply-${width}-${theme}.png` });
        }
      }
    }
  });
});
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
        const prefilled = popover.getByRole('combobox', { name: 'Message' });
        await expect(prefilled).toHaveText('', { useInnerText: true });
        await expect(prefilled).toBeFocused();
        expect(
          await prefilled.evaluate((node) => {
            const selection = node.ownerDocument.getSelection();
            return (
              !!selection?.isCollapsed &&
              !!selection.anchorNode &&
              node.contains(selection.anchorNode) &&
              selection.anchorOffset === 0
            );
          }),
        ).toBe(true);
        await expect(first.getByTestId('comments-toggle')).toHaveAttribute(
          'aria-expanded',
          'false',
        );
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-popover.png` });
        await prefilled.fill('@');
        await expect(first.getByRole('listbox')).toBeVisible();
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-autocomplete.png` });
        await prefilled.press('Escape');
        await prefilled.fill('');
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
    const input = await inputFor(compose, agent.name);
    await expect(input).toHaveText('', { useInnerText: true });
    const over = `@${agent.name} ${'é'.repeat(8193)}`;
    await input.fill(over);
    await input.press('Enter');
    await expect(compose.getByRole('alert')).toContainText('could not be recorded');
    await expect(input).toHaveText(over, { useInnerText: true });
    expect(agent.received()).toHaveLength(0);
    const opening = `@${agent.name} <script>plain discussion</script>\nPlease explain this.`;
    await input.fill(opening);
    await expect(compose.locator('details')).toHaveCount(0);
    await compose.getByRole('button', { name: 'Ask agent', exact: true }).click();
    await until(() => agent.received().length === 1, 'opening annotation delivered');
    expect(agent.received()[0].message).toContain('[remote: discussion-author]\n');
    expect(agent.received()[0].message).toContain(opening);
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
      'discussion-author · just now',
    );
    await expect(t2.getByRole('button', { name: text.threadResolve, exact: true })).toHaveCount(0);
    await expect(t1.getByTestId('ask-reply')).toBeVisible();
    await expect(t2.getByTestId('ask-reply')).toBeVisible();
    await expect(t1.getByTestId('ask-reply-attribution')).toContainText(
      `${agent.name} · ${text.conversationAgent}`,
    );
    await expect(t1.getByTestId('ask-reply-attribution')).not.toContainText('discussion-replier');
    await expect(t1.getByTestId('ask-state')).toHaveCount(0);
    await expect(t2.getByTestId('ask-state')).toHaveCount(0);
    await expect(t1.locator('.conversation-turn[data-turn-role=agent]')).toHaveCount(1);
    const firstReply = await t1.getByTestId('ask-reply').textContent();
    const follow = await inputFor(t2, agent.name);
    await follow.fill(`@${agent.name} A follow-up from another device.`);
    await follow.press('Shift+Enter');
    await expect(follow).toHaveText(`@${agent.name} A follow-up from another device.\n`, {
      useInnerText: true,
    });
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
      'discussion-replier · just now · edited',
    );
    await t1.getByRole('button', { name: text.threadResolve, exact: true }).click();
    await expect(row2).toContainText(text.threadResolved);
    await t1.getByRole('button', { name: text.threadReopen, exact: true }).click();
    await expect(row2).toContainText(text.threadOpen);
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
        await t1.getByRole('combobox', { name: 'Message', exact: true }).press('Escape');
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
        await t1.getByRole('combobox', { name: 'Message', exact: true }).scrollIntoViewIfNeeded();
        await first.screenshot({ path: `/tmp/1587-native-${width}-${theme}-input.png` });
      }
      await first.evaluate(() => {
        document.documentElement.dataset.theme = 'light';
      });
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.getByRole('button', { name: 'Source', exact: true }).click();
    const inserted = '<p style="height:300px">Inserted above.</p>' + html;
    const source = first.getByRole('textbox', { name: 'Source', exact: true });
    await source.fill(inserted);
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    // Saved means the draft equals the new base again; a second save before that uses a stale base.
    await expect(first.getByRole('button', { name: 'Save source', exact: true })).toBeDisabled();
    await expect(
      second.frameLocator('iframe').getByText('Inserted above.', { exact: true }),
    ).toBeVisible();
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    await source.fill(html.replace('exact quote', 'changed quote'));
    await first.getByRole('button', { name: 'Save source', exact: true }).click();
    await expect(second.frameLocator('iframe').locator('#quote')).toContainText('changed quote');
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

test('composer records plain annotations and replies without a recipient, then sends one explicitly selected no-prefix Ask', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('composer-agent', { gated: true });
    const browser = await pairBrowser(world, 'composer-author');
    const created = createPage(
      world,
      'Composer acceptance',
      '<p id="quote">Frozen original quote.</p>',
      agent.pane,
    );
    const page = await openPage(door, browser, created);
    await selectInRenderer(page, '#quote');
    await page.getByTestId('selection-ask').click();
    const compose = page.getByRole('dialog', { name: 'Annotate selection' });
    const input = compose.getByRole('combobox', { name: 'Message', exact: true });
    const plain = 'Plain annotation without a recipient.\nSecond line.';
    await input.fill(plain);
    await compose.getByRole('button', { name: 'Post comment', exact: true }).click();
    await comments(page);
    await page.getByTestId('annotation-row').filter({ hasText: 'Frozen original quote.' }).click();
    const thread = page.getByTestId('comment-thread').first();
    const original = thread
      .getByTestId('comment-entry')
      .filter({ hasText: 'Plain annotation without a recipient.' });
    const threadId = (await thread.getAttribute('data-thread-id'))!;
    const originalId = (await original.getAttribute('data-message-id'))!;
    await expect.poll(() => original.locator('.comment-body').textContent()).toBe(plain);
    await expect
      .poll(() => thread.locator('blockquote').first().textContent())
      .toBe('Frozen original quote.');
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    const edited = 'Edited plain annotation.\n  Exact spacing stays.  ';
    await edit(original, edited);
    const originalById = thread.locator(`[data-message-id="${originalId}"]`);
    await expect.poll(() => originalById.locator('.comment-body').textContent()).toBe(edited);
    const reply = thread.getByRole('combobox', { name: 'Message', exact: true });
    const plainReply = 'Plain reply without a recipient.\n  Retained bytes.  ';
    await reply.fill(plainReply);
    await thread.getByRole('button', { name: 'Post reply', exact: true }).click();
    await expect(thread.getByTestId('comment-entry')).toHaveCount(2);
    const replyEntry = thread
      .getByTestId('comment-entry')
      .filter({ hasText: 'Plain reply without a recipient.' });
    const replyId = (await replyEntry.getAttribute('data-message-id'))!;
    await expect.poll(() => replyEntry.locator('.comment-body').textContent()).toBe(plainReply);
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    const question =
      'Explain this exact quote, with no mandatory prefix.\n  Keep these spaces and this line.  ';
    await reply.fill(question);
    await inputFor(thread, agent.name);
    await expect.poll(() => reply.innerText()).toBe(question);
    // Choose, then Change the same admitted recipient; both preserve multiline bytes.
    await inputFor(thread, agent.name);
    await expect.poll(() => reply.innerText()).toBe(question);
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    const directory = process.env.COLAB_1817_CAPTURE_DIR;
    if (directory) {
      mkdirSync(directory, { recursive: true });
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
          await reply.scrollIntoViewIfNeeded();
          await page.screenshot({
            path: path.join(directory, `native-composer-${width}-${theme}.png`),
          });
        }
      }
    }
    world.armNextBarrier('after');
    await thread.getByRole('button', { name: 'Ask agent', exact: true }).click();
    const parked = await world.barrierEntered();
    const entry = thread.getByTestId('ask-entry');
    await expect(entry).toHaveCount(1);
    const operationId = (await entry.getAttribute('data-operation-id'))!;
    expect(parked.operationId).toBe(operationId);
    await until(() => agent.received().length === 1, 'one explicitly selected annotation Ask');
    const received = agent.received()[0];
    const requestId = received.requestId as string;
    expect(requestId).toMatch(/^req_[0-9a-f-]+$/);
    expect(received.identityId).toBe(agent.id);
    expect(typeof received.message).toBe('string');
    const delivered = received.message as string;
    expect(delivered.slice(-question.length)).toBe(question);
    const askComment = thread
      .getByTestId('comment-entry')
      .filter({ hasText: 'Explain this exact quote, with no mandatory prefix.' });
    const askMessageId = (await askComment.getAttribute('data-message-id'))!;
    await expect.poll(() => askComment.locator('.comment-body').textContent()).toBe(question);
    expect(agent.received()[0].message).toContain('Frozen original quote.');
    expect(agent.received()[0].message).not.toContain(`@${agent.name}`);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
    expect(
      world
        .coreCalls()
        .filter((call) => call.operation === 'dispatch.create')
        .map((call) => call.operationId),
    ).toEqual([operationId]);
    expect(agent.received(requestId)).toHaveLength(1);
    world.releaseBarrier();
    await expect(entry).toHaveAttribute('data-ledger-state', 'accepted');
    writeFileSync(path.join(agent.gate, `${requestId}.release`), '');
    const expectedReply = `ask-reply:${createHash('sha256').update(delivered).digest('hex').slice(0, 16)}`;
    await expect.poll(() => entry.getByTestId('ask-reply').textContent()).toBe(expectedReply);
    await until(
      () => agent.rows().some((row) => row.event === 'replied' && row.requestId === requestId),
      'reply for the original request',
    );
    await page.reload();
    await comments(page);
    await page.locator(`[data-testid=annotation-row][data-thread-id="${threadId}"]`).click();
    const restored = page.locator(`[data-testid=comment-thread][data-thread-id="${threadId}"]`);
    await expect
      .poll(() => restored.locator(`[data-message-id="${originalId}"] .comment-body`).textContent())
      .toBe(edited);
    await expect
      .poll(() => restored.locator(`[data-message-id="${replyId}"] .comment-body`).textContent())
      .toBe(plainReply);
    await expect
      .poll(() =>
        restored.locator(`[data-message-id="${askMessageId}"] .comment-body`).textContent(),
      )
      .toBe(question);
    await expect
      .poll(() => restored.locator('blockquote').first().textContent())
      .toBe('Frozen original quote.');
    const restoredAsk = restored.locator(
      `[data-testid=ask-entry][data-operation-id="${operationId}"]`,
    );
    await expect.poll(() => restoredAsk.getByTestId('ask-reply').textContent()).toBe(expectedReply);
    expect(agent.received(requestId)).toHaveLength(1);
    expect(
      world
        .coreCalls()
        .filter((call) => call.operation === 'dispatch.create')
        .map((call) => call.operationId),
    ).toEqual([operationId]);
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
  });
});
