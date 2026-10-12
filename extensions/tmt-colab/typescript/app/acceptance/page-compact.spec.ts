import { pageAction } from '../test/page-actions.js';
import { expect, test } from '@playwright/test';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, freePort, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

// #2846: the owner browser asks the serve how full a page is and to combine this device's own
// changes. The serve answers from the same count the CLI reports, and the page is unchanged.
test.afterEach(disposeActiveWorlds);
test('the owner browser shows the page size the CLI reports and compacts it without changing the page', async () => {
  test.setTimeout(300_000);
  await withWorld(async (world) => {
    const door = await startDoor(world, await freePort());
    const a = await pairBrowser(world, 'page-size-owner');
    const colab = world.binaries.colab;
    const json = (args: string[]) => JSON.parse(run(world, colab, args)) as Record<string, any>;
    const created = createPage(world, 'Page size', '<p>start</p>');
    let source = '<p>start</p>';
    for (let i = 0; i < 12; i++) {
      source += `<i>${String(i).padStart(2, '0')}</i>`;
      run(world, colab, ['page', 'write', created.pageId, '--file', '-', '--json'], source);
    }
    const before = json(['page', 'read', created.pageId, '--json']).usage;
    expect(before.updates).toBeGreaterThanOrEqual(12);

    const page = await openPage(door, a, created);
    await (await pageAction(page, 'About this page')).click();
    const section = page.locator('.page-size');
    await expect(section.getByRole('heading', { name: 'Page size' })).toBeVisible();
    // The browser shows the serve's numbers, not its own count.
    await expect(section.locator('dd').first()).toHaveText(`${before.updates} of 200`);
    await section.getByRole('button', { name: 'Compact page' }).click();
    await expect(section.getByRole('status')).toContainText('Content is unchanged.');
    await expect(section.getByRole('status')).toContainText(/^Combined \d+ changes? into one\./);

    const after = json(['page', 'read', created.pageId, '--json']);
    expect(after.usage.updates).toBeLessThan(before.updates);
    expect(after.usage.checkpointBytes).toBeGreaterThan(0);
    expect(after.source === source).toBe(true);
    await expect(section.locator('dd').first()).toHaveText(`${after.usage.updates} of 200`);

    // Nothing is left on this device: the CLI says so, and the browser agrees on the next ask.
    const again = json(['page', 'compact', created.pageId, '--json']);
    expect(again.compacted).toBe(false);
    expect(again.skipped).toBe('nothing-to-combine');
    await page.reload();
    await (await pageAction(page, 'Source')).click();
    expect(
      (await page.getByRole('textbox', { name: 'Source', exact: true }).inputValue()) === source,
    ).toBe(true);
  });
});
