import { expect, test } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { pageAction } from '../test/page-actions.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  composeChat,
  createPage,
  freePort,
  openPage,
  selectInRenderer,
  sendChat,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { captureResponsive } from './harness/captures.js';

test.afterEach(disposeActiveWorlds);

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
    await expect(page.locator('.reply-announcement')).toHaveText('');
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
    await compose.getByRole('button', { name: 'Send', exact: true }).click();
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
    await thread.getByRole('button', { name: 'Send', exact: true }).click();
    const card = page.locator('.tmt-ui-notification');
    await expect(card).toHaveCount(1);
    await expect(card).toContainText('A signed participant reply.');
    await expect(card.locator('blockquote')).toHaveText('Canonical quote.');
    await captureResponsive(page, 'notification-thread-queue');
    await card.getByRole('button', { name: 'Open Thread', exact: true }).click();
    await expect(original).toHaveAttribute('data-thread-id', id!);
    await expect(original).toHaveAttribute('data-anchor', 'attached');
    await expect(original.locator('blockquote').first()).toHaveText('Canonical quote.');
    await expect(original.getByTestId('comment-entry')).toHaveCount(2);
    await expect(
      original.getByTestId('comment-entry').filter({ hasText: 'A signed participant reply.' }),
    ).toBeInViewport();
    await expect(page.locator('.reply-announcement')).toHaveText('');
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
