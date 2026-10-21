import { expect, test } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { showAllPages } from '../test/home-list.js';
import { pairBrowser, startDoor } from './harness/browser.js';
import { createPage, openPage, run } from './harness/ask.js';
import { disposeActiveWorlds, withWorld } from './harness/with-world.js';

test.afterEach(disposeActiveWorlds);

test('served quick search crosses the Remote boundary, switches pages and keeps source records unchanged', async () => {
  await withWorld(async (world) => {
    const first = createPage(
      world,
      'Boundary alpha',
      '<h2>Alpha source</h2><p>Read-only search proof.</p>',
    );
    const second = createPage(world, 'Boundary beta', '<h2>Beta source</h2>');
    const third = createPage(
      world,
      'A longer boundary page title for the compact result layout',
      '<p>Third source</p>',
    );
    const read = (id: string) =>
      JSON.parse(run(world, world.binaries.colab, ['page', 'read', id, '--json'])) as {
        revision: string;
        source: string;
      };
    const before = [read(first.pageId), read(second.pageId)];
    expect(before[0].source).toContain('Alpha source');
    expect(before[1].source).toContain('Beta source');
    for (const snapshot of before) expect(typeof snapshot.revision).toBe('string');
    const door = await startDoor(world);
    const browser = await pairBrowser(world, 'search-viewer');
    // Only verified folds supply browser-local title hints; never-opened pages
    // deliberately have no searchable title. Visit normally before switching.
    const visited = await openPage(door, browser, second);
    for (const [created, title] of [
      [second, 'Boundary beta'],
      [third, 'A longer boundary page title for the compact result layout'],
    ] as const) {
      await visited.goto(new URL(created.path, `${door.address}/`).href);
      await expect(visited.locator('.tmt-ui-header h1')).toHaveText(title);
      await visited.getByRole('link', { name: /^Colab · local/ }).click();
      await showAllPages(visited);
      await expect(
        visited.locator(`[data-page-id="${created.pageId}"]`).getByRole('link'),
      ).toHaveText(title);
    }
    await visited.close();
    const page = await openPage(door, browser, first);
    const frame = page.frameLocator('iframe');
    await expect(frame.getByRole('heading', { name: 'Alpha source' })).toBeVisible();
    const shortcut = await page.evaluate(() =>
      /Mac|iPhone|iPad|iPod/.test(navigator.platform) ? 'Meta+k' : 'Control+k',
    );
    await frame.getByText('Read-only search proof.', { exact: true }).click();
    await page.keyboard.press(shortcut);
    const dialog = page.getByRole('dialog', { name: 'Search pages', exact: true });
    const input = dialog.getByRole('combobox', { name: 'Search pages', exact: true });
    await expect(input).toBeFocused();
    await expect(dialog.getByRole('option')).toHaveCount(2);
    await expect(dialog.getByRole('option').filter({ hasText: 'Boundary alpha' })).toHaveCount(0);
    const captures = process.env.COLAB_SWITCHER_NATIVE_CAPTURE_DIR;
    if (captures) {
      mkdirSync(captures, { recursive: true });
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        for (const theme of ['light', 'dark']) {
          await page.evaluate((theme) => (document.documentElement.dataset.theme = theme), theme);
          await expect(input).toBeFocused();
          await page.screenshot({ path: `${captures}/${width}-${theme}-ready.png` });
        }
      }
    }
    await page.keyboard.type('Boundary beta');
    await expect(dialog.getByRole('option')).toHaveCount(1);
    await expect(dialog.getByRole('option', { selected: true })).toContainText('Boundary beta');
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);
    await expect(page.locator('.tmt-ui-header h1')).toHaveText('Boundary beta');
    await expect(page.locator('.tmt-ui-header h1')).toBeFocused();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Beta source' }),
    ).toBeVisible();
    await page.reload();
    await expect(
      page.frameLocator('iframe').getByRole('heading', { name: 'Beta source' }),
    ).toBeVisible();
    const after = [read(first.pageId), read(second.pageId)];
    for (let i = 0; i < before.length; i++) {
      expect(after[i].revision).toBe(before[i].revision);
      expect(after[i].source).toBe(before[i].source);
    }
  });
});
