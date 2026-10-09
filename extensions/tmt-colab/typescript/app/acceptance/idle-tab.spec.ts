import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// The remote door tears down a mounted tunnel that moves no bytes for 120 s, and a browser cannot
// send WebSocket pings. Colab pings its peer, so an idle tab keeps one sync connection. Without
// it every idle tab drops every two minutes and each drop costs one journaled Remote read; Remote
// keeps 1000 per device per day, after which "Agents are unavailable." (#2170).
test.describe('idle tab keeps its sync tunnel (#2170)', () => {
  test.afterEach(disposeActiveWorlds);

  test('a live tab idle past the door tunnel limit never reconnects', async () => {
    test.setTimeout(300_000);
    await withWorld(async (world) => {
      const door = await startDoor(world, await freePort());
      const browser = await pairBrowser(world, 'idle-browser');
      const created = createPage(world, 'Idle tab', '<h1>Idle tab</h1>');
      const page = await openPage(door, browser, created);
      const sockets: { opened: number; closed: number } = { opened: 0, closed: 0 };
      page.on('websocket', (socket) => {
        if (!new URL(socket.url()).pathname.endsWith('/sync')) return;
        sockets.opened++;
        socket.on('close', () => sockets.closed++);
      });
      await expect(page.locator('.status.live .status-label')).toHaveText('Live');
      // The tab was already connected before the listener existed: count only changes from here.
      await page.reload();
      await expect(page.locator('.status.live .status-label')).toHaveText('Live');
      const opened = sockets.opened;
      expect(opened).toBeGreaterThanOrEqual(1);
      await page.waitForTimeout(150_000);
      await expect(page.locator('.status.live .status-label')).toHaveText('Live');
      expect(sockets.closed).toBe(0);
      expect(sockets.opened).toBe(opened);
    });
  });
});
