import { commentsClose, commentsRegion, openPanel } from '../test/comments-region.js';
import {
  pageAction,
  selectThread,
  selectDiscussion,
  chooseThreadFilter,
} from '../test/page-actions.js';
import { expect, test, type Locator, type Page, type Route } from '@playwright/test';
import { text } from '../src/strings.js';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { pairBrowser, restartColab, startDoor } from './harness/browser.js';
import {
  patchArgs,
  annotationInput as inputFor,
  composerText,
  composerTrace,
  composerAssets,
  createPage,
  freePort,
  openPage,
  run,
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
  // An open anchored window replaces the dock; collapse it to reach the Comments action.
  const anchored = page.getByRole('dialog', { name: 'Annotate selection' });
  if (await anchored.isVisible())
    await anchored.getByRole('button', { name: 'Collapse thread', exact: true }).click();
  // The dock's Comments action precedes verified threads after reload. Its count is absent until
  // that projection is ready; do not toggle before it and lose the action.
  await expect(
    page.locator('.page-floating-actions').getByRole('button', { name: /^Comments \(\d+\)/ }),
  ).toBeVisible();
  const panel = commentsRegion(page);
  if (!(await panel.isVisible())) await (await pageAction(page, 'Comments')).click();
  await expect(panel).toBeVisible();
}

