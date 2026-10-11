// Selected only by the child runner in harness.spec.ts, never the normal spec glob.
import { test } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './browser.js';
import { disposeActiveWorlds, withWorld } from './with-world.js';

test.afterEach(disposeActiveWorlds);

for (const racing of [false, true]) {
  test(`child timeout: ${racing ? 'cleanup already running' : 'filechooser pending'}`, async () => {
    const started = Date.now();
    await withWorld(async (world) => {
      const door = await startDoor(world);
      const browser = await pairBrowser(world, 'timeout-viewer');
      await openColab(door, browser);
      test.info().setTimeout(Date.now() - started + 250);
      if (racing) {
        world.onDispose(async () => {
          await new Promise((resolve) => setTimeout(resolve, 700));
        });
        throw new Error('injected failure before timeout');
      }
      await browser.page.waitForEvent('filechooser', { timeout: 0 });
    });
  });
}
