import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import {
  composeChat,
  createPage,
  freePort,
  openChat,
  openPage,
  run,
  sendChat,
} from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

// #2557 (#2559 C2): an epoch advance ends the owner's sync socket with STALE_EPOCH while the
// device is still paired and serve is running. The open page re-admits on its own: no reload,
// no stopped card, the page reads the next write and the composer still reaches the agent.
test('an open owner page survives an epoch advance: it stays live, reads and sends without a reload', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('readmit-agent');
    const browser = await pairBrowser(world, 'readmit-owner', { talk: true });
    const created = createPage(world, 'Epoch advance', '<h1 id="h">Before</h1>', agent.pane);
    const page = await openPage(door, browser, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');
    await openChat(page);

    const colab = (args: string[], input?: string) =>
      JSON.parse(run(world, world.binaries.colab, [...args, '--json'], input)) as Record<
        string,
        unknown
      >;
    // Adding then removing a share link advances the page epoch, which ends the live socket.
    colab(['share', 'mode', created.pageId, 'link', '--yes']);
    const link = colab(['share', 'link', 'add', created.pageId, '--yes']);
    colab(['share', 'link', 'remove', created.pageId, link.linkId as string, '--yes']);

    const navigations: string[] = [];
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations.push(frame.url());
    });
    // The stopped card may never become the final state, and the status returns to Live.
    await expect(page.locator('.status.live .status-label')).toHaveText('Live', {
      timeout: 60_000,
    });
    await expect(page.getByRole('heading', { name: 'Preview stopped', exact: true })).toHaveCount(
      0,
    );

    // It reads: a write after the advance reaches the open tab.
    colab(['page', 'write', created.pageId, '--file', '-'], '<h1 id="h">After</h1>');
    await expect(page.frameLocator('iframe').locator('#h')).toHaveText('After', {
      timeout: 60_000,
    });

    // It talks: the composer reaches the agent with the exact text.
    const composed = await composeChat(page, agent, 'Still connected?');
    await sendChat(page, composed);
    await expect
      .poll(
        () => {
          try {
            return composed.delivered();
          } catch {
            return '';
          }
        },
        { timeout: 60_000 },
      )
      .toContain('Still connected?');
    expect(navigations).toEqual([]);
  });
});
