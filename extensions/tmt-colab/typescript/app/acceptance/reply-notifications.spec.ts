import { expect, test } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { commentsRegion } from '../test/comments-region.js';
import { pageAction } from '../test/page-actions.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  composeChat,
  createPage,
  freePort,
  openPage,
  selectInRenderer,
  sendChat,
  run,
  annotationInput,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { captureResponsive } from './harness/captures.js';

test.afterEach(disposeActiveWorlds);

test('Chat unread remains after dismissing its toast, clears at the visible end, and stays cleared after reload', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('chat-unread-agent', { gated: true });
    const author = await pairBrowser(world, 'chat-unread-author', { talk: true });
    const viewer = await pairBrowser(world, 'chat-unread-viewer');
    const created = createPage(world, 'Chat unread', '<p>Chat unread fixture.</p>');
    const page = await openPage(door, author, created);
    const other = await openPage(door, viewer, created);
    const button = page
      .locator('.page-floating-actions')
      .getByRole('button', { name: /^Chat(?: · Unread)?$/ });
    const peerButton = other
      .locator('.page-floating-actions')
      .getByRole('button', { name: /^Chat(?: · Unread)?$/ });
    const ask = await sendChat(page, await composeChat(page, agent, 'One verified reply.'));
    await until(() => agent.received().length === 1, 'one unread Ask');
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    const request = agent.received()[0].requestId as string;
    writeFileSync(path.join(agent.gate, `${request}.release`), '');
    await until(() => agent.rows().some((row) => row.event === 'replied'), 'durable unread reply');
    await expect(button).toHaveAccessibleName('Chat · Unread');
    await expect(peerButton).toHaveAccessibleName('Chat');
    await page.getByRole('button', { name: 'Dismiss Chat reply', exact: true }).click();
    await expect(page.locator('.tmt-ui-notification')).toHaveCount(0);
    await expect(button).toHaveAccessibleName('Chat · Unread');
    await captureResponsive(page, 'chat-unread-persistent');
    await button.click();
    await expect(page.getByTestId('chat-panel').getByTestId('ask-reply')).toBeVisible();
    await expect(page.locator('.page-floating-actions .chat-unread')).toHaveCount(0);
    await captureResponsive(page, 'chat-unread-open');
    await page.keyboard.press('Escape');
    await expect(button).toBeFocused();
    await captureResponsive(page, 'chat-unread-cleared');
    await page.reload();
    await expect(page.locator('.status.live')).toBeVisible();
    await expect(button).toHaveAccessibleName('Chat');
    await button.click();
    await expect(
      page.locator(`[data-operation-id="${ask.operationId}"]`).getByTestId('ask-reply'),
    ).toBeVisible();
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
  });
});

test('own admitted Chat final notifies once, opens its exact turn, and survives reload without replay', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('notification-agent', { gated: true });
    const author = await pairBrowser(world, 'notification-author', { talk: true });
    const viewer = await pairBrowser(world, 'notification-viewer');
    const created = createPage(world, 'Reply notifications', '<p id="quote">Canonical quote.</p>');
    const page = await openPage(door, author, created);
    const other = await openPage(door, viewer, created);
    await expect(other.locator('.status.live')).toBeVisible();
    const ask = await sendChat(page, await composeChat(page, agent, 'Notify my own device.'));
    await until(() => agent.received().length === 1, 'one admitted notification Ask');
    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    const requestId = agent.received()[0].requestId as string;
    writeFileSync(path.join(agent.gate, `${requestId}.release`), '');
    await until(() => agent.rows().some((row) => row.event === 'replied'), 'durable final reply');
    const reply = agent.rows().find((row) => row.event === 'replied')!.body as string;
    const card = page.locator('.tmt-ui-notification');
    await expect(card).toHaveCount(1);
    await expect(card).toContainText(reply);
    await expect(page.locator('.reply-announcement')).toHaveCount(0);
    await expect(other.locator('.tmt-ui-notification')).toHaveCount(0);
    await captureResponsive(page, 'notification-chat-queue');
    await card.getByRole('button', { name: 'Open Chat', exact: true }).click();
    const target = page.locator(`[data-testid=ask-entry][data-operation-id="${ask.operationId}"]`);
    await expect(target.getByTestId('ask-reply')).toHaveText(reply);
    await expect(page.getByTestId('chat-panel').locator('.conversation-messages')).toBeFocused();
    await captureResponsive(page, 'notification-chat-target');
    // The peer sees the admitted reply in shared history, but never receives a
    // notification for another device's Ask ledger, including after it closes.
    await (await pageAction(other, 'Chat')).click();
    await expect(other.getByTestId('chat-panel').getByTestId('ask-reply')).toHaveText(reply);
    await other.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await expect(other.locator('.tmt-ui-notification')).toHaveCount(0);
    await page.reload();
    await expect(page.locator('.status.live')).toBeVisible();
    await expect(page.locator('.tmt-ui-notification')).toHaveCount(0);
    await (await pageAction(page, 'Chat')).click();
    await expect(target.getByTestId('ask-reply')).toHaveText(reply);
    expect(agent.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      1,
    );
  });
});