test.afterEach(disposeActiveWorlds);
test('a same-request annotation reply recovers from a visible transient read failure without another send', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('late-annotation-agent', { gated: true });
    const browser = await pairBrowser(world, 'late-annotation-author', { talk: true });
    const created = createPage(
      world,
      'Delayed annotation',
      '<p id="quote">Quoted passage.</p>',
      agent.pane,
    );
    const page = await openPage(door, browser, created);
    const mode = page.getByRole('button', { name: 'Selection mode', exact: true });
    await mode.click();
    await expect(mode).toHaveAttribute('aria-pressed', 'true');
    await page.frameLocator('iframe').locator('#quote').click();
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    const input = await inputFor(
      page.getByRole('dialog', { name: 'Annotate selection' }),
      agent.name,
    );
    await expect(input).toHaveText(`@${agent.name} `, { useInnerText: true });
    await input.fill(`@${agent.name} Explain this passage.`);
    // Hiding pauses future cycles but cannot cancel an in-flight result read.
    // Install response loss before Send, so no result can bypass this route.
    let failedReads = 0;
    const failedResult = async (route: Route) => {
      const request = route.request();
      if (request.method() !== 'POST' || request.postDataJSON().operation !== 'result')
        return route.continue();
      // Let the real signed read reach Remote, then lose its response. This keeps
      // admitted session sequences intact and never fabricates a result or send.
      await route.fetch();
      failedReads++;
      await route.abort('failed');
    };
    await page.route('**/append', failedResult);
    await page.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
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
    const window = page.getByRole('dialog', { name: 'Annotate selection' });
    await expect(
      window.getByText(text.askObservationUnavailable, { exact: true }),
    ).toBeInViewport();
    expect(failedReads).toBeGreaterThan(0);
    await expect(openPanel(page)).toHaveCount(0);
    const failureCaptures = process.env.COLAB_DISCUSSION_CAPTURE_DIR;
    if (failureCaptures) {
      mkdirSync(failureCaptures, { recursive: true });
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
          await expect(
            window.getByText(text.askObservationUnavailable, { exact: true }),
          ).toBeInViewport();
          await page.screenshot({
            path: `${failureCaptures}/1761-native-${width}-${theme}-read-failure.png`,
          });
        }
      }
    }
    // Finish admitted reads before removing interception and allowing recovery.
    await page.unrouteAll({ behavior: 'wait' });
    const expected = agent.rows().find((row) => row.event === 'replied')!.body as string;
    await expect(page.getByTestId('ask-reply')).toHaveText(expected);
    await expect(window.getByText(text.askObservationUnavailable, { exact: true })).toHaveCount(0);
    await expect(page.getByTestId('ask-reply-attribution')).toContainText(agent.name);
    await expect(page.getByTestId('ask-reply-attribution')).not.toContainText(
      'late-annotation-author',
    );
    await expect(page.getByTestId('ask-state')).toHaveCount(0);
    await page.reload();
    await comments(page);
    await selectDiscussion(page, 'Quoted passage.');
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
  const captureDir = process.env.COLAB_DISCUSSION_CAPTURE_DIR ?? test.info().outputPath('captures');
  mkdirSync(captureDir, { recursive: true });
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('discussion-agent');
    const a = await pairBrowser(world, 'discussion-author', { talk: true });
    const b = await pairBrowser(world, 'discussion-replier', { talk: true });
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
      await first.screenshot({ path: `${captureDir}/1587-native-${width}-light-long-top.png` });
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
      expect(
        (await first.locator('.tmt-ui-shell-bar > .tmt-ui-topbar:visible').boundingBox())?.y,
      ).toBe(0);
      await first.screenshot({
        path: `${captureDir}/1587-native-${width}-light-long-scrolled.png`,
      });
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => window.scrollTo(0, 0));
    await selectInRenderer(first, '#quote');
    await expect(first.getByTestId('selection-ask')).toBeVisible();
    await first.screenshot({ path: `${captureDir}/1587-native-1440-light-selection.png` });
    // The first open seeds the page creator; each pass removes that mention, and the composer keeps
    // that edited draft for this selection, so later passes open empty.
    let seeded = true;
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await selectInRenderer(first, '#quote');
        await first.getByTestId('selection-ask').click();
        const popover = first.getByRole('dialog', { name: 'Annotate selection' });
        const prefilled = popover.getByRole('combobox', { name: 'Message' });
        await expect(prefilled).toHaveText(seeded ? `@${agent.name} ` : '', { useInnerText: true });
        seeded = false;
        await expect(prefilled).toBeFocused();
        expect(
          await prefilled.evaluate((node) => {
            const selection = node.ownerDocument.getSelection();
            if (
              !selection?.isCollapsed ||
              !selection.anchorNode ||
              !node.contains(selection.anchorNode)
            )
              return false;
            const after = node.ownerDocument.createRange();
            after.selectNodeContents(node);
            after.setStart(selection.anchorNode, selection.anchorOffset);
            return after.toString() === '';
          }),
        ).toBe(true);
        await expect(commentsRegion(first)).toHaveCount(0);
        await first.screenshot({ path: `${captureDir}/1587-native-${width}-${theme}-popover.png` });
        await prefilled.fill('@');
        await expect(first.getByRole('listbox')).toBeVisible();
        await first.screenshot({
          path: `${captureDir}/1587-native-${width}-${theme}-autocomplete.png`,
        });
        await prefilled.press('Escape');
        // Trusted editing keys let Lexical admit the selected range before deletion.
        await prefilled.press('ControlOrMeta+A');
        await prefilled.press('Backspace');
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
    // The reader removed the mention and closed this same selection earlier.
    await expect(input).toHaveText('', { useInnerText: true });
    const over = `@${agent.name} ${'é'.repeat(8193)}`;
    await input.fill(over);
    await input.press('Enter');
    // The failed row holds the exact draft until Edit restores it to the composer.
    const unsent = compose.locator('[data-testid=comment-echo][data-echo=failed]');
    await expect(unsent.getByRole('alert')).toContainText(text.messageUnsent);
    await expect(unsent.locator('.comment-body')).toHaveText(`${agent.name} ${'é'.repeat(8193)}`);
    await expect(unsent.getByRole('button', { name: text.chatRetry, exact: true })).toBeEnabled();
    await unsent.getByRole('button', { name: text.commentEdit, exact: true }).click();
    await expect(unsent).toHaveCount(0);
    await expect(input).toHaveText(over, { useInnerText: true });
    expect(agent.received()).toHaveLength(0);
    const opening = `@${agent.name} <script>plain discussion</script>\nPlease explain this.`;
    await input.fill(opening);
    await expect(compose.locator('details')).toHaveCount(0);
    const quote = await compose.locator('.thread-state').getAttribute('title');
    const offset = await first.evaluate(() => window.scrollY);
    const replaceSource = async (source: string) => {
      const read = JSON.parse(
        run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']),
      ) as { revision: string; source: string };
      // The shown source's digest changes whether the frame is replaced or updated in place (#5022).
      const digest = await first.locator('iframe').getAttribute('data-source-digest');
      run(
        world,
        world.binaries.colab,
        patchArgs(
          world,
          created.pageId,
          ['--file', '-', '--expected-revision', read.revision, '--json'],
          read.source,
        ),
        source,
        agent.pane,
      );
      await expect(first.locator('iframe')).not.toHaveAttribute('data-source-digest', digest!);
      await expect(first.locator('.tmt-ui-topbar .status')).toContainText('Live');
    };
    await replaceSource(
      html.replace('An &amp; <em>🌍 exact quote</em> for review.', 'The source was updated.'),
    );
    await expect(input).toHaveText(opening, { useInnerText: true });
    await expect(compose.locator('.thread-state')).toHaveAttribute('title', quote!);
    await expect(compose.getByText(text.commentQuoteChanged)).toBeVisible();
    expect(Math.abs((await first.evaluate(() => window.scrollY)) - offset)).toBeLessThan(2);
    await input.evaluate((node) => Object.assign(window, { retainedAnnotationInput: node }));
    await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    await until(() => agent.received().length === 1, 'opening annotation delivered');
    expect(agent.received()[0].message).toContain('[remote: discussion-author]\n');
    expect(agent.received()[0].message).toContain(opening);
    expect(agent.received()[0].message).toContain(quote!);
    const t1 = first.getByTestId('comment-thread').first();
    await expect(openPanel(first)).toHaveCount(0);
    expect(
      await input.evaluate(
        (node) =>
          (window as unknown as { retainedAnnotationInput: Element }).retainedAnnotationInput ===
          node,
      ),
    ).toBe(true);
    await expect(t1).toHaveAttribute('data-anchor', 'detached');
    await expect(t1.getByText(text.commentQuoteChanged)).toBeVisible();
    // The agent reply is a concurrent page write; observe its admitted publication before patching.
    await expect(t1.getByTestId('ask-reply')).toHaveCount(1);
    await replaceSource(html);
    await expect(t1).toHaveAttribute('data-anchor', 'attached');
    await expect(t1.getByText(text.commentQuoteChanged)).toHaveCount(0);
    const threadId = (await t1.getAttribute('data-thread-id'))!;
    const messageId = await t1.getByTestId('comment-entry').first().getAttribute('data-message-id');
    await comments(second);
    await selectDiscussion(second, 'An & 🌍 exact quote for review.');
    const t2 = second.getByTestId('comment-thread').first();
    await expect(t2).toHaveAttribute('data-anchor', 'attached');
    await expect(t2.getByTestId('comment-entry').first().locator('.comment-byline')).toHaveText(
      'discussion-author · just now',
    );
    // Status is not writer-owned: this is another owner device, so it may resolve the thread.
    await expect(
      commentsRegion(second).getByRole('button', { name: text.threadResolve, exact: true }),
    ).toBeEnabled();
    await expect(t1.getByTestId('ask-reply')).toBeVisible();
    await expect(t2.getByTestId('ask-reply')).toBeVisible();
    await expect(t1.getByTestId('ask-reply-attribution')).toContainText(agent.name);
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
    // The anchored history contains both own and foreign turns without opening a drawer.
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await first.screenshot({ path: `${captureDir}/1761-native-${width}-${theme}-window.png` });
      }
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => (document.documentElement.dataset.theme = 'light'));
    const beforeCommentsWidth = (await first.locator('iframe').boundingBox())!.width;
    await comments(first);
    await selectThread(first, threadId);
    await commentsRegion(first)
      .getByRole('button', { name: text.threadResolve, exact: true })
      .click();
    await chooseThreadFilter(second, 'All');
    await expect(second.getByTestId('discussion-state')).toContainText(text.threadResolved);
    // A person's resolve notifies the mentioned agent exactly once, through the Ask path.
    await until(() => agent.received().length === 3, 'resolve notification delivered');
    const notice = String(agent.received()[2].message);
    expect(notice).toContain('Thread resolved by discussion-author.');
    expect(notice).toContain(quote!);
    await commentsRegion(first)
      .getByRole('button', { name: text.threadReopen, exact: true })
      .click();
    await chooseThreadFilter(second, 'Open');
    await expect(t2).toBeVisible();
    await expect(second.getByTestId('discussion-state')).toHaveCount(0);
    await expect(t2).toBeVisible();
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
    let paints = 0;
    // This author probe observes both question lookup selectors and numbered highlight anchors.
    for (const batch of traffic) {
      expect(batch).not.toContain(opening.split('\n')[0].slice(0, 32));
      for (const anchor of JSON.parse(batch)) {
        const numbered = 'ordinal' in anchor;
        expect(Object.keys(anchor).sort()).toEqual(
          numbered ? ['id', 'ordinal', 'selector'] : ['id', 'selector'],
        );
        if (numbered) {
          paints++;
          expect(Number.isSafeInteger(anchor.ordinal) && anchor.ordinal > 0).toBe(true);
        }
      }
    }
    expect(paints).toBeGreaterThan(0);
    await expect(first.getByTestId('annotation-row')).toHaveCount(0);
    await expect(marker).toHaveAttribute(
      'title',
      `Thread ${await marker.textContent()}: An & 🌍 exact quote for review.`,
    );
    await commentsClose(first, 'discussion').click();
    await marker.click();
    await expect(t1).toBeVisible();
    // Wide Discussion floats without narrowing the authored page.
    await expect(commentsRegion(first)).toBeVisible();
    await expect(first.locator('.tmt-ui-drawer[open]')).toHaveCount(0);
    const frameWidth = (await first.locator('iframe').boundingBox())!.width;
    expect(frameWidth).toBe(beforeCommentsWidth);
    await first
      .frameLocator('iframe')
      .locator('body')
      .evaluate((node) => node.ownerDocument.getSelection()?.removeAllRanges());
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      await first.evaluate(() => window.scrollTo(0, 0));
      await expect(t1).toHaveAttribute('data-anchor', 'attached');
      await commentsClose(first, 'discussion').click();
      await first.screenshot({ path: `${captureDir}/1587-native-${width}-light-markers.png` });
      await marker.click();
      await expect(commentsRegion(first)).toBeVisible();
      await expect(first.locator('.tmt-ui-drawer[open]')).toHaveCount(0);
      await comments(first);
      await selectThread(first, threadId);
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => {
          document.documentElement.dataset.theme = theme;
        }, theme);
        // Blur the field for capture; Escape now correctly dismisses the whole Discussion.
        await commentsRegion(first)
          .getByRole('heading', { name: 'Discussion', exact: true })
          .click();
        await commentsRegion(first)
          .locator('.tmt-ui-drawer-body')
          .evaluate((node) => {
            node.scrollTop = 0;
          });
        await first.screenshot({ path: `${captureDir}/1587-native-${width}-${theme}-threads.png` });
        await selectThread(first, threadId);
        await expect(t1).toHaveAttribute('data-anchor', 'attached');
        await commentsRegion(first)
          .locator('.tmt-ui-drawer-body')
          .evaluate((node) => {
            node.scrollTop = 0;
          });
        await first.screenshot({ path: `${captureDir}/1587-native-${width}-${theme}-thread.png` });
        // Comment order within a thread is not fixed; take the one this browser wrote.
        const own = t1.getByTestId('comment-entry').filter({ hasText: 'You ·' }).first();
        const menu = own.getByRole('button', { name: 'Message actions', exact: true });
        await own.hover();
        await menu.click();
        await expect(own.getByRole('menuitem')).toHaveText(['Edit', 'Delete']);
        await first.screenshot({
          path: `${captureDir}/1690-native-${width}-${theme}-comment-menu.png`,
        });
        await menu.press('Escape');
        await expect(own.getByRole('menuitem')).toHaveCount(0);
        await t1.getByRole('combobox', { name: 'Message', exact: true }).scrollIntoViewIfNeeded();
        await first.screenshot({ path: `${captureDir}/1587-native-${width}-${theme}-input.png` });
      }
      await first.evaluate(() => {
        document.documentElement.dataset.theme = 'light';
      });
      await commentsClose(first, 'discussion').click();
      await marker.click();
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await comments(first);
    await selectThread(first, threadId);
    await (await pageAction(first, 'Source')).click();
    const inserted = '<p style="height:300px">Inserted above.</p>' + html;
    const source = first.getByRole('textbox', { name: 'Page source', exact: true });
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
    await expect(t2.locator('.thread-state').first()).toHaveAttribute(
      'title',
      'Another selection.',
    );
    await reply2.hover();
    await reply2.getByRole('button', { name: 'Message actions', exact: true }).click();
    await reply2.getByRole('menuitem', { name: 'Delete', exact: true }).click();
    await expect(t1).toContainText('Comment deleted');
    await expect(t1.getByRole('button', { name: 'Delete thread', exact: true })).toHaveCount(0);
    await expect(t2.getByRole('button', { name: 'Delete thread', exact: true })).toHaveCount(0);
    await expect(t2.getByRole('combobox')).toBeVisible();
    await commentsClose(first, 'discussion').click();
    await selectInRenderer(first, '#quote');
    await first.getByTestId('selection-ask').click();
    const secondCompose = first.getByRole('dialog', { name: 'Annotate selection' });
    const secondInput = await inputFor(secondCompose, agent.name);
    await secondInput.fill('A second discussion.');
    await secondCompose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    await expect(secondCompose.getByTestId('comment-thread')).toContainText('A second discussion.');
    await secondCompose.getByRole('button', { name: 'Collapse thread', exact: true }).click();
    await comments(first);
    await expect(commentsRegion(first).locator('.discussion-position')).toContainText('/ 2');
    await first.reload();
    await comments(first);
    await selectThread(first, threadId);
    await expect(first.getByTestId('comment-thread')).toHaveAttribute('data-thread-id', threadId);
    await expect(first.getByTestId('comment-thread')).toContainText('Comment deleted');
    await restartColab(world, door);
    await second.reload();
    await comments(second);
    await expect(commentsRegion(second).locator('.discussion-position')).toContainText('/ 2');
    await selectThread(second, threadId);
    await expect(second.getByTestId('comment-thread')).toContainText('Comment deleted');
    await expect(
      second.getByTestId('comment-thread').getByTestId('ask-reply').first(),
    ).toBeVisible();
    expect(messageId).toMatch(/^[a-f0-9-]{36}$/);
    // Opening, follow-up and the one resolve notification; Reopen sends nothing.
    expect(agent.received()).toHaveLength(3);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      3,
    );
  });
});

