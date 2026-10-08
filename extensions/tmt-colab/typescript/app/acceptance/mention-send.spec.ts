import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openChat, openPage, run } from './harness/ask.js';
import { until } from './harness/process.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
test('visible mentions fan out one recorded Chat turn to live and offline saved agents without replay', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const alpha = await world.startAgent('mention-alpha');
    const beta = await world.startAgent('mention-beta');
    const offline = await world.startAgent('mention-offline');
    const created = createPage(
      world,
      'Mention routing',
      '<p>One comment, several recipients.</p>',
      alpha.pane,
    );
    world.tmux(['kill-pane', '-t', offline.pane]);
    const browser = await pairBrowser(world, 'mention-author');
    const page = await openPage(door, browser, created);
    await openChat(page);
    const panel = page.getByTestId('chat-panel');
    const input = panel.getByRole('combobox', { name: 'Message', exact: true });
    await expect(input).toHaveText(`@${alpha.name} `, { useInnerText: true });
    await input.fill(
      `@${alpha.name} @${beta.name} @${alpha.name} @${offline.name} <script>Keep exact bytes.</script>`,
    );
    await expect(panel.locator('.annotation-status-row [role=status]')).toHaveText(
      `Asks @${alpha.name}, @${beta.name}, @${offline.name} (offline).`,
    );
    await input.press('Enter');
    await until(
      () => alpha.received().length === 1 && beta.received().length === 1,
      'one actual delivery to each live agent',
    );
    const entries = panel.getByTestId('ask-entry');
    await expect(entries).toHaveCount(3);
    await expect(panel.getByTestId('comment-entry')).toHaveCount(1);
    const ids = await entries.evaluateAll((nodes) =>
      nodes.map((node) => (node as HTMLElement).dataset.operationId),
    );
    expect(new Set(ids).size).toBe(3);
    await expect(panel.getByTestId('ask-reply')).toHaveCount(2);
    const pending = JSON.parse(
      run(world, world.binaries.tmt, ['inbox', '--identity', offline.id, '--json']),
    ) as { items: { requestId: string; preview: string }[] };
    expect(pending.items).toHaveLength(1);
    expect(pending.items[0].requestId).toMatch(/^req_[0-9a-f-]+$/);
    expect(pending.items[0].preview).toContain('[remote: mention-author]');
    for (const agent of [alpha, beta])
      expect(agent.received()[0].message).toContain('<script>Keep exact bytes.</script>');
    await expect(panel.locator('[data-testid=ask-entry][data-ledger-state=accepted]')).toHaveCount(
      3,
    );
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      3,
    );
    await page.reload();
    await openChat(page);
    await expect(page.getByTestId('ask-entry')).toHaveCount(3);
    await expect(page.getByTestId('comment-entry')).toHaveCount(1);
    expect(alpha.received()).toHaveLength(1);
    expect(beta.received()).toHaveLength(1);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      3,
    );
    const plain = page
      .getByTestId('chat-panel')
      .getByRole('combobox', { name: 'Message', exact: true });
    await plain.fill('@missing This is a comment.');
    await plain.press('Enter');
    await expect(page.getByTestId('comment-entry')).toHaveCount(2);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      3,
    );
    expect(alpha.received()).toHaveLength(1);
    expect(beta.received()).toHaveLength(1);
  });
});