test('an authenticated participant comment notifies the thread author and opens the canonical quote', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const author = await pairBrowser(world, 'thread-notification-author', { talk: true });
    const writer = await pairBrowser(world, 'thread-notification-writer', { talk: true });
    const created = createPage(world, 'Thread notifications', '<p id="quote">Canonical quote.</p>');
    const page = await openPage(door, author, created);
    const other = await openPage(door, writer, created);
    await selectInRenderer(page, '#quote');
    await page.getByTestId('selection-ask').click();
    const compose = page.getByRole('dialog', { name: 'Annotate selection' });
    await compose.getByRole('combobox', { name: 'Message', exact: true }).fill('My own thread.');
    await compose.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    const original = page.getByTestId('comment-thread');
    await expect(original.getByTestId('comment-entry')).toHaveCount(1);
    const id = await original.getAttribute('data-thread-id');
    await original.getByRole('button', { name: 'Collapse thread', exact: true }).click();
    await (await pageAction(other, 'Comments')).click();
    await other.getByTestId('annotation-row').filter({ hasText: 'Canonical quote.' }).click();
    const thread = other.getByTestId('comment-thread');
    await thread
      .getByRole('combobox', { name: 'Message', exact: true })
      .fill('A signed participant reply.');
    await thread.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
    const card = page.locator('.tmt-ui-notification');
    await expect(card).toHaveCount(1);
    await expect(card).toContainText('A signed participant reply.');
    await expect(card.locator('blockquote')).toHaveText('Canonical quote.');
    await captureResponsive(page, 'notification-thread-queue');
    await card.getByRole('button', { name: 'Open Thread', exact: true }).click();
    await expect(original).toHaveAttribute('data-thread-id', id!);
    await expect(original).toHaveAttribute('data-anchor', 'attached');
    await expect(original.locator('.thread-state').first()).toHaveAttribute(
      'title',
      'Canonical quote.',
    );
    await expect(original.getByTestId('comment-entry')).toHaveCount(2);
    await expect(
      original.getByTestId('comment-entry').filter({ hasText: 'A signed participant reply.' }),
    ).toBeInViewport();
    await expect(page.locator('.reply-announcement')).toHaveCount(0);
    await captureResponsive(page, 'notification-thread-target');
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
    await page.reload();
    await expect(page.locator('.status.live')).toBeVisible();
    await expect(card).toHaveCount(0);
    await (await pageAction(page, 'Comments')).click();
    await page.getByTestId('annotation-row').filter({ hasText: 'Canonical quote.' }).click();
    await expect(original.getByTestId('comment-entry')).toHaveCount(2);
  });
});