test('composer records plain annotations and replies without a recipient, then sends one explicit mention Ask', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('composer-agent', { gated: true });
    const browser = await pairBrowser(world, 'composer-author', { talk: true });
    await composerTrace(world, browser, door.address, 'composer-owner');
    const operations: string[] = [];
    browser.context.on('request', (request) => {
      if (
        request.method() !== 'POST' ||
        !new URL(request.url()).pathname.endsWith('/append') ||
        !request.headers()['content-type']?.includes('application/json')
      )
        return;
      const operation = (request.postDataJSON() as { operation?: string }).operation;
      if (typeof operation === 'string') operations.push(operation);
    });
    const created = createPage(
      world,
      'Composer acceptance',
      '<p id="quote">Frozen original quote.</p>',
      agent.pane,
    );
    const page = await browser.context.newPage();
    let injectedContextReads = 0;
    let bootstrapContext = true;
    // Let the initial mounted session open; fail subsequent context reads before the eager
    // composer can discover its directory. No signed response or admission is fabricated.
    const unavailableDirectoryContext = async (route: Route) => {
      if (route.request().method() !== 'GET') return route.continue();
      if (bootstrapContext) {
        bootstrapContext = false;
        return route.continue();
      }
      injectedContextReads++;
      await route.abort('failed');
    };
    await page.route('**/api/session', unavailableDirectoryContext);
    await page.goto(new URL(created.path, `${door.address}/`).href);
    await selectInRenderer(page, '#quote');
    await composerAssets(page, 'composer-owner');
    const opened = operations.filter((operation) => operation === 'session.open').length;
    await page.getByTestId('selection-ask').click();
    const compose = page.getByRole('dialog', { name: 'Annotate selection' });
    const input = compose.getByRole('combobox', { name: 'Message', exact: true });
    const plain = 'Plain annotation without a recipient.\nSecond line.';
    await expect(compose.locator('.annotation-hint[role=status]')).toContainText(
      text.messageAgentsUnavailable,
    );
    await expect(input).toBeEnabled();
    await input.fill(plain);
    await expect(
      compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }),
    ).toBeEnabled();
    await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    // A plain turn becomes the same anchored window, without opening Comments or requesting an agent.
    const thread = compose.getByTestId('comment-thread');
    await expect(thread).toBeVisible();
    await expect(openPanel(page)).toHaveCount(0);
    const original = thread
      .getByTestId('comment-entry')
      .filter({ hasText: 'Plain annotation without a recipient.' });
    const threadId = (await thread.getAttribute('data-thread-id'))!;
    const originalId = (await original.getAttribute('data-message-id'))!;
    await expect.poll(() => original.locator('.comment-body').textContent()).toBe(plain);
    await expect
      .poll(() => thread.locator('.thread-state').first().getAttribute('title'))
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
    await thread.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
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
    expect(injectedContextReads).toBeGreaterThan(0);
    expect(operations.filter((operation) => operation === 'session.open')).toHaveLength(opened);
    await expect(page.frameLocator('iframe').locator('#quote')).toHaveText(
      'Frozen original quote.',
    );
    await expect(thread.getByRole('button', { name: 'Try again', exact: true })).toBeEnabled();
    await page.unroute('**/api/session', unavailableDirectoryContext);
    // The failed read says so in place; Try again recovers without closing the thread.
    await expect(
      thread.getByText('Agents are unavailable. You can still post a comment.', { exact: true }),
    ).toBeVisible();
    await thread.getByRole('button', { name: 'Try again', exact: true }).click();
    // The fresh reply composer keeps the page creator mentioned, so the hint names it once the
    // directory is read again.
    await expect(thread.locator('.annotation-hint[role="status"]')).toContainText(
      text.messageAsksSend.replace('{agents}', agent.name),
    );
    await expect(thread.getByTestId('comment-entry')).toHaveCount(2);
    const question = `@${agent.name} Explain this exact quote.\n  Keep these spaces and this line.  `;
    // Sent text draws the mention as the chip, whose icon takes the place of the typed @.
    const shown = question.replace(`@${agent.name}`, agent.name);
    await reply.fill(question);
    await inputFor(thread, agent.name);
    await expect.poll(() => composerText(reply)).toBe(question);
    // Re-reading the shared field preserves exact multiline mention bytes.
    await inputFor(thread, agent.name);
    await expect.poll(() => composerText(reply)).toBe(question);
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
    expect(operations.filter((operation) => operation === 'session.open')).toHaveLength(opened);
    world.armNextBarrier('after');
    await thread.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    const parked = await world.barrierEntered();
    // #5815 reserves the pending echo until this explicit send returns from the held core call.
    const entry = thread.getByTestId('ask-entry');
    await expect(entry).toHaveCount(0);
    await expect(thread.getByTestId('comment-echo')).toContainText('Explain this exact quote.');
    expect(typeof parked.operationId).toBe('string');
    const operationId = parked.operationId as string;
    await until(() => agent.received().length === 1, 'one explicitly selected annotation Ask');
    const received = agent.received()[0];
    const requestId = received.requestId as string;
    expect(requestId).toMatch(/^req_[0-9a-f-]+$/);
    expect(received.identityId).toBe(agent.id);
    expect(typeof received.message).toBe('string');
    const delivered = received.message as string;
    expect(delivered.slice(-question.length)).toBe(question);
    expect(agent.received()[0].message).toContain('Frozen original quote.');
    expect(agent.received()[0].message).toContain(`@${agent.name}`);
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
    await expect(entry).toHaveAttribute('data-operation-id', operationId);
    await expect(entry).toHaveAttribute('data-ledger-state', 'accepted');
    const askComment = thread
      .getByTestId('comment-entry')
      .filter({ hasText: 'Explain this exact quote.' });
    const askMessageId = (await askComment.getAttribute('data-message-id'))!;
    await expect.poll(() => askComment.locator('.comment-body').textContent()).toBe(shown);
    writeFileSync(path.join(agent.gate, `${requestId}.release`), '');
    const expectedReply = `ask-reply:${createHash('sha256').update(delivered).digest('hex').slice(0, 16)}`;
    await expect.poll(() => entry.getByTestId('ask-reply').textContent()).toBe(expectedReply);
    await until(
      () => agent.rows().some((row) => row.event === 'replied' && row.requestId === requestId),
      'reply for the original request',
    );
    await page.reload();
    await comments(page);
    await selectThread(page, threadId);
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
      .poll(() => restored.locator('.thread-state').first().getAttribute('title'))
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

