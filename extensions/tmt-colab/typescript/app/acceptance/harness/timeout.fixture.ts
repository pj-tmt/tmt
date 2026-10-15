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
      // A safe parent diagnostic is captured before either timeout path closes the context.
      await browser.page.setContent('<code data-own-publication></code>');
      await browser.page.locator('code').evaluate((node) => {
        node.setAttribute(
          'data-own-publication',
          JSON.stringify({
            root: 'messages',
            kind: 'ask-state',
            streamId: '30000000-0000-4000-8000-000000000001',
            submittedSeq: '7',
            admittedSeq: '4',
            leaderReceipt: true,
          }),
        );
      });
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