for (const [destination, forceFallback] of [
  ['Answer', false],
  ['Proposal', false],
  ['Proposal', true],
] as const)
  test(`own admitted ${destination} follow-up reveals and focuses its block across the served Remote boundary${forceFallback ? ' with viewport-coupled layout' : ''}`, async () => {
    await withWorld(async (world) => {
      const door = await startDoor(world, await freePort());
      const agent = await world.startAgent('block-notification-agent', { gated: true });
      const author = await pairBrowser(world, 'block-notification-author', { talk: true });
      const viewer = await pairBrowser(world, 'block-notification-viewer');
      const qid = '10000000-0000-4000-8000-000000000090';
      const created = createPage(
        world,
        `${destination} notifications`,
        `<div style="height:1100px">Context above the block.</div>${destination === 'Answer' ? `<tmt-question data-id="${qid}" data-kind="short" data-agent="${agent.name}"><p data-question>What should we do next?</p></tmt-question><div style="height:1000px">Context below the block.</div>` : ''}`,
      );
      if (destination === 'Proposal')
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
            'A proposal with a follow-up.',
            '--json',
          ],
          undefined,
          agent.pane,
        );
      const page = await openPage(door, author, created);
      const other = await openPage(door, viewer, created);
      await expect(other.locator('.status.live')).toBeVisible();
      const block = page.locator(
        destination === 'Answer' ? '.question-card' : '.proposal-inline .proposal-card',
      );
      await expect(block).toHaveCount(1);
      const proposalId =
        destination === 'Proposal' ? await block.getAttribute('data-proposal-id') : undefined;
      if (destination === 'Answer') {
        const row = page.locator('.question-line');
        await expect(row).toHaveAttribute('aria-expanded', 'false');
        await row.click();
        await block.getByLabel('Your answer', { exact: true }).fill('Follow this direction.');
        const send = page
          .locator('[data-question-group]')
          .getByRole('button', { name: /^Send answer/ });
        await expect(send).toBeEnabled();
        await send.click();
        await expect(row).toContainText('Answered: Follow this direction.');
        await expect(row).toHaveAttribute('aria-expanded', 'false');
        await row.click();
        await expect(
          block.getByRole('button', { name: 'Change answer', exact: true }),
        ).toBeEnabled();
        await row.click();
        await expect(row).toHaveAttribute('aria-expanded', 'false');
      } else {
        await block.getByRole('button', { name: 'Follow up', exact: true }).click();
        const input = block.getByRole('combobox', { name: 'Message', exact: true });
        // Explicit visible recipient avoids racing the async fresh-draft seed;
        // this scenario proves notifications, not creator-default initialization.
        await input.fill(`@${agent.name} Please follow up on this proposal.`);
        await annotationInput(block, agent.name);
        await expect(block.locator('.annotation-hint[role="status"]')).toContainText(
          `Asks ${agent.name}.`,
        );
        await block.getByRole('button', { name: /^Send(?: · to .+)?$/, exact: true }).click();
      }
      await until(() => agent.received().length === 1, 'one admitted block follow-up Ask');
      if (forceFallback) {
        // Exercise the existing bounded height fallback through real layout,
        // rather than fabricating renderer messages or notification records.
        await page
          .frameLocator('iframe')
          .locator('body')
          .evaluate((body) => {
            body.style.minHeight = '200vh';
          });
        await expect(page.locator('iframe[data-scroll-mode="frame"]')).toBeVisible();
        await expect(page.locator('.proposal-inline')).toHaveAttribute('data-detached', 'true');
      }
      if (destination === 'Proposal') {
        // The bounded renderer may detach during live spacer growth. Its hidden
        // inline DOM is not evidence that Send completed; Comments owns recovery.
        await expect
          .poll(async () => {
            if (await block.evaluate((node) => !!node.closest('[data-detached]')))
              return 'detached';
            return (await block.getByRole('combobox', { name: 'Message', exact: true }).count()) ===
              0 && (await block.getByRole('button', { name: 'Follow up', exact: true }).isEnabled())
              ? 'ready'
              : 'pending';
          })
          .not.toBe('pending');
      }
      await page.evaluate(() => window.scrollTo(0, 0));
      const detached = await block.evaluate((node) => !!node.closest('[data-detached]'));
      if (detached) {
        await expect(block).toBeHidden();
        await expect(page.locator('iframe[data-scroll-mode="frame"]')).toBeVisible();
      } else {
        const placement =
          destination === 'Answer' ? page.locator('.question-compact-block') : block;
        await expect(placement).toBeVisible();
        await expect
          .poll(() => placement.evaluate((node) => node.getBoundingClientRect().top))
          .toBeGreaterThan(900);
      }
      const requestId = agent.received()[0].requestId as string;
      writeFileSync(path.join(agent.gate, `${requestId}.release`), '');
      await until(
        () => agent.rows().some((row) => row.event === 'replied'),
        'durable block follow-up reply',
      );
      const reply = agent.rows().find((row) => row.event === 'replied')!.body as string;
      const card = page.locator('.tmt-ui-notification');
      await expect(card).toHaveCount(1);
      await expect(card).toContainText(reply);
      await expect(other.locator('.tmt-ui-notification')).toHaveCount(0);
      await expect(page.locator('.reply-announcement')).toHaveCount(0);
      await captureResponsive(page, `notification-${destination.toLowerCase()}-queue`);
      await card.getByRole('button', { name: `Open ${destination}`, exact: true }).click();
      const target = destination === 'Answer' ? block : page.locator('.proposal-card:visible');
      if (destination === 'Proposal') {
        await expect(target).toHaveAttribute('data-proposal-id', proposalId!);
        if (detached) await expect(commentsRegion(page)).toBeVisible();
      }
      await expect(target).toBeFocused();
      await expect(target).toHaveAttribute('data-arrived', '');
      await expect(target).toContainText(reply);
      await expect
        .poll(() => target.evaluate((node) => node.getBoundingClientRect().top))
        .toBeGreaterThanOrEqual(48);
      await expect
        .poll(() => target.evaluate((node) => node.getBoundingClientRect().top))
        .toBeLessThan(900);
      await captureResponsive(page, `notification-${destination.toLowerCase()}-target`);
      await page.keyboard.press('Tab');
      await expect(target).not.toHaveAttribute('data-arrived', '');
      await page.reload();
      await expect(page.locator('.status.live')).toBeVisible();
      await expect(card).toHaveCount(0);
      expect(agent.received()).toHaveLength(1);
      expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
        1,
      );
    });
  });
