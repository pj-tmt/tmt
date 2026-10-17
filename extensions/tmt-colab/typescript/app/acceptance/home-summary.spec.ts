import { expect, test } from '@playwright/test';
import { openColab, pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

test('a home summary the maintainer writes reaches Today in a paired browser, and clearing removes it', async () => {
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const agent = await world.startAgent('summary-agent');
    createPage(world, 'Weekly plan', '<p>Plan</p>', agent.pane);
    const browser = await pairBrowser(world, 'summary-browser');
    const page = await openColab(door, browser);
    const summary = page.locator('.today-summary');
    await expect(page.locator('.today h1')).toBeVisible();
    await expect(summary).toHaveCount(0);

    const home = (...args: string[]) =>
      run(world, world.binaries.colab, ['home', 'summary', ...args], undefined, agent.pane);
    // The write is the real CLI against the running serve; the lines are the maintainer's own.
    home('set', '--line', 'Plan updated', '--line', 'Two items need your review', '--json');
    await page.reload();
    await expect(summary.locator('.today-summary-line')).toHaveText([
      'Plan updated',
      'Two items need your review',
    ]);
    // The author is the caller's agent identity, sent by the CLI over the owner socket.
    await expect(summary.locator('.today-summary-meta')).toContainText('summary-agent');
    // The page was created before the summary, so nothing changed after it.
    await expect(summary.locator('.today-summary-meta')).not.toContainText('changed since');

    // A line over its limit is refused whole: the stored summary and Today are unchanged.
    expect(() => home('set', '--line', 'x'.repeat(25), '--line', 'fine', '--json')).toThrow(
      /COLAB_INPUT_INVALID/,
    );
    await page.reload();
    await expect(summary.locator('.today-summary-line').first()).toHaveText('Plan updated');

    home('clear', '--json');
    await page.reload();
    await expect(page.locator('.today h1')).toBeVisible();
    await expect(summary).toHaveCount(0);
  });
});