test('proposal decisions notify the exact proposer once and survive resolution and reload', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('proposal-agent', { gated: true });
    const browser = await pairBrowser(world, 'proposal-author', { talk: true });
    const created = createPage(
      world,
      'Proposal review',
      '<h1>Project notes</h1><p>Review the heading.</p>',
      agent.pane,
    );
    const added = JSON.parse(
      run(
        world,
        world.binaries.colab,
        [
          'proposal',
          'add',
          created.pageId,
          '--title',
          'Use a clearer project heading',
          '--body',
          'Explain what this page contains.',
          '--json',
        ],
        undefined,
        agent.pane,
      ),
    );
    const page = await openPage(door, browser, created);
    const card = page.locator(
      `.proposal-inline .proposal-card[data-proposal-id="${added.proposalId}"]`,
    );
    await expect(card).toBeVisible();
    const captures = process.env.COLAB_PROPOSAL_NATIVE_CAPTURE_DIR;
    async function capture(state: string) {
      if (!captures) return;
      mkdirSync(captures, { recursive: true });
      for (const width of [1440, 390])
        for (const theme of ['light', 'dark']) {
          await page.setViewportSize({ width, height: 900 });
          await page.evaluate((theme) => {
            document.documentElement.dataset.theme = theme;
          }, theme);
          await expect(card).toBeVisible();
          const spacer = page
            .frameLocator('.frame-host iframe')
            .locator('[data-colab-proposal-slot]');
          await expect
            .poll(async () => {
              const reserved = await spacer.boundingBox();
              const displayed = await card.boundingBox();
              if (!reserved || !displayed) return false;
              return reserved.height >= displayed.height && reserved.height - displayed.height <= 1;
            })
            .toBe(true);
          await page.screenshot({ path: `${captures}/${state}-${width}-${theme}.png` });
        }
    }
    await capture('open');
    await card.getByRole('button', { name: 'Approve', exact: true }).click();
    await until(() => agent.received().length === 1, 'proposal decision notification');
    await expect(card.locator('.proposal-state')).toHaveText('Approved');
    const received = agent.received()[0];
    expect(received.identityId).toBe(agent.id);
    const delivered = received.message as string;
    expect(delivered).toContain('[remote: proposal-author]\nPage: Proposal review\nLink: ');
    expect(delivered).toContain(
      '\n\nQuote:\n\n\nComment:\nApproved: Use a clearer project heading',
    );
    await capture('approved-waiting');
    writeFileSync(path.join(agent.gate, `${received.requestId}.release`), '');
    await until(
      () =>
        agent.rows().some((row) => row.event === 'replied' && row.requestId === received.requestId),
      'proposal correlated reply',
    );
    const reply = agent
      .rows()
      .find((row) => row.event === 'replied' && row.requestId === received.requestId)!
      .body as string;
    await expect(card.getByTestId('ask-reply')).toHaveText(reply);
    await capture('approved-replied');
    await card.getByRole('button', { name: 'Follow up', exact: true }).click();
    const composer = card.getByRole('combobox');
    await expect(composer).toBeFocused();
    await expect(composer).toContainText('@proposal-agent');
    await expect(page.locator('.annotation-new')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Delete thread', exact: true })).toHaveCount(0);
    await capture('follow-up');
    await composer.press('Escape');
    await expect(card.getByRole('combobox')).toHaveCount(0);
    await card.getByRole('button', { name: 'Follow up', exact: true }).click();
    await card.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(card.getByRole('combobox')).toHaveCount(0);
    await card.getByRole('button', { name: 'Resolve', exact: true }).click();
    await expect(card).toHaveAttribute('data-resolved', 'true');
    await capture('resolved');
    await card.getByRole('button', { name: 'Reopen', exact: true }).click();
    await expect(card.locator('.proposal-state')).toHaveText('Approved');
    await capture('reopened');
    await page.reload();
    await expect(card.locator('.proposal-state')).toHaveText('Approved');
    await expect(card.getByTestId('ask-reply')).toHaveText(reply);
    const rows = JSON.parse(
      run(world, world.binaries.colab, ['proposal', 'ls', created.pageId, '--json']),
    ).proposals;
    const exportParent = path.join(world.root, 'proposal-export');
    mkdirSync(exportParent, { mode: 0o700 });
    const exported = JSON.parse(
      run(world, world.binaries.colab, ['export', created.pageId, '--dir', exportParent, '--json']),
    );
    const conversation = JSON.parse(
      readFileSync(path.join(exported.directory, 'conversations.json'), 'utf8'),
    );
    expect(conversation.asks).toHaveLength(1);
    const ask = conversation.asks[0];
    expect(ask.thread).toBe(added.proposalId);
    expect(ask.machine).toBe(rows[0].proposal.proposer.machineId);
    expect(ask.agent).toBe(rows[0].proposal.proposer.agentId);
    expect(ask.messageIds).toEqual([rows[0].comments[0].id]);
    expect(ask.requestId).toBe(received.requestId);
    expect(delivered).toBe(`[remote: proposal-author]\n${ask.message}`);
    expect(rows).toHaveLength(1);
    expect(rows[0].decision.decision).toBe('approved');
    expect(rows[0].resolved).toBe(false);
    expect(
      rows[0].comments.filter(
        (comment: { body: string }) => comment.body === 'Approved: Use a clearer project heading',
      ),
    ).toHaveLength(1);
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
  });
});

