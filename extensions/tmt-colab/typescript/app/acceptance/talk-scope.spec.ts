import { expect, test } from '@playwright/test';
import { text } from '../src/strings.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  askEntry,
  clientId,
  composeChat,
  createPage,
  freePort,
  openChat,
  openPage,
  sendChat,
} from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #2359: pairing for page access and pairing to send are different grants. A device without
// sending reads the page and the agent list, and a real Ask is refused before any effect.
test.afterEach(disposeActiveWorlds);
test('a device paired without sending reads the page, is refused with the sending notice, keeps that refusal across reload and wakes nothing; a sending device does', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('talk-scope-agent');
    const created = createPage(
      world,
      'Sending scope',
      '<h1>Sending scope</h1><p>Read here, send only when enabled.</p>',
      agent.pane,
    );
    const dispatches = () =>
      world.coreCalls().filter((call) => call.operation === 'dispatch.create');

    const reader = await pairBrowser(world, 'talk-reader');
    const page = await openPage(door, reader, created);
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Sending scope' }),
    ).toBeVisible();
    await openChat(page);
    const panel = page.getByTestId('chat-panel');
    const input = panel.getByRole('combobox', { name: 'Message', exact: true });
    // The agent list is readable without sending: the draft starts addressed to the page's agent.
    await expect(input).toHaveText(`@${agent.name} `, { useInnerText: true });
    // The notice names the device the owner turns sending on for: Remote's own client ID.
    const notice = text.askTalkNotEnabled(clientId(world, 'talk-reader'));
    const composed = await composeChat(page, agent, 'Can this device send?');
    const { operationId } = await sendChat(page, composed);
    const entry = panel.getByTestId('ask-entry');
    await expect(entry).toHaveCount(1);
    await expect(entry).toHaveAttribute('data-ledger-state', 'refused');
    await expect(entry).toContainText(notice);
    expect(dispatches()).toHaveLength(0);
    expect(agent.received()).toHaveLength(0);

    await page.reload();
    await openChat(page);
    const restored = page.getByTestId('chat-panel').getByTestId('ask-entry');
    await expect(restored).toHaveCount(1);
    await expect(restored).toHaveAttribute('data-operation-id', operationId);
    await expect(restored).toHaveAttribute('data-ledger-state', 'refused');
    await expect(restored).toContainText(notice);
    expect(dispatches()).toHaveLength(0);
    expect(agent.received()).toHaveLength(0);

    // Positive control: the same page, a device paired with sending, one real delivery.
    const sender = await pairBrowser(world, 'talk-sender', { talk: true });
    const sendPage = await openPage(door, sender, created);
    const sending = await composeChat(sendPage, agent, 'Can this device send?');
    const sent = await sendChat(sendPage, sending);
    await until(() => agent.received().length === 1, 'one delivery from the sending device');
    expect(dispatches()).toHaveLength(1);
    // The page's Chat is shared, so the reader's refused turn is visible here too; only this
    // device's own turn is judged.
    const own = askEntry(sendPage, sent.operationId);
    await expect(own).toHaveAttribute('data-ledger-state', 'accepted');
    await expect(own).not.toContainText(notice);
  });
});
