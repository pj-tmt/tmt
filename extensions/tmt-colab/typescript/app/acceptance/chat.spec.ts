import { pageAction } from '../test/page-actions.js';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { pairBrowser, restartColab, startDoor } from './harness/browser.js';
import {
  composeChat,
  composerTrace,
  composerAssets,
  createPage,
  freePort,
  openChat,
  openPage,
  sendChat,
  askEntry,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
test('page-visible device Chat threads send exact bytes once, preserve drafts, keep overlay geometry and survive reload/restart', async () => {
  await withWorld(async (world) => {
    const captureDirectory =
      process.env.COLAB_1817_CAPTURE_DIR ?? path.join(world.root, 'captures');
    fs.mkdirSync(captureDirectory, { recursive: true });
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('chat-agent', { gated: true });
    const firstBrowser = await pairBrowser(world, 'chat-author');
    const secondBrowser = await pairBrowser(world, 'chat-viewer');
    await composerTrace(world, firstBrowser, door.address, 'chat-owner');
    await composerTrace(world, secondBrowser, door.address, 'chat-viewer');
    const source =
      '<script>const every=Array.prototype.every;window.anchorTraffic=[];Array.prototype.every=function(callback,...args){try{window.anchorTraffic.push(JSON.stringify(this))}catch{}return every.call(this,callback,...args)};</script><style>body{margin:0;padding:24px;font:16px/1.6 sans-serif}p{max-width:70ch}</style><h1>Page conversations</h1>' +
      Array.from(
        { length: 40 },
        (_, index) =>
          `<section><h2>Paragraph ${index + 1}</h2><p>Visible page content moves beneath the one-row header. The Chat overlay keeps this page at its original width.</p></section>`,
      ).join('') +
      '<h2 id="scroll-end">END OF PAGE</h2>';
    const created = createPage(world, 'Page conversations', source, agent.pane);
    const first = await openPage(door, firstBrowser, created);
    const second = await openPage(door, secondBrowser, created);
    await expect(
      first
        .frameLocator('iframe')
        .getByRole('heading', { name: 'Page conversations', exact: true }),
    ).toBeVisible();
    await composerAssets(first, 'chat-owner');
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
      await first.screenshot({
        path: path.join(captureDirectory, `1645-native-${width}-light-long-top.png`),
      });
      await first.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
      const windowScrollTop = await first.evaluate(() => document.scrollingElement!.scrollTop);
      const frameScrollTop = await first
        .frameLocator('iframe')
        .locator('html')
        .evaluate((node) => node.ownerDocument.scrollingElement!.scrollTop);
      expect(windowScrollTop).toBeGreaterThan(1000);
      expect(frameScrollTop).toBe(0);
      await expect(first.frameLocator('iframe').locator('#scroll-end')).toBeInViewport();
      await first.screenshot({
        path: path.join(captureDirectory, `1645-native-${width}-light-long-scrolled.png`),
      });
      console.log(
        JSON.stringify({ width, windowScrollTop, frameScrollTop, marker: 'END OF PAGE' }),
      );
    }
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => window.scrollTo(0, 0));
    await expect
      .poll(async () => {
        const frameHeight = (await first.locator('iframe').boundingBox())!.height;
        const contentHeight = await first
          .frameLocator('iframe')
          .locator('body')
          .evaluate((node) => node.getBoundingClientRect().height);
        return Math.abs(frameHeight - contentHeight);
      })
      .toBeLessThan(2);
    const before = await first.locator('iframe').boundingBox();
    await openChat(first);
    const panel = first.getByTestId('chat-panel');
    const input = panel.getByRole('combobox', { name: 'Message' });
    await expect(input).toHaveText(`@${agent.name} `, { useInnerText: true });
    await expect(panel.locator('.annotation-compose details')).toHaveCount(0);
    await expect(panel).toContainText('Visible to everyone with page access.');
    await expect(panel.getByRole('button', { name: 'Delete thread', exact: true })).toHaveCount(0);
    expect(await first.locator('iframe').boundingBox()).toEqual(before);
    const opening = '<script>private Chat turn</script> Explain this page.';
    await input.fill(opening);
    await first.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await openChat(first);
    await expect(input).toHaveText(opening, { useInnerText: true });
    expect(agent.received()).toHaveLength(0);
    const draft = await composeChat(
      first,
      agent,
      '<script>private Chat turn</script> Explain this page.',
    );
    const sent = await sendChat(first, draft);
    await until(() => agent.received().length === 1, 'first Chat received');
    expect(draft.delivered()).toContain('Quote:\n\n\nComment:');
    expect(draft.delivered()).toContain(
      `Comment:\n@${agent.name} <script>private Chat turn</script> Explain this page.`,
    );
    const own = panel.getByTestId('chat-thread').first();
    const writer = (await own.getAttribute('data-writer'))!;
    await expect(own).toHaveAttribute('data-thread-id', writer);
    await expect(askEntry(first, sent.operationId).getByTestId('ask-state')).toContainText(
      'Waiting for',
    );
    const requestId = agent.received()[0].requestId as string;
    fs.writeFileSync(`${agent.gate}/${requestId}.release`, '');
    const reply =
      'ask-reply:' + createHash('sha256').update(draft.delivered()).digest('hex').slice(0, 16);
    await expect(askEntry(first, sent.operationId).getByTestId('ask-reply')).toHaveText(reply);
    const exchange = askEntry(first, sent.operationId);
    const userTurn = exchange.locator('.conversation-turn[data-turn-role=user]');
    await expect(userTurn.locator('header')).toContainText('You · just now');
    await expect(exchange.getByTestId('ask-state')).toHaveCount(0);
    await expect(exchange).toHaveAttribute('data-ledger-state', 'accepted');
    // One quiet menu holds Delete for one's own message; no disclosure of the sent bytes exists.
    await userTurn.hover();
    await userTurn.getByRole('button', { name: 'Message actions', exact: true }).click();
    await expect(userTurn.getByRole('menuitem')).toHaveText(['Delete']);
    await userTurn.getByRole('button', { name: 'Message actions', exact: true }).press('Escape');
    await expect(userTurn.getByRole('menuitem')).toHaveCount(0);
    await expect(userTurn.locator('details')).toHaveCount(0);
    await expect(panel).not.toContainText('Show exactly what');
    await expect(exchange.locator('.conversation-turn[data-turn-role=agent]')).toHaveCount(1);
    await expect(exchange.getByTestId('ask-reply-attribution')).toContainText(agent.name);
    await expect(exchange.locator('.conversation-turn[data-turn-role=agent] details')).toHaveCount(
      0,
    );
    await openChat(second);
    await expect(
      second.getByTestId('chat-thread').filter({ hasText: 'private Chat turn' }),
    ).toBeVisible();
    await expect(askEntry(second, sent.operationId).getByTestId('ask-reply')).toHaveText(reply);
    await expect(panel.locator('script')).toHaveCount(0);
    await expect(first.frameLocator('iframe').locator('[data-colab-thread]')).toHaveCount(0);
    await expect(panel.locator('.annotation-compose > details')).toHaveCount(0);
    for (const width of [1440, 390]) {
      await first.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await first.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
        await input.fill(`@${agent.name} A retained follow-up draft.`);
        await first.screenshot({
          path: path.join(captureDirectory, `1645-native-${width}-${theme}-chat.png`),
        });
        const menu = first.getByRole('button', { name: 'Message actions' }).first();
        await menu.click();
        await expect(first.getByRole('menuitem')).toBeVisible();
        await first.screenshot({
          path: path.join(captureDirectory, `1690-native-${width}-${theme}-menu.png`),
        });
        await menu.press('Escape');
        await expect(first.getByRole('menuitem')).toHaveCount(0);
        await input.fill('@');
        const option = first.getByRole('option');
        await expect(option).toBeInViewport();
        await expect
          .poll(() =>
            option.evaluate((node) => {
              const box = node.getBoundingClientRect();
              return node.contains(
                document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2),
              );
            }),
          )
          .toBe(true);
        await first.screenshot({
          path: path.join(captureDirectory, `1645-native-${width}-${theme}-autocomplete.png`),
        });
        await option.click();
        await expect(input).toHaveText(`@${agent.name} `, { useInnerText: true });
        await input.fill('@');
        await input.press('Escape');
        await expect(input).toBeVisible();
      }
    }
    await expect(input).toBeFocused();
    await expect(input).toHaveText('@', { useInnerText: true });
    const retainedComposer = await input.elementHandle();
    await first.setViewportSize({ width: 1440, height: 900 });
    await first.evaluate(() => (document.documentElement.dataset.theme = 'light'));
    await expect
      .poll(() =>
        first
          .locator('.page-drawer[data-panel=chat][open]')
          .evaluate((node) => node.matches(':modal')),
      )
      .toBe(false);
    await expect(input).toBeFocused();
    expect(await input.evaluate((node, retained) => node === retained, retainedComposer)).toBe(
      true,
    );
    await expect(input).toHaveText('@', { useInnerText: true });
    const next = await composeChat(first, agent, 'Follow up with the earlier answer.');
    const follow = await sendChat(first, next);
    await until(() => agent.received().length === 2, 'follow-up Chat received');
    expect(next.delivered()).toContain('Earlier conversation (quoted data)');
    expect(next.delivered()).toContain(opening);
    expect(next.delivered()).toContain(reply);
    fs.writeFileSync(`${agent.gate}/${agent.received()[1].requestId}.release`, '');
    await expect(askEntry(first, follow.operationId).getByTestId('ask-reply')).toBeVisible();
    const secondDraft = await composeChat(second, agent, 'A separate asking-device conversation.');
    const secondSent = await sendChat(second, secondDraft);
    await until(() => agent.received().length === 3, 'second device Chat received');
    fs.writeFileSync(`${agent.gate}/${agent.received()[2].requestId}.release`, '');
    await expect(askEntry(second, secondSent.operationId).getByTestId('ask-reply')).toBeVisible();
    await expect(panel.getByTestId('chat-thread')).toHaveCount(2);
    const ids = await panel.getByTestId('chat-thread').evaluateAll((nodes) =>
      nodes.map((node) => ({
        id: (node as HTMLElement).dataset.threadId,
        writer: (node as HTMLElement).dataset.writer,
      })),
    );
    expect(new Set(ids.map((value) => value.id)).size).toBe(2);
    expect(ids.every((value) => value.id === value.writer)).toBe(true);
    const traffic = await first
      .frameLocator('iframe')
      .locator('html')
      .evaluate(
        (node) =>
          (node.ownerDocument.defaultView as unknown as { anchorTraffic: string[] }).anchorTraffic,
      );
    expect(traffic.length).toBeGreaterThan(0);
    expect(traffic.join('')).not.toContain('private Chat turn');
    expect(traffic.join('')).not.toContain('Follow up with the earlier answer');
    await (await pageAction(first, 'Comments')).click();
    await expect(first.getByTestId('annotation-row')).toHaveCount(0);
    await expect(first.getByRole('button', { name: 'Delete thread', exact: true })).toHaveCount(0);
    await first.getByRole('button', { name: '+ Comment on page', exact: true }).click();
    await first
      .getByRole('combobox', { name: 'Post comment', exact: true })
      .fill('Ordinary page comment');
    await first.getByRole('button', { name: 'Post comment', exact: true }).click();
    await expect(first.getByTestId('annotation-row')).toHaveCount(1);
    await first.reload();
    await openChat(first);
    await expect(first.getByTestId('chat-thread')).toHaveCount(2);
    await expect(askEntry(first, sent.operationId).getByTestId('ask-reply')).toHaveText(reply);
    await restartColab(world, door);
    await first.reload();
    await openChat(first);
    await expect(first.getByTestId('chat-thread')).toHaveCount(2);
    await expect(askEntry(first, sent.operationId).getByTestId('ask-reply')).toHaveText(reply);
    expect(agent.received()).toHaveLength(3);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      3,
    );
  });
});