test('answer blocks publish verified normal annotations and keep changes and agent replies in one thread', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('answer-agent', { gated: true });
    const browser = await pairBrowser(world, 'answer-reader', { talk: true });
    const created = createPage(
      world,
      'Question review',
      '<h1>Project notes</h1><p>Before the question.</p><tmt-question data-id="10000000-0000-4000-8000-000000000090" data-kind="single" data-agent="answer-agent"><p data-question>Which heading should we use?</p><span data-choice>Keep it</span><span data-choice>Change it</span></tmt-question><p>The page continues here.</p>',
      agent.pane,
      [{ id: '10000000-0000-4000-8000-000000000090', agent: agent.id }],
    );
    const page = await openPage(door, browser, created);
    const card = page.locator('.question-group-card');
    const row = card.locator('.question-line');
    await expect(row).toHaveAttribute('aria-expanded', 'false');
    await row.click();
    await expect(
      card.getByRole('button', { name: 'Send answer · to @answer-agent', exact: true }),
    ).toBeVisible();
    await expect(card.getByRole('button', { name: /^Send answer/ })).toHaveText('Send answer');
    await card.getByLabel('Keep it', { exact: true }).check();
    await card.getByRole('button', { name: /^Send answer/ }).click();
    await expect(row).toHaveAttribute('data-state', 'answered');
    await expect(row.locator('.question-line-answer')).toHaveText('Keep it');
    await expect(row).toHaveAttribute('aria-expanded', 'false');
    await row.click();
    await until(() => agent.received().length === 1, 'first question answer');
    const first = agent.received()[0];
    expect(first.identityId).toBe(agent.id);
    expect(first.message).toContain('Quote:\nWhich heading should we use?');
    expect(first.message).toContain(
      'Answer to “Which heading should we use?”: Keep it\nPage: Question review\n@answer-agent',
    );
    writeFileSync(path.join(agent.gate, `${first.requestId}.release`), '');
    await until(
      () =>
        agent.rows().some((row) => row.event === 'replied' && row.requestId === first.requestId),
      'question correlated reply',
    );
    const reply = agent
      .rows()
      .find((row) => row.event === 'replied' && row.requestId === first.requestId)!.body as string;
    await expect(
      card.getByRole('heading', { name: 'Conversation · Agent replied', exact: true }),
    ).toBeVisible();
    await expect(card.getByTestId('ask-reply')).toHaveText(reply);
    const threads = () =>
      JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])).threads;
    const initial = threads();
    expect(initial).toHaveLength(1);
    expect(initial[0].anchor.exact).toBe('Which heading should we use?');
    expect(initial[0].comments[0].questionId).toBe('10000000-0000-4000-8000-000000000090');
    const metadata = () =>
      JSON.parse(run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']));
    expect(metadata().questionStates[0].state).toBe('answered');
    await card.getByRole('button', { name: 'Change answer', exact: true }).click();
    await card.getByLabel('Change it', { exact: true }).check();
    await card.getByRole('button', { name: /^Send answer/ }).click();
    await until(() => agent.received().length === 2, 'changed question answer');
    await expect(row).toHaveAttribute('data-state', 'answered');
    await expect(row.locator('.question-line-answer')).toHaveText('Change it');
    const changed = threads();
    expect(changed).toHaveLength(1);
    expect(changed[0].id).toBe(initial[0].id);
    expect(changed[0].comments).toHaveLength(2);
    expect(
      changed[0].comments.map((comment: { questionId: string }) => comment.questionId),
    ).toEqual(Array(2).fill('10000000-0000-4000-8000-000000000090'));
    await page.reload();
    await expect(row).toHaveAttribute('data-state', 'answered');
    await expect(row.locator('.question-line-answer')).toHaveText('Change it');
    await expect(row).toHaveAttribute('aria-expanded', 'false');
    await row.click();
    await expect(card.getByRole('button', { name: 'Change answer', exact: true })).toBeEnabled();
    expect(agent.received()).toHaveLength(2);
    const captures = process.env.COLAB_QUESTION_NATIVE_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      await page.screenshot({ path: `${captures}/question-native-reloaded.png` });
    }
  });
});

