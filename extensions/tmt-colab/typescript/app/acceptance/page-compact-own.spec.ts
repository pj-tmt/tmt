import { pageAction } from '../test/page-actions.js';
import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  askState,
  composeChat,
  createPage,
  freePort,
  openChat,
  openPage,
  run,
  sendChat,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #2846 PR 3: only the device that wrote a stream can combine it. The owner browser's own
// changes (its Chat questions) sit in the page's tail until this browser seals a checkpoint over
// its stream. The serve stores it, the tail shrinks, and a fresh load from the checkpoint shows
// every message again.
test.afterEach(disposeActiveWorlds);
test("the owner browser combines its own stream's changes into one checkpoint and loses none", async () => {
  test.setTimeout(300_000);
  test.info().annotations.push({
    type: 'known-product-failure',
    description:
      'https://github.com/pj-tmt/tmt/issues/4275: own checkpoint append conflicts after compaction',
  });
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('own-compact-agent', { gated: true });
    const owner = await pairBrowser(world, 'own-compact-owner', { talk: true });
    const colab = world.binaries.colab;
    const usage = (id: string) =>
      (JSON.parse(run(world, colab, ['page', 'read', id, '--json'])) as Record<string, any>).usage;
    const created = createPage(world, 'Own compaction', '<p>start</p>', agent.pane);
    const page = await openPage(door, owner, created);
    const questions = ['first', 'second', 'third'];
    for (const question of questions) {
      const sent = await sendChat(page, await composeChat(page, agent, `Question ${question}`));
      // A visible intent precedes the Send result's own-stream update. Finish that write
      // before combining; the gated recipient cannot add a reply afterward.
      await expect(askState(page, sent.operationId)).toHaveAttribute('data-state', 'accepted');
    }
    const entries = page.getByTestId('chat-panel').getByTestId('ask-entry');
    await expect(entries).toHaveCount(questions.length);

    // Those are changes on this browser's stream: to the serve they are another device's.
    const before = usage(created.pageId);
    expect(before.otherDeviceUpdates).toBeGreaterThanOrEqual(questions.length);

    await page.getByRole('button', { name: 'Close Chat', exact: true }).click();
    await (await pageAction(page, 'About this page')).click();
    const section = page.locator('.page-size');
    await expect(section.getByRole('heading', { name: 'Page size' })).toBeVisible();
    // This device's own changes are its own to combine, not "on other devices".
    await expect(section).not.toContainText('other devices');
    await section.getByRole('button', { name: 'Compact page' }).click();
    await expect(section.getByRole('status')).toContainText(/^Combined \d+ changes? into one\./);

    const after = usage(created.pageId);
    expect(after.otherDeviceUpdates).toBe(0);
    expect(after.updates).toBeLessThan(before.updates);
    expect(after.checkpointBytes).toBeGreaterThan(before.checkpointBytes);
    // Nothing is left to combine on this device.
    await expect(section.getByRole('button', { name: 'Compact page' })).toHaveCount(0);

    // A fresh load starts from the checkpoint and shows every message.
    await page.reload();
    await openChat(page);
    await expect(page.getByTestId('chat-panel').getByTestId('ask-entry')).toHaveCount(
      questions.length,
    );
    // And the stream goes on from the checkpoint: one more question still lands.
    await sendChat(page, await composeChat(page, agent, 'Question fourth'));
    await expect(page.getByTestId('chat-panel').getByTestId('ask-entry')).toHaveCount(
      questions.length + 1,
    );
  });
});
