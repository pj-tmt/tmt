import { expect, test } from '@playwright/test';
import { pairBrowser, restartRemote, startDoor } from './harness/browser.js';
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

// #2572 (#2559 C1a): a Remote restart ends the open page's tunnel and Remote session while the
// device is still paired and serve is running. The page reconnects on its own: no Reconnect
// click, no reload, then it reads the next write and the composer reaches the agent.
test('an open owner page survives a Remote restart without a click or a reload', async () => {
  test.setTimeout(240_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('reconnect-agent');
    const browser = await pairBrowser(world, 'reconnect-owner', { talk: true });
    const created = createPage(world, 'Remote restart', '<h1 id="h">Before</h1>', agent.pane);
    const page = await openPage(door, browser, created);
    await expect(page.locator('.status.live .status-label')).toHaveText('Live');
    await openChat(page);
    const navigations: string[] = [];
    page.on('framenavigated', (frame) => {
      if (frame === page.mainFrame()) navigations.push(frame.url());
    });

    // Remote dies and comes back on the same port, as after a tmt upgrade.
    await door.remote.kill();
    await new Promise((resolve) => setTimeout(resolve, 3000));
    await restartRemote(world, door);

    await expect(page.locator('.status.live .status-label')).toHaveText('Live', {
      timeout: 90_000,
    });
    await expect(page.getByRole('button', { name: 'Reconnect', exact: true })).toHaveCount(0);

    // It reads: a write after the restart reaches the open tab.
    run(
      world,
      world.binaries.colab,
      ['page', 'write', created.pageId, '--file', '-', '--json'],
      '<h1 id="h">After</h1>',
    );
    await expect(page.frameLocator('iframe').locator('#h')).toHaveText('After', {
      timeout: 60_000,
    });

    // It talks: the composer reaches the agent with the exact text.
    const composed = await composeChat(page, agent, 'Back after the restart?');
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
      .toContain('Back after the restart?');
    expect(navigations).toEqual([]);
  });
});