test('paired comment-only selection creates one durable comment without any Ask dispatch', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'selection-comment-only');
    const created = createPage(
      world,
      'Comment selection',
      '<p id="quote">Comment without talk.</p>',
    );
    const page = await openPage(door, browser, created);
    const mode = page.getByRole('button', { name: 'Selection mode', exact: true });
    await mode.click();
    await expect(mode).toHaveAttribute('aria-pressed', 'true');
    await page.frameLocator('iframe').locator('#quote').click();
    const window = page.getByRole('dialog', { name: 'Annotate selection' });
    const input = window.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toBeFocused();
    await expect(input).toHaveText('', { useInnerText: true });
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    await input.fill('A durable selected comment');
    await window.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    await expect(window.getByTestId('comment-entry').locator('.comment-body')).toHaveText(
      'A durable selected comment',
    );
    await page.reload();
    await comments(page);
    await selectDiscussion(page, 'Comment without talk.');
    await expect(page.getByTestId('comment-entry').locator('.comment-body')).toHaveText(
      'A durable selected comment',
    );
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});

test('answer Send names start with visible text through the served question group', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const browser = await pairBrowser(world, 'answer-names', { talk: true });
    const created = createPage(
      world,
      'Question names',
      '<h1>Question names</h1><tmt-questions data-id="10000000-0000-4000-8000-000000000100"><p data-title>Review choices</p><tmt-question data-id="10000000-0000-4000-8000-000000000101" data-kind="single"><p data-question>Keep the heading?</p><span data-choice>Yes</span><span data-choice>No</span></tmt-question><tmt-question data-id="10000000-0000-4000-8000-000000000102" data-kind="short"><p data-question>Any details?</p></tmt-question></tmt-questions>',
      undefined,
      [
        { id: '10000000-0000-4000-8000-000000000101' },
        { id: '10000000-0000-4000-8000-000000000102' },
      ],
    );
    const page = await openPage(door, browser, created);
    const group = page.locator('[data-question-group]');
    const rows = group.locator('.question-line');
    await rows.first().click();
    const first = group.locator('.question-card:visible');
    await first.getByLabel('Yes', { exact: true }).check();
    const send = group.getByRole('button', { name: /^Send 1 answer/ });
    await expect(send).toHaveText('Send 1 answer');
    await expect(
      group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
    ).toBeVisible();
    await send.click();
    await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
    await expect(rows.first()).toHaveAttribute('data-state', 'answered');
    await expect(rows.first().locator('.question-line-answer')).toHaveText('Yes');
    await rows.nth(1).click();
    const last = group.locator('.question-card:visible');
    await last.getByLabel('Your answer', { exact: true }).fill('One detail');
    await expect(group.getByRole('button', { name: /^Send 1 answer/ })).toHaveText('Send 1 answer');
    await expect(
      group.getByRole('button', { name: 'Send 1 answer · to thread only', exact: true }),
    ).toBeVisible();
    const threads = JSON.parse(
      run(world, world.binaries.colab, ['threads', created.pageId, '--json']),
    ).threads;
    expect(threads).toHaveLength(1);
    expect(threads[0].anchor.exact).toBe('Keep the heading?');
    expect(threads[0].comments[0].body).toContain('Answer to “Keep the heading?”: Yes');
  });
});

