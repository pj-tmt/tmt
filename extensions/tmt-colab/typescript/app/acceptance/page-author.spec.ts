import { pageAction } from '../test/page-actions.js';
import { expect, test } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { pairBrowser, startDoor, openReaderLink } from './harness/browser.js';
import { createPage, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);
test('original author survives later publishing and owner reload and reads to a reader without becoming a recipient', async () => {
  await withWorld(async (world) => {
    const creator = await world.startAgent('author-creator');
    const publisher = await world.startAgent('author-publisher');
    const created = createPage(world, 'Author record', '<h1>Created source</h1>', creator.pane);
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'author-viewer');
    const page = await openPage(door, browser, created);
    const caption = page.locator('.tmt-ui-caption');
    await expect(caption).toHaveText(`By ${creator.name}`);
    // Browser Save carries no CLI label, so it removes the previous one; the original stays.
    await (await pageAction(page, 'Source')).click();
    const source = page.locator('.page-drawer[data-panel=source]');
    await source
      .getByRole('textbox', { name: 'Source', exact: true })
      .fill('<h1>Browser source</h1>');
    await source.getByRole('button', { name: 'Save source' }).click();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Browser source' }),
    ).toBeVisible();
    await expect(caption).toHaveText(`By ${creator.name}`);
    const read = JSON.parse(
      run(world, world.binaries.colab, ['page', 'read', created.pageId, '--json']),
    );
    run(
      world,
      world.binaries.colab,
      [
        'page',
        'write',
        created.pageId,
        '--file',
        '-',
        '--expected-revision',
        read.revision,
        '--json',
      ],
      '<h1>Later source</h1>',
      publisher.pane,
    );
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Later source' }),
    ).toBeVisible();
    await expect(caption).toHaveText(`By ${creator.name}`);
    await page.reload();
    await expect(caption).toHaveText(`By ${creator.name}`);
    await (await pageAction(page, 'About this page')).click();
    await expect(page.locator('.page-attribution')).toContainText(
      `Latest publisher${publisher.name}`,
    );
    const captures = process.env.COLAB_AUTHOR_NATIVE_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 844 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((value) => (document.documentElement.dataset.theme = value), theme);
          await expect(caption).toHaveText(`By ${creator.name}`);
          await page.screenshot({ path: `${captures}/${width}-${theme}-owner.png` });
        }
      }
    }
    await page.setViewportSize({ width: 1440, height: 844 });
    await page.getByRole('button', { name: 'Close About this page', exact: true }).click();
    await (await pageAction(page, 'Export page')).click();
    const pending = page.waitForEvent('download');
    await page.getByRole('button', { name: /Download manifest.json/ }).click();
    const downloaded = await pending;
    expect(await downloaded.failure()).toBeNull();
    const manifest = JSON.parse(readFileSync((await downloaded.path())!, 'utf8'));
    expect(manifest.originalAuthor).toBe(creator.name);
    expect(manifest.publisherAgent).toBe(publisher.name);
    await page.getByRole('button', { name: 'Close export', exact: true }).click();
    run(world, world.binaries.colab, ['share', 'mode', created.pageId, 'link', '--yes', '--json']);
    const link = JSON.parse(
      run(world, world.binaries.colab, ['share', 'link', 'add', created.pageId, '--yes', '--json']),
    );
    const reader = await openReaderLink(world, door, link.readerPath, 'author-reader');
    await expect(reader.page.locator('.tmt-ui-caption')).toHaveText(`By ${creator.name}`);
    await reader.page.locator('.reader-information summary').click();
    await expect(reader.page.locator('.page-attribution')).toContainText(
      `Latest publisher${publisher.name}`,
    );
    await expect(reader.page.locator('textarea, [data-testid=ask-action]')).toHaveCount(0);
    if (captures) {
      for (const width of [1440, 390]) {
        await reader.page.setViewportSize({ width, height: 844 });
        for (const theme of ['light', 'dark']) {
          await reader.page.evaluate(
            (value) => (document.documentElement.dataset.theme = value),
            theme,
          );
          await reader.page.screenshot({ path: `${captures}/${width}-${theme}-reader.png` });
        }
      }
    }
    expect(creator.received()).toHaveLength(0);
    expect(publisher.received()).toHaveLength(0);
    expect(world.coreCalls().filter((call) => call.operation === 'dispatch.create')).toHaveLength(
      0,
    );
  });
});
