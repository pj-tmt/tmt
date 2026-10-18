import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, expectLive, freePort, openPage } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';
import { text } from '../src/strings.js';

test.afterEach(disposeActiveWorlds);

test('a page entered again in the same tab is live and writable, also on a slow machine', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('reentry-agent');
    const author = await pairBrowser(world, 'reentry-author', { talk: true });
    const reading = '10000000-0000-4000-8000-000000000601';
    const created = createPage(
      world,
      'Re-entry page',
      `<h1>Re-entry</h1><tmt-read data-id="${reading}" data-agent="${agent.name}" data-why="Read before deciding"><h2 data-title>First decision</h2><p>Content.</p></tmt-read>`,
      agent.pane,
    );
    const page = await openPage(door, author, created);
    // A slow machine: the window in which the page just left is still on screen is wide.
    const cdp = await author.context.newCDPSession(page);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: 6 });
    await expectLive(page);
    // Leave the page inside the app and come back to it.
    await page.getByRole('link', { name: text.home }).click();
    await page.getByRole('link', { name: 'Re-entry page' }).click();
    const band = page.locator(`[data-inline-reading="${reading}"]`);
    await expect(band).toHaveAttribute('data-reading-state', 'unread');
    // The page is written through the connection of this visit: Mark read records and notifies.
    await band.getByRole('button', { name: 'Mark read', exact: true }).click();
    await expect(band).toHaveAttribute('data-reading-state', 'read', { timeout: 30_000 });
    await expect.poll(() => agent.received().length).toBe(1);
  });
});