test('a served question group records three answers and asks each recipient once, with one canonical reply and ordinary Change answer', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const alpha = await world.startAgent('batch-alpha', { gated: true });
    const beta = await world.startAgent('batch-beta', { gated: true });
    const browser = await pairBrowser(world, 'batch-reader', { talk: true });
    const created = createPage(
      world,
      'Batch review',
      '<h1>Batch review</h1><tmt-questions data-id="10000000-0000-4000-8000-000000000110"><p data-title>Review choices</p><tmt-question data-id="10000000-0000-4000-8000-000000000111" data-kind="single" data-agent="batch-alpha"><p data-question>Keep the heading?</p><span data-choice>Yes</span><span data-choice>No</span></tmt-question><tmt-question data-id="10000000-0000-4000-8000-000000000112" data-kind="short" data-agent="batch-beta"><p data-question>Any details?</p></tmt-question><tmt-question data-id="10000000-0000-4000-8000-000000000113" data-kind="long" data-agent="batch-alpha"><p data-question>Any tradeoffs?</p></tmt-question></tmt-questions>',
      alpha.pane,
      [
        { id: '10000000-0000-4000-8000-000000000111', agent: alpha.id },
        { id: '10000000-0000-4000-8000-000000000112', agent: beta.id },
        { id: '10000000-0000-4000-8000-000000000113', agent: alpha.id },
      ],
    );
    const page = await openPage(door, browser, created);
    const group = page.locator('[data-question-group]');
    const rows = group.locator('.question-line');
    await rows.first().click();
    const current = () => group.locator('.question-card:visible');
    await expect(
      group.getByRole('button', { name: 'Remove @batch-alpha', exact: true }),
    ).toHaveCount(1);
    await expect(
      group.getByRole('button', { name: 'Remove @batch-beta', exact: true }),
    ).toHaveCount(1);
    await current().getByLabel('Yes', { exact: true }).check();
    await rows.nth(1).click();
    await current().getByLabel('Your answer', { exact: true }).fill('Keep the introduction brief');
    await rows.nth(2).click();
    await current().getByLabel('Your answer', { exact: true }).fill('Explain the tradeoffs');
    expect(alpha.received()).toHaveLength(0);
    expect(beta.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    await group
      .getByRole('button', { name: 'Send 3 answers · to @batch-alpha, @batch-beta', exact: true })
      .click();
    await until(
      () => alpha.received().length === 1 && beta.received().length === 1,
      'one batch Ask per recipient',
    );
    await expect(rows.nth(2)).toHaveAttribute('aria-expanded', 'false');
    await rows.nth(2).click();
    for (const agent of [alpha, beta]) {
      const receipt = agent.received()[0];
      expect(receipt.identityId).toBe(agent.id);
      expect(receipt.message).toContain('Quote:\nKeep the heading?');
      expect(receipt.message).toContain(
        'Answers to 3 questions:\nKeep the heading?: Yes\nAny details?: Keep the introduction brief\nAny tradeoffs?: Explain the tradeoffs',
      );
      writeFileSync(path.join(agent.gate, `${receipt.requestId}.release`), '');
    }
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      2,
    );
    const threads = () =>
      JSON.parse(run(world, world.binaries.colab, ['threads', created.pageId, '--json'])).threads;
    const before: { id: string; anchor: { exact: string }; comments: { body: string }[] }[] =
      threads();
    expect(before).toHaveLength(3);
    expect(new Set(before.map((thread) => thread.id)).size).toBe(3);
    expect(before.map((thread) => thread.anchor.exact).sort()).toEqual([
      'Any details?',
      'Any tradeoffs?',
      'Keep the heading?',
    ]);
    for (const thread of before) expect(thread.comments).toHaveLength(1);
    await expect(group.getByTestId('ask-reply')).toHaveCount(2);
    await expect(current().getByTestId('ask-reply')).toHaveCount(0);
    // Admitted Ask projection does not promise recipient ordering. Pin both identities
    // and the shared count without treating their arrival order as a UI contract.
    await expect(current().locator('.question-shared-ask')).toHaveText(
      /^Asked (?:batch-alpha, batch-beta|batch-beta, batch-alpha) with 3 answers\.$/,
    );
    await current().getByRole('button', { name: 'Open reply · Q1', exact: true }).click();
    await expect(current().getByTestId('ask-entry').first()).toBeFocused();
    await expect(current().getByTestId('ask-reply')).toHaveCount(2);
    await page.reload();
    await expect(group.getByTestId('ask-reply')).toHaveCount(2);
    expect(alpha.received()).toHaveLength(1);
    expect(beta.received()).toHaveLength(1);
    await rows.nth(1).click();
    await current().getByRole('button', { name: 'Change answer', exact: true }).click();
    await current().getByLabel('Your answer', { exact: true }).fill('One revised detail');
    // Reload restores answer history before the fresh directory has seeded recipients.
    await group
      .getByRole('button', {
        name: 'Send 1 answer · to @batch-alpha, @batch-beta',
        exact: true,
      })
      .click();
    await until(
      () => alpha.received().length === 2 && beta.received().length === 2,
      'one-question follow-up per recipient',
    );
    for (const agent of [alpha, beta]) {
      expect(agent.received()[1].message).toContain('Answer to “Any details?”: One revised detail');
      expect(agent.received()[1].message).not.toContain('Answers to 3 questions:');
    }
    const after: typeof before = threads();
    expect(after).toHaveLength(3);
    for (const original of before) {
      const updated = after.find((thread) => thread.id === original.id);
      expect(updated?.comments).toHaveLength(original.anchor.exact === 'Any details?' ? 2 : 1);
    }
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      4,
    );
  });
});

test('served question controls follow closed details and retain an unsent draft', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('details-answer-agent', { gated: true });
    const browser = await pairBrowser(world, 'details-answer-reader', { talk: true });
    const id = '10000000-0000-4000-8000-000000000096';
    const created = createPage(
      world,
      'Questions in details',
      `<h1>Review notes</h1><details id="questions"><summary>Review questions</summary><tmt-question data-id="${id}" data-kind="short"><p data-question>Which heading?</p></tmt-question></details><p>After the questions.</p>`,
      agent.pane,
      [{ id, agent: agent.id }],
    );
    const page = await openPage(door, browser, created);
    const details = page.frameLocator('iframe').locator('#questions');
    const row = page.locator('.question-line');
    const slot = page.locator('.question-compact-inline');
    await expect(slot).toHaveCSS('display', 'none');
    await details.locator('summary').click();
    await expect(row).toBeVisible();
    await row.click();
    const answer = page.getByLabel('Your answer', { exact: true });
    await answer.fill('A retained draft');
    await details.locator('summary').click();
    await expect(slot).toHaveCSS('display', 'none');
    expect(
      await answer.evaluate((node) => {
        (node as HTMLElement).focus();
        return document.activeElement === node;
      }),
    ).toBe(false);
    await details.locator('summary').click();
    await expect(answer).toHaveValue('A retained draft');
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});

test('served non-compact slots hide through nested details without losing drafts or publishing', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('details-slots-agent', { gated: true });
    const browser = await pairBrowser(world, 'details-slots-reader', { talk: true });
    const created = createPage(world, 'Slots in details', '<h1>Release review</h1>', agent.pane);
    const proposal = JSON.parse(
      run(
        world,
        world.binaries.colab,
        [
          'proposal',
          'add',
          created.pageId,
          '--title',
          'Review this proposal',
          '--body',
          'Keep the release notes clear.',
          '--json',
        ],
        undefined,
        agent.pane,
      ),
    );
    const source = `<h1>Release review</h1><details id="outer"><summary>Review decisions</summary><details id="inner" open><summary>Section details</summary><tmt-review data-id="10000000-0000-4000-8000-000000000097" data-agent="${agent.name}"><h2 data-title>Review section</h2><p>Review the release notes.</p></tmt-review><tmt-proposal data-id="${proposal.proposalId}"></tmt-proposal><tmt-read data-id="10000000-0000-4000-8000-000000000098" data-agent="${agent.name}" data-why="Read before release"><h2 data-title>Release reading</h2><p>Read these release notes.</p></tmt-read></details></details><p>After the decisions.</p>`;
    run(
      world,
      world.binaries.colab,
      patchArgs(world, created.pageId, ['--file', '-', '--json']),
      source,
      agent.pane,
    );
    const page = await openPage(door, browser, created);
    const outer = page.frameLocator('iframe').locator('#outer'),
      inner = page.frameLocator('iframe').locator('#inner');
    const slots = page.locator('[data-inline-review],[data-inline-proposal],[data-inline-reading]');
    await expect(slots).toHaveCount(3);
    for (const slot of await slots.all()) await expect(slot).toHaveCSS('display', 'none');
    await outer.locator(':scope > summary').click();
    for (const slot of await slots.all()) await expect(slot).toBeVisible();
    const review = page.locator('[data-inline-review]'),
      proposalCard = page.locator('[data-inline-proposal]');
    await review.getByRole('button', { name: 'Add a note', exact: true }).click();
    await review.getByLabel('Note (optional)', { exact: true }).fill('An unsent review');
    await proposalCard.getByRole('button', { name: 'Follow up', exact: true }).click();
    await proposalCard.getByRole('combobox').fill('An unsent proposal');
    await inner.locator(':scope > summary').click();
    for (const slot of await slots.all()) {
      await expect(slot).toHaveCSS('display', 'none');
      expect(
        await slot
          .locator('button')
          .first()
          .evaluate((node) => {
            (node as HTMLElement).focus();
            return document.activeElement === node;
          }),
      ).toBe(false);
    }
    await outer.locator(':scope > summary').click();
    await outer.locator(':scope > summary').click();
    for (const slot of await slots.all()) await expect(slot).toHaveCSS('display', 'none');
    await inner.locator(':scope > summary').click();
    await expect(review.getByLabel('Note (optional)', { exact: true })).toHaveValue(
      'An unsent review',
    );
    await expect(proposalCard.getByRole('combobox')).toHaveText('An unsent proposal');
    const after = JSON.parse(
      run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']),
    );
    expect(after.source).toBe(source);
    expect(agent.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});
